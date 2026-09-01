use crate::{
    lang::{Arity, Comments, InnerKind},
    qterm::{tb, QTerm, QTermTag},
    term::Term,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::Result;
use std::sync::Arc;
use tree_sitter::Parser;

/**************************************************************/

pub struct PythonProvider(tree_sitter::Parser);

impl Default for PythonProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::python::LANGUAGE.into())
            .expect("Error loading Python parser");
        Self(parser)
    }
}

impl TSProvider for PythonProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        "__HOLE__"
    }

    fn hashbang(&self) -> Option<&'static str> {
        Some("#!/usr/bin/env python3")
    }

    /// Derived from the grammar's `REPEAT` rules by `bin/gen-arity`, not
    /// hand-curated — see `quilt/src/langs/arity.rs` (#202).
    fn arity(&self, tag: &str) -> Arity {
        Arity::from_table(crate::langs::arity::PYTHON, tag)
    }

    fn typ(&self, tag: &str) -> InnerKind {
        if tag == "module" {
            InnerKind::File
        } else if tag == "assignment" || tag.ends_with("statement") || tag.ends_with("definition") {
            InnerKind::Stmt
        } else {
            InnerKind::Expr
        }
    }

    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        if qterm.len() != 1 {
            return Ok((qterm, InnerKind::File));
        }
        let qterm = qterm.squash();
        // Upstream marks `expression_statement` as a supertype (tree-sitter-python
        // `26855eab`), so the node no longer appears in a parse tree: `f(x)` comes
        // back as `call`, not `expression_statement(call(...))`.
        //
        // Quilt still uses that tag to *mean* "this fragment sits in statement
        // position" — `classify_term` reads the tag alone. So where this used to
        // match a wrapper the parser produced, it now synthesizes one. The rule is
        // still in the grammar (a supertype is hidden from trees, not deleted), and
        // a `QTerm` tag is quilt's own IR rather than an obligation to mirror
        // tree-sitter, so generated code is unchanged either way.
        let QTermTag::Tuple(name) = qterm.tag() else {
            return Ok((qterm, ikind.unwrap_or(InnerKind::Expr)));
        };
        if &*name == "tuple_expression" {
            // A bare tuple (`a, b`) renders without delimiters, so the fragment
            // splices flat into expression position. Keep it whole rather than
            // squashing past it. Upstream moved this case out of
            // `expression_statement` into its own node in the same release.
            return Ok((qterm, InnerKind::Expr));
        }
        if &*name == "assignment" {
            // An assignment is always a statement, regardless of position.
            return Ok((qterm, InnerKind::Stmt));
        }
        match self.typ(&name) {
            // An expression the caller explicitly placed in statement position:
            // give it the wrapper back, so it classifies as a statement.
            InnerKind::Expr if ikind == Some(InnerKind::Stmt) => {
                let wrapped = tb("expression_statement").c(&Arc::new(qterm)).build();
                Ok((wrapped, InnerKind::Stmt))
            }
            InnerKind::Expr => Ok((qterm, InnerKind::Expr)),
            // Already statement-shaped (`if_statement`, `function_definition`, …).
            // An explicit `Expr` hint still wins, as it did before.
            _ if ikind == Some(InnerKind::Expr) => Ok((qterm, InnerKind::Expr)),
            _ => Ok((qterm, InnerKind::Stmt)),
        }
    }
}

pub type PythonLanguage = TSLanguage<PythonProvider>;
pub type DynPythonLanguage = DynTSLanguage<PythonProvider>;

impl Comments for PythonLanguage {
    const LINE: Option<&'static str> = Some("#");
}
