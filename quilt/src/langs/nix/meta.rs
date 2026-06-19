//! The Nix meta-language: Nix as a *host* that drives generation.
//!
//! A `.nix.quilt` file is a Nix metaprogram. Where the Rust/Python hosts emit
//! builder calls into a `QTerm` runtime, this host has **no runtime library**:
//! it represents generated code as plain Nix strings (see [`super::ops`]). A
//! quote `↖ … ↗` expands to a Nix string literal and a host unquote `↙x↘` to
//! Nix's own `${x}` antiquotation, so evaluating the expanded program (e.g.
//! with `nix eval`) yields the generated code as a string.
//!
//! Because everything is a string, a Nix host can generate *any* target
//! language — `bash↖…↗`, `nix↖…↗`, … all reconstruct the same way.

use miette::Result;

use super::ops::{build_quote_str, build_str_code, build_unquote_str};
use crate::lang::Arity;
use crate::meta::OuterKind;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

#[derive(Default)]
pub struct NixMetaLanguage;

impl MetaLanguage for NixMetaLanguage {
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
    /// build sequences functionally instead (`map`, `concatStringsSep`).
    fn wrap_child(&self, qterm: Arc<QTerm>, _okind: OuterKind) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// `↑` renders a host value as text for interpolation. There is no runtime
    /// `QTerm`, so the only spelling is Nix's own `toString`; cross-language
    /// lifts have no spelling.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "" | "nix" => Ok("toString"),
            _ => miette::bail!("nix can't lift into {target:?}: only homogeneous `toString`"),
        }
    }
}
