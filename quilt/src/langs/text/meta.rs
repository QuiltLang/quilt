//! The text meta-language: the *identity* host.
//!
//! Text holds the object-level code as unparsed lines — same tags, same
//! `cmds`, same text — and refuses every operator with a diagnostic naming a
//! real host. The mechanism is [`HoldMetaLanguage`](crate::langs::hold), shared
//! with HTML (`langs::html::meta`); this file is the text dialect of it, and
//! the tests that show what "holds the object level code as unparsed lines"
//! *means*.
//!
//! `langs/omni.rs` leaves `text` out of its `metas` section, so `Omni` never
//! reaches this. The type is `pub`, though, so a consumer can wire it into a
//! `Single` or a `DictMulti` by hand — which used to hit three `todo!()`s and
//! abort with no explanation (#174, finding J). That is the class of panic
//! issue #11 set out to remove: `langs/text/lang.rs` beside it was converted at
//! the time and this file was missed, while both `conformance/spec/text.toml`
//! and `docs/wiki/concrete-languages.md` already documented the expansion as
//! identity.

pub use crate::langs::hold::{HoldMetaLanguage, Holds};

/**************************************************************/

/// Plain text: the original identity host.
pub struct Text;

impl Holds for Text {
    const NAME: &'static str = "text";
    const ANNO: &'static str = "txt";
}

pub type TextMetaLanguage = HoldMetaLanguage<Text>;

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Arity;
    use crate::meta::MetaLanguage as _;
    use crate::prelude::*;
    use crate::qterm::QTerm;
    use crate::term::{CmdOrHole, STerm as _, Term as _};

    /// The `cmds` the parser gives a quote: annotation, opening glyph, the
    /// body's hole, closing glyph (see `Multi::build_nodes`).
    fn bracket_cmds(anno: &str, open: &str, close: &str) -> Vec<CmdOrHole> {
        vec![cmd(write(anno)), cmd(write(open)), HOLE, cmd(write(close))]
    }

    /// A tuple comes back as itself — same tag, same structure, same text. This
    /// is the whole contract: the object-level code is held, not translated.
    #[test]
    fn holds_a_tuple_verbatim() -> Result<()> {
        let meta = TextMetaLanguage::default();
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
        let meta = TextMetaLanguage::default();
        let cmds = bracket_cmds("rs", "↖", "↗");
        let held = meta.expand_quote("text", "text", 1, "rs", &leaf("text", "x + 1"), &cmds)?;

        assert_eq!(held.coparse(), "rs↖x + 1↗");
        assert_eq!(held.tag(), crate::qterm::QTermTag::tuple("text"));
        Ok(())
    }

    /// Same for an unquote that does not reach ground.
    #[test]
    fn holds_a_nested_unquote_verbatim() -> Result<()> {
        let meta = TextMetaLanguage::default();
        let cmds = bracket_cmds("", "↙", "↘");
        let held = meta.expand_unquote("text", "text", 1, "text", &leaf("text", "v"), &cmds)?;

        assert_eq!(held.coparse(), "↙v↘");
        Ok(())
    }

    /// A hand-built node whose holes and children disagree is an error, not a
    /// silently truncated fragment.
    #[test]
    fn mismatched_holes_are_an_error() {
        let meta = TextMetaLanguage::default();
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
        let meta = TextMetaLanguage::default();
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
    use crate::prelude::*;
    use crate::term::STerm as _;

    /// A `DictMulti` with text as both the object language and the host.
    fn text_host() -> DictMulti {
        let mut multi = DictMulti::default();
        multi.add_lang("text", bx(DynTextLanguage));
        multi.add_meta("text", bx(TextMetaLanguage::default()));
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
