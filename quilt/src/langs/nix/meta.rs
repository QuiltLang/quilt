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
    /// concatenation. A child that is *itself* a sequence goes through the `←`
    /// operator instead of this hook — see [`Self::emit_str`].
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

    /// `←` appends a *sequence* of fragments into the surrounding container.
    ///
    /// The imperative reading of emit — append one term to a `b_` accumulator,
    /// once per loop iteration — has no counterpart in Nix: it is a pure
    /// expression language with no statements and nothing to mutate. Its
    /// functional counterpart does: build the list of fragments with `map` and
    /// hand the whole list to `←`, which joins it into one fragment. So
    ///
    /// ```nix
    /// nix↖[ ↙← (map (n: nix↖"↙n↘"↗) names)↘ ]↗
    /// ```
    ///
    /// is the string model's counterpart of a runtime-backed host's `↖…↗.←`
    /// inside a ground loop. The spelling is `builtins`-only (this host ships
    /// no runtime library) and is applied prefix, by juxtaposition, like
    /// `↑`/`toString`: `← xs`, not `xs.←`.
    ///
    /// The separator is a newline because it is the only one that is *correct*
    /// for every container: Nix is whitespace-insensitive, so `[ a\nb ]` and
    /// `{ inherit a\nb; }` are the lists the source meant, and a generated
    /// line-oriented target (bash, python, …) gets one statement per line
    /// rather than a run-on. Where a different separator is wanted (a
    /// comma-separated `formals`, say), call `builtins.concatStringsSep`
    /// directly — `←` is exactly its newline partial application.
    fn emit_str(&self) -> Result<&'static str> {
        Ok("(builtins.concatStringsSep \"\\n\")")
    }

    /// No spelling: `↓` compiles a term and deserializes the result back, which
    /// needs the `QTerm` runtime this host doesn't have. Generation-time
    /// evaluation is ordinary Nix — compute the value in a `let` and splice it
    /// with `↙…↘`.
    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        miette::bail!(
            "nix can't reduce `{target}↓`: the string-based meta has no `QTerm` runtime to \
             evaluate a fragment — compute the value in ordinary Nix and splice it with `↙…↘`"
        )
    }

    /// No spelling: Nix is untyped and has no annotation syntax, so there is
    /// nowhere for `⟨T⟩` to go. (Lean, the other string host, answers `String`.)
    fn type_str(&self) -> Result<&'static str> {
        miette::bail!(
            "nix has no type for `⟨T⟩`: Nix is untyped, and a generated fragment is just a \
             string — drop the annotation"
        )
    }

    /// In the string model a name is its own text, so `⟨N⟩` is the identity —
    /// which for a Nix string is what `toString` already does.
    fn name_str(&self) -> Result<&'static str> {
        Ok("toString")
    }
}
