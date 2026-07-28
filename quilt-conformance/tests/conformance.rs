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
