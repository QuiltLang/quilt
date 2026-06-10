use clap::{Args, Parser, Subcommand, ValueEnum};
use miette::IntoDiagnostic;
#[cfg(feature = "bootstrap")]
use quilt::langs::bootstrap::Bootstrap;
use quilt::{
    lang::Language,
    langs::omni::Omni,
    multi::{Languages, MetaLanguages, Multi},
    prelude::*,
    term::STerm,
};
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
    let input = fs::read_to_string(input_filename).expect("Should have been able to read the file");
    let expanded = match args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, output_filename);
            let sterm = multi.parse_chain(&chain, &input)?;
            multi.expand_lang(chain[0], &sterm)?
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, output_filename);
            let sterm = multi.parse_chain(&chain, &input)?;
            multi.expand_lang(chain[0], &sterm)?
        }
    };

    generate(output_filename, &expanded)
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
    let input_filename = &args.filename;
    let base = input_filename
        .strip_suffix(".quilt")
        .unwrap_or(input_filename);
    let lang = base.split('.').next_back().unwrap();

    let input = fs::read_to_string(input_filename).into_diagnostic()?;

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
    let sterm = multi.parse_chain(chain, input)?;
    multi.expand_lang(host, &sterm)?.dump(path)?;
    Ok(hashbang)
}

fn generate(filename: &str, x: &Arc<QTerm>) -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>()[1..].join(" ");
    let header = format!("//! DO NOT EDIT. GENERATED BY `quilt {args}`.");
    x.dump_with_cmds(filename, &[write(&header), NL, NL], &[])
}
