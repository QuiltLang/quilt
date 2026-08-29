//! Per-document analysis state.
//!
//! Plain owned data: the text, its token stream, the region tree over it, and
//! the diagnostics found on the way.
//!
//! There is no incremental re-parse and no old tree to hand back. Scanning is
//! ~5 µs/KiB (`cargo bench -p quiltlang --bench parse`), so a keystroke in a
//! 100 KiB file costs half a millisecond to re-scan from scratch — cheaper than
//! the bookkeeping incremental parsing would need, and it cannot go stale.

use crate::lineindex::LineIndex;
use crate::regions::{self, Region, SyntaxError};
use quilt::node::Token;
use tower_lsp::lsp_types::Url;

#[derive(Debug)]
pub struct Document {
    pub text: String,
    pub version: i32,
    pub line_index: LineIndex,
    /// Language-extension chain from the filename, ground language first —
    /// `shaders.wgsl.rs.quilt` → `["rs", "wgsl"]` (see
    /// [`crate::adapters::lang_chain`]).
    pub chain: Vec<String>,
    /// Ground-language key from the filename (`rs`, `py`, …), if any; the
    /// first element of `chain`.
    pub ground: Option<String>,
    /// Region tree (root is the whole-file ground region).
    pub region: Region,
    /// Quilt-level syntax errors.
    pub errors: Vec<SyntaxError>,
    /// Every quilt token with its byte range, tiling the source — what the
    /// projections copy from.
    pub tokens: Vec<Token>,
}

impl Document {
    pub fn new(uri: &Url, text: String, version: i32) -> Self {
        let chain = crate::adapters::lang_chain(uri);
        let ground = chain.first().cloned();
        let tokens = regions::tokens(&text);
        let errors = regions::errors(&text);
        let region = regions::regions(&text, &tokens, &self::chain_refs(&chain));
        let line_index = LineIndex::new(&text);
        Self {
            text,
            version,
            line_index,
            chain,
            ground,
            region,
            errors,
            tokens,
        }
    }
}

/// Borrow a `String` chain as the `&[&str]` the analysis phases take.
pub fn chain_refs(chain: &[String]) -> Vec<&str> {
    chain.iter().map(String::as_str).collect()
}
