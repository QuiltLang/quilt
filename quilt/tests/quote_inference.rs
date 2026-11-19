//! Which language a bare `↖…↗` is. An annotation names it; a bare quote
//! takes the annotation of the last annotated quote before it at the same
//! level, then the enclosing quote's language (from inside its unquote), then
//! the file's extension chain, then the host. Each test expands the bare
//! spelling and the same code with every annotation written out, and asserts
//! they agree.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/// Expand `code` under the extension `chain`, coparsed.
fn expand(chain: &[&str], code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(chain, code)?;
    assert_eq!(code, q.coparse_quilt(), "parsing loses nothing");
    Ok(omni.expand_lang(chain[0], &q)?.coparse())
}

fn same(chain: &[&str], bare: &str, explicit: &str) -> Result<()> {
    let (b, e) = (expand(chain, bare)?, expand(chain, explicit)?);
    assert_eq!(b, e, "\n--- bare:\n{bare}\n--- explicit:\n{explicit}");
    Ok(())
}

/// The case the rule is for: one annotation, then bare quotes after it.
#[test]
fn a_bare_quote_after_an_annotated_one_inherits_its_language() -> Result<()> {
    same(
        &["rs"],
        indoc! {r#"
            fn f() {
                let a = py↖x + 1↗;
                let b = ↖[y]↗;
            }
            fn g() -> Arc<QTerm> {
                ↖z + 2↗
            }
        "#},
        indoc! {r#"
            fn f() {
                let a = py↖x + 1↗;
                let b = py↖[y]↗;
            }
            fn g() -> Arc<QTerm> {
                py↖z + 2↗
            }
        "#},
    )
}

/// Each annotation takes over from the one before it.
#[test]
fn a_later_annotation_takes_over() -> Result<()> {
    same(
        &["rs"],
        "fn f() { (py↖x + 1↗, ↖[y]↗, rs↖1 + 2↗, ↖[z]↗); }\n",
        "fn f() { (py↖x + 1↗, py↖[y]↗, rs↖1 + 2↗, rs↖[z]↗); }\n",
    )
}

/// Before any annotation, a bare quote is the host's (here Rust's), as before.
#[test]
fn a_bare_quote_before_any_annotation_is_the_host() -> Result<()> {
    same(
        &["rs"],
        "fn f() { (↖[y]↗, py↖x + 1↗); }\n",
        "fn f() { (rs↖[y]↗, py↖x + 1↗); }\n",
    )
}

/// Only siblings count: an annotation inside an unquote is a level of its
/// own, and does not reach the quotes after its enclosing quote.
#[test]
fn an_annotation_in_a_nested_level_does_not_leak_out() -> Result<()> {
    same(
        &["rs"],
        "fn f() { (↖g(↙py↖[x]↗↘)↗, ↖[y]↗); }\n",
        "fn f() { (rs↖g(↙py↖[x]↗↘)↗, rs↖[y]↗); }\n",
    )
}

/// An inherited language is the quote's language all the way down: a bare
/// quote inside its unquote resolves to it, as inside an annotated one.
#[test]
fn an_inherited_language_reaches_into_the_quote() -> Result<()> {
    same(
        &["rs"],
        "fn f() { (py↖[x]↗, ↖g(↙↖[y]↗↘)↗); }\n",
        "fn f() { (py↖[x]↗, py↖g(↙py↖[y]↗↘)↗); }\n",
    )
}

/// The sibling rule comes before the extension chain: in
/// `shaders.wgsl.rs.quilt`, a bare quote is WGSL until an annotation says
/// otherwise, and is that annotation's language after it.
#[test]
fn a_sibling_annotation_comes_before_the_chain_default() -> Result<()> {
    same(
        &["rs", "wgsl"],
        indoc! {r#"
            fn f(y: &Arc<QTerm>) {
                let a = ↖bitcast<u32>(↙y↘)↗;
                let b = rs↖1 + 2↗;
                let c = ↖[z]↗;
            }
        "#},
        indoc! {r#"
            fn f(y: &Arc<QTerm>) {
                let a = wgsl↖bitcast<u32>(↙y↘)↗;
                let b = rs↖1 + 2↗;
                let c = rs↖[z]↗;
            }
        "#},
    )
}
