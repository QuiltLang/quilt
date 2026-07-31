//! The Zsh target language.
//!
//! Zsh is only ever a *quoted* language (e.g. `zsh↖ … ↗` inside Rust): Quilt
//! parses Zsh fragments into a `QTerm` and the host language's `MetaLanguage`
//! drives expansion. There is therefore no `ZshMetaLanguage` — only this
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

pub struct ZshProvider(tree_sitter::Parser);

impl Default for ZshProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::zsh::LANGUAGE.into())
            .expect("Error loading Zsh parser");
        Self(parser)
    }
}

impl TSProvider for ZshProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // The grammar.js fork already defines `quilt_hole` and adds it to
        // statement/expression positions, but parser.c hasn't been regenerated
        // yet (tree-sitter generate takes very long on this grammar).  Until
        // then `__QUILT_HOLE__` parses as a `word` node, and range-based hole
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

    /// Delegated to [`crate::langs::shell`], which bash shares (issue #150): the
    /// two dialects had two independently maintained copies of this table, and
    /// they drifted.
    fn arity(&self, tag: &str) -> Arity {
        shell::arity(tag)
    }

    /// The shell grammars have no `identifier` node kind at all — a bare word is
    /// a `word` — so the trait default would tag a deferred operator with a kind
    /// this grammar does not define.
    fn ident_tag(&self) -> &'static str {
        "word"
    }

    fn hashbang(&self) -> Option<&'static str> {
        Some("#!/usr/bin/env zsh")
    }
}

pub type ZshLanguage = TSLanguage<ZshProvider>;
pub type DynZshLanguage = DynTSLanguage<ZshProvider>;
