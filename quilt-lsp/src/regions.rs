//! Parsing `.quilt` source into a position-aware structure.
//!
//! `quilt`'s own `Node`/`QTerm` IR discards source ranges, so the server can't
//! use them for position mapping. Instead we re-walk the `tree_sitter_quilt`
//! CST directly — it carries byte ranges on every node — and build:
//!
//! * a [`Region`] tree (ground vs `↖↗` quote vs `↙↘` unquote), used by later
//!   phases to project each language into its own virtual document, and
//! * a list of syntax errors for diagnostics.

use std::ops::Range;
use tree_sitter::{Node, Parser, Tree};

/// Byte length of an arrow glyph (`↖↗↙↘↑↓`). They are all 3 bytes in UTF-8.
pub(crate) const ARROW_LEN: usize = "↖".len();

/// Build a tree-sitter parser configured for the Quilt grammar.
pub fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_quilt::LANGUAGE.into())
        .expect("loading the Quilt grammar should never fail");
    parser
}

/// Parse `.quilt` source into a CST. Pass `old_tree` for incremental re-parse
/// (tree-sitter reuses unchanged subtrees). Parsing the raw document text (no
/// wrapping) keeps byte offsets aligned with what the editor sees.
pub fn parse(parser: &mut Parser, text: &str, old_tree: Option<&Tree>) -> Tree {
    parser
        .parse(text, old_tree)
        .expect("parse only returns None when cancelled, which we never do")
}

/// A syntax error discovered in the quilt structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub range: Range<usize>,
    pub message: String,
}

/// Collect tree-sitter `ERROR`/`MISSING` nodes as syntax errors.
pub fn collect_errors(tree: &Tree) -> Vec<SyntaxError> {
    let mut out = Vec::new();
    visit_errors(tree.root_node(), &mut out);
    out
}

fn visit_errors(node: Node, out: &mut Vec<SyntaxError>) {
    if node.is_missing() {
        // Map raw tree-sitter node kinds to the actual glyphs the user wrote.
        let glyph = match node.kind() {
            "right_quote" => "↗",
            "right_unquote" => "↘",
            other => other,
        };
        out.push(SyntaxError {
            range: node.byte_range(),
            message: format!("missing `{glyph}`"),
        });
        return;
    }
    if node.is_error() {
        // Prefer to report at each unclosed opening bracket in the ERROR
        // subtree rather than the full ERROR span, which can cover the whole
        // rest of the file and produce an overwhelming red underline.
        let prev_len = out.len();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "left_quote" => out.push(SyntaxError {
                    range: child.byte_range(),
                    message: "unclosed `↖` — add `↗` to close".into(),
                }),
                "left_unquote" => out.push(SyntaxError {
                    range: child.byte_range(),
                    message: "unclosed `↙` — add `↘` to close".into(),
                }),
                _ if child.has_error() => visit_errors(child, out),
                _ => {}
            }
        }
        if out.len() == prev_len {
            // No bracket or nested error found; point at the first character.
            let start = node.start_byte();
            let end = (start + 1).min(node.end_byte());
            out.push(SyntaxError {
                range: start..end,
                message: "unexpected syntax".into(),
            });
        }
        return;
    }
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_errors(child, out);
    }
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

/// The annotation on a quote/unquote node, e.g. `wgsl` in `wgsl↖…↗`: the
/// opening token's text is `<anno>↖` (or `<anno>↙`); strip the arrow. Empty
/// for plain brackets (or a malformed mid-edit node).
pub(crate) fn node_anno<'t>(text: &'t str, node: Node) -> &'t str {
    let Some(open) = node.child(0) else {
        return "";
    };
    let open_text = &text[open.byte_range()];
    open_text
        .get(..open_text.len().saturating_sub(ARROW_LEN))
        .unwrap_or("")
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
pub fn regions(text: &str, tree: &Tree, chain: &[&str]) -> Region {
    let zipper = LangZipper::from_chain(chain);
    let root = tree.root_node();
    let mut children = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect(text, child, &zipper, 0, &mut children);
    }
    Region {
        kind: RegionKind::Ground,
        lang: zipper.current().map(str::to_string),
        anno: String::new(),
        body: 0..text.len(),
        stage: 0,
        children,
    }
}

fn collect(text: &str, node: Node, zipper: &LangZipper, parent_stage: i32, out: &mut Vec<Region>) {
    let kind = match node.kind() {
        "quote" => RegionKind::Quote,
        "unquote" => RegionKind::Unquote,
        // content / newline / glyphs / comments belong to the parent region.
        _ => return,
    };

    let count = node.child_count();
    if count < 2 {
        return; // malformed (mid-edit); errors are reported separately.
    }
    let open = node.child(0).expect("bracket node has an opening token");
    let close = node
        .child(count - 1)
        .expect("bracket node has a closing token");

    let anno = node_anno(text, node).to_string();

    // Resolve the body's language by mirroring the lang zipper in `multi.rs`:
    // a quote takes its annotation, the next chain default, or the enclosing
    // language; an unquote drops back to the language one level up (its
    // annotation, like in quilt proper, does not select a language).
    let zipper = match kind {
        RegionKind::Quote => zipper.quote(&anno),
        RegionKind::Unquote | RegionKind::Ground => zipper.unquote(),
    };
    let lang = zipper.current().map(str::to_string);

    let stage = parent_stage + if kind == RegionKind::Quote { 1 } else { -1 };
    let body = open.end_byte()..close.start_byte();

    let mut children = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip the bracket tokens; recurse into nested quote/unquote bodies.
        if child.id() == open.id() || child.id() == close.id() {
            continue;
        }
        collect(text, child, &zipper, stage, &mut children);
    }

    out.push(Region {
        kind,
        lang,
        anno,
        body,
        stage,
        children,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_of(text: &str) -> Tree {
        parse(&mut parser(), text, None)
    }

    #[test]
    fn clean_file_has_no_errors() {
        let text = "fn main() {\n    let x = ↖1 + 2↗;\n}\n";
        assert!(collect_errors(&tree_of(text)).is_empty());
    }

    #[test]
    fn unclosed_quote_is_an_error() {
        let text = "let x = ↖1 + 2;\n";
        let errs = collect_errors(&tree_of(text));
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
        let errs = collect_errors(&tree_of(text));
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
        let errs = collect_errors(&tree_of(text));
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
        let errs = collect_errors(&tree_of(text));
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
        let tree = tree_of(text);
        let root = regions(text, &tree, &["rs"]);
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
        let tree = tree_of(text);
        let root = regions(text, &tree, &["rs"]);
        let q = &root.children[0];
        assert_eq!(q.anno, "wgsl");
        assert_eq!(q.lang.as_deref(), Some("wgsl"));
    }

    #[test]
    fn nested_quote_and_unquote_stages() {
        // ground -> quote(+1) -> unquote(0)
        let text = "↖ ↙x↘ ↗\n";
        let tree = tree_of(text);
        let root = regions(text, &tree, &["rs"]);
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
        let tree = tree_of(text);
        let root = regions(text, &tree, &["rs", "wgsl"]);
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

    #[test]
    fn annotation_resets_chain_defaults() {
        // An annotated quote pins its language; an un-annotated quote nested
        // inside it inherits the annotation, not the chain default (mirrors
        // `Zipper::cons` clearing `anti`).
        let text = "let x = py↖a ↖b↗ c↗;\n";
        let tree = tree_of(text);
        let root = regions(text, &tree, &["rs", "wgsl"]);
        let q = &root.children[0];
        assert_eq!(q.lang.as_deref(), Some("py"));
        let q2 = &q.children[0];
        assert_eq!(q2.lang.as_deref(), Some("py"));
    }
}
