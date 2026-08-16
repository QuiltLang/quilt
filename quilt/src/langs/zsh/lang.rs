//! The Zsh target language.
//!
//! Zsh is only ever a *quoted* language (e.g. `zsh↖ … ↗` inside Rust): Quilt
//! parses Zsh fragments into a `QTerm` and the host language's `MetaLanguage`
//! drives expansion. There is therefore no `ZshMetaLanguage` — only this
//! `Language` impl, built on the shared tree-sitter helper.

use crate::{
    lang::{Arity, Comments, InnerKind},
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
        // statement/expression positions, but parser.c has never been
        // regenerated from it — and cannot be: `tree-sitter generate` panics on
        // that rule, so the fork parks it (QuiltLang/tree-sitter-zsh#1).
        //
        // Nothing here depends on it. `__QUILT_HOLE__` parses as a `word`, and
        // `treesitter.rs` finds a hole by byte range: as its own node where it
        // stands alone, and otherwise by splitting the token that swallowed it
        // — a string, a comment, or a longer word (issue #221).
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

    /// Derived from the grammar's `REPEAT` rules by `bin/gen-arity`, not
    /// hand-curated — see `quilt/src/langs/arity.rs` (#202).
    ///
    /// Zsh's grammar is a fork of bash's, so the two tables mostly coincide
    /// without being maintained in step (#150) and without the shared
    /// hand-written table that issue first reached for. Where they part company
    /// it is now a grammar fact rather than drift: `function_definition` is
    /// variadic here and not in bash because zsh's rule takes
    /// `repeat1(field('name', …))` — `function a b c { … }` defines three
    /// functions at once, which bash has no syntax for.
    /// `bash_and_zsh_agree_on_shared_kinds` in
    /// `quilt-conformance/tests/grammar_tags.rs` pins the exceptions, so a
    /// *new* divergence still has to be looked at.
    fn arity(&self, tag: &str) -> Arity {
        Arity::from_table(crate::langs::arity::ZSH, tag)
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

impl Comments for ZshLanguage {
    const LINE: Option<&'static str> = Some("#");
}
