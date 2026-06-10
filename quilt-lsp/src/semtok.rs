//! Remapping LSP semantic tokens from a virtual document back to `.quilt`.
//!
//! Tokens are delta-encoded as flat groups of five `u32`s
//! `[Δline, Δstart, length, type, modifiers]`, each relative to the previous
//! token. We decode to absolute virtual positions, map each token's span back
//! to quilt coordinates (dropping any that land in synthetic / masked text, or
//! that would straddle a quilt line), and re-encode. Downstream token `type` /
//! `modifiers` indices pass through unchanged because the legend we advertise
//! starts with the downstream server's own legend (extended with appended
//! types, never reordered — see `register_semantic_tokens`). Tokens from other
//! sources (the tree-sitter fragment highlighter, [`crate::tshl`]) are merged
//! in as [`Tok`]s before encoding.

use crate::lineindex::{Encoding, LineIndex};
use crate::projection::Projection;
use tower_lsp::lsp_types::Position;

/// One absolute token in quilt coordinates.
pub struct Tok {
    pub line: u32,
    pub start: u32,
    pub length: u32,
    pub ty: u32,
    pub modifiers: u32,
}

/// Remap a downstream `data` array (virtual coords) to absolute quilt-coords
/// tokens. Callers merge in tokens from other sources, then [`encode`].
pub fn remap(
    data: &[u32],
    proj: &Projection,
    quilt_text: &str,
    quilt_index: &LineIndex,
    enc: Encoding,
) -> Vec<Tok> {
    let mut toks = Vec::new();
    let (mut line, mut ch) = (0u32, 0u32);

    for g in data.chunks_exact(5) {
        let (d_line, d_start, length, ty, modifiers) = (g[0], g[1], g[2], g[3], g[4]);
        if d_line > 0 {
            line += d_line;
            ch = d_start;
        } else {
            ch += d_start;
        }
        if length == 0 {
            continue;
        }

        // Virtual span → byte offsets → quilt byte offsets.
        let v_start = proj.line_index.offset(
            &proj.text,
            Position {
                line,
                character: ch,
            },
            enc,
        );
        let v_end = proj.line_index.offset(
            &proj.text,
            Position {
                line,
                character: ch + length,
            },
            enc,
        );
        let q_start = proj.map.virtual_to_quilt(v_start);
        let q_end = proj.map.virtual_to_quilt(v_end);
        if q_end <= q_start {
            continue; // synthetic / masked: collapses to a point
        }

        let qs = quilt_index.position(quilt_text, q_start, enc);
        let qe = quilt_index.position(quilt_text, q_end, enc);
        if qs.line != qe.line || qe.character <= qs.character {
            continue; // straddles a quilt line: can't be one token
        }

        toks.push(Tok {
            line: qs.line,
            start: qs.character,
            length: qe.character - qs.character,
            ty,
            modifiers,
        });
    }
    toks
}

/// Sort absolute tokens and delta-encode them into an LSP `data` array.
/// Tokens from different sources (ground remap, interleaved fragments) arrive
/// out of order.
pub fn encode(mut toks: Vec<Tok>) -> Vec<u32> {
    toks.sort_by_key(|t| (t.line, t.start));

    let mut out = Vec::with_capacity(toks.len() * 5);
    let (mut prev_line, mut prev_start) = (0u32, 0u32);
    for t in toks {
        let d_line = t.line - prev_line;
        let d_start = if d_line == 0 {
            t.start - prev_start
        } else {
            t.start
        };
        out.extend_from_slice(&[d_line, d_start, t.length, t.ty, t.modifiers]);
        prev_line = t.line;
        prev_start = t.start;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{language_adapter, meta_adapter};
    use crate::projection::project;

    fn project_ground(src: &str) -> Projection {
        project(
            src,
            meta_adapter("rs").unwrap(),
            language_adapter("rs").unwrap(),
            &["rs"],
        )
    }

    fn decode(data: &[u32]) -> Vec<(u32, u32, u32)> {
        let mut out = Vec::new();
        let (mut l, mut c) = (0u32, 0u32);
        for g in data.chunks_exact(5) {
            if g[0] > 0 {
                l += g[0];
                c = g[1];
            } else {
                c += g[1];
            }
            out.push((l, c, g[2]));
        }
        out
    }

    #[test]
    fn ground_token_passes_through() {
        // Construct-free source projects to identity, so a token stays put.
        let src = "fn main() {}\n";
        let p = project_ground(src);
        let qi = LineIndex::new(src);
        // token: line 0, char 3, len 4 ("main")
        let data = vec![0, 3, 4, 7, 0];
        let out = encode(remap(&data, &p, src, &qi, Encoding::Utf16));
        assert_eq!(decode(&out), vec![(0, 3, 4)]);
    }

    #[test]
    fn token_in_fragment_maps_to_quote() {
        // The quote body `1 + 2` is appended as a fragment; a token there must
        // map back onto the quote in the quilt source (line 0).
        let src = "let x = ↖1 + 2↗;\n";
        let p = project_ground(src);
        let qi = LineIndex::new(src);

        // Find the virtual position of the `1` inside the fragment and craft a
        // token there.
        let one = p.text.find("1 + 2").expect("fragment body present");
        let vpos = p.line_index.position(&p.text, one, Encoding::Utf16);
        let data = vec![vpos.line, vpos.character, 1, 5, 0];
        let out = encode(remap(&data, &p, src, &qi, Encoding::Utf16));
        let toks = decode(&out);
        assert_eq!(toks.len(), 1);
        // The `1` is on quilt line 0 at the char just after `↖` (col 9).
        assert_eq!(toks[0], (0, 9, 1));
    }

    #[test]
    fn synthetic_token_dropped() {
        let src = "let x = ↖1↗;\n";
        let p = project_ground(src);
        let qi = LineIndex::new(src);
        // A token over the `()` placeholder (synthetic) at line 0 char 8.
        let data = vec![0, 8, 2, 0, 0];
        let out = encode(remap(&data, &p, src, &qi, Encoding::Utf16));
        assert!(
            out.is_empty(),
            "synthetic placeholder token should be dropped"
        );
    }

    #[test]
    fn encode_orders_mixed_sources() {
        // Tokens merged from two sources arrive out of order; encode sorts.
        let toks = vec![
            Tok {
                line: 2,
                start: 4,
                length: 3,
                ty: 1,
                modifiers: 0,
            },
            Tok {
                line: 0,
                start: 1,
                length: 2,
                ty: 0,
                modifiers: 0,
            },
        ];
        let out = encode(toks);
        assert_eq!(decode(&out), vec![(0, 1, 2), (2, 4, 3)]);
    }
}
