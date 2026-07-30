//! The conformance suite.
//!
//! One `#[test]` per language, so libtest fans the work across threads and each
//! language pays for exactly one parser construction (see `registry`). A
//! failure lists every probe that did not hold for that language rather than
//! stopping at the first, because when a cross-cutting change breaks a language
//! you want the whole blast radius in one run.

use quilt_conformance::{
    battery, matrix::Axis, matrix_json_path, matrix_md_path, registry, run_all, spec::Spec,
    spec_dir,
};

/// Run one language's battery and report every failure at once.
fn check(name: &str) {
    let specs = Spec::load_all(&spec_dir()).expect("specs load");
    let spec = specs
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no spec for {name:?} in conformance/spec/"));

    let outcome = battery::run_language(spec).expect("battery runs");
    assert!(
        outcome.failures.is_empty(),
        "{} conformance failure(s) for {name}:\n\n{}\n\nThe spec is \
         conformance/spec/{name}.toml — either the implementation regressed or the \
         claim is wrong. Update whichever is out of date, then run `bin/gen-matrix`.",
        outcome.failures.len(),
        outcome
            .failures
            .iter()
            .map(|f| format!("  • {f}"))
            .collect::<Vec<_>>()
            .join("\n\n"),
    );
}

macro_rules! language_tests {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                check(stringify!($name));
            }
        )*
    };
}

language_tests!(bash, html, lean, nix, python, rust, text, typescript, wgsl, zsh);

/// Every registered language must have a spec, and every spec must name a
/// registered language. This is the check that makes it impossible to add a
/// language to `Omni` and skip the battery — the failure mode that left bash,
/// zsh and text with no coverage at all.
#[test]
fn every_language_has_a_spec() {
    let specs = Spec::load_all(&spec_dir()).expect("specs load");
    let spec_names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();

    let missing: Vec<&&str> = registry::LANGUAGES
        .iter()
        .filter(|l| !spec_names.contains(*l))
        .collect();
    assert!(
        missing.is_empty(),
        "registered language(s) with no conformance spec: {missing:?}\n\
         Add conformance/spec/<lang>.toml — see docs/wiki/adding-a-language.md.",
    );

    let extra: Vec<&&str> = spec_names
        .iter()
        .filter(|n| !registry::LANGUAGES.contains(n))
        .collect();
    assert!(
        extra.is_empty(),
        "spec file(s) for unregistered language(s): {extra:?}",
    );
}

/// The registry's host list must match the languages that actually have a
/// `MetaLanguage`, so `registry::HOSTS` cannot drift from `langs/omni.rs`.
#[test]
fn host_list_matches_registry() {
    let actual: Vec<&str> = registry::LANGUAGES
        .iter()
        .copied()
        .filter(|l| registry::meta(l).is_some())
        .collect();
    assert_eq!(
        actual,
        registry::HOSTS,
        "registry::HOSTS is stale relative to the MetaLanguages actually registered",
    );
}

/// Each language must be constructible without panicking. Cheap on its own, but
/// it isolates "the parser could not even be built" from a probe failure.
#[test]
fn every_language_constructs() {
    for name in registry::LANGUAGES {
        assert!(
            registry::language(name).is_ok(),
            "registry::language({name:?}) failed",
        );
    }
}

/// Every axis must be answered by every spec, and every note/issue rule must
/// hold. `Spec::load_all` enforces this, so the test is a direct assertion that
/// the specs are complete.
#[test]
fn specs_are_complete() {
    let specs = Spec::load_all(&spec_dir()).expect("specs load and validate");
    assert_eq!(
        specs.len(),
        registry::LANGUAGES.len(),
        "expected one spec per registered language",
    );
    for spec in &specs {
        for axis in Axis::ALL {
            spec.claim(*axis)
                .unwrap_or_else(|e| panic!("{}: {e}", spec.name));
        }
    }
}

/// The committed matrix must match what the harness produces right now. This is
/// the gate that keeps the website's table honest; `bin/check-matrix` runs the
/// same comparison in CI.
#[test]
fn committed_matrix_is_current() {
    let (matrix, _failures) = run_all().expect("battery runs");

    for (path, fresh) in [
        (matrix_json_path(), matrix.to_json()),
        (matrix_md_path(), matrix.to_markdown()),
    ] {
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{} is missing ({e}) — run `bin/gen-matrix` and commit it",
                path.display()
            )
        });
        assert!(
            committed == fresh,
            "{} is stale — run `bin/gen-matrix` and commit the result",
            path.display(),
        );
    }
}

/// The shared runtime corpus (#159), run against the Rust implementation. The
/// Python and Node runners execute the same file, so a divergence between the
/// three published runtimes fails here or in `bin/test-runtimes`.
#[test]
fn runtime_corpus_rust() {
    let failures = quilt_conformance::runtime::run().expect("corpus loads");
    assert!(
        failures.is_empty(),
        "{} runtime corpus failure(s) for rust:\n\n{}\n\nThe corpus is \
         conformance/runtime/cases.json.",
        failures.len(),
        failures
            .iter()
            .map(|f| format!("  • {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// The cross-language grid (#158): every host against every target. Reports
/// every failing cell at once — when a cross-cutting change breaks the grid you
/// want the blast radius, not the first cell.
#[test]
fn cross_language_grid() {
    let specs = Spec::load_all(&spec_dir()).expect("specs load");
    let (failures, cells) = quilt_conformance::cross::run(&specs).expect("grid runs");
    assert!(
        failures.is_empty(),
        "{} of {cells} cross-language cell(s) failed:\n\n{}\n",
        failures.len(),
        failures
            .iter()
            .map(|f| format!("  • {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert!(cells >= 40, "expected a full grid, got {cells} cells");
}

/// A refused lift must name both the host and the target. "cannot lift" without
/// saying between what sends the reader to the source.
#[test]
fn lift_errors_name_both_ends() {
    let specs = Spec::load_all(&spec_dir()).expect("specs load");
    let failures = quilt_conformance::cross::check_lift_errors(&specs).expect("runs");
    assert!(
        failures.is_empty(),
        "{} unactionable lift error(s):\n\n{}\n",
        failures.len(),
        failures
            .iter()
            .map(|f| format!("  • {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Every language must work as the non-ground member of a `.a.b.quilt` chain,
/// which is what the `chain-member` axis claims (#158).
#[test]
fn chain_members() {
    let specs = Spec::load_all(&spec_dir()).expect("specs load");
    let (failures, checked) = quilt_conformance::cross::check_chain_members(&specs).expect("runs");
    assert!(
        failures.is_empty(),
        "{} of {checked} chain-member check(s) failed:\n\n{}\n",
        failures.len(),
        failures
            .iter()
            .map(|f| format!("  • {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// `bin/check-examples` recognises a generated file by its header comment. If
/// `quilt::langs::comment_prefix` can emit a spelling that script's regex does
/// not list, the file stops being recognised and silently drops out of the
/// expand-diff — losing coverage with no failure anywhere.
///
/// That is not hypothetical: moving TypeScript from `//!` to `//` did exactly
/// this, and the only symptom was the compared-output count going from 21 to 20.
#[test]
fn check_examples_recognises_every_header_prefix() {
    let script = std::fs::read_to_string(quilt_conformance::repo_root().join("bin/check-examples"))
        .expect("bin/check-examples is readable");

    let line = script
        .lines()
        .find(|l| l.trim_start().starts_with("header_re="))
        .expect("bin/check-examples defines header_re");

    for lang in quilt_conformance::registry::LANGUAGES {
        let Some(prefix) = quilt::langs::comment_prefix(lang) else {
            continue;
        };
        // The regex spells alternatives as `(//!|//|#|--)`; a prefix is covered
        // when it appears as one of them.
        let covered = line.split(['(', ')', '|']).any(|alt| alt == prefix);
        assert!(
            covered,
            "bin/check-examples' header_re does not list {prefix:?} (the prefix for {lang}), so a \
             generated {lang} file would not be recognised and would drop out of the expand diff.\n\
             header_re line: {line}",
        );
    }
}

/// `reduce(lift(x)) == x` for the Rust runtime, at the level the harness can
/// check without invoking rust-script: lifting a term must produce *constructor
/// code*, and that code must name the term's own tag and content — the property
/// that makes evaluating it reproduce the term. The Python and Node runners
/// prove the stronger form by actually evaluating (#166).
#[test]
fn lift_law_produces_constructor_code() {
    use quilt::prelude::*;

    for (label, term, want) in [
        ("leaf", leaf("integer", "7"), r#"leaf("integer", "7")"#),
        ("sym", sym("+"), r#"sym("+")"#),
        (
            "builder",
            tb("b").c(&leaf("integer", "1")).b(),
            r#"tb("b").c(&leaf("integer", "1")).b()"#,
        ),
    ] {
        let lifted = term.qlift().coparse();
        assert_eq!(lifted, want, "{label}: lifting a term must rebuild it");
        assert_ne!(
            lifted,
            term.coparse(),
            "{label}: lifting a term must not be the identity — that is the bug \
             in #166, where reduce would then evaluate the term's own code"
        );
    }
}
