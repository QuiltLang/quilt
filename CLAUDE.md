# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Cargo commands run from the repo root (the Cargo workspace root). The `bin/` scripts work from anywhere when the direnv env is active.

```sh
# Build / test / lint / format (from repo root)
cargo build
cargo test
cargo test -p quilt node   # single test
cargo clippy
cargo fmt

# Expand a .quilt file (bin/quilt wraps `cargo run -p quilt --`)
quilt expand path/to/file.rs.quilt
quilt expand path/to/file.py.quilt

# Run a .quilt file directly (also usable as a shebang: #!/usr/bin/env quilt run).
# Defaults to the Omni (production) multi; pass `-m bootstrap` for the bootstrap one.
quilt run path/to/script.rs.quilt   # rust-script runner
quilt run path/to/script.py.quilt   # python3 runner (needs `bin/build-py` first)

# Build the quilt_python PyO3 module (the runtime .py.quilt files target).
# Required once before running .py.quilt files; rebuild after editing the bindings.
build-py

# Bootstrap — regenerates quilt/src/langs/rust/meta.rs from mk_meta.rs.quilt.
# Two stages: bootstrap0 expands with BootstrapMetaLanguage, bootstrap1 with the
# generated RustMetaLanguage (self-hosting). `bootstrap` runs both in order; a
# clean run leaves meta.rs unchanged.
bootstrap     # = bootstrap0 then bootstrap1
bootstrap0    # BootstrapMetaLanguage only
bootstrap1    # RustMetaLanguage only (self-hosted)

# Regenerate the tree-sitter-quilt parser after editing grammar.js
ts-gen
```

## Architecture

Quilt is a multi-stage, multi-language metaprogramming system. A `.quilt` file is source code in some language (e.g. Rust, Python) with Unicode arrow brackets spliced in to embed quoted/unquoted fragments of other languages. The system parses these files, produces a `QTerm` IR, and can expand the IR back into ordinary source code.

### Core IR: `QTerm` (`src/qterm.rs`)

The central type. An enum with three variants:
- `Tuple { tag, terms, cmds }` — an AST node for a specific language. `tag` is the tree-sitter node kind (e.g. `"block"`, `"expression_statement"`). `cmds` is a sequence of `StrCmd`s (write/newline/push-prefix/pop-prefix) with holes (`CmdOrHole`) that interleave the children when serializing.
- `Quote { tag, index, lang, term, cmds }` — a quoted fragment: `↖...↗` or `lang↖...↗`. `index` tracks quasi-quote nesting depth.
- `Unquote { tag, index, lang, term, cmds }` — an unquoted splice: `↙...↘` or `lang↙...↘`.

`QTermBuilder` (`tb/qb/ub` constructors) is the builder API: chain `.w()`, `.c()`, `.n()`, `.p()`, `.x()`, `.b()` for write/child/newline/push/pop/build.

### Surface syntax: `Node` (`src/node.rs`)

The Quilt-level AST parsed by tree-sitter-quilt. Contains `Content`, `NewLine`, `Quote { anno, nodes }`, `Unquote { anno, nodes }`, `Lift` (↑), `Reduce` (↓), `Emit` (←), `Type` (⟨T⟩), `Name` (⟨N⟩). The quilt grammar lives in `rust/tree-sitter-quilt/grammar.js`.

### Language traits (`src/lang.rs`, `src/meta.rs`)

Two trait families:

**`Language` / `LanguagePost`** — parses a flat sequence of `FlatNode`s (strings and holes) into a `QTerm` via a two-phase parse:
- `parse_pre(ikind, code)` → `LanguagePost` (parse with placeholder holes)
- `parse_post(plugs)` → `Arc<QTerm>` (substitute real child terms into holes)

**`MetaLanguage`** — controls how `QTerm`s are expanded during macro expansion:
- `expand_quote`, `expand_unquote`, `expand_tuple` — the three cases of the expander
- `wrap_child` — optionally wraps an expanded child (used for emit/splice)

### The multi-language engine: `Multi<LS, MS>` (`src/multi.rs`)

`Multi` holds a `Languages` registry and a `MetaLanguages` registry. Key entry points:
- `parse_lang(lang, src)` — parses a `.quilt` source string into a `QTerm` tree by recursively descending through nested quote/unquote brackets, dispatching each fragment to the appropriate `Language`.
- `expand_lang(lang, qterm)` — expands a `QTerm` to a flat `QTerm` (no `Quote`/`Unquote` nodes) using the `MetaLanguage` for the outermost language.

`Expander` inside `multi.rs` is the recursive expansion engine. `Stage` tracks quasi-quote depth: `Ground` (running code) vs `Sky(lang, depth)` (inside quotes).

### Heterogeneous lifting (`src/lift.rs`)

`↑` is target-directed: `MetaLanguage::lift_str(target)` picks the spelling, where `target` defaults to the language of the enclosing quote (threaded through `build_nodes` as `splice_target`). Rust's spellings (`langs::rust::ops::lift_spelling`) are `qlift()` for rust→rust and `qlift_to::<Wgsl>()` for rust→wgsl. `src/lift.rs` (always compiled, no parser deps — wasm consumers use it) defines `LiftTo<L>` keyed by marker types (`Rust`, `Wgsl`) plus the `QLiftTo` postfix helper; per-(type, language) impls own the target's tags and spellings (e.g. `LiftTo<Wgsl> for u32` → `leaf("int_literal", "3u")`).

### Concrete languages (`src/langs/`)

Each language module (python, rust, text, bootstrap) provides:
- `lang.rs` — implements the `Language` trait. There is no hard dependency on tree-sitter; a language can implement `Language` directly. The current languages (python, rust, text) happen to use the `TSLanguage<P: TSProvider>` helper (`src/treesitter.rs`), which wraps a tree-sitter parser. `TSProvider` supplies the parser, the hole placeholder string (`{}` for Rust, `__HOLE__` for Python), and an `unwrap` method that squashes the tree-sitter root and infers `InnerKind` (Expr/Stmt/File). The bootstrap language implements `Language` directly without tree-sitter.
- `meta.rs` — implements `MetaLanguage`. Rust's is **generated** by bootstrap from `mk_meta.rs.quilt`; python/text/bootstrap are hand-written. The `expand_*` methods are thin wrappers that delegate to `ops.rs`, and each meta also supplies the operator spellings (`lift_str`/`reduce_str`/`emit_str`/`type_str`/`name_str`) that the `↑ ↓ ← ⟨T⟩ ⟨N⟩` glyphs expand to.
- `ops.rs` (rust and python) — hand-written helpers that build the output `QTerm` **directly** via the builder: `build_tuple_code` / `build_quote_code` / `build_unquote_code` / `build_variadic_block`, plus `name` (and, for rust, `qlift` and `reduce`). Bootstrap's analogue is `strlift.rs`, which instead lifts to a string and re-parses it — a slower shortcut used only for bootstrapping.
- `mod.rs` — re-exports types.

`src/langs/omni.rs` defines `Omni` (the default `Multi` used by the CLI) using enum-dispatch over all enabled languages.

### Bootstrap (`src/langs/bootstrap/`)

A two-step self-hosting process that generates `src/langs/rust/meta.rs`:
1. **Step 1** — expands `mk_meta.rs.quilt` (using `Bootstrap` multi) → `mk_meta.rs`
2. **Step 2** — runs the compiled `mk_meta.rs` code to produce and write `meta.rs`, then `cargo fmt`s it

`mk_meta.rs.quilt` is a Rust source file that uses `⟨T⟩` (type placeholder) to refer to `Arc<QTerm>` without hard-coding it.

### Output: `StrCmd` / `PrefixWriter` (`src/strcmd.rs`)

Serialization is driven by a stack-based `StrCmd` sequence embedded in each `QTerm`. `PrefixWriter` maintains an indentation prefix stack; `StrCmd::NewLine` emits a newline then the current prefix.

### Tree-sitter grammars (`rust/tree-sitter-*/`)

- `tree-sitter-quilt` — the Quilt bracket language (arrow brackets and special symbols). Source in `grammar.js`; generated C parser in `src/parser.c`.
- `tree-sitter-rust` and `tree-sitter-python` — forked from upstream, modified to support hole nodes (`{}` and `__HOLE__` respectively) as valid parse-tree positions.

### Other crates

- `quilt_python` — PyO3 bindings exposing quilt's core IR (`QTerm`, the fluent `tb/.c/.w/.n/.p/.x/.e/.b` builder, `leaf/sym/quote/unquote/cmd/write/push/name/qlift`, `NL/POP/HOLE`, and `.coparse()`) to Python. This is the runtime that expanded `.py.quilt` files target (`PythonMetaLanguage` emits calls into it). The Cargo crate is `quilt_python`, but the Python import name is **`quilt`** (`from quilt import *`): a `quilt/` package whose `__init__.py` re-exports the native `quilt._quilt` module. Built abi3 (one `.so` for CPython ≥3.8) via `bin/build-py`; `quilt run` puts it on `PYTHONPATH` for `python3` runs. See `examples/hello.py.quilt`.

The `rust/` directory also holds `s2s` (a ratatui TUI), `sharede` (shared/deduplicated data structures), `nanobots_old` (sandbox), and `quilt_old`, but these are **not** in the Cargo workspace — `rust/Cargo.toml` lists only `quilt`, `quilt_python`, `tree-sitter-quilt`, so `cargo build`/`test` ignore them.

### Clippy configuration

The workspace enables `clippy::pedantic` but suppresses several lints globally (see `rust/Cargo.toml`). Run `cargo clippy` to check.
