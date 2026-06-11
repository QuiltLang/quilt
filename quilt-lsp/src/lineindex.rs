//! Position-encoding layer: conversions between byte offsets and LSP
//! [`Position`]s.
//!
//! This is the single most error-prone part of an embedded-language server.
//! LSP positions count **UTF-16 code units** by default, while everything in
//! Quilt (and tree-sitter) is in **bytes** — and the arrow glyphs `↖↗↙↘↑↓` are
//! 3 bytes / 1 UTF-16 unit each. Every conversion in the server routes through
//! here so the byte ↔ UTF-16 ↔ (line, column) arithmetic lives in exactly one
//! tested place.
//!
//! UTF-8 is supported too, for clients that negotiate it via `positionEncoding`.

// Character widths are 1..=4 and line/column counts comfortably fit u32 (LSP
// itself types them as u32), so these casts never truncate.
#![allow(clippy::cast_possible_truncation)]

use std::ops::Range;
use tower_lsp::lsp_types::{Position, PositionEncodingKind};

/// Which unit the client counts `Position::character` in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// `character` counts UTF-16 code units (the LSP default).
    #[default]
    Utf16,
    /// `character` counts UTF-8 bytes.
    Utf8,
}

impl Encoding {
    /// Pick an encoding from the client's advertised `positionEncodings`,
    /// preferring UTF-8 (cheapest, exact) then falling back to the UTF-16
    /// default that every client supports.
    pub fn negotiate(offered: Option<&[PositionEncodingKind]>) -> Self {
        match offered {
            Some(kinds) if kinds.contains(&PositionEncodingKind::UTF8) => Encoding::Utf8,
            _ => Encoding::Utf16,
        }
    }

    pub fn as_kind(self) -> PositionEncodingKind {
        match self {
            Encoding::Utf16 => PositionEncodingKind::UTF16,
            Encoding::Utf8 => PositionEncodingKind::UTF8,
        }
    }

    /// Width of `c` in this encoding's units.
    fn width(self, c: char) -> u32 {
        match self {
            Encoding::Utf16 => c.len_utf16() as u32,
            Encoding::Utf8 => c.len_utf8() as u32,
        }
    }
}

/// Precomputed line structure over a document, owning the byte offset of each
/// line start. Conversions take the source text so the index itself stays
/// cheap to keep alongside a document.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset of the first character of each line. Always starts with `0`.
    line_starts: Vec<usize>,
    /// Total length of the document in bytes.
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            line_starts,
            len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte offset → LSP `Position`. Offsets past the end clamp to the end.
    pub fn position(&self, text: &str, offset: usize, enc: Encoding) -> Position {
        let offset = offset.min(self.len);
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        };
        let line_start = self.line_starts[line];
        let character = text[line_start..offset].chars().map(|c| enc.width(c)).sum();
        Position {
            line: line as u32,
            character,
        }
    }

    /// LSP `Position` → byte offset. Positions past the end of a line clamp to
    /// the line's end (including its trailing newline); positions past the end
    /// of the document clamp to its length.
    pub fn offset(&self, text: &str, pos: Position, enc: Encoding) -> usize {
        let line = pos.line as usize;
        let Some(&line_start) = self.line_starts.get(line) else {
            return self.len;
        };
        let line_end = self.line_starts.get(line + 1).copied().unwrap_or(self.len);

        let mut units = 0u32;
        let mut byte = line_start;
        for c in text[line_start..line_end].chars() {
            if units >= pos.character {
                break;
            }
            units += enc.width(c);
            byte += c.len_utf8();
        }
        byte
    }

    /// Byte range → LSP `Range`.
    pub fn range(
        &self,
        text: &str,
        range: Range<usize>,
        enc: Encoding,
    ) -> tower_lsp::lsp_types::Range {
        tower_lsp::lsp_types::Range {
            start: self.position(text, range.start, enc),
            end: self.position(text, range.end, enc),
        }
    }

    /// LSP `Range` → byte range.
    pub fn byte_range(
        &self,
        text: &str,
        range: tower_lsp::lsp_types::Range,
        enc: Encoding,
    ) -> Range<usize> {
        self.offset(text, range.start, enc)..self.offset(text, range.end, enc)
    }

    /// Byte offset → `(row, column_in_bytes)` for building a tree-sitter
    /// `InputEdit`. `column` is the byte distance from the line start, which
    /// is what tree-sitter's `Point::column` expects.
    pub fn byte_to_row_col(&self, byte_offset: usize) -> (usize, usize) {
        let byte_offset = byte_offset.min(self.len);
        let line = match self.line_starts.binary_search(&byte_offset) {
            Ok(l) => l,
            Err(next) => next - 1,
        };
        (line, byte_offset - self.line_starts[line])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn ascii_roundtrip() {
        let text = "abc\ndef\n";
        let idx = LineIndex::new(text);
        for enc in [Encoding::Utf16, Encoding::Utf8] {
            for offset in 0..=text.len() {
                let p = idx.position(text, offset, enc);
                assert_eq!(
                    idx.offset(text, p, enc),
                    offset,
                    "offset {offset} enc {enc:?}"
                );
            }
        }
    }

    #[test]
    fn line_starts() {
        let idx = LineIndex::new("a\nbb\n\nc");
        assert_eq!(idx.line_count(), 4);
        let text = "a\nbb\n\nc";
        assert_eq!(idx.position(text, 0, Encoding::Utf16), pos(0, 0));
        assert_eq!(idx.position(text, 2, Encoding::Utf16), pos(1, 0));
        assert_eq!(idx.position(text, 5, Encoding::Utf16), pos(2, 0));
        assert_eq!(idx.position(text, 6, Encoding::Utf16), pos(3, 0));
    }

    #[test]
    fn arrow_glyph_widths() {
        // `↖` is 3 bytes, 1 UTF-16 unit. Quilt source is full of these.
        let text = "x↖y↗\n";
        let idx = LineIndex::new(text);

        // After `x` (1 byte) then `↖` (3 bytes) we are at byte 4.
        let after_arrow = idx.position(text, 4, Encoding::Utf16);
        assert_eq!(after_arrow, pos(0, 2)); // 1 unit for x + 1 unit for ↖
        assert_eq!(idx.offset(text, after_arrow, Encoding::Utf16), 4);

        // In UTF-8 the same byte offset is character == byte column.
        let after_arrow_u8 = idx.position(text, 4, Encoding::Utf8);
        assert_eq!(after_arrow_u8, pos(0, 4));
        assert_eq!(idx.offset(text, after_arrow_u8, Encoding::Utf8), 4);
    }

    #[test]
    fn astral_plane_two_utf16_units() {
        // An emoji outside the BMP is 1 char, 4 UTF-8 bytes, 2 UTF-16 units.
        let text = "a😀b";
        let idx = LineIndex::new(text);
        let p = idx.position(text, 5, Encoding::Utf16); // byte 5 == after the emoji
        assert_eq!(p, pos(0, 3)); // a(1) + emoji(2)
        assert_eq!(idx.offset(text, p, Encoding::Utf16), 5);
    }

    #[test]
    fn clamps_past_end() {
        let text = "ab\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.offset(text, pos(99, 99), Encoding::Utf16), text.len());
        assert_eq!(idx.position(text, 999, Encoding::Utf16), pos(1, 0));
    }
}
