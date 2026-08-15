//! SQL is a *target* language: `sql↖ … ↗` fragments embedded in a Rust host,
//! expanded by the Rust `MetaLanguage`.
//!
//! The focus here is the two wrapper retries in `langs::sql::lang`, which is
//! where SQL differs from every other target: `program` holds only statements,
//! so neither a bare expression fragment nor a hole at statement position
//! parses on its own, and both are reached by parsing inside a synthetic
//! `SELECT …` and stripping it back out (#219, #234). The stripping is invisible
//! in the result, so what pins it is that the source round-trips and that the
//! shapes which must *not* be accepted still are not.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/// Parse `code` and assert `coparse_quilt` reproduces it exactly.
fn roundtrip(code: &str) -> Result<()> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    assert_eq!(code, q.coparse_quilt());
    Ok(())
}

/// Parse + expand `code`, returning the coparsed builder source.
fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
}

/* ─────────────────────────── fragment shapes ─────────────────────────── */

#[test]
fn roundtrip_statement() -> Result<()> {
    roundtrip("const Q: T = sql↖SELECT id FROM t WHERE org = ↙org↘↗;\n")
}

/// A bare expression: no place for it in `program`, so this only parses because
/// `parse_pre` retries inside `SELECT …`.
#[test]
fn roundtrip_bare_expression() -> Result<()> {
    roundtrip("const P: T = sql↖org = ↙name↘↗;\n")
}

/// A statement that keeps its terminator stays wrapped in `program`, so the `;`
/// survives the round trip.
#[test]
fn roundtrip_terminated_statement() -> Result<()> {
    roundtrip("const Q: T = sql↖SELECT 1;↗;\n")
}

/* ──────────────────── holes at statement position (#234) ──────────────── */

#[test]
fn roundtrip_statement_hole() -> Result<()> {
    roundtrip(indoc! {r#"
        const S: T = sql↖
            SELECT 1;
            ↙stmt↘;
            SELECT 2;
        ↗;
    "#})
}

/// `program` lets the last statement go unterminated, so a hole may too.
#[test]
fn roundtrip_final_statement_hole() -> Result<()> {
    roundtrip(indoc! {r#"
        const S: T = sql↖
            SELECT 1;
            ↙stmt↘
        ↗;
    "#})
}

#[test]
fn roundtrip_several_statement_holes() -> Result<()> {
    roundtrip(indoc! {r#"
        const S: T = sql↖
            ↙a↘;
            SELECT 2;
            ↙b↘;
        ↗;
    "#})
}

/// The hole becomes a direct child of `program`, not a `SELECT` of its own —
/// which is what makes the spliced statement land beside its siblings rather
/// than inside a query. `program` is variadic, so the children are emitted.
#[test]
fn statement_hole_expands_into_the_program() -> Result<()> {
    let out = expand(indoc! {r#"
        const S: T = sql↖
            SELECT 1;
            ↙stmt↘;
        ↗;
    "#})?;
    assert!(
        out.contains("stmt.emit(&mut b_)"),
        "the hole should be emitted into the program builder, got:\n{out}"
    );
    assert!(
        !out.contains("keyword_select\", \"SELECT\").w(\" \").c(&stmt"),
        "no synthetic SELECT may survive around the hole:\n{out}"
    );
    // Exactly the two `SELECT`s the source has — one for `SELECT 1`, none
    // introduced by the wrapper.
    assert_eq!(
        out.matches("\"SELECT\"").count(),
        1,
        "the wrapper's SELECT leaked into the output:\n{out}"
    );
    Ok(())
}

/* ─────────────────── shapes the retries must still refuse ─────────────── */

/// A hole in the middle of a script with no terminator after it is ill-formed
/// SQL whatever fills it — two statements need a separator — so the wrapper
/// must not paper over it.
#[test]
fn unterminated_mid_script_hole_is_still_an_error() {
    let mut omni = Omni::default();
    let err = omni
        .parse(indoc! {r#"
            const S: T = sql↖
                SELECT 1;
                ↙stmt↘
                SELECT 2;
            ↗;
        "#})
        .expect_err("a statement hole with no separator before the next statement must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("__QUILT_HOLE__"),
        "the original parse error should be reported, not a wrapper artefact: {msg}"
    );
}

/// A hole that is *not* alone on its line is not in statement position, so it
/// keeps the ordinary expression treatment. `SELECT ↙col↘, x` parses on its own
/// and must not be rewritten by the statement wrapper.
#[test]
fn expression_hole_on_a_shared_line_is_untouched() -> Result<()> {
    roundtrip("const Q: T = sql↖SELECT ↙col↘, x FROM t↗;\n")
}

/// A `SELECT ↙col↘` the *author* wrote must survive: the stripper matches on
/// hole ordinals it recorded, not on the shape alone.
#[test]
fn authored_select_of_a_hole_survives() -> Result<()> {
    let code = indoc! {r#"
        const S: T = sql↖
            SELECT ↙col↘;
            ↙stmt↘;
        ↗;
    "#};
    roundtrip(code)?;
    let out = expand(code)?;
    // The authored `SELECT` is still there; the synthetic one is not.
    assert_eq!(
        out.matches("\"SELECT\"").count(),
        1,
        "the authored SELECT should survive and no other should appear:\n{out}"
    );
    assert!(
        out.contains("col"),
        "the authored select's hole is gone:\n{out}"
    );
    Ok(())
}
