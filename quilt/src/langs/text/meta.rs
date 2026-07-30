//! The text meta-language: the *identity* host.
//!
//! Every other meta translates a quoted fragment into host code that rebuilds
//! it — Rust and Python emit builder calls into a `QTerm` runtime, Nix and Lean
//! emit string literals in the host's own syntax. Text has neither: no
//! expressions to translate *into*, and no runtime to translate *for*. So it
//! does the one thing a text host can do — it **holds the object-level code as
//! unparsed lines**: same tags, same `cmds`, same text. Expanding a text-hosted
//! quote yields the quoted term itself, and `coparse` gives the fragment back
//! verbatim.
//!
//! That makes the three `expand_*` methods a structural identity. The operator
//! spellings (`↑ ↓ ← ⟨T⟩ ⟨N⟩`) are the other half of the same fact: each is a
//! *host* operator that has to expand into a host expression, and text has
//! none — so each returns an error naming a real host, rather than leaking a
//! `__LIFT__`-style placeholder into the output.
//!
//! `langs/omni.rs` leaves `text` out of its `metas` section, so `Omni` never
//! reaches this. The type is `pub`, though, so a consumer can wire it into a
//! `Single` or a `DictMulti` by hand — which used to hit three `todo!()`s and
//! abort with no explanation (#174, finding J). That is the class of panic
//! issue #11 set out to remove: `langs/text/lang.rs` beside it was converted at
//! the time and this file was missed, while both `conformance/spec/text.toml`
//! and `docs/wiki/concrete-languages.md` already documented the expansion as
//! identity.

use miette::{bail, Result};

use crate::lang::Arity;
use crate::meta::OuterKind;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

/// Rebuild a node as itself: same `tag`, same `cmds`, `terms` back in its
/// holes. This is the whole of the text meta — the object-level code is kept
/// rather than translated, so the `cmds` that serialized the fragment before
/// expansion serialize it after.
///
/// Every node the expander hands over comes from a parse, where the hole count
/// and the child count agree by construction. A hand-built term need not, and
/// silently dropping children would be a worse failure than saying so.
fn hold(tag: &str, terms: &[Arc<QTerm>], cmds: &[CmdOrHole]) -> Result<Arc<QTerm>> {
    let holes = cmds.iter().filter(|c| matches!(c, CmdOrHole::Hole)).count();
    if holes != terms.len() {
        bail!(
            "text can't hold a {tag:?} node: {holes} hole(s) for {} child term(s)",
            terms.len()
        );
    }
    Ok(tuple(tag, terms, cmds))
}

#[derive(Default)]
pub struct TextMetaLanguage;

impl MetaLanguage for TextMetaLanguage {
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
        hold(tag, std::slice::from_ref(qterm), cmds)
    }

    /// An unquote that does not reach ground, held verbatim like a nested
    /// quote. (One that *does* reach ground never arrives here: the expander
    /// splices the ground term in its place, which for a text host is the
    /// ground text itself.)
    fn expand_unquote(
        &self,
        _lang1: &str,
        tag: &str,
        _i: Index,
        _lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        hold(tag, std::slice::from_ref(qterm), cmds)
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
        hold(tag, qterms, cmds)
    }

    /// Identity: a child is already woven into its parent by the `cmds` that
    /// [`hold`] keeps, so there is nothing to wrap it in. Emit and splice have
    /// no accumulator to reach (see [`Self::emit_str`]).
    fn wrap_child(&self, qterm: Arc<QTerm>, _okind: OuterKind) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// No spelling: `↑` turns a host *value* into a term, and text has no
    /// values — a fragment of text is already the text it stands for.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        bail!(
            "text can't lift `↑` into {target:?}: it holds code rather than building it, so \
             there is no expression for a lift to be — quote text from a real host \
             (`txt↖ … ↗` in a `.rs.quilt` or `.py.quilt` file) and lift there"
        )
    }

    /// No spelling: `↓` evaluates a fragment at generation time, which needs a
    /// runtime to evaluate it *with*. Nothing in a text file runs.
    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        bail!(
            "text can't reduce `{target}↓`: it holds code rather than running it, so there is \
             no runtime to evaluate a fragment with — reduce in a real host that quotes \
             `txt↖ … ↗`"
        )
    }

    /// No spelling: `←` appends into a `b_` accumulator, which only a meta that
    /// *builds* its output has. Text holds its output whole.
    fn emit_str(&self) -> Result<&'static str> {
        bail!(
            "text can't emit `←`: it holds code rather than building it, so there is no `b_` \
             accumulator to emit into — emit in a real host that quotes `txt↖ … ↗`"
        )
    }

    /// No spelling: `⟨T⟩` names the type of a generated fragment in the host's
    /// own syntax, and plain text has no type syntax to name it in.
    fn type_str(&self) -> Result<&'static str> {
        bail!("text has no type for `⟨T⟩`: plain text has no type syntax — drop the annotation")
    }

    /// No spelling: `⟨N⟩` takes a string to an identifier term. In text a name
    /// is already its own text, so the operator *is* the identity — and there
    /// is no identity function to write it as.
    fn name_str(&self) -> Result<&'static str> {
        bail!(
            "text has no spelling for `⟨N⟩`: a name in text is already its own text, so the \
             operator would be the identity — drop it"
        )
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{STerm as _, Term as _};

    /// The `cmds` the parser gives a quote: annotation, opening glyph, the
    /// body's hole, closing glyph (see `Multi::build_nodes`).
    fn bracket_cmds(anno: &str, open: &str, close: &str) -> Vec<CmdOrHole> {
        vec![cmd(write(anno)), cmd(write(open)), HOLE, cmd(write(close))]
    }

    /// A tuple comes back as itself — same tag, same structure, same text. This
    /// is the whole contract: the object-level code is held, not translated.
    #[test]
    fn holds_a_tuple_verbatim() -> Result<()> {
        let meta = TextMetaLanguage;
        let node = tb("text")
            .w("a=")
            .c(&leaf("text", "1"))
            .w(" b=")
            .c(&leaf("text", "2"))
            .b();
        let QTerm::Tuple { tag, terms, cmds } = &*node else {
            unreachable!("built a tuple")
        };

        let held = meta.expand_tuple("text", tag, terms, cmds, Arity::Variadic)?;
        assert_eq!(held, node, "expansion must be the identity");
        assert_eq!(held.coparse(), "a=1 b=2");
        Ok(())
    }

    /// A nested quote keeps its glyphs *and* its annotation, so the later stage
    /// that owns it sees exactly what the author wrote.
    #[test]
    fn holds_a_nested_quote_verbatim() -> Result<()> {
        let meta = TextMetaLanguage;
        let cmds = bracket_cmds("rs", "↖", "↗");
        let held = meta.expand_quote("text", "text", 1, "rs", &leaf("text", "x + 1"), &cmds)?;

        assert_eq!(held.coparse(), "rs↖x + 1↗");
        assert_eq!(held.tag(), crate::qterm::QTermTag::tuple("text"));
        Ok(())
    }

    /// Same for an unquote that does not reach ground.
    #[test]
    fn holds_a_nested_unquote_verbatim() -> Result<()> {
        let meta = TextMetaLanguage;
        let cmds = bracket_cmds("", "↙", "↘");
        let held = meta.expand_unquote("text", "text", 1, "text", &leaf("text", "v"), &cmds)?;

        assert_eq!(held.coparse(), "↙v↘");
        Ok(())
    }

    /// A hand-built node whose holes and children disagree is an error, not a
    /// silently truncated fragment.
    #[test]
    fn mismatched_holes_are_an_error() {
        let meta = TextMetaLanguage;
        let err = meta
            .expand_tuple(
                "text",
                "text",
                &[leaf("text", "1")],
                &[cmd(write("no holes here"))],
                Arity::Variadic,
            )
            .expect_err("one child, no hole");
        assert!(err.to_string().contains("0 hole(s) for 1"), "{err}");
    }

    /// Every operator spelling is an `Err` naming the alternative — never a
    /// panic, and never a `__LIFT__`-style placeholder leaking into output.
    /// The conformance battery (#144) treats a panic as a hard failure for
    /// every language including the unsupported cases, which is what surfaced
    /// the same bug in `langs/text/lang.rs`.
    #[test]
    fn operators_error_with_an_alternative() {
        let meta = TextMetaLanguage;
        let refusals = [
            ("↑", meta.lift_str("text")),
            ("↓", meta.reduce_str("")),
            ("←", meta.emit_str()),
            ("⟨T⟩", meta.type_str()),
            ("⟨N⟩", meta.name_str()),
        ];
        for (glyph, refusal) in refusals {
            let err = refusal
                .expect_err("text has no host expressions")
                .to_string();
            assert!(err.starts_with("text "), "{glyph}: {err}");
            assert!(err.contains(glyph), "should name the operator: {err}");
        }
    }
}

/**************************************************************/

/// End-to-end: text wired up as a host by hand, the way a consumer outside
/// `Omni` reaches this meta. These are the tests that show what "holds the
/// object level code as unparsed lines" *means*.
#[cfg(all(test, feature = "parse"))]
mod host_tests {
    use super::*;
    use crate::langs::text::lang::DynTextLanguage;
    use crate::multi::DictMulti;
    use crate::term::STerm as _;

    /// A `DictMulti` with text as both the object language and the host.
    fn text_host() -> DictMulti {
        let mut multi = DictMulti::default();
        multi.add_lang("text", bx(DynTextLanguage));
        multi.add_meta("text", bx(TextMetaLanguage));
        multi.add_alias("txt", "text");
        multi
    }

    fn expand(src: &str) -> Result<String> {
        let mut multi = text_host();
        let parsed = multi.parse_chain(&["text"], src)?;
        Ok(multi.expand_lang("text", &parsed)?.coparse())
    }

    /// Ground text is its own output, and a quote at ground contributes its
    /// body — the brackets are staging, not content.
    #[test]
    fn a_quote_contributes_its_text() -> Result<()> {
        assert_eq!(expand("Hello ↖world↗!")?, "Hello world!");
        Ok(())
    }

    /// Multiple lines survive as lines: the `cmds` that laid the fragment out
    /// are the ones held.
    #[test]
    fn lines_stay_lines() -> Result<()> {
        assert_eq!(
            expand("↖alpha\nbeta\n\ndelta↗")?,
            "alpha\nbeta\n\ndelta",
            "held text keeps its line structure"
        );
        Ok(())
    }

    /// An unquote reaching ground splices the ground term, which in a text host
    /// is ground text — so `↖…↙x↘…↗` reads straight through.
    #[test]
    fn a_ground_unquote_splices_its_text() -> Result<()> {
        assert_eq!(expand("↖a ↙b↘ c↗")?, "a b c");
        Ok(())
    }

    /// A quote *inside* a quote belongs to the next stage, so its glyphs are
    /// still there afterwards — this is the `expand_quote` path.
    #[test]
    fn an_inner_quote_survives_expansion() -> Result<()> {
        assert_eq!(expand("↖outer ↖inner↗ tail↗")?, "outer ↖inner↗ tail");
        Ok(())
    }

    /// A ground operator is refused with a diagnostic rather than expanding to
    /// a placeholder. `↑` is spelled while the fragment is being built, so this
    /// covers the spelling accessors through the real parse path.
    #[test]
    fn a_ground_operator_is_refused() {
        let err = expand("value: ↑\n")
            .expect_err("text has no lift")
            .to_string();
        assert!(err.contains("text can't lift"), "{err}");
    }
}
