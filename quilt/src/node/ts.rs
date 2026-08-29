//! The tree-sitter path: the Quilt surface parser Quilt used to have, kept as
//! the oracle for the one it has now.
//!
//! `tree-sitter-quilt/grammar.js` is still the *specification* of Quilt's
//! surface syntax — `quilt-lsp`'s `regions` and the VS Code extension read it,
//! so it is maintained regardless — and issue #254 replaced the parser, not the
//! grammar. Keeping this path compiling is what lets
//! `tests/parser_differential.rs` run [`Node::parse_ts`] and [`Node::parse`]
//! over the same corpus and require the trees to be equal, so the two
//! descriptions of Quilt cannot drift apart in silence.
//!
//! Nothing else calls into here. [`Node::parse`] is the parser.

use super::{Node, ESCAPE_LEN};
use crate::prelude::*;
use miette::{bail, LabeledSpan};
use std::sync::Arc;

impl Node {
    /// Parse a source string into a list of `Node`s, via tree-sitter.
    ///
    /// The reference implementation of [`Node::parse`], and only that: see the
    /// module docs.
    ///
    /// Malformed bracket structure is a diagnostic, not a panic: an unbalanced
    /// `↖`/`↙` or a stray `↗`/`↘` leaves tree-sitter `ERROR`/`MISSING` nodes in
    /// the tree, which used to reach the `unreachable!` in [`Self::from_ts`] and
    /// abort the process — including in `quilt check`, whose whole job is
    /// reporting diagnostics. See [`syntax_error`].
    pub fn parse_ts(code: &str) -> Result<Box<[Self]>> {
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
    /// The opener token is `([a-z][a-z0-9]*)?↖` (resp. `…↙`) per the grammar,
    /// so the annotation is the opener's text with the glyph stripped. The body
    /// is every child between the opener and the closer.
    fn bracket(node: &tree_sitter::Node, code: &str, glyph: char) -> Result<Bracket> {
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
}

/// The two halves [`Node::bracket`] splits a `quote`/`unquote` node into: its
/// language annotation and its body nodes.
type Bracket = (Box<str>, Box<[Arc<Node>]>);

/// Strip the trailing `glyph` from an opener/operator token's text, leaving its
/// language annotation.
///
/// The grammar spells these tokens `([a-z][a-z0-9]*)?↖` / `…↙` / `…↓`, so this
/// is exact. It replaces `text[..text.len() - ARROW_LEN]`, which assumed the
/// glyph's byte width and would slice mid-codepoint (a panic) if a glyph of
/// another width were ever added.
///
/// Nothing here constrains the annotation's *shape* — that lives entirely in
/// the grammar, which is why widening it to admit `lean4` (issue #222) needed
/// no change on this side.
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
    // A `MISSING` node is zero-width, and at end of input there is nothing
    // under it to underline: miette renders the snippet with no caret at all
    // and the reader gets a byte offset one past the source. Point at the
    // bracket that was left open instead — its opener is where the fix goes,
    // and it is a span that exists.
    let (span, what) = match (node.start_byte() == node.end_byte())
        .then(|| node.parent().and_then(|p| p.child(0)))
        .flatten()
    {
        Some(opener) => (
            opener.start_byte()..opener.end_byte(),
            "this bracket is never closed",
        ),
        None if node.is_missing() => (
            node.start_byte()..node.end_byte(),
            "expected something here",
        ),
        None => (node.start_byte()..node.end_byte(), "here"),
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
