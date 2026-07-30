//! The Bash target language.
//!
//! Bash is only ever a *quoted* language (e.g. `bash↖ … ↗` inside Rust): Quilt
//! parses Bash fragments into a `QTerm` and the host language's `MetaLanguage`
//! drives expansion. There is therefore no `BashMetaLanguage` — only this
//! `Language` impl, built on the shared tree-sitter helper.

use crate::{
    lang::{Arity, InnerKind},
    langs::shell,
    qterm::QTerm,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::Result;
use tree_sitter::Parser;

/**************************************************************/

pub struct BashProvider(tree_sitter::Parser);

impl Default for BashProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::bash::LANGUAGE.into())
            .expect("Error loading Bash parser");
        Self(parser)
    }
}

impl TSProvider for BashProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // The grammar.js fork defines `quilt_hole` and adds it to
        // statement/expression positions, but parser.c hasn't been regenerated
        // (tree-sitter generate is slow on this grammar).  Until then
        // `__QUILT_HOLE__` parses as a `word` node, and the range-based hole
        // detection in `treesitter.rs` still works correctly.
        "__QUILT_HOLE__"
    }

    /// Squash the `program` wrapper around a single quoted fragment so the
    /// term is the fragment itself (command / statement). A multi-statement
    /// fragment (a whole script) stays a `program`.
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return Ok((qterm, InnerKind::default()));
        };
        if &**tag != "program" {
            return Ok((qterm, InnerKind::default()));
        }
        if terms.len() != 1 {
            return Ok((qterm, InnerKind::File));
        }
        let kind = match &*terms[0] {
            QTerm::Tuple { tag, .. } if shell::is_expr_tag(tag) => InnerKind::Expr,
            _ => InnerKind::Stmt,
        };
        Ok((qterm.squash(), kind))
    }

    /// Delegated to [`crate::langs::shell`], which zsh shares (issue #150): the
    /// two dialects had two independently maintained copies of this table, and
    /// they drifted.
    fn arity(&self, tag: &str) -> Arity {
        shell::arity(tag)
    }

    fn hashbang(&self) -> Option<&'static str> {
        Some("#!/usr/bin/env bash")
    }
}

pub type BashLanguage = TSLanguage<BashProvider>;
pub type DynBashLanguage = DynTSLanguage<BashProvider>;
