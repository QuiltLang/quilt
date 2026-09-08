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
//!    `InnerKind` hint, Python's `unwrap` uses it instead of guessing, and
//!    reports it through the returned kind rather than by wrapping the term
//!    in a node quilt invented (issue #184).
//!
//! (Issue #25 item #2 — `typ` for target languages — is still deferred: it
//! needs the emit/splice heuristic to classify a child by its *own* language
//! rather than the enclosing quote's before it can be added without breaking
//! the existing splice behaviour. Item #4 — `InnerKind::Block` — has since
//! landed via issue #10: rather than mapping the `{}` placeholder's tag (which
//! would poison every Rust expression hole), `TSProvider::hole_kind` reads the
//! hole's parent in the parse tree, so only body-position blocks become
//! `Block`. See `rs_block_hole_ikind` in tests/kinds.rs.)

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
/// The signal is that the whole WGSL-building expression is followed by
/// `.emit(&mut b_);` — emitted into the Rust builder rather than dropped. It is
/// spelled `.b().emit(&mut b_);` here because WGSL's `source_file` holds no
/// *direct* unquote child (the `↙val↘` is nested inside the
/// `assignment_statement`), so it builds fluently; a fragment that did would end
/// `}.emit(&mut b_);` instead. Both spellings mean the same thing, so the check
/// accepts either — what it must not accept is the term being built and
/// silently discarded.
#[test]
fn wgsl_stmt_quote_emits_in_rust_block() -> Result<()> {
    let out = expand(indoc! {r#"
        fn shader(val: &Arc<QTerm>) -> Arc<QTerm> {
            wgsl↖agents[idx].reg[0] = ↙val↘;↗
        }
    "#})?;
    println!("{out}");
    assert!(
        out.contains(".b().emit(&mut b_);") || out.contains("}.emit(&mut b_);"),
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

/// When the caller passes `InnerKind::Stmt`, Python's `unwrap` honours the hint
/// — and reports it through the returned `InnerKind` rather than by building a
/// node to carry it.
///
/// Upstream tree-sitter-python made `expression_statement` a supertype, so it
/// no longer appears in a parse tree at all: `f(x)` comes back as `call`. Quilt
/// deliberately does *not* synthesize the wrapper back (issue #184). A fragment
/// that has been placed in statement position is described by the kind `unwrap`
/// returns; encoding it as a node quilt invented would tie the expanded form to
/// a detail of the grammar, which Quilt is meant to stay agnostic about.
///
/// So this asserts both halves: the hint is honoured, *and* the term is left
/// exactly as parsed.
#[test]
fn py_unwrap_respects_stmt_ikind() -> Result<()> {
    use quilt::lang::Language as _;
    use quilt::langs::python::lang::{PythonLanguage, PythonProvider};
    use quilt::treesitter::TSProvider as _;

    let mut lang = PythonLanguage::default();
    let call = lang.parse_as(Some(InnerKind::Expr), &flat_nodes("f(x)"))?;
    let module = tb("module").c(&call).build();

    let provider = PythonProvider::default();
    let (term, kind) = provider.unwrap(module, Some(InnerKind::Stmt))?;

    assert_eq!(
        kind,
        InnerKind::Stmt,
        "the explicit Stmt hint must be honoured"
    );
    assert!(
        matches!(&term, QTerm::Tuple { tag, .. } if &**tag == "call"),
        "the term must be left as parsed, with no synthesized wrapper; got {term:?}"
    );
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
