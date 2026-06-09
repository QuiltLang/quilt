//! The HTML target language.
//!
//! HTML is only ever a *quoted* language (e.g. `html↖ … ↗` inside Rust): Quilt
//! parses HTML fragments into a `QTerm` and the host language's `MetaLanguage`
//! (Rust's) drives expansion. There is therefore no `HtmlMetaLanguage` — only
//! this `Language` impl, built on the shared tree-sitter helper.
//!
//! The forked grammar accepts holes in node position, interleaved with the
//! `raw_text` of `<script>`/`<style>` elements, and in attribute values — the
//! places interpolation is needed when templating a page.

use crate::{
    lang::{Arity, InnerKind},
    qterm::QTerm,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use tree_sitter::Parser;

/**************************************************************/

pub struct HtmlProvider(tree_sitter::Parser);

impl Default for HtmlProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .expect("Error loading HTML parser");
        Self(parser)
    }
}

impl TSProvider for HtmlProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // Matches `quilt_hole: $ => "__QUILT_HOLE__"` in the forked grammar.
        "__QUILT_HOLE__"
    }

    /// Squash the `document` wrapper around a single quoted fragment so the
    /// term is the fragment itself (element / text / …). A multi-node fragment
    /// (a whole page) stays a `document`.
    ///
    /// The returned `InnerKind` is advisory only (`parse_pre` discards it); we
    /// never panic on an unexpected shape, like the WGSL provider.
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return (qterm, InnerKind::default());
        };
        if &**tag != "document" {
            return (qterm, InnerKind::default());
        }
        if terms.len() != 1 {
            // empty file, or several top-level nodes (a whole page)
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
            // The containers with `repeat(…)` children in the grammar.
            "document"
            | "element"
            | "script_element"
            | "style_element"
            | "start_tag"
            | "self_closing_tag"
            | "quoted_attribute_value" => Arity::Variadic,
            _ => Arity::Unknown,
        }
    }

    fn hashbang(&self) -> Option<&'static str> {
        None
    }
}

/// Tags that are HTML "expressions" (used only to label squashed
/// single-fragment quotes; the label is advisory).
fn is_expr_tag(tag: &str) -> bool {
    matches!(
        tag,
        "text" | "entity" | "raw_text" | "attribute_value" | "quilt_hole"
    )
}

pub type HtmlLanguage = TSLanguage<HtmlProvider>;
pub type DynHtmlLanguage = DynTSLanguage<HtmlProvider>;
