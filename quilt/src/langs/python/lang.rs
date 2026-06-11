use crate::{
    lang::{Arity, InnerKind},
    qterm::{QTerm, QTermTag},
    term::Term,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use tree_sitter::Parser;

/**************************************************************/

pub struct PythonProvider(tree_sitter::Parser);

impl Default for PythonProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
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

    fn arity(&self, tag: &str) -> Arity {
        match tag {
            "module" | "block" => Arity::Variadic,
            _ => Arity::Unknown,
        }
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

    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        if qterm.len() != 1 {
            return (qterm, InnerKind::File);
        }
        let qterm = qterm.squash();
        if qterm.tag() == QTermTag::tuple("expression_statement") {
            // When the caller explicitly placed the hole in statement position,
            // honour that: keep the `expression_statement` wrapper and report
            // Stmt even for a non-assignment expression like `foo()`.
            if ikind == Some(InnerKind::Stmt) {
                return (qterm, InnerKind::Stmt);
            }
            // A bare tuple (`a, b`) keeps its elements directly under the
            // statement — there is no single inner node to squash to. Keep
            // the statement whole; bare tuples render without delimiters, so
            // the fragment splices flat into expression position.
            if qterm.len() != 1 {
                return (qterm, InnerKind::Expr);
            }
            let qterm = qterm.squash();
            if qterm.tag() == QTermTag::tuple("assignment") {
                // An assignment is always a statement, regardless of position.
                return (qterm, InnerKind::Stmt);
            }
            return (qterm, InnerKind::Expr);
        }
        // If the caller explicitly expected an expression (e.g. the hole was
        // in expression position), trust that over the default Stmt guess.
        if ikind == Some(InnerKind::Expr) {
            return (qterm, InnerKind::Expr);
        }
        (qterm, InnerKind::Stmt)
    }
}

pub type PythonLanguage = TSLanguage<PythonProvider>;
pub type DynPythonLanguage = DynTSLanguage<PythonProvider>;
