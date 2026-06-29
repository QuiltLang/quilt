//! Projecting a `.quilt` document into a single-language virtual document.
//!
//! The **ground projection** copies every ground-language byte verbatim and
//! replaces each quilt construct (`↖…↗`, `↙…↘`, and the `↑↓←⟨T⟩⟨N⟩` glyphs) with
//! a small placeholder, recording the mapping in a [`SourceMap`]. For a file
//! with no quilt constructs (e.g. `hello.rs.quilt`) the projection is
//! byte-identical to the source, so the downstream server sees ordinary code.
//!
//! Placeholders keep the host language roughly parseable; any diagnostics the
//! downstream server reports *on synthetic spans* are dropped by the router
//! (see [`Projection::is_synthetic`]), so the placeholders never surface as
//! spurious errors.

use crate::adapters::{language_adapter, CommentSyntax, LanguageAdapter, MetaLanguageAdapter};
use crate::lineindex::{Encoding, LineIndex};
use crate::regions::{self, LangZipper};
use crate::srcmap::{Builder, SourceMap};
use std::ops::Range;
use tower_lsp::lsp_types::{Position, Range as LspRange};

/// A virtual document plus the map back to quilt coordinates.
#[derive(Debug, Clone)]
pub struct Projection {
    pub text: String,
    pub line_index: LineIndex,
    pub map: SourceMap,
    /// Virtual byte ranges of appended quote fragments. Their tokens are kept
    /// (for highlighting) but their diagnostics are suppressed (wrapping makes
    /// them unreliable).
    pub fragment_ranges: Vec<Range<usize>>,
}

/// Build the projection of `text` for ground language `meta`, with quoted
/// fragments analyzed as `lang`. `chain` is the filename's language-extension
/// chain (ground first, see [`crate::adapters::lang_chain`]); it decides what
/// language each un-annotated quote is.
///
/// Two passes over the quilt CST:
/// 1. **Ground (stage-aware):** every stage-0 span is copied verbatim — *even
///    stage-0 `↙…↘` splices nested inside quotes*, which are reabsorbed into a
///    ground splice block at the quote's site (so the host server resolves the
///    ground names they reference). Quotes become `meta.splice_block()`.
/// 2. **Fragments:** each quoted body *in `lang`* is appended, wrapped by
///    `lang.wrap_fragment`, so the server tokenizes the embedded language.
///    Quotes in other languages (e.g. WGSL in a Rust host) get no fragment —
///    their bodies must not reach the `lang` server — though their stage-0
///    splices are still reabsorbed by pass 1.
///
/// A construct-free file projects to itself byte-for-byte. The source map's
/// copied spans stay disjoint: stage-0 bytes appear once (ground), stage-≥1
/// content at most once (its fragment).
pub fn project(
    text: &str,
    meta: &dyn MetaLanguageAdapter,
    lang: &dyn LanguageAdapter,
    chain: &[&str],
) -> Projection {
    let mut parser = regions::parser();
    let tree = regions::parse(&mut parser, text, None);
    let root = tree.root_node();

    let mut b = Builder::new();
    let mut quotes: Vec<(tree_sitter::Node, LangZipper)> = Vec::new();

    let env = Ground { meta, lang };
    let zipper = LangZipper::from_chain(chain);
    emit_ground(
        &mut b,
        text,
        root,
        0..text.len(),
        0,
        &env,
        &zipper,
        &mut quotes,
    );

    // Fragment pass (transitive: a fragment surfaces nested quotes).
    let mut fragment_ranges = Vec::new();
    let mut i = 0usize;
    let mut n = 0usize; // fragment label; counted separately so skips leave no gap
    while i < quotes.len() {
        let (q, qz) = quotes[i].clone();
        i += 1;
        // Only quotes in this projection's fragment language are appended.
        let is_lang = qz.current().is_some_and(|key| {
            language_adapter(key).is_some_and(|a| a.language_id() == lang.language_id())
        });
        if !is_lang {
            continue;
        }
        let Some(window) = inner_window(q) else {
            continue;
        };
        let (pre, post) = lang.wrap_fragment(n);
        n += 1;
        let start = b.len();
        b.synth(&pre);
        emit_fragment(&mut b, text, q, window, lang, &qz, &mut quotes);
        b.synth(&post);
        fragment_ranges.push(start..b.len());
    }

    let (mut vtext, map) = b.finish();
    // A shebang (`#!/…`) on the first line is valid for `quilt` scripts but
    // not for Rust parsers: `#!` starts an inner attribute, and `/usr/bin/…` is
    // not valid attribute syntax. Replace `#!` → `//` (same byte length,
    // preserves all positions) so the downstream server treats it as a comment.
    // Languages whose line comment already starts with `#` (e.g. Python) need no
    // rewrite — the shebang is a comment to them as-is.
    if vtext.starts_with("#!") && !lang.comment_syntax().line.starts_with('#') {
        // SAFETY: `#!` are ASCII (0x23, 0x21); we overwrite with `//` (0x2F,
        // 0x2F), also ASCII single-byte. The string stays valid UTF-8.
        let bytes = unsafe { vtext.as_bytes_mut() };
        bytes[0] = b'/';
        bytes[1] = b'/';
    }
    let line_index = LineIndex::new(&vtext);
    Projection {
        text: vtext,
        line_index,
        map,
        fragment_ranges,
    }
}

/// One embedded-language fragment (a `wgsl↖…↗` quote) projected into its own
/// standalone virtual document. Unlike the merged ground projection, each
/// fragment is independent — a WGSL quote is a complete shader module — so they
/// must not share a document (their top-level definitions would collide).
#[derive(Debug, Clone)]
pub struct FragmentDoc {
    /// Quilt byte range of the quote's inner body, used to route a cursor
    /// position to the fragment it falls in.
    pub quilt_range: Range<usize>,
    /// The fragment's virtual document and its map back to quilt coordinates.
    pub proj: Projection,
}

/// Project every quote written in `lang` into its own [`FragmentDoc`]. `chain`
/// resolves the language of un-annotated quotes (see
/// [`crate::adapters::lang_chain`]). Used for embedded *target* languages whose
/// server analyzes each quoted fragment as a standalone unit (e.g. WGSL →
/// wgsl-analyzer). Nested `↙…↘` splices in another language are masked to the
/// fragment language's placeholder so the fragment stays parseable.
pub fn project_fragments(
    text: &str,
    lang: &dyn LanguageAdapter,
    chain: &[&str],
) -> Vec<FragmentDoc> {
    let mut parser = regions::parser();
    let tree = regions::parse(&mut parser, text, None);

    let zipper = LangZipper::from_chain(chain);
    let mut quotes: Vec<(tree_sitter::Node, LangZipper)> = Vec::new();
    collect_quotes(text, tree.root_node(), &zipper, &mut quotes);

    let mut out = Vec::new();
    let mut sink = Vec::new(); // emit_fragment queues nested quotes here; ignored.
    for (q, qz) in quotes {
        let is_lang = qz.current().is_some_and(|key| {
            language_adapter(key).is_some_and(|a| a.language_id() == lang.language_id())
        });
        if !is_lang {
            continue;
        }
        let Some(window) = inner_window(q) else {
            continue;
        };
        let mut b = Builder::new();
        emit_fragment(&mut b, text, q, window.clone(), lang, &qz, &mut sink);
        let (vtext, map) = b.finish();
        let line_index = LineIndex::new(&vtext);
        out.push(FragmentDoc {
            quilt_range: window,
            proj: Projection {
                text: vtext,
                line_index,
                map,
                fragment_ranges: Vec::new(),
            },
        });
    }
    out
}

/// Project a **sky-first template** (`*.tmpl.quilt`) into its target-language
/// virtual document. Unlike [`project`], which treats the file as a ground-first
/// program, a template *is* the body of an implicit `target↖ … ↗`: the whole
/// file is target-language source and every `↙name↘` is a parameter hole (a free
/// variable filled at instantiation), not a ground splice. So the entire
/// document is projected as one target-language fragment with each construct
/// (`↙…↘` holes, nested quotes, glyphs) masked to the target's placeholder —
/// keeping the body parseable so its server tokenizes / analyzes it. `chain` is
/// the template's language chain (host-first; the target is its last element);
/// it mirrors `Multi::parse_template` entering at the target's quote.
pub fn project_sky(text: &str, lang: &dyn LanguageAdapter, chain: &[&str]) -> Projection {
    let mut parser = regions::parser();
    let tree = regions::parse(&mut parser, text, None);
    // Enter the target's implicit quote, so a nested `↙…↘` pops back to the host
    // exactly as inside a real quote (the holes' bodies are host expressions).
    let zipper = LangZipper::from_chain(chain).quote("");
    let mut sink = Vec::new(); // nested quotes are masked here, not re-projected
    let mut b = Builder::new();
    emit_fragment(
        &mut b,
        text,
        tree.root_node(),
        0..text.len(),
        lang,
        &zipper,
        &mut sink,
    );
    let (vtext, map) = b.finish();
    let line_index = LineIndex::new(&vtext);
    Projection {
        text: vtext,
        line_index,
        map,
        fragment_ranges: Vec::new(),
    }
}

/// The parameter holes of a sky-first template: each `↙name↘` whose body is a
/// bare identifier is a free variable the instantiation must supply (mirroring
/// quilt's `template_params`). Returns `(name, quilt byte range of the `↙…↘`)`
/// in first-seen order, de-duplicated by name. Holes whose body is a richer host
/// expression (a Tier B concern) are not parameters and are skipped.
pub fn sky_param_holes(text: &str) -> Vec<(String, Range<usize>)> {
    let mut parser = regions::parser();
    let tree = regions::parse(&mut parser, text, None);
    let mut out = Vec::new();
    collect_param_holes(text, tree.root_node(), &mut out);
    out
}

fn collect_param_holes(text: &str, node: tree_sitter::Node, out: &mut Vec<(String, Range<usize>)>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "unquote" {
            if let Some(window) = inner_window(child) {
                let body = text[window].trim();
                if is_ident(body) && !out.iter().any(|(n, _)| n == body) {
                    out.push((body.to_string(), child.byte_range()));
                }
            }
        }
        collect_param_holes(text, child, out);
    }
}

/// Whether `s` is a plain identifier — the body of a `↙name↘` parameter hole.
/// Mirrors quilt's `multi::ident_name`: a letter or `_` then alphanumerics/`_`.
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Walk the whole quilt CST collecting every `↖…↗` quote with its resolved
/// language zipper, threading quote/unquote nesting (see [`LangZipper`]).
fn collect_quotes<'a>(
    text: &str,
    node: tree_sitter::Node<'a>,
    zipper: &LangZipper,
    out: &mut Vec<(tree_sitter::Node<'a>, LangZipper)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "quote" => {
                let qz = zipper.quote(regions::node_anno(text, child));
                out.push((child, qz.clone()));
                collect_quotes(text, child, &qz, out);
            }
            "unquote" => collect_quotes(text, child, &zipper.unquote(), out),
            _ => collect_quotes(text, child, zipper, out),
        }
    }
}

/// Byte window between a quote/unquote's brackets, `None` if malformed (mid-edit).
fn inner_window(node: tree_sitter::Node) -> Option<Range<usize>> {
    let count = node.child_count();
    if count < 2 {
        return None;
    }
    Some(node.child(0)?.end_byte()..node.child(count - 1)?.start_byte())
}

/// The language pair driving a ground projection: the host `meta` (how stage-0
/// splices and glyphs are reabsorbed) and `lang` (the syntax used for comment
/// translation). For v1 both resolve to the same Rust adapter.
struct Ground<'e> {
    meta: &'e dyn MetaLanguageAdapter,
    lang: &'e dyn LanguageAdapter,
}

/// Emit the ground (stage-0) projection over `window`. `stage` is the current
/// quasi-quote depth: at stage 0 we copy ground source; deeper we copy nothing
/// except stage-0 code reachable through `↙…↘` (which lowers the stage).
/// `zipper` resolves the language of each quote encountered (see
/// [`LangZipper`]); quotes are queued together with their resolved zipper.
#[allow(clippy::too_many_arguments)]
fn emit_ground<'a>(
    b: &mut Builder,
    text: &str,
    node: tree_sitter::Node<'a>,
    window: Range<usize>,
    stage: i32,
    env: &Ground,
    zipper: &LangZipper,
    quotes: &mut Vec<(tree_sitter::Node<'a>, LangZipper)>,
) {
    let mut pos = window.start;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.end_byte() <= window.start || child.start_byte() >= window.end {
            continue; // bracket token or outside the window
        }
        let start = child.start_byte().max(window.start);
        let end = child.end_byte().min(window.end);
        if stage == 0 && start > pos {
            // Ground gap. Comments are hidden in the quilt CST, so they surface
            // here between visible nodes; translate their glyphs (everything
            // else in a stage-0 gap is whitespace the comment regex absorbed).
            emit_comment_gap(b, pos, &text[pos..start], env.lang.comment_syntax());
        }
        match child.kind() {
            "quote" => {
                let qz = zipper.quote(regions::node_anno(text, child));
                if stage == 0 {
                    quotes.push((child, qz.clone())); // fragment candidate
                }
                let body = inner_window(child).unwrap_or(start..end);
                if stage == 0 {
                    // A quote in ground position becomes a splice block holding
                    // its stage-0 `↙…↘` bodies (in the quote's local scope).
                    let block = env.meta.splice_block();
                    b.synth(block.open);
                    emit_ground(b, text, child, body, 1, env, &qz, quotes);
                    b.synth(block.close);
                } else {
                    emit_ground(b, text, child, body, stage + 1, env, &qz, quotes);
                }
            }
            "unquote" => {
                let body = inner_window(child).unwrap_or(start..end);
                let inner = stage - 1;
                emit_ground(b, text, child, body, inner, env, &zipper.unquote(), quotes);
                if inner == 0 {
                    b.synth(env.meta.splice_block().terminator);
                }
            }
            "lift" | "reduce" | "emit" | "type" | "name" => {
                if stage == 0 {
                    b.synth(env.meta.glyph_placeholder());
                }
            }
            // Preserve line count: a newline inside a quote is emitted (synth,
            // not copied — the fragment owns those bytes) so ground code after a
            // multi-line quote keeps its line numbers.
            "newline" => {
                if stage == 0 {
                    b.copy(start, &text[start..end]);
                } else {
                    b.synth("\n");
                }
            }
            _ => {
                if stage == 0 {
                    b.copy(start, &text[start..end]);
                }
            }
        }
        pos = end;
    }
    if stage == 0 && pos < window.end {
        emit_comment_gap(b, pos, &text[pos..window.end], env.lang.comment_syntax());
    }
}

/// Emit a quoted fragment for highlighting: the embedded-language content is
/// copied; nested quotes/unquotes/glyphs are masked to `lang`'s placeholder
/// (nested quotes are also queued for their own fragment). `zipper` is the
/// fragment's own resolved zipper, used to resolve nested quote languages.
fn emit_fragment<'a>(
    b: &mut Builder,
    text: &str,
    node: tree_sitter::Node<'a>,
    window: Range<usize>,
    lang: &dyn LanguageAdapter,
    zipper: &LangZipper,
    quotes: &mut Vec<(tree_sitter::Node<'a>, LangZipper)>,
) {
    let mut pos = window.start;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.end_byte() <= window.start || child.start_byte() >= window.end {
            continue;
        }
        let start = child.start_byte().max(window.start);
        let end = child.end_byte().min(window.end);
        if start > pos {
            // Gap between embedded-language nodes: holds any hidden comments.
            emit_comment_gap(b, pos, &text[pos..start], lang.comment_syntax());
        }
        match child.kind() {
            "quote" => {
                mask_multiline(b, &text[start..end], lang.splice_placeholder());
                quotes.push((child, zipper.quote(regions::node_anno(text, child))));
            }
            "unquote" => mask_multiline(b, &text[start..end], lang.splice_placeholder()),
            "lift" | "reduce" | "emit" | "type" | "name" => b.synth(lang.splice_placeholder()),
            _ => b.copy(start, &text[start..end]),
        }
        pos = end;
    }
    if pos < window.end {
        emit_comment_gap(b, pos, &text[pos..window.end], lang.comment_syntax());
    }
}

/// Translate quilt comment glyphs in a gap to the host language's comment
/// syntax. Comments are *hidden* in the quilt CST, so they reach us as the text
/// between visible nodes; the only other thing a gap can hold is the whitespace
/// the comment regex absorbs (and, for a construct-free file, the whole body —
/// which has no glyphs, so it copies through unchanged, preserving identity).
///
/// We copy everything verbatim — preserving newlines, indentation, and the
/// comment *body* (so downstream comment tokens map back onto the source) — and
/// synthesize only the delimiters (`⟨//⟩`→`//`, `⟨/*⟩`→`/*`, `⟨*/⟩`→`*/`). A
/// tiny state machine avoids retranslating a `⟨/*⟩`-looking glyph that sits
/// *inside* a line/block comment, where it is mere comment text.
fn emit_comment_gap(b: &mut Builder, quilt_start: usize, gap: &str, cs: CommentSyntax) {
    const LINE: &str = "⟨//⟩";
    const BLOCK_OPEN: &str = "⟨/*⟩";
    const BLOCK_CLOSE: &str = "⟨*/⟩";

    enum State {
        /// Outside any comment: delimiters are structural.
        Ground,
        /// Inside `⟨//⟩…`: runs to end of line; inner glyphs are text.
        Line,
        /// Inside `⟨/*⟩…⟨*/⟩`: runs to the closer; inner glyphs are text.
        Block,
    }

    let mut state = State::Ground;
    let mut i = 0; // scan cursor (byte offset within `gap`)
    let mut run = 0; // start of the current verbatim run
    let flush = |b: &mut Builder, run: usize, i: usize| {
        if i > run {
            b.copy(quilt_start + run, &gap[run..i]);
        }
    };

    while i < gap.len() {
        match state {
            State::Ground if gap[i..].starts_with(LINE) => {
                flush(b, run, i);
                b.synth(cs.line);
                i += LINE.len();
                run = i;
                state = State::Line;
            }
            State::Ground if gap[i..].starts_with(BLOCK_OPEN) => {
                flush(b, run, i);
                b.synth(cs.block_open);
                i += BLOCK_OPEN.len();
                run = i;
                state = State::Block;
            }
            State::Block if gap[i..].starts_with(BLOCK_CLOSE) => {
                flush(b, run, i);
                b.synth(cs.block_close);
                i += BLOCK_CLOSE.len();
                run = i;
                state = State::Ground;
            }
            // A line comment ends at (and keeps) its newline.
            State::Line if gap.as_bytes()[i] == b'\n' => {
                i += 1;
                state = State::Ground;
            }
            // Any other byte is copied verbatim; advance a whole UTF-8 char.
            _ => i += utf8_len(gap.as_bytes()[i]),
        }
    }
    flush(b, run, gap.len());
}

/// Byte length of the UTF-8 char that starts with `first`.
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Replace a (possibly multi-line) construct with `placeholder`, preserving the
/// newlines it spanned so following line numbers stay put.
fn mask_multiline(b: &mut Builder, span: &str, placeholder: &str) {
    b.synth(placeholder);
    for _ in 0..span.bytes().filter(|&c| c == b'\n').count() {
        b.synth("\n");
    }
}

impl Projection {
    /// Quilt position → virtual position, or `None` if the position is not in
    /// this projection's language (i.e. inside a masked construct).
    pub fn to_virtual(
        &self,
        quilt_text: &str,
        quilt_index: &LineIndex,
        enc: Encoding,
        pos: Position,
    ) -> Option<Position> {
        let qoff = quilt_index.offset(quilt_text, pos, enc);
        let voff = self.map.quilt_to_virtual(qoff)?;
        Some(self.line_index.position(&self.text, voff, enc))
    }

    /// Virtual position → quilt position.
    pub fn to_quilt(
        &self,
        quilt_text: &str,
        quilt_index: &LineIndex,
        enc: Encoding,
        vpos: Position,
    ) -> Position {
        let voff = self.line_index.offset(&self.text, vpos, enc);
        let qoff = self.map.virtual_to_quilt(voff);
        quilt_index.position(quilt_text, qoff, enc)
    }

    /// Virtual range → quilt range.
    pub fn to_quilt_range(
        &self,
        quilt_text: &str,
        quilt_index: &LineIndex,
        enc: Encoding,
        r: LspRange,
    ) -> LspRange {
        LspRange {
            start: self.to_quilt(quilt_text, quilt_index, enc, r.start),
            end: self.to_quilt(quilt_text, quilt_index, enc, r.end),
        }
    }

    /// Whether a virtual range lies entirely within synthetic (placeholder)
    /// text — used to drop spurious downstream diagnostics on placeholders.
    pub fn is_synthetic(&self, enc: Encoding, r: LspRange) -> bool {
        let start = self.line_index.offset(&self.text, r.start, enc);
        let end = self.line_index.offset(&self.text, r.end, enc);
        // Synthetic iff both endpoints collapse to the same quilt anchor while
        // the virtual span is non-empty (a real copied span would advance the
        // quilt offset across its length).
        end > start && self.map.virtual_to_quilt(start) == self.map.virtual_to_quilt(end)
    }

    /// Whether a virtual range falls inside an appended quote fragment.
    pub fn is_in_fragment(&self, enc: Encoding, r: LspRange) -> bool {
        let start = self.line_index.offset(&self.text, r.start, enc);
        self.fragment_ranges
            .iter()
            .any(|fr| start >= fr.start && start < fr.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{language_adapter, meta_adapter};
    use crate::lineindex::Encoding;

    fn proj(src: &str) -> Projection {
        proj_chain(src, &["rs"])
    }

    fn proj_chain(src: &str, chain: &[&str]) -> Projection {
        project(
            src,
            meta_adapter("rs").unwrap(),
            language_adapter("rs").unwrap(),
            chain,
        )
    }

    #[test]
    fn pure_rust_is_identity() {
        let src = "fn main() {\n    println!(\"hi\");\n}\n";
        let p = proj(src);
        assert_eq!(
            p.text, src,
            "projection of construct-free source is identity"
        );
        // Every offset round-trips.
        for off in 0..=src.len() {
            assert_eq!(p.map.quilt_to_virtual(off), Some(off));
            assert_eq!(p.map.virtual_to_quilt(off), off);
        }
    }

    #[test]
    fn quote_becomes_splice_block_and_appended_fragment() {
        let src = "let x = ↖1 + 2↗;\n";
        let p = proj(src);
        // A quote with no `↙…↘` becomes an empty splice block in ground...
        assert!(p.text.starts_with("let x = { };\n"), "ground: {:?}", p.text);
        // ...and the quote body is appended in a wrapper fragment for tokenizing.
        assert!(p.text.contains("fn _quilt_q0()"), "fragment: {:?}", p.text);
        assert!(p.text.contains("1 + 2"));
        assert_eq!(p.fragment_ranges.len(), 1);
    }

    #[test]
    fn ground_splice_is_inlined_as_ground() {
        // The stage-0 `↙foo()↘` inside the quote is reabsorbed into ground code,
        // so the host server can resolve `foo` against the ground definition.
        let src = "fn f() { let p = ↖x ↙foo()↘ y↗; }\n";
        let p = proj(src);
        assert!(p.text.contains("{ foo(); }"), "ground: {:?}", p.text);

        // The `foo` inside `↙…↘` maps to a *ground* (non-fragment) position and
        // round-trips.
        let enc = Encoding::Utf16;
        let qi = LineIndex::new(src);
        let foo_byte = src.find("foo()").unwrap();
        let foo_q = qi.position(src, foo_byte, enc);
        let foo_v = p
            .to_virtual(src, &qi, enc, foo_q)
            .expect("foo maps into ground");
        let voff = p.line_index.offset(&p.text, foo_v, enc);
        assert!(
            !p.fragment_ranges
                .iter()
                .any(|fr| voff >= fr.start && voff < fr.end),
            "splice should be ground, not a fragment"
        );
        assert_eq!(p.to_quilt(src, &qi, enc, foo_v), foo_q);
    }

    #[test]
    fn ground_position_maps_through_quote() {
        let src = "let x = ↖1 + 2↗;\nfoo\n";
        let p = proj(src);
        let enc = Encoding::Utf16;
        let qi = LineIndex::new(src);
        let foo_q = Position {
            line: 1,
            character: 0,
        };
        let foo_v = p.to_virtual(src, &qi, enc, foo_q).unwrap();
        assert_eq!(
            foo_v,
            Position {
                line: 1,
                character: 0
            }
        );
        assert_eq!(p.to_quilt(src, &qi, enc, foo_v), foo_q);
    }

    #[test]
    fn position_inside_quote_maps_into_fragment() {
        let src = "let x = ↖1 + 2↗;\n";
        let p = proj(src);
        let enc = Encoding::Utf16;
        let qi = LineIndex::new(src);
        // `1` is stage-1 quoted content → maps into the appended fragment.
        let inside = Position {
            line: 0,
            character: 9,
        };
        let v = p
            .to_virtual(src, &qi, enc, inside)
            .expect("maps into fragment");
        let voff = p.line_index.offset(&p.text, v, enc);
        assert!(
            p.fragment_ranges
                .iter()
                .any(|fr| voff >= fr.start && voff < fr.end),
            "expected virtual offset {voff} within a fragment range {:?}",
            p.fragment_ranges
        );
        assert_eq!(p.to_quilt(src, &qi, enc, v), inside);
    }

    #[test]
    fn wgsl_quote_gets_no_fragment_but_keeps_splices() {
        // From `shaders.wgsl.rs.quilt`: an un-annotated quote is WGSL — its
        // body must not be appended as a Rust fragment (rust-analyzer would
        // choke on it), but its stage-0 splices are still reabsorbed as ground
        // Rust so `foo` resolves.
        let src = "fn f() { let p = ↖x ↙foo()↘ y↗; }\n";
        let p = proj_chain(src, &["rs", "wgsl"]);
        assert!(p.text.contains("{ foo(); }"), "ground: {:?}", p.text);
        assert!(
            p.fragment_ranges.is_empty(),
            "no Rust fragment for a WGSL quote"
        );
        assert!(!p.text.contains("_quilt_q"));
    }

    #[test]
    fn annotated_rust_quote_still_gets_fragment_in_wgsl_chain() {
        // An explicit `rust↖…↗` overrides the chain default and is appended.
        // It is the first fragment: the skipped WGSL quote leaves no gap.
        let src = "let a = ↖w↗; let b = rust↖1 + 2↗;\n";
        let p = proj_chain(src, &["rs", "wgsl"]);
        assert!(p.text.contains("fn _quilt_q0()"), "fragment: {:?}", p.text);
        assert!(p.text.contains("1 + 2"));
        assert!(!p.text.contains('w'), "WGSL body must not be appended");
        assert_eq!(p.fragment_ranges.len(), 1);
    }

    #[test]
    fn annotated_wgsl_quote_skipped_without_chain() {
        // Explicit `wgsl↖…↗` in a plain `.rs.quilt` file: same skip.
        let src = "let x = wgsl↖1u + 2u↗;\n";
        let p = proj(src);
        assert!(p.text.starts_with("let x = { };\n"), "ground: {:?}", p.text);
        assert!(p.fragment_ranges.is_empty());
    }

    #[test]
    fn multiline_quote_preserves_following_line_numbers() {
        let src = "let p = ↖{\n    a\n    b\n}↗;\nafter\n";
        let p = proj(src);
        let qi = LineIndex::new(src);
        let enc = Encoding::Utf16;
        // `after` is on quilt line 4; it must remain on virtual line 4.
        let after_q = Position {
            line: 4,
            character: 0,
        };
        let after_v = p.to_virtual(src, &qi, enc, after_q).unwrap();
        assert_eq!(after_v.line, 4);
    }

    #[test]
    fn line_comment_becomes_host_line_comment() {
        // `⟨//⟩` must not leak into the projection (it would be invalid Rust);
        // it becomes `//`, and the comment body (incl. stray glyphs like `↖`)
        // rides along as ordinary line-comment text. Indentation is preserved.
        let src = "fn f() {\n    ⟨//⟩ hi ↖\n    let x = 1;\n}\n";
        let p = proj(src);
        assert!(!p.text.contains('⟨'), "no quilt glyphs leak: {:?}", p.text);
        assert!(p.text.contains("\n    // hi ↖\n"), "ground: {:?}", p.text);
        // The comment line stays line 1; following code keeps its line number.
        assert_eq!(p.text.lines().nth(2), Some("    let x = 1;"));
    }

    #[test]
    fn block_comment_becomes_host_block_comment() {
        let src = "let y = ⟨/*⟩ blk\nmore ⟨*/⟩ 2;\n";
        let p = proj(src);
        assert!(!p.text.contains('⟨'), "no quilt glyphs leak: {:?}", p.text);
        assert!(p.text.contains("/* blk\nmore */"), "ground: {:?}", p.text);
        // Two source lines in, two lines out (newline inside the block kept).
        assert_eq!(p.text, "let y = /* blk\nmore */ 2;\n");
    }

    #[test]
    fn comment_body_maps_back_to_quilt() {
        let src = "x ⟨//⟩ note\n";
        let p = proj(src);
        let enc = Encoding::Utf16;
        let qi = LineIndex::new(src);
        // A position on `note` (comment body) round-trips through the projection.
        let q = qi.position(src, src.find("note").unwrap(), enc);
        let v = p
            .to_virtual(src, &qi, enc, q)
            .expect("comment body is copied");
        assert_eq!(p.to_quilt(src, &qi, enc, v), q);
    }

    #[test]
    fn adjacent_line_comments_each_translated() {
        // Two stacked line comments arrive as one hidden gap (the second eats the
        // newline); both delimiters must be translated.
        let src = "⟨//⟩ a\n⟨//⟩ b\nfn f() {}\n";
        let p = proj(src);
        assert!(!p.text.contains('⟨'), "no glyphs leak: {:?}", p.text);
        assert!(p.text.starts_with("// a\n// b\n"), "ground: {:?}", p.text);
        // `fn f` stays on line 2.
        assert_eq!(p.text.lines().nth(2), Some("fn f() {}"));
    }

    #[test]
    fn comment_inside_quote_is_translated_in_fragment() {
        // A comment inside a quoted fragment must also be translated so the
        // fragment tokenizes cleanly. (A block comment keeps the closing `↗` off
        // the comment; a same-line `⟨//⟩` would absorb the `↗`, as in any
        // language where `//` runs to end of line.)
        let src = "let p = ↖a ⟨/*⟩ c ⟨*/⟩ b↗;\n";
        let p = proj(src);
        assert!(!p.text.contains('⟨'), "no glyphs leak: {:?}", p.text);
        assert!(p.text.contains("a /* c */ b"), "fragment: {:?}", p.text);
    }

    #[test]
    #[cfg(feature = "python")]
    fn python_ground_projection_is_valid_python() {
        let meta = meta_adapter("py").unwrap();
        let lang = language_adapter("py").unwrap();
        let src = "expr = ↖1 + 2↗\nscaled = ↖↙ten↘ * 100↗\n";
        let p = project(src, meta, lang, &["py"]);
        // A quote with no ground splices → empty-tuple placeholder.
        assert!(p.text.contains("expr = ()"), "ground: {:?}", p.text);
        // A stage-0 `↙ten↘` splice is reabsorbed as a ground tuple referencing
        // `ten`, so pyright resolves it against the ground binding.
        assert!(p.text.contains("scaled = (ten, )"), "ground: {:?}", p.text);
        // Each Python quote is appended as a parenthesized fragment for tokens.
        assert!(p.text.contains("_quilt_q0 = ("), "fragment: {:?}", p.text);
        assert_eq!(p.fragment_ranges.len(), 2);
    }

    #[test]
    #[cfg(feature = "wgsl")]
    fn wgsl_fragments_each_project_to_their_own_module() {
        let lang = language_adapter("wgsl").unwrap();
        let src = "fn a() -> String { wgsl↖const W: u32 = ↙w.↑↘;↗.coparse() }\n\
                   fn b() -> String { wgsl↖fn main() {}↗.coparse() }\n";
        let frags = project_fragments(src, lang, &["rs", "wgsl"]);
        assert_eq!(
            frags.len(),
            2,
            "two WGSL quotes → two independent fragments"
        );
        // The Rust splice `↙w.↑↘` is masked to a WGSL value placeholder.
        assert!(
            frags[0].proj.text.contains("const W: u32 = 0;"),
            "frag0: {:?}",
            frags[0].proj.text
        );
        assert!(
            frags[1].proj.text.contains("fn main() {}"),
            "frag1: {:?}",
            frags[1].proj.text
        );
        // Quilt range is non-empty (used to route a cursor into the fragment).
        assert!(frags[0].quilt_range.start < frags[0].quilt_range.end);

        // A position on `W` inside the first fragment round-trips quilt↔virtual.
        let enc = Encoding::Utf16;
        let qi = LineIndex::new(src);
        let w_byte = src.find("const W").unwrap() + "const ".len();
        let w_q = qi.position(src, w_byte, enc);
        let w_v = frags[0]
            .proj
            .to_virtual(src, &qi, enc, w_q)
            .expect("W maps into the fragment");
        assert_eq!(frags[0].proj.to_quilt(src, &qi, enc, w_v), w_q);
    }

    #[test]
    #[cfg(feature = "python")]
    fn python_shebang_is_not_rewritten() {
        let meta = meta_adapter("py").unwrap();
        let lang = language_adapter("py").unwrap();
        let src = "#!/usr/bin/env quilt\nx = ↖1↗\n";
        let p = project(src, meta, lang, &["py"]);
        // `#!` is already a comment in Python — it must stay, not become `//`.
        assert!(
            p.text.starts_with("#!/usr/bin/env"),
            "shebang: {:?}",
            p.text
        );
    }

    #[test]
    fn shebang_becomes_comment() {
        // `#!/usr/bin/env quilt` is valid for `quilt` scripts but
        // a parse error in Rust (inner attribute syntax). It must become `//`
        // preserving all byte positions.
        let src = "#!/usr/bin/env quilt\nfn main() {}\n";
        let p = proj(src);
        assert!(p.text.starts_with("//"), "shebang → comment: {:?}", p.text);
        assert_eq!(&p.text[..2], "//", "first two bytes are //");
        assert_eq!(&p.text[2..], &src[2..], "rest of text is unchanged");
        assert_eq!(p.text.len(), src.len(), "byte length preserved");
        // Positions are identity-mapped.
        assert_eq!(p.map.quilt_to_virtual(0), Some(0));
        assert_eq!(p.map.quilt_to_virtual(2), Some(2));
    }

    #[test]
    fn sky_param_holes_are_listed_in_order() {
        // Bare-identifier `↙name↘` holes are the template's parameters, in
        // first-seen order with duplicates removed; a richer host expression is
        // not a parameter.
        let src = "GREETING = ↙greeting↘\nAUDIENCE = ↙names↘\nX = ↙greeting↘\nY = ↙a + b↘\n";
        let names: Vec<String> = sky_param_holes(src).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["greeting", "names"]);
    }

    #[test]
    #[cfg(feature = "html")]
    fn sky_template_projects_body_as_target_language() {
        // An `index.html.tmpl.quilt` body is HTML with `↙title↘` parameter holes;
        // it projects as one HTML document with each hole masked to a placeholder.
        let lang = language_adapter("html").unwrap();
        let src = "<head><title>↙title↘</title></head>\n";
        let p = project_sky(src, lang, &["html"]);
        assert!(
            p.text
                .contains(&format!("<title>{}</title>", lang.splice_placeholder())),
            "masked body: {:?}",
            p.text
        );
        assert!(!p.text.contains('↙'), "no quilt glyphs leak: {:?}", p.text);
        // A position on `head` (target-language text) round-trips quilt↔virtual.
        let enc = Encoding::Utf16;
        let qi = LineIndex::new(src);
        let head_q = qi.position(src, src.find("head").unwrap(), enc);
        let head_v = p.to_virtual(src, &qi, enc, head_q).expect("head maps in");
        assert_eq!(p.to_quilt(src, &qi, enc, head_v), head_q);
    }

    #[test]
    #[cfg(feature = "python")]
    fn sky_template_python_body_is_valid_python() {
        // A `greeting.py.tmpl.quilt` body projects to valid Python: the holes
        // become `__q__` placeholders, and the rest (incl. a `#` comment line)
        // copies through verbatim so the body still parses as a Python module.
        let lang = language_adapter("py").unwrap();
        let src = "# greet\nGREETING = ↙greeting↘\nAUDIENCE = ↙names↘\n";
        let p = project_sky(src, lang, &["py"]);
        assert_eq!(p.text, "# greet\nGREETING = __q__\nAUDIENCE = __q__\n");
        // A position on `GREETING` (target-language text) round-trips both ways.
        let enc = Encoding::Utf16;
        let qi = LineIndex::new(src);
        let g_q = qi.position(src, src.find("GREETING").unwrap(), enc);
        let g_v = p.to_virtual(src, &qi, enc, g_q).expect("GREETING maps in");
        assert_eq!(p.to_quilt(src, &qi, enc, g_v), g_q);
    }
}
