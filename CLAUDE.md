# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Cargo commands run from the repo root (the Cargo workspace root). The `bin/` scripts work from anywhere when the direnv env is active.

```sh
# Build / test / lint / format (from repo root)
cargo build
cargo test                 # or `ctest` (wrapper that works from anywhere)
cargo test -p quilt node   # single test
cargo clippy               # or `lint` (adds --tests)
cargo fmt

# Expand a .quilt file (bin/quilt wraps `cargo run -p quilt --`)
quilt expand path/to/file.rs.quilt
quilt expand path/to/file.py.quilt
quilt expand path/to/shaders.wgsl.rs.quilt   # language chain, see below

# Run a .quilt file directly (also usable as a shebang: #!/usr/bin/env quilt run).
# Defaults to the Omni (production) multi; pass `-m bootstrap` for the bootstrap one.
quilt run path/to/script.rs.quilt   # rust-script runner
quilt run path/to/script.py.quilt   # python3 runner (needs `bin/build-py` first)

# Build the quilt_python PyO3 module (the runtime .py.quilt files target).
# Required once before running .py.quilt files; rebuild after editing the bindings.
build-py

# Bootstrap — regenerates quilt/src/langs/rust/meta.rs from mk_meta.rs.quilt.
# Both stages `quilt run` mk_meta.rs.quilt (which writes meta.rs): bootstrap0
# expands it with the BootstrapMetaLanguage (`-m bootstrap`, feature `bootstrap`),
# bootstrap1 with the freshly generated RustMetaLanguage (`-m omni`, self-hosting).
# `bootstrap` runs both in order; a clean run leaves meta.rs unchanged.
bootstrap     # = bootstrap0 then bootstrap1
bootstrap0    # BootstrapMetaLanguage only
bootstrap1    # RustMetaLanguage only (self-hosted)

# Regenerate the tree-sitter-quilt parser after editing grammar.js
ts-gen
```

The file stem determines the **language chain**: reading the extensions right-to-left, the rightmost is the ground language and the rest are the default languages for nested un-annotated quotes — `shaders.wgsl.rs.quilt` → ground `rs`, un-annotated quotes default to `wgsl` (see `lang_chain` in `quilt/src/bin.rs`).

## Workspace layout

Workspace members (root `Cargo.toml`): `quilt` (core library + CLI), `quilt-lsp` (LSP server), `quilt-python` (PyO3 bindings; Cargo crate `quilt_python`), `tree-sitter-quilt` (grammar for the quilt bracket language). The other grammars (`tree-sitter-rust`, `-python`, `-html`, `-wgsl`, `-bash`, `-zsh`) are pulled as git dependencies from their forks under `github.com/QuiltLang` — they are *not* in this repo. Non-crate directories: `bin/` (helper scripts), `tools/quilt/` (VS Code extension), `docs/wiki/` (documentation wiki), `examples/`, `nix/` + `.envrc` (direnv environment).

The `nanobots` project (gas-metered state-machine toolchain) lives in a **sibling repo** (`../nanobots`); it consumes quilt as a library (see Feature flags below).

## Architecture

Quilt is a multi-stage, multi-language metaprogramming system. A `.quilt` file is source code in some language (e.g. Rust, Python) with Unicode arrow brackets spliced in to embed quoted/unquoted fragments of other languages. The system parses these files, produces a `QTerm` IR, and can expand the IR back into ordinary source code. The `docs/wiki/` pages cover all of this in more depth.

### Core IR: `QTerm` (`quilt/src/qterm.rs`)

The central type. An enum with three variants:
- `Tuple { tag, terms, cmds }` — an AST node for a specific language. `tag` is the tree-sitter node kind (e.g. `"block"`, `"expression_statement"`). `cmds` is a sequence of `StrCmd`s (write/newline/push-prefix/pop-prefix) with holes (`CmdOrHole`) that interleave the children when serializing.
- `Quote { tag, index, lang, term, cmds }` — a quoted fragment: `↖...↗` or `lang↖...↗`. `index` tracks quasi-quote nesting depth.
- `Unquote { tag, index, lang, term, cmds }` — an unquoted splice: `↙...↘` or `lang↙...↘`.

`QTermBuilder` (`tb/qb/ub` constructors) is the builder API: chain `.w()`, `.c()`, `.n()`, `.p()`, `.x()`, `.b()` for write/child/newline/push/pop/build.

Supporting modules: `term.rs` (the generic `Term` trait, `ArcTerm`, `STerm`), `validate.rs` (the `Validate` trait), `zipper.rs` (persistent list/zipper utilities), `strcmd.rs` (serialization, below).

### Surface syntax: `Node` (`quilt/src/node.rs`)

The Quilt-level AST parsed by tree-sitter-quilt. Contains `Content`, `NewLine`, `Quote { anno, nodes }`, `Unquote { anno, nodes }`, `Lift` (↑), `Reduce` (↓), `Emit` (←), `Type` (⟨T⟩), `Name` (⟨N⟩). The quilt grammar lives in `tree-sitter-quilt/grammar.js`.

### Language traits (`quilt/src/lang.rs`, `quilt/src/meta.rs`)

Two trait families:

**`Language` / `LanguagePost`** — parses a flat sequence of `FlatNode`s (strings and holes) into a `QTerm` via a two-phase parse:
- `parse_pre(ikind, code)` → `LanguagePost` (parse with placeholder holes)
- `parse_post(plugs)` → `Arc<QTerm>` (substitute real child terms into holes)

**`MetaLanguage`** — controls how `QTerm`s are expanded during macro expansion:
- `expand_quote`, `expand_unquote`, `expand_tuple` — the three cases of the expander
- `wrap_child` — optionally wraps an expanded child (used for emit/splice)

### The multi-language engine: `Multi<LS, MS>` (`quilt/src/multi.rs`)

`Multi` holds a `Languages` registry and a `MetaLanguages` registry. Key entry points:
- `parse_lang(lang, src)` — parses a `.quilt` source string into a `QTerm` tree by recursively descending through nested quote/unquote brackets, dispatching each fragment to the appropriate `Language`.
- `parse_chain(chain, src)` — like `parse_lang`, but takes the language chain derived from the file stem (the CLI uses this).
- `expand_lang(lang, qterm)` — expands a `QTerm` to a flat `QTerm` (no `Quote`/`Unquote` nodes) using the `MetaLanguage` for the outermost language.

`Expander` inside `multi.rs` is the recursive expansion engine. `Stage` tracks quasi-quote depth: `Ground` (running code) vs `Sky(lang, depth)` (inside quotes).

### Heterogeneous lifting (`quilt/src/lift.rs`)

`↑` is target-directed: `MetaLanguage::lift_str(target)` picks the spelling, where `target` defaults to the language of the enclosing quote (threaded through `build_nodes` as `splice_target`). Rust's spellings (`langs::rust::ops::lift_spelling`) are `qlift()` for rust→rust and `qlift_to::<Wgsl>()` for rust→wgsl. `lift.rs` (always compiled, no parser deps — wasm consumers use it) defines `LiftTo<L>` keyed by marker types (`Rust`, `Wgsl`, `Bash`, `Zsh`) plus the `QLiftTo` postfix helper; per-(type, language) impls own the target's tags and spellings (e.g. `LiftTo<Wgsl> for u32` → `leaf("int_literal", "3u")`).

### Concrete languages (`quilt/src/langs/`)

**Host languages** (rust, python) provide:
- `lang.rs` — implements the `Language` trait. There is no hard dependency on tree-sitter; a language can implement `Language` directly. The tree-sitter-backed languages use the `TSLanguage<P: TSProvider>` helper (`quilt/src/treesitter.rs`), which wraps a tree-sitter parser. `TSProvider` supplies the parser, the hole placeholder string (`{}` for Rust, `__HOLE__` for Python), and an `unwrap` method that squashes the tree-sitter root and infers `InnerKind` (Expr/Stmt/File).
- `meta.rs` — implements `MetaLanguage`. Rust's is **generated** by bootstrap from `mk_meta.rs.quilt`; python's is hand-written. The `expand_*` methods are thin wrappers that delegate to `ops.rs`, and each meta also supplies the operator spellings (`lift_str`/`reduce_str`/`emit_str`/`type_str`/`name_str`) that the `↑ ↓ ← ⟨T⟩ ⟨N⟩` glyphs expand to.
- `ops.rs` — hand-written helpers that build the output `QTerm` **directly** via the builder: `build_tuple_code` / `build_quote_code` / `build_unquote_code` / `build_variadic_block`, plus `name` (and, for rust, `qlift` and `reduce`).

**Target-only languages** (wgsl, html, zsh, bash) provide just `lang.rs` — they can be quoted (`wgsl↖...↗`) but have no `MetaLanguage`, so the host's meta drives expansion. **Text** additionally has a minimal hand-written `meta.rs`.

**Bootstrap** (`langs/bootstrap/`) is internal-only: it implements `Language` directly without tree-sitter, and its meta uses `strlift.rs`, which lifts to a string and re-parses it — a slower shortcut used only for bootstrapping.

`langs/omni.rs` defines `Omni` (the default `Multi` used by the CLI) using enum-dispatch over all enabled languages. Registry keys: `rust`/`rs`, `python`/`py`, `text`/`txt`, `wgsl`, `html`, `zsh`, `bash`.

### Feature flags

Each language is gated behind a Cargo feature (see `quilt/Cargo.toml`); all are on by default. The `parse` feature gates tree-sitter (the Quilt-source parser, the `Language` providers, `omni`, and `Multi`'s parse path). The runtime that expanded code targets (the `QTerm` builders, `qlift`, `coparse`) is tree-sitter-free, so consumers like `nanobots-codegen` depend on quilt with `default-features = false, features = ["rust"]` and build for `wasm32-unknown-unknown` without the tree-sitter C runtime.

### Bootstrap (`quilt/src/langs/bootstrap/`)

A two-stage self-hosting process that regenerates `quilt/src/langs/rust/meta.rs`. Both stages `quilt run` the same program, `mk_meta.rs.quilt`, which produces and writes `meta.rs` (then `cargo fmt`s it):
1. **bootstrap0** — expands it with the `Bootstrap` multi (`BootstrapMetaLanguage`, which works without `meta.rs`)
2. **bootstrap1** — expands it with the `Omni` multi, i.e. the freshly generated `RustMetaLanguage` (self-hosting); a clean run leaves `meta.rs` unchanged

`mk_meta.rs.quilt` is a Rust source file that uses `⟨T⟩` (type placeholder) to refer to `Arc<QTerm>` without hard-coding it.

### Output: `StrCmd` / `PrefixWriter` (`quilt/src/strcmd.rs`)

Serialization is driven by a stack-based `StrCmd` sequence embedded in each `QTerm`. `PrefixWriter` maintains an indentation prefix stack; `StrCmd::NewLine` emits a newline then the current prefix.

### Other crates

- `quilt-lsp` — a multiplexing Language Server for `.quilt` files (tower-lsp). It parses the quilt structure, projects each language into a virtual document, proxies LSP traffic to per-language downstream servers (currently `rust-analyzer` for the ground language), and remaps positions in both directions. See `quilt-lsp/README.md` and `docs/wiki/lsp.md`.
- `quilt-python/` (crate `quilt_python`) — PyO3 bindings exposing quilt's core IR (`QTerm`, the fluent `tb/.c/.w/.n/.p/.x/.e/.b` builder, `leaf/sym/quote/unquote/cmd/write/push/name/qlift`, `NL/POP/HOLE`, and `.coparse()`) to Python. This is the runtime that expanded `.py.quilt` files target (`PythonMetaLanguage` emits calls into it). The Python import name is **`quilt`** (`from quilt import *`): a `quilt/` package whose `__init__.py` re-exports the native `quilt._quilt` module. Built abi3 (one `.so` for CPython ≥3.8) via `bin/build-py` (maturin); `quilt run` puts it on `PYTHONPATH` for `python3` runs. See `examples/hello.py.quilt`.
- `tree-sitter-quilt` — the Quilt bracket language (arrow brackets and special symbols). Source in `grammar.js`; regenerate the parser with `ts-gen`.

### Clippy configuration

The workspace enables `clippy::pedantic` but suppresses several lints globally (see `[workspace.lints.clippy]` in the root `Cargo.toml`). Run `cargo clippy` (or `bin/lint`) to check.
