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

    /// Where the pattern and value sit among the children of a
    /// [`pattern_tag`](Self::pattern_tag) tuple, as `(pattern_ix, value_ix)`.
    ///
    /// The expander used to hardcode this: it scanned for a child tagged
    /// literally `"="` and took the pattern from `terms[1]`. Both assumptions are
    /// Rust's, sitting in the language-agnostic core — so a host whose binding
    /// form uses `:=` or `<-` could not have pattern-lets even after supplying a
    /// `pattern_tag`, and `let mut ↖p↗ = v` (where `mutable_specifier` shifts the
    /// pattern off index 1) silently expanded to Rust that does not compile
    /// (issue #174, finding E3).
    ///
    /// `None` means this tuple is not a pattern-let, so the expander treats it as
    /// an ordinary ground tuple. Implementations will usually delegate to
    /// [`crate::qmatch::pattern_binding_at`] with their separator token.
    fn pattern_binding(&self, terms: &[Arc<QTerm>]) -> Option<(usize, usize)> {
        let _ = terms;
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
    /// The spelling `←` expands to. Like [`Self::lift_str`] this returns a
    /// `Result` because not every meta-language *has* an emit: a string-based
    /// meta (nix, lean) has no `b_` accumulator to emit into, and must fail
    /// loudly rather than leak the [`EMIT`] placeholder into generated code.
    #[inline]
    fn emit_str(&self) -> Result<&'static str> {
        Ok(EMIT)
    }
    /// The spelling `⟨T⟩` expands to: the type of a quilt term in this host's
    /// meta-code (`Arc<QTerm>` for Rust). `Result` because a host language
    /// without a way to write types has no spelling for it.
    #[inline]
    fn type_str(&self) -> Result<&'static str> {
        Ok(TYPE)
    }
    /// The spelling `⟨N⟩` expands to: the function taking a string to an
    /// identifier term (`name` for Rust). In a string-based meta a name is its
    /// own text, so this is the host's identity function.
    #[inline]
    fn name_str(&self) -> Result<&'static str> {
        Ok(NAME)
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

    fn pattern_binding(&self, terms: &[Arc<QTerm>]) -> Option<(usize, usize)> {
        (**self).pattern_binding(terms)
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

    fn emit_str(&self) -> Result<&'static str> {
        (**self).emit_str()
    }

    fn type_str(&self) -> Result<&'static str> {
        (**self).type_str()
    }

    fn name_str(&self) -> Result<&'static str> {
        (**self).name_str()
    }
}
