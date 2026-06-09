use crate::lang::Arity;
use crate::prelude::*;
use crate::qterm::QTerm;
use crate::term::CmdOrHole;
use std::fmt::Debug;
use std::sync::Arc;

/**************************************************************/

pub const LIFT: &str = "__LIFT__";
pub const REDUCE: &str = "__REDUCE__";
pub const EMIT: &str = "__EMIT__";
pub const TYPE: &str = "__TYPE__";
pub const NAME: &str = "__NAME__";

#[derive(Debug, Clone, Copy, Default)]
pub enum OuterKind {
    #[default]
    None,
    Emit,
    Splice,
}

pub trait MetaLanguage {
    fn expand_quote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;
    fn expand_unquote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;
    fn expand_tuple(
        &self,
        lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity, // should we split expand_tuple based on variadic vs not?
    ) -> Result<Arc<QTerm>>;

    /// Wrap an expanded child of a tuple.
    /// * `qterm` - the child term to wrap
    /// * `okind` - an outer kind like splice/emit/none
    fn wrap_child(&self, qterm: Arc<QTerm>, _okind: OuterKind) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// The spelling `↑` expands to when lifting into the object language
    /// `target` (e.g. Rust lifting a value into a WGSL term). The homogeneous
    /// case is `target` == the meta-language's own language; the default
    /// ignores `target`, preserving that behavior for metas without
    /// heterogeneous support.
    #[inline]
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        let _ = target;
        Ok(LIFT)
    }
    // TODO: support heterogenous reduction: meta-lang A reducing lang B
    #[inline]
    fn reduce_str(&self) -> &'static str {
        REDUCE
    }
    #[inline]
    fn emit_str(&self) -> &'static str {
        EMIT
    }
    #[inline]
    fn type_str(&self) -> &'static str {
        TYPE
    }
    #[inline]
    fn name_str(&self) -> &'static str {
        NAME
    }
}

/**************************************************************/

impl MetaLanguage for Box<dyn MetaLanguage> {
    fn expand_quote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        (**self).expand_quote(lang1, tag, i, lang2, qterm, cmds)
    }

    fn expand_unquote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        (**self).expand_unquote(lang1, tag, i, lang2, qterm, cmds)
    }

    fn expand_tuple(
        &self,
        lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity, // should we split expand_tuple based on variadic vs not?
    ) -> Result<Arc<QTerm>> {
        (**self).expand_tuple(lang1, tag, qterms, cmds, arity)
    }

    fn lift_str(&self, target: &str) -> Result<&'static str> {
        (**self).lift_str(target)
    }
}
