use indoc::indoc;
use quilt::langs::bootstrap::Bootstrap;
use quilt::prelude::*;
use quilt::term::STerm;
use std::ops::Range;

/**************************************************************/

#[test]
fn bootstrap() -> Result<()> {
    let mut bootstrap = Bootstrap::default();
    let code_inner = "1 + 2";
    let code = format!("let expr = ↖{code_inner}↗;");
    let qterm = bootstrap.parse(&code)?;
    // dbg!(&qterm);
    let expanded = bootstrap.expand(&qterm);
    // dbg!(&expanded);
    let coparsed = expanded?.coparse();
    println!("{coparsed}");
    assert_eq!(
        coparsed,
        r#"let expr = tb("binary_expression").c(&leaf("integer_literal", "1")).w(" ").c(&sym("+")).w(" ").c(&leaf("integer_literal", "2")).b();"#
    );
    let expr = tuple(
        "binary_expression",
        &[
            leaf("integer_literal", "1"),
            sym("+"),
            leaf("integer_literal", "2"),
        ],
        &[HOLE, cmd(write(" ")), HOLE, cmd(write(" ")), HOLE],
    );
    // dbg!(&expr);
    let coparsed_inner = expr.coparse();
    // println!("{expr_coparsed}");
    assert_eq!(coparsed_inner, code_inner);
    Ok(())
}

#[test]
fn reduce() -> Result<()> {
    let mut bootstrap = Bootstrap::default();
    let code = "3..5";
    let qterm = bootstrap.parse(code)?;
    let reduced: Range<i32> = bs_reduce(&qterm)?;
    dbg!(&reduced);
    Ok(())
}

#[test]
fn reduce_repeated() -> Result<()> {
    let mut bootstrap = Bootstrap::default();
    let code = "{
        let mut i: i32 = 123; 
        i = i.↑.↓?; // can repeat this
        i
    }";
    let qterm = bootstrap.parse(code)?;
    let reduced: i32 = bs_reduce(&qterm)?;
    dbg!(&reduced);
    Ok(())
}

#[test]
fn variadic() -> Result<()> {
    let mut bootstrap = Bootstrap::default();
    let code = indoc! {r#"
        ↖fn foo() {
            println!("Hello");
            println!("World"); 
        }↗
    "#};
    let qterm = bootstrap.parse(code)?;
    let expanded = bootstrap.expand(&qterm)?;
    let coparsed = expanded.coparse();
    println!("{coparsed}");
    let reduced: QTerm = bs_reduce(&expanded)?;
    // dbg!(&reduced);
    let reduced_coparsed = reduced.coparse();
    sep();
    println!("{reduced_coparsed}");
    Ok(())
}

#[test]
fn reduce_qterm() -> Result<()> {
    let mut bootstrap = Bootstrap::default();
    let code = "↖1 + 2↗";
    let qterm = bootstrap.parse(code)?;
    let expanded = bootstrap.expand(&qterm)?;
    // dbg!(&expanded);
    let coparsed = expanded.coparse();
    println!("{coparsed}");
    let reduced: QTerm = bs_reduce(&expanded)?;
    // dbg!(&reduced);
    let reduced_coparsed = reduced.coparse();
    sep();
    println!("{reduced_coparsed}");
    Ok(())
}

#[test]
fn splicing() -> Result<()> {
    let mut bootstrap = Bootstrap::default();
    let code = indoc! {r#"{
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
    }"#};
    let qterm = bootstrap.parse(code)?;
    let expanded = bootstrap.expand(&qterm)?;
    // dbg!(&expanded);
    let coparsed = expanded.coparse();
    println!("{coparsed}");
    let reduced: QTerm = bs_reduce(&expanded)?;
    // dbg!(&reduced);
    let reduced_coparsed = reduced.coparse();
    sep();
    println!("{reduced_coparsed}");
    sep();
    let () = bs_reduce(&arc(reduced))?;

    Ok(())
}

#[test]
fn splicing_nested() -> Result<()> {
    let mut bootstrap = Bootstrap::default();
    let code = indoc! {r#"{
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
    }"#};
    let qterm = bootstrap.parse(code)?;
    let expanded = bootstrap.expand(&qterm)?;
    // dbg!(&expanded);
    let coparsed = expanded.coparse();
    println!("{coparsed}");
    let reduced: QTerm = bs_reduce(&expanded)?;
    // dbg!(&reduced);
    let reduced_coparsed = reduced.coparse();
    sep();
    println!("{reduced_coparsed}");
    sep();
    let () = bs_reduce(&arc(reduced))?;

    Ok(())
}

/// The `Lift` impls the bootstrap generates must tag each literal with the kind
/// the Rust parser gives it.
///
/// Floats shared the integers' template until #174, so every float impl emitted
/// `leaf("integer_literal", …)` — a float tagged as an integer. Nothing called
/// `Lift`, so nothing noticed; this pins the tags now that `mk_meta.rs.quilt`
/// derives the float template from a float literal.
#[test]
fn generated_lift_tags_match_the_literal_kind() {
    use quilt::langs::rust::meta::Lift;

    assert_eq!(1.5f32.lift(), leaf("float_literal", "1.5f32"));
    assert_eq!(0.5f64.lift(), leaf("float_literal", "0.5f64"));
    // Integers are unchanged.
    assert_eq!(7i32.lift(), leaf("integer_literal", "7i32"));
    assert_eq!(7u8.lift(), leaf("integer_literal", "7u8"));
}
