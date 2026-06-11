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
use std::hash::{Hash, Hasher};
use std::fs;

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
        (None, None) => {
            use clap::CommandFactory;
            Cli::command().print_help().into_diagnostic()?;
            std::process::exit(2);
        }
    }
}

fn expand(args: &ExpandArgs) -> Result<()> {
    let input_filename = &args.filename;
    let output_filename = input_filename.strip_suffix(".quilt").unwrap();

    let canonical = fs::canonicalize(input_filename)
        .unwrap_or_else(|_| input_filename.into());
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
    let stem = filename
        .strip_suffix(".quilt")
        .ok_or_else(|| miette!("expected a .quilt file"))?;
    let input = fs::read_to_string(filename).into_diagnostic()?;

    // Strip a shebang line like `run` does, so executable scripts check clean
    let input = if input.starts_with("#!") {
        input.lines().skip(1).collect::<Vec<_>>().join("\n")
    } else {
        input
    };

    match multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, stem);
            let sterm = multi.parse_chain(&chain, &input)?;
            multi.expand_lang(chain[0], &sterm)?;
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, stem);
            let sterm = multi.parse_chain(&chain, &input)?;
            multi.expand_lang(chain[0], &sterm)?;
        }
    }
    Ok(())
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
    // Resolve symlinks so an extension-less entry point (`bin/issues ->
    // ../examples/issue_triage.html.py.quilt`) derives the language chain from
    // the target's name, and use only the file name so dots in directories
    // can't leak into it.
    let input_path = fs::canonicalize(&args.filename).into_diagnostic()?;
    let file_name = input_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| miette!("invalid filename: {}", args.filename))?;
    let base = file_name.strip_suffix(".quilt").unwrap_or(file_name);
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
    let path = temp_file.path().to_str().unwrap().to_string();

    let hashbang = match &args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, base);
            expand_to(&mut multi, &chain, &input, &path)?
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, base);
            expand_to(&mut multi, &chain, &input, &path)?
        }
    };
    tracing::debug!("expanded to: {path}");

    let hashbang =
        hashbang.ok_or_else(|| miette!("language '{lang}' is not runnable via 'quilt'"))?;
    let runner = hashbang
        .trim_start_matches("#!")
        .split_whitespace()
        .next_back()
        .unwrap();
    let mut runner_cmd = std::process::Command::new(runner);
    if runner.ends_with("rust-script") {
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
                "quilt = {{ path = \"{quilt_dir}\", default-features = false, features = [\"{quilt_feature}\"] }}"
            )],
        )?;
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
    }

    runner_cmd.arg(&path).args(&args.args);
    let cmd_str = std::iter::once(runner_cmd.get_program())
        .chain(runner_cmd.get_args())
        .map(|s| s.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!("running: {cmd_str}");
    let status = runner_cmd.status().into_diagnostic()?;

    std::process::exit(status.code().unwrap_or(1));
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

fn generate(filename: &str, x: &Arc<QTerm>) -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>()[1..].join(" ");
    let header = format!("//! DO NOT EDIT. GENERATED BY `quilt {args}`.");
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
