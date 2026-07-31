//! Integration tests for the `quilt` binary (issue #160).
//!
//! The CLI had essentially no coverage: `expand` / `check` / `run`, `-m`,
//! language-chain derivation from the file stem, shebang handling, exit codes
//! and the generated-file header were all untested — and **#136 lived exactly
//! there** (a Rust `//!` comment on every generated file, whatever the
//! language).
//!
//! These drive the real binary through `CARGO_BIN_EXE_quilt` rather than
//! calling into the library, because the things worth testing here are the
//! parts only the binary does: argument handling, what lands on stdout versus
//! stderr, and what the process exits with.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn quilt() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_quilt"));
    // The dev shell exports `RUST_LOG=info` (see .envrc), which puts tracing's
    // `running: …` line on the CLI's stdout. Assertions about what a command
    // printed should not depend on the developer's environment.
    c.env_remove("RUST_LOG");
    c
}

/// A scratch directory that cleans itself up.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "quilt-cli-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("scratch dir");
        Dir(p)
    }

    fn write(&self, name: &str, body: &str) -> PathBuf {
        let p = self.0.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(&p, body).expect("write fixture");
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&Path]) -> Output {
    quilt().args(args).output().expect("quilt runs")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/* ── check: exit codes ─────────────────────────────────────────────────── */

#[test]
fn check_valid_file_succeeds() {
    let d = Dir::new("ok");
    let f = d.write("ok.rs.quilt", "let x = ↖1 + 2↗;\n");
    let o = run(&[Path::new("check"), &f]);
    assert!(
        o.status.success(),
        "expected success, stderr:\n{}",
        stderr(&o)
    );
    assert!(stdout(&o).contains(": ok"), "stdout: {}", stdout(&o));
}

#[test]
fn check_invalid_file_exits_nonzero() {
    let d = Dir::new("bad");
    let f = d.write("bad.rs.quilt", "let x = ↙1 + 2↘;\n");
    let o = run(&[Path::new("check"), &f]);
    assert_eq!(o.status.code(), Some(1), "stderr:\n{}", stderr(&o));
}

#[test]
fn check_missing_file_exits_nonzero() {
    let d = Dir::new("missing");
    let f = d.0.join("nope.rs.quilt");
    let o = run(&[Path::new("check"), &f]);
    assert_eq!(o.status.code(), Some(1));
}

/// One broken file must not hide the rest: every file is checked, and the
/// summary counts them.
#[test]
fn check_reports_every_file_before_failing() {
    let d = Dir::new("multi");
    let ok = d.write("ok.rs.quilt", "let x = ↖1 + 2↗;\n");
    let bad = d.write("bad.rs.quilt", "let x = ↙1 + 2↘;\n");
    let o = run(&[Path::new("check"), &ok, &bad]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).contains(": ok"),
        "the good file should still report ok"
    );
    assert!(
        stderr(&o).contains("1 of 2 file(s) failed"),
        "summary should count both files, got:\n{}",
        stderr(&o)
    );
}

/* ── check: diagnostics ────────────────────────────────────────────────── */

/// `check` is the CI and pre-commit path, so its diagnostics are the ones a
/// contributor actually reads. They used to be bare byte offsets
/// ("source bytes 8..19") while `expand` rendered a caret under the source.
#[test]
fn check_renders_a_source_snippet() {
    let d = Dir::new("snippet");
    let f = d.write("bad.rs.quilt", "let x = ↙1 + 2↘;\n");
    let e = stderr(&run(&[Path::new("check"), &f]));
    assert!(e.contains("unquote depth too high"), "{e}");
    assert!(
        e.contains("let x = ↙1 + 2↘;"),
        "the offending line should be shown, got:\n{e}"
    );
    assert!(e.contains(":1:9"), "expected a line:col, got:\n{e}");
}

/// A stripped shebang must not shift the line numbers of everything after it.
#[test]
fn check_shebang_does_not_shift_line_numbers() {
    let d = Dir::new("shebang");
    let f = d.write("sheb.rs.quilt", "#!/usr/bin/env quilt\nlet x = ↙1 + 2↘;\n");
    let e = stderr(&run(&[Path::new("check"), &f]));
    assert!(
        e.contains(":2:9"),
        "the error is on line 2 of the file; got:\n{e}"
    );
}

#[test]
fn check_accepts_a_valid_shebang_script() {
    let d = Dir::new("shebang-ok");
    let f = d.write("ok.rs.quilt", "#!/usr/bin/env quilt\nlet x = ↖1 + 2↗;\n");
    let o = run(&[Path::new("check"), &f]);
    assert!(o.status.success(), "stderr:\n{}", stderr(&o));
}

/// Documents a real inconsistency rather than asserting it is right: `run`
/// happily executes an extension-less shebang script (that is what `bin/issues`
/// is), but `check` refuses anything without a `.quilt` suffix — so those files
/// can be run and never validated in CI.
#[test]
fn check_rejects_a_file_without_the_quilt_suffix() {
    let d = Dir::new("nosuffix");
    let f = d.write("issues", "#!/usr/bin/env quilt\nlet x = ↖1 + 2↗;\n");
    let o = run(&[Path::new("check"), &f]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("expected a .quilt file"),
        "stderr:\n{}",
        stderr(&o)
    );
}

/* ── expand ────────────────────────────────────────────────────────────── */

#[test]
fn expand_writes_the_sibling_file() {
    let d = Dir::new("expand");
    let f = d.write("e.rs.quilt", "let x = ↖1 + 2↗;\n");
    let o = run(&[Path::new("expand"), &f]);
    assert!(o.status.success(), "stderr:\n{}", stderr(&o));
    let out = d.0.join("e.rs");
    assert!(out.exists(), "expand should write {}", out.display());
    let body = std::fs::read_to_string(&out).unwrap();
    assert!(body.contains("tb(\"binary_expression\")"), "body:\n{body}");
}

/// The `DO NOT EDIT` header has to be a comment *in the language generated* —
/// issue #136, where every language got Rust's `//!`.
#[test]
fn expand_header_comment_matches_the_language() {
    for (name, src, prefix) in [
        ("h.rs.quilt", "let x = ↖1 + 2↗;\n", "//!"),
        ("h.py.quilt", "x = ↖1 + 2↗\n", "#"),
        ("h.nix.quilt", "nix↖1↗\n", "#"),
        ("h.ts.quilt", "let x = ↖1 + 2↗;\n", "//"),
    ] {
        let d = Dir::new(&format!("hdr-{}", name.replace('.', "-")));
        let f = d.write(name, src);
        let o = run(&[Path::new("expand"), &f]);
        assert!(o.status.success(), "{name}: stderr:\n{}", stderr(&o));
        let out = d.0.join(name.strip_suffix(".quilt").unwrap());
        let body = std::fs::read_to_string(&out).unwrap();
        let first = body.lines().next().unwrap_or_default();
        assert!(
            first.starts_with(&format!("{prefix} DO NOT EDIT")),
            "{name}: header should start with {prefix:?}, got {first:?}"
        );
    }
}

/// The file stem is the language chain: `shaders.wgsl.rs.quilt` is Rust on the
/// ground with bare quotes defaulting to WGSL.
#[test]
fn expand_derives_the_language_chain_from_the_stem() {
    let d = Dir::new("chain");
    let f = d.write("s.wgsl.rs.quilt", "let shader = ↖1u↗;\n");
    let o = run(&[Path::new("expand"), &f]);
    assert!(o.status.success(), "stderr:\n{}", stderr(&o));
    let body = std::fs::read_to_string(d.0.join("s.wgsl.rs")).unwrap();
    assert!(
        body.contains("int_literal"),
        "the bare quote should have parsed as WGSL, got:\n{body}"
    );
}

#[test]
fn expand_reports_errors_with_a_snippet() {
    let d = Dir::new("expand-bad");
    let f = d.write("bad.rs.quilt", "let x = ↙1 + 2↘;\n");
    let o = run(&[Path::new("expand"), &f]);
    assert!(!o.status.success());
    let e = stderr(&o);
    assert!(e.contains("let x = ↙1 + 2↘;"), "stderr:\n{e}");
}

/* ── run: the TypeScript/Node backend ──────────────────────────────────── */

/// The Node runtime `quilt run` binds the bare `quilt` import to, and the
/// wasm-pack build it needs. Absent in a checkout that has not run
/// `bin/build-ts`, so the tests below skip rather than fail there — the CI
/// `typescript` job builds it, as the `python` job does for `quilt_python`.
fn node_runtime_built() -> bool {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quilt-wasm/pkg/quilt_wasm.js")
        .exists()
}

fn skip_without_node_runtime(test: &str) -> bool {
    if node_runtime_built() {
        return false;
    }
    eprintln!("skipping {test}: quilt-wasm is not built for Node — run `bin/build-ts`");
    true
}

/// A `.ts.quilt` file has to actually run: the expanded program imports the
/// runtime by bare specifier, and Node's ESM resolver ignores `NODE_PATH`, so
/// before #153 `quilt run` died at the import.
#[test]
fn run_typescript_resolves_the_runtime_import() {
    if skip_without_node_runtime("run_typescript_resolves_the_runtime_import") {
        return;
    }
    let d = Dir::new("run-ts");
    let f = d.write(
        "hello.ts.quilt",
        "import { tb, leaf, sym } from \"quilt\";\n\
         const frag = ts↖1 + 2↗;\n\
         console.log(frag.coparse());\n",
    );
    let o = run(&[Path::new("run"), &f]);
    assert!(o.status.success(), "stderr:\n{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "1 + 2");
}

/// `↓` on the CLI — the whole point of #153. The first reduce is plain
/// TypeScript; the second is a *generated stage* whose source still holds
/// glyphs, so it only works if the runtime re-invokes the expander.
#[test]
fn run_typescript_reduces() {
    if skip_without_node_runtime("run_typescript_reduces") {
        return;
    }
    let d = Dir::new("run-ts-reduce");
    // Everything printed is a string: `console.log` inspects non-string
    // arguments, which adds ANSI colour whenever $FORCE_COLOR is set.
    let f = d.write(
        "reduce.ts.quilt",
        "import { tb, leaf, sym, quote, unquote, cmd, write, qlift, HOLE } from \"quilt\";\n\
         console.log(`plain: ${(ts↖6 * 7↗).↓}`);\n\
         const stage2 = ts↖(a) => ts↖(x) => ↙↑(a)↘ * x↗↗;\n\
         const gen = stage2.↓;\n\
         console.log(`staged: ${gen(7).coparse()}`);\n\
         console.log(`value: ${gen(7).↓(6)}`);\n",
    );
    let o = run(&[Path::new("run"), &f]);
    assert!(o.status.success(), "stderr:\n{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("plain: 42"), "stdout:\n{out}");
    assert!(out.contains("staged: (x) => 7 * x"), "stdout:\n{out}");
    assert!(out.contains("value: 42"), "stdout:\n{out}");
}

/// Reduce has to re-expand a generated stage under the right language chain, or
/// the stage's own quotes resolve to the wrong language. `quilt run` passes the
/// running chain as `$QUILT_CHAIN` and the runtime reads the stage's chain off
/// it: one entry along, since the chain is indexed by quote depth — but never
/// dropping the ground, or the stage would stop being a TypeScript program.
#[test]
fn run_typescript_reduces_under_the_language_chain() {
    if skip_without_node_runtime("run_typescript_reduces_under_the_language_chain") {
        return;
    }
    let imports = "import { tb, leaf, sym, quote, unquote, cmd, write, push, \
                   name, qlift, qlift_html, NL, POP, HOLE } from \"quilt\";\n";
    for (name, body) in [
        // [ts, ts, html]: the stage is the un-annotated depth-1 quote, and its
        // own un-annotated quote is HTML — so the stage's chain is [ts, html].
        (
            "gen.html.ts.ts.quilt",
            "const s = ↖(t) => ↖<li>↙↑(t)↘</li>↗↗;\n",
        ),
        // [ts, html]: the stage has to be written `ts↖…↗`, and dropping the
        // ground would re-expand it as an HTML program.
        (
            "gen.html.ts.quilt",
            "const s = ts↖(t) => html↖<li>↙↑(t)↘</li>↗↗;\n",
        ),
    ] {
        let d = Dir::new(&format!("run-ts-chain-{}", name.replace('.', "-")));
        let f = d.write(
            name,
            &format!("{imports}{body}console.log(s.↓(\"Fix <bug>\").coparse());\n"),
        );
        let o = run(&[Path::new("run"), &f]);
        assert!(o.status.success(), "{name}: stderr:\n{}", stderr(&o));
        // The HTML lift escapes, which also proves the fragment was built by the
        // HTML side of the runtime rather than spliced as TypeScript text.
        assert_eq!(stdout(&o).trim(), "<li>Fix &lt;bug&gt;</li>", "{name}");
    }
}

/// The script's exit code is the CLI's exit code, so a failing `.ts.quilt`
/// program is not reported as a success.
#[test]
fn run_typescript_propagates_the_exit_code() {
    if skip_without_node_runtime("run_typescript_propagates_the_exit_code") {
        return;
    }
    let d = Dir::new("run-ts-exit");
    let f = d.write("boom.ts.quilt", "process.exit(3);\n");
    let o = run(&[Path::new("run"), &f]);
    assert_eq!(o.status.code(), Some(3), "stderr:\n{}", stderr(&o));
}

/* ── general ───────────────────────────────────────────────────────────── */

#[test]
fn help_lists_the_subcommands() {
    let o = quilt().arg("--help").output().expect("runs");
    assert!(o.status.success());
    let s = stdout(&o);
    for sub in ["expand", "check", "run"] {
        assert!(s.contains(sub), "--help should mention {sub}, got:\n{s}");
    }
}
