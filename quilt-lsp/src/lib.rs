//! `quilt-lsp`: a multiplexing Language Server for Quilt (`.quilt`) files.
//!
//! A `.quilt` file is one ground-language program (chosen by its inner
//! extension, e.g. `foo.rs.quilt` → Rust) with fragments of other languages
//! spliced in via `↖↗`/`↙↘`. This server acts as a host/router: it parses the
//! quilt structure, projects each language into its own virtual document, and
//! (from Phase 1 on) proxies LSP traffic to per-language downstream servers,
//! remapping positions in both directions.

pub mod adapters;
pub mod child;
pub mod document;
pub mod lineindex;
pub mod projection;
pub mod regions;
pub mod semtok;
pub mod server;
pub mod srcmap;
pub mod translate;
