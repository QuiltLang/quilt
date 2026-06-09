use miette::Result;

use super::ops::{build_quote_code, build_tuple_code, build_unquote_code, build_variadic_block};
use crate::lang::Arity;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

#[derive(Default)]
pub struct PythonMetaLanguage;

impl MetaLanguage for PythonMetaLanguage {
    fn expand_quote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_quote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_unquote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_tuple(
        &self,
        _lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity,
    ) -> Result<Arc<QTerm>> {
        Ok(if arity == Arity::Variadic {
            build_variadic_block(tag, cmds, qterms)
        } else {
            build_tuple_code(tag, cmds, qterms)
        })
    }

    // No heterogeneous lifting from Python yet: `target` is ignored and `↑`
    // always lifts into Python.
    fn lift_str(&self, _target: &str) -> Result<&'static str> {
        Ok("qlift()")
    }

    fn reduce_str(&self) -> &'static str {
        "reduce()"
    }

    fn name_str(&self) -> &'static str {
        "name"
    }

    fn type_str(&self) -> &'static str {
        "QTerm"
    }

    fn emit_str(&self) -> &'static str {
        "emit(b_)"
    }
}
