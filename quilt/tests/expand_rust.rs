//! The production `RustMetaLanguage` (via `Omni`) must expand to exactly the same
//! code as the reference `BootstrapMetaLanguage` (via `Bootstrap`). These tests
//! expand the same inputs through both engines and assert identical output, so we
//! get a fast feedback loop without invoking `rust-script`.
//!
//! Two oracles, doing different jobs (#157):
//!
//! * **The differential check** in `expand_both` — Omni against Bootstrap — is a
//!   real assertion. It validates *semantics* while being indifferent to
//!   spelling, so it keeps working across a refactor that invalidates every
//!   snapshot below.
//! * **The snapshots** pin the exact generated code. They are what makes a
//!   deliberate change to builder spelling a bulk `cargo insta review` instead
//!   of N hand-edited string literals — and they gave the four tests that
//!   previously only `println!`ed their output a real assertion for free.
//!
//! Semantic results (a reduced value, a `qmatch` binding) and negative
//! invariants ("this must *not* emit") stay as ordinary assertions: they state
//! something that must be true, not something that merely is true today.

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
    insta::assert_snapshot!(expand_both("let expr = ↖1 + 2↗;")?);
    Ok(())
}

#[test]
fn quote_expr() -> Result<()> {
    insta::assert_snapshot!(expand_both("↖1 + 2↗")?);
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
    insta::assert_snapshot!(out);
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
    insta::assert_snapshot!(out);
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
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn ground_tail_quote_stays_value() -> Result<()> {
    // A tail-expression quote parses with the same outer tag as a
    // statement-position one (`expression_statement`), but its body is an
    // expression: it must remain the block's value, not get emitted.
    let out = expand_both("fn f() -> Arc<QTerm> {\n    ↖1 + 2↗\n}")?;
    assert!(!out.contains(".emit("), "{out}");
    insta::assert_snapshot!(out);
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
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn pattern_let() -> Result<()> {
    // A quote in the binding position of a `let` is a pattern (issue #18):
    // its ground unquotes become metavariables and the statement destructures
    // the value by matching its shape.
    insta::assert_snapshot!(expand_both("let ↖1 + ↙x↘↗ = rhs;")?);
    Ok(())
}

/// `let mut ↖p↗ = v;` is a pattern-let too.
///
/// The expander used to take the pattern from a fixed `terms[1]`, which is the
/// `mutable_specifier` here, not the pattern. The quote then expanded as an
/// ordinary term-builder *in binding position*, so `quilt expand` happily wrote
/// Rust that does not compile:
///
/// ```text
/// let mut tb("binary_expression").c(&a) … .b() = v;
/// ```
///
/// The meta-language now reports the pattern and value positions relative to its
/// own separator token, so an extra child cannot shift them (#174, finding E3).
#[test]
fn pattern_let_with_mut() -> Result<()> {
    let out = expand_both("let mut ↖1 + ↙x↘↗ = rhs;")?;
    assert!(
        out.contains("qmatch_n"),
        "`let mut` should still be a pattern-let: {out}"
    );
    assert!(
        !out.contains("tb(\"binary_expression\").c(&x)"),
        "the pattern must not expand as a term builder in binding position: {out}"
    );
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn pattern_let_value_quote_untouched() -> Result<()> {
    // Only the binding position triggers pattern matching: a value quote and
    // a type-position quote expand as before.
    let out = expand_both("let x = ↖1 + 2↗;")?;
    assert!(!out.contains("qmatch_n"), "{out}");
    let out = expand_both("let x: ↖T↗ = rhs;")?;
    assert!(!out.contains("qmatch_n"), "{out}");
    Ok(())
}

#[test]
fn pattern_let_duplicate_var_rejected() {
    let mut omni = Omni::default();
    let qterm = omni.parse("let ↖↙x↘ + ↙x↘↗ = rhs;").unwrap();
    let err = omni.expand(&qterm).unwrap_err();
    assert!(err.to_string().contains("more than once"), "{err}");
}

#[test]
fn pattern_let_non_ident_var_rejected() {
    let mut omni = Omni::default();
    let qterm = omni.parse("let ↖1 + ↙f(x)↘↗ = rhs;").unwrap();
    let err = omni.expand(&qterm).unwrap_err();
    assert!(err.to_string().contains("plain identifier"), "{err}");
}

#[test]
fn pattern_let_runtime() -> Result<()> {
    // End-to-end: expand and run the issue #18 example shape. The Rust
    // pattern destructures a Rust quote, the Python pattern a Python quote;
    // each metavariable binds the matched source text.
    let mut omni = Omni::default();
    let code = indoc! {r#"{
        let ↖1 + ↙x↘↗ = ↖1 + 2↗;
        let py↖def f(↙args↘): pass↗ = py↖def f(y: int): pass↗;
        format!("{} | {}", x.coparse(), args.coparse())
    }"#};
    let qterm = omni.parse(code)?;
    let out: String = omni.expand(&qterm)?.reduce()?;
    assert_eq!(out, "2 | y: int");
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
    insta::assert_snapshot!(out);
    Ok(())
}
