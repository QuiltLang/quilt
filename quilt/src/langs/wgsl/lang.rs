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
    /// The returned `InnerKind` is advisory only (`parse_pre` discards it); we
    /// never panic on an unexpected shape, unlike the Rust provider.
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return (qterm, InnerKind::default());
        };
        if &**tag != "source_file" {
            return (qterm, InnerKind::default());
        }
        if terms.len() != 1 {
            // empty file, or several top-level declarations (a whole shader)
            return (qterm, InnerKind::File);
        }
        let kind = match &*terms[0] {
            QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
            _ => InnerKind::Stmt,
        };
        (qterm.squash(), kind)
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
