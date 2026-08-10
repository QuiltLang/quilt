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
//! Each provider now answers from a table derived from *its own* grammar
//! (`bin/gen-arity`, issue #202), which is why the two agree: a construct the
//! grammars spell the same way classifies the same way because the grammars do,
//! not because both read one hand-written list. `quilt-conformance`'s
//! `bash_and_zsh_agree_on_shared_kinds` guards that, pinning the one place the
//! forks genuinely part company, and these tests guard the behaviour it
//! produces.
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

/// Same source, same expansion — the property #150 was after, now reached by
/// deriving each dialect's table from its own grammar rather than by sharing one.
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
/// worked before the fix too. Pinned so a change to the derived tables cannot
/// regress it while fixing the word-list case.
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

/// `((` inside a double-quoted string is literal text — in both shells
/// (issue #212).
///
/// zsh's grammar offered the bare `((…))` arithmetic *command* opener as an
/// alternative inside `string`, so the `((` token was in the lexer's valid set
/// at every position in a string and won the same-length tie against
/// `string_content`, whose `token(prec(-1, …))` loses it. `echo "(("` was a
/// parse error, and so was every string a Rust `↑` lifted that happened to
/// contain `((` — silently, since the lift itself succeeded. Bash, sharing the
/// lineage, was never affected, which is what made this a grammar bug rather
/// than a hole in `sh_dquote_escape`.
///
/// The two halves below are the whole shape of the fix. A `$`-less `((` is
/// content; the `$`-sigil forms zsh really does expand inside a string still
/// parse *as* an `arithmetic_expansion`, because restricting `string` to them is
/// the fix rather than dropping arithmetic from strings altogether.
#[test]
fn double_parens_in_a_string_are_content_but_dollar_arithmetic_is_not() -> Result<()> {
    for (fragment, want_arith) in [
        ("echo \"((\"", false),
        ("echo \"(())\"", false),
        ("echo \"x = ((a+b))\"", false),
        ("echo \"$((1 + 1))\"", true),
        ("echo \"$[1 + 1]\"", true),
    ] {
        for shell in ["bash", "zsh"] {
            let src = format!("const X: T = {shell}↖{fragment}↗;\n");
            let mut omni = Omni::default();
            let q = omni.parse(&src)?;
            assert_eq!(src, q.coparse(), "{shell}: {fragment} did not round-trip");

            let out = expand_in(shell, fragment)?;
            assert_eq!(
                out.contains(r#"tb("arithmetic_expansion")"#),
                want_arith,
                "{shell}: {fragment} should{} have parsed an arithmetic_expansion:\n{out}",
                if want_arith { "" } else { " not" },
            );
        }
    }
    Ok(())
}

/// The bare `((…))` arithmetic *command*, which the fix deliberately keeps —
/// outside a string it is still an `arithmetic_expansion`, in the two positions
/// zsh allows it (issue #212).
#[test]
fn bare_arithmetic_commands_still_parse_in_zsh() -> Result<()> {
    for fragment in ["(( x > 1 ))", "if (( x > 1 )); then\n    echo hi\nfi"] {
        let out = expand_in("zsh", fragment)?;
        assert!(
            out.contains(r#"tb("arithmetic_expansion")"#),
            "zsh: {fragment} no longer parses as an arithmetic_expansion:\n{out}",
        );
    }
    Ok(())
}
