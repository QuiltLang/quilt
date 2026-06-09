//! The production `RustMetaLanguage` (via `Omni`) must expand to exactly the same
//! code as the reference `BootstrapMetaLanguage` (via `Bootstrap`). These tests
//! expand the same inputs through both engines and assert identical output, so we
//! get a fast feedback loop without invoking `rust-script`.

use indoc::indoc;
use quilt::langs::bootstrap::Bootstrap;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;
use std::ops::Range;

/// Expand `code` with both engines, assert identical output, and return it.
fn expand_both(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let oq = omni.parse(code)?;
    let omni_out = omni.expand(&oq)?.coparse();

    let mut bootstrap = Bootstrap::default();
    let bq = bootstrap.parse(code)?;
    let boot_out = bootstrap.expand(&bq)?.coparse();

    assert_eq!(
        omni_out, boot_out,
        "RustMetaLanguage output differs from BootstrapMetaLanguage"
    );
    Ok(omni_out)
}

#[test]
fn simple() -> Result<()> {
    let out = expand_both("let expr = ↖1 + 2↗;")?;
    assert_eq!(
        out,
        r#"let expr = tb("binary_expression").c(&leaf("integer_literal", "1")).w(" ").c(&sym("+")).w(" ").c(&leaf("integer_literal", "2")).b();"#
    );
    Ok(())
}

#[test]
fn quote_expr() -> Result<()> {
    let out = expand_both("↖1 + 2↗")?;
    println!("{out}");
    Ok(())
}

#[test]
fn variadic() -> Result<()> {
    let out = expand_both(indoc! {r#"
        ↖fn foo() {
            println!("Hello");
            println!("World");
        }↗
    "#})?;
    println!("{out}");
    Ok(())
}

#[test]
fn splicing() -> Result<()> {
    let out = expand_both(indoc! {r#"{
        fn mk(i: usize) -> Result<⟨T⟩> {
            Ok(↖{
                ↙{
                    for c in 0..i {
                        if c != 0 {NL.←;}
                        ↖println!("hi");↗.←;
                    }
                }↘
            }↗)
        }
        mk(3).unwrap()
    }"#})?;
    println!("{out}");
    Ok(())
}

#[test]
fn reduce() -> Result<()> {
    let mut omni = Omni::default();
    let code = "3..5";
    let qterm = omni.parse(code)?;
    let reduced: Range<i32> = qterm.reduce()?;
    assert_eq!(reduced, 3..5);
    Ok(())
}

#[test]
fn splicing_nested() -> Result<()> {
    let out = expand_both(indoc! {r#"{
        fn mk(i: usize) -> Result<⟨T⟩> {
            Ok(↖{
                ↙{
                    for c in 0..i {
                        {
                            if c != 0 {NL.←;}
                            ↖println!("hi");↗
                        }.←;
                    }
                }↘
            }↗)
        }
        mk(3).unwrap()
    }"#})?;
    println!("{out}");
    Ok(())
}
