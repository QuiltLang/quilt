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
    // `block` is variadic, but this one holds no unquote, so there is nothing
    // that could contribute more than one child: it builds with the fluent
    // chain rather than the `{ let mut b_ = ..; .. }` accumulator. `splicing`
    // and `splicing_nested` below cover the accumulator form, which is what a
    // block with a dynamic child still gets.
    let out = expand_both(indoc! {r#"
        ↖fn foo() {
            println!("Hello");
            println!("World");
        }↗
    "#})?;
    assert!(!out.contains("let mut b_"), "{out}");
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
fn ground_tail_stmt_quote_stays_value() -> Result<()> {
    // The same tail position, but the quoted body is an item or a block —
    // statement-shaped, so the emit inference used to fire and write
    // `….emit(&mut b_);` into a function that has no `b_`. Outside every
    // unquote there is no enclosing builder to emit into, so the quote is the
    // function's value whatever its body.
    for code in [
        "fn f() -> Arc<QTerm> {\n    ↖fn g() {}↗\n}",
        "fn f() -> Arc<QTerm> {\n    ↖{ 1 }↗\n}",
        "fn f(x: Arc<QTerm>) -> Arc<QTerm> {\n    ↖{ let y = ↙x↘; }↗\n}",
    ] {
        let out = expand_both(code)?;
        assert!(!out.contains("emit(&mut b_)"), "{code}\n=>\n{out}");
    }
    insta::assert_snapshot!(expand_both("fn f() -> Arc<QTerm> {\n    ↖fn g() {}↗\n}")?);
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
fn ground_stmt_sequence_unquote_spliced() -> Result<()> {
    // The same rule for a body holding *several* statements, which parses with
    // the file root rather than a statement one.
    //
    // This used to be emitted, appending `.emit(&mut b_);` after the body's
    // last statement — `…emit(&mut b_);.emit(&mut b_);` here, which does not
    // parse. It went unnoticed because the failure is invisible whenever the
    // last statement is block-shaped (`for`, `if`, `{…}`): those are valid
    // method receivers evaluating to `()`, so the stray call landed on the
    // no-op `impl Emit for ()`. `mk_meta.rs.quilt` ends its unquote with a
    // `for`, so the bootstrap bootstrapped right past it.
    let out = expand_both(indoc! {r#"
        let p = ↖
            fn keep() {}

            ↙
                let n = 3;
                ↖fn made() {}↗.←;
            ↘
        ↗;
    "#})?;
    assert!(!out.contains(";.emit(&mut b_)"), "{out}");
    // The body is spliced verbatim, statements and all.
    assert!(out.contains("let n = 3;"), "{out}");
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

/// A pattern whose root is a variadic node — a list, a block, a Lean
/// `inductive` — must compile too.
///
/// A variadic node with an unquote among its children expands to an
/// accumulator block, `{ let mut b_ = tb(..); ..; b_.b() }`, and the pattern
/// was passed as `qmatch_n(&{ .. }, ..)`. The `&QTerm` that `qmatch_n` expects
/// flows *into* the block, so its tail `b_.b()`, an `Arc<QTerm>`, is a type
/// error there, and no deref coercion applies. Expansion succeeded and the
/// generated program did not compile.
#[test]
fn pattern_let_variadic_pattern_runtime() -> Result<()> {
    let mut omni = Omni::default();
    let code = indoc! {r#"{
        let ↖[↙a↘, ↙b↘]↗ = ↖[1, 2]↗;
        format!("{} | {}", a.coparse(), b.coparse())
    }"#};
    let qterm = omni.parse(code)?;
    let out: String = omni.expand(&qterm)?.reduce()?;
    assert_eq!(out, "1 | 2");
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

/* ─────────────── a quote as a whole closure body (#241) ─────────────── */

/// A quote as the **entire body of a closure** must parse.
///
/// It did not until QuiltLang/tree-sitter-rust#1: substituting the hole turns
/// `|x| ↖…↗` into `|x| {}`, and the fork's `quilt_hole` was a plain `"{}"`
/// token. Tree-sitter's lexer prefers the longest match among the tokens valid
/// in a state, so wherever a pattern could still continue — exactly where a
/// closure's parameter list has just closed — `{}` outbid the `{` that starts a
/// `block` and the whole file parsed as an or-pattern. The error named the file,
/// not the closure, which is what made it hard to place.
///
/// The asymmetry is the tell, so all four shapes are pinned: the two that
/// *failed* (one bare parameter, and none) and the two that always worked
/// (several parameters, and a type annotation), which is what an ambiguity fix
/// could plausibly break in the other direction.
#[test]
fn quote_as_a_whole_closure_body() -> Result<()> {
    let out = expand_both("let f = xs.iter().map(|x| ↖1 + ↙x↘↗);")?;
    assert!(
        out.contains("binary_expression"),
        "the closure body should expand to builder calls, got:\n{out}"
    );
    Ok(())
}

#[test]
fn quote_as_a_closure_body_without_parameters() -> Result<()> {
    expand_both("let f = || ↖1↗;")?;
    Ok(())
}

#[test]
fn quote_as_a_closure_body_with_a_reference_pattern() -> Result<()> {
    expand_both("let f = xs.iter().map(|&(a, b)| ↖↙a↘ + ↙b↘↗);")?;
    Ok(())
}

/// The shapes that always parsed, because an or-pattern was already impossible
/// by the time the body was reached. Pinned so a future change to the hole
/// token cannot fix one direction by breaking the other.
#[test]
fn closure_bodies_that_always_worked_still_do() -> Result<()> {
    expand_both("let f = xs.iter().map(|a, b| ↖↙a↘ + ↙b↘↗);")?;
    expand_both("let f = xs.iter().map(|x: u8| ↖↙x↘ + 1↗);")?;
    Ok(())
}

/// A hole in *pattern* and *type* position is what `quilt_hole` exists for, and
/// the token-precedence fix must not cost either.
#[test]
fn holes_in_pattern_and_type_position_still_parse() -> Result<()> {
    expand_both("let ↖x↗ = 1;")?;
    expand_both("let v: Vec<⟨T⟩> = Vec::new();")?;
    Ok(())
}

/* ------------------ a sequence spliced into a list (#293) ----------------- */

/// Several terms spliced into a comma-separated container — an array, an
/// argument list — are separated with `, `. They used to be written back to
/// back: `[12]`, `f(12)`.
#[test]
fn a_vec_spliced_into_a_list_is_comma_separated() -> Result<()> {
    let mut omni = Omni::default();
    let code = indoc! {r#"{
        let xs: Vec<Arc<QTerm>> = vec![1.qlift(), 2.qlift()];
        let ys: Vec<i64> = vec![1, 2, 3];
        let a = ↖[↙xs↘]↗;
        let f = ↖f(↙xs↘)↗;
        let c = ↖const A: [i64; 3] = [↙{ for y in &ys { y.↑.←; } }↘];↗;
        format!("{} | {} | {}", a.coparse(), f.coparse(), c.coparse())
    }"#};
    let qterm = omni.parse(code)?;
    let out: String = omni.expand(&qterm)?.reduce()?;
    assert_eq!(out, "[1, 2] | f(1, 2) | const A: [i64; 3] = [1, 2, 3];");
    Ok(())
}

/// Separators the author already placed are kept, not doubled: a literal
/// element before the splice, and the workaround of emitting `,` by hand.
#[test]
fn hand_placed_list_separators_are_kept() -> Result<()> {
    let mut omni = Omni::default();
    let code = indoc! {r#"{
        let xs: Vec<Arc<QTerm>> = vec![1.qlift(), 2.qlift()];
        let a = ↖[0, ↙xs↘, 3]↗;
        let b = ↖[↙{
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    sym(",").←;
                    " ".←;
                }
                x.←;
            }
        }↘]↗;
        format!("{} | {}", a.coparse(), b.coparse())
    }"#};
    let qterm = omni.parse(code)?;
    let out: String = omni.expand(&qterm)?.reduce()?;
    assert_eq!(out, "[0, 1, 2, 3] | [1, 2]");
    Ok(())
}

/* ──────── holes in item, variant, field and match-arm position (#302) ──────── */

// A hole could stand only where the Rust grammar fork had a `quilt_hole`: type
// and pattern position, plus wherever `{}` happens to parse as an empty block
// (expression and statement position, and the top level of a file). These are
// the positions a code generator reaches for next, each of which was a parse
// error.
//
// The snapshots pin the builder calls. The `*_runs` tests go one step further:
// they run the generator, then compile and run the Rust it generated.

/// Run the generator `code` (a Rust expression returning the `coparse` of a
/// quoted block), then compile and run the block it generated. Returns the
/// generated source and its value.
fn generate_and_run<T: serde::Serialize + for<'de> serde::Deserialize<'de>>(
    code: &str,
) -> Result<(String, T)> {
    let mut omni = Omni::default();
    let generator = omni.parse(code)?;
    let src: String = omni.expand(&generator)?.reduce()?;
    let mut omni = Omni::default();
    let generated = omni.parse(&src)?;
    Ok((src.clone(), generated.reduce()?))
}

const GEN: &str = "fn gen(a: ⟨T⟩, xs: Vec<⟨T⟩>) -> ⟨T⟩ {\n    BODY\n}";

fn expand_gen(body: &str) -> Result<String> {
    expand_both(&GEN.replace("BODY", body))
}

#[test]
fn hole_in_enum_variant_position() -> Result<()> {
    insta::assert_snapshot!(
        "hole_as_the_only_enum_variant",
        expand_gen("↖enum Color { ↙a↘ }↗")?
    );
    insta::assert_snapshot!(
        "hole_after_an_enum_variant",
        expand_gen("↖enum Color { Red, ↙a↘ }↗")?
    );
    insta::assert_snapshot!(
        "hole_as_emitted_enum_variants",
        expand_gen("↖enum Color { ↙{ for x in xs { x.← } }↘ }↗")?
    );
    Ok(())
}

#[test]
fn hole_in_enum_variant_position_runs() -> Result<()> {
    let (src, value): (_, u32) = generate_and_run(indoc! {r#"{
        let green = ↖Green↗;
        ↖{
            enum Color { Red, ↙green↘, Blue }
            let _ = (Color::Red, Color::Blue);
            Color::Green as u32
        }↗.coparse()
    }"#})?;
    assert!(src.contains("enum Color { Red, Green, Blue }"), "{src}");
    assert_eq!(value, 1);
    Ok(())
}

#[test]
fn hole_in_struct_field_position() -> Result<()> {
    insta::assert_snapshot!(
        "hole_as_the_only_struct_field",
        expand_gen("↖struct S { ↙a↘ }↗")?
    );
    insta::assert_snapshot!(
        "hole_after_a_struct_field",
        expand_gen("↖struct S { x: u32, ↙a↘ }↗")?
    );
    Ok(())
}

/// The field comes out of another struct by a pattern-let, which puts the same
/// hole position in a pattern quote.
#[test]
fn hole_in_struct_field_position_runs() -> Result<()> {
    let (src, value): (_, u32) = generate_and_run(indoc! {r#"{
        let ↖struct Q { ↙field↘ }↗ = ↖struct Q { y: u32 }↗;
        ↖{
            struct P { x: u32, ↙field↘ }
            let p = P { x: 1, y: 2 };
            p.x + p.y
        }↗.coparse()
    }"#})?;
    assert!(src.contains("struct P { x: u32, y: u32 }"), "{src}");
    assert_eq!(value, 3);
    Ok(())
}

/// A struct pattern's fields and its path were parse errors (`let {} { {} } =
/// v;` has no `struct_pattern` reading in the upstream grammar). The fork
/// takes a hole in both places.
#[test]
fn hole_in_struct_pattern_position() -> Result<()> {
    insta::assert_snapshot!(
        "hole_as_a_let_struct_pattern_type_and_fields",
        expand_gen("↖let ↙a↘ { ↙xs↘ } = v;↗")?
    );
    insta::assert_snapshot!(
        "hole_as_a_struct_pattern_field",
        expand_gen("↖let S { ↙a↘ } = v;↗")?
    );
    insta::assert_snapshot!(
        "hole_among_struct_pattern_fields_in_a_match_arm",
        expand_gen("↖match v { S { x, ↙xs↘, .. } => x }↗")?
    );
    Ok(())
}

/// The struct type and a sequence of fields, one field alone, and a field
/// beside literal ones in a match arm: each destructures a real struct. The
/// arm's `w: ww` comes out of another arm by a pattern-let, which puts the
/// same hole in a pattern quote.
#[test]
fn hole_in_struct_pattern_position_runs() -> Result<()> {
    let (src, value): (_, u32) = generate_and_run(indoc! {r#"{
        let ty = ↖P↗;
        let fs: Vec<Arc<QTerm>> = vec![↖x↗, ↖y↗];
        let f = ↖z↗;
        let ↖match q { Q { ↙rest↘ } => 0 }↗ = ↖match q { Q { w: ww } => 0 }↗;
        ↖{
            struct P { w: u32, x: u32, y: u32, z: u32 }
            let p = P { w: 8, x: 1, y: 2, z: 4 };
            let ↙ty↘ { ↙fs↘, .. } = p;
            let P { ↙f↘, .. } = p;
            let s = match p { P { w: 0, .. } => 0, P { ↙rest↘, .. } => ww };
            x + y + z + s
        }↗.coparse()
    }"#})?;
    assert!(src.contains("let P { x, y, .. } = p;"), "{src}");
    assert!(src.contains("let P { z, .. } = p;"), "{src}");
    assert_eq!(value, 15);
    Ok(())
}

/// A hole as the path of a struct or tuple struct pattern at the start of a
/// match arm, and a path that begins with holes. At an arm's start the fork
/// lexes the hole as `{` `}` (a hole may also stand for whole arms there), so
/// these pin that the term is the same `quilt_hole` either way.
#[test]
fn hole_as_a_pattern_path_in_a_match_arm() -> Result<()> {
    insta::assert_snapshot!(
        "hole_as_a_tuple_struct_pattern_path_in_a_match_arm",
        expand_gen("↖match v { ↙a↘(↙xs↘) => 0 }↗")?
    );
    insta::assert_snapshot!(
        "hole_as_a_struct_pattern_path_in_a_match_arm",
        expand_gen("↖match v { ↙a↘ { ↙xs↘ } => 0 }↗")?
    );
    insta::assert_snapshot!(
        "holes_as_a_path_in_a_match_arm",
        expand_gen("↖match v { ↙a↘::↙a↘(↙xs↘) => 0 }↗")?
    );
    insta::assert_snapshot!("holes_as_a_path_expression", expand_gen("↖↙a↘::↙a↘↗")?);
    Ok(())
}

/// Every pattern-path hole, in a `let` and in match arms, with sequences
/// spliced into the field lists, plus `↙ty↘::↙v↘` as an expression: the
/// generated program destructures a real enum.
#[test]
fn hole_as_a_pattern_path_runs() -> Result<()> {
    let (src, value): (_, Vec<u32>) = generate_and_run(indoc! {r#"{
        let (ty, unit) = (↖E↗, ↖Unit↗);
        let (pair, rec) = (↖E::Pair↗, ↖E::Rec↗);
        let ab: Vec<Arc<QTerm>> = vec![↖a↗, ↖b↗];
        let xy: Vec<Arc<QTerm>> = vec![↖x↗, ↖y↗];
        ↖{
            enum E { Pair(u32, u32), Rec { x: u32, y: u32 }, Unit }
            let f = |e: E| match e {
                ↙pair↘(↙ab↘) => a + b,
                ↙rec↘ { ↙xy↘ } => x * y,
                ↙ty↘::↙unit↘ => 0,
            };
            let ↙pair↘(↙ab↘) = E::Pair(1, 2) else { unreachable!() };
            let ↙rec↘ { ↙xy↘ } = (E::Rec { x: 3, y: 4 }) else { unreachable!() };
            let k = ↙ty↘::↙unit↘;
            vec![f(E::Pair(1, 2)), f(E::Rec { x: 3, y: 4 }), f(k), a + b + x + y]
        }↗.coparse()
    }"#})?;
    assert!(src.contains("E::Pair(a, b) => a + b,"), "{src}");
    assert!(src.contains("E::Rec { x, y } => x * y,"), "{src}");
    assert!(src.contains("E::Unit => 0,"), "{src}");
    assert!(src.contains("let k = E::Unit;"), "{src}");
    assert_eq!(value, vec![3, 12, 0, 10]);
    Ok(())
}

#[test]
fn hole_in_item_name_position() -> Result<()> {
    insta::assert_snapshot!("hole_as_a_fn_name", expand_gen("↖fn ↙a↘() -> u32 { 1 }↗")?);
    insta::assert_snapshot!(
        "hole_as_a_struct_name",
        expand_gen("↖struct ↙a↘ { x: u32 }↗")?
    );
    insta::assert_snapshot!("hole_as_a_const_name", expand_gen("↖const ↙a↘: u32 = 1;↗")?);
    Ok(())
}

/// `const` is the awkward one: in statement position `const {` also starts a
/// `const_block`, so the grammar takes the hole there as two tokens and tells
/// the readings apart by the `:` after it. A `const` block must still parse.
#[test]
fn hole_in_item_name_position_runs() -> Result<()> {
    let (src, value): (_, u32) = generate_and_run(indoc! {r#"{
        let (f, s, c) = (↖three↗, ↖Pair↗, ↖FOUR↗);
        ↖{
            fn ↙f↘() -> u32 { 3 }
            struct ↙s↘ { x: u32 }
            const ↙c↘: u32 = 4;
            let _ = const { 0 };
            three() + Pair { x: FOUR }.x
        }↗.coparse()
    }"#})?;
    assert!(src.contains("fn three() -> u32 { 3 }"), "{src}");
    assert!(src.contains("const FOUR: u32 = 4;"), "{src}");
    assert_eq!(value, 7);
    Ok(())
}

#[test]
fn hole_in_declaration_list_position() -> Result<()> {
    insta::assert_snapshot!(
        "hole_as_the_only_impl_item",
        expand_gen("↖impl X { ↙a↘ }↗")?
    );
    insta::assert_snapshot!(
        "hole_after_an_impl_item",
        expand_gen("↖impl X { fn g() {} ↙a↘ }↗")?
    );
    insta::assert_snapshot!(
        "hole_as_emitted_mod_items",
        expand_gen("↖mod m { fn g() {} ↙{ for x in xs { x.← } }↘ }↗")?
    );
    Ok(())
}

#[test]
fn hole_in_declaration_list_position_runs() -> Result<()> {
    let (src, value): (_, u32) = generate_and_run(indoc! {r#"{
        let method = ↖fn two(&self) -> u32 { 2 }↗;
        let fns = vec![↖pub fn one() -> u32 { 1 }↗, ↖pub fn ten() -> u32 { 10 }↗];
        ↖{
            struct S;
            impl S { fn zero(&self) -> u32 { 0 } ↙method↘ }
            mod m { ↙{ for f in fns { f.← } }↘ }
            S.zero() + S.two() + m::one() + m::ten()
        }↗.coparse()
    }"#})?;
    assert!(src.contains("fn two(&self)"), "{src}");
    assert_eq!(value, 13);
    Ok(())
}

#[test]
fn hole_in_match_arm_position() -> Result<()> {
    insta::assert_snapshot!(
        "hole_as_the_only_match_arm",
        expand_gen("↖match x { ↙a↘ }↗")?
    );
    insta::assert_snapshot!(
        "hole_as_arms_before_a_fallback",
        expand_gen("↖match x { ↙xs↘ _ => 0 }↗")?
    );
    insta::assert_snapshot!(
        "hole_after_a_match_arm",
        expand_gen("↖match x { 0 => 1, ↙a↘ }↗")?
    );
    insta::assert_snapshot!(
        "hole_as_emitted_arms_before_a_fallback",
        expand_gen("↖match x { ↙{ for x in xs { x.← } }↘ _ => 0 }↗")?
    );
    Ok(())
}

/// A hole that *begins* an arm's pattern is still a pattern: the token after
/// it (`=>`, `|`, `if`) decides, and `|` goes to the pattern.
#[test]
fn hole_beginning_a_match_pattern_is_still_a_pattern() -> Result<()> {
    for code in [
        "↖match x { ↙a↘ => 0 }↗",
        "↖match x { ↙a↘ | 1 => 0 }↗",
        "↖match x { ↙a↘ if c => 0 }↗",
    ] {
        let out = expand_gen(code)?;
        assert!(out.contains(r#"tb("match_arm")"#), "{code}:\n{out}");
    }
    Ok(())
}

/// Arms are taken out of other `match`es by pattern-lets (the same hole
/// position, in a pattern quote) and spliced between literal arms: one alone,
/// then a sequence. Each arm carries its own `,`, so no separator is needed.
#[test]
fn hole_in_match_arm_position_runs() -> Result<()> {
    let (src, value): (_, Vec<u32>) = generate_and_run(indoc! {r#"{
        let ↖match x { ↙one↘ }↗ = ↖match x { 1 => 10, }↗;
        let ↖match x { ↙two↘ }↗ = ↖match x { 2 => 20, }↗;
        let ↖match x { ↙three↘ }↗ = ↖match x { 3 => 30, }↗;
        let rest = vec![two, three];
        ↖{
            let f = |x: u32| match x { 0 => 0, ↙one↘ ↙{ for arm in rest { arm.← } }↘ _ => 99 };
            (0..5).map(f).collect::<Vec<u32>>()
        }↗.coparse()
    }"#})?;
    assert!(src.contains("1 => 10,"), "{src}");
    assert_eq!(value, vec![0, 10, 20, 30, 99]);
    Ok(())
}
