use clap::{Args, Parser, Subcommand, ValueEnum};
use miette::{IntoDiagnostic, NamedSource};
#[cfg(feature = "bootstrap")]
use quilt::langs::bootstrap::Bootstrap;
use quilt::{
    lang::Language,
    langs::omni::Omni,
    multi::{Languages, MetaLanguages, Multi},
    prelude::*,
    term::STerm,
};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};

/**************************************************************/

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
#[command(args_conflicts_with_subcommands = true)]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// `run` is the default subcommand, so a `#!/usr/bin/env quilt` shebang
    /// (which invokes `quilt <script> <args>...`) runs the script.
    #[command(flatten)]
    run: Option<RunArgs>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Expand a file
    Expand(ExpandArgs),
    /// Run a .quilt file as a script
    Run(RunArgs),
    /// Validate .quilt files without writing output
    Check(CheckArgs),
    /// Clear the expand cache
    Clean,
}

#[derive(Args, Debug)]
struct ExpandArgs {
    /// file to expand
    #[clap(index = 1)]
    filename: String,
    /// multi-language to use
    #[clap(short, long, default_value_t, value_enum)]
    multi: MultiOptions,
}

#[derive(Debug, Clone, Default, ValueEnum)]
pub enum MultiOptions {
    /// The production path. `bootstrap` is opt-in via `-m bootstrap`.
    #[default]
    Omni,
    #[cfg(feature = "bootstrap")]
    Bootstrap,
}

#[derive(Args, Debug)]
struct CheckArgs {
    /// .quilt files to check
    #[clap(required = true)]
    filenames: Vec<String>,
    /// multi-language to use
    #[clap(short, long, default_value_t, value_enum)]
    multi: MultiOptions,
}

#[derive(Args, Debug)]
struct RunArgs {
    /// .quilt file to run
    filename: String,
    /// multi-language to use
    #[clap(short, long, default_value_t, value_enum)]
    multi: MultiOptions,
    /// Arguments to pass to the script
    #[clap(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/**************************************************************/

#[allow(clippy::unnecessary_wraps)]
fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Cli::parse();

    match (&args.command, &args.run) {
        (Some(Commands::Expand(args)), _) => expand(args),
        (Some(Commands::Run(args)), _) | (None, Some(args)) => run(args),
        (Some(Commands::Check(args)), _) => check(args),
        (Some(Commands::Clean), _) => clean(),
        (None, None) => {
            use clap::CommandFactory;
            Cli::command().print_help().into_diagnostic()?;
            std::process::exit(2);
        }
    }
}

fn clean() -> Result<()> {
    let Some(dir) = cache_dir() else {
        println!("No cache directory configured.");
        return Ok(());
    };
    if !dir.exists() {
        println!("Cache directory does not exist: {}", dir.display());
        return Ok(());
    }
    let count = fs::read_dir(&dir)
        .into_diagnostic()?
        .filter(|e| {
            e.as_ref()
                .ok()
                .and_then(|e| e.path().extension().map(|x| x == "postcard"))
                .unwrap_or(false)
        })
        .count();
    fs::remove_dir_all(&dir).into_diagnostic()?;
    println!(
        "Cleared {count} cached expansion(s) from {}.",
        dir.display()
    );
    Ok(())
}

fn expand(args: &ExpandArgs) -> Result<()> {
    let input_filename = &args.filename;
    // `expand` genuinely needs the suffix — the output file *is* the input name
    // with `.quilt` sliced off — so unlike `check` it is right to insist on one.
    // It used to `unwrap()` here, which turned `quilt expand bin/issues` into a
    // panic instead of a diagnostic (issue #188).
    let output_filename = input_filename
        .strip_suffix(".quilt")
        .ok_or_else(|| miette!("expected a .quilt file: {input_filename}"))?;

    let canonical = fs::canonicalize(input_filename).unwrap_or_else(|_| input_filename.into());
    let path_key = canonical.to_string_lossy().into_owned();
    let (mtime_secs, mtime_nanos) = fs::metadata(input_filename)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or((0, 0), |d| (d.as_secs(), d.subsec_nanos()));
    let multi_key = match args.multi {
        MultiOptions::Omni => "omni",
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => "bootstrap",
    };

    if let Some(cached) = cache_load(&path_key, mtime_secs, mtime_nanos, multi_key) {
        return generate(output_filename, &cached);
    }

    let input = fs::read_to_string(input_filename).expect("Should have been able to read the file");
    // attach the source so span-carrying errors render the offending snippet
    let with_src =
        |e: miette::Report| e.with_source_code(NamedSource::new(input_filename, input.clone()));
    let expanded = match args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, output_filename);
            let sterm = multi.parse_chain(&chain, &input).map_err(with_src)?;
            multi.expand_lang(chain[0], &sterm).map_err(with_src)?
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, output_filename);
            let sterm = multi.parse_chain(&chain, &input).map_err(with_src)?;
            multi.expand_lang(chain[0], &sterm).map_err(with_src)?
        }
    };

    cache_store(&path_key, mtime_secs, mtime_nanos, multi_key, &expanded);
    generate(output_filename, &expanded)
}

/// Validate each file like `expand` would (parse + expansion), but discard the
/// result instead of writing it — for CI pipelines and pre-commit hooks that
/// don't want generated files. Checks every file before failing so one broken
/// file doesn't hide errors in the rest.
fn check(args: &CheckArgs) -> Result<()> {
    let mut failures = 0;
    for filename in &args.filenames {
        match check_file(filename, &args.multi) {
            Ok(()) => println!("{filename}: ok"),
            Err(report) => {
                failures += 1;
                eprintln!("{filename}: {report:?}");
            }
        }
    }
    if failures > 0 {
        return Err(miette!(
            "{failures} of {} file(s) failed to check",
            args.filenames.len()
        ));
    }
    Ok(())
}

fn check_file(filename: &str, multi: &MultiOptions) -> Result<()> {
    let (path, stem) = resolve_stem(filename)?;
    let input = fs::read_to_string(&path).into_diagnostic()?;

    // Strip a shebang line like `run` does, so executable scripts check clean.
    // Blank the line rather than removing it: every span the parser produces is
    // a byte offset into this string, so dropping the line would shift every
    // diagnostic that follows it by the shebang's length and report the wrong
    // line. Overwriting with spaces keeps both byte offsets and line numbers
    // exact, and a whitespace-only first line is inert in every language we
    // parse.
    let input = if input.starts_with("#!") {
        let end = input.find('\n').unwrap_or(input.len());
        format!("{}{}", " ".repeat(end), &input[end..])
    } else {
        input
    };

    // Attach the source so span-carrying errors render the offending snippet,
    // exactly as `expand` does. Without this `check` reported bare byte offsets
    // ("source bytes 8..19") while `expand` rendered a caret under the source —
    // and `check` is the CI and pre-commit path, so it is the one a contributor
    // actually reads.
    let with_src =
        |e: miette::Report| e.with_source_code(NamedSource::new(filename, input.clone()));

    match multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, &stem);
            let sterm = multi.parse_chain(&chain, &input).map_err(with_src)?;
            multi.expand_lang(chain[0], &sterm).map_err(with_src)?;
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, &stem);
            let sterm = multi.parse_chain(&chain, &input).map_err(with_src)?;
            multi.expand_lang(chain[0], &sterm).map_err(with_src)?;
        }
    }
    Ok(())
}

/// Where a file's language chain comes from: the *resolved* file's own name,
/// with a `.quilt` suffix stripped if it has one. Returns the resolved path
/// alongside it, so the caller reads the same file the name was taken from.
///
/// Symlinks are followed, so an extension-less entry point (`bin/issues ->
/// ../examples/issue_triage.html.py.quilt`) derives its chain from the target's
/// name, and only the file *name* counts, so dots in a directory can't leak
/// into it.
///
/// `run` has always resolved names this way; `check` instead sliced `.quilt`
/// off the path it was handed and refused anything without it — so a script
/// that ships in a repo to be run could never be validated in CI (issue #188).
/// The two now share this, because `check` is documented as validating a file
/// "exactly as `expand` would", and two subcommands disagreeing about which
/// files exist is the defect. A name that resolves to no registered language
/// still fails, just later and with a message that names the language rather
/// than the suffix.
fn resolve_stem(filename: &str) -> Result<(std::path::PathBuf, String)> {
    let path = fs::canonicalize(filename).into_diagnostic()?;
    let stem = {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| miette!("invalid filename: {filename}"))?;
        name.strip_suffix(".quilt").unwrap_or(name).to_owned()
    };
    Ok((path, stem))
}

/// Derive the language chain from a `.quilt` file's stem (the name with the
/// `.quilt` suffix already stripped). Reading right-to-left, peel off each
/// extension that names a registered language: the rightmost is the host
/// (ground) language and the rest are the default languages for nested
/// un-annotated quotes — so `shaders.wgsl.rs` → `["rs", "wgsl"]` and the plain
/// `main.rs` → `["rs"]`. The basename never counts, even when it looks like a
/// language (`text.rs` → `["rs"]`). Always yields at least the last part (even
/// if it isn't a known language) so the downstream parse surfaces a clear
/// error, as it did before chains existed.
fn lang_chain<'a, LS: Languages, MS: MetaLanguages>(
    multi: &Multi<LS, MS>,
    stem: &'a str,
) -> Vec<&'a str> {
    let parts: Vec<&str> = stem.split('.').collect();
    let mut chain: Vec<&str> = parts[1..]
        .iter()
        .rev()
        .copied()
        .take_while(|part| multi.get_lang(part).is_ok())
        .collect();
    if chain.is_empty() {
        chain.push(parts.last().copied().unwrap_or(""));
    }
    chain
}

fn run(args: &RunArgs) -> Result<()> {
    let (input_path, base) = resolve_stem(&args.filename)?;
    let lang = base.split('.').next_back().unwrap();

    let input = fs::read_to_string(&input_path).into_diagnostic()?;

    // Strip shebang line so the language parser doesn't see `#!`
    let input = if input.starts_with("#!") {
        input.lines().skip(1).collect::<Vec<_>>().join("\n")
    } else {
        input
    };

    let temp_file = tempfile::Builder::new()
        .suffix(&format!(".{lang}"))
        .tempfile()
        .into_diagnostic()?;
    let mut path = temp_file.path().to_str().unwrap().to_string();

    // The chain travels with the run (as `$QUILT_CHAIN`, ground first) so a
    // runtime that re-invokes the expander on a *generated* stage — `↓` — can
    // expand it under the same defaults for un-annotated quotes.
    let (hashbang, chain_key) = match &args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, &base);
            (
                expand_to(&mut multi, &chain, &input, &path)?,
                chain.join("."),
            )
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, &base);
            (
                expand_to(&mut multi, &chain, &input, &path)?,
                chain.join("."),
            )
        }
    };
    tracing::debug!("expanded to: {path}");

    let hashbang =
        hashbang.ok_or_else(|| miette!("language '{lang}' is not runnable via 'quilt'"))?;
    // The interpreter is not always the last word: TypeScript's shebang is
    // `#!/usr/bin/env -S node --experimental-strip-types`, so taking the last
    // word executed the flag (issue #174). `parse_hashbang` unwraps `env` the way
    // `env -S` does and keeps the interpreter's own arguments.
    let (runner, runner_args) = quilt::lang::parse_hashbang(hashbang).ok_or_else(|| {
        miette!("language '{lang}' has a shebang naming no interpreter: {hashbang:?}")
    })?;
    let mut runner_cmd = std::process::Command::new(runner);
    runner_cmd.args(&runner_args);
    // Per-runner setup: each host needs its runtime importable, and each spells
    // that its own way — a cargo manifest, `PYTHONPATH`, a `node_modules`. Only
    // node needs a scratch directory to hold the latter, hence the `Option`.
    let node_dir = if runner.ends_with("rust-script") {
        // Embed a cargo manifest in the script so its operators resolve against
        // *this* quilt crate (so `quilt` works from any directory, not just
        // `rust/quilt`) with the matching feature set: `qlift`/`name` (Omni)
        // live under `rust`, `bs_*` under `bootstrap`.
        let quilt_dir = env!("CARGO_MANIFEST_DIR");
        let quilt_feature = match args.multi {
            MultiOptions::Omni => "rust",
            #[cfg(feature = "bootstrap")]
            MultiOptions::Bootstrap => "bootstrap",
        };
        prepend_cargo_manifest(
            &path,
            &[format!(
                "quilt = {{ path = \"{quilt_dir}\", package = \"quiltlang\", default-features = false, features = [\"{quilt_feature}\"] }}"
            )],
        )?;
        None
    } else if runner.ends_with("python3") || runner.ends_with("python") {
        // Make the `quilt_python` extension module (the runtime that expanded
        // .py.quilt files target) importable. It lives next to this crate; build
        // it with `bin/build-py`.
        let py_dir = format!("{}/../quilt-python", env!("CARGO_MANIFEST_DIR"));
        let pythonpath = match std::env::var("PYTHONPATH") {
            Ok(existing) if !existing.is_empty() => format!("{py_dir}:{existing}"),
            _ => py_dir,
        };
        runner_cmd.env("PYTHONPATH", pythonpath);
        // Hand the running expander's own path to the script so the runtime's
        // `expand`/`run` helpers can re-invoke `quilt expand` on generated
        // fragments that still contain Quilt glyphs (which plain-Python eval,
        // i.e. `reduce`/`.↓`, can't parse). `quilt` isn't necessarily on PATH.
        if let Ok(exe) = std::env::current_exe() {
            runner_cmd.env("QUILT", exe);
        }
        None
    } else if runner.ends_with("node") {
        // Node's ESM resolver ignores NODE_PATH, so the bare `import … from
        // "quilt"` an expanded .ts.quilt program carries resolves only against a
        // `node_modules` directory *above the script*. A bare temp file has
        // none, so `quilt run foo.ts.quilt` died at the import before it could
        // run anything — which is why `↓` was browser-only (issue #153).
        let dir = node_workspace()?;
        let script = dir.path().join(&base);
        fs::rename(&path, &script).into_diagnostic()?;
        path = script.to_string_lossy().into_owned();
        // As for python: hand the running expander's own path to the script, so
        // the runtime's `↓` can re-invoke `quilt expand` on a generated stage
        // that still contains Quilt glyphs. `quilt` isn't necessarily on PATH.
        if let Ok(exe) = std::env::current_exe() {
            runner_cmd.env("QUILT", exe);
        }
        runner_cmd.env("QUILT_CHAIN", &chain_key);
        Some(dir)
    } else {
        None
    };

    runner_cmd.arg(&path).args(&args.args);
    let cmd_str = std::iter::once(runner_cmd.get_program())
        .chain(runner_cmd.get_args())
        .map(|s| s.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!("running: {cmd_str}");
    let status = runner_cmd.status().into_diagnostic()?;

    // `process::exit` runs no destructors, so clean the scratch files up here
    // rather than leaving one temp file (and, for node, a directory) per run.
    drop(temp_file);
    drop(node_dir);
    std::process::exit(status.code().unwrap_or(1));
}

/// A private directory for a Node run, holding a `node_modules` with the two
/// packages the browser demos bind in their import map: `quilt` — the
/// reduce-enabled runtime (`quilt-wasm/node`), which is what supplies `↓` — and
/// `quilt-wasm`, the raw wasm-pack package. The script is moved in beside it, so
/// Node's ordinary "walk up looking for `node_modules`" resolution finds both.
///
/// Both live next to this crate and are built by `bin/build-ts`; a missing build
/// is reported here rather than as a Node module-resolution stack trace.
fn node_workspace() -> Result<tempfile::TempDir> {
    let quilt_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let runtime = quilt_dir.join("../quilt-wasm/node");
    let wasm = quilt_dir.join("../quilt-wasm/pkg");
    if !wasm.join("quilt_wasm.js").exists() {
        return Err(miette!(
            "the quilt-wasm runtime is not built for Node ({}) — run `bin/build-ts` \
             and try again",
            wasm.display()
        ));
    }

    let dir = tempfile::tempdir().into_diagnostic()?;
    let modules = dir.path().join("node_modules");
    fs::create_dir_all(&modules).into_diagnostic()?;
    symlink_dir(&runtime, &modules.join("quilt"))?;
    symlink_dir(&wasm, &modules.join("quilt-wasm"))?;
    Ok(dir)
}

/// Symlink a directory, the one filesystem call whose spelling differs by
/// platform. Node resolves a symlinked package to its real path, so each
/// package's own relative imports keep working.
fn symlink_dir(target: &std::path::Path, link: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    let r = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let r = std::os::windows::fs::symlink_dir(target, link);
    #[cfg(not(any(unix, windows)))]
    let r: std::io::Result<()> = Err(std::io::Error::other("symlinks are unsupported here"));
    r.into_diagnostic().map_err(|e| {
        e.context(format!(
            "linking {} -> {}",
            link.display(),
            target.display()
        ))
    })
}

/// Prepend a rust-script cargo manifest (a `//! ```cargo` doc-comment block)
/// declaring `deps` to the script at `path`, so rust-script builds it with
/// those dependencies instead of needing `-d` command-line args.
fn prepend_cargo_manifest(path: &str, deps: &[String]) -> Result<()> {
    let mut manifest = String::from("//! ```cargo\n//! [dependencies]\n");
    for dep in deps {
        manifest.push_str("//! ");
        manifest.push_str(dep);
        manifest.push('\n');
    }
    manifest.push_str("//! ```\n\n");
    let body = fs::read_to_string(path).into_diagnostic()?;
    fs::write(path, format!("{manifest}{body}")).into_diagnostic()?;
    Ok(())
}

fn expand_to<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    chain: &[&str],
    input: &str,
    path: &str,
) -> Result<Option<&'static str>> {
    let host = chain[0];
    let hashbang = multi.get_lang(host)?.hashbang();
    // attach the source so span-carrying errors render the offending snippet
    let with_src = |e: miette::Report| e.with_source_code(input.to_string());
    let sterm = multi.parse_chain(chain, input).map_err(with_src)?;
    multi
        .expand_lang(host, &sterm)
        .map_err(with_src)?
        .dump(path)?;
    Ok(hashbang)
}

/// The line-comment introducer for the `DO NOT EDIT` header, chosen from the
/// generated file's extension so the header is valid in the language we just
/// generated.
///
/// The table itself lives in `quilt::langs::comment_prefix`, beside the
/// language modules rather than here: as a hardcoded match in the CLI it was
/// disconnected from the language registry, so a new host silently inherited
/// Rust's `//!` — issue #136. Anything unrecognised still falls back to `//!`,
/// preserving the previous behaviour for extensions that name no language.
fn header_comment(filename: &str) -> &'static str {
    std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(quilt::langs::comment_prefix)
        .unwrap_or("//!")
}

fn generate(filename: &str, x: &Arc<QTerm>) -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>()[1..].join(" ");
    let header = format!(
        "{} DO NOT EDIT. GENERATED BY `quilt {args}`.",
        header_comment(filename)
    );
    x.dump_with_cmds(filename, &[write(&header), NL, NL], &[])
}

// --- Expand cache -----------------------------------------------------------
//
// File-based cache for the expanded QTerm, keyed by (canonical path, mtime,
// multi variant, binary version, binary mtime).  Invalidation is trivial
// because .quilt files have no transitive imports.  The binary mtime ensures
// the cache is discarded on every `cargo build`, so changes to MetaLanguage
// or Language implementations are never silently ignored.  Cache misses are
// silent: we just fall back to a full parse+expand.

/// Mtime of the running executable, as (secs, nanos) since `UNIX_EPOCH`.
/// Returns (0, 0) if unavailable (e.g. proc-replaced binaries or unusual fs).
fn binary_mtime() -> (u64, u32) {
    std::env::current_exe()
        .ok()
        .and_then(|p| fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or((0, 0), |d| (d.as_secs(), d.subsec_nanos()))
}

fn cache_hash(path: &str, mtime_secs: u64, mtime_nanos: u32, multi: &str) -> u64 {
    let (bin_secs, bin_nanos) = binary_mtime();
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    mtime_secs.hash(&mut h);
    mtime_nanos.hash(&mut h);
    multi.hash(&mut h);
    env!("CARGO_PKG_VERSION").hash(&mut h);
    bin_secs.hash(&mut h);
    bin_nanos.hash(&mut h);
    h.finish()
}

fn cache_dir() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("XDG_CACHE_HOME") {
        return Some(std::path::PathBuf::from(p).join("quilt"));
    }
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".cache").join("quilt"))
}

fn cache_load(path: &str, mtime_secs: u64, mtime_nanos: u32, multi: &str) -> Option<Arc<QTerm>> {
    let dir = cache_dir()?;
    let hash = cache_hash(path, mtime_secs, mtime_nanos, multi);
    let file = dir.join(format!("{hash:016x}.postcard"));
    let bytes = fs::read(file).ok()?;
    postcard::from_bytes(&bytes).ok()
}

fn cache_store(path: &str, mtime_secs: u64, mtime_nanos: u32, multi: &str, term: &Arc<QTerm>) {
    let Some(dir) = cache_dir() else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let hash = cache_hash(path, mtime_secs, mtime_nanos, multi);
    let file = dir.join(format!("{hash:016x}.postcard"));
    if let Ok(bytes) = postcard::to_stdvec(term.as_ref()) {
        let _ = fs::write(file, bytes);
    }
}
