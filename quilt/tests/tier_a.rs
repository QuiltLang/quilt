//! Tier A instantiation (issue #87): fill a sky-first template's holes with
//! lifted parameter values, no host run.

use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::template::{instantiate, ParamEnv, ParamValue};
use quilt::term::STerm;

/// Parse `src` sky-first in `target` and instantiate it against `env`, returning
/// the finished output source (trimmed of the file's trailing newline).
fn render(target: &str, src: &str, env: &ParamEnv) -> Result<String> {
    let mut omni = Omni::default();
    let t = omni.parse_sky(target, src)?;
    Ok(instantiate(&t, env)?.coparse().trim_end().to_owned())
}

fn env(pairs: &[(&str, ParamValue)]) -> ParamEnv {
    pairs
        .iter()
        .map(|(k, v)| ((*k).into(), v.clone()))
        .collect()
}

/**************************************************************/
// Python target: string, int, list.

#[test]
fn python_string() -> Result<()> {
    let out = render(
        "py",
        "print(↙greeting↘)\n",
        &env(&[("greeting", "hello".into())]),
    )?;
    assert_eq!(out, r#"print("hello")"#);
    Ok(())
}

#[test]
fn python_int() -> Result<()> {
    let out = render("py", "x = ↙n↘\n", &env(&[("n", ParamValue::Int(42))]))?;
    assert_eq!(out, "x = 42");
    Ok(())
}

#[test]
fn python_list() -> Result<()> {
    let out = render(
        "py",
        "xs = ↙nums↘\n",
        &env(&[("nums", vec![1_i64, 2, 3].into())]),
    )?;
    assert_eq!(out, "xs = [1, 2, 3]");
    Ok(())
}

/**************************************************************/
// Rust target: string, int, list.

#[test]
fn rust_string() -> Result<()> {
    let out = render("rs", "let s = ↙name↘;\n", &env(&[("name", "bob".into())]))?;
    assert_eq!(out, r#"let s = "bob";"#);
    Ok(())
}

#[test]
fn rust_int_and_list() -> Result<()> {
    let out = render(
        "rs",
        "let a = ↙xs↘;\n",
        &env(&[("xs", vec![1_i64, 2, 3].into())]),
    )?;
    assert_eq!(out, "let a = [1, 2, 3];");
    Ok(())
}

/**************************************************************/
// HTML target: text content, entity-escaped so the value stays inert data.

#[test]
fn html_text_is_escaped() -> Result<()> {
    let out = render(
        "html",
        "<p>↙msg↘</p>\n",
        &env(&[("msg", "<b>&you</b>".into())]),
    )?;
    assert_eq!(out, "<p>&lt;b&gt;&amp;you&lt;/b&gt;</p>");
    Ok(())
}

/**************************************************************/
// Errors.

#[test]
fn missing_param_errors_with_span() {
    let mut omni = Omni::default();
    let src = "x = ↙value↘\n";
    let t = omni.parse_sky("py", src).unwrap();
    let err = instantiate(&t, &ParamEnv::new()).unwrap_err();
    assert!(
        err.to_string()
            .contains("missing template parameter `value`"),
        "got: {err}"
    );
    // points at the hole
    let start = src.find('↙').unwrap();
    let labels: Vec<_> = err.labels().expect("should carry a label").collect();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].offset(), start);
}

#[test]
fn host_expression_hole_is_tier_b() {
    let mut omni = Omni::default();
    // `↙a + b↘` is a host expression, not a bare parameter name.
    let t = omni.parse_sky("py", "x = ↙a + b↘\n").unwrap();
    let err = instantiate(&t, &env(&[("a", ParamValue::Int(1))])).unwrap_err();
    assert!(err.to_string().contains("Tier B"), "got: {err}");
}
