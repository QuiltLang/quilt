# Quilt LSP

**Crate:** `quilt-lsp/`

`quilt-lsp` is a multiplexing Language Server for `.quilt` files. It sits between the editor and the language-specific downstream servers (currently `rust-analyzer`), handling:

1. Quilt-level syntax diagnostics (bracket errors, structure problems).
2. Ground-language features (hover, go-to-definition, completion, diagnostics) by projecting the `.quilt` file into a virtual plain-language document and proxying to the downstream server.
3. Semantic token highlighting inside `↖…↗` quote fragments.
4. Folding ranges for quilt regions.

## Architecture

```
Editor (VS Code)
    │
    │  LSP over stdio
    ▼
quilt-lsp  (server.rs)
    │
    ├─ tree-sitter-quilt parse      → syntax diagnostics + regions (regions.rs)
    │
    ├─ Projection                   → virtual document (projection.rs)
    │   ├─ ground text (verbatim, with quilt constructs replaced by placeholders)
    │   └─ appended quote fragments (wrapped in fn _quilt_qN)
    │
    ├─ SourceMap                    → bidirectional byte-position mapping (srcmap.rs)
    │
    └─ ChildServer (rust-analyzer)  → downstream LSP (child.rs)
        │  LSP over stdio
        └─ proxied requests/responses, positions remapped via LineIndex + translate.rs
```

### Module responsibilities

| Module          | Responsibility                                                                |
|-----------------|-------------------------------------------------------------------------------|
| `lineindex.rs`  | Byte ↔ UTF-16/UTF-8 ↔ `Position` conversion. One place for all encoding math. |
| `regions.rs`    | Parse the quilt CST → region tree + syntax errors                             |
| `srcmap.rs`     | Bidirectional byte map between quilt and a virtual document                   |
| `projection.rs` | Build the ground virtual document + its source map                            |
| `child.rs`      | Spawn, frame, and route a downstream LSP server                               |
| `translate.rs`  | Remap downstream results (ranges, URIs) back to quilt coordinates             |
| `server.rs`     | The editor-facing server, LSP method handlers, merged diagnostics             |
| `adapters.rs`   | Language-specific placeholders and comment syntax                             |
| `semtok.rs`     | Semantic token merging and remap                                              |

## The projection

The **ground projection** copies every ground-language byte verbatim and replaces each quilt construct (`↖…↗`, `↙…↘`, `↑↓←⟨T⟩⟨N⟩`) with a small placeholder that keeps the host language roughly parseable (e.g. `_ /* ↖ */` for a quote hole in Rust).

A file with no quilt constructs (e.g. `examples/hello.rs.quilt`) projects byte-for-byte to itself — `rust-analyzer` sees ordinary Rust.

Quote fragments are **appended** to the virtual document, each wrapped in a `fn _quilt_qN() { … }` so `rust-analyzer` tokenizes them for highlighting. Diagnostics from appended fragments are suppressed (their wrapping makes positions unreliable); only their semantic tokens are kept.

The `SourceMap` records the mapping between quilt byte positions and virtual document byte positions, and `LineIndex` converts between bytes and `Position` for LSP messages.

## Downstream server lifecycle

A single `rust-analyzer` process is spawned lazily when the first `.rs.quilt` file is opened. It is kept alive for the session. All document operations on the projected virtual document are forwarded to it as `workspace/didChangeWatchedFiles` notifications or `textDocument/didOpen` / `textDocument/didChange` overlay updates.

The downstream server is sent the de-quilted URI (`foo.rs`, not `foo.rs.quilt`) so it resolves the file inside the real Cargo project.

## Implemented features

- **Syntax diagnostics** — bracket/structure errors on every `.quilt` file.
- **Rust ground language** via rust-analyzer:
  - Hover, go-to-definition, completion, diagnostics.
  - Positions mapped between `.quilt` and the projected `.rs`.
- **Semantic tokens** — including inside `↖…↗` quotes.
- **Folding** — quilt regions + ground server folds.

## Not yet implemented

- Hover/definition for ground code spliced into quotes via `↙…↘`.
- `↙name↘` → go-to-definition in the ground language.
- Downstream servers for Python and WGSL (adapter extension points exist).

## Configuration

### Environment variables

| Variable                  | Description                                                                                             |
|---------------------------|---------------------------------------------------------------------------------------------------------|
| `QUILT_LSP_RUST_ANALYZER` | Override the downstream Rust server command (whitespace-separated). Default: `rust-analyzer` on `PATH`. |
| `RUST_LOG`                | `tracing` log filter; logs go to stderr.                                                                |

### VS Code settings

Set via `quilt-lsp.*` extension settings (see [Editor Setup](editor-setup.md)):

| Setting                      | Description                                             |
|------------------------------|---------------------------------------------------------|
| `quilt-lsp.serverPath`       | Path to the `quilt-lsp` binary                          |
| `quilt-lsp.rustAnalyzerPath` | Override rust-analyzer (sets `QUILT_LSP_RUST_ANALYZER`) |
| `quilt-lsp.trace.server`     | `off` \| `messages` \| `verbose`                        |

## Building and testing

```sh
# Build
cargo build -p quilt-lsp

# Unit tests (position maps, projection, translate)
cargo test -p quilt-lsp

# End-to-end tests (drive the server over stdio):
python3 quilt-lsp/tests/smoke_lsp.py        target/debug/quilt-lsp
python3 quilt-lsp/tests/integration_mock.py target/debug/quilt-lsp
python3 quilt-lsp/tests/integration_ra.py   target/debug/quilt-lsp

# Integration tests for specific features:
python3 quilt-lsp/tests/integration_gotodef.py target/debug/quilt-lsp
python3 quilt-lsp/tests/integration_semtok.py  target/debug/quilt-lsp
```

## Installing

```sh
cargo install --path quilt-lsp
# puts `quilt-lsp` on your PATH
```

Or run `bin/install_tools`, which installs the LSP and the VS Code extension together (see [Editor Setup](editor-setup.md)).
