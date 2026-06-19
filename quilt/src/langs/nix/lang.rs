//! The Nix target language.
//!
//! Nix is only ever a *quoted* language (e.g. `nix↖ … ↗` inside Rust): Quilt
//! parses Nix fragments into a `QTerm` and the host language's `MetaLanguage`
//! drives expansion. There is therefore no `NixMetaLanguage` — only this
//! `Language` impl, built on the shared tree-sitter helper.
//!
//! Nix is purely expression-oriented: a whole file is a single expression
//! wrapped in `source_code`, and every fragment is an expression (its node tag
//! ends in `_expression`). There are no statements or items, so `unwrap` only
//! ever yields `Expr` (a single fragment) or `File` (a whole file we can't
//! squash to one expression — empty, or an expression preceded by comments).

use crate::{
    lang::{Arity, InnerKind},
    qterm::QTerm,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::Result;
use tree_sitter::Parser;

/**************************************************************/

pub struct NixProvider(tree_sitter::Parser);

impl Default for NixProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::nix::LANGUAGE.into())
            .expect("Error loading Nix parser");
        Self(parser)
    }
}

impl TSProvider for NixProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // `__QUILT_HOLE__` is a valid Nix identifier
        // (`[a-zA-Z_][a-zA-Z0-9_'-]*`), so it parses as a `variable_expression`
        // in any expression position. The range-based hole detection in
        // `treesitter.rs` then recognises it by its byte range, exactly as for
        // the other vendored grammars — no `quilt_hole` node needed.
        "__QUILT_HOLE__"
    }

    /// Squash the `source_code` wrapper around a single quoted fragment so the
    /// term is the expression itself. Nix is expression-only, so a single
    /// fragment is always an `Expr`; a `source_code` we can't reduce to one
    /// child (empty, or an expression preceded by comments) stays `File`.
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return Ok((qterm, InnerKind::default()));
        };
        if &**tag != "source_code" {
            return Ok((qterm, InnerKind::default()));
        }
        if terms.len() != 1 {
            return Ok((qterm, InnerKind::File));
        }
        Ok((qterm.squash(), InnerKind::Expr))
    }

    /// Nix sequence containers — list literals (`[ … ]`), attribute/binding
    /// sets (`{ … }`, `let … in`, `rec { … }`), `inherit (…) a b;` attr lists,
    /// and function formals (`{ a, b, ... }:`) — hold a variable number of
    /// same-kind children, so their host expansion is a `build_variadic_block`
    /// that emits each child. Everything else has a fixed shape and is rebuilt
    /// positionally.
    fn arity(&self, tag: &str) -> Arity {
        match tag {
            "source_code" | "list_expression" | "binding_set" | "inherited_attrs" | "formals" => {
                Arity::Variadic
            }
            _ => Arity::Unknown,
        }
    }

    /// Every Nix fragment is an expression (its node tag ends in `_expression`),
    /// so a hole demands an `Expr`. Non-expression tags (`binding`, `attrpath`,
    /// …) fall back to the `File` default — a `__QUILT_HOLE__` always parses as
    /// a `variable_expression`, so holes never land there.
    fn typ(&self, tag: &str) -> InnerKind {
        if is_expr_tag(tag) {
            InnerKind::Expr
        } else {
            InnerKind::default()
        }
    }

    fn hashbang(&self) -> Option<&'static str> {
        None
    }
}

/// Tags that are Nix expressions. Every `_expression` subtype in the grammar
/// ends in `_expression`, which also covers the `variable_expression` a hole
/// (`__QUILT_HOLE__`) parses as.
fn is_expr_tag(tag: &str) -> bool {
    tag.ends_with("_expression")
}

pub type NixLanguage = TSLanguage<NixProvider>;
pub type DynNixLanguage = DynTSLanguage<NixProvider>;
