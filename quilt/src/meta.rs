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

    /// The ground-tuple tag that introduces a pattern-let (e.g. Rust's
    /// `let_declaration`), or `None` if this meta-language has no pattern
    /// matching. A ground tuple with this tag whose binding position holds a
    /// quote is expanded as `let ↖pattern↗ = value;` (see
    /// `Expander::expand_pattern_let` and `crate::qmatch`).
    fn pattern_tag(&self) -> Option<&'static str> {
        None
    }

    /// Code for a pattern metavariable: the expression spliced where `↙name↘`
    /// sits inside a pattern quote (e.g. `mvar("name")` for Rust).
    fn pattern_var(&self, name: &str) -> Result<Arc<QTerm>> {
        let _ = name;
        Err(miette!(
            "this meta-language does not support pattern matching"
        ))
    }

    /// The two terms a pattern-let rewrites to: the destructuring binder that
    /// replaces the pattern quote (e.g. `[a, b]`) and the matching call that
    /// replaces the initializer (e.g. `qmatch_n(&<pattern>, &<value>)`).
    fn pattern_let(
        &self,
        names: &[Box<str>],
        pattern: &Arc<QTerm>,
        value: &Arc<QTerm>,
    ) -> Result<(Arc<QTerm>, Arc<QTerm>)> {
        let _ = (names, pattern, value);
        Err(miette!(
            "this meta-language does not support pattern matching"
        ))
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
    /// The spelling `↓` expands to when reducing with meta-language `target`
    /// (e.g. `py↓` inside a Rust meta-program invokes Python evaluation).
    /// The homogeneous case is `target` == `""` (no annotation); the default
    /// ignores `target`, preserving existing behavior for metas without
    /// heterogeneous support.
    #[inline]
    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        let _ = target;
        Ok(REDUCE)
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

    fn wrap_child(&self, qterm: Arc<QTerm>, okind: OuterKind) -> Result<Arc<QTerm>> {
        (**self).wrap_child(qterm, okind)
    }

    fn pattern_tag(&self) -> Option<&'static str> {
        (**self).pattern_tag()
    }

    fn pattern_var(&self, name: &str) -> Result<Arc<QTerm>> {
        (**self).pattern_var(name)
    }

    fn pattern_let(
        &self,
        names: &[Box<str>],
        pattern: &Arc<QTerm>,
        value: &Arc<QTerm>,
    ) -> Result<(Arc<QTerm>, Arc<QTerm>)> {
        (**self).pattern_let(names, pattern, value)
    }

    fn lift_str(&self, target: &str) -> Result<&'static str> {
        (**self).lift_str(target)
    }

    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        (**self).reduce_str(target)
    }

    fn emit_str(&self) -> &'static str {
        (**self).emit_str()
    }

    fn type_str(&self) -> &'static str {
        (**self).type_str()
    }

    fn name_str(&self) -> &'static str {
        (**self).name_str()
    }
}
