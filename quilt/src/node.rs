use crate::strcmd::PrefixWriter;
use crate::term::Term;
use crate::{prelude::*, term::STerm};
use miette::{bail, LabeledSpan};
use std::{fmt::Debug, iter::empty, sync::Arc};

/**************************************************************/

/// UTF-8 length of a Quilt glyph. Every glyph in [`GLYPHS`] is this wide (see
/// `test glyph_lengths_are_uniform`), which is what lets callers doing byte
/// arithmetic over the surface syntax — e.g. `quilt-lsp`'s `regions` — use a
/// single constant. Derived from a glyph rather than written as `3` so it cannot
/// drift from the glyph set it describes.
pub const ARROW_LEN: usize = '↖'.len_utf8();
/// UTF-8 length of the `\` that introduces an escape.
pub const ESCAPE_LEN: usize = '\\'.len_utf8();

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
    /// Parse a source string into a list of `Node`s.
    ///
    /// Malformed bracket structure is a diagnostic, not a panic: an unbalanced
    /// `↖`/`↙` or a stray `↗`/`↘` leaves tree-sitter `ERROR`/`MISSING` nodes in
    /// the tree, which used to reach the `unreachable!` in [`Self::from_ts`] and
    /// abort the process — including in `quilt check`, whose whole job is
    /// reporting diagnostics. See [`syntax_error`].
    pub fn parse(code: &str) -> Result<Box<[Self]>> {
        let mut parser = tree_sitter::Parser::default();
        parser
            .set_language(&tree_sitter_quilt::LANGUAGE.into())
            .expect("Error loading Quilt grammar");
        let tree = parser
            .parse(code, None)
            .ok_or_else(|| miette!("failed to parse Quilt source"))?;
        let root = tree.root_node();
        if root.has_error() {
            return Err(syntax_error(root));
        }

        let mut nodes = Vec::new();
        for child in root.children(&mut root.walk()) {
            nodes.push(Self::from_ts(&child, code)?);
        }
        Ok(nodes.into())
    }

    /// Convert a tree-sitter node + source string to a `Node`.
    ///
    /// An unrecognised node kind is an error rather than a panic, so adding a
    /// rule to `tree-sitter-quilt/grammar.js` without teaching this function
    /// about it degrades to a reportable diagnostic (issue #11).
    pub fn from_ts(node: &tree_sitter::Node, code: &str) -> Result<Self> {
        let text = |n: &tree_sitter::Node| -> &str {
            let range = n.range();
            &code[range.start_byte..range.end_byte]
        };
        Ok(match node.kind() {
            "content" => Node::Content(text(node).into()),
            "escape" => Node::Content(text(node)[ESCAPE_LEN..].into()),
            "newline" => Node::NewLine,
            "quote" => {
                let (anno, nodes) = Self::bracket(node, code, '↖')?;
                let span = node.start_byte()..node.end_byte();
                Node::Quote { anno, nodes, span }
            }
            "unquote" => {
                let (anno, nodes) = Self::bracket(node, code, '↙')?;
                let span = node.start_byte()..node.end_byte();
                Node::Unquote { anno, nodes, span }
            }
            "lift" => Node::Lift,
            "reduce" => Node::Reduce {
                anno: strip_glyph(text(node), '↓')?.into(),
            },
            "emit" => Node::Emit,
            "type" => Node::Type,
            "name" => Node::Name,
            "plain_line_comment" => Node::PlainLineComment(text(node).into()),
            "plain_block_comment" => Node::PlainBlockComment(text(node).into()),
            kind => bail!(
                labels = vec![LabeledSpan::at(
                    node.start_byte()..node.end_byte(),
                    "this node"
                )],
                "Quilt parser: unhandled node kind {kind:?}. This is a gap in \
                 `Node::from_ts`; please report it."
            ),
        })
    }

    /// Split a `quote`/`unquote` node into its language annotation and body.
    ///
    /// The opener token is `[a-z]*↖` (resp. `[a-z]*↙`) per the grammar, so the
    /// annotation is the opener's text with the glyph stripped. The body is
    /// every child between the opener and the closer.
    fn bracket(
        node: &tree_sitter::Node,
        code: &str,
        glyph: char,
    ) -> Result<(Box<str>, Box<[Arc<Node>]>)> {
        let open = node
            .child(0)
            .ok_or_else(|| miette!("Quilt parser: bracket with no opening token"))?;
        let range = open.range();
        let anno = strip_glyph(&code[range.start_byte..range.end_byte], glyph)?.into();

        // children(..) yields the opener and closer too; the body is what sits
        // between them. `saturating_sub` rather than `- 1` so a bracket missing
        // its closer can't underflow (the `has_error` check in `parse` should
        // have caught that already, but this function is public).
        let last = node.child_count().saturating_sub(1);
        let mut nodes = Vec::new();
        for i in 1..last {
            let child = node
                .child(u32::try_from(i).unwrap())
                .ok_or_else(|| miette!("Quilt parser: missing bracket child {i}"))?;
            nodes.push(arc(Self::from_ts(&child, code)?));
        }
        Ok((anno, nodes.into()))
    }

    pub fn coparse(nodes: &[Self]) -> Box<str> {
        let mut buf = std::io::BufWriter::new(Vec::new());
        let mut writer = PrefixWriter::new(&mut buf);
        for n in nodes {
            n.write(&mut writer);
        }
        let bytes = buf.into_inner().unwrap();
        String::from_utf8(bytes).unwrap().into()
    }
}

/// Strip the trailing `glyph` from an opener/operator token's text, leaving its
/// language annotation.
///
/// The grammar spells these tokens `[a-z]*↖` / `[a-z]*↙` / `[a-z]*↓`, so this is
/// exact. It replaces `text[..text.len() - ARROW_LEN]`, which assumed the glyph's
/// byte width and would slice mid-codepoint (a panic) if a glyph of another width
/// were ever added.
fn strip_glyph(text: &str, glyph: char) -> Result<&str> {
    text.strip_suffix(glyph)
        .ok_or_else(|| miette!("Quilt parser: expected {text:?} to end with {glyph:?}"))
}

/// The most specific `ERROR`/`MISSING` node under `node`, for pointing a
/// diagnostic at the smallest span the parse can justify.
fn first_error(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    for child in node.children(&mut node.walk()) {
        if child.is_error() || child.is_missing() || child.has_error() {
            return first_error(child);
        }
    }
    (node.is_error() || node.is_missing()).then_some(node)
}

/// A diagnostic for malformed Quilt bracket structure, pointing at the offending
/// span. Callers holding the source text (the CLI, the LSP) can attach it with
/// [`miette::Report::with_source_code`] to render the snippet.
fn syntax_error(root: tree_sitter::Node) -> miette::Report {
    let node = first_error(root).unwrap_or(root);
    let span = node.start_byte()..node.end_byte();
    let what = if node.is_missing() {
        "expected something here"
    } else {
        "here"
    };
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

/// The characters Quilt gives special meaning to, and hence the ones `\` can
/// escape: the four quote/unquote arrows, lift, reduce, emit, and the `⟨…⟩`
/// delimiters. Must stay in sync with the `_char` / `_non_escape` / `escape`
/// character classes in `tree-sitter-quilt/grammar.js`.
pub const GLYPHS: [char; 9] = ['↖', '↗', '↙', '↘', '↑', '↓', '←', '⟨', '⟩'];

/// Escape every Quilt glyph in `s` with a leading `\`.
///
/// This is the inverse of the grammar's `escape` rule and exists so that
/// [`Node::coparse`] round-trips: a `Node::Content` can only hold a glyph if it
/// came from a `\`-escape in the source (the grammar's `_char` class excludes
/// all of them), so writing the glyph bare would make it re-parse as the
/// operator instead of as content.
pub fn escape(s: &str) -> Box<str> {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if GLYPHS.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out.into()
}

/// Strip the `\` from every escaped Quilt glyph in `s` — the inverse of
/// [`escape`]. Parsing does this structurally (via the grammar's `escape` rule),
/// so this is for callers holding raw source text.
pub fn unescape(s: &str) -> Box<str> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // Only a glyph is consumed by the backslash; anything else (`\n` in
            // a string literal, say) is `_non_escape` and stays verbatim.
            match chars.clone().next() {
                Some(g) if GLYPHS.contains(&g) => {
                    out.push(g);
                    chars.next();
                }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out.into()
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
                for n in nodes {
                    n.write(writer);
                }
                writer.write("↗");
            }
            Node::Unquote { anno, nodes, .. } => {
                writer.write(anno);
                writer.write("↙");
                for n in nodes {
                    n.write(writer);
                }
                writer.write("↘");
            }
            Node::Lift => writer.write("↑"),
            Node::Reduce { anno } => {
                writer.write(anno);
                writer.write("↓");
            }
            Node::Emit => writer.write("←"),
            Node::Type => writer.write("⟨T⟩"),
            Node::Name => writer.write("⟨N⟩"),
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

    /// Malformed bracket structure is an `Err`, never a panic. Each of these
    /// used to abort the process via `unreachable!("… \"ERROR\"")` in
    /// [`Node::from_ts`] — including under `quilt check`, which exists to report
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

    /// An unrecognised node kind is a diagnostic, not a panic — so adding a rule
    /// to `grammar.js` without teaching `from_ts` about it degrades gracefully.
    /// `source_file` stands in for such a kind: it is a real node the grammar
    /// produces, and one `from_ts` is never handed.
    #[test]
    fn unhandled_node_kind_is_an_error() {
        let mut parser = tree_sitter::Parser::default();
        parser
            .set_language(&tree_sitter_quilt::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse("abc", None).unwrap();
        let err = Node::from_ts(&tree.root_node(), "abc")
            .expect_err("`source_file` is not a kind `from_ts` handles");
        assert!(
            err.to_string().contains("unhandled node kind"),
            "diagnostic should name the kind, got: {err}"
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
