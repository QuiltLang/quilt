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
use tree_sitter::Parser;

/**************************************************************/

pub struct WgslProvider(tree_sitter::Parser);

impl Default for WgslProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_wgsl::LANGUAGE.into())
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
    /// Unlike in Rust, WGSL statement fragments are typically wrapped in
    /// `source_file` with **two** children — the statement node and a trailing
    /// `;` sibling — so `terms.len() != 1` and the squash is not attempted.
    /// The returned `InnerKind` is determined by inspecting the child(ren),
    /// and is now stored in [`TSLanguagePost::inner_kind`] rather than
    /// discarded.
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return (qterm, InnerKind::default());
        };
        if &**tag != "source_file" {
            return (qterm, InnerKind::default());
        }
        match terms.len() {
            0 => (qterm, InnerKind::File),
            1 => {
                // Single child: expression or top-level declaration.
                let kind = match &*terms[0] {
                    QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
                    _ => InnerKind::Stmt,
                };
                (qterm.squash(), kind)
            }
            2 => {
                // A WGSL statement with trailing `;`: `source_file(stmt, ;)`.
                // The fragment is a statement even though it has two children.
                // Check whether the second child is a bare semicolon leaf.
                let is_semi = match &*terms[1] {
                    QTerm::Tuple { tag, terms: semi_terms, .. } => {
                        &**tag == ";" && semi_terms.is_empty()
                    }
                    _ => false,
                };
                if is_semi {
                    // Single statement + trailing `;` — classify as Stmt.
                    // Do not squash: the semicolon is part of the output.
                    (qterm, InnerKind::Stmt)
                } else {
                    // Two top-level declarations (e.g. a global + a function).
                    (qterm, InnerKind::File)
                }
            }
            _ => {
                // Multiple top-level items: a whole (partial) shader.
                (qterm, InnerKind::File)
            }
        }
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

    /// Classify a WGSL tag as expression, statement, or file-level.
    fn typ(&self, tag: &str) -> InnerKind {
        if tag == "source_file" {
            InnerKind::File
        } else if is_expr_tag(tag) {
            InnerKind::Expr
        } else {
            // Everything else is statement-level (assignments, control flow,
            // top-level declarations, compound statements, …).
            InnerKind::Stmt
        }
    }

    /// Classify a fully-parsed WGSL term.
    ///
    /// Overrides the default to handle `source_file(stmt, ;)` fragments:
    /// a single WGSL statement wrapped in `source_file` with a trailing `;`
    /// sibling has `terms.len() == 2`, so `typ("source_file")` alone would
    /// return `File`.  Here we look at the children to return `Stmt`.
    fn classify_term(&self, term: &QTerm) -> InnerKind {
        match term {
            QTerm::Tuple { tag, terms, .. } if &**tag == "source_file" => match terms.len() {
                0 => InnerKind::File,
                1 => match &*terms[0] {
                    QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
                    _ => InnerKind::Stmt,
                },
                2 => {
                    // Check for stmt + trailing `;` pattern.
                    let is_semi = match &*terms[1] {
                        QTerm::Tuple { tag, terms: semi_terms, .. } => {
                            &**tag == ";" && semi_terms.is_empty()
                        }
                        _ => false,
                    };
                    if is_semi { InnerKind::Stmt } else { InnerKind::File }
                }
                _ => InnerKind::File,
            },
            QTerm::Tuple { tag, .. } => self.typ(tag),
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
