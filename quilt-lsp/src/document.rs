//! Per-document analysis state.
//!
//! Holds only plain owned data (no tree-sitter handles), so a `Document` is
//! `Send + Sync` and can live in the shared document store. The CST is parsed
//! transiently during analysis and distilled into the [`Region`] tree and the
//! list of syntax errors.

use crate::lineindex::LineIndex;
use crate::regions::{self, Region, SyntaxError};
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
}

impl Document {
    pub fn new(uri: &Url, text: String, version: i32) -> Self {
        let chain = crate::adapters::lang_chain(uri);
        let ground = chain.first().cloned();
        let mut parser = regions::parser();
        let tree = regions::parse(&mut parser, &text);
        let errors = regions::collect_errors(&tree);
        let region = regions::regions(&text, &tree, &self::chain_refs(&chain));
        let line_index = LineIndex::new(&text);
        Self {
            text,
            version,
            line_index,
            chain,
            ground,
            region,
            errors,
        }
    }
}

/// Borrow a `String` chain as the `&[&str]` the analysis phases take.
pub fn chain_refs(chain: &[String]) -> Vec<&str> {
    chain.iter().map(String::as_str).collect()
}
