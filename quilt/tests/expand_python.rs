//! The Python `PythonMetaLanguage` (via `Omni`) expands `↖..↗`/`↙..↘` in a
//! `.py`-host file to Python builder source that reconstructs the quoted term.
//! There is no Python `quilt` runtime yet, so (like `expand_rust`'s structural
//! tests) these assert on the emitted source string rather than running it.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/// Parse + expand `code` as a Python host and return the emitted source.
fn expand_py(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_lang("py", code)?;
    Ok(omni.expand_lang("py", &q)?.coparse())
}

#[test]
fn simple() -> Result<()> {
    // Quote on the RHS of an assignment; emits Python builder source (no `&`).
    let out = expand_py("x = ↖1 + 2↗")?;
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn quote_expr() -> Result<()> {
    let out = expand_py("↖1 + 2↗")?;
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn variadic() -> Result<()> {
    // The `block` (suite) is variadic: when it holds a dynamic child its
    // statements go through the fluent `.e(..)` emit chain rather than Rust's
    // imperative `{ let mut b_ = ..; .. }` block. `.e` is what lets one child
    // contribute several statements.
    let out = expand_py(indoc! {r#"
        ↖def foo():
            ↙stmts↘
            print("World")
        ↗
    "#})?;
    assert!(
        out.contains(r#"tb("block").e("#),
        "a block with a dynamic child should use a fluent .e() emit chain; got:\n{out}"
    );

    // With no dynamic child there is nothing to append, so the same block is
    // built with plain `.c(..)` children — the same term, less generated code.
    let literal = expand_py(indoc! {r#"
        ↖def foo():
            print("Hello")
            print("World")
        ↗
    "#})?;
    assert!(
        literal.contains(r#"tb("block").c(tb("expression_statement")"#),
        "a hole-free block should use plain .c() children; got:\n{literal}"
    );
    assert!(
        literal.ends_with(".b()"),
        "expansion should end with a .b() build call; got:\n{literal}"
    );
    Ok(())
}

#[test]
fn unquote() -> Result<()> {
    // A nested quote/unquote: the inner `↙..↘` becomes an `unquote(..)` call.
    let out = expand_py("↖↖↙1↘↗↗")?;
    assert!(
        out.contains("unquote("),
        "nested unquote should emit an unquote(..) call; got:\n{out}"
    );
    Ok(())
}

#[test]
fn heterogeneous_lift_into_html() -> Result<()> {
    // A `↑` inside an unquote in an `html↖…↗` quote lifts *into HTML*: it
    // expands to the runtime's `qlift_html` (which entity-escapes strings),
    // not the homogeneous `.qlift()`.
    let out = expand_py("t = html↖<p>↙↑(title)↘</p>↗")?;
    assert!(
        out.contains("qlift_html(title)"),
        "lift into an html quote should spell qlift_html; got:\n{out}"
    );
    Ok(())
}

#[test]
fn homogeneous_lift_is_prefix() -> Result<()> {
    // The python→python `↑` spells the prefix `qlift` function (a method
    // can't hang off builtin ints), written `↑(value)`.
    let out = expand_py("t = ↖[↙↑(n)↘]↗")?;
    assert!(
        out.contains("qlift(n)"),
        "homogeneous lift should spell the prefix qlift function; got:\n{out}"
    );
    Ok(())
}

#[test]
fn bare_tuple_quote() -> Result<()> {
    // A bare tuple keeps its elements directly under the expression
    // statement; the quote must not try to squash past it. This is the
    // fold-through-a-quote join: `a, b` splices flat into expression
    // position, so folding it again stays a flat comma-separated list.
    //
    // The children are `.e`, not `.c`, because `expression_statement` is one of
    // the containers the derived arity table picks up (#202) — its rule is
    // `commaSep1(expression)`, which is exactly what a bare tuple exercises. For
    // a single child the two are the same call; what `.e` adds is that an emit
    // (`←`) can splice a whole sequence into the tuple.
    let out = expand_py("p = ↖↙a↘, ↙b↘↗")?;
    assert!(
        out.contains(r#"tb("expression_statement").e(a).e(sym(",")).w(" ").e(b)"#),
        "a bare tuple quote should keep the expression_statement whole; got:\n{out}"
    );
    Ok(())
}

#[test]
fn block_body_dedents_at_top_level() -> Result<()> {
    // A block-opened quote (`↖\n…↗`) whose body is indented for readability is
    // dedented even at the top level (no enclosing indent), so it expands the
    // same as the flush-left form. Regression test for the dedent only firing
    // when the quote sat in already-indented code.
    let indented = expand_py("x = ↖\n    a = 1\n    b = 2\n↗")?;
    let flush = expand_py("x = ↖\na = 1\nb = 2\n↗")?;
    assert_eq!(
        indented, flush,
        "an indented block body should dedent to the flush-left expansion"
    );
    Ok(())
}

#[test]
fn inline_body_is_not_dedented() -> Result<()> {
    // An inline-opened quote (`↖foo…`) keeps its body's indentation, which is
    // meaningful (e.g. a function body) rather than cosmetic: the opener line
    // sits at column 0 and bounds the common indent to nothing.
    let out = expand_py("x = ↖def f():\n    return 1\n↗")?;
    assert!(
        out.contains(r#".p("    ").n().c(tb("block").c(tb("return_statement")"#),
        "inline-opened body keeps its meaningful indentation (return nested in \
         the function block, not dedented to a sibling); got:\n{out}"
    );
    Ok(())
}
