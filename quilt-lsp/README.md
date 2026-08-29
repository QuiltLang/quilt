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
  a `quilt::node::scan`, on every `.quilt` file.
- **Full Rust support for the ground language** via a downstream `rust-analyzer`:
  hover, go-to-definition, completion, and diagnostics, with positions mapped
  between the `.quilt` file and the projected `.rs` virtual document.
- **Python as a ground language** via a downstream Python server (pyright by
  default, overridable with `QUILT_LSP_PYTHON_SERVER`): hover, go-to-definition,
  completion, and diagnostics for `.py.quilt` files. Quote placeholders are
  typed by a synthetic *ground prologue* (see below), so ground lines that
  consume a quoted value no longer mistype.
- **Semantic-token highlighting**, including inside `↖…↗` quotes: each quoted
  Rust fragment is appended to the virtual document (wrapped in `fn _quilt_qN`)
  so rust-analyzer tokenizes it; tokens are remapped back onto the quote. When
  the ground server provides no semantic tokens at all (pyright doesn't — a
  Pylance-only feature) the whole ground projection is highlighted in-process
  with tree-sitter instead, so `.py.quilt` files still get code coloring.
- **Folding** for quilt regions plus the ground server's folds.

The ground language is projected by copying its bytes verbatim and replacing
each quilt construct (`↖…↗`, `↙…↘`, and the `↑↓←⟨T⟩⟨N⟩` glyphs) with a small
placeholder; a [`SourceMap`](src/srcmap.rs) records the mapping. A
construct-free file (e.g. `examples/hello.rs.quilt`) projects to itself
byte-for-byte. The projection is opened to rust-analyzer under the *de-quilted*
file URI (`foo.rs`) as an overlay, so it resolves inside the real cargo project.

Diagnostics inside appended quote fragments are suppressed (their wrapping makes
them unreliable); their tokens are kept for highlighting.

### The ground prologue

A placeholder that merely *parses* is not enough for diagnostics: the downstream
server still has to accept the ground lines that consume it.
`MetaLanguageAdapter::ground_prologue` prepends synthetic declarations to the
virtual document for exactly that. Python declares

```python
# pyright: reportWildcardImportFromLibrary=false
import typing as _quilt_typing
__q__: _quilt_typing.Any = ...
```

and projects each quote to `__q__(…)`, carrying any stage-0 `↙…↘` splices as
arguments. `Any` covers every position a placeholder appears in — called
(`↑(x)`, `⟨N⟩(…)`, the splice block), attribute base (`t.↓`), operand, and bare
name. Rust needs no prologue (its `__q__` is an unresolved name, and diagnostics
on it land on synthetic text and are dropped).

The prologue is one synthetic span at the head of the document, so positions map
correctly by construction: it shifts virtual lines by its own length and nothing
else. Diagnostics landing inside it are dropped
(`Projection::is_in_prologue` — checked separately from `is_synthetic`, which by
design only catches non-empty spans).

Measured on the seven `.py.quilt` files in `examples/` (pyright 1.1.x, `quilt`
module built): zero published diagnostics, down from one mistype per
quote-consuming line, while genuine ground errors still surface.

### Why Lean opts out

`LeanAdapter::publishes_diagnostics` is `false`, and unlike Python this is not a
placeholder-typing problem that a prologue can fix. Measured against
`examples/lean_specialize.rs.quilt` (6 Lean quotes, Lean 4.32.1), all 6
fragments error and none of the errors is a missing `import` — so supplying the
enclosing module's imports/`variable`s, the obvious first guess, addresses none
of the real causes. Two of the causes are tractable (a quoted *term* is not a
valid Lean file; `_` is illegal in name position and unsolvable in term
position). Two are not:

- **Free variables bound by the generated context.** `body()` returns
  `lean↖x↗`, where `x` is bound by the `def ↙name↘ (x : Nat)` assembled in a
  *different* function. A `variable (x : Nat)` prologue does fix it, but nothing
  in the source says the fragment lands in that binder — host control flow
  decides at generation time.
- **Spliced names in applied-head position.** `↙name↘ x` needs a placeholder
  Lean will apply, and no type-agnostic term works: `sorry`, an
  `axiom q : ∀ {α : Sort u}, α`, and an `opaque` all give *"Function expected at
  q but this term has type ?m"*. Only the exact arrow type
  (`variable {q : Nat → Nat}`) works, and that type comes from the host program.

Fixing only the tractable two leaves 4 of 6 fragments spuriously red, so Lean
stays opted out. Highlighting, hover and go-to-definition are unaffected.

Not yet implemented (designed-for extension points): hover/definition for ground
code *spliced into* quotes via `↙…↘`, the `↙name↘`→ground go-to-definition, and
diagnostics for a `.lean.quilt` *ground* projection (its `[…]` quote placeholder
mistypes where a `String` is wanted — the same class of problem the Python
prologue solved, but the fix needs a per-body wrapper so the placeholder can
mirror Lean's `s!"{a}{b}"` interpolation rather than a list).

## Architecture

| Module | Responsibility |
|---|---|
| `lineindex` | byte ↔ UTF-16/UTF-8 ↔ `Position` (the one place encoding math lives) |
| `regions` | `quilt::node::scan` → region tree + syntax errors |
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
