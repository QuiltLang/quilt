use clap::{Args, Parser, Subcommand, ValueEnum};
use miette::{bail, Context, IntoDiagnostic, NamedSource};
#[cfg(feature = "bootstrap")]
use quilt::langs::bootstrap::Bootstrap;
use quilt::{
    dir_template::{dir_params, instantiate_dir_with},
    lang::Language,
    langs::omni::Omni,
    multi::{lang_chain, template_params, Languages, MetaLanguages, Multi},
    prelude::*,
    template::{instantiate, strip_tier_b_marker, tier_b_program, ParamEnv, ParamValue},
    term::STerm,
};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

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
    /// Fill a sky-first template's holes with parameters (single-file Tier A)
    Instantiate(InstantiateArgs),
    /// Run a program that emits a `QTree` and materialize it to a directory
    Scaffold(ScaffoldArgs),
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
struct InstantiateArgs {
    /// Sky-first template to instantiate (a `*.tmpl.quilt` file)
    #[clap(index = 1)]
    filename: String,
    /// Where to write the output (defaults to stdout)
    #[clap(short, long)]
    out: Option<String>,
    /// Set a parameter: `--set name=value` (repeatable). A value that parses as
    /// an integer, float, or `true`/`false` is taken as that; otherwise it is a
    /// string. Use `--values` for lists and explicit typing.
    #[clap(long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,
    /// A TOML file of parameter values (`name = value`; arrays become lists).
    /// Merged under `--set`, which overrides it.
    #[clap(long)]
    values: Option<String>,
    /// List the template's inferred parameters (the free variables of its holes
    /// and templated path segments) and exit, without instantiating. Needs no
    /// `--set`/`--out`. Works for a single `*.tmpl.quilt` file or a template dir.
    #[clap(long)]
    describe: bool,
    /// multi-language to use
    #[clap(short, long, default_value_t, value_enum)]
    multi: MultiOptions,
}

#[derive(Args, Debug)]
struct ScaffoldArgs {
    /// Scaffold program: a `*.tree.<host>.quilt` file that builds a `QTree` and
    /// hands it over with `emit_tree(&t)`.
    filename: String,
    /// Directory to materialize the emitted tree under (created if absent).
    #[clap(short, long)]
    out: String,
    /// Set a scaffold parameter: `--set name=value` (repeatable). Each is
    /// exposed to the program as `QUILT_PARAM_<name>` (read via
    /// `quilt::scaffold_param`). Typing follows `instantiate`'s rule.
    #[clap(long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,
    /// A TOML file of parameter values, merged under `--set` (which overrides
    /// it).
    #[clap(long)]
    values: Option<String>,
    /// What to do when an output path already exists.
    #[clap(long = "on-conflict", value_enum, default_value_t = ConflictArg::Error)]
    on_conflict: ConflictArg,
    /// Compute and print the write plan but write nothing.
    #[clap(long)]
    dry_run: bool,
    /// Where to materialize the tree: `fs` writes files under `--out`; `nix`
    /// lowers the tree to a Nix `linkFarm` expression written to `--out` (a
    /// `.nix` file) that `nix build` turns into a store directory (issue #98).
    #[clap(long, value_enum, default_value_t = SinkArg::Fs)]
    sink: SinkArg,
    /// multi-language to use
    #[clap(short, long, default_value_t, value_enum)]
    multi: MultiOptions,
    /// Arguments passed through to the scaffold program.
    #[clap(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// Which sink `quilt scaffold` materializes the emitted tree through.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum SinkArg {
    /// Write files to the filesystem under `--out` (the default).
    #[default]
    Fs,
    /// Lower the tree to a Nix derivation written to `--out`.
    Nix,
}

/// CLI spelling of [`OnConflict`] for `--on-conflict` (clap can't derive
/// `ValueEnum` on the library type).
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum ConflictArg {
    /// Refuse to touch an existing path (the safe default).
    #[default]
    Error,
    /// Overwrite existing files.
    Overwrite,
    /// Leave existing files untouched.
    Skip,
    /// Rename the existing file to `<path>.orig`, then write the new one.
    Backup,
}

impl From<ConflictArg> for OnConflict {
    fn from(c: ConflictArg) -> Self {
        match c {
            ConflictArg::Error => OnConflict::Error,
            ConflictArg::Overwrite => OnConflict::Overwrite,
            ConflictArg::Skip => OnConflict::Skip,
            ConflictArg::Backup => OnConflict::Backup,
        }
    }
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
        (Some(Commands::Instantiate(args)), _) => instantiate_cmd(args),
        (Some(Commands::Scaffold(args)), _) => scaffold_cmd(args),
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
    let output_filename = input_filename.strip_suffix(".quilt").unwrap();

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

    // A `*.tmpl.quilt` file is a sky-first template, not a ground-first program:
    // checking it ground-first would fail at its first free `↙…↘` hole. Validate
    // it by parsing sky-first instead (instantiation needs parameters, which
    // `check` has none of, so parsing is as far as it goes).
    if let Some(tmpl_stem) = stem.strip_suffix(".tmpl") {
        match multi {
            MultiOptions::Omni => {
                let mut multi = Omni::default();
                let chain = lang_chain(&multi, tmpl_stem);
                multi.parse_template(&chain, &input)?;
            }
            #[cfg(feature = "bootstrap")]
            MultiOptions::Bootstrap => {
                let mut multi = Bootstrap::default();
                let chain = lang_chain(&multi, tmpl_stem);
                multi.parse_template(&chain, &input)?;
            }
        }
        return Ok(());
    }

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

/// Instantiate a sky-first template (`quilt instantiate`): a single
/// `*.tmpl.quilt` file (issue #88) or a whole template *directory* (issue #90).
/// Fills `↙…↘` holes from the merged `--values`/`--set` environment via Tier A
/// (`quilt::template::instantiate`), with Tier B behind a `#!tier-b` marker.
/// This path never touches the expand cache — the output depends on the
/// externally-supplied parameters.
fn instantiate_cmd(args: &InstantiateArgs) -> Result<()> {
    // `--describe` inspects the template's parameter signature without filling
    // it, so it needs neither parameters nor an output target (issue #99).
    if args.describe {
        return describe_cmd(args);
    }

    let env = build_env(&args.set, args.values.as_deref())?;

    // A directory input is a template *directory*: walk it into a QTree and
    // materialize that under `--out` (issue #90).
    if Path::new(&args.filename).is_dir() {
        return instantiate_dir_cmd(args, &env);
    }

    instantiate_file_cmd(args, &env)
}

/// `quilt instantiate --describe` (issue #99): print the inferred parameter
/// signature of a template — the union of the free variables of every hole and
/// templated path segment — without instantiating. The same inference the fill
/// path uses (`dir_params` for a directory, `parse_template` + `template_params`
/// for a single file), so what is listed is exactly what an instantiation must
/// supply.
fn describe_cmd(args: &InstantiateArgs) -> Result<()> {
    let path = Path::new(&args.filename);
    let params = if path.is_dir() {
        match args.multi {
            MultiOptions::Omni => dir_params(&mut Omni::default(), path)?,
            #[cfg(feature = "bootstrap")]
            MultiOptions::Bootstrap => dir_params(&mut Bootstrap::default(), path)?,
        }
    } else {
        describe_file_params(args)?
    };

    if params.is_empty() {
        println!("{}: no parameters", args.filename);
    } else {
        println!("{}: {} parameter(s)", args.filename, params.len());
        for p in &params {
            println!("  {p}");
        }
    }
    Ok(())
}

/// The inferred parameters of a single `*.tmpl.quilt` template file: parse it
/// sky-first (stripping any `#!tier-b` marker, whose bare-name holes still
/// count) and collect its free variables.
fn describe_file_params(args: &InstantiateArgs) -> Result<Vec<Box<str>>> {
    let filename = &args.filename;
    let stem = filename
        .strip_suffix(".quilt")
        .and_then(|s| s.strip_suffix(".tmpl"))
        .ok_or_else(|| {
            miette!(
                "expected a *.tmpl.quilt template file or a template directory, got {filename:?}"
            )
        })?;
    let raw_input = fs::read_to_string(filename).into_diagnostic()?;
    let body = match strip_tier_b_marker(&raw_input) {
        Some(b) => b.to_owned(),
        None => raw_input,
    };

    let template = match args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            let chain = lang_chain(&multi, stem);
            multi.parse_template(&chain, &body)?
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            let chain = lang_chain(&multi, stem);
            multi.parse_template(&chain, &body)?
        }
    };
    Ok(template_params(&template))
}

/// Build the parameter environment from `--values` (read first) then `--set`
/// (which overrides it). Shared by `instantiate` and `scaffold`.
fn build_env(set: &[String], values: Option<&str>) -> Result<ParamEnv> {
    let mut env = ParamEnv::new();
    if let Some(path) = values {
        let text = fs::read_to_string(path).into_diagnostic()?;
        merge_toml_values(&mut env, &text)?;
    }
    for kv in set {
        let (key, value) = kv
            .split_once('=')
            .ok_or_else(|| miette!("--set expects KEY=VALUE, got {kv:?}"))?;
        env.insert(key.into(), infer_scalar(value));
    }
    Ok(env)
}

/// Instantiate a template *directory* (issue #90): walk it into a `QTree` —
/// each `*.tmpl.quilt` file filled against `env`, every other file copied
/// verbatim — and materialize that tree under `--out` through an `FsSink`. A
/// directory can't go to stdout, so `--out` is required.
fn instantiate_dir_cmd(args: &InstantiateArgs, env: &ParamEnv) -> Result<()> {
    let out = args.out.as_ref().ok_or_else(|| {
        miette!("instantiating a directory needs an output directory: pass --out <dir>")
    })?;
    let dir = Path::new(&args.filename);

    // Tier B files run the Python host, exactly as the single-file path does.
    let mut render =
        |chain: &[&str], body: &str, env: &ParamEnv| render_tier_b_chain(chain, body, env);
    let tree = match args.multi {
        MultiOptions::Omni => {
            let mut multi = Omni::default();
            instantiate_dir_with(&mut multi, dir, env, &mut render)?
        }
        #[cfg(feature = "bootstrap")]
        MultiOptions::Bootstrap => {
            let mut multi = Bootstrap::default();
            instantiate_dir_with(&mut multi, dir, env, &mut render)?
        }
    };

    materialize(&tree, out, OnConflict::Error, false)
}

/// Materialize `tree` under `out` through an [`FsSink`], applying the conflict
/// policy and dry-run flag, then report what was (or would be) written. Shared
/// by directory `instantiate` and `scaffold`.
fn materialize(tree: &QTree, out: &str, on_conflict: OnConflict, dry_run: bool) -> Result<()> {
    let opts = WriteOptions {
        on_conflict,
        dry_run,
        ..WriteOptions::default()
    };
    let mut sink = FsSink::with_options(out, opts)?;
    write_tree(&mut sink, tree)?;
    let report = sink.report().clone();
    sink.finish()?;
    let verb = if dry_run { "would write" } else { "wrote" };
    eprintln!("{verb} {} path(s) under {out}", report.actions.len());
    if !report.is_empty() {
        eprint!("{report}");
    }
    Ok(())
}

/// Run a scaffold program (`quilt scaffold`, issue #95): expand and run the
/// `*.tree.<host>.quilt` file with [`prepare_runner`] (the same pipeline as
/// `run`), but with a sidecar file for it to `emit_tree` into and each parameter
/// exposed as `QUILT_PARAM_<name>`. Decode the emitted [`QTree`] and materialize
/// it under `--out`, honoring the write policy. Never touches the expand cache.
fn scaffold_cmd(args: &ScaffoldArgs) -> Result<()> {
    let env = build_env(&args.set, args.values.as_deref())?;
    let (mut cmd, temp_file) = prepare_runner(&args.filename, &args.multi)?;

    // The program hands its tree back over a postcard sidecar file, off its own
    // stdout/stderr (which it may use for prompts/logging).
    let tree_out = tempfile::Builder::new()
        .suffix(".postcard")
        .tempfile()
        .into_diagnostic()?;
    cmd.env(TREE_OUT_ENV, tree_out.path());
    for (name, value) in &env {
        cmd.env(format!("{PARAM_ENV_PREFIX}{name}"), param_to_env(value));
    }
    cmd.arg(temp_file.path()).args(&args.args);

    let status = cmd.status().into_diagnostic()?;
    if !status.success() {
        bail!("scaffold program exited unsuccessfully ({status})");
    }

    let bytes = fs::read(tree_out.path()).into_diagnostic()?;
    if bytes.is_empty() {
        bail!("scaffold program produced no tree — did it call `emit_tree(&t)`?");
    }
    let tree: QTree = postcard::from_bytes(&bytes)
        .into_diagnostic()
        .wrap_err("decoding the QTree the scaffold program emitted")?;

    match args.sink {
        SinkArg::Fs => materialize(&tree, &args.out, args.on_conflict.into(), args.dry_run),
        SinkArg::Nix => emit_nix(&tree, &args.filename, &args.out, args.dry_run),
    }
}

/// Lower `tree` to a Nix derivation (issue #98) and write it to `out` (a `.nix`
/// file). The derivation is named after the scaffold program's stem. Under
/// `dry_run` the Nix is printed to stdout instead.
fn emit_nix(tree: &QTree, program: &str, out: &str, dry_run: bool) -> Result<()> {
    // Name the derivation after the program file's stem (e.g. `cargo_crate` from
    // `cargo_crate.tree.rs.quilt`), falling back to `scaffold`.
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split('.').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("scaffold");
    let mut sink = NixSink::new(name);
    write_tree(&mut sink, tree)?;
    let source = sink.into_source();

    if dry_run {
        print!("{source}");
        return Ok(());
    }
    if let Some(parent) = Path::new(out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).into_diagnostic()?;
        }
    }
    fs::write(out, source.as_bytes()).into_diagnostic()?;
    eprintln!("wrote {out} (nix derivation `{name}`)");
    Ok(())
}

/// Render a parameter value for the `QUILT_PARAM_<name>` environment variable a
/// scaffold program reads. Scalars stringify plainly; a list is comma-joined
/// (best effort — a program needing structured input should read it itself).
fn param_to_env(value: &ParamValue) -> String {
    match value {
        ParamValue::Str(s) => s.clone(),
        ParamValue::Int(i) => i.to_string(),
        ParamValue::Float(f) => format!("{f:?}"),
        ParamValue::Bool(b) => if *b { "true" } else { "false" }.to_owned(),
        ParamValue::List(xs) => xs.iter().map(param_to_env).collect::<Vec<_>>().join(","),
    }
}

/// Instantiate a single-file sky-first template (issue #88): parse the
/// `*.tmpl.quilt` file sky-first and write the filled result to `--out` (or
/// stdout).
fn instantiate_file_cmd(args: &InstantiateArgs, env: &ParamEnv) -> Result<()> {
    let filename = &args.filename;
    // The `.tmpl.quilt` marker selects sky-first parsing; the remaining
    // extensions give the language chain (e.g. `greeting.py` -> `["py"]`).
    let stem = filename
        .strip_suffix(".quilt")
        .and_then(|s| s.strip_suffix(".tmpl"))
        .ok_or_else(|| {
            miette!(
                "expected a *.tmpl.quilt template file or a template directory, got {filename:?}"
            )
        })?;

    let raw_input = fs::read_to_string(filename).into_diagnostic()?;
    // A leading `#!tier-b` marker line opts into Tier B (host-backed holes).
    let (tier_b, input) = match strip_tier_b_marker(&raw_input) {
        Some(body) => (true, body.to_owned()),
        None => (false, raw_input),
    };

    let output: String = if tier_b {
        render_tier_b(stem, &input, env)?
    } else {
        let with_src =
            |e: miette::Report| e.with_source_code(NamedSource::new(filename, input.clone()));
        let rendered = match args.multi {
            MultiOptions::Omni => {
                let mut multi = Omni::default();
                let chain = lang_chain(&multi, stem);
                instantiate_template(&mut multi, &chain, &input, env).map_err(with_src)?
            }
            #[cfg(feature = "bootstrap")]
            MultiOptions::Bootstrap => {
                let mut multi = Bootstrap::default();
                let chain = lang_chain(&multi, stem);
                instantiate_template(&mut multi, &chain, &input, env).map_err(with_src)?
            }
        };
        rendered.coparse()
    };

    match &args.out {
        Some(path) => {
            fs::write(path, output.as_bytes()).into_diagnostic()?;
            eprintln!("wrote {path}");
        }
        None => print!("{output}"),
    }
    Ok(())
}

/// Render a Tier B template (single-file path): derive the language chain from
/// `stem`, then [`render_tier_b_chain`].
fn render_tier_b(stem: &str, body: &str, env: &ParamEnv) -> Result<String> {
    let multi = Omni::default();
    let chain = lang_chain(&multi, stem);
    render_tier_b_chain(&chain, body, env)
}

/// Render a Tier B template: source-wrap `body` into a Python-host metaprogram
/// with the declared parameters in scope, expand it, run it, and return its
/// stdout — the instantiated output. Holes may be arbitrary host expressions
/// over the parameters. Tier B always uses the Omni multi (it needs the real
/// Python runtime) and never touches the expand cache. The directory path
/// (issue #90) calls this per `#!tier-b` file with its own chain.
fn render_tier_b_chain(chain: &[&str], body: &str, env: &ParamEnv) -> Result<String> {
    let mut multi = Omni::default();
    let host = chain[0];
    let target = *chain.last().unwrap_or(&host);

    // Wrap first so an unsupported host (e.g. rust) fails fast and clearly.
    let params: Vec<(Box<str>, ParamValue)> =
        env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let program = tier_b_program(host, target, body, &params)?;

    // Validate the *bare-name* holes are all supplied (host-expression holes are
    // checked by the Python runtime when render runs).
    let template = multi.parse_template(chain, body)?;
    let missing: Vec<String> = template_params(&template)
        .into_iter()
        .filter(|p| !env.contains_key(p))
        .map(String::from)
        .collect();
    if !missing.is_empty() {
        bail!("missing template parameter(s): {}", missing.join(", "));
    }

    // Expand the host metaprogram to plain host code, then run it.
    let sterm = multi.parse_chain(&[host], &program)?;
    let expanded = multi.expand_lang(host, &sterm)?;
    let temp = tempfile::Builder::new()
        .suffix(".py")
        .tempfile()
        .into_diagnostic()?;
    let path = temp.path().to_str().unwrap();
    expanded.dump(path)?;
    run_python_capture(path)
}

/// Run the expanded Python metaprogram at `path` and capture its stdout, wiring
/// `PYTHONPATH`/`QUILT` the way `run` does so the `quilt` Python runtime imports.
fn run_python_capture(path: &str) -> Result<String> {
    let py_dir = format!("{}/../quilt-python", env!("CARGO_MANIFEST_DIR"));
    let pythonpath = match std::env::var("PYTHONPATH") {
        Ok(existing) if !existing.is_empty() => format!("{py_dir}:{existing}"),
        _ => py_dir,
    };
    let mut cmd = std::process::Command::new("python3");
    cmd.env("PYTHONPATH", pythonpath);
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("QUILT", exe);
    }
    let out = cmd.arg(path).output().into_diagnostic()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("Tier B render failed:\n{stderr}");
    }
    String::from_utf8(out.stdout).into_diagnostic()
}

/// Parse `input` sky-first and instantiate it against `env`. Reports *all*
/// missing parameters up front (clearer than failing at the first hole), then
/// fills the holes.
fn instantiate_template<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    chain: &[&str],
    input: &str,
    env: &ParamEnv,
) -> Result<Arc<QTerm>> {
    let template = multi.parse_template(chain, input)?;
    let missing: Vec<String> = template_params(&template)
        .into_iter()
        .filter(|p| !env.contains_key(p))
        .map(String::from)
        .collect();
    if !missing.is_empty() {
        bail!("missing template parameter(s): {}", missing.join(", "));
    }
    instantiate(&template, env)
}

/// Infer a `--set name=value` scalar's type: integer, then float, then bool,
/// else string. Lists and explicit typing come from `--values` TOML instead.
fn infer_scalar(value: &str) -> ParamValue {
    if let Ok(i) = value.parse::<i64>() {
        ParamValue::Int(i)
    } else if let Ok(f) = value.parse::<f64>() {
        ParamValue::Float(f)
    } else {
        match value {
            "true" => ParamValue::Bool(true),
            "false" => ParamValue::Bool(false),
            _ => ParamValue::Str(value.to_owned()),
        }
    }
}

/// Merge a `--values` TOML table into `env`. Top-level keys become parameters;
/// nested tables and datetimes have no `ParamValue` representation and error.
fn merge_toml_values(env: &mut ParamEnv, text: &str) -> Result<()> {
    let table: toml::Table = toml::from_str(text).into_diagnostic()?;
    for (key, value) in table {
        env.insert(key.into_boxed_str(), toml_to_param(&value)?);
    }
    Ok(())
}

fn toml_to_param(value: &toml::Value) -> Result<ParamValue> {
    Ok(match value {
        toml::Value::String(s) => ParamValue::Str(s.clone()),
        toml::Value::Integer(i) => ParamValue::Int(*i),
        toml::Value::Float(f) => ParamValue::Float(*f),
        toml::Value::Boolean(b) => ParamValue::Bool(*b),
        toml::Value::Array(xs) => {
            ParamValue::List(xs.iter().map(toml_to_param).collect::<Result<Vec<_>>>()?)
        }
        toml::Value::Table(_) => bail!("nested tables are not supported as template parameters"),
        toml::Value::Datetime(_) => {
            bail!("datetime values are not supported as template parameters")
        }
    })
}

fn run(args: &RunArgs) -> Result<()> {
    let (mut runner_cmd, temp_file) = prepare_runner(&args.filename, &args.multi)?;
    runner_cmd.arg(temp_file.path()).args(&args.args);
    let cmd_str = std::iter::once(runner_cmd.get_program())
        .chain(runner_cmd.get_args())
        .map(|s| s.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!("running: {cmd_str}");
    let status = runner_cmd.status().into_diagnostic()?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Expand a `.quilt` script to a temp host-source file and build the runner
/// [`Command`](std::process::Command) for it — resolving the runner from the
/// host language's hashbang and wiring rust-script's cargo manifest or python's
/// `PYTHONPATH`/`QUILT`. The command is returned *without* the script path or
/// trailing args, so callers add those (and any extra env): `run` runs it
/// directly, `scaffold` adds the tree sidecar. The returned temp file holds the
/// expanded source and must outlive the command run.
fn prepare_runner(
    filename: &str,
    multi: &MultiOptions,
) -> Result<(std::process::Command, tempfile::NamedTempFile)> {
    // Resolve symlinks so an extension-less entry point (`bin/issues ->
    // ../examples/issue_triage.html.py.quilt`) derives the language chain from
    // the target's name, and use only the file name so dots in directories
    // can't leak into it.
    let input_path = fs::canonicalize(filename).into_diagnostic()?;
    let file_name = input_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| miette!("invalid filename: {filename}"))?;
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

    let hashbang = match multi {
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
        let quilt_feature = match multi {
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
    }

    Ok((runner_cmd, temp_file))
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
    // Share the DO-NOT-EDIT marker with the tree write-policy stamper so
    // idempotent regen can recognize machine-owned files (issues #93/#94).
    let header = quilt::sink::header_line("//!", &args);
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
