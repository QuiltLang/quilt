//! Tree-sitter semantic tokens for embedded-language fragments.
//!
//! Embedded target languages may have no downstream semantic-token support at
//! all (wgsl-analyzer advertises no `semanticTokensProvider`; see
//! <https://github.com/wgsl-analyzer/wgsl-analyzer/issues/342>), so the server
//! highlights their quote bodies itself, with the same tree-sitter grammars
//! quilt uses for parsing. Each fragment's standalone projection is parsed and
//! run through the grammar's highlight query; capture names are mapped to LSP
//! standard token types, spans are mapped back to quilt coordinates through
//! the fragment's source map, and the result is merged with the remapped
//! downstream tokens (see `Inner::semantic_tokens`).
//!
//! The token *type indices* come from the legend extension performed at
//! registration: the downstream server's legend is advertised with any of
//! [`TOKEN_TYPES`] it lacks appended at the end, so downstream indices stay
//! valid and ours resolve by name.

use crate::lineindex::{Encoding, LineIndex};
use crate::projection::Projection;
use crate::semtok::Tok;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use streaming_iterator::StreamingIterator;

/// LSP standard token types the capture mapping can emit; the advertised
/// legend is extended with whichever of these the downstream legend lacks.
pub const TOKEN_TYPES: &[&str] = &[
    "comment",
    "decorator",
    "function",
    "keyword",
    "macro",
    "namespace",
    "number",
    "operator",
    "parameter",
    "property",
    "string",
    "struct",
    "type",
    "variable",
];

/// Map a highlight-query capture name (nvim-style, e.g. `keyword.function`,
/// `type.builtin`) to an LSP standard token type. `None` drops the capture:
/// punctuation (themes rarely color it; the `TextMate` layer already provides
/// a base) and `error` (the quilt/downstream diagnostics own error reporting).
fn lsp_token_type(capture: &str) -> Option<&'static str> {
    match capture.split('.').next().unwrap_or(capture) {
        "number" | "float" => Some("number"),
        "boolean" | "keyword" | "repeat" | "conditional" | "storageclass" => Some("keyword"),
        "type" => Some("type"),
        "function" | "method" => Some("function"),
        "parameter" => Some("parameter"),
        "structure" | "struct" => Some("struct"),
        "field" | "property" => Some("property"),
        "attribute" => Some("decorator"),
        "constant" | "variable" => Some("variable"),
        "operator" => Some("operator"),
        "comment" => Some("comment"),
        "string" => Some("string"),
        "namespace" | "module" => Some("namespace"),
        "macro" => Some("macro"),
        _ => None, // punctuation.*, error, …
    }
}

/// A compiled grammar + highlight query for one embedded language.
pub struct Highlighter {
    language: tree_sitter::Language,
    query: tree_sitter::Query,
    /// Query capture index → LSP token type (`None`: dropped capture).
    capture_types: Vec<Option<&'static str>>,
}

/// The highlighter for a downstream `languageId`, if one is compiled in.
/// Compiled once on first use; a query that fails to compile (grammar drift)
/// logs and disables itself rather than failing requests.
pub fn highlighter(lang_id: &str) -> Option<&'static Highlighter> {
    match lang_id {
        #[cfg(feature = "wgsl")]
        "wgsl" => {
            static WGSL: std::sync::OnceLock<Option<Highlighter>> = std::sync::OnceLock::new();
            WGSL.get_or_init(|| {
                Highlighter::new(
                    tree_sitter_wgsl::LANGUAGE.into(),
                    include_str!("../queries/wgsl-highlights.scm"),
                )
            })
            .as_ref()
        }
        _ => None,
    }
}

impl Highlighter {
    fn new(language: tree_sitter::Language, query_src: &str) -> Option<Self> {
        let query = match tree_sitter::Query::new(&language, query_src) {
            Ok(q) => q,
            Err(e) => {
                tracing::warn!("highlight query failed to compile: {e}");
                return None;
            }
        };
        let capture_types = query
            .capture_names()
            .iter()
            .map(|n| lsp_token_type(n))
            .collect();
        Some(Self {
            language,
            query,
            capture_types,
        })
    }

    /// Highlight `text`, returning non-overlapping `(byte_range, token_type)`
    /// spans in document order. Same-range captures resolve to the *earliest*
    /// pattern in the query file (tree-sitter convention: specific patterns
    /// precede the `(identifier) @variable` catch-all); across nested ranges
    /// the narrower span wins (leaf tokens shadow container captures like
    /// `(type_declaration) @type`).
    pub fn spans(&self, text: &str) -> Vec<(Range<usize>, &'static str)> {
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&self.language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(text, None) else {
            return Vec::new();
        };

        // (start, end) → (pattern index, type): same-range, first pattern wins.
        let mut by_range: HashMap<(usize, usize), (usize, &'static str)> = HashMap::new();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), text.as_bytes());
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let Some(ty) = self.capture_types[cap.index as usize] else {
                    continue;
                };
                let r = cap.node.byte_range();
                if r.is_empty() {
                    continue;
                }
                let key = (r.start, r.end);
                let better = by_range
                    .get(&key)
                    .is_none_or(|&(pi, _)| m.pattern_index < pi);
                if better {
                    by_range.insert(key, (m.pattern_index, ty));
                }
            }
        }

        // Narrowest-first greedy insertion: leaf tokens land, then any
        // container span overlapping one is dropped.
        let mut spans: Vec<_> = by_range.into_iter().collect();
        spans.sort_by_key(|&((s, e), (pi, _))| (e - s, pi, s));
        let mut occupied: BTreeMap<usize, usize> = BTreeMap::new(); // start → end
        let mut out = Vec::new();
        for ((s, e), (_, ty)) in spans {
            // `occupied` holds disjoint intervals, so only the nearest one
            // starting below `e` can overlap `s..e`.
            let overlaps = occupied
                .range(..e)
                .next_back()
                .is_some_and(|(_, &prev_end)| prev_end > s);
            if overlaps {
                continue;
            }
            occupied.insert(s, e);
            out.push((s..e, ty));
        }
        out.sort_by_key(|(r, _)| r.start);
        out
    }
}

/// Semantic tokens for one embedded fragment, in absolute quilt coordinates.
/// Spans are split per line (LSP tokens are single-line unless the client
/// opts into multiline support) and mapped through the fragment's source map;
/// pieces that collapse into masked splices are dropped.
#[allow(clippy::implicit_hasher)] // one internal call site, default hasher
pub fn fragment_tokens(
    hl: &Highlighter,
    proj: &Projection,
    quilt_text: &str,
    quilt_index: &LineIndex,
    enc: Encoding,
    type_index: &HashMap<&'static str, u32>,
) -> Vec<Tok> {
    let mut out = Vec::new();
    for (range, ty) in hl.spans(&proj.text) {
        let Some(&ty_idx) = type_index.get(ty) else {
            continue;
        };
        let mut start = range.start;
        while start < range.end {
            let end = proj.text[start..range.end]
                .find('\n')
                .map_or(range.end, |i| start + i);
            push_tok(
                &mut out,
                proj,
                quilt_text,
                quilt_index,
                enc,
                start..end,
                ty_idx,
            );
            start = end + 1;
        }
    }
    out
}

/// Map one single-line virtual span to a quilt token, dropping synthetic and
/// line-straddling spans (same rules as the downstream remap).
fn push_tok(
    out: &mut Vec<Tok>,
    proj: &Projection,
    quilt_text: &str,
    quilt_index: &LineIndex,
    enc: Encoding,
    r: Range<usize>,
    ty: u32,
) {
    if r.is_empty() {
        return;
    }
    let q_start = proj.map.virtual_to_quilt(r.start);
    let q_end = proj.map.virtual_to_quilt(r.end);
    if q_end <= q_start {
        return; // synthetic: a masked splice placeholder
    }
    let qs = quilt_index.position(quilt_text, q_start, enc);
    let qe = quilt_index.position(quilt_text, q_end, enc);
    if qs.line != qe.line || qe.character <= qs.character {
        return;
    }
    out.push(Tok {
        line: qs.line,
        start: qs.character,
        length: qe.character - qs.character,
        ty,
        modifiers: 0,
    });
}

#[cfg(test)]
#[cfg(feature = "wgsl")]
mod tests {
    use super::*;
    use crate::adapters::language_adapter;
    use crate::projection::project_fragments;

    fn wgsl() -> &'static Highlighter {
        highlighter("wgsl").expect("wgsl highlight query compiles against the pinned grammar")
    }

    fn span_text<'t>(
        text: &'t str,
        spans: &[(Range<usize>, &'static str)],
    ) -> Vec<(&'t str, &'static str)> {
        spans
            .iter()
            .map(|(r, ty)| (&text[r.clone()], *ty))
            .collect()
    }

    #[test]
    fn highlights_basic_wgsl() {
        let src = "fn main() { let x = 3u; }";
        let got = span_text(src, &wgsl().spans(src));
        assert!(got.contains(&("fn", "keyword")), "{got:?}");
        assert!(got.contains(&("let", "keyword")), "{got:?}");
        assert!(got.contains(&("3u", "number")), "{got:?}");
        // First-pattern-wins: `main` is captured by the function pattern, not
        // the later `(identifier) @variable` catch-all.
        assert!(got.contains(&("main", "function")), "{got:?}");
        assert!(!got.contains(&("main", "variable")), "{got:?}");
    }

    #[test]
    fn leaf_tokens_shadow_container_captures() {
        // `(type_declaration) @type` captures `array<u32>` wholesale, but the
        // inner `u32` builtin (and the `<`/`>` operators) are narrower captures
        // and must win; the container span is dropped, not emitted overlapping.
        let src = "var g: array<u32>;";
        let spans = wgsl().spans(src);
        let got = span_text(src, &spans);
        assert!(got.contains(&("u32", "type")), "{got:?}");
        assert!(!got.iter().any(|(t, _)| *t == "array<u32>"), "{got:?}");
        // Non-overlapping invariant.
        for w in spans.windows(2) {
            assert!(w[0].0.end <= w[1].0.start, "overlap: {spans:?}");
        }
    }

    #[test]
    fn fragment_tokens_map_to_quilt_and_skip_splices() {
        // A WGSL quote with a Rust splice: tokens land on the WGSL text in
        // quilt coordinates; nothing is emitted over the masked `↙…↘`.
        let src = "fn f() -> String { wgsl↖const W: u32 = ↙w.↑↘;↗.coparse() }\n";
        let lang = language_adapter("wgsl").unwrap();
        let frags = project_fragments(src, lang, &["rs", "wgsl"]);
        assert_eq!(frags.len(), 1);

        let type_index: HashMap<&'static str, u32> = TOKEN_TYPES
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, u32::try_from(i).unwrap()))
            .collect();
        let qi = LineIndex::new(src);
        let toks = fragment_tokens(
            wgsl(),
            &frags[0].proj,
            src,
            &qi,
            Encoding::Utf16,
            &type_index,
        );
        assert!(!toks.is_empty());

        // Every token lies within the quote body; reconstruct its text.
        let line: Vec<char> = src.lines().next().unwrap().chars().collect();
        let texts: Vec<String> = toks
            .iter()
            .map(|t| {
                assert_eq!(t.line, 0);
                line[t.start as usize..(t.start + t.length) as usize]
                    .iter()
                    .collect()
            })
            .collect();
        assert!(texts.iter().any(|t| t == "const"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "W"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "u32"), "{texts:?}");
        // The masked splice (placeholder `0`) must produce no token: nothing
        // mapped onto the `↙w.↑↘` glyph span.
        assert!(
            !texts
                .iter()
                .any(|t| t.contains('↙') || t.contains('w') && t.len() == 1),
            "{texts:?}"
        );
    }
}
