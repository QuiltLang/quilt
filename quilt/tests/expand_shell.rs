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
//! `bash_and_zsh_agree_on_shared_kinds` in
//! `quilt-conformance/tests/grammar_tags.rs`, which holds the two tables to
//! agreement on every node kind both grammars define.
//!
//! One subtlety decides how the fragments below are written. Since #180 a
//! variadic node with **no unquote among its direct children** builds fluently
//! anyway — the accumulator only buys something when a child can contribute
//! zero-or-many terms, and only an unquote can. So `for x in a b; do ↙body↘ done`
//! no longer distinguishes the two arities: the hole is inside the `do_group`,
//! and the `for_statement` around it emits the same fluent chain either way.
//! Each fragment therefore puts the hole **directly inside** the container under
//! test, which is the position where arity is still observable.

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
/// Each parses in both shells and puts a hole *directly* inside the container —
/// in its word list, condition or name rather than its body — so the container's
/// own arity, not its body's, decides the shape of the generated code.
const SHARED_CONTAINERS: &[(&str, &str)] = &[
    ("for_statement", "for x in ↙items↘; do\n    echo hi\ndone"),
    ("while_statement", "while ↙cond↘; do\n    echo hi\ndone"),
    (
        "c_style_for_statement",
        "for ((↙init↘; i<3; i++)); do\n    echo hi\ndone",
    ),
    ("file_redirect", "echo hi > ↙f↘"),
    // Not reconciled by #150 — already variadic in both — but cheap to pin as
    // the control group.
    ("if_statement", "if true; then\n    ↙body↘\nfi"),
    ("case_statement", "case ↙x↘ in\n    a) echo a ;;\nesac"),
];

/// Constructs #150 reconciled as variadic that the grammars say are *not*, with
/// the rule that settles each. Since #202 the tables are derived from the
/// grammars, so these expand as fixed-arity nodes in both shells.
///
/// They are pinned here rather than deleted because the reconciliation made a
/// specific claim about them, and "the grammar disagreed" is the answer — worth
/// keeping visible, and worth failing on if it silently reverses.
///
/// `bash::function_definition` is absent from *both* lists: it is the one shared
/// kind where the grammars genuinely differ, so it is fixed-arity in bash and
/// variadic in zsh. See `SHELL_DIVERGENCES` in
/// `quilt-conformance/tests/grammar_tags.rs`.
const SHARED_LEAVES: &[(&str, &str)] = &[
    // `X=↙v↘` — one name, one value. The only repeat the rule can reach is
    // inside the `word` an alias puts the value in.
    ("variable_assignment", "X=↙v↘"),
    // `[[ ↙cond↘ ]]` — the repeat belongs to the `binary_expression` the
    // condition parses as, not to the test.
    ("test_command", "[[ ↙cond↘ ]]"),
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

/// The constructs the grammars do not give a repeated child expand as
/// fixed-arity nodes, in both shells (issue #202).
///
/// The mirror of the test above, and the reason it is worth having: an emit into
/// one of these was never going to append a sequence — the tree has nowhere to
/// put it — so treating it as a container was a claim the shells could not keep.
#[test]
fn shared_leaves_are_fixed_arity_in_both_shells() -> Result<()> {
    let mut wrong = Vec::new();

    for (tag, fragment) in SHARED_LEAVES {
        for shell in ["bash", "zsh"] {
            let out = expand_in(shell, fragment)?;
            if is_variadic_form(&out, tag) {
                wrong.push(format!(
                    "  {shell} {tag}: expected the fixed-arity .c(&…) chain, got:\n{out}"
                ));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "{} shell construct(s) expanded as variadic that their grammars give no \
         repeated child (issue #202):\n\n{}\n\n\
         The tables come from `bin/gen-arity`; if the grammar really did gain a repeat \
         here, move the tag to SHARED_CONTAINERS and update conformance/spec/{{bash,zsh}}.toml.",
        wrong.len(),
        wrong.join("\n"),
    );
    Ok(())
}

/// Zsh's `function_definition` is variadic and bash's is not, because the
/// grammars differ: zsh's rule is `repeat1(field('name', …))`, so
/// `function a b c { … }` defines three functions at once and bash has no such
/// syntax.
///
/// #150 reconciled the two tables by hand and made both variadic, which is how a
/// bash construct came to be treated as a container on the strength of zsh's
/// grammar. Deriving each from its own grammar (#202) separates them again.
#[test]
fn function_definition_follows_each_shell_grammar() -> Result<()> {
    let fragment = "↙name↘() {\n    echo hi\n}";

    let bash = expand_in("bash", fragment)?;
    assert!(
        !is_variadic_form(&bash, "function_definition"),
        "bash's function_definition takes exactly one name, so it is not a container; got:\n{bash}"
    );

    let zsh = expand_in("zsh", fragment)?;
    assert!(
        is_variadic_form(&zsh, "function_definition"),
        "zsh's function_definition takes repeat1(name), so it is a container; got:\n{zsh}"
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
/// silent (issue #157). Zsh's shape is pinned to this one by
/// `for_loop_expands_identically_across_shells` above, modulo the loop-variable
/// leaf kind.
///
/// Note the two forms side by side: `for_statement` holds no unquote directly,
/// so since #180 it builds fluently, while the `do_group` that does hold `body`
/// keeps the accumulator and the `body.emit(&mut b_)` that #150 is about.
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
