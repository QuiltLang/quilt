use crate::{
    lang::{Arity, Comments, InnerKind},
    qterm::{QTerm, QTermTag},
    term::Term,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::Result;
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

    /// `repr()` answers a query with Python's own literal for the value,
    /// which is what makes a machine answer the inverse of lift. The
    /// `PYTHONPATH` entry resolves `from quilt import *` against the package
    /// next to this crate — the same path `reduce_py` teaches its one-shot
    /// script — so a machine can be fed expanded meta-code, not just plain
    /// Python.
    /// The spec itself is data, so it lives in the runtime-side table
    /// (`machine::script_spec`) that `qspawn` also reads — one source of
    /// truth for the registry, the battery and `⟨M⟩` (issue #273).
    fn machine_spec(&self) -> Option<crate::machine::MachineSpec> {
        crate::machine::script_spec("python")
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
        // Quilt does not put the wrapper back. The statement-ness of a fragment is
        // reported through the returned `InnerKind`, not encoded as a node quilt
        // invented — keeping a wrapper alive so the kind could be read back off a
        // tag would tie the expanded form to an implementation detail of the
        // grammar, which is exactly the coupling Quilt is meant to avoid (#184).
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
            // An expression the caller explicitly placed in statement position.
            // The term is unchanged; only the reported kind says so.
            InnerKind::Expr if ikind == Some(InnerKind::Stmt) => Ok((qterm, InnerKind::Stmt)),
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
