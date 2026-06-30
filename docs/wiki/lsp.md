# Quilt LSP

**Crate:** `quilt-lsp/`

`quilt-lsp` is a multiplexing Language Server for `.quilt` files. It sits between the editor and the language-specific downstream servers (`rust-analyzer` for Rust, pyright for Python, wgsl-analyzer for WGSL), handling:

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
| `tshl.rs`       | Tree-sitter semantic tokens for embedded fragments (WGSL, HTML, bash, zsh, Nix) |

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
- **Python ground language** via pyright (`pyright-langserver --stdio` by
  default; override with `QUILT_LSP_PYTHON_SERVER`):
  - Hover, go-to-definition, completion for `.py.quilt` files.
  - Semantic highlighting from the in-process tree-sitter highlighter
    (`tshl.rs`): pyright provides no semantic tokens (a Pylance-only
    feature), so the whole ground projection is highlighted as a fallback —
    which also works when pyright isn't installed at all.
  - Downstream diagnostics are suppressed: the projection replaces each quote
    with a `()` placeholder expression, which mistypes any ground line that
    consumes a quoted value, so pyright's errors would be spurious
    (`PythonAdapter::publishes_diagnostics` is `false`).
- **Semantic tokens** — including inside `↖…↗` quotes. Embedded-language
  quotes (`wgsl↖…↗`, `html↖…↗`, `bash↖…↗`, `zsh↖…↗`, `nix↖…↗`) are highlighted
  by the server itself with tree-sitter highlight queries (`tshl.rs`), since
  their downstream servers may provide no semantic tokens (wgsl-analyzer
  advertises none) or not exist at all (html/bash/zsh/nix are highlight-only:
  `server_command` is `None`, so their fragments are projected but never
  opened downstream); the downstream legend is advertised with the
  tree-sitter token types appended. A *ground* server with the same gap
  (pyright) gets the same treatment: the ground projection is highlighted
  in-process whenever the downstream server can't answer
  `semanticTokens/full`.
- **Folding** — quilt regions + ground server folds.
- **Sky-first templates** (`*.tmpl.quilt`) — a directory-scaffolding template
  (issue #84) is the body of an implicit `target↖ … ↗`, not a ground-first
  program: the whole file is target-language source and each `↙name↘` is a
  *parameter hole* (a free variable filled at instantiation), not a ground
  splice. `template_chain` recognizes these files (strip the `.tmpl` marker, then
  read the extension chain — the target is its last element); `project_sky`
  projects the whole body as one target-language document with every hole masked
  to the target's placeholder, mirroring `Multi::parse_template`. The body is
  then driven exactly like an embedded `target↖…↗` quote (one whole-file
  `EmbeddedFragment`): in-process tree-sitter highlighting, and downstream
  analysis where the target has a per-file server (`*.wgsl.tmpl.quilt` →
  wgsl-analyzer). `document_symbol` lists the template's parameters
  (`sky_param_holes`). Routing *host* targets (`*.py.tmpl.quilt` → pyright,
  `*.rs.tmpl.quilt` → rust-analyzer) and directory-level awareness remain
  follow-ups (issue #117).

### Design notes — semantic tokens

Constraints learned while building the token pipeline (June 2026):

- **Positional and whole-document features degrade differently.** Hover and
  go-to-definition route one cursor position to the right server (ground
  projection, or a per-fragment `FragmentDoc` for embedded languages), so they
  work for embedded quotes "for free". Semantic tokens are whole-document:
  every contributing source must merge into one stream under one legend.
- **wgsl-analyzer provides no semantic tokens at all** (`semanticTokensProvider`
  is `None` upstream — wgsl-analyzer/wgsl-analyzer#342; its own VS Code
  extension highlights via a static TextMate grammar). Embedded-fragment
  tokens therefore come from in-process tree-sitter highlighting (`tshl.rs`),
  which also works when no WGSL server is installed.
- **A fragment language needs no server at all.** html/bash/zsh/nix adapters
  return `None` from `server_command`: `embedded_sync` skips their `didOpen`
  (`ensure_embedded_child` bails before spawning), and the fragments exist
  purely so `tshl.rs` can highlight them. Their queries come from the grammar
  crates' own consts (`HIGHLIGHTS_QUERY`/`HIGHLIGHT_QUERY`), so only WGSL's
  stays vendored.
- **Legends merge by appending.** The editor accepts one legend per
  registration. Advertising the downstream server's legend with our
  tree-sitter token-type names appended — never reordered — lets downstream
  `data` arrays pass through untouched while fragment tokens resolve by name
  (`SemtokRegistration::type_index`, set together with the registered legend).
- **A legend-less ground server still needs a registration.** pyright never
  supplies a legend, so waiting for one would leave `.py.quilt` files with no
  semantic tokens at all (the editor only requests them after we register).
  Opening a host document whose language has a tree-sitter highlighter
  therefore registers a *fallback* legend (the tree-sitter token types alone),
  which is upgraded — unregister, re-register, refresh — when the first real
  downstream legend arrives, so opening a `.py.quilt` before a `.rs.quilt`
  doesn't pin a legend that would mis-index rust-analyzer's pass-through
  token data.
- **Highlight-query conventions vary** — in two ways. Naming: the forks'
  queries use nvim capture names (`@conditional`, `@storageclass`). Ordering:
  the vendored WGSL query is nvim-flavored (specific patterns first, catch-all
  `(identifier) @variable` last — first pattern wins), while the Python fork's
  own query is upstream-flavored (catch-all first, later patterns override —
  last wins); each `Highlighter` declares its `Order`. Nested ranges resolve
  to the *narrowest* span either way, so emitted tokens never overlap. The
  WGSL query is vendored under `quilt-lsp/queries/` because that fork's Rust
  binding exposes no `HIGHLIGHTS_QUERY` const; the Python binding has one.
- **Downstream capability gaps are harmless for rust-analyzer.** quilt-lsp
  advertises no `semanticTokens` client capability to children; rust-analyzer
  returns identical tokens and legend either way (verified empirically).
- **TextMate stays as the base layer.** The VS Code extension injects
  `source.wgsl` / `source.rust` / … into annotated quotes; semantic tokens
  override where they exist, and the static grammar fills the gaps (including
  the warm-up window before a downstream server first responds).

## Not yet implemented

- Hover/definition for ground code spliced into quotes via `↙…↘`.
- `↙name↘` → go-to-definition in the ground language.
- Python ground diagnostics (needs a type-aware quote placeholder; see above).
- Sky-first templates whose target is a *host* language (`*.py.tmpl.quilt`,
  `*.rs.tmpl.quilt`) are highlighted in-process but not yet routed to a
  downstream ground server, and `*.tree.<host>.quilt` scaffold programs / whole
  template *directories* have no special awareness (issue #117).

## Configuration

### Environment variables

| Variable                  | Description                                                                                             |
|---------------------------|---------------------------------------------------------------------------------------------------------|
| `QUILT_LSP_RUST_ANALYZER` | Override the downstream Rust server command (whitespace-separated). Default: `rust-analyzer` on `PATH`. |
| `QUILT_LSP_PYTHON_SERVER` | Override the downstream Python server command. Default: `pyright-langserver --stdio` on `PATH`.         |
| `QUILT_LSP_WGSL_SERVER`   | Override the downstream WGSL server command. Default: `wgsl-analyzer` on `PATH`.                        |
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
python3 quilt-lsp/tests/integration_python.py  target/debug/quilt-lsp  # .py.quilt ground (mock + pyright)
```

## Installing

```sh
cargo install --path quilt-lsp
# puts `quilt-lsp` on your PATH
```

Or run `bin/install_tools`, which installs the LSP and the VS Code extension together (see [Editor Setup](editor-setup.md)).
