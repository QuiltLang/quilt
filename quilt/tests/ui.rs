//! The `tests/ui/` diagnostic corpus (issue #160, Tier 5 of the conformance
//! epic #144): a directory of **invalid** `.quilt` inputs whose rendered
//! diagnostics are snapshotted, so error *text* and *span positions* are
//! regression-tested rather than incidentally exercised.
//!
//! Spans were their own issue (#4) and only a handful of tests assert them; the
//! CLI tests in `cli.rs` cover the error *paths* (a bad file exits 1, a snippet
//! is rendered) but not what each error kind actually reads like. A wrong span
//! or a message that stops naming both ends of a failed lift is a silent
//! regression today. Here it is a snapshot diff.
//!
//! ## What a case is
//!
//! One file per case, `ui/<name>.<chain>.quilt`, holding the smallest input
//! that provokes the error. The extensions before `.quilt` are the language
//! chain, exactly as on the command line — `nix_reduce.nix.quilt` is a Nix
//! host, `python_lift_into_wgsl.py.quilt` a Python one — so a case reads the
//! way a user would write it. Add a case by dropping in a file, running
//! `cargo insta review`, and listing it in `corpus_is_complete` below.
//!
//! Every case must fail: a file that expands cleanly is reported as an error,
//! because a "ui test" that stopped being a diagnostic has stopped testing
//! anything.
//!
//! ## Why it renders in-process instead of running the binary
//!
//! The rendering is done here with an explicitly configured
//! [`GraphicalReportHandler`] rather than by spawning `quilt check` and
//! capturing stderr. The binary's rendering is miette's global hook, whose
//! width, colour and box-drawing charset are all sniffed from the environment
//! (`supports-unicode` consults `LANG`/`TERM`) — so a committed snapshot would
//! differ between a developer's terminal and CI. Pinning the handler makes the
//! snapshot a property of quilt, not of the machine.
//!
//! What the CLI *adds* on top — attaching the file's source, blanking a
//! shebang so line numbers stay true — is reproduced faithfully below, and
//! `cli.rs` separately asserts the real binary renders a snippet with a
//! `line:col`. Between them the whole path is covered.

use miette::{GraphicalReportHandler, GraphicalTheme, NamedSource};
use quilt::langs::omni::Omni;
use quilt::multi::{Languages, MetaLanguages, Multi};
use quilt::term::STerm;
use std::path::Path;

/// Render a report the way the CLI would, but deterministically: a fixed
/// 80-column width, unicode box drawing, no colour.
fn render(report: &miette::Report) -> String {
    let mut out = String::new();
    GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor())
        .with_width(80)
        .render_report(&mut out, report.as_ref())
        .expect("render");
    out
}

/// The language chain a `.quilt` stem names, mirroring `lang_chain` in
/// `bin.rs`: read right-to-left, every extension that names a registered
/// language joins the chain (rightmost = ground), and the basename never
/// counts. An unrecognised final extension is still passed through, so a
/// mistyped language reports itself instead of silently becoming Rust — which
/// is what `unknown_ground_language.cobol.quilt` pins.
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

/// Parse + expand one case exactly as `quilt check` does, returning the
/// rendered diagnostic — or `Err(the expanded output)` if it did not fail.
fn diagnose(path: &Path) -> std::result::Result<String, String> {
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let stem = file_name.strip_suffix(".quilt").expect("a .quilt case");
    let input = std::fs::read_to_string(path).expect("read case");

    // Blank a shebang rather than dropping the line, like `check_file` does, so
    // every byte offset — and hence every reported line — stays exact.
    let input = if input.starts_with("#!") {
        let end = input.find('\n').unwrap_or(input.len());
        format!("{}{}", " ".repeat(end), &input[end..])
    } else {
        input
    };

    // The source is attached under the case's *file name*, never its path: the
    // snapshot has to read the same from a worktree, a CI checkout and a
    // vendored crate.
    let with_src =
        |e: miette::Report| render(&e.with_source_code(NamedSource::new(file_name, input.clone())));

    let mut multi = Omni::default();
    let chain = lang_chain(&multi, stem);
    let sterm = match multi.parse_chain(&chain, &input) {
        Ok(sterm) => sterm,
        Err(e) => return Ok(with_src(e)),
    };
    match multi.expand_lang(chain[0], &sterm) {
        Ok(expanded) => Err(expanded.coparse()),
        Err(e) => Ok(with_src(e)),
    }
}

/// Every file in `ui/`, snapshotted. `insta::glob!` runs the body once per
/// case and reports *all* mismatches at the end rather than stopping at the
/// first, which is what makes a bulk review (`cargo insta review`) work after a
/// deliberate change to a message.
#[test]
fn diagnostics() {
    insta::glob!("ui/*.quilt", |path| {
        match diagnose(path) {
            Ok(rendered) => insta::assert_snapshot!(rendered),
            Err(expanded) => panic!(
                "{}: expected a diagnostic, but it expanded cleanly to:\n{expanded}\n\
                 A ui case must fail — if this input is now legal, move it to the \
                 matching expand_*.rs test.",
                path.display()
            ),
        }
    });
}

/// The corpus is only worth as much as its coverage, and a case file is easy to
/// add and easy to forget. Pin the roster so deleting one is a deliberate diff
/// rather than a quiet loss, and so the list of error kinds under test is
/// readable in one place.
#[test]
fn corpus_is_complete() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui");
    let mut found: Vec<String> = std::fs::read_dir(&dir)
        .expect("tests/ui")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| Path::new(n).extension().is_some_and(|e| e == "quilt"))
        .collect();
    found.sort();

    let expected = [
        // bracket structure and depth
        "unbalanced_bracket.rs.quilt",
        "unquote_depth.rs.quilt",
        // the language registry
        "unknown_annotation.rs.quilt",
        "unknown_ground_language.cobol.quilt",
        // registered as a Language but not as a MetaLanguage: text has a meta,
        // but it is deliberately absent from Omni, so a .txt.quilt file parses
        // and then has nothing to host it
        "text_meta_unavailable.txt.quilt",
        // a target's own grammar rejecting the fragment
        "target_parse_error.rs.quilt",
        // heterogeneous operators with no spelling for that pair
        "python_lift_into_wgsl.py.quilt",
        "rust_lift_into_html.rs.quilt",
        "rust_reduce_via_wgsl.rs.quilt",
        // pattern-let (issue #18)
        "pattern_duplicate_metavar.rs.quilt",
        "pattern_non_identifier.rs.quilt",
        // the string-based hosts refusing what they have no runtime for
        "lean_emit.lean.quilt",
        "lean_reduce.lean.quilt",
        "nix_reduce.nix.quilt",
        "nix_type_annotation.nix.quilt",
    ];
    let mut expected: Vec<String> = expected.iter().map(ToString::to_string).collect();
    expected.sort();

    assert_eq!(
        found, expected,
        "tests/ui/ has drifted from the roster in this test — add the new case here \
         (with a comment saying which error kind it pins) or restore the deleted one"
    );
}
