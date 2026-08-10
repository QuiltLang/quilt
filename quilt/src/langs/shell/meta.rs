//! The shell meta-languages: bash and zsh as *hosts* that drive generation
//! (issue #151).
//!
//! A `.bash.quilt` / `.zsh.quilt` file is a shell metaprogram. Where the
//! Rust/Python hosts emit builder calls into a `QTerm` runtime, these hosts have
//! **no runtime library**: they represent generated code as ordinary
//! double-quoted shell words (see [`super::ops`]). A quote `↖ … ↗` expands to
//! `" … "` and a host unquote `↙x↘` is spliced into it verbatim, because a shell
//! expansion carries its own `$`. So running the expanded script — which is what
//! `quilt run foo.bash.quilt` does, and what makes
//! [`hashbang`][crate::lang::Language::hashbang] reachable at last — prints the
//! generated code.
//!
//! Because everything is a string, a shell host can generate *any* target
//! language: `bash↖…↗`, `rs↖…↗`, `wgsl↖…↗` all reconstruct the same way.
//!
//! # What a shell host cannot spell
//!
//! Quilt's five operator glyphs each expand to a fixed spelling that is spliced
//! into the ground source and re-parsed, applied *prefix* to what follows. A
//! shell has no prefix-applied word operators at all — juxtaposition is command
//! invocation, and a command is not a word — so four of the five have no
//! spelling here and say so instead of leaking a placeholder into a generated
//! script:
//!
//! * `↑` (lift) and `⟨N⟩` (name) would be the identity: a shell value is
//!   already text, and a name is its own text. There is nothing to convert with
//!   and nothing to convert to, so both point the caller at `↙…↘`.
//! * `←` (emit) needs a `b_` accumulator, which the string model does not have —
//!   the same limit Lean's host has (#132), for the same reason. Nix answers it
//!   with `builtins.concatStringsSep`, applied prefix; the shell's join is
//!   `printf '%s\n' "${xs[@]}"`, which takes its operand *inside* a
//!   substitution rather than after a prefix, so it cannot be a spelling. Write
//!   it in the unquote instead.
//! * `↓` (reduce) needs the `QTerm` runtime no string host ships.
//!
//! `⟨T⟩` is the fifth, and it fails too: the shell is untyped.

use miette::Result;
use std::marker::PhantomData;

use super::ops::{build_quote_str, build_str_code, build_unquote_str};
use crate::lang::Arity;
use crate::meta::OuterKind;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

/// Which shell a [`ShellMetaLanguage`] speaks.
///
/// The two dialects reconstruct fragments identically — zsh's grammar is a fork
/// of bash's and their double-quoting rules are the same — so the dialect is
/// carried only so that a refusal names the host the user actually wrote, the
/// way [`crate::langs::lean::meta`] has to accept both `lean` and `lean4`.
pub trait ShellDialect: Default + Send + Sync + 'static {
    /// The canonical registry name, as it appears in an error message.
    const NAME: &'static str;
}

/// Marker: the bash dialect.
#[derive(Default)]
pub struct BashDialect;

impl ShellDialect for BashDialect {
    const NAME: &'static str = "bash";
}

/// Marker: the zsh dialect.
#[derive(Default)]
pub struct ZshDialect;

impl ShellDialect for ZshDialect {
    const NAME: &'static str = "zsh";
}

/**************************************************************/

/// A shell host: string-based generation, parameterised by dialect.
#[derive(Default)]
pub struct ShellMetaLanguage<D: ShellDialect>(PhantomData<D>);

/// The bash host.
pub type BashMetaLanguage = ShellMetaLanguage<BashDialect>;

/// The zsh host.
pub type ZshMetaLanguage = ShellMetaLanguage<ZshDialect>;

impl<D: ShellDialect> MetaLanguage for ShellMetaLanguage<D> {
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
    /// so a child is woven into its parent purely by [`Self::expand_tuple`]'s
    /// concatenation.
    fn wrap_child(&self, qterm: Arc<QTerm>, _okind: OuterKind) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// No spelling. `↑` renders a host value as text, and every shell value
    /// *is* text — so the honest spelling is the empty one, which would make
    /// `↑` an invisible no-op in the generated script rather than an operator.
    /// Refuse instead, and name the thing that does work.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        let host = D::NAME;
        miette::bail!(
            "{host} can't lift `{target}↑`: a shell value is already text, so there is no \
             conversion to spell — splice it with `↙…↘`, which interpolates it as written"
        )
    }

    /// No spelling: `←` needs a `b_` accumulator to append to, which the string
    /// model doesn't have (see [`Self::wrap_child`]). Unlike Nix's
    /// `builtins.concatStringsSep`, the shell's join cannot stand in for one:
    /// an operator spelling is applied *prefix* to what follows it, and
    /// `printf` takes its operand inside a substitution.
    fn emit_str(&self) -> Result<&'static str> {
        let host = D::NAME;
        miette::bail!(
            "{host} can't emit `←`: the string-based meta has no `b_` accumulator to emit into, \
             and a shell join takes its operand inside a substitution rather than after a prefix \
             — collect the fragments in an array and splice the join itself, \
             `↙$(printf '%s\\n' \"${{frags[@]}}\")↘`"
        )
    }

    /// No spelling: `↓` compiles a term and deserializes the result back, which
    /// needs the `QTerm` runtime this host doesn't have. Generation-time
    /// evaluation is ordinary shell — compute the value in a variable and
    /// splice it with `↙…↘`.
    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        let host = D::NAME;
        miette::bail!(
            "{host} can't reduce `{target}↓`: the string-based meta has no `QTerm` runtime to \
             evaluate a fragment — compute the value in ordinary shell and splice it with `↙…↘`"
        )
    }

    /// No spelling: the shell is untyped — every value is a word — so there is
    /// nothing for `⟨T⟩` to name and nowhere to write it.
    fn type_str(&self) -> Result<&'static str> {
        let host = D::NAME;
        miette::bail!(
            "{host} has no type for `⟨T⟩`: the shell is untyped and a generated fragment is just \
             a word — drop the annotation"
        )
    }

    /// No spelling, for [`Self::lift_str`]'s reason: a name in the string model
    /// is its own text, and the shell has no identity function to say so with.
    fn name_str(&self) -> Result<&'static str> {
        let host = D::NAME;
        miette::bail!(
            "{host} has no spelling for `⟨N⟩`: a name is already its own text here — splice it \
             with `↙…↘`"
        )
    }
}
