use indoc::indoc;
use quilt::{
    lang::{flat_nodes, InnerKind, Language as _},
    langs::{python::lang::PythonLanguage, rust::lang::RustLanguage},
    prelude::*,
    qterm::QTermTag,
    term::Term as _,
};

/**************************************************************/

fn rs_helper(ikind: Option<InnerKind>, code: &str, expected_tag: &QTermTag) -> Result<()> {
    let mut lang = RustLanguage::default();
    let qterm = lang.parse_as(ikind, &flat_nodes(code))?;
    // dbg!(&qterm);
    assert_eq!(qterm.coparse(), code);
    assert_eq!(&qterm.tag(), expected_tag);
    Ok(())
}

#[test]
fn rs_expr() -> Result<()> {
    rs_helper(
        Some(InnerKind::Expr),
        "1 + 2",
        &QTermTag::tuple("binary_expression"),
    )
}

#[test]
fn rs_stmt() -> Result<()> {
    rs_helper(
        Some(InnerKind::Stmt),
        "let x = 1 + 2;",
        &QTermTag::tuple("let_declaration"),
    )
}

#[test]
fn rs_file() -> Result<()> {
    rs_helper(
        Some(InnerKind::File),
        indoc! {r#"
            fn f() { println!("Hello, world!"); }
            fn main() { f(); }
        "#},
        &QTermTag::tuple("source_file"),
    )
}

#[test]
fn rs_auto_expr() -> Result<()> {
    rs_helper(None, "1 + 2", &QTermTag::tuple("binary_expression"))
}

#[test]
fn rs_auto_stmt() -> Result<()> {
    rs_helper(None, "let x = 1 + 2;", &QTermTag::tuple("let_declaration"))
}

#[test]
fn rs_auto_file() -> Result<()> {
    rs_helper(
        None,
        indoc! {r#"
            fn f() { println!("Hello, world!"); }
            fn main() { f(); }
        "#},
        &QTermTag::tuple("source_file"),
    )
}

/**************************************************************/

fn py_helper(ikind: Option<InnerKind>, code: &str, expected_tag: &QTermTag) -> Result<()> {
    let mut lang = PythonLanguage::default();
    let qterm = lang.parse_as(ikind, &flat_nodes(code))?;
    // dbg!(&qterm);
    assert_eq!(qterm.coparse(), code);
    assert_eq!(&qterm.tag(), expected_tag);
    Ok(())
}

#[test]
fn py_expr() -> Result<()> {
    py_helper(
        Some(InnerKind::Expr),
        "1 + 2",
        &QTermTag::tuple("binary_operator"),
    )
}

#[test]
fn py_stmt() -> Result<()> {
    py_helper(
        Some(InnerKind::Stmt),
        "x = 1 + 2",
        &QTermTag::tuple("assignment"),
    )
}

#[test]
fn py_file() -> Result<()> {
    py_helper(
        Some(InnerKind::File),
        indoc! {r#"
            def f(): print("Hello, world!")
            def main(): f()
        "#},
        &QTermTag::tuple("module"),
    )
}

#[test]
fn py_auto_expr() -> Result<()> {
    py_helper(None, "1 + 2", &QTermTag::tuple("binary_operator"))
}

#[test]
fn py_auto_stmt() -> Result<()> {
    py_helper(None, "x = 1 + 2", &QTermTag::tuple("assignment"))
}

#[test]
fn py_auto_stmt_def() -> Result<()> {
    py_helper(
        None,
        "def f(): return",
        &QTermTag::tuple("function_definition"),
    )
}

#[test]
fn py_auto_file() -> Result<()> {
    py_helper(
        None,
        indoc! {r#"
            def f(): print("Hello, world!")
            def main(): f()
        "#},
        &QTermTag::tuple("module"),
    )
}
