//! The Lean meta-language: Lean as a *host* that drives generation.
//!
//! A `.lean.quilt` file is a Lean 4 metaprogram. Where the Rust/Python hosts
//! emit builder calls into a `QTerm` runtime, this host has **no runtime
//! library**: it represents generated code as plain Lean strings (see
//! [`super::ops`]). A quote `↖ … ↗` expands to an interpolated string literal
//! `s!" … "` and a host unquote `↙x↘` to Lean's own `{x}` interpolation, so
//! evaluating the expanded program (e.g. `#eval`, or `lean file.lean`) yields
//! the generated code as a `String`. See issue #132 for the rationale and the
//! path to a real `QTerm` runtime.
//!
//! Because everything is a string, a Lean host can generate *any* target
//! language — `lean↖…↗`, `rs↖…↗`, `wgsl↖…↗` all reconstruct the same way.

use miette::Result;

use super::ops::{build_quote_str, build_str_code, build_unquote_str};
use crate::lang::Arity;
use crate::meta::OuterKind;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

#[derive(Default)]
pub struct LeanMetaLanguage;

impl MetaLanguage for LeanMetaLanguage {
    fn expand_tuple(
        &self,
        _lang1: &str,
        _tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        _arity: Arity,
    ) -> Result<Arc<QTerm>> {
        // Strings have no builder/accumulator, so variadic and fixed nodes
        // reconstruct identically: literal text plus spliced children.
        Ok(build_str_code(cmds, qterms))
    }

    fn expand_quote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_quote_str(lang2, qterm))
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_unquote_str(lang2, qterm))
    }

    /// Identity: the string model has no `b_` accumulator to emit/splice into,
    /// so a child is woven into its parent purely by `expand_tuple`'s
    /// concatenation. Emit/splice in *ground* loops is therefore unsupported —
    /// build sequences functionally instead (`List.map`, `String.intercalate`).
    fn wrap_child(&self, qterm: Arc<QTerm>, _okind: OuterKind) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// `↑` renders a host value as text for interpolation. There is no runtime
    /// `QTerm`, so the only spelling is Lean's own `toString`; cross-language
    /// lifts have no spelling.
    ///
    /// (Inside a splice Lean already applies `ToString`, so `↑` is only needed
    /// where a value is used as a `String` in host position.)
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "" | "lean" => Ok("toString"),
            _ => miette::bail!("lean can't lift into {target:?}: only homogeneous `toString`"),
        }
    }

    /// No spelling: `←` needs a `b_` accumulator to emit into, which the string
    /// model doesn't have (see [`Self::wrap_child`]). Fail here rather than let
    /// the `__EMIT__` placeholder leak into the generated Lean.
    fn emit_str(&self) -> Result<&'static str> {
        miette::bail!(
            "lean can't emit `←`: the string-based meta has no `b_` accumulator to emit into — \
             build sequences functionally instead (`List.map` + `String.intercalate`)"
        )
    }

    /// No spelling: `↓` compiles a term and deserializes the result back, which
    /// needs the `QTerm` runtime this host doesn't have. Generation-time
    /// evaluation is ordinary Lean — run it outside the quote and splice the
    /// value with `↙…↘`.
    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        miette::bail!(
            "lean can't reduce `{target}↓`: the string-based meta has no `QTerm` runtime to \
             evaluate a fragment — compute the value in ordinary Lean and splice it with `↙…↘`"
        )
    }

    /// A generated fragment *is* a `String` here, so that is the type `⟨T⟩`
    /// names — the annotation `examples/lean_host.lean.quilt` writes by hand.
    fn type_str(&self) -> Result<&'static str> {
        Ok("String")
    }

    /// In the string model a name is its own text, so `⟨N⟩` is Lean's identity.
    fn name_str(&self) -> Result<&'static str> {
        Ok("id")
    }
}
