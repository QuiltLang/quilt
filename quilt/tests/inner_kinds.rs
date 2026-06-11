//! Tests for the inner/outer kind machinery described in issue #25.
//!
//! These tests document the *untapped potential* from the issue:
//!
//! 1. `classify_term` — languages can inspect the full parsed term to determine
//!    its kind, not just the root tag name.  This closes the "feedback loop"
//!    where WGSL statements were invisible to the emit heuristic because they
//!    are wrapped in `source_file` (with `len == 2` due to the trailing `;`).
//!
//! 2. `typ` for target languages — WGSL, Bash, Zsh and HTML implement `typ`
//!    so holes carry the correct `InnerKind` for their syntactic position.
//!
//! 3. Cross-language emit — a WGSL/Bash statement quote in statement position
//!    inside a Rust block fires `OuterKind::Emit` (same as a Rust stmt quote).
//!
//! 4. `InnerKind::Block` — Rust's braced block gets its own kind so the
//!    tail-expression / statement-list distinction is exact.
//!
//! 5. Python `unwrap` respects `ikind` — when the caller passes an explicit
//!    `InnerKind` hint, Python's `unwrap` uses it instead of guessing.

use indoc::indoc;
use quilt::{
    lang::{flat_nodes, InnerKind},
    langs::omni::Omni,
    prelude::*,
    term::Term as _,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
}

// ══════════════════════════════════════════════════════════════════════════════
// 2. `typ` for target languages
// ══════════════════════════════════════════════════════════════════════════════

/// WGSL implements `typ` so every hole position carries the correct kind.
/// Prior to the fix, `WgslProvider::typ` returned `InnerKind::File` for all
/// tags (the `TSProvider` default).
#[test]
fn wgsl_typ_stmt_tags() {
    use quilt::langs::wgsl::lang::WgslProvider;
    use quilt::treesitter::TSProvider as _;
    let p = WgslProvider::default();
    // Statement-level WGSL nodes
    assert_eq!(p.typ("assignment_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("variable_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("return_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("if_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("for_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("while_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("loop_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("switch_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("compound_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("discard_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("break_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("continue_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("call_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("increment_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("decrement_statement"), InnerKind::Stmt);
    // Expression-level WGSL nodes
    assert_eq!(p.typ("binary_expression"), InnerKind::Expr);
    assert_eq!(p.typ("unary_expression"), InnerKind::Expr);
    assert_eq!(p.typ("identifier"), InnerKind::Expr);
    assert_eq!(p.typ("int_literal"), InnerKind::Expr);
    assert_eq!(p.typ("float_literal"), InnerKind::Expr);
    assert_eq!(p.typ("bool_literal"), InnerKind::Expr);
    assert_eq!(p.typ("bitcast_expression"), InnerKind::Expr);
    assert_eq!(p.typ("call_expression"), InnerKind::Expr);
    // File-level
    assert_eq!(p.typ("source_file"), InnerKind::File);
}

/// Bash implements `typ`.
/// Prior to the fix, all tags returned `InnerKind::File`.
#[test]
fn bash_typ_stmt_tags() {
    use quilt::langs::bash::lang::BashProvider;
    use quilt::treesitter::TSProvider as _;
    let p = BashProvider::default();
    assert_eq!(p.typ("command"), InnerKind::Stmt);
    assert_eq!(p.typ("if_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("for_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("while_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("function_definition"), InnerKind::Stmt);
    assert_eq!(p.typ("variable_assignment"), InnerKind::Stmt);
    assert_eq!(p.typ("declaration_command"), InnerKind::Stmt);
    // Expressions
    assert_eq!(p.typ("word"), InnerKind::Expr);
    assert_eq!(p.typ("string"), InnerKind::Expr);
    assert_eq!(p.typ("number"), InnerKind::Expr);
    assert_eq!(p.typ("concatenation"), InnerKind::Expr);
    assert_eq!(p.typ("command_substitution"), InnerKind::Expr);
    assert_eq!(p.typ("expansion"), InnerKind::Expr);
    // File
    assert_eq!(p.typ("program"), InnerKind::File);
}

/// Zsh implements `typ`.
/// Prior to the fix, all tags returned `InnerKind::File`.
#[test]
fn zsh_typ_stmt_tags() {
    use quilt::langs::zsh::lang::ZshProvider;
    use quilt::treesitter::TSProvider as _;
    let p = ZshProvider::default();
    assert_eq!(p.typ("command"), InnerKind::Stmt);
    assert_eq!(p.typ("if_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("for_statement"), InnerKind::Stmt);
    // Expressions
    assert_eq!(p.typ("word"), InnerKind::Expr);
    assert_eq!(p.typ("string"), InnerKind::Expr);
    // File
    assert_eq!(p.typ("program"), InnerKind::File);
}

/// HTML implements `typ`.
/// Prior to the fix, all tags returned `InnerKind::File`.
#[test]
fn html_typ_stmt_tags() {
    use quilt::langs::html::lang::HtmlProvider;
    use quilt::treesitter::TSProvider as _;
    let p = HtmlProvider::default();
    // Elements / block-like nodes → Stmt
    assert_eq!(p.typ("element"), InnerKind::Stmt);
    assert_eq!(p.typ("script_element"), InnerKind::Stmt);
    assert_eq!(p.typ("style_element"), InnerKind::Stmt);
    // Inline content → Expr
    assert_eq!(p.typ("text"), InnerKind::Expr);
    assert_eq!(p.typ("attribute_value"), InnerKind::Expr);
    assert_eq!(p.typ("raw_text"), InnerKind::Expr);
    // File
    assert_eq!(p.typ("document"), InnerKind::File);
}

// ══════════════════════════════════════════════════════════════════════════════
// 1. classify_term — accurate cross-language emit detection
// ══════════════════════════════════════════════════════════════════════════════

/// `classify_term` on WGSL correctly classifies a `source_file(stmt, ;)`
/// term as `InnerKind::Stmt` even though the root tag is `source_file`.
///
/// Before the fix this returned `InnerKind::File` because WGSL statement
/// fragments are wrapped in `source_file` with two children (the statement
/// node and the trailing `;`), so `terms.len() != 1` and `unwrap` fell
/// through to the "whole shader" branch.
#[test]
fn wgsl_classify_term_stmt() -> Result<()> {
    use quilt::lang::Language as _;
    use quilt::langs::wgsl::lang::WgslLanguage;
    let mut lang = WgslLanguage::default();
    // A WGSL assignment statement
    let q = lang.parse_auto(&flat_nodes("agents[idx].reg[0] = value;"))?;
    assert_eq!(lang.classify_term(&q), InnerKind::Stmt);
    Ok(())
}

/// `classify_term` on WGSL correctly classifies a single expression.
#[test]
fn wgsl_classify_term_expr() -> Result<()> {
    use quilt::lang::Language as _;
    use quilt::langs::wgsl::lang::WgslLanguage;
    let mut lang = WgslLanguage::default();
    let q = lang.parse_auto(&flat_nodes("x + y"))?;
    assert_eq!(lang.classify_term(&q), InnerKind::Expr);
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════
// 3. Cross-language emit
// ══════════════════════════════════════════════════════════════════════════════

/// A WGSL statement quote in a Rust function body (statement position) should
/// fire `OuterKind::Emit`, just like a Rust statement quote does.
///
/// Before the fix: WGSL fragments are wrapped in `source_file` with a
/// trailing `;`, so `terms.len() == 2` and the emit heuristic (which called
/// `wgsl_lang.typ("source_file")`) returned `InnerKind::File` → no emit.
/// After the fix: `classify_term` inspects the content and returns `Stmt` →
/// emit fires and the expansion contains `.emit(&mut b_)`.
#[test]
fn wgsl_stmt_quote_emits_in_rust_block() -> Result<()> {
    let out = expand(indoc! {r#"
        fn shader(val: &Arc<QTerm>) -> Arc<QTerm> {
            wgsl↖agents[idx].reg[0] = ↙val↘;↗
        }
    "#})?;
    println!("{out}");
    assert!(
        out.contains(".emit(&mut b_)"),
        "WGSL stmt quote should emit into Rust block builder, got:\n{out}"
    );
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════
// 4. InnerKind::Block
// ══════════════════════════════════════════════════════════════════════════════

/// Rust's `typ` returns `InnerKind::Block` for a braced block, not `Expr` or
/// `Stmt`.  This is needed so the tail-expression / statement-list distinction
/// is exact: a `{}` in tail-expression position should be `Block`, not `Stmt`.
///
/// Before the fix `InnerKind::Block` did not exist.
#[test]
fn rs_block_innerkind() {
    use quilt::langs::rust::lang::RustProvider;
    use quilt::treesitter::TSProvider as _;
    let p = RustProvider::default();
    assert_eq!(p.typ("block"), InnerKind::Block);
    // Other kinds unaffected
    assert_eq!(p.typ("expression_statement"), InnerKind::Stmt);
    assert_eq!(p.typ("let_declaration"), InnerKind::Stmt);
    assert_eq!(p.typ("binary_expression"), InnerKind::Expr);
    assert_eq!(p.typ("source_file"), InnerKind::File);
}

/// Parsing a Rust braced block returns a `QTerm` tagged `block`, and `parse_as`
/// with `InnerKind::Block` round-trips the source text correctly.
#[test]
fn rs_parse_block() -> Result<()> {
    use quilt::lang::Language as _;
    use quilt::langs::rust::lang::RustLanguage;
    use quilt::qterm::QTermTag;
    let mut lang = RustLanguage::default();
    let code = "{ let x = 1; x }";
    let q = lang.parse_as(Some(InnerKind::Block), &flat_nodes(code))?;
    assert_eq!(q.coparse(), code);
    assert_eq!(&q.tag(), &QTermTag::tuple("block"));
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════
// 5. Python `unwrap` respects `ikind`
// ══════════════════════════════════════════════════════════════════════════════

/// When the caller passes `InnerKind::Stmt`, Python's `unwrap` should not
/// squash an `expression_statement` down to its inner expression — it should
/// keep the statement wrapper so the result is clearly a statement.
///
/// Before the fix Python's `unwrap` ignored the `ikind` hint.
#[test]
fn py_unwrap_respects_stmt_ikind() -> Result<()> {
    use quilt::lang::Language as _;
    use quilt::langs::python::lang::PythonLanguage;
    let mut lang = PythonLanguage::default();
    // `f(x)` parses as `expression_statement(call(...))`.
    // With ikind=Stmt the result should be classified as Stmt (not Expr).
    let q = lang.parse_as(Some(InnerKind::Stmt), &flat_nodes("f(x)"))?;
    // The returned term should look like a statement (call is expression so
    // the expression_statement is squashed to `call` currently; with the
    // fix the InnerKind returned should be Stmt, i.e. the unwrap result
    // classifies it as Stmt).
    // We test by asking the language to classify the parsed term.
    assert_eq!(lang.classify_term(&q), InnerKind::Stmt);
    Ok(())
}

/// When the caller passes `InnerKind::Expr`, a call expression should be
/// classified as `Expr`, not `Stmt`.
#[test]
fn py_unwrap_respects_expr_ikind() -> Result<()> {
    use quilt::lang::Language as _;
    use quilt::langs::python::lang::PythonLanguage;
    let mut lang = PythonLanguage::default();
    let q = lang.parse_as(Some(InnerKind::Expr), &flat_nodes("f(x)"))?;
    assert_eq!(lang.classify_term(&q), InnerKind::Expr);
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════
// inner_kind() from LanguagePost — the feedback loop
// ══════════════════════════════════════════════════════════════════════════════

/// `parse_pre` returns a `LanguagePost` whose `inner_kind()` carries the
/// classified kind of the parsed fragment — the result from `unwrap`.
/// This closes the feedback loop described in the issue.
///
/// Before the fix `_ikind` was discarded (`treesitter.rs:253`) and there
/// was no way to retrieve it from the `LanguagePost` object.
#[test]
fn parse_pre_inner_kind_wgsl_stmt() -> Result<()> {
    use quilt::lang::{Language as _, LanguagePost as _};
    use quilt::langs::wgsl::lang::WgslLanguage;
    let mut lang = WgslLanguage::default();
    let code = flat_nodes("agents[idx].reg[0] = value;");
    let post = lang.parse_pre(None, &code)?;
    assert_eq!(post.inner_kind(), InnerKind::Stmt);
    Ok(())
}

#[test]
fn parse_pre_inner_kind_wgsl_expr() -> Result<()> {
    use quilt::lang::{Language as _, LanguagePost as _};
    use quilt::langs::wgsl::lang::WgslLanguage;
    let mut lang = WgslLanguage::default();
    let code = flat_nodes("x + y");
    let post = lang.parse_pre(None, &code)?;
    assert_eq!(post.inner_kind(), InnerKind::Expr);
    Ok(())
}

#[test]
fn parse_pre_inner_kind_rust_stmt() -> Result<()> {
    use quilt::lang::{Language as _, LanguagePost as _};
    use quilt::langs::rust::lang::RustLanguage;
    let mut lang = RustLanguage::default();
    let code = flat_nodes("let x = 1;");
    let post = lang.parse_pre(None, &code)?;
    assert_eq!(post.inner_kind(), InnerKind::Stmt);
    Ok(())
}
