# quilt-lsp

A multiplexing Language Server for Quilt (`.quilt`) files.

A `.quilt` file is one ground-language program — chosen by its inner extension
(`foo.rs.quilt` → Rust) — with fragments of other languages spliced in via
`↖↗`/`↙↘`. `quilt-lsp` is a **host/router**: it parses the quilt structure,
projects each language into its own virtual document, proxies LSP traffic to
per-language downstream servers, and remaps positions in both directions.

## Status

Implemented:

- **Quilt syntax diagnostics** — bracket/structure errors from the
  tree-sitter-quilt parse, on every `.quilt` file.
- **Full Rust support for the ground language** via a downstream `rust-analyzer`:
  hover, go-to-definition, completion, and diagnostics, with positions mapped
  between the `.quilt` file and the projected `.rs` virtual document.
- **Semantic-token highlighting**, including inside `↖…↗` quotes: each quoted
  Rust fragment is appended to the virtual document (wrapped in `fn _quilt_qN`)
  so rust-analyzer tokenizes it; tokens are remapped back onto the quote.
- **Folding** for quilt regions plus the ground server's folds.

The ground language is projected by copying its bytes verbatim and replacing
each quilt construct (`↖…↗`, `↙…↘`, and the `↑↓←⟨T⟩⟨N⟩` glyphs) with a small
placeholder; a [`SourceMap`](src/srcmap.rs) records the mapping. A
construct-free file (e.g. `examples/hello.rs.quilt`) projects to itself
byte-for-byte. The projection is opened to rust-analyzer under the *de-quilted*
file URI (`foo.rs`) as an overlay, so it resolves inside the real cargo project.

Diagnostics inside appended quote fragments are suppressed (their wrapping makes
them unreliable); their tokens are kept for highlighting.

Not yet implemented (designed-for extension points): hover/definition for ground
code *spliced into* quotes via `↙…↘`, the `↙name↘`→ground go-to-definition, and
downstream servers for Python/WGSL.

## Architecture

| Module | Responsibility |
|---|---|
| `lineindex` | byte ↔ UTF-16/UTF-8 ↔ `Position` (the one place encoding math lives) |
| `regions` | tree-sitter-quilt parse → region tree + syntax errors |
| `srcmap` | bidirectional byte map between quilt and a virtual document |
| `projection` | build the ground virtual document + its source map |
| `child` | spawn/frame/route a downstream LSP (rust-analyzer) |
| `translate` | remap downstream *results* (ranges/URIs) back to quilt coords |
| `server` | the editor-facing server + routing + merged diagnostics |

## Build & test

```sh
cargo build -p quilt-lsp
cargo test  -p quilt-lsp          # unit tests (position maps, projection, translate)

# End-to-end (drive the server over stdio):
python3 quilt-lsp/tests/smoke_lsp.py        target/debug/quilt-lsp   # quilt diagnostics
python3 quilt-lsp/tests/integration_mock.py target/debug/quilt-lsp   # proxy w/ mock server
python3 quilt-lsp/tests/integration_ra.py   target/debug/quilt-lsp   # proxy w/ real rust-analyzer
```

## Configuration

- `QUILT_LSP_RUST_ANALYZER` — override the downstream Rust server command
  (whitespace-separated, e.g. a custom path). Defaults to `rust-analyzer` on
  `PATH`.
- `RUST_LOG` — standard `tracing` filter; logs go to stderr.

Cargo features mirror quilt's own (`rust` by default; `python`, `wgsl`
reserved). v1 ships the `rust` adapter only.

## Editor setup

The VS Code extension in [`tools/quilt`](../../tools/quilt) launches this server
for `.quilt` files. From that directory: `npm install`, ensure `quilt-lsp` is on
`PATH` (`cargo install --path .` or set `quilt-lsp.serverPath`), and reload.
