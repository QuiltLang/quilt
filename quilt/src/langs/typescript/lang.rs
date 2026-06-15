use crate::{
    lang::{Arity, InnerKind},
    qterm::{QTerm, QTermTag},
    term::Term,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::Result;
use tree_sitter::Parser;

/**************************************************************/

pub struct TypeScriptProvider(tree_sitter::Parser);

impl Default for TypeScriptProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::typescript::LANGUAGE_TYPESCRIPT.into())
            .expect("Error loading TypeScript parser");
        Self(parser)
    }
}

impl TSProvider for TypeScriptProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // Matches `quilt_hole: _ => '__HOLE__'` in the forked grammar.
        "__HOLE__"
    }

    fn hashbang(&self) -> Option<&'static str> {
        Some("#!/usr/bin/env -S node --experimental-strip-types")
    }

    fn arity(&self, tag: &str) -> Arity {
        match tag {
            // The block-like containers with `repeat(…)` children.
            "program" | "statement_block" | "class_body" | "object" | "array" | "arguments"
            | "named_imports" | "object_pattern" | "array_pattern" | "switch_body" => {
                Arity::Variadic
            }
            _ => Arity::Unknown,
        }
    }

    fn typ(&self, tag: &str) -> InnerKind {
        if tag == "program" {
            InnerKind::File
        } else if tag.ends_with("_statement")
            || tag.ends_with("_declaration")
            || tag.ends_with("_definition")
        {
            InnerKind::Stmt
        } else {
            InnerKind::Expr
        }
    }

    /// Strip the `program` root around a quoted fragment and infer whether the
    /// content is an expression, statement, or whole file — mirroring the
    /// Python provider. A single top-level `expression_statement` squashes to
    /// its inner expression (so `ts↖foo()↗` splices in expression position);
    /// the hole's known position (`ikind`) settles the ambiguous cases.
    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        // empty file, or several top-level nodes — keep the `program` whole.
        if qterm.len() != 1 {
            return Ok((qterm, InnerKind::File));
        }
        let qterm = qterm.squash();
        if qterm.tag() == QTermTag::tuple("expression_statement") {
            // An expression with a trailing `;` (len 2) or a bare comma
            // sequence: keep the statement, but it splices flat as an
            // expression.
            if qterm.len() != 1 {
                return Ok((qterm, InnerKind::Expr));
            }
            let inner = qterm.squash();
            // When the caller explicitly placed the hole in statement position,
            // honour that (keep the `expression_statement`); otherwise treat a
            // bare expression statement as an expression.
            if ikind == Some(InnerKind::Stmt) {
                return Ok((qterm, InnerKind::Stmt));
            }
            return Ok((inner, InnerKind::Expr));
        }
        // A declaration / control-flow statement / other top-level node. Trust
        // an explicit expression expectation over the default Stmt guess.
        if ikind == Some(InnerKind::Expr) {
            return Ok((qterm, InnerKind::Expr));
        }
        let tag = match &qterm {
            QTerm::Tuple { tag, .. } => self.typ(tag),
            _ => InnerKind::Stmt,
        };
        Ok((qterm, tag))
    }
}

pub type TypeScriptLanguage = TSLanguage<TypeScriptProvider>;
pub type DynTypeScriptLanguage = DynTSLanguage<TypeScriptProvider>;
