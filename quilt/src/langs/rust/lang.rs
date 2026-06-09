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

    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        // dbg!(&qterm);
        #[allow(clippy::single_match)]
        match &qterm {
            QTerm::Tuple { tag, terms, .. } => match &**tag {
                "source_file" => {
                    if terms.len() == 1 {
                        let q0 = &terms[0];
                        match &**q0 {
                            QTerm::Tuple { tag, .. } => {
                                if tag.ends_with("statement")
                                    || tag.ends_with("item")
                                    || tag.ends_with("declaration")
                                    || **tag == *"{}"
                                {
                                    (qterm.squash(), InnerKind::Stmt)
                                } else {
                                    (qterm.squash(), InnerKind::Expr)
                                }
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

    fn hashbang(&self) -> Option<&'static str> {
        Some("#!/usr/bin/env rust-script")
    }
}

pub type RustLanguage = TSLanguage<RustProvider>;
pub type DynRustLanguage = DynTSLanguage<RustProvider>;
