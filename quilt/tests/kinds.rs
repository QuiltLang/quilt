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

/// Holes record the `InnerKind` their syntactic position demands.
#[test]
fn rs_hole_ikinds() -> Result<()> {
    use quilt::lang::{FlatNode, LanguagePost as _};
    let mut lang = RustLanguage::default();
    let code = [
        FlatNode::Str("fn main() { "),
        FlatNode::Hole, // statement position
        FlatNode::Str(" let x = "),
        FlatNode::Hole, // expression position
        FlatNode::Str("; }"),
    ];
    let post = lang.parse_pre(None, &code)?;
    let kinds = post.holes().iter().map(|h| h.ikind).collect::<Vec<_>>();
    assert_eq!(kinds, [Some(InnerKind::Stmt), Some(InnerKind::Expr)]);
    Ok(())
}

/// `typ` distinguishes items (definitions) from statements and expressions.
#[test]
fn rs_typ_kinds() {
    use quilt::lang::Language as _;
    let lang = RustLanguage::default();
    assert_eq!(lang.typ("source_file"), InnerKind::File);
    assert_eq!(lang.typ("function_item"), InnerKind::Item);
    assert_eq!(lang.typ("struct_item"), InnerKind::Item);
    assert_eq!(lang.typ("use_declaration"), InnerKind::Item);
    // `let` is a statement, not an item, despite the "declaration" suffix.
    assert_eq!(lang.typ("let_declaration"), InnerKind::Stmt);
    assert_eq!(lang.typ("expression_statement"), InnerKind::Stmt);
    assert_eq!(lang.typ("binary_expression"), InnerKind::Expr);
    // A `block`'s tag alone is an expression (a block expression).
    assert_eq!(lang.typ("block"), InnerKind::Expr);
}

/// A hole in block-body position is typed `Block`, while a hole in value
/// position stays `Expr` — even though both are spelled with the same `{}`
/// placeholder. The distinction comes from `hole_kind` reading the parent.
#[test]
fn rs_block_hole_ikind() -> Result<()> {
    use quilt::lang::{FlatNode, LanguagePost as _};
    let mut lang = RustLanguage::default();

    // `fn f() <hole>` — the hole is the function body block.
    let body = [FlatNode::Str("fn f() "), FlatNode::Hole];
    let kinds = lang
        .parse_pre(None, &body)?
        .holes()
        .iter()
        .map(|h| h.ikind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, [Some(InnerKind::Block)]);

    // `let x = <hole>;` — the hole is the let value, an expression.
    let value = [
        FlatNode::Str("let x = "),
        FlatNode::Hole,
        FlatNode::Str(";"),
    ];
    let kinds = lang
        .parse_pre(None, &value)?
        .holes()
        .iter()
        .map(|h| h.ikind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, [Some(InnerKind::Expr)]);
    Ok(())
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
