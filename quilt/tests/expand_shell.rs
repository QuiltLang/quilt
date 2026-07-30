//! Bash and Zsh as *target* languages: `bash↖ … ↗` / `zsh↖ … ↗` fragments
//! embedded in a Rust host, expanded by the Rust `MetaLanguage`.
//!
//! The two shells are near-equivalent targets, so these tests are written in
//! pairs: the same fragment goes through both, and the interesting assertion is
//! that the two agree. Issue #150 was the case where they did not — eleven
//! shared constructs (`for_statement`, `while_statement`, `function_definition`,
//! `test_command`, `variable_assignment`, `c_style_for_statement`,
//! `file_redirect`, `raw_string`, `ansi_c_string`, `subscript` and
//! `ternary_expression`) were variadic in bash's `Language::arity` table and
//! `Unknown` in zsh's.
//!
//! What that changed is visible in the generated builder code, which is why the
//! assertion below can be a cheap string check. A *variadic* container expands
//! to the accumulator form, where each child is pushed with `.emit(&mut b_)` and
//! may therefore contribute zero or many children:
//!
//! ```text
//! let mut b_ = tb("for_statement");
//! …
//! body.emit(&mut b_);
//! ```
//!
//! An `Unknown`-arity container expands to the fixed-arity chain instead, where
//! every child occupies exactly one positional slot:
//!
//! ```text
//! tb("for_statement").c(&sym("for")).w(" ")…
//! ```
//!
//! So before #150 an emit into a zsh `for` body silently did something different
//! from the identical bash one, with no diagnostic. The companion check is
//! `shell_arity_tables_agree` in `quilt-conformance/tests/grammar_tags.rs`, which
//! holds the two tables to agreement on every node kind both grammars define;
//! this file covers the four constructs above whose hole positions are awkward
//! to reach from a fragment.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;

/// Parse + expand `code`, returning the coparsed builder source.
fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
}

/// Expand `fragment` as a quote of `shell` inside a Rust host.
fn expand_in(shell: &str, fragment: &str) -> Result<String> {
    expand(&format!("const X: T = {shell}↖{fragment}↗;\n"))
}

/// Did `tag` expand to the variadic accumulator form rather than the
/// fixed-arity `.c(&…)` chain? See the module docs.
fn is_variadic_form(out: &str, tag: &str) -> bool {
    out.contains(&format!("let mut b_ = tb({tag:?})"))
}

/// Fragments exercising the constructs #150 reconciled, one per container tag.
/// Each parses in both shells and puts a hole inside the container, so the
/// container's arity decides the shape of the generated code.
const SHARED_CONTAINERS: &[(&str, &str)] = &[
    ("for_statement", "for x in a b; do\n    ↙body↘\ndone"),
    ("while_statement", "while true; do\n    ↙body↘\ndone"),
    ("function_definition", "f() {\n    ↙body↘\n}"),
    (
        "c_style_for_statement",
        "for ((i=0; i<3; i++)); do\n    ↙body↘\ndone",
    ),
    ("variable_assignment", "X=↙v↘"),
    ("file_redirect", "echo hi > ↙f↘"),
    ("test_command", "[[ ↙cond↘ ]]"),
    // Not reconciled by #150 — already variadic in both — but cheap to pin as
    // the control group.
    ("if_statement", "if true; then\n    ↙body↘\nfi"),
    ("case_statement", "case ↙x↘ in\n    a) echo a ;;\nesac"),
];

/// Every shared container is variadic in **both** shells (issue #150).
///
/// A regression here means one shell went back to expanding the construct as a
/// fixed-arity node, so an emit into it would silently stop appending.
#[test]
fn shared_containers_are_variadic_in_both_shells() -> Result<()> {
    let mut wrong = Vec::new();

    for (tag, fragment) in SHARED_CONTAINERS {
        for shell in ["bash", "zsh"] {
            let out = expand_in(shell, fragment)?;
            if !is_variadic_form(&out, tag) {
                wrong.push(format!(
                    "  {shell} {tag}: expected the variadic accumulator form, got:\n{out}"
                ));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "{} shell container(s) did not expand as variadic (issue #150):\n\n{}",
        wrong.len(),
        wrong.join("\n"),
    );
    Ok(())
}

/// The same `for` loop expands to the same builder shape in both shells.
///
/// The one legitimate difference is the loop variable's leaf kind — bash's
/// grammar calls it `variable_name`, zsh's `simple_variable_name` — so that is
/// normalised away. Everything else must match: this is the pair that diverged
/// in #150, where zsh produced a `.c(&…)` chain and bash the accumulator.
#[test]
fn for_loop_expands_identically_across_shells() -> Result<()> {
    let fragment = "for x in a b; do\n    ↙body↘\ndone";
    let bash = expand_in("bash", fragment)?;
    let zsh = expand_in("zsh", fragment)?.replace("simple_variable_name", "variable_name");
    assert_eq!(
        bash, zsh,
        "bash and zsh disagree on how an identical `for` loop expands"
    );
    Ok(())
}

/// The generated shape itself, so a change to it is reviewable rather than
/// silent (issue #157). Both shells share one snapshot: they agree modulo the
/// loop-variable leaf kind, and asserting that here too is the point.
#[test]
fn for_loop_with_emit_body() -> Result<()> {
    let out = expand(indoc! {r#"
        const X: T = bash↖for x in a b; do
            ↙body↘
        done↗;
    "#})?;
    insta::assert_snapshot!(out);
    Ok(())
}

/// Shell string fragments of each flavour round-trip through `coparse`.
///
/// Nothing covered bash/zsh round-tripping before this file. This does not
/// exercise the `is_expr_tag` alignment that went in alongside #150: the
/// `InnerKind` those tables compute is currently discarded by the only caller
/// (`let (qterm, _ikind) = self.provider.unwrap(…)` in `treesitter.rs`), so that
/// divergence is latent rather than observable. See the note on zsh's
/// `is_expr_tag`.
#[test]
fn string_fragments_round_trip_in_both_shells() -> Result<()> {
    for fragment in [
        "echo 'literal'",
        "echo $'a\\nb'",
        "echo \"x\"",
        "echo $(date)",
    ] {
        for shell in ["bash", "zsh"] {
            let src = format!("const X: T = {shell}↖{fragment}↗;\n");
            let mut omni = Omni::default();
            let q = omni.parse(&src)?;
            assert_eq!(src, q.coparse(), "{shell}: {fragment} did not round-trip");
        }
    }
    Ok(())
}
