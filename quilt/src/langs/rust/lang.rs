use crate::{
    lang::{Arity, InnerKind},
    qterm::QTerm,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::{miette, Result};
use tree_sitter::Parser;

/**************************************************************/

pub struct RustProvider(tree_sitter::Parser);

impl Default for RustProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("Error loading Rust parser");
        Self(parser)
    }
}

impl TSProvider for RustProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        "{}"
    }

    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        // dbg!(&qterm);
        #[allow(clippy::single_match)]
        Ok(match &qterm {
            QTerm::Tuple { tag, terms, .. } => match &**tag {
                "source_file" => {
                    if terms.len() == 1 {
                        let q0 = &terms[0];
                        match &**q0 {
                            QTerm::Tuple { tag, .. } => {
                                // The hole's position (when known) settles
                                // ambiguous bodies like a bare `{ }`; otherwise
                                // guess from the tag.
                                let kind = match ikind {
                                    Some(k) if k != InnerKind::File => k,
                                    _ if **tag == *"{}" => InnerKind::Stmt,
                                    _ => self.typ(tag),
                                };
                                (qterm.squash(), kind)
                            }
                            _ => return Err(unsupported_shape(&qterm)),
                        }
                    } else {
                        (qterm, InnerKind::File)
                    }
                }
                "{}" => (qterm, InnerKind::Expr),
                _ => return Err(unsupported_shape(&qterm)),
            },
            _ => return Err(unsupported_shape(&qterm)),
        })
    }

    fn arity(&self, tag: &str) -> Arity {
        match tag {
            "block" | "source_file" => Arity::Variadic,
            _ => Arity::Unknown,
        }
    }

    fn typ(&self, tag: &str) -> InnerKind {
        if tag == "source_file" {
            InnerKind::File
        } else if tag.ends_with("statement")
            || tag.ends_with("item")
            || tag.ends_with("declaration")
        {
            InnerKind::Stmt
        } else {
            InnerKind::Expr
        }
    }

    fn hashbang(&self) -> Option<&'static str> {
        Some("#!/usr/bin/env rust-script")
    }
}

/// Diagnostic for a Rust fragment whose tree-sitter parse shape the provider
/// doesn't know how to unwrap. Replaces the `unimplemented!` panics that used
/// to crash the expander on unusual-but-valid Rust (issue #11); the parse's
/// s-expression is included so the gap can be reported and reproduced.
fn unsupported_shape(qterm: &QTerm) -> miette::Report {
    miette!(
        "Quilt can't place this Rust fragment — unsupported parse shape: {}.\n\
         This is a gap in Quilt's Rust support; please report it along with the \
         fragment that triggered it.",
        qterm.sexp()
    )
}

pub type RustLanguage = TSLanguage<RustProvider>;
pub type DynRustLanguage = DynTSLanguage<RustProvider>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qterm::tb;

    /// A parse shape the provider can't place (here, a root tag that is neither
    /// `source_file` nor the `{}` hole) now surfaces a diagnostic instead of
    /// panicking via `unimplemented!` (issue #11).
    #[test]
    fn unsupported_shape_returns_err_not_panic() {
        let provider = RustProvider::default();
        let err = provider
            .unwrap(tb("not_a_real_node_kind").build(), None)
            .expect_err("an unrecognised parse shape should be an error");
        assert!(
            err.to_string().contains("unsupported parse shape"),
            "diagnostic should name the unsupported shape, got: {err}"
        );
    }

    /// A well-formed single-node `source_file` still unwraps successfully.
    #[test]
    fn source_file_single_node_unwraps_ok() {
        let provider = RustProvider::default();
        let qterm = tb("source_file").c(&tb("expression_statement").b()).build();
        assert!(provider.unwrap(qterm, None).is_ok());
    }
}
