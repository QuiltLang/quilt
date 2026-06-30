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
- **Python as a ground language** via a downstream Python server (pyright by
  default, overridable with `QUILT_LSP_PYTHON_SERVER`): hover, go-to-definition,
  and completion for `.py.quilt` files. Downstream diagnostics are suppressed
  for now — the projection's `()` quote placeholders mistype ground lines that
  consume a quoted value, so pyright errors would be spurious.
- **Semantic-token highlighting**, including inside `↖…↗` quotes: each quoted
  Rust fragment is appended to the virtual document (wrapped in `fn _quilt_qN`)
  so rust-analyzer tokenizes it; tokens are remapped back onto the quote. When
  the ground server provides no semantic tokens at all (pyright doesn't — a
  Pylance-only feature) the whole ground projection is highlighted in-process
  with tree-sitter instead, so `.py.quilt` files still get code coloring.
- **Folding** for quilt regions plus the ground server's folds.
- **Sky-first templates** (`*.tmpl.quilt`): a directory-scaffolding template is
  the body of an implicit `target↖ … ↗`, not a ground-first program — the whole
  file is target-language source and each `↙name↘` is a *parameter hole*. Such a
  file is projected sky-first as one target-language document (holes masked) so
  it edits with the target language: in-process tree-sitter highlighting for the
  body, downstream analysis where the target has a per-file server (e.g.
  `*.wgsl.tmpl.quilt` → wgsl-analyzer), and document symbols listing the
  template's parameters. (Routing host targets like `*.py.tmpl.quilt` to their
  ground servers, and directory-level awareness, are the remaining follow-ups —
  see issue #117.)

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
Python ground diagnostics (needs a type-aware quote placeholder).

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
python3 quilt-lsp/tests/integration_python.py target/debug/quilt-lsp # .py.quilt ground (mock + pyright)
```

## Configuration

- `QUILT_LSP_RUST_ANALYZER` — override the downstream Rust server command
  (whitespace-separated, e.g. a custom path). Defaults to `rust-analyzer` on
  `PATH`.
- `QUILT_LSP_PYTHON_SERVER` — override the downstream Python server command.
  Defaults to `pyright-langserver --stdio` on `PATH`.
- `QUILT_LSP_WGSL_SERVER` — override the downstream WGSL server command.
  Defaults to `wgsl-analyzer` on `PATH`.
- `RUST_LOG` — standard `tracing` filter; logs go to stderr.

Cargo features mirror quilt's own: `rust`, `python`, `wgsl`, `html`, `bash`,
and `zsh` are all enabled by default. The last three are highlight-only: their
quoted fragments get in-process tree-sitter semantic tokens, with no
downstream server.

## Editor setup

The VS Code extension in [`tools/quilt`](../../tools/quilt) launches this server
for `.quilt` files. From that directory: `npm install`, ensure `quilt-lsp` is on
`PATH` (`cargo install --path .` or set `quilt-lsp.serverPath`), and reload.
