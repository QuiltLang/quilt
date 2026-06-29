//! Per-document analysis state.
//!
//! Holds plain owned data plus the tree-sitter [`Tree`] (which is `Send +
//! Sync`) so that [`crate::server`] can hand the old tree back on the next
//! keystroke and let tree-sitter do an incremental re-parse.

use crate::lineindex::LineIndex;
use crate::regions::{self, Region, SyntaxError};
use tower_lsp::lsp_types::Url;
use tree_sitter::Tree;

/// Sky-first template (`*.tmpl.quilt`) metadata: the file is the body of an
/// implicit `target↖ … ↗` rather than a ground-first program. Present only for
/// template documents (see [`crate::adapters::template_chain`]).
#[derive(Debug, Clone)]
pub struct Template {
    /// Target language key (`wgsl`, `py`, `html`, …) — the language the template
    /// body is written in, and the one its virtual document is analyzed as.
    pub target: String,
}

#[derive(Debug)]
pub struct Document {
    pub text: String,
    pub version: i32,
    pub line_index: LineIndex,
    /// Language-extension chain from the filename, ground language first —
    /// `shaders.wgsl.rs.quilt` → `["rs", "wgsl"]` (see
    /// [`crate::adapters::lang_chain`]). For a `*.tmpl.quilt` template this is
    /// the sky chain (the `.tmpl` marker stripped), so its last element is the
    /// target language.
    pub chain: Vec<String>,
    /// Ground-language key from the filename (`rs`, `py`, …), if any; the
    /// first element of `chain`.
    pub ground: Option<String>,
    /// Sky-first template metadata, `Some` iff this is a `*.tmpl.quilt` file.
    pub template: Option<Template>,
    /// Region tree (root is the whole-file ground region).
    pub region: Region,
    /// Quilt-level syntax errors.
    pub errors: Vec<SyntaxError>,
    /// The raw CST, kept so the next `did_change` can pass it to tree-sitter
    /// as the old tree for incremental re-parse.
    pub ts_tree: Tree,
}

impl Document {
    pub fn new(uri: &Url, text: String, version: i32, old_tree: Option<&Tree>) -> Self {
        // A `*.tmpl.quilt` file is a sky-first template: its chain comes from the
        // stem with the `.tmpl` marker stripped, and its body is the target
        // language (the chain's last element). A normal file uses the plain chain.
        let (chain, template) = match crate::adapters::template_chain(uri) {
            Some(chain) => {
                let target = chain.last().cloned().map(|target| Template { target });
                (chain, target)
            }
            None => (crate::adapters::lang_chain(uri), None),
        };
        let ground = chain.first().cloned();
        let mut parser = regions::parser();
        let tree = regions::parse(&mut parser, &text, old_tree);
        let errors = regions::collect_errors(&tree);
        let region = regions::regions(&text, &tree, &self::chain_refs(&chain));
        let line_index = LineIndex::new(&text);
        Self {
            text,
            version,
            line_index,
            chain,
            ground,
            template,
            region,
            errors,
            ts_tree: tree,
        }
    }
}

/// Borrow a `String` chain as the `&[&str]` the analysis phases take.
pub fn chain_refs(chain: &[String]) -> Vec<&str> {
    chain.iter().map(String::as_str).collect()
}
