//! Quilt's surface parser: hand-written, one pass, straight to [`Node`].
//!
//! Quilt's outer syntax is a *lexical* language — glyph-delimited brackets over
//! otherwise-opaque text — so the whole grammar is a scanner plus one rule for
//! nesting. Running a generalised parser over it (issue #254) meant building a
//! CST, walking it, and converting every node into the [`Node`] we actually
//! wanted, with the byte ranges recovered from the tree on the way past. This
//! module skips both the tree and the walk.
//!
//! ## What it is faithful to
//!
//! `tree-sitter-quilt/grammar.js` remains the *specification* — it is what the
//! VS Code extension and `quilt-lsp`'s `regions` read, so it is not going
//! anywhere, and a second parser that quietly disagreed with it would be worse
//! than none. [`crate::node::ts`] keeps the tree-sitter path alive as an
//! oracle and `tests/parser_differential.rs` runs both over a corpus, so
//! "these agree" is checked rather than asserted.
//!
//! The behaviours that are easy to get wrong, all pinned by that test and by
//! the unit tests below:
//!
//! * **Longest-token-at-each-position.** `1a↓` is content `1` then a reduce
//!   annotated `a`, because the annotation regex `([a-z][a-z0-9]*)?↓` cannot
//!   start at the digit. `x = 42↖1↗` keeps `42` as content for the same reason.
//! * **`//` always wins over content**, wherever it appears — the grammar gives
//!   `plain_line_comment` a token precedence over the character class, so
//!   `https://example.com` really is content `https:` plus a comment.
//! * **Inside a bracket a line comment stops at `↗`/`↘`** (issue #226) while at
//!   ground level it runs to end of line, closing arrows and all.
//! * **Quilt's own comments vanish**, and the line form swallows a preceding
//!   `\n` plus indentation, so removing one leaves no blank line behind. They
//!   never merge the content on either side into one node.
//! * **An escape is its own node.** `a\↖b` is three `Content`s, not one.
//! * **An unterminated `/* … */` is not a comment at all**, it is content.
//!
//! ## Depth
//!
//! Nesting is tracked on an explicit [`Vec`] rather than by recursing, so
//! parsing has no depth limit of its own: `↖`×`100_000` is a diagnostic, where
//! the tree-sitter path aborted the process. That is the contract
//! `fuzz/fuzz_targets/parse_quilt.rs` and `quilt check` rest on.
//!
//! What is left is not the parser's: a deeply nested [`Node`] is a deeply
//! nested *value*, and `Node`'s derived `Drop` recurses like any tree's. The
//! ceiling moved from ~`2_000` nested brackets to ~`8_000` rather than away, and
//! moving it further means an iterative `Drop` for `Node` — a change to the
//! type, not to this file.

use super::Node;
use crate::glyphs::{ESCAPE_LEN, GLYPHS};
use crate::prelude::*;
use miette::LabeledSpan;

/// Quilt's own line comment. Stripped from the output entirely, unlike
/// [`Node::PlainLineComment`], which passes through.
const Q_LINE: &str = "⟨//⟩";
/// Quilt's own block comment delimiters. Also stripped.
const Q_BLOCK_OPEN: &str = "⟨/*⟩";
const Q_BLOCK_CLOSE: &str = "⟨*/⟩";
/// The `⟨T⟩` type placeholder and `⟨N⟩` name placeholder.
const TYPE: &str = "⟨T⟩";
const NAME: &str = "⟨N⟩";

/// Parse Quilt source into a flat list of [`Node`]s. See [`Node::parse`].
pub(super) fn parse(src: &str) -> Result<Box<[Node]>> {
    Parser {
        src,
        pos: 0,
        probe_failed_until: 0,
        no_block_close_from: usize::MAX,
        no_q_block_close_from: usize::MAX,
    }
    .run()
}

/// One open `anno↖` / `anno↙` and the nodes collected inside it so far.
struct Frame {
    anno: Box<str>,
    /// Byte offset of the *whole* opener token, annotation included — which is
    /// where the "never closed" diagnostic points.
    open: usize,
    /// `true` for `↖…↗`, `false` for `↙…↘`.
    quote: bool,
    nodes: Vec<Node>,
}

impl Frame {
    /// The opener token's byte range: the annotation plus its glyph.
    fn opener(&self) -> Span {
        let glyph = if self.quote { '↖' } else { '↙' };
        self.open..self.open + self.anno.len() + glyph.len_utf8()
    }
}

/// What one turn of the scanner produced.
enum Step {
    /// End of input.
    Done,
    /// A token that leaves no node behind — one of Quilt's own comments.
    Skip,
    Node(Node),
    Open {
        anno: Box<str>,
        quote: bool,
        open: usize,
    },
    /// A closer matching the frame on top of the stack; carries its end offset
    /// so the bracket's span can be sealed.
    Close {
        end: usize,
    },
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    /// Memo for [`Parser::comment_after_newline`]. A `\n…` probe's outcome
    /// depends only on where the whitespace run *ends*, so once one has failed
    /// every `\n` inside that run fails too. Without this, a file of blank
    /// lines is quadratic.
    probe_failed_until: usize,
    /// Memo: there is no `*/` at or after this offset. A failed search from `p`
    /// settles every `p' >= p`, which keeps `/*/*/*…` linear.
    no_block_close_from: usize,
    /// The same, for `⟨*/⟩`.
    no_q_block_close_from: usize,
}

impl Parser<'_> {
    fn run(mut self) -> Result<Box<[Node]>> {
        let mut stack: Vec<Frame> = Vec::new();
        let mut out: Vec<Node> = Vec::new();
        loop {
            match self.step(stack.last().map(|f| f.quote))? {
                Step::Done => {
                    // Anything still open at end of input is the error, and the
                    // *innermost* one is the most specific thing to point at.
                    return match stack.last() {
                        Some(frame) => Err(never_closed(frame)),
                        None => Ok(out.into()),
                    };
                }
                Step::Skip => {}
                Step::Node(node) => sink(&mut stack, &mut out).push(node),
                Step::Open { anno, quote, open } => stack.push(Frame {
                    anno,
                    open,
                    quote,
                    nodes: Vec::new(),
                }),
                Step::Close { end } => {
                    let frame = stack.pop().expect("`step` only closes an open frame");
                    let span = frame.open..end;
                    let nodes = frame.nodes.into_iter().map(arc).collect();
                    let anno = frame.anno;
                    let node = if frame.quote {
                        Node::Quote { anno, nodes, span }
                    } else {
                        Node::Unquote { anno, nodes, span }
                    };
                    sink(&mut stack, &mut out).push(node);
                }
            }
        }
    }

    /// Consume one token. `open` is the top frame's kind, or `None` at ground
    /// level — it decides both whether a closer is expected and how far a line
    /// comment may run (issue #226).
    fn step(&mut self, open: Option<bool>) -> Result<Step> {
        let start = self.pos;
        let Some(c) = self.peek() else {
            return Ok(Step::Done);
        };
        match c {
            // A Quilt comment may begin with the newline before it, taking the
            // line's indentation with it. Only if the whole token matches,
            // though — otherwise this is an ordinary line break.
            '\n' => {
                if self.comment_after_newline(open.is_some()) {
                    return Ok(Step::Skip);
                }
                self.pos += 1;
                Ok(Step::Node(Node::NewLine))
            }
            '⟨' => self.angle(open.is_some()),
            '↗' | '↘' => {
                let quote = c == '↗';
                match open {
                    Some(o) if o == quote => {
                        self.pos += c.len_utf8();
                        Ok(Step::Close { end: self.pos })
                    }
                    Some(_) if quote => Err(error(
                        glyph_span(start, c),
                        "expected `↘` here: the open bracket is a `↙` unquote",
                    )),
                    Some(_) => Err(error(
                        glyph_span(start, c),
                        "expected `↗` here: the open bracket is a `↖` quote",
                    )),
                    None => Err(error(
                        glyph_span(start, c),
                        if quote {
                            "no `↖` is open here"
                        } else {
                            "no `↙` is open here"
                        },
                    )),
                }
            }
            '↖' | '↙' => {
                self.pos += c.len_utf8();
                Ok(Step::Open {
                    anno: "".into(),
                    quote: c == '↖',
                    open: start,
                })
            }
            '↑' => {
                self.pos += c.len_utf8();
                Ok(Step::Node(Node::Lift))
            }
            '↓' => {
                self.pos += c.len_utf8();
                Ok(Step::Node(Node::Reduce { anno: "".into() }))
            }
            '←' => {
                self.pos += c.len_utf8();
                Ok(Step::Node(Node::Emit))
            }
            '⟩' => Err(error(
                glyph_span(start, c),
                "a bare `⟩` has no meaning; `\\⟩` writes one literally",
            )),
            // `\` + glyph is an escape and becomes content holding the bare
            // glyph. `\` + anything else is ordinary content (the grammar's
            // `_non_escape`), and a `\` with nothing after it is an error.
            '\\' => match self.char_at(start + ESCAPE_LEN) {
                Some(g) if GLYPHS.contains(&g) => {
                    self.pos = start + ESCAPE_LEN + g.len_utf8();
                    Ok(Step::Node(Node::Content(
                        self.src[start + ESCAPE_LEN..self.pos].into(),
                    )))
                }
                Some(_) => self.content(),
                None => Err(error(
                    start..start + ESCAPE_LEN,
                    "a `\\` escape needs a character after it",
                )),
            },
            '/' if self.at(start, "//") => {
                self.pos = start + 2;
                let end = self.line_end(open.is_some());
                self.pos = end;
                Ok(Step::Node(Node::PlainLineComment(
                    self.src[start..end].into(),
                )))
            }
            '/' if self.at(start, "/*") => match self.block_close(start + 2) {
                Some(end) => {
                    self.pos = end;
                    Ok(Step::Node(Node::PlainBlockComment(
                        self.src[start..end].into(),
                    )))
                }
                // An unterminated `/*` is not a comment token at all, so it is
                // plain content — matching the grammar, where the token simply
                // fails to match and the character class picks the `/` up.
                None => self.content(),
            },
            // An annotated opener or reduce: `[a-z][a-z0-9]*` immediately
            // followed by `↖`, `↙` or `↓`.
            c if c.is_ascii_lowercase() => {
                let run = self.ident_run_end(start);
                match self.annotation_glyph(run) {
                    Some(g) => {
                        let anno: Box<str> = self.src[start..run].into();
                        self.pos = run + g.len_utf8();
                        Ok(match g {
                            '↓' => Step::Node(Node::Reduce { anno }),
                            _ => Step::Open {
                                anno,
                                quote: g == '↖',
                                open: start,
                            },
                        })
                    }
                    None => self.content(),
                }
            }
            _ => self.content(),
        }
    }

    /// A run of ordinary characters, ending where any other token could begin.
    ///
    /// Always consumes at least one character: every caller has already
    /// established that the character at [`Self::pos`] is not the start of some
    /// other token.
    fn content(&mut self) -> Result<Step> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            match c {
                '\n' => break,
                '\\' => match self.char_at(self.pos + ESCAPE_LEN) {
                    // An `escape` token starts here.
                    Some(g) if GLYPHS.contains(&g) => break,
                    Some(other) => self.pos += ESCAPE_LEN + other.len_utf8(),
                    None => {
                        return Err(error(
                            self.pos..self.pos + ESCAPE_LEN,
                            "a `\\` escape needs a character after it",
                        ))
                    }
                },
                _ if GLYPHS.contains(&c) => break,
                '/' if self.at(self.pos, "//") => break,
                '/' if self.at(self.pos, "/*") && self.block_close(self.pos + 2).is_some() => break,
                // A run of `[a-z0-9]` may end in an annotation. If it does, the
                // annotation begins at the run's first *letter* — a digit
                // cannot start one — and everything before that stays content.
                _ if c.is_ascii_lowercase() || c.is_ascii_digit() => {
                    let run = self.ident_run_end(self.pos);
                    let anno = self
                        .annotation_glyph(run)
                        .and_then(|_| self.first_lowercase(self.pos, run));
                    match anno {
                        // `step` dispatches an annotation before it ever calls
                        // `content`, so `anno == self.pos` only happens after
                        // this loop has already consumed something.
                        Some(anno) if anno > start => {
                            self.pos = anno;
                            break;
                        }
                        _ => self.pos = run,
                    }
                }
                _ => self.pos += c.len_utf8(),
            }
        }
        debug_assert!(self.pos > start, "content must make progress");
        Ok(Step::Node(Node::Content(self.src[start..self.pos].into())))
    }

    /// A token opening with `⟨`: the two placeholders, or one of Quilt's own
    /// comments. Anything else is a stray glyph.
    fn angle(&mut self, bracketed: bool) -> Result<Step> {
        let start = self.pos;
        if self.at(start, TYPE) {
            self.pos = start + TYPE.len();
            return Ok(Step::Node(Node::Type));
        }
        if self.at(start, NAME) {
            self.pos = start + NAME.len();
            return Ok(Step::Node(Node::Name));
        }
        if self.at(start, Q_LINE) {
            self.pos = start + Q_LINE.len();
            self.pos = self.line_end(bracketed);
            return Ok(Step::Skip);
        }
        if self.at(start, Q_BLOCK_OPEN) {
            return match self.q_block_close(start + Q_BLOCK_OPEN.len()) {
                Some(end) => {
                    self.pos = end;
                    Ok(Step::Skip)
                }
                None => Err(error(
                    start..start + Q_BLOCK_OPEN.len(),
                    "this `⟨/*⟩` comment is never closed",
                )),
            };
        }
        Err(error(
            glyph_span(start, '⟨'),
            "`⟨` only opens `⟨T⟩`, `⟨N⟩`, `⟨//⟩` or `⟨/*⟩`",
        ))
    }

    /// A Quilt comment reached from the newline in front of it, which it takes
    /// with it along with the indentation that follows — so deleting the
    /// comment does not leave a blank line. Returns whether one was consumed;
    /// on `false` nothing has moved.
    fn comment_after_newline(&mut self, bracketed: bool) -> bool {
        if self.pos < self.probe_failed_until {
            return false;
        }
        let bytes = self.src.as_bytes();
        let mut p = self.pos + 1;
        // The grammar spells this `/\n\s*/`, and tree-sitter's `\s` is the six
        // ASCII whitespace characters — `\u{a0}` and friends are not in it.
        while bytes
            .get(p)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        {
            p += 1;
        }
        if self.at(p, Q_LINE) {
            self.pos = p + Q_LINE.len();
            self.pos = self.line_end(bracketed);
            return true;
        }
        if self.at(p, Q_BLOCK_OPEN) {
            if let Some(end) = self.q_block_close(p + Q_BLOCK_OPEN.len()) {
                self.pos = end;
                return true;
            }
        }
        self.probe_failed_until = p;
        false
    }

    /// Where a line comment that started at [`Self::pos`] ends: the next
    /// newline, or — inside a bracket — the next closing arrow, whichever comes
    /// first (issue #226).
    fn line_end(&self, bracketed: bool) -> usize {
        let rest = &self.src[self.pos..];
        let end = rest
            .find(|c: char| c == '\n' || (bracketed && (c == '↗' || c == '↘')))
            .unwrap_or(rest.len());
        self.pos + end
    }

    /// The end offset of the first `*/` at or after `from`.
    fn block_close(&mut self, from: usize) -> Option<usize> {
        if from >= self.no_block_close_from {
            return None;
        }
        let Some(i) = self.src[from..].find("*/") else {
            self.no_block_close_from = from;
            return None;
        };
        Some(from + i + 2)
    }

    /// The end offset of the `⟨*/⟩` that closes a `⟨/*⟩` opened at `from`.
    ///
    /// **Not** the first `⟨*/⟩` substring after `from`. The grammar spells the
    /// body as a repeat of "any char that is not `⟨`, or a `⟨` that is not the
    /// start of the terminator", and those alternatives consume the character
    /// *after* the `⟨` as well — so a `⟨` can swallow the `⟨` that would have
    /// closed the comment. `⟨/*⟩⟨⟨*/⟩` is the smallest case: the body's `⟨⟨`
    /// eats both, leaving `*/⟩`, and the comment is never closed. Scanning for
    /// the substring calls that a comment; the grammar does not.
    fn q_block_close(&mut self, from: usize) -> Option<usize> {
        // Cheap, sound pre-filter: if the terminator does not occur at all,
        // neither does an aligned one, and that settles every later `from` too
        // — which is what keeps a long file of unterminated `⟨/*⟩` linear.
        if from >= self.no_q_block_close_from {
            return None;
        }
        if !self.src[from..].contains(Q_BLOCK_CLOSE) {
            self.no_q_block_close_from = from;
            return None;
        }
        let mut p = from;
        while let Some(c) = self.char_at(p) {
            if c != '⟨' {
                p += c.len_utf8();
                continue;
            }
            if self.at(p, Q_BLOCK_CLOSE) {
                return Some(p + Q_BLOCK_CLOSE.len());
            }
            // Not the terminator, so the body matched one of `⟨[^*]`,
            // `⟨\*[^/]`, `⟨\*/[^⟩]` — and each consumes the character that
            // ruled the terminator out, not just the `⟨`. That trailing
            // character is the whole subtlety: it can itself be the `⟨` of a
            // real `⟨*/⟩`, which the body then eats.
            p += c.len_utf8();
            for expected in ['*', '/', '⟩'] {
                // Input ran out mid-alternative: no body, so no comment.
                let got = self.char_at(p)?;
                p += got.len_utf8();
                if got != expected {
                    break;
                }
            }
        }
        None
    }

    /// The end of the maximal `[a-z0-9]` run starting at `from`.
    fn ident_run_end(&self, from: usize) -> usize {
        let bytes = self.src.as_bytes();
        let mut p = from;
        while bytes
            .get(p)
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            p += 1;
        }
        p
    }

    /// The glyph an annotation ending at `run` would be attached to, if any.
    fn annotation_glyph(&self, run: usize) -> Option<char> {
        match self.char_at(run) {
            Some(g @ ('↖' | '↙' | '↓')) => Some(g),
            _ => None,
        }
    }

    /// The first `[a-z]` in `src[from..to]`, where an annotation could start.
    fn first_lowercase(&self, from: usize, to: usize) -> Option<usize> {
        self.src.as_bytes()[from..to]
            .iter()
            .position(u8::is_ascii_lowercase)
            .map(|i| from + i)
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn char_at(&self, at: usize) -> Option<char> {
        self.src.get(at..).and_then(|s| s.chars().next())
    }

    fn at(&self, at: usize, what: &str) -> bool {
        self.src[at..].starts_with(what)
    }
}

/// The byte range of a single glyph at `at`.
fn glyph_span(at: usize, glyph: char) -> Span {
    at..at + glyph.len_utf8()
}

/// Where the next node goes: into the innermost open bracket, or into the
/// top-level list.
fn sink<'a>(stack: &'a mut [Frame], out: &'a mut Vec<Node>) -> &'a mut Vec<Node> {
    match stack.last_mut() {
        Some(frame) => &mut frame.nodes,
        None => out,
    }
}

/// The diagnostic for a bracket still open at end of input.
///
/// It points at the *opener* rather than at the end of the file: that is where
/// the fix goes, and a zero-width span past the last byte renders with no caret
/// at all. `tests/ui/unbalanced_bracket.rs.quilt` pins the rendering.
fn never_closed(frame: &Frame) -> miette::Report {
    error(frame.opener(), "this bracket is never closed")
}

/// A diagnostic for malformed Quilt surface syntax, pointing at the offending
/// span. Callers holding the source text (the CLI, the LSP) can attach it with
/// [`miette::Report::with_source_code`] to render the snippet.
fn error(span: Span, what: &str) -> miette::Report {
    miette!(
        labels = vec![LabeledSpan::at(span.clone(), what)],
        help = "Quilt brackets must be balanced and nested: `↖…↗` quotes and \
                `↙…↘` unquotes. A glyph meant as literal text needs a `\\` \
                escape.",
        "malformed Quilt syntax (source bytes {}..{})",
        span.start,
        span.end,
    )
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    /// One label, and it says what it should. `tests/ui/` snapshots the
    /// rendering of the unclosed-bracket case; these pin the rest, which the
    /// differential test deliberately does not compare (tree-sitter's error
    /// recovery reports what its parse *states* justify — for `↖x↘` that is the
    /// whole input — while the scanner reports the token it choked on).
    #[test]
    fn diagnostics_point_at_the_offending_token() {
        for (src, span, label) in [
            ("let x = ↖1 + 2;\n", 8..11, "this bracket is never closed"),
            ("py↖", 0..5, "this bracket is never closed"),
            ("aaaa↗bbbb", 4..7, "no `↖` is open here"),
            ("↘", 0..3, "no `↙` is open here"),
            (
                "↖x↘",
                4..7,
                "expected `↗` here: the open bracket is a `↖` quote",
            ),
            (
                "↙x↗",
                4..7,
                "expected `↘` here: the open bracket is a `↙` unquote",
            ),
            ("⟨X⟩", 0..3, "`⟨` only opens `⟨T⟩`, `⟨N⟩`, `⟨//⟩` or `⟨/*⟩`"),
            (
                "⟩",
                0..3,
                "a bare `⟩` has no meaning; `\\⟩` writes one literally",
            ),
            ("a\\", 1..2, "a `\\` escape needs a character after it"),
            ("⟨/*⟩ nope", 0..8, "this `⟨/*⟩` comment is never closed"),
        ] {
            let err = Node::parse(src).expect_err("should not parse");
            assert!(
                err.to_string().contains("malformed Quilt syntax"),
                "{src:?}: {err}"
            );
            let labels: Vec<_> = err.labels().into_iter().flatten().collect();
            assert_eq!(
                labels.len(),
                1,
                "{src:?}: expected one label, got {labels:?}"
            );
            assert_eq!(
                labels[0].offset()..labels[0].offset() + labels[0].len(),
                span,
                "{src:?}: wrong span"
            );
            assert_eq!(labels[0].label(), Some(label), "{src:?}: wrong label");
        }
    }

    /// Nesting lives on a `Vec`, not the call stack, so *parsing* has no depth
    /// limit: 100k unclosed brackets is a diagnostic, where the tree-sitter
    /// path aborted the process well before that.
    ///
    /// The balanced case is capped lower on purpose. It builds the term, and a
    /// term that deep is a recursive value whose `Drop` recurses — nothing to
    /// do with the parser, and the reason this is `4_000` and not `100_000`. Even
    /// so it is twice what the tree-sitter path survived (`↖`×2000 with its
    /// closers overflowed the stack outright).
    #[test]
    fn deep_nesting_does_not_overflow() {
        assert!(Node::parse(&"↖".repeat(100_000)).is_err());

        let balanced = format!("{}x{}", "↖".repeat(4_000), "↗".repeat(4_000));
        let nodes = Node::parse(&balanced).expect("balanced brackets, however deep");
        assert_eq!(nodes.len(), 1);
        assert_eq!(&*Node::coparse(&nodes), balanced);
    }

    /// The scans that could go quadratic, at a size where quadratic would not
    /// finish: an unterminated `⟨/*⟩` or `/*` searches to end of input, and a
    /// `\n` before whitespace probes for a comment that is not there. Each is
    /// memoised on the offset that settled it, and each of these inputs is one
    /// where the memo is the only thing standing between linear and 10^10
    /// character steps.
    #[test]
    fn pathological_input_stays_linear() {
        for src in [
            "⟨/*⟩".repeat(50_000),
            "/*".repeat(50_000),
            "\n".repeat(200_000),
            "\n \t".repeat(100_000),
        ] {
            // The assertion is that this returns at all, in test-suite time.
            let _ = Node::parse(&src);
        }
    }

    /// A `⟨/*⟩` comment ends at the first *aligned* `⟨*/⟩`, not the first one
    /// that occurs — the grammar's body alternatives consume the character
    /// after a `⟨`, which can be the `⟨` that would have closed the comment.
    /// Found by the sweep in `tests/parser_differential.rs`, not by hand.
    #[test]
    fn a_block_comment_terminator_can_be_eaten() {
        // `⟨⟨` is one body item, so by the time the scan resumes the `⟨*/⟩`
        // that follows has become `*/⟩`, and nothing closes the comment.
        assert!(Node::parse("⟨/*⟩⟨⟨*/⟩").is_err());
        // The same eaten `⟨*/⟩`, but with a second one behind it: the comment
        // ends at the *last* occurrence here, not the first.
        assert_eq!(&*Node::parse("⟨/*⟩⟨⟨*/⟩⟨*/⟩").expect("closed"), &[]);
        // `⟨*/⟨` is the four-character alternative — `⟨\*/[^⟩]` — and it eats
        // the `⟨` that starts the real terminator just the same.
        assert!(Node::parse("⟨/*⟩⟨*/⟨*/⟩").is_err());
        assert_eq!(&*Node::parse("⟨/*⟩⟨*/⟨⟨*/⟩").expect("closed"), &[]);
    }
}
