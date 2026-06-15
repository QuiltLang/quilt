//! The WGSL target language.
//!
//! WGSL is only ever a *quoted* language (e.g. `wgsl↖ … ↗` inside Rust): Quilt
//! parses WGSL fragments into a `QTerm` and the host language's `MetaLanguage`
//! (Rust's) drives expansion. There is therefore no `WgslMetaLanguage` — only
//! this `Language` impl, built on the shared tree-sitter helper.

use crate::{
    lang::{Arity, InnerKind},
    qterm::QTerm,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::Result;
use tree_sitter::Parser;

/**************************************************************/

pub struct WgslProvider(tree_sitter::Parser);

impl Default for WgslProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::wgsl::LANGUAGE.into())
            .expect("Error loading WGSL parser");
        Self(parser)
    }
}

impl TSProvider for WgslProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // Matches `quilt_hole: $ => "__QUILT_HOLE__"` in the forked grammar.
        "__QUILT_HOLE__"
    }

    /// Squash the `source_file` wrapper around a single quoted fragment so the
    /// term is the fragment itself (expression / statement / declaration). A
    /// multi-declaration fragment (a whole shader) stays a `source_file`.
    ///
    /// The returned `InnerKind` is advisory only (`parse_pre` discards it; the
    /// emit heuristic re-derives the kind from the term via `classify_term`).
    /// We accept any shape here rather than rejecting unrecognised ones, unlike
    /// the Rust provider (which errors on shapes it can't place).
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return Ok((qterm, InnerKind::default()));
        };
        if &**tag != "source_file" {
            return Ok((qterm, InnerKind::default()));
        }
        if terms.len() != 1 {
            // empty file, or several top-level declarations (a whole shader),
            // or a statement plus its trailing `;` — all kept whole. The
            // statement/`;` shape is recognised by `classify_term` below.
            return Ok((qterm, InnerKind::File));
        }
        let kind = match &*terms[0] {
            QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
            _ => InnerKind::Stmt,
        };
        Ok((qterm.squash(), kind))
    }

    fn arity(&self, tag: &str) -> Arity {
        match tag {
            "source_file"
            | "compound_statement"
            | "case_compound_statement"
            | "switch_statement"
            | "struct_declaration" => Arity::Variadic,
            _ => Arity::Unknown,
        }
    }

    /// Classify a fully-parsed WGSL term as expression / statement / file.
    ///
    /// This is what closes the feedback loop for the emit heuristic (issue
    /// #25): unlike `typ`, which only sees a root tag, `classify_term` inspects
    /// the whole term. WGSL needs it because a single statement fragment is
    /// wrapped in `source_file` with a trailing `;` sibling, so `terms.len() ==
    /// 2` and the root tag alone (`source_file`) would read as `File` even
    /// though the fragment is really a `Stmt`.
    fn classify_term(&self, term: &QTerm) -> InnerKind {
        match term {
            QTerm::Tuple { tag, terms, .. } if &**tag == "source_file" => match terms.len() {
                1 => match &*terms[0] {
                    QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
                    _ => InnerKind::Stmt,
                },
                2 => {
                    // A single statement plus its trailing `;`: still a `Stmt`.
                    let is_semi = match &*terms[1] {
                        QTerm::Tuple {
                            tag,
                            terms: semi_terms,
                            ..
                        } => &**tag == ";" && semi_terms.is_empty(),
                        _ => false,
                    };
                    if is_semi {
                        InnerKind::Stmt
                    } else {
                        InnerKind::File
                    }
                }
                // Empty (0) or several top-level declarations (3+): a whole
                // (or partial) shader.
                _ => InnerKind::File,
            },
            QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
            QTerm::Tuple { .. } => InnerKind::Stmt,
            _ => InnerKind::default(),
        }
    }

    fn hashbang(&self) -> Option<&'static str> {
        None
    }
}

/// Tags that are WGSL expressions (used only to label squashed single-fragment
/// quotes; the label is advisory).
fn is_expr_tag(tag: &str) -> bool {
    tag.ends_with("_expression")
        || matches!(
            tag,
            "int_literal"
                | "float_literal"
                | "bool_literal"
                | "const_literal"
                | "identifier"
                | "quilt_hole"
        )
}

pub type WgslLanguage = TSLanguage<WgslProvider>;
pub type DynWgslLanguage = DynTSLanguage<WgslProvider>;
