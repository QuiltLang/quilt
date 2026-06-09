//! try all 2^3 locations for newlines

use indoc::indoc;
use pretty_assertions::assert_eq;
use quilt::{langs::omni::Omni, prelude::*};

/**************************************************************/

fn roundtrip(code: &str) -> Result<()> {
    let code = code.trim();
    let mut omni = Omni::default();
    let term = omni.parse(code)?;

    println!("'{}'", term.coparse());
    assert_eq!(code, term.coparse(), "code should roundtrip");

    println!("'{}'", term[5].coparse());
    assert!(
        code.contains(&term[5].coparse()),
        "code should naively contain the quote because it isn't indented"
    );

    println!("'{}'", term[5][0].coparse());
    assert!(
        !term[5][0].coparse().contains("\npass"),
        "the pass should be indented"
    );

    Ok(())
}

// IDEA: should we change where newlines are captured? ex: should t001 having a trailing newline inside the quote?

#[test]
fn t000() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖def foo(): pass↗;
    "#})
}

#[test]
fn t001() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖def foo(): pass
        ↗;
    "#})
}

#[test]
fn t010() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖def foo():
            pass↗;
    "#})
}

#[test]
fn t011() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖def foo():
            pass
        ↗;
    "#})
}

#[test]
fn t100() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖
            def foo(): pass↗;
    "#})
}

#[test]
fn t101() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖
            def foo(): pass
        ↗;
    "#})
}

#[test]
fn t110() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖
            def foo():
                pass↗;
    "#})
}

#[test]
fn t111() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖
            def foo():
                pass
        ↗;
    "#})
}
