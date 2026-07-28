//! `node::GLYPHS` and the three character classes in `tree-sitter-quilt/grammar.js`
//! describe the same set — the characters Quilt gives special meaning to, and
//! hence the ones `\` can escape. Both places say so in a comment:
//!
//! ```text
//! // NOTE: the three classes below must list the same glyphs, and must match
//! // GLYPHS in quilt/src/node.rs …
//! ```
//!
//! …and until now nothing enforced it. That is exactly the shape of #141: `←`
//! was Quilt's emit glyph but missing from the grammar's escape class *and*
//! from `escape`/`unescape`, so `\←` parsed as a literal backslash-arrow. The
//! comment was there; the check was not.
//!
//! Four places have to agree, and a mismatch in any of them is silent:
//!
//! | | role |
//! |---|---|
//! | `_char` | characters content may *not* contain bare |
//! | `_non_escape` | `\` followed by a non-glyph — stays literal |
//! | `escape` | `\` followed by a glyph — becomes an escape node |
//! | `node::GLYPHS` | what `escape`/`unescape` put a `\` in front of |
//!
//! (#156)

use quilt::node::GLYPHS;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn grammar_js() -> Option<String> {
    // The grammar lives in a sibling crate of `quilt` in this repo. A consumer
    // building from a published crate has no such sibling, so skip rather than
    // fail — the check is for this workspace.
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "tree-sitter-quilt",
        "grammar.js",
    ]
    .iter()
    .collect();
    std::fs::read_to_string(path).ok()
}

/// Pull the character set out of one of the grammar's classes, e.g.
/// `_char: $ => /[^\\↖↗↙↘↑↓←⟨⟩\n]/` → the glyphs between the brackets.
fn class_glyphs(src: &str, rule: &str) -> BTreeSet<char> {
    let line = src
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("{rule}:")))
        .unwrap_or_else(|| panic!("grammar.js has no `{rule}` rule"));
    // Everything inside the regex that is one of the glyphs we care about.
    // Deliberately not a full regex parser: we only need "which of the
    // interesting characters does this class mention".
    line.chars().filter(|c| GLYPHS.contains(c)).collect()
}

#[test]
fn grammar_classes_match_glyphs() {
    let Some(src) = grammar_js() else {
        eprintln!("skipping: tree-sitter-quilt/grammar.js not found (published-crate build)");
        return;
    };

    let want: BTreeSet<char> = GLYPHS.iter().copied().collect();

    for rule in ["_char", "_non_escape", "escape"] {
        let got = class_glyphs(&src, rule);
        assert_eq!(
            got,
            want,
            "grammar.js `{rule}` and node::GLYPHS disagree.\n\
             in grammar only: {:?}\n\
             in GLYPHS only:  {:?}\n\
             Both must list every character Quilt gives special meaning to; a \
             glyph missing from `escape` cannot be written literally with `\\`, \
             and one missing from GLYPHS is not re-escaped on output.",
            got.difference(&want).collect::<Vec<_>>(),
            want.difference(&got).collect::<Vec<_>>(),
        );
    }
}

/// The emit glyph specifically, because it is the one that was missing (#141)
/// and the one with a live collision: Lean spells monadic bind `←`.
#[test]
fn emit_glyph_is_escapable() {
    assert!(
        GLYPHS.contains(&'←'),
        "`←` must be escapable: it is Quilt's emit glyph and Lean's monadic bind (#141)"
    );
    if let Some(src) = grammar_js() {
        for rule in ["_char", "_non_escape", "escape"] {
            assert!(
                class_glyphs(&src, rule).contains(&'←'),
                "grammar.js `{rule}` is missing `←`"
            );
        }
    }
}
