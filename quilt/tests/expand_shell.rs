//! bash and zsh are *target* languages: `bash↖ … ↗` / `zsh↖ … ↗` fragments
//! embedded in a Rust host, expanded by the Rust `MetaLanguage`.
//!
//! `tree-sitter-zsh` is a fork of `tree-sitter-bash`, so the same shell source
//! must expand the same way in both — which it did not, before issue #150. The
//! two `Language::arity` tables were maintained separately and drifted across
//! twelve node kinds, including `for_statement`. Since arity is what decides
//! between `build_variadic_block` (a `b_` accumulator that `←` appends into) and
//! `build_tuple_code` (fixed positional children), an emit into a zsh `for`
//! generated code referencing a `b_` that was never declared — a compile error
//! in the *generated* file, from source that compiled fine as bash.
//!
//! `quilt::langs::shell` is now the one table both providers answer from;
//! `quilt-conformance`'s `shell_dialects_agree_on_shared_kinds` guards the table,
//! and these tests guard the behaviour it produces.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;

/// Parse + expand `code`, returning the coparsed builder source.
fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
}

/// The same host program, once per dialect.
fn both_dialects(template: &str) -> Result<(String, String)> {
    Ok((
        expand(&template.replace("SHELL", "bash"))?,
        expand(&template.replace("SHELL", "zsh"))?,
    ))
}

/// Erase the two spellings that genuinely differ between the grammars, so a
/// diff is about arity rather than about node naming: zsh calls bash's
/// `variable_name` `simple_variable_name`, and bash's `simple_expansion`
/// (`$f`) `variable_ref`.
fn normalize(src: &str) -> String {
    src.replace("simple_variable_name", "variable_name")
        .replace("simple_expansion", "VAR_EXPANSION")
        .replace("variable_ref", "VAR_EXPANSION")
        .replace("Bash", "SHELL")
        .replace("Zsh", "SHELL")
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

/// The regression #150 describes: `for_statement` must be a variadic container
/// in *both* dialects, so `←` into a `for` word list has a `b_` to append to.
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
