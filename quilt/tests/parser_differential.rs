//! The hand-written Quilt parser and the tree-sitter grammar must agree.
//!
//! Issue #254 replaced [`Node::parse`]'s tree-sitter path with a hand-written
//! scanner. `tree-sitter-quilt/grammar.js` did not go away — `quilt-lsp`'s
//! `regions` and the VS Code extension read it — so Quilt now has *two*
//! descriptions of its own surface syntax, and the failure mode to be afraid of
//! is not a crash but a quiet divergence: a `//` that stops being a comment at
//! some position, an annotation that starts one character early, a `\n`
//! swallowed by a comment on one side and not the other.
//!
//! So this test runs both over the same corpus and requires the *terms* to be
//! equal — `Node` derives `PartialEq` over its spans too, so a byte range that
//! shifts is a failure as much as a node kind that changes.
//!
//! ## What the corpus is
//!
//! Three sources, because they fail in different ways:
//!
//! 1. **Every `.quilt` file in the repo** — real input, including the four
//!    under `examples/` whose comments contain arrows and the `tests/ui/` cases
//!    that are deliberately malformed.
//! 2. **The tree-sitter corpus** (`tree-sitter-quilt/test/corpus/`) — the cases
//!    someone already thought were worth pinning at the grammar level.
//! 3. **Every sequence of up to three interesting tokens** — ~16k inputs from
//!    an alphabet chosen for the places the two parsers could disagree:
//!    annotations abutting digits, `/` next to `*` and `/`, `\` next to a
//!    glyph, `⟨` next to something that is not a placeholder. Exhaustive over a
//!    small alphabet finds the interactions a hand-written case list does not.
//!
//! ## What agreement means
//!
//! Both parsers must agree on *whether* an input parses, and on the tree when
//! it does. They need not agree on where a diagnostic points: tree-sitter's
//! error recovery reports what its parse states can justify (a stray `↗` in
//! `↖x↘` widens to the whole input), while the scanner reports the token it
//! choked on. The hand-written spans are narrower; `node::tests` pins the ones
//! that matter and `tests/ui/` snapshots the rendering.

#![cfg(feature = "parse")]

use pretty_assertions::assert_eq;
use quilt::node::Node;
use std::path::{Path, PathBuf};

/// Assert both parsers see `src` the same way.
fn agree(src: &str, what: &str) {
    let new = Node::parse(src);
    let old = Node::parse_ts(src);
    match (new, old) {
        (Ok(new), Ok(old)) => assert_eq!(new, old, "{what}: trees differ for {src:?}"),
        (Err(_), Err(_)) => {}
        (Ok(new), Err(e)) => panic!(
            "{what}: the hand-written parser accepted {src:?} as {new:?}, \
             but the grammar rejects it: {e}"
        ),
        (Err(e), Ok(old)) => panic!(
            "{what}: the grammar accepted {src:?} as {old:?}, \
             but the hand-written parser rejects it: {e}"
        ),
    }
}

/// The repo root — `quilt/`'s parent. Absent from a published-crate build, in
/// which case the file-corpus tests skip themselves.
fn repo_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .to_path_buf();
    root.join("examples").is_dir().then_some(root)
}

fn quilt_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // `target/` and `node_modules/` hold no sources of ours and are
            // large enough to make this test feel broken.
            if !matches!(&*name, "target" | "node_modules" | ".git") {
                quilt_files(&path, out);
            }
        } else if name.ends_with(".quilt") {
            out.push(path);
        }
    }
}

/// Every `.quilt` file in the repository, parsed both ways.
#[test]
fn repository_sources_agree() {
    let Some(root) = repo_root() else {
        eprintln!("skipping: not a workspace checkout");
        return;
    };
    let mut files = Vec::new();
    quilt_files(&root, &mut files);
    assert!(
        files.len() > 20,
        "expected the repo's .quilt corpus, found {} files — has the walk broken?",
        files.len()
    );
    for path in files {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        agree(&src, &path.display().to_string());
    }
}

/// The tree-sitter corpus files: `====` headers, the input, `----`, the
/// expected S-expression. Only the inputs are of interest here.
#[test]
fn grammar_corpus_agrees() {
    let Some(root) = repo_root() else {
        eprintln!("skipping: not a workspace checkout");
        return;
    };
    let dir = root.join("tree-sitter-quilt/test/corpus");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: no {}", dir.display());
        return;
    };
    let mut cases = 0;
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for case in corpus_inputs(&text) {
            agree(&case, &format!("{}", entry.path().display()));
            cases += 1;
        }
    }
    assert!(
        cases > 10,
        "expected the grammar corpus, found {cases} cases"
    );
}

/// Split a `tree-sitter test` corpus file into its input sections.
fn corpus_inputs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        // `====…` name `====…`, then the input, then `----…`.
        if !line.starts_with("====") {
            continue;
        }
        lines.next(); // the case name
        if lines.next().is_none_or(|l| !l.starts_with("====")) {
            continue;
        }
        let mut input = Vec::new();
        for line in lines.by_ref() {
            if line.starts_with("----") {
                break;
            }
            input.push(line);
        }
        // Corpus inputs are written with a blank line before the `----`; it is
        // a separator, not part of the case.
        while input.last().is_some_and(|l| l.is_empty()) {
            input.pop();
        }
        if !input.is_empty() {
            out.push(format!("{}\n", input.join("\n")));
        }
    }
    out
}

/// The alphabet for the exhaustive sweep. Every entry is here because it is a
/// place the two parsers could plausibly disagree, not because it is common.
const ALPHABET: &[&str] = &[
    // brackets and operators, annotated and bare
    "↖", "↗", "↙", "↘", "↑", "↓", "←", "py↖", "py↙", "py↓", "a1↓", "1a↓",
    // the `⟨…⟩` family: two placeholders, two comment openers, one closer, and
    // a `⟨` that is none of them
    "⟨T⟩", "⟨N⟩", "⟨//⟩", "⟨/*⟩", "⟨*/⟩", "⟨", "⟩",
    // comments and the characters they are built from
    "//", "/*", "*/", "/", "*", "// x", "/* x */",
    // escapes, and a backslash that is not one
    "\\↖", "\\⟨", "\\\\", "\\x", "\\",
    // whitespace, which the `\n\s*⟨//⟩` prefix eats
    "\n", " ", "\t", "\n  ",
    // ordinary content, including the digit/letter boundary an annotation
    // must respect
    "x", "42", "ab1c", "aB",
];

/// Every sequence of up to three tokens from [`ALPHABET`], both ways.
///
/// Three is where it stops being free: 41 tokens is `41 + 1_681 + 68_921` inputs,
/// and the tree-sitter half of each allocates a whole CST. It runs in a few
/// seconds and covers every adjacency, which is where the interesting
/// disagreements live.
#[test]
fn exhaustive_short_sequences_agree() {
    for a in ALPHABET {
        agree(a, "sweep");
        for b in ALPHABET {
            let two = format!("{a}{b}");
            agree(&two, "sweep");
            for c in ALPHABET {
                agree(&format!("{two}{c}"), "sweep");
            }
        }
    }
}

/// The cases that motivated specific lines of the scanner. Redundant with the
/// sweep for the most part, and kept anyway: when this file fails, a named case
/// says what broke, where a generated one says only that something did.
#[test]
fn named_edge_cases_agree() {
    for src in [
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
    ] {
        agree(src, "named");
    }
}

/// The same alphabet, in longer random sequences.
///
/// The exhaustive sweep stops at three tokens because four is 2.8 million
/// inputs. The interactions that need more than three are real, though — the
/// `⟨/*⟩` body's alternatives can swallow the `⟨` of the `⟨*/⟩` that would have
/// closed it, and the shortest case where that *matters* is
/// `lean4↖⟨/*⟩↙↗⟨*/⟨*/⟩↗`, seven tokens deep. So: a fixed seed, a fixed count,
/// and no dependency on how a machine feels today.
///
/// 50k is what keeps this under a second in a debug build. Raise it to sweep
/// harder — `QUILT_PARSER_SWEEP=8000000 cargo test --release -p quiltlang
/// --test parser_differential` is what this landed on.
#[test]
fn random_sequences_agree() {
    let n: u64 = std::env::var("QUILT_PARSER_SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000);

    // xorshift64, inline: the corpus has to be the same on every machine and
    // every run, and a rand dependency for eight lines is not worth it.
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
            let i = next() % SWEEP_ALPHABET.len() as u64;
            src.push_str(SWEEP_ALPHABET[usize::try_from(i).expect("index fits")]);
        }
        agree(&src, "random");
    }
}

/// [`ALPHABET`] plus the fragments that only matter once they can be adjacent
/// to each other: the pieces of `⟨*/⟩`, which is how a comment terminator gets
/// eaten one character at a time.
const SWEEP_ALPHABET: &[&str] = &[
    "↖", "↗", "↙", "↘", "↑", "↓", "←", "py↖", "py↙", "py↓", "a1↓", "1a↓", "⟨T⟩", "⟨N⟩", "⟨//⟩",
    "⟨/*⟩", "⟨*/⟩", "⟨", "⟩", "//", "/*", "*/", "/", "*", "// x", "/* x */", "\\↖", "\\⟨", "\\\\",
    "\\x", "\\", "\n", " ", "\t", "\n  ", "\r", "x", "42", "ab1c", "aB", "lean4↖", "⟨*", "⟨/",
    "*⟩", "/⟩",
];
