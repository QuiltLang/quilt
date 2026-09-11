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
    /// Interactive machine session: each line is Quilt source, expanded and
    /// fed to the ground language's default machine
    Repl(ReplArgs),
    /// Run a notebook: an .html.quilt page whose quoted cells run on the
    /// machines of their languages, rendered back into the page
    Notebook(NotebookArgs),
    /// Clear the expand cache
    Clean,
}

#[derive(Args, Debug)]
struct NotebookArgs {
    /// .html.quilt file to run
    filename: String,
    /// Where to write the rendered page (default: the input name without
    /// `.quilt`)
    #[clap(short, long)]
    out: Option<String>,
    /// Write the rendered page to stdout instead of a file
    #[clap(long)]
    stdout: bool,
    /// Open the rendered page in the default browser
    #[clap(long)]
    open: bool,
    /// Exit non-zero if any cell failed (the page is still written)
    #[clap(long)]
    strict: bool,
}

#[derive(Args, Debug)]
struct ExpandArgs {
    /// file to expand
    #[clap(index = 1)]
    filename: String,
    /// multi-language to use
    #[clap(short, long, default_value_t, value_enum)]
    multi: MultiOptions,
    /// Open the generated file with this line instead of the host's runtime
    /// import (e.g. `--prelude 'use crate::prelude::*;'`)
    #[clap(long, value_name = "LINE", conflicts_with = "no_prelude")]
    prelude: Option<String>,
    /// Generate no runtime import, even where the host has one
    #[clap(long)]
    no_prelude: bool,
}

/// How the generated file's runtime import is chosen (issue #274). The default
/// — neither flag — asks the host's meta-language, which is right whenever the
/// runtime is the published one.
///
/// The escape hatch exists because the import path is not universal. Code
/// generated *inside this repo* wants `use crate::prelude::*`, and a runtime
/// that re-invokes the expander on a generated stage (quilt-python's `expand`,
/// quilt-wasm's) wants no import at all — it evaluates the result in a scope
/// that already holds the runtime, and in TypeScript's case in a `node:vm`
/// script, where an ESM `import` is a syntax error.
///
/// The flags are repeated on `expand` and `run` rather than shared through one
/// flattened `Args`: `RunArgs` is itself flattened into `Cli` (as an `Option`,
/// which is what makes `run` the default subcommand and the
/// `#!/usr/bin/env quilt` shebang work), and clap cannot see a *nested* flatten
/// through that — `quilt <script>` stopped meaning `quilt run <script>` and
/// printed help instead.
#[derive(Clone, Copy, Default)]
struct PreludeOpts<'a> {
    text: Option<&'a str>,
    off: bool,
}

impl<'a> PreludeOpts<'a> {
    fn new(text: Option<&'a String>, off: bool) -> Self {
        PreludeOpts {
            text: text.map(String::as_str),
            off,
        }
    }
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
struct ReplArgs {
    /// Language chain, as in a file stem: `py`, or `wgsl.py` (rightmost is
    /// the ground language — the one the machine speaks)
    #[clap(default_value = "py")]
    chain: String,
}

#[derive(Args, Debug)]
struct RunArgs {
    /// .quilt file to run
    filename: String,
    /// multi-language to use
    #[clap(short, long, default_value_t, value_enum)]
    multi: MultiOptions,
    /// Open the generated file with this line instead of the host's runtime
    /// import (e.g. `--prelude 'use crate::prelude::*;'`)
    #[clap(long, value_name = "LINE", conflicts_with = "no_prelude")]
    prelude: Option<String>,
    /// Generate no runtime import, even where the host has one
    #[clap(long)]
    no_prelude: bool,
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
        (Some(Commands::Repl(args)), _) => repl(args),
        (Some(Commands::Notebook(args)), _) => notebook(args),
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

    // An HTML-ground file is a notebook, and its expansion proper is the
    // identity (`langs::html::meta`) — the cells held, nothing run. The
    // artifact anyone wants from `expand` is the *rendered* notebook, so
    // that is what is written; `check` is where the identity expansion
    // does its job (validating every cell without running one). Never
    // cached: a run is not a function of the source alone.
    if matches!(args.multi, MultiOptions::Omni) && ground_is_html(output_filename) {
        return notebook(&NotebookArgs {
            filename: input_filename.clone(),
            out: Some(output_filename.to_string()),
            stdout: false,
            open: false,
            strict: false,
        });
    }

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

    let cached = cache_load(&path_key, mtime_secs, mtime_nanos, multi_key);
    let hit = cached.is_some();

    // Read even on a cache hit: the *term* is what expansion is expensive for,
    // and the prelude is decided from the source (does it use any construct?
    // does it already import the runtime? which languages does it quote?), so
    // the file is needed either way. The read is a rounding error next to the
    // parse the cache exists to skip.
    let input = fs::read_to_string(input_filename).expect("Should have been able to read the file");
    // attach the source so span-carrying errors render the offending snippet
    let with_src =
        |e: miette::Report| e.with_source_code(NamedSource::new(input_filename, input.clone()));
    let opts = PreludeOpts::new(args.prelude.as_ref(), args.no_prelude);
    let (expanded, prelude) = match args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, output_filename);
            let expanded = if let Some(cached) = cached {
                cached
            } else {
                let sterm = multi.parse_chain(&chain, &input).map_err(with_src)?;
                multi.expand_lang(chain[0], &sterm).map_err(with_src)?
            };
            let prelude = prelude_for(&multi, &chain, &input, opts);
            (expanded, prelude)
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, output_filename);
            let expanded = if let Some(cached) = cached {
                cached
            } else {
                let sterm = multi.parse_chain(&chain, &input).map_err(with_src)?;
                multi.expand_lang(chain[0], &sterm).map_err(with_src)?
            };
            let prelude = prelude_for(&multi, &chain, &input, opts);
            (expanded, prelude)
        }
    };

    if !hit {
        cache_store(&path_key, mtime_secs, mtime_nanos, multi_key, &expanded);
    }
    generate(output_filename, &expanded, prelude.as_deref())
}

/// The runtime import the file expanded from `input` should open with, or
/// `None` (issue #274).
///
/// `None` in four cases: `--no-prelude`; a host whose meta needs none (nix,
/// lean and text generate no runtime calls at all); and the two rules that keep
/// the injection from becoming a footgun:
///
/// * **The source uses no Quilt construct.** `examples/hello.rs.quilt` expands
///   to itself and must stay import-free: Rust tolerates an unused glob
///   silently, but a Python or TypeScript linter will not.
/// * **The author already imported the runtime.** Two identical globs are
///   harmless in Rust and Python, but a doubled *named* import is a hard error
///   in TypeScript — so this is checked rather than assumed. See
///   [`Prelude::present_in`](quilt::meta::Prelude::present_in).
///
/// `targets` — which only a target-directed host like TypeScript reads — is the
/// language chain plus every annotation the source's quotes name, so that a
/// `sql↖…↗` inside a `.rs.quilt` counts even though the chain never mentions
/// SQL. It over-approximates: a language listed but never lifted into costs an
/// unused name, one missing costs a name that is not in scope.
fn prelude_for<LS: Languages, MS: MetaLanguages>(
    multi: &Multi<LS, MS>,
    chain: &[&str],
    input: &str,
    opts: PreludeOpts<'_>,
) -> Option<String> {
    if opts.off {
        return None;
    }
    let used = quilt::node::constructs(input);
    if !used.any {
        return None;
    }
    if let Some(text) = opts.text {
        return (!input.contains(text.trim())).then(|| text.to_owned());
    }
    let mut targets: Vec<&str> = chain.to_vec();
    for anno in &used.annos {
        if !targets.contains(&&**anno) {
            targets.push(anno);
        }
    }
    // A language with no meta is not an error here — the expansion that just
    // succeeded proves the host is fine — so a failed lookup is simply "no
    // prelude".
    let prelude = multi.prelude(chain[0], &targets).ok().flatten()?;
    (!prelude.present_in(input)).then(|| prelude.text.into_owned())
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

/// `quilt repl` (docs/design/machines.md, phase 4): a REPL is the ground
/// machine with the expander in front. Each line is Quilt source: parsed
/// with the chain, expanded by the host's meta-language, classified, and fed
/// to the host's default machine from the park — so definitions persist
/// across lines exactly as they persist across reduces.
fn repl(args: &ReplArgs) -> Result<()> {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};

    let mut multi = Omni::default();
    let stem = format!("repl.{}", args.chain);
    let chain = lang_chain(&multi, &stem);
    let host = chain[0];
    if host == "html" {
        return repl_html(&mut multi);
    }
    // Spawn eagerly, so "this language has no machine" is the first line out
    // rather than a surprise after the first input.
    multi.machine(host)?;

    let stdin = std::io::stdin();
    let tty = stdin.is_terminal();
    if tty {
        eprintln!("quilt repl — ground language {host}; ctrl-D to exit");
    }
    let mut lines = stdin.lock().lines();
    loop {
        if tty {
            eprint!("{host}> ");
            let _ = std::io::stderr().flush();
        }
        let Some(line) = lines.next() else { break };
        let line = line.into_diagnostic()?;
        if line.trim().is_empty() {
            continue;
        }
        // An error ends the line, not the session.
        match repl_line(&mut multi, &chain, &line) {
            Ok(Some(out)) => println!("{out}"),
            Ok(None) => {}
            Err(e) => eprintln!("{e:?}"),
        }
    }
    Ok(())
}

/// `quilt repl html`: a notebook session, one fragment per line. Markup
/// defines (by id) or appends to the page; a quote — `py↖x = 5↗`,
/// `sql↖SELECT 1;↗` — is a cell, run on its language's machine, so this is
/// the polyglot REPL: each line names its language, and every language's
/// definitions persist. `#id` on its own reads an element back. The page
/// is printed at the end of the session.
#[cfg(feature = "html")]
fn repl_html(multi: &mut Omni) -> Result<()> {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};

    let mut nb = quilt::notebook::Notebook::new(multi);
    let stdin = std::io::stdin();
    let tty = stdin.is_terminal();
    if tty {
        eprintln!(
            "quilt repl — the page is the machine: markup defines, `lang↖…↗` runs a cell, \
             `#id` reads an element; ctrl-D prints the page"
        );
    }
    let mut lines = stdin.lock().lines();
    loop {
        if tty {
            eprint!("html> ");
            let _ = std::io::stderr().flush();
        }
        let Some(line) = lines.next() else { break };
        let line = line.into_diagnostic()?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(id) = trimmed.strip_prefix('#') {
            match nb.page().find(id) {
                Some(el) => println!("{}", el.coparse()),
                None => eprintln!("no element with id {id:?}"),
            }
            continue;
        }
        match nb.feed_source(&line) {
            Ok(ids) => {
                for cell in nb.cells().iter().filter(|c| ids.contains(&c.id)) {
                    if let Some(error) = &cell.error {
                        eprintln!("{error}");
                        continue;
                    }
                    let err = cell.stderr.trim();
                    if !err.is_empty() {
                        eprintln!("{err}");
                    }
                    let out = cell.stdout.trim();
                    if !out.is_empty() {
                        println!("{out}");
                    }
                    if let Some(value) = &cell.value {
                        if out.lines().next_back() != Some(&**value)
                            && !quilt::notebook::NO_VALUE.contains(&value.trim())
                        {
                            println!("{value}");
                        }
                    }
                }
            }
            Err(e) => eprintln!("{e:?}"),
        }
    }
    if tty {
        eprintln!();
    }
    println!("{}", nb.render());
    Ok(())
}

/// Whether a file stem's ground language is HTML — i.e. the file is a
/// notebook. Asked before the expand cache is consulted, so it builds its
/// own registry.
#[cfg(feature = "html")]
fn ground_is_html(stem: &str) -> bool {
    let multi = Omni::default();
    lang_chain(&multi, stem)[0] == "html"
}

/// `quilt notebook`: run an `.html.quilt` page's cells on their languages'
/// machines and write the page back with the results in it — see
/// `quilt::notebook`. `expand` and `run` on an HTML-ground file come here
/// too; `check` does not, which is what makes it safe.
#[cfg(feature = "html")]
fn notebook(args: &NotebookArgs) -> Result<()> {
    let (path, stem) = resolve_stem(&args.filename)?;
    let input = fs::read_to_string(&path).into_diagnostic()?;
    // Blank a shebang line rather than dropping it, as `check` does, so the
    // spans in cell diagnostics stay exact.
    let input = if input.starts_with("#!") {
        let end = input.find('\n').unwrap_or(input.len());
        format!("{}{}", " ".repeat(end), &input[end..])
    } else {
        input
    };

    let mut multi = Omni::default();
    let chain = lang_chain(&multi, &stem);
    if chain[0] != "html" {
        return Err(miette!(
            "a notebook is an .html.quilt file — its ground language is HTML — but {} has \
             ground language {:?}",
            args.filename,
            chain[0]
        ));
    }
    let with_src = |e: miette::Report| {
        e.with_source_code(NamedSource::new(args.filename.clone(), input.clone()))
    };
    let rendered = quilt::notebook::run(&mut multi, &input).map_err(with_src)?;

    let failures = rendered.failures();
    for cell in &failures {
        let first = cell
            .error
            .as_deref()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("");
        eprintln!("cell {} [{}] failed: {first}", cell.id, cell.lang);
    }
    let cli_args = std::env::args().collect::<Vec<_>>()[1..].join(" ");
    let html = format!(
        "<!-- DO NOT EDIT. GENERATED BY `quilt {cli_args}`. -->\n{}\n",
        rendered.html
    );
    if args.stdout {
        print!("{html}");
    } else {
        let out = args.out.clone().unwrap_or_else(|| {
            args.filename
                .strip_suffix(".quilt")
                .map_or_else(|| format!("{}.html", args.filename), str::to_string)
        });
        fs::write(&out, html).into_diagnostic()?;
        eprintln!(
            "wrote {out} ({} cell(s), {} failed)",
            rendered.cells.len(),
            failures.len()
        );
        if args.open {
            open_in_browser(&out)?;
        }
    }
    if args.strict && !failures.is_empty() {
        return Err(miette!("{} cell(s) failed", failures.len()));
    }
    Ok(())
}

/// Hand a file to the platform's opener.
#[cfg(feature = "html")]
fn open_in_browser(path: &str) -> Result<()> {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(windows) {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    std::process::Command::new(program)
        .args(args)
        .arg(path)
        .spawn()
        .into_diagnostic()
        .map_err(|e| e.context(format!("opening {path} with {program}")))?;
    Ok(())
}

// Without the `html` feature (the bootstrap build, for one) there is no
// notebook: the subcommand stays listed and says so when invoked, and an
// HTML-ground file is nothing `expand`/`run` recognise.
#[cfg(not(feature = "html"))]
fn without_html(what: &str) -> miette::Report {
    miette!("{what} needs quilt built with the `html` feature")
}

#[cfg(not(feature = "html"))]
fn repl_html(_multi: &mut Omni) -> Result<()> {
    Err(without_html("`quilt repl html`"))
}

#[cfg(not(feature = "html"))]
fn ground_is_html(_stem: &str) -> bool {
    false
}

#[cfg(not(feature = "html"))]
fn notebook(_args: &NotebookArgs) -> Result<()> {
    Err(without_html("`quilt notebook`"))
}

/// One REPL turn: parse, expand, classify, feed. `Some` is the text to show —
/// a query's answered literal, else whatever the feed printed.
fn repl_line(multi: &mut Omni, chain: &[&str], line: &str) -> Result<Option<String>> {
    let host = chain[0];
    let term = multi.parse_chain(chain, line)?;
    let expanded = multi.expand_lang(host, &term)?;
    let kind = multi.classify_for_feed(host, &expanded)?;
    let answer = multi.machine(host)?.feed(kind, &expanded)?;
    // What the feed said on stderr is worth seeing even when it succeeded —
    // a warning, say — and the persistent kernels frame it per feed.
    let err = answer.stderr.trim();
    if !err.is_empty() {
        eprintln!("{err}");
    }
    if let Some(value) = answer.value {
        return Ok(Some(value.into()));
    }
    let out = answer.stdout.trim();
    Ok((!out.is_empty()).then(|| out.to_string()))
}

fn run(args: &RunArgs) -> Result<()> {
    let (input_path, base) = resolve_stem(&args.filename)?;
    let lang = base.split('.').next_back().unwrap();

    // Running a notebook is rendering it: the page goes to stdout, the way a
    // program's output would, so `./notes.html.quilt > notes.html` works
    // under a `#!/usr/bin/env quilt` shebang.
    if lang == "html" && matches!(args.multi, MultiOptions::Omni) {
        if !args.args.is_empty() {
            return Err(miette!(
                "a notebook takes no arguments (got {:?})",
                args.args
            ));
        }
        return notebook(&NotebookArgs {
            filename: args.filename.clone(),
            out: None,
            stdout: true,
            open: false,
            strict: false,
        });
    }

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
    let opts = PreludeOpts::new(args.prelude.as_ref(), args.no_prelude);
    let (hashbang, chain_key) = match &args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, &base);
            (
                expand_to(&mut multi, &chain, &input, &path, opts)?,
                chain.join("."),
            )
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, &base);
            (
                expand_to(&mut multi, &chain, &input, &path, opts)?,
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
    opts: PreludeOpts<'_>,
) -> Result<Option<&'static str>> {
    let host = chain[0];
    let hashbang = multi.get_lang(host)?.hashbang();
    // attach the source so span-carrying errors render the offending snippet
    let with_src = |e: miette::Report| e.with_source_code(input.to_string());
    let sterm = multi.parse_chain(chain, input).map_err(with_src)?;
    let body = multi.expand_lang(host, &sterm).map_err(with_src)?.coparse();
    // This is the injection that matters most: the temp file is handed straight
    // to an interpreter, so a missing runtime import is a failed *run*, not a
    // stale artifact someone notices later.
    //
    // No `strip_shebang` here: `run` takes the shebang off the *input* before
    // expanding, since the runner comes from the language rather than from what
    // the script's own `#!` line names.
    let body = match prelude_for(multi, chain, input, opts) {
        Some(prelude) => with_prelude(&body, &prelude, line_comment(host)),
        None => body,
    };
    fs::write(path, body).into_diagnostic()?;
    Ok(hashbang)
}

/// The comment introducer for the `DO NOT EDIT` header, asked of the language
/// the generated file's extension names, so the header is valid in the language
/// we just generated.
///
/// The extension is the language name here — `.py`, `.rs`, `.lean` are exactly
/// the registry's aliases, and `generate` runs on the expand-cache-hit path,
/// before any `Multi` exists to ask a better question of. The answer itself
/// comes from the language (`Comments::HEADER`, via the registry): as a
/// hardcoded match in the CLI it was disconnected from the language impls, so a
/// new host silently inherited Rust's `//!` — issues #136, #194.
///
/// Anything the registry does not recognise still falls back to `//!`,
/// preserving the previous behaviour for extensions that name no language.
fn header_comment(filename: &str) -> &'static str {
    std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(quilt::langs::header_comment)
        .unwrap_or("//!")
}

/// Write the expanded term to `filename`, behind the `DO NOT EDIT` header and
/// — when the host has one and the source did not write it — the runtime import
/// the generated code calls into (issue #274).
///
/// The prelude is written *here*, into the file, rather than built into the
/// term: it is a property of the artifact, not of the expansion. Keeping it out
/// of the term is what leaves `quilt check` a pure parse-and-expand and the
/// expander snapshots (issue #157) unchanged by this feature. The shebang the
/// source carried is dropped for the mirror-image reason — see
/// [`strip_shebang`].
fn generate(filename: &str, x: &Arc<QTerm>, prelude: Option<&str>) -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>()[1..].join(" ");
    let header = format!(
        "{} DO NOT EDIT. GENERATED BY `quilt {args}`.",
        header_comment(filename)
    );
    let lang = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let body = x.coparse();
    let body = strip_shebang(&body);
    let body = match prelude {
        Some(prelude) => with_prelude(body, prelude, line_comment(lang)),
        None => body.to_owned(),
    };
    fs::write(filename, format!("{header}\n\n{body}")).into_diagnostic()
}

/// `body` with the shebang the `.quilt` source carried taken off.
///
/// A `#!` line in a `.quilt` file says how to run *that file*: it is what makes
/// `./examples/countdown.rs.quilt` and the extension-less `bin/issues` work.
/// It says nothing about the artifact, and `run` already ignores it — the
/// interpreter comes from [`Language::hashbang`](quilt::lang::Language::hashbang),
/// the *language's* answer, not from whatever the script's own line names — so
/// `run` strips it before expanding and `check` blanks it. Only `expand` kept
/// it, where it was never a shebang in the first place: the `DO NOT EDIT`
/// header goes on line 1, so the copied `#!` landed on line 3 and meant
/// nothing. In Rust it was worse than nothing — `#!` there is inner-attribute
/// syntax, so every generated file that came from an executable script failed
/// to parse (`error[E0753]`).
///
/// The blank line that separated the shebang from the code goes with it. An
/// inner attribute is *not* a shebang, by exactly the rule rustc uses: `#!` is
/// a shebang only when the next character is not `[`.
fn strip_shebang(body: &str) -> &str {
    let Some(rest) = body.strip_prefix("#!") else {
        return body;
    };
    if rest.starts_with('[') {
        return body;
    }
    let rest = rest.find('\n').map_or("", |i| &rest[i + 1..]);
    rest.strip_prefix('\n').unwrap_or(rest)
}

/// The line-comment introducer for `lang`, asked of the registry the same way
/// [`header_comment`] asks for the header's. Used to recognise the leading
/// comment header an inserted prelude has to go *after*; `//` for anything the
/// registry does not know, matching [`header_comment`]'s fallback.
fn line_comment(lang: &str) -> &'static str {
    quilt::langs::line_comment(lang).unwrap_or("//")
}

/// `body` opened with `prelude`, placed after whatever has to stay at the top
/// (see [`prelude_offset`]).
fn with_prelude(body: &str, prelude: &str, line_comment: &str) -> String {
    let at = prelude_offset(body, line_comment);
    let (before, after) = body.split_at(at);
    if after.is_empty() {
        format!("{before}{prelude}\n")
    } else {
        format!("{before}{prelude}\n\n{after}")
    }
}

/// The byte offset in `body` at which a runtime import can be inserted: after
/// the leading header — blank lines, whole-line comments, a shebang and Rust's
/// inner attributes — and before the first item.
///
/// Not cosmetic. Rust's `//!` doc comments and `#![…]` attributes are *inner*,
/// and inner means "before any item", so an import written above a file's `//!`
/// header turns the header itself into a syntax error (`expected outer doc
/// comment`) — which is what nine of this repo's own examples do. Python and
/// TypeScript are laxer, but a header is what a reader expects first there too,
/// and a `#!` shebang is only a shebang on line 1.
///
/// A line comment is recognised by `line_comment`, the ground language's own
/// introducer, so `//!` and `//` both count for Rust and `#`-anything for
/// Python (its shebang included).
fn prelude_offset(body: &str, line_comment: &str) -> usize {
    let mut at = 0;
    let mut rest = body;
    while !rest.is_empty() {
        let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let line = rest[..line_end].trim_end();
        let take = if line.is_empty() || line.starts_with(line_comment) {
            line_end
        } else if line.starts_with("#![") {
            // An inner attribute, which may wrap across lines: take it whole,
            // by balancing its brackets. Splitting one is worse than placing
            // the import above it.
            match attribute_end(rest) {
                Some(end) => end,
                None => break,
            }
        } else if at == 0 && line.starts_with("#!") {
            line_end // a shebang, which is only one on the first line
        } else {
            break;
        };
        at += take;
        rest = &rest[take..];
    }
    at
}

/// The byte length of the `#![…]` inner attribute `s` starts with, counting to
/// the `]` that balances its `[` (so a nested `[..]` inside does not end it) and
/// through the newline after it. `None` if the brackets never balance.
fn attribute_end(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    let after = i + c.len_utf8();
                    return Some(s[after..].find('\n').map_or(s.len(), |j| after + j + 1));
                }
            }
            _ => {}
        }
    }
    None
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

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    /// The REPL rides the park: a definition fed by one line answers a
    /// query on a later line. Plain Python only — the quilt runtime module
    /// is a `bin/build-py` artifact the unit suite must not require.
    #[test]
    fn repl_lines_share_the_machine() {
        let mut multi = Omni::default();
        assert_eq!(repl_line(&mut multi, &["py"], "x = 5").unwrap(), None);
        assert_eq!(
            repl_line(&mut multi, &["py"], "x + 2").unwrap().as_deref(),
            Some("7")
        );
    }

    /// The injected runtime import goes *after* the file's header, because
    /// Rust's `//!` and `#![…]` are inner and inner means "before any item" —
    /// an import above them is a syntax error, not a style choice.
    #[test]
    fn a_prelude_goes_after_the_header() {
        // Each case names where the first *item* starts, so the expected
        // offset is read off the fixture rather than counted by hand.
        let at = |body: &str, comment: &str, item: &str| {
            assert_eq!(
                prelude_offset(body, comment),
                body.find(item).unwrap(),
                "{body:?}"
            );
        };
        // Nothing to skip.
        at("fn main() {}\n", "//", "fn");
        // A doc header, its trailing blank line included.
        at("//! docs\n//! more\n\nfn main() {}\n", "//", "fn");
        // A shebang, and only on the first line.
        at("#!/usr/bin/env quilt\nfn main() {}\n", "//", "fn");
        at("fn main() {}\n#!/usr/bin/env quilt\n", "//", "fn");
        // An inner attribute is taken whole, brackets balanced across lines.
        at("#![allow(dead_code)]\nfn f() {}\n", "//", "fn");
        at("#![allow(\n    dead_code\n)]\nfn f() {}\n", "//", "fn");
        // An unbalanced one stops the scan rather than splitting it.
        at("#![allow(\nfn f() {}\n", "//", "#![");
        // Python's introducer covers its shebang and its comments alike.
        at("#!/usr/bin/env quilt\n# hi\nx = 1\n", "#", "x =");
    }

    /// The `.quilt` script's own shebang does not travel into the artifact,
    /// where it would not be on line 1 and so would not be a shebang — but an
    /// inner attribute, which is spelled the same way up to one character, does.
    #[test]
    fn a_shebang_is_dropped_and_an_attribute_is_not() {
        assert_eq!(
            strip_shebang("#!/usr/bin/env quilt\n\nfn main() {}\n"),
            "fn main() {}\n"
        );
        // Without the blank line, and with nothing after it at all.
        assert_eq!(strip_shebang("#!/usr/bin/env quilt\nx = 1\n"), "x = 1\n");
        assert_eq!(strip_shebang("#!/usr/bin/env quilt\n"), "");
        assert_eq!(strip_shebang("#!/usr/bin/env quilt"), "");
        // `#!` is a shebang only when the next character is not `[`.
        let attr = "#![allow(dead_code)]\nfn f() {}\n";
        assert_eq!(strip_shebang(attr), attr);
        // …and a file that never had one is untouched.
        let plain = "fn main() {}\n#!/usr/bin/env quilt\n";
        assert_eq!(strip_shebang(plain), plain);
    }

    /// A bad line reports and leaves the session usable.
    #[test]
    fn repl_errors_do_not_poison_the_session() {
        let mut multi = Omni::default();
        assert!(repl_line(&mut multi, &["py"], "1 +").is_err());
        assert_eq!(
            repl_line(&mut multi, &["py"], "20 + 22")
                .unwrap()
                .as_deref(),
            Some("42")
        );
    }
}
