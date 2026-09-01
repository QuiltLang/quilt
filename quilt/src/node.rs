mod parse;
pub use parse::{scan, ParseError, Token, TokenKind};

use crate::strcmd::PrefixWriter;
use crate::term::Term;
use crate::{prelude::*, term::STerm};
use std::{borrow::Borrow, fmt::Debug, iter::empty, sync::Arc};

/**************************************************************/

// The glyph set and the `\` escape are facts about the surface syntax with no
// parser in them, and the runtime-only build needs them to render a term back
// as `.quilt` source (#223) — so they live in `crate::glyphs`, which is not
// behind the `parse` feature, and are re-exported here, where every caller
// already looks for them.
pub use crate::glyphs::{escape, unescape, ARROW_LEN, ESCAPE_LEN, GLYPHS};

// The spellings the writer below shares with the parser, so the two cannot
// drift apart.
use parse::{NAME, Q_BLOCK_EMPTY, TYPE};

/**************************************************************/

/// Raw Quilt AST with unparsed string content
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum Node {
    Content(Box<str>),
    NewLine,
    Quote {
        anno: Box<str>,
        nodes: Box<[Arc<Node>]>,
        /// Byte range of the whole `anno↖…↗` in the parsed source.
        span: Span,
    },
    Unquote {
        anno: Box<str>,
        nodes: Box<[Arc<Node>]>,
        /// Byte range of the whole `anno↙…↘` in the parsed source.
        span: Span,
    },
    Lift,
    Reduce {
        /// The meta-language annotation on `↓`, e.g. `"py"` for `py↓`.
        /// Empty string means homogeneous (use the current meta-language).
        anno: Box<str>,
    },
    Emit,
    Type,
    Name,
    /// Plain `// …` line comment: passes through verbatim to output.
    /// The `/.*/` in the grammar consumes the rest of the line as raw text,
    /// so Quilt special chars inside are not interpreted.
    PlainLineComment(Box<str>),
    /// Plain `/* … */` block comment: passes through verbatim to output.
    PlainBlockComment(Box<str>),
}

impl Node {
    /// Parse a source string into a list of [`Node`]s.
    ///
    /// Hand-written recursive descent over Quilt's surface syntax, straight to
    /// the term structure (issue #254). [`scan`] is the recovering half of the
    /// same parser, for callers that want every diagnostic and a tree anyway.
    ///
    /// Malformed bracket structure is a diagnostic, not a panic: an unbalanced
    /// `↖`/`↙` or a stray `↗`/`↘` returns an `Err` carrying a labelled span, so
    /// `quilt check` — whose whole job is reporting diagnostics — keeps
    /// reporting instead of aborting.
    pub fn parse(code: &str) -> Result<Box<[Self]>> {
        parse::parse(code)
    }

    pub fn coparse(nodes: &[Self]) -> Box<str> {
        let mut buf = std::io::BufWriter::new(Vec::new());
        let mut writer = PrefixWriter::new(&mut buf);
        Self::write_seq(nodes, &mut writer);
        let bytes = buf.into_inner().unwrap();
        String::from_utf8(bytes).unwrap().into()
    }

    /// Write a run of siblings, with the separators the surface syntax needs
    /// for the result to parse back (issue #256).
    ///
    /// Writing each node's own spelling one after another is not enough:
    /// Quilt's comment tokens are the one place where a node's text can reach
    /// past its own end, and two nodes that are perfectly well-formed apart can
    /// lex as one token together.
    ///
    /// * A `// …` comment runs to the end of its line, closing brackets
    ///   included, so *anything* written after one on the same line vanishes
    ///   into it. A tree can hold that shape without being malformed —
    ///   `//c⟨/*⟩x⟨*/⟩↖q↗` parses fine, and the Quilt comment takes the newline
    ///   with it — and the printed form then had a `↗` with no opener. Only a
    ///   newline ends a line comment, so that is what goes in.
    /// * A `/` ending one node and a `/` or `*` starting the next spell a
    ///   comment introducer that neither node contains. The gap between them
    ///   was a Quilt comment the parser dropped (`a/⟨/*⟩z⟨*/⟩/b` is two content
    ///   nodes), so an empty [`Q_BLOCK_EMPTY`] comment puts a gap back — the
    ///   one separator Quilt has that leaves no text behind.
    ///
    /// Both separators are idempotent: re-parsing what they produce yields a
    /// sequence this writes the same way again.
    fn write_seq<N: Borrow<Self>, W: std::io::Write>(
        nodes: &[N],
        writer: &mut PrefixWriter<'_, W>,
    ) {
        // What the text written so far ends in, as the lexer will read it back:
        // a `\` still waiting to pair with the next character; a `/` that would
        // begin a token, so a `/` or `*` after it spells a comment; and a
        // `// …` comment still holding its line open.
        //
        // The pairing is why `bare_slash` is not just "the last byte is `/`":
        // content is lexed in `\X` units, so the `/` ending `a\/` is the second
        // half of one and cannot start anything — `a\/// x` prints itself
        // unchanged, and separating it would be a bug, not a fix.
        //
        // A trailing `*` before a `/` is likewise not a seam. It would spell
        // `*/`, which matters only if some earlier `/*` is still open — and one
        // cannot be, because two nodes are only ever adjacent where a
        // `⟨/*⟩…⟨*/⟩` was dropped, and that spells a `*/` in the source, so the
        // earlier `/*` was a `/* … */` comment there and the node before the
        // seam is that comment rather than content. Separating anyway would be
        // worse than useless: the separator carries a `*/` of its own.
        let mut pending_escape = false;
        let mut bare_slash = false;
        let mut in_line_comment = false;
        for n in nodes {
            let n = n.borrow();
            // A node that writes nothing leaves the seam exactly where it was.
            let Some(first) = n.first_byte() else {
                continue;
            };
            if in_line_comment && first != b'\n' {
                writer.newline();
            } else if bare_slash && matches!(first, b'/' | b'*') {
                writer.write(Q_BLOCK_EMPTY);
            }
            n.write(writer);
            match n {
                Node::Content(s) => {
                    for c in escape(s).chars() {
                        if pending_escape {
                            pending_escape = false;
                            bare_slash = false;
                        } else if c == '\\' {
                            pending_escape = true;
                            bare_slash = false;
                        } else {
                            bare_slash = c == '/';
                        }
                    }
                    in_line_comment = false;
                }
                // Every other node is a whole token: it leaves nothing dangling
                // for the next one to join onto.
                Node::PlainLineComment(_) => {
                    pending_escape = false;
                    bare_slash = false;
                    in_line_comment = true;
                }
                _ => {
                    pending_escape = false;
                    bare_slash = false;
                    in_line_comment = false;
                }
            }
        }
    }

    /// The first byte [`STerm::write`] emits for this node, or `None` if it
    /// emits nothing. Only `\n`, `/` and `*` are load-bearing in
    /// [`Self::write_seq`], but the whole answer is cheaper to state than three
    /// special cases are to keep in step with the writer.
    fn first_byte(&self) -> Option<u8> {
        let first = |s: &str| s.as_bytes().first().copied();
        match self {
            // `escape` only ever *prepends* a `\` to a glyph.
            Node::Content(s) => match s.chars().next() {
                Some(c) if GLYPHS.contains(&c) => Some(b'\\'),
                _ => first(s),
            },
            Node::NewLine => Some(b'\n'),
            Node::Quote { anno, .. } => first(anno).or_else(|| first("↖")),
            Node::Unquote { anno, .. } => first(anno).or_else(|| first("↙")),
            Node::Reduce { anno } => first(anno).or_else(|| first("↓")),
            Node::Lift => first("↑"),
            Node::Emit => first("←"),
            Node::Type => first(TYPE),
            Node::Name => first(NAME),
            Node::PlainLineComment(s) | Node::PlainBlockComment(s) => first(s),
        }
    }
}

/**************************************************************/

pub enum NodeTag {
    Content,
    NewLine,
    Quote,
    Unquote,
    Lift,
    Reduce,
    Emit,
    Name,
    Type,
    PlainLineComment,
    PlainBlockComment,
}

impl Term for Node {
    type Tag = NodeTag;

    fn tag(&self) -> Self::Tag {
        match self {
            Node::Content(_) => NodeTag::Content,
            Node::NewLine => NodeTag::NewLine,
            Node::Quote { .. } => NodeTag::Quote,
            Node::Unquote { .. } => NodeTag::Unquote,
            Node::Lift => NodeTag::Lift,
            Node::Reduce { .. } => NodeTag::Reduce,
            Node::Emit => NodeTag::Emit,
            Node::Type => NodeTag::Type,
            Node::Name => NodeTag::Name,
            Node::PlainLineComment(_) => NodeTag::PlainLineComment,
            Node::PlainBlockComment(_) => NodeTag::PlainBlockComment,
        }
    }

    fn children(&self) -> impl Iterator<Item = &Self> {
        let ret: Box<dyn Iterator<Item = _>> = match self {
            Node::Quote { nodes, .. } | Node::Unquote { nodes, .. } => {
                bx(nodes.iter().map(|x| x.as_ref()))
            }
            _ => bx(empty()),
        };
        ret
    }

    fn len(&self) -> usize {
        match self {
            Node::Quote { nodes, .. } | Node::Unquote { nodes, .. } => nodes.len(),
            _ => 0,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Node::Quote { nodes, .. } | Node::Unquote { nodes, .. } => nodes.is_empty(),
            _ => true,
        }
    }
}

impl STerm for Node {
    fn write<W: std::io::Write>(&self, writer: &mut crate::strcmd::PrefixWriter<'_, W>) {
        match self {
            Node::Content(s) => writer.write(&escape(s)),
            Node::NewLine => writer.newline(),
            Node::Quote { anno, nodes, .. } => {
                writer.write(anno);
                writer.write("↖");
                Node::write_seq(nodes, writer);
                writer.write("↗");
            }
            Node::Unquote { anno, nodes, .. } => {
                writer.write(anno);
                writer.write("↙");
                Node::write_seq(nodes, writer);
                writer.write("↘");
            }
            Node::Lift => writer.write("↑"),
            Node::Reduce { anno } => {
                writer.write(anno);
                writer.write("↓");
            }
            Node::Emit => writer.write("←"),
            Node::Type => writer.write(TYPE),
            Node::Name => writer.write(NAME),
            Node::PlainLineComment(s) | Node::PlainBlockComment(s) => writer.write(s),
        }
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node() -> Result<()> {
        let source_code = indoc::indoc! {"
            Some Python: py↖1+2↗
            ↑↓
        "};
        let nodes = Node::parse(source_code)?;
        dbg!(&nodes);
        let source_code2 = &*Node::coparse(&nodes);
        assert_eq!(source_code, source_code2);
        Ok(())
    }

    #[test]
    fn plain_comments_coparse() -> Result<()> {
        let source_code = "// line comment\n/* block comment */\ncode\n";
        let nodes = Node::parse(source_code)?;
        assert!(matches!(&nodes[0], Node::PlainLineComment(s) if &**s == "// line comment"));
        assert!(matches!(&nodes[2], Node::PlainBlockComment(s) if &**s == "/* block comment */"));
        assert_eq!(&*Node::coparse(&nodes), source_code);
        Ok(())
    }

    /// What `coparse` prints must parse, and print the same thing again.
    ///
    /// The law `fuzz/fuzz_targets/escape_roundtrip.rs` states, against the
    /// inputs that broke it (issue #256). Every case here is *valid* Quilt
    /// whose parse leaves a node sequence that cannot simply be concatenated
    /// back: Quilt's own comments are dropped from the tree, so they leave
    /// behind a seam that the surrounding text closes over when it is re-read.
    #[test]
    fn coparse_output_parses_and_is_a_fixpoint() {
        let cases = [
            // The crash libFuzzer found. `⟨/*⟩…⟨*/⟩` takes the newline in front
            // of it, so the `//n//` line comment reached the `↖` and swallowed
            // it, leaving `↗` with no opener.
            "ea//n//\n⟨/*⟩↖K//n*⟨*/⟩↖\n\0\0\0\0\0\0\0x↗;fft/",
            // The same shape, minimised, at ground level and inside a bracket.
            "//c\n⟨/*⟩z⟨*/⟩↖b↗",
            "↖//c\n⟨/*⟩z⟨*/⟩↖b↗↗",
            // A `//` that exists in neither node: the dropped comment was the
            // only thing keeping the two slashes apart.
            "a/⟨/*⟩z⟨*/⟩/↖b↗",
            // And the `/*` and `*/` spellings of the same seam.
            "a/⟨/*⟩z⟨*/⟩*↖b↗",
            "a/⟨/*⟩z⟨*/⟩*↖b*/↗",
            "a/⟨/*⟩z⟨*/⟩/* c */↖b↗",
            // Seams that must *not* grow a separator.
            "a/b/c",
            "// line\n/* block */\ncode\n",
            "↖//c↗",
        ];
        for src in cases {
            let nodes = Node::parse(src).unwrap_or_else(|e| panic!("{src:?} does not parse: {e}"));
            let printed = Node::coparse(&nodes);
            let reparsed = Node::parse(&printed).unwrap_or_else(|e| {
                panic!("{src:?} printed {printed:?}, which does not parse: {e}")
            });
            assert_eq!(
                &*Node::coparse(&reparsed),
                &*printed,
                "{src:?} printed {printed:?}, which does not print itself"
            );
        }
    }

    /// The separators are only written where they are needed. Both of these
    /// have a case in them that a separator would *break*, and both came out
    /// of the sweep in `tests/parser_corpus.rs` rather than out of thinking
    /// about it:
    ///
    /// * `a\/// x` is content `a\/` and a comment `// x`. The `/` ending the
    ///   content is the second half of a `\/` unit and cannot start a token,
    ///   so the two are already adjacent in valid source.
    /// * `↑/*//` ends its content with `*`, not `/`. Separating there would put
    ///   the separator's own `*/` where it closes the still-open `/*`, in the
    ///   middle of a `⟨*/⟩` — leaving a stray `⟩` and no parse at all.
    #[test]
    fn separators_are_not_written_where_the_source_already_parses() -> Result<()> {
        for src in [
            "// line\n/* block */\ncode\n",
            "a/b/c ↖x↗ /* c */ // done\n",
            "↖//c↗",
            "↖a/↗/b",
            "a\\/// x",
            "↑/*//",
        ] {
            assert_eq!(&*Node::coparse(&Node::parse(src)?), src, "{src:?}");
        }
        Ok(())
    }

    /// An empty `⟨/*⟩⟨*/⟩` really is inert — that is the whole reason
    /// `write_seq` can use it as a separator.
    #[test]
    fn the_empty_block_comment_writes_nothing() -> Result<()> {
        assert!(Node::parse(Q_BLOCK_EMPTY)?.is_empty());
        assert_eq!(&*Node::coparse(&Node::parse("a⟨/*⟩⟨*/⟩b")?), "ab");
        Ok(())
    }

    #[test]
    fn annotated_reduce() -> Result<()> {
        let source_code = "↓ py↓ rs↓";
        let nodes = Node::parse(source_code)?;
        assert_eq!(nodes.len(), 5); // ↓, space, py↓, space, rs↓
        assert!(matches!(&nodes[0], Node::Reduce { anno } if anno.is_empty()));
        assert!(matches!(&nodes[2], Node::Reduce { anno } if &**anno == "py"));
        assert!(matches!(&nodes[4], Node::Reduce { anno } if &**anno == "rs"));
        let roundtrip = &*Node::coparse(&nodes);
        assert_eq!(roundtrip, source_code);
        Ok(())
    }

    /// An annotation may carry digits after its first letter, in all three
    /// annotated openers — so the registered alias `lean4` can be *written*
    /// (issue #222). It could not be, under the old `[a-z]*↖`: `lean4↖…↗` was
    /// the content `lean4` followed by an un-annotated quote, and the mistake
    /// surfaced somewhere else entirely.
    #[test]
    fn annotations_take_digits_after_the_first_letter() -> Result<()> {
        let source_code = "lean4↖x↗ lean4↙y↘ lean4↓";
        let nodes = Node::parse(source_code)?;
        assert_eq!(nodes.len(), 5); // quote, space, unquote, space, reduce
        assert!(matches!(&nodes[0], Node::Quote { anno, .. } if &**anno == "lean4"));
        assert!(matches!(&nodes[2], Node::Unquote { anno, .. } if &**anno == "lean4"));
        assert!(matches!(&nodes[4], Node::Reduce { anno } if &**anno == "lean4"));
        assert_eq!(&*Node::coparse(&nodes), source_code);
        Ok(())
    }

    /// The other half of that rule, and the reason it is not simply
    /// `[a-z0-9]*`: a *number* abutting the glyph stays content. `42↖…↗` is the
    /// literal `42` and a bare quote — which defaults to the host language —
    /// not a quote of some language named "42".
    #[test]
    fn a_bare_number_is_not_an_annotation() -> Result<()> {
        let source_code = "x = 42↖1↗";
        let nodes = Node::parse(source_code)?;
        assert!(matches!(&nodes[0], Node::Content(s) if &**s == "x = 42"));
        assert!(matches!(&nodes[1], Node::Quote { anno, .. } if anno.is_empty()));
        assert_eq!(&*Node::coparse(&nodes), source_code);
        Ok(())
    }

    /// Malformed bracket structure is an `Err`, never a panic. Each of these
    /// used to abort the process via an `unreachable!` in the tree-sitter
    /// parser — including under `quilt check`, which exists to report
    /// diagnostics and which lost the whole run (exit 101) to a single stray
    /// glyph.
    ///
    /// Only *surface* malformation belongs here. A top-level `↙x↘` is
    /// well-formed syntax — an unquote with no enclosing quote is a depth error,
    /// caught later by the expander with its own diagnostic
    /// (`Expander::expand` → `unquote_depth_error`).
    #[test]
    fn malformed_brackets_are_errors_not_panics() {
        for src in [
            "fn main() { let x = ↖1 + 2; }", // unclosed quote
            "fn main() { let x = 1 ↗ 2; }",  // stray quote close
            "fn main() { ↘ }",               // stray unquote close
            "↖↙↗",                           // closer/opener interleaved
            "py↖",                           // annotated opener, nothing after
        ] {
            let err = Node::parse(src)
                .expect_err("malformed Quilt source should be an Err, not Ok or a panic");
            assert!(
                err.to_string().contains("malformed Quilt syntax"),
                "diagnostic should name the syntax error, got: {err}"
            );
        }
    }

    /// A well-formed nesting of every bracket form still parses, so the
    /// `has_error` gate isn't rejecting valid input.
    #[test]
    fn well_formed_brackets_still_parse() -> Result<()> {
        for src in ["↖↗", "↖\n↗", "py↖1+2↗", "rs↖ wgsl↖ ↙x↘ ↗ ↗", "↖↙↖↙x↘↗↘↗"]
        {
            let nodes = Node::parse(src).map_err(|e| e.wrap_err(format!("parsing {src:?}")))?;
            assert_eq!(&*Node::coparse(&nodes), src, "{src:?} did not round-trip");
        }
        Ok(())
    }

    /// The error span points *inside* the offending source rather than at the
    /// whole file, so the rendered diagnostic underlines something useful.
    #[test]
    fn syntax_error_span_is_narrow() {
        let src = "aaaaaaaaaa↗bbbbbbbbbb";
        let err = Node::parse(src).expect_err("stray `↗` should not parse");
        let labels: Vec<_> = err.labels().into_iter().flatten().collect();
        assert_eq!(labels.len(), 1, "expected exactly one label: {err}");
        let len = labels[0].len();
        assert!(
            len < src.len(),
            "label should be narrower than the whole source, got {len} of {}",
            src.len()
        );
    }

    /// Every glyph in [`GLYPHS`] is escapable: `\<glyph>` parses to plain
    /// content holding the bare glyph, and `coparse` puts the `\` back, so the
    /// source round-trips. Before issue #141 only `↑` and `↓` were re-escaped
    /// on output (and `←` was not escapable at all), so `\⟨` and friends were
    /// lossy.
    #[test]
    fn every_glyph_round_trips() -> Result<()> {
        for g in GLYPHS {
            let glyph = g.to_string();
            let source_code = format!("a \\{g} b");
            let nodes = Node::parse(&source_code)?;
            assert!(
                nodes
                    .iter()
                    .any(|n| matches!(n, Node::Content(s) if **s == *glyph)),
                "`\\{g}` did not parse to content holding `{g}`: {nodes:?}"
            );
            assert_eq!(
                &*Node::coparse(&nodes),
                source_code,
                "`\\{g}` lost its escape"
            );
        }
        Ok(())
    }

    /// `escape` and `unescape` are inverses over the whole glyph set, and a
    /// backslash before a *non*-glyph (`_non_escape` in the grammar) is left
    /// alone in both directions.
    #[test]
    fn escape_unescape_are_inverses() {
        let raw = "x ← y ↑ z ⟨ w ↖ v";
        let escaped = "x \\← y \\↑ z \\⟨ w \\↖ v";
        assert_eq!(&*escape(raw), escaped);
        assert_eq!(&*unescape(escaped), raw);
        // `\n` here is a literal backslash-n, not a newline: not a glyph, so it
        // survives unescaping verbatim rather than losing its backslash.
        assert_eq!(&*unescape("a \\n b"), "a \\n b");
        assert_eq!(&*escape("no glyphs here"), "no glyphs here");
    }

    /// [`ARROW_LEN`] is one constant for the whole glyph set, and `quilt-lsp`'s
    /// `regions`/`code_actions` do byte arithmetic with it (`body.end +
    /// ARROW_LEN`), so *every* glyph must be that wide — not just the six arrows
    /// this test used to cover. Adding a glyph of another width to [`GLYPHS`]
    /// fails here rather than silently shifting LSP ranges.
    #[test]
    fn glyph_lengths_are_uniform() {
        for g in GLYPHS {
            assert_eq!(
                g.len_utf8(),
                ARROW_LEN,
                "glyph {g:?} is {} bytes, but ARROW_LEN is {ARROW_LEN}",
                g.len_utf8()
            );
        }
        assert_eq!(ESCAPE_LEN, 1, "the escape introducer is a one-byte `\\`");
    }
}
