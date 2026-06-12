use crate::{
    lang::{Arity, InnerKind},
    qterm::QTerm,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
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

    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        // dbg!(&qterm);
        #[allow(clippy::single_match)]
        match &qterm {
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
                            _ => unimplemented!("{}", qterm.sexp()),
                        }
                    } else {
                        (qterm, InnerKind::File)
                    }
                }
                "{}" => (qterm, InnerKind::Expr),
                _ => unimplemented!("{}", qterm.sexp()),
            },
            _ => unimplemented!("{}", qterm.sexp()),
        }
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

pub type RustLanguage = TSLanguage<RustProvider>;
pub type DynRustLanguage = DynTSLanguage<RustProvider>;
