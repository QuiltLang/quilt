//! The identity meta-language, shared by every host that *holds* code
//! rather than computing with it.
//!
//! Every other meta translates a quoted fragment into host code that rebuilds
//! it — Rust and Python emit builder calls into a `QTerm` runtime, Nix and Lean
//! emit string literals in the host's own syntax. A held language has neither:
//! no expressions to translate *into*, and no runtime to translate *for*. So
//! it does the one thing such a host can do — it **holds the object-level code
//! as unparsed lines**: same tags, same `cmds`, same text. Expanding a held
//! quote yields the quoted term itself, and `coparse` gives the fragment back
//! verbatim.
//!
//! That makes the three `expand_*` methods a structural identity. The operator
//! spellings (`↑ ↓ ← ⟨T⟩ ⟨N⟩`) are the other half of the same fact: each is a
//! *host* operator that has to expand into a host expression, and a held
//! language has none — so each returns an error naming a real host, rather
//! than leaking a `__LIFT__`-style placeholder into the output.
//!
//! Two languages are held: text (`langs::text::meta`, the original) and HTML
//! (`langs::html::meta`), whose notebooks run the held cells on machines
//! instead. [`Holds`] is the two-line dialect marker that puts the right
//! language name in the refusals. This module is not feature-gated, because
//! either language may be enabled without the other.

use std::marker::PhantomData;

use miette::{bail, Result};

use crate::lang::Arity;
use crate::meta::OuterKind;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

/// What distinguishes one identity host from another: only how it is named
/// in the diagnostics that refuse the operators.
pub trait Holds {
    /// The language name, as the refusals spell it (`text`, `html`).
    const NAME: &'static str;
    /// The quote annotation a real host would use to reach this language
    /// (`txt`, `html`) — the alternative every refusal names.
    const ANNO: &'static str;
}

/// The identity meta-language, for a language that holds code rather than
/// computing with it. See the module docs.
pub struct HoldMetaLanguage<H: Holds>(PhantomData<H>);

impl<H: Holds> Default for HoldMetaLanguage<H> {
    fn default() -> Self {
        HoldMetaLanguage(PhantomData)
    }
}

/**************************************************************/

/// Rebuild a node as itself: same `tag`, same `cmds`, `terms` back in its
/// holes. This is the whole of an identity meta — the object-level code is
/// kept rather than translated, so the `cmds` that serialized the fragment
/// before expansion serialize it after.
///
/// Every node the expander hands over comes from a parse, where the hole count
/// and the child count agree by construction. A hand-built term need not, and
/// silently dropping children would be a worse failure than saying so.
fn hold(name: &str, tag: &str, terms: &[Arc<QTerm>], cmds: &[CmdOrHole]) -> Result<Arc<QTerm>> {
    let holes = cmds.iter().filter(|c| matches!(c, CmdOrHole::Hole)).count();
    if holes != terms.len() {
        bail!(
            "{name} can't hold a {tag:?} node: {holes} hole(s) for {} child term(s)",
            terms.len()
        );
    }
    Ok(tuple(tag, terms, cmds))
}

impl<H: Holds> MetaLanguage for HoldMetaLanguage<H> {
    /// A nested quote (`lang↖…↗` at quote depth > 0) belongs to a later stage,
    /// so it stays as written. The quote's own `cmds` already hold its
    /// annotation and both glyphs around a single hole for the body, so holding
    /// them reproduces the source — including whether the author annotated the
    /// quote at all, which `lang2` alone would not say.
    fn expand_quote(
        &self,
        _lang1: &str,
        tag: &str,
        _i: Index,
        _lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        hold(H::NAME, tag, std::slice::from_ref(qterm), cmds)
    }

    /// An unquote that does not reach ground, held verbatim like a nested
    /// quote. (One that *does* reach ground never arrives here: the expander
    /// splices the ground term in its place, which for an identity host is
    /// the ground text itself.)
    fn expand_unquote(
        &self,
        _lang1: &str,
        tag: &str,
        _i: Index,
        _lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        hold(H::NAME, tag, std::slice::from_ref(qterm), cmds)
    }

    /// The quoted code itself. Variadic and fixed nodes are held identically —
    /// there is no builder whose shape would differ between them.
    fn expand_tuple(
        &self,
        _lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        _arity: Arity,
    ) -> Result<Arc<QTerm>> {
        hold(H::NAME, tag, qterms, cmds)
    }

    /// Identity: a child is already woven into its parent by the `cmds` that
    /// [`hold`] keeps, so there is nothing to wrap it in. Emit and splice have
    /// no accumulator to reach (see [`Self::emit_str`]).
    fn wrap_child(&self, qterm: Arc<QTerm>, _okind: OuterKind) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// No spelling: `↑` turns a host *value* into a term, and an identity host
    /// has no values — a fragment is already the text it stands for.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        bail!(
            "{} can't lift `↑` into {target:?}: it holds code rather than building it, so \
             there is no expression for a lift to be — quote {} from a real host \
             (`{}↖ … ↗` in a `.rs.quilt` or `.py.quilt` file) and lift there",
            H::NAME,
            H::NAME,
            H::ANNO
        )
    }

    /// No spelling: `↓` evaluates a fragment at generation time, which needs a
    /// runtime to evaluate it *with*. Nothing in a held file runs.
    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        bail!(
            "{} can't reduce `{target}↓`: it holds code rather than running it, so there is \
             no runtime to evaluate a fragment with — reduce in a real host that quotes \
             `{}↖ … ↗`",
            H::NAME,
            H::ANNO
        )
    }

    /// No spelling: `←` appends into a `b_` accumulator, which only a meta that
    /// *builds* its output has. An identity host holds its output whole.
    fn emit_str(&self) -> Result<&'static str> {
        bail!(
            "{} can't emit `←`: it holds code rather than building it, so there is no `b_` \
             accumulator to emit into — emit in a real host that quotes `{}↖ … ↗`",
            H::NAME,
            H::ANNO
        )
    }

    /// No spelling: `⟨T⟩` names the type of a generated fragment in the host's
    /// own syntax, and a held language has no type syntax to name it in.
    fn type_str(&self) -> Result<&'static str> {
        bail!(
            "{} has no type for `⟨T⟩`: it has no type syntax — drop the annotation",
            H::NAME
        )
    }

    /// No spelling: `⟨N⟩` takes a string to an identifier term. In a held
    /// language a name is already its own text, so the operator *is* the
    /// identity — and there is no identity function to write it as.
    fn name_str(&self) -> Result<&'static str> {
        bail!(
            "{} has no spelling for `⟨N⟩`: a name in {} is already its own text, so the \
             operator would be the identity — drop it",
            H::NAME,
            H::NAME
        )
    }
}
