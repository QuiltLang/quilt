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
fn ground_stmt_quote_emits() -> Result<()> {
    // A quoted statement in statement position of ground code would previously
    // build a term and silently drop it; it is now emitted into the enclosing
    // builder, same as writing `.←;` explicitly.
    let out = expand_both(indoc! {r#"
        let p = ↖{
            ↙{
                ↖println!("hi");↗
            }↘
        }↗;
    "#})?;
    assert!(out.contains(".b().emit(&mut b_);"), "{out}");
    Ok(())
}

#[test]
fn ground_tail_quote_stays_value() -> Result<()> {
    // A tail-expression quote parses with the same outer tag as a
    // statement-position one (`expression_statement`), but its body is an
    // expression: it must remain the block's value, not get emitted.
    let out = expand_both("fn f() -> Arc<QTerm> {\n    ↖1 + 2↗\n}")?;
    assert!(!out.contains(".emit("), "{out}");
    Ok(())
}

#[test]
fn ground_unit_unquote_spliced() -> Result<()> {
    // An unquote whose ground body is a statement-shaped block (imperative
    // emits, unit value) is spliced as a plain statement instead of the
    // `{...}.emit(&mut b_);` unit-emit workaround.
    let out = expand_both(indoc! {r#"
        let p = ↖{
            ↙{
                for i in 0..3 {
                    ↖println!("hi");↗.←;
                }
            }↘
        }↗;
    "#})?;
    assert!(!out.contains("}.emit(&mut b_);"), "{out}");
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
