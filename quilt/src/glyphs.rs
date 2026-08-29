//! The characters Quilt reserves, and the `\` escape that lets a fragment use
//! one as ordinary text.
//!
//! These are facts about Quilt's *surface syntax* and involve no parser, which
//! is why they live here rather than in [`crate::node`]: `node` is behind the
//! `parse` feature, and the writer in [`crate::strcmd`] — which is on the
//! runtime-only path that `quilt-wasm` and `nanobots-codegen` build for
//! `wasm32-unknown-unknown` (#162) — needs them to render a term back as
//! `.quilt` source (#223). `node` re-exports everything here, so
//! `quilt::node::GLYPHS` and friends keep working.

/// UTF-8 length of a Quilt glyph. Every glyph in [`GLYPHS`] is this wide (see
/// `node::tests::glyph_lengths_are_uniform`), which is what lets callers doing
/// byte arithmetic over the surface syntax — e.g. `quilt-lsp`'s `regions` — use
/// a single constant. Derived from a glyph rather than written as `3` so it
/// cannot drift from the glyph set it describes.
pub const ARROW_LEN: usize = '↖'.len_utf8();
/// UTF-8 length of the `\` that introduces an escape.
pub const ESCAPE_LEN: usize = '\\'.len_utf8();

/// The characters Quilt gives special meaning to, and hence the ones `\` can
/// escape: the four quote/unquote arrows, lift, reduce, emit, and the `⟨…⟩`
/// delimiters.
///
/// This is the single source of truth. `node::parse` reads it directly when
/// deciding what may appear bare in content and what a `\` escapes, so the
/// parser cannot drift from it — which it could when the same set was also
/// spelled out in three character classes in a tree-sitter grammar, and did
/// (`←` was Quilt's emit glyph but missing from the grammar's escape class, so
/// `\←` parsed as a literal backslash-arrow — issue #141).
pub const GLYPHS: [char; 9] = ['↖', '↗', '↙', '↘', '↑', '↓', '←', '⟨', '⟩'];

/// The glyphs that are *operators* rather than brackets: `↑` lift, `↓` reduce,
/// `←` emit.
///
/// The distinction matters exactly once, in [`crate::qterm`]'s source renderer:
/// an operator inside an unresolved quote is deferred to the next stage and
/// plugged back into the term as a bare glyph (see `Multi::build_nodes`), so it
/// must reach the output unescaped, while the same glyph arriving as escaped
/// *content* must not. Brackets are never deferred, so they are never ambiguous.
pub const OPERATOR_GLYPHS: [char; 3] = ['↑', '↓', '←'];

/// Escape every Quilt glyph in `s` with a leading `\`.
///
/// This is the inverse of the grammar's `escape` rule and exists so that
/// `Node::coparse` round-trips: a `Node::Content` can only hold a glyph if it
/// came from a `\`-escape in the source (the grammar's `_char` class excludes
/// all of them), so writing the glyph bare would make it re-parse as the
/// operator instead of as content.
#[must_use]
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
#[must_use]
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

/// Whether `s` is exactly one deferred Quilt operator, as
/// `Multi::build_nodes` plugs it back into a term: `↑`, `←`, or a reduce with
/// its optional language annotation (`↓`, `py↓`).
#[must_use]
pub fn is_deferred_operator(s: &str) -> bool {
    match s.strip_suffix('↓') {
        // A reduce annotation is `[a-z]*` in the grammar, so anything else with
        // a trailing `↓` is content that merely ends in the glyph.
        Some(anno) => anno.chars().all(|c| c.is_ascii_lowercase()),
        None => s == "↑" || s == "←",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_operators_are_exactly_the_plugged_spellings() {
        // `a↓` is in this list, not the next one: the grammar spells a reduce
        // annotation `[a-z]*`, so a one-letter annotation is as real as `py`.
        for s in ["↑", "←", "↓", "a↓", "py↓", "rs↓"] {
            assert!(is_deferred_operator(s), "{s:?} is a deferred operator");
        }
        // Content that merely contains or ends with a glyph is not.
        for s in ["", "x", "↖", "⟨", "x↑", "↑↑", "1 ↓", "A↓", "x ↓"] {
            assert!(!is_deferred_operator(s), "{s:?} is not one");
        }
    }
}
