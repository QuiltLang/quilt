//! Tests for the inner/outer kind machinery described in issue #25.
//!
//! These tests cover the parts of the issue this change addresses:
//!
//! 1. `classify_term` — languages can inspect the full parsed term to determine
//!    its kind, not just the root tag name.  This closes the "feedback loop"
//!    where WGSL statements were invisible to the emit heuristic because they
//!    are wrapped in `source_file` (with `len == 2` due to the trailing `;`).
//!
//! 2. Cross-language emit — a WGSL statement quote in statement position inside
//!    a Rust block fires `OuterKind::Emit` (same as a Rust stmt quote), so the
//!    built term is emitted instead of silently dropped.
//!
//! 3. Python `unwrap` respects `ikind` — when the caller passes an explicit
//!    `InnerKind` hint, Python's `unwrap` uses it instead of guessing.
//!
//! (Issue #25 items #2 `typ` for target languages and #4 `InnerKind::Block`
//! are deferred: both need the emit/splice heuristic to classify a child by its
//! *own* language rather than the enclosing quote's before they can be added
//! without breaking the existing splice behaviour.)

use indoc::indoc;
use quilt::{
    lang::{flat_nodes, InnerKind},
    langs::omni::Omni,
    prelude::*,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
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
// 2. Cross-language emit
// ══════════════════════════════════════════════════════════════════════════════

/// A WGSL statement quote in a Rust function body (statement position) should
/// fire `OuterKind::Emit`, just like a Rust statement quote does: the term the
/// quote builds is emitted into the enclosing Rust builder instead of being
/// built and silently dropped.
///
/// Before the fix: WGSL fragments are wrapped in `source_file` with a trailing
/// `;`, so `terms.len() == 2`; the emit heuristic classified the body with
/// `wgsl.typ("source_file")` → `InnerKind::File` → no emit. After the fix the
/// heuristic uses `wgsl.classify_term(body)`, which inspects the children and
/// returns `Stmt`, so emit fires.
///
/// The signal is the *outer* `}.emit(&mut b_);` — the whole WGSL-building block
/// emitted into the Rust builder. (The inner `.emit(&mut b_)` calls belong to
/// WGSL's own block builder and are present regardless, so a bare
/// `.emit(&mut b_)` substring is *not* a reliable check.)
#[test]
fn wgsl_stmt_quote_emits_in_rust_block() -> Result<()> {
    let out = expand(indoc! {r#"
        fn shader(val: &Arc<QTerm>) -> Arc<QTerm> {
            wgsl↖agents[idx].reg[0] = ↙val↘;↗
        }
    "#})?;
    println!("{out}");
    assert!(
        out.contains("}.emit(&mut b_);"),
        "WGSL stmt quote should emit the built term into the Rust block builder, got:\n{out}"
    );
    Ok(())
}

/// The contrast to [`wgsl_stmt_quote_emits_in_rust_block`]: a WGSL *expression*
/// quote in tail position is a value, not a statement, so it must stay the
/// block's value and *not* be emitted. `wgsl.classify_term` returns `Expr` for
/// a bare expression body, so the emit heuristic does not fire.
#[test]
fn wgsl_expr_quote_stays_value_in_rust_block() -> Result<()> {
    let out = expand(indoc! {r#"
        fn shader() -> Arc<QTerm> {
            wgsl↖x + y↗
        }
    "#})?;
    println!("{out}");
    assert!(
        !out.contains("}.emit(&mut b_);"),
        "WGSL expr quote should stay a value, not emit, got:\n{out}"
    );
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════
// 3. Python `unwrap` respects `ikind`
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
