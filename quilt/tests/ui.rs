//! The `tests/ui/` diagnostic corpus (issue #160, the last open Tier 5 item of
//! the conformance epic #144).
//!
//! `tests/cli.rs` covers the error *paths* — that a bad file exits 1, that a
//! snippet is rendered somewhere in the output. It asserts on substrings, so the
//! thing a contributor actually reads (the full rendered diagnostic, and above
//! all *where the caret lands*) was still not regression-tested. Spans were
//! their own issue (#4) and "no `unimplemented!` panics" was another (#11);
//! neither had a gate that would notice them regressing.
//!
//! So: a directory of deliberately *invalid* inputs, each run through the real
//! binary, with the whole rendered `miette` report snapshotted. A change to an
//! error message, a help text, a label, or a span becomes a reviewable snapshot
//! diff rather than something nobody sees until a user hits it.
//!
//! # What each case must do
//!
//! Every fixture has to **fail with a diagnostic** — not panic, not succeed.
//! `run_check` asserts a non-zero exit, so a fixture that starts *passing*
//! (because we implemented the thing it probes) fails this suite loudly and asks
//! to be reclassified, the same bidirectional honesty `bin/check-matrix` applies
//! to the support matrix. Because the whole corpus goes through one `Command`,
//! a `todo!()`/`unimplemented!` on any of these paths shows up as a signal
//! (exit 101, "panicked at") in the snapshot rather than as a green test.
//!
//! # Adding a case
//!
//! Drop the fixture in `tests/ui/`, add a line to [`CASES`] saying what it
//! probes, and run `cargo insta review`. [`every_fixture_is_declared`] fails in
//! both directions, so a fixture can't be added without a rationale and a
//! rationale can't outlive its fixture.
//!
//! # Determinism
//!
//! The rendering has to be identical on a developer's machine and on CI, so
//! `quilt()` pins everything `miette` reads from the environment: colour off
//! (`NO_COLOR`, and the `FORCE_COLOR`/`CLICOLOR_FORCE` overrides that beat it),
//! unicode box-drawing on (`LC_ALL`), and no `RUST_LOG` line from the dev
//! shell. Width is not pinned because miette falls back to 80 columns whenever
//! stderr is not a terminal, which it never is under `Command::output`.
//!
//! Fixtures are invoked by *relative* path from `tests/ui/`, so the filename
//! miette prints is the fixture name rather than a machine-specific absolute
//! path.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Every fixture in `tests/ui/`, and what it is here to pin.
///
/// The second field is documentation, not an assertion — but the pairing is
/// enforced (see [`every_fixture_is_declared`]), which is what stops the
/// directory from accumulating fixtures nobody can explain.
const CASES: &[(&str, &str)] = &[
    // ── spanned diagnostics: the ones #4 was about ──────────────────────────
    (
        "unquote_depth.rs.quilt",
        "an unquote at ground level: the canonical span-carrying error",
    ),
    (
        "unbalanced_bracket.rs.quilt",
        "a quote that is never closed — the Quilt grammar's own syntax error, \
         with a span and a help",
    ),
    // ── the object language rejecting a fragment ────────────────────────────
    (
        "object_parse_error.rs.quilt",
        "a quote whose body is not valid Rust",
    ),
    // ── language resolution ─────────────────────────────────────────────────
    (
        "unknown_annotation.rs.quilt",
        "an annotation naming a language that is not registered",
    ),
    (
        "unknown_ground_language.klingon.quilt",
        "the same, reached through the file stem instead: chain derivation \
         falling off the end of the registry",
    ),
    (
        "text_meta_unavailable.txt.quilt",
        "text has a MetaLanguage, but it is deliberately absent from Omni, so \
         a .txt.quilt file cannot be a host",
    ),
    // ── operators a host does not support ───────────────────────────────────
    (
        "lift_unsupported_target.rs.quilt",
        "`↑` into a target with no LiftTo impls — the ragged corner of the lift \
         grid (#149)",
    ),
    (
        "lean_emit_unsupported.lean.quilt",
        "`←` from the string-based Lean host, which has no `b_` accumulator \
         (#132) — must refuse rather than leak `__EMIT__` (#190)",
    ),
    (
        "nix_reduce_unsupported.nix.quilt",
        "`↓` from the string-based Nix host, which has no QTerm runtime (#155)",
    ),
    (
        "nix_type_unsupported.nix.quilt",
        "`⟨T⟩` in a host with no type syntax to name",
    ),
    // ── pattern-let ─────────────────────────────────────────────────────────
    (
        "pattern_let_non_ident.rs.quilt",
        "a metavariable that is not a plain identifier",
    ),
    (
        "pattern_let_duplicate_var.rs.quilt",
        "the same metavariable bound twice",
    ),
    // ── the CLI's own argument-level refusals ───────────────────────────────
    (
        "no_quilt_suffix",
        "an extension-less shebang script: `run` executes these, `check` \
         refuses them (#188)",
    ),
];

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui")
}

/// The binary, with every environment input to the rendering pinned. See the
/// module docs for why each one is here.
fn quilt() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_quilt"));
    c.current_dir(ui_dir());
    // The dev shell exports RUST_LOG=info (see .envrc), which puts tracing's
    // `running: …` line in the output.
    c.env_remove("RUST_LOG");
    // NO_COLOR is honoured by miette, but FORCE_COLOR / CLICOLOR_FORCE outrank
    // it — and a developer shell may well set them (this one does).
    c.env_remove("FORCE_COLOR");
    c.env_remove("CLICOLOR_FORCE");
    c.env_remove("CLICOLOR");
    c.env("NO_COLOR", "1");
    // miette picks ASCII box-drawing when the locale doesn't advertise UTF-8,
    // which is a bare CI container. Pin the UTF-8 rendering everywhere.
    c.env("LC_ALL", "C.UTF-8");
    c.env("LANG", "C.UTF-8");
    c
}

/// `quilt check <fixture>` rendered for snapshotting: the exit status plus
/// stderr, which is where `check` writes diagnostics.
///
/// stdout carries only the `<file>: ok` lines, which `tests/cli.rs` covers; a
/// fixture here never produces one.
fn run_check(fixture: &str) -> String {
    let out = quilt()
        .arg("check")
        .arg(fixture)
        .output()
        .expect("quilt runs");

    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        !out.status.success(),
        "{fixture} is in the UI corpus but `quilt check` accepted it.\n\
         Either it stopped being invalid — in which case move it to \
         tests/cli.rs or examples/ — or the error it probed regressed into \
         silence.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains('\u{1b}'),
        "{fixture}: ANSI escapes leaked into the diagnostic, so the snapshot \
         would depend on the developer's terminal. Check the env scrubbing in \
         `quilt()`.\nstderr:\n{stderr}"
    );

    format!(
        "$ quilt check {fixture}\nexit: {}\n\n{}",
        out.status
            .code()
            .map_or_else(|| "signal (no exit code)".to_string(), |c| c.to_string()),
        stderr.trim_end()
    )
}

/// The corpus. One snapshot per fixture, named after it.
#[test]
fn diagnostics() {
    for (fixture, _why) in CASES {
        let name = fixture.replace(['.', '-'], "_");
        insta::assert_snapshot!(name, run_check(fixture));
    }
}

/// The directory and [`CASES`] must agree. Without this a fixture could be
/// added with no rationale (and, more importantly, no snapshot — the loop above
/// only visits what is declared), or a rationale could outlive the file it
/// describes.
#[test]
fn every_fixture_is_declared() {
    let mut on_disk: Vec<String> = std::fs::read_dir(ui_dir())
        .expect("tests/ui exists")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    on_disk.sort();

    let mut declared: Vec<String> = CASES.iter().map(|(f, _)| (*f).to_string()).collect();
    declared.sort();

    assert_eq!(
        on_disk, declared,
        "tests/ui/ and the CASES table disagree — every fixture needs an entry \
         saying what it probes, and every entry needs a fixture"
    );
}

/// `check` on a path that isn't there. Not a fixture for the obvious reason,
/// but the same class of thing: a message a user reads.
#[test]
fn missing_file() {
    let out = quilt()
        .arg("check")
        .arg("does_not_exist.rs.quilt")
        .output()
        .expect("quilt runs");
    assert!(!out.status.success());
    insta::assert_snapshot!(
        "missing_file",
        String::from_utf8_lossy(&out.stderr).trim_end()
    );
}
