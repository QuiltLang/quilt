//! Do the tag tables still match the grammars they describe?
//!
//! Quilt's per-language providers carry tables of tree-sitter node kinds as Rust
//! string literals: `Language::typ`'s exact-match arms, `is_expr_tag`,
//! `MetaLanguage::pattern_tag`, plus the tags declared in
//! `conformance/spec/*.toml`. Nothing checked any of them against the grammar.
//!
//! The failure mode is silent. A tag that is misspelled, or that a grammar bump
//! under `bin/sync-grammars` renamed, simply stops matching: the lookup falls
//! through to its default and the expander quietly changes behaviour with no
//! diagnostic. Issue #150 (bash and zsh arity tables drifted apart) is one
//! instance; issue #174 is the survey.
//!
//! `Language::arity` is no longer one of them — its tables are generated from
//! the grammars by `bin/gen-arity` and gated by `bin/check-arity` (#202), which
//! is a stronger guarantee than any check here could make. That also supersedes
//! the shared bash/zsh table #150 first reached for: the two dialects now agree
//! because their *grammars* do, not because they read one hand-written list.
//! The checks below cover what is still written by hand, plus two that keep the
//! generated tables honest:
//!
//! 1. [`spec_tags_are_real_node_kinds`] — a hard assertion that every tag any
//!    spec names is a node kind its grammar defines.
//! 2. [`variadic_tags_snapshot`] — a snapshot of the tags each provider actually
//!    treats as variadic, computed by asking the provider about every kind in the
//!    grammar. This is now a check that the *generated* tables are wired up and
//!    reach the provider, and a reviewable record of what emit can splice into
//!    (the #157 approach: review a diff, don't hand-maintain N literals).
//! 3. [`ident_tags_are_real_node_kinds`] — the one tag the expander constructs
//!    rather than parses (`Language::ident_tag`) must also be a kind its grammar
//!    defines (#174, finding E2).
//! 4. [`providers_use_the_derived_tables`] — every provider agrees with a *fresh*
//!    derivation, not merely with the committed file, which a stale table and the
//!    code reading it could satisfy together.
//! 5. [`bash_and_zsh_agree_on_shared_kinds`] — the two shells classify the node
//!    kinds their grammars share the same way, except where the grammars
//!    themselves differ.

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
/// exactly the set that affects expansion.
///
/// Since #202 the tables come from `bin/gen-arity`, so this no longer catches
/// drift between a table and its grammar — `bin/check-arity` does that at the
/// source. What it still catches is a provider that stopped consulting its
/// table, or consults the wrong one; and it remains the readable record of
/// where emit (`←`) may splice, which is the part a reviewer wants to see.
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

/// `Language::ident_tag` names a node kind its grammar actually defines.
///
/// The expander constructs exactly one term itself rather than parsing it: the
/// placeholder for an operator deferred to a later stage. That used to be
/// hardcoded `leaf("identifier", …)` in `multi.rs` — a Rust tag applied to every
/// language, and not a kind bash, zsh or html define at all (#174, finding E2).
/// Now each language answers, and this checks the answers against the grammars.
#[test]
fn ident_tags_are_real_node_kinds() {
    let mut failures = Vec::new();

    for name in registry::LANGUAGES {
        let Some(grammar) = registry::grammar(name) else {
            continue; // `text` has no grammar to check against
        };
        let lang = registry::language(name).expect("language builds");
        let tag = lang.ident_tag();
        if !registry::node_kinds(&grammar).contains(tag) {
            failures.push(format!(
                "{name}: ident_tag() is {tag:?}, not a node kind in the {name} grammar"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{}\n\nOverride `ident_tag` in that language's provider with a kind its \
         grammar defines.",
        failures.join("\n"),
    );
}

/// Every provider answers `arity` from the table its grammar derives — no
/// language quietly keeps a hand-written one.
///
/// [`variadic_tags_snapshot`] would show a divergence as a diff, but a diff is
/// only as good as the review that reads it, and "this language stopped
/// following its grammar" is the one change that must never be waved through.
/// So it is asserted rather than snapshotted, and asserted against a fresh
/// derivation rather than against `quilt/src/langs/arity.rs` — a committed file
/// and the code that reads it can agree with each other while both being stale.
/// `bin/check-arity` covers the remaining gap, that the committed file matches
/// what the generator now produces.
#[test]
fn providers_use_the_derived_tables() {
    use quilt_conformance::arity;

    let mut failures = Vec::new();
    for &name in registry::LANGUAGES {
        let Some(grammar_json) = arity::Grammar::load(name).expect("grammar.json loads") else {
            continue; // `text` has no grammar
        };
        let compiled = registry::grammar(name).expect("a language with grammar.json has a parser");
        let derived: BTreeSet<String> = grammar_json
            .variadic_tags(&registry::node_kinds(&compiled))
            .into_iter()
            .collect();

        let lang = registry::language(name).expect("language builds");
        let actual: BTreeSet<String> = registry::node_kinds(&compiled)
            .into_iter()
            .filter(|kind| lang.arity(kind) == Arity::Variadic)
            .map(str::to_string)
            .collect();

        for tag in derived.difference(&actual) {
            failures.push(format!(
                "{name}: {tag:?} is derived as variadic but arity() says not"
            ));
        }
        for tag in actual.difference(&derived) {
            failures.push(format!(
                "{name}: arity() says {tag:?} is variadic but the grammar has no repeat for it"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} node kind(s) where a provider's arity() disagrees with its grammar:\n\n{}\n\n\
         `Language::arity` must be `Arity::from_table(crate::langs::arity::<LANG>, tag)` \
         and nothing else (#202). If the table itself looks wrong, the derivation lives in \
         quilt-conformance/src/arity.rs — fix it there and run `bin/gen-arity`, so every \
         language gets the correction.",
        failures.len(),
        failures.join("\n"),
    );
}

/// Node kinds bash and zsh share but classify differently, each with the reason
/// the *grammars* give.
///
/// Anything not listed here must classify the same way in both.
const SHELL_DIVERGENCES: &[(&str, &str)] = &[(
    "function_definition",
    "zsh's rule is `repeat1(field('name', …))` — `function a b c { … }` defines \
     three functions at once, which bash has no syntax for",
)];

/// Bash and zsh must classify every node kind their grammars *share* the same
/// way, unless the grammars themselves disagree.
///
/// The two are documented as near-equivalent — `concrete-languages.md` says of
/// bash: *"Same as Zsh — a separate target with Bash-specific quoting
/// semantics"* — and zsh's grammar is a fork of bash's. But their `arity` tables
/// were once maintained by hand and independently, and had drifted by eleven
/// tags: `for_statement`, `while_statement`, `function_definition`,
/// `test_command`, `variable_assignment`, `c_style_for_statement`,
/// `file_redirect`, `raw_string`, `ansi_c_string`, `subscript` and
/// `ternary_expression` were variadic in bash and not in zsh. Arity decides
/// whether the expander treats a node as a container to emit into, so an emit
/// into a zsh `for` body behaved differently from the identical bash one, with
/// no diagnostic (#150).
///
/// Both tables are now derived from their own grammar (#202), which is the
/// shared source of truth #150 was reaching for and a better one than either
/// hand-written list — the reconciliation had moved the two *toward each other*
/// and away from what either grammar said, giving zsh five tags (`raw_string`,
/// `subscript`, `test_command`, …) that have no repeat in the zsh grammar at
/// all, purely because bash declared them.
///
/// So this no longer prevents drift; it reports where the forks really parted.
/// A shared kind that classifies differently is now either a genuine grammar
/// difference — in which case it belongs in [`SHELL_DIVERGENCES`] with its
/// reason — or a sign that one fork was bumped and the other was not, which is
/// exactly what wants a human's eye.
///
/// Restricting to the shared kinds is what makes this a real invariant rather
/// than a wish: it says nothing about the constructs only one shell has (zsh's
/// `repeat_statement`, bash's `simple_expansion`), so a legitimate
/// grammar-specific tag never trips it.
#[test]
fn bash_and_zsh_agree_on_shared_kinds() {
    let bash_grammar = registry::grammar("bash").expect("bash grammar");
    let zsh_grammar = registry::grammar("zsh").expect("zsh grammar");
    let bash = registry::language("bash").expect("bash language builds");
    let zsh = registry::language("zsh").expect("zsh language builds");

    let shared: BTreeSet<&str> = registry::node_kinds(&bash_grammar)
        .intersection(&registry::node_kinds(&zsh_grammar))
        .copied()
        .collect();

    let mut disagreements = Vec::new();
    let mut unused_reasons: BTreeSet<&str> =
        SHELL_DIVERGENCES.iter().map(|(kind, _)| *kind).collect();
    for kind in shared {
        let (b, z) = (bash.arity(kind), zsh.arity(kind));
        if b == z {
            continue;
        }
        if unused_reasons.remove(kind) {
            continue;
        }
        disagreements.push(format!("  {kind}: bash {b:?}, zsh {z:?}"));
    }

    assert!(
        disagreements.is_empty(),
        "bash and zsh disagree on the arity of {} node kind(s) that both grammars define, \
         with no recorded reason:\n{}\n\n\
         Both tables are generated from their own grammar by `bin/gen-arity`, so this is \
         a claim about the two forks, not about the tables. Read the rules in \
         quilt/grammars/{{bash,zsh}}/grammar.json: if they really differ, add the kind to \
         SHELL_DIVERGENCES in this file with the reason; if they do not, one fork was \
         bumped without the other (#150).",
        disagreements.len(),
        disagreements.join("\n"),
    );

    assert!(
        unused_reasons.is_empty(),
        "SHELL_DIVERGENCES records {} node kind(s) that bash and zsh now classify the same \
         way: {}\n\n\
         A grammar bump closed the gap, so the entry is stale — drop it, and take the \
         chance to check the two forks did not converge by accident.",
        unused_reasons.len(),
        unused_reasons
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .join(", "),
    );
}
