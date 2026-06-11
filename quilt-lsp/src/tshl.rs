//! Tree-sitter semantic tokens for languages whose downstream server provides
//! none.
//!
//! Two consumers, same mechanism:
//!
//! * **Embedded fragments** — embedded target languages may have no downstream
//!   semantic-token support at all (wgsl-analyzer advertises no
//!   `semanticTokensProvider`; see
//!   <https://github.com/wgsl-analyzer/wgsl-analyzer/issues/342>), so the
//!   server highlights their quote bodies itself.
//! * **The ground projection** — a *host* language's downstream server may
//!   equally lack semantic tokens (pyright does; that feature is
//!   Pylance-only), so the whole ground projection of a `.py.quilt` file is
//!   highlighted the same way as a fallback (see `Inner::semantic_tokens`).
//!
//! Either way the projection is parsed with the same tree-sitter grammars
//! quilt uses for parsing and run through the grammar's highlight query;
//! capture names are mapped to LSP standard token types, spans are mapped back
//! to quilt coordinates through the projection's source map, and the result is
//! merged with any remapped downstream tokens.
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
/// a base), `error` (the quilt/downstream diagnostics own error reporting),
/// and `embedded` (a container around `$(…)`-style substitutions whose
/// contents carry their own captures).
fn lsp_token_type(capture: &str) -> Option<&'static str> {
    match capture.split('.').next().unwrap_or(capture) {
        "number" | "float" => Some("number"),
        "boolean" | "keyword" | "repeat" | "conditional" | "storageclass" => Some("keyword"),
        // HTML element names color like types (the TextMate `entity.name.tag`
        // convention); LSP has no dedicated tag token type.
        "type" | "constructor" | "tag" => Some("type"),
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

/// Which pattern wins when several capture the same node. Highlight queries
/// come in two orderings: nvim-flavored files put specific patterns before the
/// `(identifier) @variable` catch-all (first wins); upstream tree-sitter files
/// put the catch-all first and let later patterns override (last wins).
#[derive(Clone, Copy, PartialEq)]
enum Order {
    FirstWins,
    LastWins,
}

/// A compiled grammar + highlight query for one language.
pub struct Highlighter {
    language: tree_sitter::Language,
    query: tree_sitter::Query,
    /// Query capture index → LSP token type (`None`: dropped capture).
    capture_types: Vec<Option<&'static str>>,
    order: Order,
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
                // The vendored query is nvim-flavored (specific patterns first).
                Highlighter::new(
                    tree_sitter_wgsl::LANGUAGE.into(),
                    include_str!("../queries/wgsl-highlights.scm"),
                    Order::FirstWins,
                )
            })
            .as_ref()
        }
        #[cfg(feature = "python")]
        "python" => {
            static PYTHON: std::sync::OnceLock<Option<Highlighter>> = std::sync::OnceLock::new();
            PYTHON
                .get_or_init(|| {
                    // The grammar fork's own query is upstream-flavored: it opens
                    // with the `(identifier) @variable` catch-all and overrides
                    // with later, more specific patterns.
                    Highlighter::new(
                        tree_sitter_python::LANGUAGE.into(),
                        tree_sitter_python::HIGHLIGHTS_QUERY,
                        Order::LastWins,
                    )
                })
                .as_ref()
        }
        #[cfg(feature = "html")]
        "html" => {
            static HTML: std::sync::OnceLock<Option<Highlighter>> = std::sync::OnceLock::new();
            HTML.get_or_init(|| {
                // The fork ships upstream tree-sitter-html's own query
                // (upstream-flavored; one pattern per node, so order is moot).
                Highlighter::new(
                    tree_sitter_html::LANGUAGE.into(),
                    tree_sitter_html::HIGHLIGHTS_QUERY,
                    Order::LastWins,
                )
            })
            .as_ref()
        }
        #[cfg(feature = "bash")]
        "bash" => {
            static BASH: std::sync::OnceLock<Option<Highlighter>> = std::sync::OnceLock::new();
            BASH.get_or_init(|| {
                // The fork ships upstream tree-sitter-bash's own query
                // (upstream-flavored: later patterns override).
                Highlighter::new(
                    tree_sitter_bash::LANGUAGE.into(),
                    tree_sitter_bash::HIGHLIGHT_QUERY,
                    Order::LastWins,
                )
            })
            .as_ref()
        }
        #[cfg(feature = "zsh")]
        "zsh" => {
            static ZSH: std::sync::OnceLock<Option<Highlighter>> = std::sync::OnceLock::new();
            ZSH.get_or_init(|| {
                // Same shape as the bash query (the zsh grammar forked it).
                Highlighter::new(
                    tree_sitter_zsh::LANGUAGE.into(),
                    tree_sitter_zsh::HIGHLIGHT_QUERY,
                    Order::LastWins,
                )
            })
            .as_ref()
        }
        _ => None,
    }
}

impl Highlighter {
    fn new(language: tree_sitter::Language, query_src: &str, order: Order) -> Option<Self> {
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
            order,
        })
    }

    /// Pattern-priority key: lower wins. Identity for first-wins queries,
    /// inverted for last-wins ones.
    fn priority(&self, pattern_index: usize) -> usize {
        match self.order {
            Order::FirstWins => pattern_index,
            Order::LastWins => usize::MAX - pattern_index,
        }
    }

    /// Highlight `text`, returning non-overlapping `(byte_range, token_type)`
    /// spans in document order. Same-range captures resolve per the query's
    /// [`Order`] (e.g. for first-wins, specific patterns precede the
    /// `(identifier) @variable` catch-all); across nested ranges the narrower
    /// span wins (leaf tokens shadow container captures like
    /// `(type_declaration) @type`).
    pub fn spans(&self, text: &str) -> Vec<(Range<usize>, &'static str)> {
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&self.language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(text, None) else {
            return Vec::new();
        };

        // (start, end) → (priority, type): same-range, lowest priority wins.
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
                let prio = self.priority(m.pattern_index);
                let better = by_range.get(&key).is_none_or(|&(p, _)| prio < p);
                if better {
                    by_range.insert(key, (prio, ty));
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

/// Semantic tokens for one projection — an embedded fragment's standalone
/// document or a host's whole ground projection — in absolute quilt
/// coordinates. Spans are split per line (LSP tokens are single-line unless
/// the client opts into multiline support) and mapped through the projection's
/// source map; pieces that collapse into masked splices or synthetic wrappers
/// are dropped.
#[allow(clippy::implicit_hasher)] // internal call sites only, default hasher
pub fn projection_tokens(
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
mod tests {
    use super::*;
    #[cfg(any(
        feature = "wgsl",
        feature = "python",
        feature = "html",
        feature = "bash",
        feature = "zsh"
    ))]
    use crate::adapters::language_adapter;
    #[cfg(any(feature = "wgsl", feature = "bash", feature = "html"))]
    use crate::projection::project_fragments;

    #[cfg(feature = "wgsl")]
    fn wgsl() -> &'static Highlighter {
        highlighter("wgsl").expect("wgsl highlight query compiles against the pinned grammar")
    }

    #[allow(dead_code)] // used only by the feature-gated tests
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
    #[cfg(feature = "wgsl")]
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
    #[cfg(feature = "wgsl")]
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
    #[cfg(feature = "wgsl")]
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
        let toks = projection_tokens(
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

    #[cfg(feature = "python")]
    fn python() -> &'static Highlighter {
        highlighter("python").expect("python highlight query compiles against the pinned grammar")
    }

    #[test]
    #[cfg(feature = "python")]
    fn highlights_basic_python() {
        let src = "def greet(name):\n    return MAX + len(name)\n";
        let got = span_text(src, &python().spans(src));
        assert!(got.contains(&("def", "keyword")), "{got:?}");
        assert!(got.contains(&("return", "keyword")), "{got:?}");
        // Last-pattern-wins: the query opens with the `(identifier) @variable`
        // catch-all, so `greet` and `len` must resolve to the later, more
        // specific function patterns, not to "variable".
        assert!(got.contains(&("greet", "function")), "{got:?}");
        assert!(got.contains(&("len", "function")), "{got:?}");
        assert!(!got.contains(&("greet", "variable")), "{got:?}");
        // `(#match? …)` predicates are honored: SCREAMING_CASE matches the
        // @constant pattern (→ "variable"), and an ordinary lowercase name
        // does not get promoted by it.
        assert!(got.contains(&("MAX", "variable")), "{got:?}");
        assert!(got.contains(&("name", "variable")), "{got:?}");
    }

    #[test]
    #[cfg(feature = "python")]
    fn python_ground_projection_tokens_map_to_quilt() {
        use crate::adapters::meta_adapter;
        use crate::projection::project;

        // A `.py.quilt` ground with one quote: tokens must cover the ground
        // Python *and* the quote body (appended as a fragment in the same
        // projection), all in quilt coordinates, with nothing on the glyphs.
        let src = "def f(x):\n    return x\n\nq = ↖1 + 2↗\n";
        let meta = meta_adapter("py").unwrap();
        let lang = language_adapter("py").unwrap();
        let proj = project(src, meta, lang, &["py"]);

        let type_index: HashMap<&'static str, u32> = TOKEN_TYPES
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, u32::try_from(i).unwrap()))
            .collect();
        let qi = LineIndex::new(src);
        let toks = projection_tokens(python(), &proj, src, &qi, Encoding::Utf16, &type_index);

        let lines: Vec<Vec<char>> = src.lines().map(|l| l.chars().collect()).collect();
        let texts: Vec<(u32, String)> = toks
            .iter()
            .map(|t| {
                let line = &lines[t.line as usize];
                let s: String = line[t.start as usize..(t.start + t.length) as usize]
                    .iter()
                    .collect();
                (t.line, s)
            })
            .collect();
        // Ground tokens.
        assert!(texts.contains(&(0, "def".to_string())), "{texts:?}");
        assert!(texts.contains(&(1, "return".to_string())), "{texts:?}");
        // Quote-body tokens land back inside `↖1 + 2↗` on line 3.
        assert!(texts.contains(&(3, "1".to_string())), "{texts:?}");
        assert!(texts.contains(&(3, "2".to_string())), "{texts:?}");
        // Nothing maps onto the quote glyphs or the synthetic `()` placeholder.
        assert!(
            !texts
                .iter()
                .any(|(_, s)| s.contains('↖') || s.contains('↗')),
            "{texts:?}"
        );
    }

    #[test]
    #[cfg(feature = "html")]
    fn highlights_basic_html() {
        let src = "<p class=\"intro\">hi</p><!-- note -->";
        let got = span_text(
            src,
            &highlighter("html").expect("html query compiles").spans(src),
        );
        // `@tag` is the capture-audit addition: element names map to "type".
        assert!(got.contains(&("p", "type")), "{got:?}");
        assert!(got.contains(&("class", "decorator")), "{got:?}");
        assert!(got.contains(&("intro", "string")), "{got:?}");
        assert!(got.contains(&("<!-- note -->", "comment")), "{got:?}");
        // `@punctuation.bracket` (`<`, `>`, `</`) stays dropped.
        assert!(!got.iter().any(|(t, _)| *t == "<"), "{got:?}");
    }

    #[test]
    #[cfg(feature = "bash")]
    fn highlights_basic_bash() {
        let src = "if true; then echo \"hi\" $(ls -l); fi # done\n";
        let got = span_text(
            src,
            &highlighter("bash").expect("bash query compiles").spans(src),
        );
        assert!(got.contains(&("if", "keyword")), "{got:?}");
        assert!(got.contains(&("then", "keyword")), "{got:?}");
        assert!(got.contains(&("echo", "function")), "{got:?}");
        assert!(got.contains(&("\"hi\"", "string")), "{got:?}");
        assert!(got.contains(&("# done", "comment")), "{got:?}");
        // Inside the `@embedded` substitution (dropped container), the inner
        // command and its `-` flag (`@constant`, `#match?`-gated) still land.
        assert!(got.contains(&("ls", "function")), "{got:?}");
        assert!(got.contains(&("-l", "variable")), "{got:?}");
        assert!(!got.iter().any(|(t, _)| t.starts_with("$(")), "{got:?}");
    }

    #[test]
    #[cfg(feature = "zsh")]
    fn highlights_basic_zsh() {
        let src = "for f in a b; do tar -czf \"$f.tar.gz\" $f; done # all\n";
        let got = span_text(
            src,
            &highlighter("zsh").expect("zsh query compiles").spans(src),
        );
        assert!(got.contains(&("for", "keyword")), "{got:?}");
        assert!(got.contains(&("do", "keyword")), "{got:?}");
        assert!(got.contains(&("done", "keyword")), "{got:?}");
        assert!(got.contains(&("tar", "function")), "{got:?}");
        assert!(got.contains(&("-czf", "variable")), "{got:?}");
        assert!(got.contains(&("# all", "comment")), "{got:?}");
    }

    #[test]
    #[cfg(feature = "bash")]
    fn bash_fragment_tokens_map_to_quilt_and_skip_splices() {
        // A bash quote with Rust splices (the `bash_backup.rs.quilt` shape):
        // tokens land on the bash text in quilt coordinates; nothing is
        // emitted over the masked `↙…↘` splices.
        let src = "fn f() -> String { bash↖tar -czf ↙archive.↑↘ /etc↗.coparse() }\n";
        let lang = language_adapter("bash").unwrap();
        let frags = project_fragments(src, lang, &["rs"]);
        assert_eq!(frags.len(), 1);

        let type_index: HashMap<&'static str, u32> = TOKEN_TYPES
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, u32::try_from(i).unwrap()))
            .collect();
        let qi = LineIndex::new(src);
        let toks = projection_tokens(
            highlighter("bash").unwrap(),
            &frags[0].proj,
            src,
            &qi,
            Encoding::Utf16,
            &type_index,
        );
        assert!(!toks.is_empty());

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
        assert!(texts.iter().any(|t| t == "tar"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "-czf"), "{texts:?}");
        // The masked splice (placeholder `__q__`) must produce no token:
        // nothing mapped onto the `↙archive.↑↘` glyph span.
        assert!(
            !texts
                .iter()
                .any(|t| t.contains('↙') || t.contains("archive")),
            "{texts:?}"
        );
    }
}
