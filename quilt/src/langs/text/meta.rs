use miette::Result;

use crate::lang::Arity;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

#[derive(Default)]
pub struct TextMetaLanguage;

impl MetaLanguage for TextMetaLanguage {
    fn expand_quote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        _lang2: &str,
        _qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        todo!()
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        _lang2: &str,
        _qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        todo!()
    }

    fn expand_tuple(
        &self,
        _lang1: &str,
        _tag: &str,
        _qterms: &[Arc<QTerm>],
        _cmds: &[CmdOrHole],
        _arity: Arity,
    ) -> Result<Arc<QTerm>> {
        todo!()
    }
}
