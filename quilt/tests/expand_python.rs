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
    assert_eq!(
        out,
        r#"x = tb("binary_operator").c(leaf("integer", "1")).w(" ").c(sym("+")).w(" ").c(leaf("integer", "2")).b()"#
    );
    Ok(())
}

#[test]
fn quote_expr() -> Result<()> {
    let out = expand_py("↖1 + 2↗")?;
    assert_eq!(
        out,
        r#"tb("binary_operator").c(leaf("integer", "1")).w(" ").c(sym("+")).w(" ").c(leaf("integer", "2")).b()"#
    );
    Ok(())
}

#[test]
fn variadic() -> Result<()> {
    // The `block` (suite) is variadic: its statements are emitted via the fluent
    // `.e(..)` chain rather than Rust's imperative `{ let mut b_ = ..; .. }` block.
    let out = expand_py(indoc! {r#"
        ↖def foo():
            print("Hello")
            print("World")
        ↗
    "#})?;
    assert!(
        out.contains(r#"tb("block").e(tb("expression_statement")"#),
        "variadic block should use a fluent .e() emit chain; got:\n{out}"
    );
    assert!(
        out.ends_with(".b()"),
        "expansion should end with a .b() build call; got:\n{out}"
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
    let out = expand_py("p = ↖↙a↘, ↙b↘↗")?;
    assert!(
        out.contains(r#"tb("expression_statement").c(a).c(sym(",")).w(" ").c(b)"#),
        "a bare tuple quote should keep the expression_statement whole; got:\n{out}"
    );
    Ok(())
}
