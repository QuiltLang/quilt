//! What the Quilt surface parser does, snapshotted.
//!
//! Until issue #254 there were two parsers — the hand-written one and
//! `tree-sitter-quilt` — and a differential test held them to
//! producing identical `Node` trees over this corpus. That is what the corpus
//! is *for*, and it earned its keep: the sweep below caught two real bugs in
//! the scanner before either parser shipped, both about a `⟨/*⟩` comment body
//! eating the `⟨` that would have closed it.
//!
//! The grammar is gone now, so there is no second parser to agree with. What
//! survives is the corpus and the answers, frozen: these snapshots were taken
//! while both parsers still existed and still agreed. "The parser still does
//! what it did on the day it was checked against the grammar" is a weaker
//! claim than "two independent implementations agree", and it is the strongest
//! one available with a single implementation — so it is worth keeping rather
//! than losing along with the grammar.
//!
//! The invariant tests below are not snapshots and are stronger for it: they
//! hold over generated input, not just the recorded cases.

use quilt::node::{Node, TokenKind};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Render one case the way the snapshot records it: the source, then the
/// parse — the tree, or the diagnostic if it did not parse.
fn render(src: &str) -> String {
    let mut out = String::new();
    writeln!(out, "--- {src:?}").expect("write");
    match Node::parse(src) {
        Ok(nodes) => writeln!(out, "    {nodes:?}").expect("write"),
        Err(e) => {
            let labels: Vec<_> = e
                .labels()
                .into_iter()
                .flatten()
                .map(|l| {
                    format!(
                        "{}..{} {:?}",
                        l.offset(),
                        l.offset() + l.len(),
                        l.label().unwrap_or("")
                    )
                })
                .collect();
            writeln!(out, "    ERR {e} [{}]", labels.join(", ")).expect("write");
        }
    }
    out
}

/// The cases from the deleted `tree-sitter-quilt/test/corpus/` — the ones
/// someone already thought were worth pinning at the grammar level, kept here
/// now that there is no grammar to pin them against.
const GRAMMAR_CORPUS: &[&str] = &[
    // glyphs: Escape every glyph
    "a\\↖b\\↗c\\↙d\\↘e\\↑f\\↓g\\←h\\⟨i\\⟩j\n",
    // glyphs: Escaped emit is not an emit
    "def m : IO Unit := do\n  let x \\← IO.getStdout\n  pure ()\n",
    // glyphs: ASCII bind passes through untouched
    "let x <- IO.getStdout\n",
    // glyphs: Backslash before a non-glyph stays content
    "a\\nb\\tc\\dd\\\"e\n",
    // glyphs: Escaped backslash then glyph
    "a\\\\↖b↗\n",
    // glyphs: Lift operator
    "↙x.↑↘\n",
    // glyphs: Emit operator
    "↖a↗.←\n",
    // glyphs: Reduce operator
    "↓↖1 + 2↗\n",
    // glyphs: Annotated reduce
    "py↓↖1 + 2↗\n",
    // glyphs: Glyph at start and end of file
    "↖a↗\n",
    // glyphs: Type and name in context
    "fn mk() -> Result<⟨T⟩> { ⟨N⟩ }\n",
    // glyphs: Annotated reduce with a digit
    "lean4↓↖1 + 2↗\n",
    // nesting: Unquote inside a quote
    "↖a ↙b↘ c↗\n",
    // nesting: Named unquote
    "↖a py↙b↘ c↗\n",
    // nesting: Bare unquote at top level
    "↙x↘\n",
    // nesting: Quote nested in a quote
    "↖a ↖b↗ c↗\n",
    // nesting: Nesting depth three
    "↖a ↖b ↖c↗ d↗ e↗\n",
    // nesting: Unquote nested in an unquote
    "↖a ↙b ↖c ↙d↘↗↘ e↗\n",
    // nesting: Empty quote
    "↖↗\n",
    // nesting: Empty unquote inside a quote
    "↖↙↘↗\n",
    // nesting: Adjacent quotes
    "↖a↗↖b↗\n",
    // nesting: Multiline quote
    "↖\ndef foo():\n    pass\n↗\n",
    // nesting: Both annotations on one line
    "py↖a wgsl↖b↗ rs↙c↘ d↗\n",
    // test: Escape
    "abc\\↖def\\↗ghi\n",
    // test: Escape Escape
    "abc\\\\↖def\\\\↗ghi\n",
    // test: Named Quote
    "1py↖1↗\n",
    // test: Symbols
    "⟨T⟩⟨N⟩\\⟨N\\⟩\n",
    // test: Line Comments
    "⟨//⟩ comment ↖\n1 + 2\n⟨//⟩ comment\n",
    // test: Block Comments
    "⟨/*⟩ comment ↖ ⟨ */⟩ ⟨* /⟩ ⟨*/ ⟩ ⟨*/⟩\n1 + ⟨/*⟩ foo ⟨*/⟩ 2\n⟨/*⟩\nmulti\nline\ncomment\n⟨*/⟩\n",
    // test: Plain Line Comments
    "// comment with ↑ quilt chars ↓\n1 + 2\n// another comment\n",
    // test: Plain Block Comments
    "/* block comment with ↑ quilt chars */\ncode\n/* multi\nline\nblock */\n",
    // test: Plain Comment URL Split
    "let url = \"https://example.com\";\n",
    // test: Annotation With A Digit
    "lean4↖n + 1↗\n",
    // test: Annotation With A Digit, Unquote
    "lean4↖↙lean4↙x↘↘↗\n",
    // test: Digits Alone Are Not An Annotation
    "x = 42↖1↗\n",
    // test: Plain Line Comment Does Not Eat The Closing Quote
    "let b = rs↖let x = 1; // hi↗;\n",
    // test: Plain Line Comment Does Not Eat The Closing Unquote
    "rs↖let y = ↙f() // hi↘;↗\n",
    // test: Quilt Line Comment Does Not Eat The Closing Quote
    "rs↖let x = 1; ⟨//⟩ note↗\n",
    // test: A Ground-Level Line Comment Still Reaches End Of Line
    "// prose about the ↙…↘ hole and the ↖…↗ brackets\n1 + 2\n⟨//⟩ and a quilt comment about ↙…↘ too\n",
];

/// Hand-picked cases, each of which was a decision in the scanner: where a
/// comment wins over content, how far one runs, what an escape does to
/// adjacent content, where an annotation starts, and which malformed inputs
/// are diagnostics.
const EDGE_CASES: &[&str] = &[
    // `//` outranks content wherever it lands
    "https://example.com",
    "a // b",
    "a//b\n//c",
    // …but is bounded inside a bracket (#226)
    "rs↖// hi↗;",
    "↖a//b↗c",
    "↖ // x\n y ↗",
    "// comment ↗ arrow",
    // Quilt's own comments vanish, newline and indentation included
    "⟨//⟩ hi",
    "x\n  ⟨//⟩ hi\ny",
    "x\n   \n  ⟨//⟩ hi\ny",
    "x\n\t⟨//⟩ hi",
    "x\n \u{a0}⟨//⟩ hi",
    "x\r\n⟨//⟩ hi",
    "a\n  ⟨/*⟩x⟨*/⟩\nb",
    "⟨/*⟩a⟨*/⟩⟨/*⟩b⟨*/⟩",
    "x⟨/*⟩y⟨*/⟩z",
    "↖⟨/*⟩ ↗ ⟨*/⟩↗",
    // a `⟨/*⟩` ends at the first *aligned* `⟨*/⟩`, not the first one that
    // occurs — the body's alternatives eat the character after a `⟨`
    "⟨/*⟩⟨⟨*/⟩",
    "⟨/*⟩⟨⟨*/⟩⟨*/⟩",
    "⟨/*⟩⟨*/⟨*/⟩",
    "⟨/*⟩⟨*/⟨⟨*/⟩",
    // an unterminated block comment is content, not a comment
    "/* unterminated",
    "/*/",
    "/**/",
    "/***/",
    "/* a ** / b */",
    // annotations, and the digits that are not one
    "x = 42↖1↗",
    "1a↓",
    "ab1c↖x↗",
    "ab1c x↖y↗",
    "aBc↖x↗",
    "a_c↖x↗",
    "lean4↖x↗ lean4↙y↘ lean4↓",
    // escapes are their own nodes
    "a \\↖ b",
    "a\\↖b",
    "\\↖\\↗",
    "\\⟨T⟩",
    "a\\nb",
    "↖\\↗↗",
    // malformed
    "fn main() { let x = ↖1 + 2; }",
    "fn main() { let x = 1 ↗ 2; }",
    "fn main() { ↘ }",
    "↖↙↗",
    "py↖",
    "↖x↘",
    "↙x↗",
    "↖//x↘↗",
    "⟨X⟩",
    "a\\",
];

#[test]
fn edge_cases() {
    let rendered: String = EDGE_CASES.iter().copied().map(render).collect();
    insta::assert_snapshot!(rendered);
}

#[test]
fn grammar_corpus() {
    let rendered: String = GRAMMAR_CORPUS.iter().copied().map(render).collect();
    insta::assert_snapshot!(rendered);
}

/// The alphabet for the generated sweeps. Every entry is here because it is a
/// place the scanner has to make a decision, not because it is common.
const ALPHABET: &[&str] = &[
    "↖", "↗", "↙", "↘", "↑", "↓", "←", "py↖", "py↙", "py↓", "a1↓", "1a↓", "⟨T⟩", "⟨N⟩", "⟨//⟩",
    "⟨/*⟩", "⟨*/⟩", "⟨", "⟩", "//", "/*", "*/", "/", "*", "// x", "/* x */", "\\↖", "\\⟨", "\\\\",
    "\\x", "\\", "\n", " ", "\t", "\n  ", "\r", "x", "42", "ab1c", "aB", "lean4↖", "⟨*", "⟨/",
    "*⟩", "/⟩",
];

/// Run `check` over every input the sweeps generate: the repo's own `.quilt`
/// files, the recorded cases, every sequence of up to three tokens, and 50k
/// seeded-random longer ones.
///
/// Three tokens is where exhaustive stops being free (four is 2.8M inputs), and
/// the random pass covers what needs more — the second `⟨*/⟩` bug was seven
/// tokens deep. Raise it with `QUILT_PARSER_SWEEP=8000000 cargo test --release`.
fn over_every_input(check: impl Fn(&str)) {
    if let Some(root) = repo_root() {
        let mut files = Vec::new();
        collect_quilt_files(&root, &mut files);
        assert!(
            files.len() > 20,
            "expected the repo's .quilt corpus, found {}",
            files.len()
        );
        for path in files {
            if let Ok(src) = std::fs::read_to_string(&path) {
                check(&src);
            }
        }
    }
    for src in EDGE_CASES.iter().chain(GRAMMAR_CORPUS) {
        check(src);
    }
    for a in ALPHABET {
        check(a);
        for b in ALPHABET {
            let two = format!("{a}{b}");
            check(&two);
            for c in ALPHABET {
                check(&format!("{two}{c}"));
            }
        }
    }
    let n: u64 = std::env::var("QUILT_PARSER_SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000);
    // xorshift64, inline: the corpus has to be the same on every machine and
    // every run, and a `rand` dependency for eight lines is not worth it.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..n {
        let len = 2 + next() % 7;
        let mut src = String::new();
        for _ in 0..len {
            let i = next() % ALPHABET.len() as u64;
            src.push_str(ALPHABET[usize::try_from(i).expect("index fits")]);
        }
        check(&src);
    }
}

/// `scan`'s tokens tile the source: concatenating them reproduces it exactly,
/// no gaps and no overlaps.
///
/// This is the contract `quilt-lsp` builds its projections on — it copies
/// source bytes span by span, so a gap would silently drop text out of a
/// virtual document and every position after it would be wrong.
#[test]
fn tokens_tile_the_source() {
    over_every_input(|src| {
        let (tokens, _) = quilt::node::scan(src);
        let mut at = 0;
        for t in &tokens {
            assert_eq!(t.span.start, at, "gap or overlap before {t:?} in {src:?}");
            assert!(t.span.end > t.span.start, "empty token {t:?} in {src:?}");
            at = t.span.end;
        }
        assert_eq!(at, src.len(), "tokens stop short of the end of {src:?}");
    });
}

/// The strict and recovering halves of the parser agree on whether an input is
/// well-formed. They share `step`, so this is really asking whether the two
/// drivers over it handle every `Step` the same way.
#[test]
fn strict_and_recovering_agree_on_validity() {
    over_every_input(|src| {
        let strict = Node::parse(src).is_ok();
        let (_, errors) = quilt::node::scan(src);
        assert_eq!(
            strict,
            errors.is_empty(),
            "{src:?}: Node::parse ok={strict} but scan reported {errors:?}"
        );
    });
}

/// …and on the bracket structure. Every `Quote`/`Unquote` in the tree has a
/// matching `OpenQuote`/`CloseQuote` pair in the token stream at the same byte
/// offsets, which is what lets `quilt-lsp` rebuild the region tree from tokens
/// alone.
#[test]
fn brackets_match_between_tree_and_tokens() {
    over_every_input(|src| {
        let Ok(nodes) = Node::parse(src) else {
            return;
        };
        let (tokens, _) = quilt::node::scan(src);
        let mut from_tokens = Vec::new();
        let mut stack = Vec::new();
        for t in &tokens {
            match t.kind {
                TokenKind::OpenQuote | TokenKind::OpenUnquote => stack.push(t.span.start),
                TokenKind::CloseQuote | TokenKind::CloseUnquote => {
                    let open = stack.pop().expect("a closer has an opener");
                    from_tokens.push(open..t.span.end);
                }
                _ => {}
            }
        }
        from_tokens.sort_by_key(|s| (s.start, s.end));

        let mut from_tree = Vec::new();
        collect_spans(&nodes, &mut from_tree);
        from_tree.sort_by_key(|s| (s.start, s.end));

        assert_eq!(from_tokens, from_tree, "{src:?}: bracket spans differ");
    });
}

fn collect_spans(nodes: &[Node], out: &mut Vec<std::ops::Range<usize>>) {
    for node in nodes {
        if let Node::Quote { nodes, span, .. } | Node::Unquote { nodes, span, .. } = node {
            out.push(span.clone());
            let owned: Vec<Node> = nodes.iter().map(|n| (**n).clone()).collect();
            collect_spans(&owned, out);
        }
    }
}

/// The repo root — `quilt/`'s parent. Absent from a published-crate build, in
/// which case the file corpus is skipped and the rest still runs.
fn repo_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .to_path_buf();
    root.join("examples").is_dir().then_some(root)
}

fn collect_quilt_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !matches!(&*name, "target" | "node_modules" | ".git") {
                collect_quilt_files(&path, out);
            }
        } else if name.ends_with(".quilt") {
            out.push(path);
        }
    }
}
