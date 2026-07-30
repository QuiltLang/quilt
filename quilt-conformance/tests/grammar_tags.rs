//! Do the hand-written tag tables still match the grammars they describe?
//!
//! Quilt's per-language providers carry tables of tree-sitter node kinds as Rust
//! string literals: `Language::arity`'s variadic allowlist (50 entries for bash),
//! `Language::typ`'s exact-match arms, `is_expr_tag`, `MetaLanguage::pattern_tag`,
//! plus the tags declared in `conformance/spec/*.toml`. Nothing checked any of
//! them against the grammar.
//!
//! The failure mode is silent. A tag that is misspelled, or that a grammar bump
//! under `bin/sync-grammars` renamed, simply stops matching: `arity` falls
//! through to `Arity::Unknown` and the expander quietly stops treating that node
//! as variadic, changing emit/splice behaviour with no diagnostic. Issue #150
//! (bash and zsh arity tables drifted apart) is one instance; issue #174 is the
//! survey.
//!
//! Two checks here, both derived from the grammar rather than from a second copy
//! of the table:
//!
//! 1. [`spec_tags_are_real_node_kinds`] — a hard assertion that every tag any
//!    spec names is a node kind its grammar defines.
//! 2. [`variadic_tags_snapshot`] — a snapshot of the tags each provider actually
//!    treats as variadic, computed by asking the provider about every kind in the
//!    grammar. A grammar bump that renames or drops a kind shows up as a
//!    reviewable diff instead of a silent behaviour change (the #157 approach:
//!    review a diff, don't hand-maintain N literals).

use quilt::lang::{Arity, Language};
use quilt_conformance::{registry, spec::Spec, spec_dir};
use std::collections::BTreeSet;

/// Every tag a spec names, with the field it came from (for the failure message).
fn spec_tags(spec: &Spec) -> Vec<(String, &'static str)> {
    let mut tags = Vec::new();
    for f in &spec.fragments {
        tags.push((f.tag.clone(), "fragments[].tag"));
    }
    for p in &spec.lift {
        tags.push((p.tag.clone(), "lift[].tag"));
    }
    for t in spec.kinds.keys() {
        tags.push((t.clone(), "kinds"));
    }
    for t in &spec.variadic {
        tags.push((t.clone(), "variadic"));
    }
    for t in &spec.not_variadic {
        tags.push((t.clone(), "not_variadic"));
    }
    if let Some(t) = &spec.meta.pattern_tag {
        tags.push((t.clone(), "meta.pattern_tag"));
    }
    tags
}

/// Tags that are Quilt's own inventions rather than grammar node kinds, so the
/// grammar is the wrong authority for them.
///
/// * `{}` is the Rust hole spelling (`RustProvider::hole_str`): Quilt reuses an
///   empty block as its hole, and `qsym("{}")` tags the placeholder with it.
/// * `text` is the whole tag vocabulary of the grammar-less `text` language.
fn is_quilt_own_tag(tag: &str) -> bool {
    matches!(tag, "{}" | "text")
}

/// Every tag named in `conformance/spec/*.toml` is a node kind its grammar
/// actually defines.
///
/// This currently passes for all ten languages — it is a regression net, not a
/// bug report. It is the cheap half of the guard #174 asks for: it needs nothing
/// vendored beyond the parsers already in `quilt/grammars/`, because tree-sitter
/// exposes each grammar's whole symbol table at runtime.
#[test]
fn spec_tags_are_real_node_kinds() {
    let specs = Spec::load_all(&spec_dir()).expect("specs load");
    let mut failures = Vec::new();

    for spec in &specs {
        let Some(lang) = registry::grammar(&spec.name) else {
            continue; // `text` has no grammar to check against
        };
        let kinds = registry::node_kinds(&lang);
        for (tag, field) in spec_tags(spec) {
            if is_quilt_own_tag(&tag) || kinds.contains(tag.as_str()) {
                continue;
            }
            failures.push(format!(
                "{}: {field} names {tag:?}, which is not a node kind in the {} grammar",
                spec.name, spec.name
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} spec tag(s) name node kinds their grammar does not define:\n\n{}\n\n\
         Either the tag is misspelled, or `bin/sync-grammars` pulled a grammar \
         that renamed it — in which case the provider's own table in \
         quilt/src/langs/<lang>/lang.rs needs the same update.",
        failures.len(),
        failures.join("\n"),
    );
}

/// Which node kinds does each provider actually treat as variadic?
///
/// Asks the provider about every kind in its grammar, so the answer is the
/// intersection of "the table says variadic" and "the grammar has this kind" —
/// exactly the set that affects expansion. A tag in the table that the grammar
/// does not define cannot appear here, so its disappearance from this snapshot
/// is the signal that it drifted.
///
/// Reviewing a diff is the point (issue #157): `cargo insta review` shows a
/// grammar bump's effect on emit/splice behaviour as an explicit before/after.
#[test]
fn variadic_tags_snapshot() {
    use std::fmt::Write as _;

    let mut report = String::new();

    for name in registry::LANGUAGES {
        let Some(grammar) = registry::grammar(name) else {
            writeln!(report, "{name}: (no tree-sitter grammar)\n").unwrap();
            continue;
        };
        let lang = registry::language(name).expect("language builds");
        let variadic: BTreeSet<&str> = registry::node_kinds(&grammar)
            .into_iter()
            .filter(|kind| lang.arity(kind) == Arity::Variadic)
            .collect();

        writeln!(report, "{name}: {} variadic", variadic.len()).unwrap();
        for kind in &variadic {
            writeln!(report, "  {kind}").unwrap();
        }
        report.push('\n');
    }

    insta::assert_snapshot!(report);
}
