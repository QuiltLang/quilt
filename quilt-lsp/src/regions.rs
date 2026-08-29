//! Parsing `.quilt` source into a position-aware structure.
//!
//! `quilt`'s `Node` tree carries source ranges on its brackets but not on the
//! text between them, and it drops Quilt's own comments entirely — so it is not
//! enough on its own to map positions or to translate a `⟨//⟩` into the host
//! language's `//`. [`quilt::node::scan`] is: it hands back every token with its
//! byte range, comments included, and the tokens tile the source exactly.
//!
//! Before issue #254 this module re-walked a `tree_sitter_quilt` CST for the
//! same information. The scanner replaced it, and it is the *same* scanner
//! `Node::parse` runs — so the server and the compiler can no longer disagree
//! about where a bracket is, which two parsers eventually would have.
//!
//! From those tokens this builds:
//!
//! * a [`Region`] tree (ground vs `↖↗` quote vs `↙↘` unquote), used by later
//!   phases to project each language into its own virtual document, and
//! * a list of syntax errors for diagnostics.

use quilt::node::{scan, Token, TokenKind};
use std::ops::Range;

/// Byte length of an arrow glyph (`↖↗↙↘↑↓`). They are all 3 bytes in UTF-8.
pub(crate) const ARROW_LEN: usize = "↖".len();

/// Scan `.quilt` source into tokens. Never fails: malformed input still yields
/// tokens for everything it could read, which is what keeps the server useful
/// on a half-typed buffer (see [`errors`]).
pub fn tokens(text: &str) -> Vec<Token> {
    scan(text).0
}

/// A syntax error discovered in the quilt structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub range: Range<usize>,
    pub message: String,
}

/// Every quilt-level syntax error in `text`, in source order.
///
/// The messages come from the parser itself rather than being reconstructed
/// here from node kinds — which is how `missing right_quote` used to leak into
/// a user-facing diagnostic.
pub fn errors(text: &str) -> Vec<SyntaxError> {
    scan(text)
        .1
        .into_iter()
        .map(|e| SyntaxError {
            range: e.span,
            message: e.message.into_string(),
        })
        .collect()
}

/// What language family a region's body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// The host program (stage 0) and anything textually outside brackets.
    Ground,
    /// Body of a `↖…↗` quote (raises quasi-quote stage).
    Quote,
    /// Body of a `↙…↘` unquote (lowers stage).
    Unquote,
}

/// Tracks the current language across nested brackets, mirroring the lang
/// `Zipper` in quilt's `multi.rs`: `stack` is the trail of enclosing languages
/// (top = current, like `list`), `pending` the defaults for successively
/// deeper un-annotated quotes (like `anti`) — seeded from the filename's
/// extension chain, and re-fed by [`Self::unquote`] so a quote re-entered from
/// inside a splice gets its language back.
#[derive(Debug, Clone, Default)]
pub(crate) struct LangZipper {
    stack: Vec<String>,
    pending: Vec<String>,
}

impl LangZipper {
    /// Seed from a filename extension chain, ground language first
    /// (see [`crate::adapters::lang_chain`]).
    pub(crate) fn from_chain(chain: &[&str]) -> Self {
        let mut z = Self::default();
        if let Some((ground, defaults)) = chain.split_first() {
            z.stack.push((*ground).to_string());
            z.pending
                .extend(defaults.iter().rev().map(|s| (*s).to_string()));
        }
        z
    }

    /// The language of the current region, if known.
    pub(crate) fn current(&self) -> Option<&str> {
        self.stack.last().map(String::as_str)
    }

    /// Enter a `↖…↗` quote: an annotation selects the language explicitly (and
    /// resets the pending defaults, like `Zipper::cons`); otherwise take the
    /// next pending default (`Zipper::back`), falling back to the current
    /// language.
    pub(crate) fn quote(&self, anno: &str) -> Self {
        let mut z = self.clone();
        if anno.is_empty() {
            let next = z.pending.pop().or_else(|| z.stack.last().cloned());
            z.stack.extend(next);
        } else {
            z.pending.clear();
            z.stack.push(anno.to_string());
        }
        z
    }

    /// Enter a `↙…↘` unquote: drop back to the enclosing language; the one we
    /// leave becomes the next quote default again (`Zipper::tail`).
    pub(crate) fn unquote(&self) -> Self {
        let mut z = self.clone();
        if let Some(cur) = z.stack.pop() {
            z.pending.push(cur);
        }
        z
    }
}

/// The annotation on an opening token, e.g. `wgsl` in `wgsl↖…↗`: the token's
/// text is `<anno>↖` (or `<anno>↙`), so strip the arrow. Empty for a plain
/// bracket.
pub(crate) fn open_anno<'t>(text: &'t str, open: &Range<usize>) -> &'t str {
    text.get(open.start..open.end - ARROW_LEN).unwrap_or("")
}

/// A contiguous span of one language at one quasi-quote stage.
#[derive(Debug, Clone)]
pub struct Region {
    pub kind: RegionKind,
    /// Resolved language key for this region's body, e.g. `"rs"`. `None` when it
    /// can't be inferred (no annotation and no known enclosing language).
    pub lang: Option<String>,
    /// The bracket annotation, e.g. `wgsl` in `wgsl↖…↗`. Empty for plain `↖…↗`.
    pub anno: String,
    /// Byte range of the body *between* the brackets (excludes the bracket
    /// tokens themselves). For the root this is the whole document.
    pub body: Range<usize>,
    /// Quasi-quote depth; ground is 0, each enclosing quote +1, unquote -1.
    pub stage: i32,
    /// Nested quote/unquote regions directly inside this one.
    pub children: Vec<Region>,
}

/// Build the region tree for a document. `chain` is the language-extension
/// chain from the filename, ground language first (see
/// [`crate::adapters::lang_chain`]).
///
/// A bracket left unclosed — which, in an editor, is most of them most of the
/// time — still gets a region, running to the end of the file. That is what
/// keeps a fragment highlighted while its `↗` is still being typed.
pub fn regions(text: &str, tokens: &[Token], chain: &[&str]) -> Region {
    let zipper = LangZipper::from_chain(chain);
    // Regions under construction, outermost first; `open[0]` is the whole file.
    let mut open = vec![Region {
        kind: RegionKind::Ground,
        lang: zipper.current().map(str::to_string),
        anno: String::new(),
        body: 0..text.len(),
        stage: 0,
        children: Vec::new(),
    }];
    let mut zippers = vec![zipper];

    for token in tokens {
        match token.kind {
            TokenKind::OpenQuote | TokenKind::OpenUnquote => {
                let quote = token.kind == TokenKind::OpenQuote;
                let anno = open_anno(text, &token.span);
                // Mirror the lang zipper in `multi.rs`: a quote takes its
                // annotation, the next chain default, or the enclosing
                // language; an unquote drops back one level (its annotation,
                // like in quilt proper, does not select a language).
                let inner = zippers.last().expect("the ground zipper is never popped");
                let inner = if quote {
                    inner.quote(anno)
                } else {
                    inner.unquote()
                };
                let stage = open
                    .last()
                    .expect("the ground region is never popped")
                    .stage
                    + if quote { 1 } else { -1 };
                open.push(Region {
                    kind: if quote {
                        RegionKind::Quote
                    } else {
                        RegionKind::Unquote
                    },
                    lang: inner.current().map(str::to_string),
                    anno: anno.to_string(),
                    // Sealed by the matching closer, or at end of file.
                    body: token.span.end..text.len(),
                    stage,
                    children: Vec::new(),
                });
                zippers.push(inner);
            }
            // `scan` only emits a closer for a bracket it has open, so there
            // is always something to pop but the ground region.
            TokenKind::CloseQuote | TokenKind::CloseUnquote if open.len() > 1 => {
                let mut done = open.pop().expect("checked by the guard");
                zippers.pop();
                done.body.end = token.span.start;
                open.last_mut()
                    .expect("the ground region")
                    .children
                    .push(done);
            }
            _ => {}
        }
    }
    while open.len() > 1 {
        let done = open.pop().expect("checked");
        open.last_mut()
            .expect("the ground region")
            .children
            .push(done);
    }
    open.pop().expect("the ground region")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions_of(text: &str, chain: &[&str]) -> Region {
        regions(text, &tokens(text), chain)
    }

    #[test]
    fn clean_file_has_no_errors() {
        let text = "fn main() {\n    let x = ↖1 + 2↗;\n}\n";
        assert!(errors(text).is_empty());
    }

    #[test]
    fn unclosed_quote_is_an_error() {
        let text = "let x = ↖1 + 2;\n";
        let errs = errors(text);
        assert!(
            !errs.is_empty(),
            "expected a syntax error for the unclosed ↖"
        );
    }

    #[test]
    fn unclosed_quote_error_is_localized_to_bracket() {
        // The error must be a small squiggle on the `↖` glyph (3 bytes), not a
        // huge span covering the rest of the file.
        let text = "fn main() {\n    let x = ↖1 + 2;\n}\n";
        let errs = errors(text);
        assert_eq!(errs.len(), 1, "exactly one error: {errs:?}");
        let span = errs[0].range.end - errs[0].range.start;
        assert!(
            span <= ARROW_LEN,
            "error span {span} bytes should be ≤ ARROW_LEN ({ARROW_LEN}): {:?}",
            errs[0]
        );
    }

    #[test]
    fn unclosed_unquote_error_is_localized_to_bracket() {
        let text = "↖ x ↙y + z\n↗\n";
        let errs = errors(text);
        assert!(!errs.is_empty(), "expected an error for the unclosed ↙");
        // Every error should be small.
        for e in &errs {
            let span = e.range.end - e.range.start;
            assert!(
                span <= ARROW_LEN,
                "error span {span} bytes should be ≤ ARROW_LEN: {e:?}"
            );
        }
    }

    #[test]
    fn missing_glyph_message_uses_arrow_symbol() {
        // MISSING node messages must show the actual glyph, not the ts node kind.
        let text = "let x = ↖1 + 2;\n";
        let errs = errors(text);
        for e in &errs {
            assert!(
                !e.message.contains("right_quote"),
                "message should not expose ts node kind: {e:?}"
            );
        }
    }

    #[test]
    fn extracts_quote_region() {
        let text = "let x = ↖1 + 2↗;\n";
        let root = regions_of(text, &["rs"]);
        assert_eq!(root.kind, RegionKind::Ground);
        assert_eq!(root.children.len(), 1);
        let q = &root.children[0];
        assert_eq!(q.kind, RegionKind::Quote);
        assert_eq!(q.lang.as_deref(), Some("rs")); // inherited from ground
        assert_eq!(q.stage, 1);
        assert_eq!(&text[q.body.clone()], "1 + 2");
    }

    #[test]
    fn annotation_overrides_language() {
        let text = "x = wgsl↖1.0↗;\n";
        let root = regions_of(text, &["rs"]);
        let q = &root.children[0];
        assert_eq!(q.anno, "wgsl");
        assert_eq!(q.lang.as_deref(), Some("wgsl"));
    }

    #[test]
    fn nested_quote_and_unquote_stages() {
        // ground -> quote(+1) -> unquote(0)
        let text = "↖ ↙x↘ ↗\n";
        let root = regions_of(text, &["rs"]);
        let q = &root.children[0];
        assert_eq!(q.kind, RegionKind::Quote);
        assert_eq!(q.stage, 1);
        assert_eq!(q.children.len(), 1);
        let u = &q.children[0];
        assert_eq!(u.kind, RegionKind::Unquote);
        assert_eq!(u.stage, 0);
    }

    #[test]
    fn chain_defaults_unannotated_quote() {
        // From `shaders.wgsl.rs.quilt`: an un-annotated quote defaults to
        // WGSL, a splice inside it drops back to Rust, and a quote inside the
        // splice is WGSL again (the zipper re-feeds the default).
        let text = "let x = ↖a ↙f(↖b↗)↘ c↗;\n";
        let root = regions_of(text, &["rs", "wgsl"]);
        assert_eq!(root.lang.as_deref(), Some("rs"));
        let q = &root.children[0];
        assert_eq!(q.lang.as_deref(), Some("wgsl"));
        let u = &q.children[0];
        assert_eq!(u.kind, RegionKind::Unquote);
        assert_eq!(u.lang.as_deref(), Some("rs"));
        let q2 = &u.children[0];
        assert_eq!(q2.kind, RegionKind::Quote);
        assert_eq!(q2.lang.as_deref(), Some("wgsl"));
    }

    /// The point of scanning with recovery rather than parsing strictly: a
    /// buffer mid-keystroke still has regions.
    ///
    /// `quilt`'s `Node::parse` returns `Err` and nothing else for this input.
    /// If the server used it, typing `↖` would blank every region, projection
    /// and highlight in the file until the matching `↗` was typed — so the
    /// quote region has to exist, and to run to the end of the file, while the
    /// bracket is still open.
    #[test]
    fn an_unclosed_quote_still_has_a_region() {
        let text = "let x = wgsl↖1.0\nlet y = 2;\n";
        assert!(
            quilt::node::Node::parse(text).is_err(),
            "the strict parser rejects this"
        );

        let root = regions_of(text, &["rs"]);
        assert_eq!(root.children.len(), 1, "{root:?}");
        let q = &root.children[0];
        assert_eq!(q.kind, RegionKind::Quote);
        assert_eq!(q.lang.as_deref(), Some("wgsl"));
        assert_eq!(q.stage, 1);
        assert_eq!(&text[q.body.clone()], "1.0\nlet y = 2;\n");
        assert_eq!(
            errors(text).len(),
            1,
            "one diagnostic, not none and not many"
        );
    }

    /// A stray closer is one diagnostic, and does not take the rest of the file
    /// down with it: the well-formed quote after it still gets a region.
    #[test]
    fn recovery_is_local_to_the_bad_bracket() {
        let text = "a ↗ b py↖1↗ c\n";
        let errs = errors(text);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].range, 2..5, "points at the stray `↗`");

        let root = regions_of(text, &["rs"]);
        assert_eq!(root.children.len(), 1, "{root:?}");
        assert_eq!(root.children[0].anno, "py");
    }

    /// A closer of the wrong kind closes the bracket anyway, so one typo is one
    /// diagnostic rather than one per bracket for the rest of the file.
    #[test]
    fn a_mismatched_closer_still_closes() {
        let text = "↖x↘ ↖y↗\n";
        assert_eq!(errors(text).len(), 1, "{:?}", errors(text));
        let root = regions_of(text, &["rs"]);
        assert_eq!(root.children.len(), 2, "{root:?}");
    }

    #[test]
    fn annotation_resets_chain_defaults() {
        // An annotated quote pins its language; an un-annotated quote nested
        // inside it inherits the annotation, not the chain default (mirrors
        // `Zipper::cons` clearing `anti`).
        let text = "let x = py↖a ↖b↗ c↗;\n";
        let root = regions_of(text, &["rs", "wgsl"]);
        let q = &root.children[0];
        assert_eq!(q.lang.as_deref(), Some("py"));
        let q2 = &q.children[0];
        assert_eq!(q2.lang.as_deref(), Some("py"));
    }
}
