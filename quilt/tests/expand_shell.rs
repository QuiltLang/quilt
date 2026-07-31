//! bash and zsh as *target* languages: `bash↖ … ↗` / `zsh↖ … ↗` fragments
//! embedded in a Rust host, expanded by the Rust `MetaLanguage`.
//!
//! `tree-sitter-zsh` is a fork of `tree-sitter-bash`, so the same shell source
//! must expand the same way in both — which it did not, before issue #150. The
//! two `Language::arity` tables were maintained separately and drifted across
//! twelve node kinds, including `for_statement`, `while_statement`,
//! `function_definition`, `test_command`, `c_style_for_statement`,
//! `file_redirect`, `raw_string`, `ansi_c_string`, `subscript` and
//! `ternary_expression` — every one of which the zsh grammar also defines and
//! parses to a structurally identical tree.
//!
//! What that changed is visible in the generated builder code, which is why the
//! assertions below can be cheap string checks. A *variadic* container expands
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
//! So an emit into a zsh `for` generated code referencing a `b_` that was never
//! declared — a compile error in the *generated* file, from source that expanded
//! fine as bash, with no diagnostic at expansion time.
//!
//! `quilt::langs::shell` is now the one table both providers answer from;
//! `quilt-conformance`'s `shell_dialects_agree_on_shared_kinds` guards the table,
//! and these tests guard the behaviour it produces.
//!
//! One subtlety decides how the fragments below are written. Since #180 a
//! variadic node with **no unquote among its direct children** builds fluently
//! anyway — the accumulator only buys something when a child can contribute
//! zero-or-many terms, and only an unquote can. So `for x in a b; do ↙body↘ done`
//! no longer distinguishes the two arities at the `for_statement`: the hole is
//! inside the `do_group`, and the `for_statement` around it emits the same fluent
//! chain either way. Each fragment therefore puts the hole **directly inside**
//! the container under test, which is the position where arity is still
//! observable.

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

/// The same host program, once per dialect.
fn both_dialects(template: &str) -> Result<(String, String)> {
    Ok((
        expand(&template.replace("SHELL", "bash"))?,
        expand(&template.replace("SHELL", "zsh"))?,
    ))
}

/// Did `tag` expand to the variadic accumulator form rather than the
/// fixed-arity `.c(&…)` chain? See the module docs.
fn is_variadic_form(out: &str, tag: &str) -> bool {
    out.contains(&format!("let mut b_ = tb({tag:?})"))
}

/// Erase the spellings that genuinely differ between the grammars, so a diff is
/// about arity rather than about node naming: zsh calls bash's `variable_name`
/// `simple_variable_name`, and bash's `simple_expansion` (`$f`) `variable_ref`.
fn normalize(src: &str) -> String {
    src.replace("simple_variable_name", "variable_name")
        .replace("simple_expansion", "VAR_EXPANSION")
        .replace("variable_ref", "VAR_EXPANSION")
        .replace("Bash", "SHELL")
        .replace("Zsh", "SHELL")
}

/// Fragments exercising the constructs #150 reconciled, one per container tag.
/// Each parses in both shells and puts a hole *directly* inside the container —
/// in its word list, condition or name rather than its body — so the container's
/// own arity, not its body's, decides the shape of the generated code.
const SHARED_CONTAINERS: &[(&str, &str)] = &[
    ("for_statement", "for x in ↙items↘; do\n    echo hi\ndone"),
    ("while_statement", "while ↙cond↘; do\n    echo hi\ndone"),
    ("function_definition", "↙name↘() {\n    echo hi\n}"),
    (
        "c_style_for_statement",
        "for ((↙init↘; i<3; i++)); do\n    echo hi\ndone",
    ),
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

const FOR_LOOP: &str = indoc! {r#"
    fn script(words: &[String]) -> Arc<QTerm> {
        SHELL↖
            for f in ↙{ for w in words { SHELL↖↙w.↑↘↗.←; } }↘
            do
                echo $f
            done
        ↗
    }
"#};

/// The regression #150 describes, end to end: `for_statement` must be a variadic
/// container in *both* dialects, so `←` into a `for` word list has a `b_` to
/// append to.
///
/// Before the fix zsh expanded this to `tb("for_statement").c(…).w(…)`, leaving
/// the `.emit(&mut b_)` calls from the spliced Rust block with no accumulator in
/// scope.
#[test]
fn for_statement_is_a_variadic_container_in_both_dialects() -> Result<()> {
    let (bash, zsh) = both_dialects(FOR_LOOP)?;
    for (name, out) in [("bash", &bash), ("zsh", &zsh)] {
        assert!(
            out.contains(r#"let mut b_ = tb("for_statement")"#),
            "{name} expanded `for` to a fixed-arity node, so `←` into its word \
             list has no `b_` to emit into:\n{out}",
        );
    }
    Ok(())
}

/// Same source, same expansion — the property the shared table exists to give.
#[test]
fn the_same_shell_source_expands_identically_in_both_dialects() -> Result<()> {
    let (bash, zsh) = both_dialects(FOR_LOOP)?;
    assert_eq!(
        normalize(&bash),
        normalize(&zsh),
        "bash and zsh expanded the same source differently; the only permitted \
         differences are the grammars' own node names, which `normalize` erases",
    );
    Ok(())
}

/// The same `for` loop expands to the same builder shape in both shells, with
/// the hole in the loop *body* rather than the word list.
///
/// The one legitimate difference is the loop variable's leaf kind — bash's
/// grammar calls it `variable_name`, zsh's `simple_variable_name` — so that is
/// normalised away. Everything else must match.
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

/// `do_group` was variadic in both all along, so an emit into a loop *body*
/// worked before the fix too. Pinned so the shared table cannot regress it while
/// fixing the word-list case.
#[test]
fn emit_into_a_loop_body_works_in_both_dialects() -> Result<()> {
    let (bash, zsh) = both_dialects(indoc! {r#"
        fn script(cmds: &[Arc<QTerm>]) -> Arc<QTerm> {
            SHELL↖
                for f in *.txt
                do
                    ↙{ for c in cmds { c.←; } }↘
                done
            ↗
        }
    "#})?;
    for (name, out) in [("bash", &bash), ("zsh", &zsh)] {
        assert!(
            out.contains(r#"let mut b_ = tb("do_group")"#),
            "{name} expanded the loop body to a fixed-arity node:\n{out}",
        );
    }
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

/// The other half of #150: bash's table claimed *both* `variable_assignment`
/// (singular) and `variable_assignments` (plural). Only the plural is a
/// sequence — `X=1 Y=2` — while the singular is a fixed name/`=`/value triple,
/// so declaring it variadic wrapped every `X=…` in a `b_` block that could only
/// ever take one child. zsh never claimed it; bash no longer does either.
#[test]
fn only_the_plural_assignment_node_is_a_container() -> Result<()> {
    let (bash, zsh) = both_dialects(indoc! {r#"
        fn script(v: &Arc<QTerm>) -> Arc<QTerm> {
            SHELL↖X=↙v↘↗
        }
    "#})?;
    for (name, out) in [("bash", &bash), ("zsh", &zsh)] {
        assert!(
            out.contains(r#"tb("variable_assignment")"#),
            "{name} did not build a `variable_assignment` at all:\n{out}",
        );
        assert!(
            !out.contains(r#"let mut b_ = tb("variable_assignment")"#),
            "{name} still treats the singular `variable_assignment` as a \
             variadic container:\n{out}",
        );
    }
    Ok(())
}

/// Shell string fragments of each flavour round-trip through `coparse`.
///
/// Nothing covered bash/zsh round-tripping before this file. This does not
/// exercise the `is_expr_tag` alignment that the shared table also carries: the
/// `InnerKind` it computes is currently discarded by the only caller
/// (`let (qterm, _ikind) = self.provider.unwrap(…)` in `treesitter.rs`), so that
/// divergence is latent rather than observable.
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
