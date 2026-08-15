# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Work modes

A prompt may select a work mode with `mode=<name>` (e.g. `mode=merge`). If no
mode is given, use `mode=pr`.

**Always work in a dedicated git worktree — never check out a branch in the
main working directory.** Create a fresh worktree for every task (e.g. via the
`EnterWorktree` tool, or `git worktree add`) rather than `git checkout`/`git
switch`-ing branches in place. This keeps `main` clean and lets work proceed in
isolation.

| Mode                | Delivery                       | Questions                                                |
| ------------------- | ------------------------------ | -------------------------------------------------------- |
| `pr` **(default)**  | Cut a PR                       | Ask as necessary                                         |
| `fast+pr`           | Cut a PR                       | Don't stop — leave them as comments on the PR            |
| `merge`             | Merge to `main` without asking | Ask as necessary                                         |
| `fast`              | Merge to `main` without asking | Don't stop — file them as GitHub issues to discuss later |

Details:

- **`mode=pr`** — The default. Work in a worktree and cut a PR. Ask questions as
  necessary.
- **`mode=fast+pr`** — Like `pr`, but don't stop to ask questions; put them in
  the PR as comments.
- **`mode=merge`** — Work in a worktree, then merge to `main` without asking.
  Resolve any merge conflicts as necessary. Ask questions as necessary.
- **`mode=fast`** — Like `merge`, but don't stop to ask questions. File them as
  GitHub issues to discuss later, labeled `question`.

In every mode, get `main check` green before merging or cutting a PR — it runs
the same `bin/` gates the CI matrix does (see Commands below).

## Commands

`main` is the front door: `main` on its own lists everything, and any other
`bin/` script is reachable as `main <script>`. The `bin/` scripts also work
directly, from anywhere, when the direnv env is active. Cargo commands run from
the repo root (the Cargo workspace root).

```sh
main                  # what's available
main run <file>       # run a .quilt file      (= quilt <file>)
main expand <file>    # expand a .quilt file   (= quilt expand <file>)
main build            # cargo build
main test [args]      # cargo test             (= ctest)
main lint             # cargo clippy --tests   (= lint)
main fmt              # cargo fmt --all
main check            # pre-commit gate: fmt, clippy, tests, bootstrap,
                      #   quilt grammar, arity tables, lift shapes,
                      #   support matrix, examples
main check --all      # …plus feature matrix, vendored grammars, the python
                      #   and typescript runtimes
main <script> [args]  # any other bin/ script, e.g. `main sync-grammars`

# Build / test / lint / format (from repo root)
cargo build
cargo test                 # or `ctest` (wrapper that works from anywhere)
cargo test -p quiltlang node   # single test
cargo clippy               # or `lint` (adds --tests)
cargo fmt                  # `fmt-check` verifies formatting without writing (CI)

# Expand a .quilt file (bin/quilt wraps `cargo run -p quiltlang --`)
quilt expand path/to/file.rs.quilt
quilt expand path/to/file.py.quilt
quilt expand path/to/shaders.wgsl.rs.quilt   # language chain, see below

# Validate .quilt files (parse + expand, no output written) — for CI / pre-commit
quilt check path/to/file.rs.quilt path/to/other.py.quilt

# Run a .quilt file directly. `run` is the default subcommand, so it can be
# omitted (which is what makes the `#!/usr/bin/env quilt` shebang work).
# Defaults to the Omni (production) multi; pass `-m bootstrap` for the bootstrap one.
quilt path/to/script.rs.quilt       # rust-script runner
quilt path/to/script.py.quilt       # python3 runner (needs `bin/build-py` first)
quilt path/to/script.ts.quilt       # node runner (needs `bin/build-ts` first)

# Build the quilt_python PyO3 module (the runtime .py.quilt files target).
# Required once before running .py.quilt files; rebuild after editing the bindings.
build-py
test-py       # pytest the bindings + `quilt run` the .py.quilt examples

# Build the quilt-wasm runtime for node (what .ts.quilt files target on the CLI).
# Required once before running .ts.quilt files; rebuild after editing the bindings.
# `quilt run` binds the bare `quilt` import to quilt-wasm/node, which adds `↓`
# (reduce) on top of it by shelling back out to the expander — issue #153.
build-ts
test-ts       # node tests for `↓` + `quilt run` the .ts.quilt examples

# Bootstrap — regenerates quilt/src/langs/rust/meta.rs from mk_meta.rs.quilt.
# Both stages run mk_meta.rs.quilt via quilt (which writes meta.rs): bootstrap0
# expands it with the BootstrapMetaLanguage (`-m bootstrap`, feature `bootstrap`),
# bootstrap1 with the freshly generated RustMetaLanguage (`-m omni`, self-hosting).
# `bootstrap` runs both in order; a clean run leaves meta.rs unchanged.
bootstrap     # = bootstrap0 then bootstrap1
bootstrap0    # BootstrapMetaLanguage only
bootstrap1    # RustMetaLanguage only (self-hosted)
check-bootstrap   # run bootstrap and fail if meta.rs changed (CI / pre-commit)

# Regenerate the tree-sitter-quilt parser after editing grammar.js
ts-gen
# Run the quilt grammar corpus + verify src/parser.c matches grammar.js (CI).
# The corpus lives in tree-sitter-quilt/test/corpus/; `tree-sitter test --update`
# fills in expected trees, but review them — an --update accepts whatever the
# parser currently does, right or wrong.
check-grammar-quilt

# Language support matrix (issue #144). Capability claims live in
# conformance/spec/<lang>.toml; the battery in quilt-conformance/ verifies each
# one and regenerates conformance/support-matrix.json (rendered by the website's
# support table) + docs/wiki/support-matrix.md.
gen-matrix        # verify claims, rewrite both artifacts
check-matrix      # gen-matrix + fail if either artifact drifted (CI / pre-commit)
cargo test -p quilt-conformance          # the battery; one test per language
cargo test -p quilt-conformance rust     # a single language

# Variadic-container tables (issue #202). Which node kinds can hold a variable
# number of children — i.e. where emit (`←`) may splice a sequence — is derived
# from each grammar's `REPEAT` rules rather than hand-curated per language. The
# derivation lives in quilt-conformance/src/arity.rs; it reads the grammar.json
# that bin/sync-grammars vendors alongside each parser.c, and writes the tables
# to quilt/src/langs/arity.rs, where every `Language::arity` reads them.
gen-arity         # rewrite quilt/src/langs/arity.rs from the grammars
check-arity       # gen-arity + fail if the file drifted (CI / pre-commit)

# Lift shapes (issue #203). Every `LiftTo` impl — the shape `↑` builds when it
# lifts a Rust value into a quote of another language — is derived from a sample
# literal parsed by that language's vendored grammar, rather than transcribed
# from parser output by hand (which is how #176 and fed278b happened). The
# generator is quilt itself: `quilt/src/lift/mk_lifts.rs.quilt` parses e.g.
# `py↖"s"↗`, lifts the term with `↑` to get the builder calls that rebuild it,
# and substitutes the sample's own text for the runtime expression.
gen-lifts         # rewrite quilt/src/lift/gen.rs from the sample literals
check-lifts       # gen-lifts + fail if the file drifted (CI / pre-commit)

# Expander output is snapshotted, not pinned with inline string literals
# (issue #157), so a deliberate change to generated code is a bulk review
# rather than N hand edits. Snapshots live in quilt/tests/snapshots/.
cargo insta review               # accept/reject changed snapshots interactively
INSTA_UPDATE=always cargo test   # accept everything (use when you mean it)
# CI uploads *.snap.new as a workflow artifact when a snapshot check fails.

# Runtime parity (issue #159): drive all three published runtimes — quiltlang,
# quilt-python, quilt-wasm — through one shared corpus
# (conformance/runtime/cases.json), so a divergence between them is a test
# failure. Slow (builds a PyO3 cdylib + a wasm artifact), so CI runs it nightly;
# the Rust third also runs in the normal `cargo test`.
test-runtimes

# Feature combinations that are contracts, not defaults (issue #162): quiltlang
# with default-features = false must stay runtime-only (no tree-sitter) so
# quilt-wasm and the sibling nanobots-codegen can build it for
# wasm32-unknown-unknown. Also builds each language feature alone.
check-features

# Properties and fuzzing (issue #161) — the invariants the battery asserts for
# a handful of spec values, re-stated over generated input.
#
# Properties live in quilt-conformance/tests/properties.rs and are driven by the
# same conformance/spec/*.toml corpora, so a new language gets them from its
# spec file. They run in the normal `cargo test` at a few hundred cases each;
# the nightly job re-runs them at 20000. Failures shrink to a minimal case and
# are saved to quilt-conformance/tests/proptest-regressions/.
PROPTEST_CASES=20000 cargo test -p quilt-conformance --test properties

# cargo-fuzz targets over the Quilt surface syntax and the expander: arbitrary
# input must return Err, never panic. Nightly + on demand, never a gate — it
# needs a *nightly* toolchain (libFuzzer wants -Zsanitizer) while the repo is
# pinned to stable, so it is the one thing not runnable from the devShell as-is:
#   cargo install cargo-fuzz --locked && rustup toolchain install nightly
fuzz              # every target, 60s each
fuzz 300          # every target, 300s each
fuzz parse_quilt  # one target
fuzz list         # the targets

# Downstream smoke build (issue #189): clone QuiltLang/nanobots, repoint its
# quilt dependency at this checkout, expand its .rs.quilt sources with this
# expander and compile the two crates that consume quilt. Nightly and
# informational — it depends on a repo whose state we do not control.
check-downstream            # clone to a temp dir, build, clean up
check-downstream DIR        # use DIR (cloned if empty, reused if not)

# Build/install the editor tooling: cargo-installs quilt-lsp, npm-installs the
# VS Code extension, symlinks tools/quilt into ~/.vscode/extensions
install_tools
```

The file stem determines the **language chain**: reading the extensions right-to-left, the rightmost is the ground language and the rest are the default languages for nested un-annotated quotes — `shaders.wgsl.rs.quilt` → ground `rs`, un-annotated quotes default to `wgsl` (see `lang_chain` in `quilt/src/bin.rs`).

## Workspace layout

Workspace members (root `Cargo.toml`): `quilt` (core library + CLI; Cargo package `quiltlang` with `[lib] name = "quilt"` — `quilt` is taken on crates.io), `quilt-lsp` (LSP server), `quilt-conformance` (dev-only capability-matrix harness, `publish = false`; in `default-members` so plain `cargo test` runs it), `quilt-python` (PyO3 bindings; Cargo crate `quilt_python`), `tree-sitter-quilt` (grammar for the quilt bracket language). The other grammars (`tree-sitter-rust`, `-python`, `-typescript`, `-html`, `-wgsl`, `-bash`, `-zsh`, `-nix`, `-lean`, `-sql`) live in forks under `github.com/QuiltLang` (pinned by rev in the root `Cargo.toml` `[workspace.dependencies]`). `quilt` does **not** depend on them as crates: it vendors their generated parsers under `quilt/grammars/<lang>/` and compiles them in `build.rs` (so `quiltlang` has no git deps and can publish to crates.io — issue #32). The vendored copies are regenerated from the pinned forks with `bin/sync-grammars` (the forks stay the canonical source; it also vendors each grammar's `highlights.scm` for python/html/bash/zsh/nix/lean/sql, exposed as `quilt::grammars::<lang>::HIGHLIGHTS_QUERY`, and each grammar's `grammar.json`, which `bin/gen-arity` derives the variadic tables from — issue #202); `bin/check-grammars` (CI) fails if they drift. `quilt-lsp` no longer depends on the forks either: it takes its grammar `LANGUAGE`s and highlight queries from the published `quiltlang` (`quilt::grammars`), so it is now crates.io-publishable too. Non-crate directories: `bin/` (helper scripts, fronted by `bin/main`), `tools/quilt/` (VS Code extension), `docs/wiki/` (documentation wiki), `examples/`, `nix/` + `.envrc` (direnv environment, which also puts `bin/` on `PATH`).

The `nanobots` project (gas-metered state-machine toolchain) lives in a **sibling repo** (`../nanobots`); it consumes quilt as a library (see Feature flags below).

## Architecture

Quilt is a polyglot metaprogramming language. A `.quilt` file is source code in some language (e.g. Rust, Python) with Unicode arrow brackets spliced in to embed quoted/unquoted fragments of other languages. The system parses these files, produces a `QTerm` IR, and can expand the IR back into ordinary source code. The `docs/wiki/` pages cover all of this in more depth.

### Core IR: `QTerm` (`quilt/src/qterm.rs`)

The central type. An enum with three variants:
- `Tuple { tag, terms, cmds }` — an AST node for a specific language. `tag` is the tree-sitter node kind (e.g. `"block"`, `"expression_statement"`). `cmds` is a sequence of `StrCmd`s (write/newline/push-prefix/pop-prefix) with holes (`CmdOrHole`) that interleave the children when serializing.
- `Quote { tag, index, lang, term, cmds, span }` — a quoted fragment: `↖...↗` or `lang↖...↗`. `index` tracks quasi-quote nesting depth.
- `Unquote { tag, index, lang, term, cmds, span }` — an unquoted splice: `↙...↘` or `lang↙...↘`.

`span` is the byte range of the quote/unquote in the original source (`Option<Span>`; attached by `build_nodes`, `None` for constructed terms). It is diagnostic metadata only — ignored by `PartialEq` and used to point errors like "unquote depth too high" at the offending source.

`QTermBuilder` (`tb/qb/ub` constructors) is the builder API: chain `.w()`, `.c()`, `.n()`, `.p()`, `.x()`, `.b()` for write/child/newline/push/pop/build.

Supporting modules: `term.rs` (the generic `Term` trait, `ArcTerm`, `STerm`), `validate.rs` (the `Validate` trait), `zipper.rs` (persistent list/zipper utilities), `strcmd.rs` (serialization, below).

### Surface syntax: `Node` (`quilt/src/node.rs`)

The Quilt-level AST parsed by tree-sitter-quilt. Contains `Content`, `NewLine`, `Quote { anno, nodes, span }`, `Unquote { anno, nodes, span }`, `Lift` (↑), `Reduce` (↓), `Emit` (←), `Type` (⟨T⟩), `Name` (⟨N⟩). The quilt grammar lives in `tree-sitter-quilt/grammar.js`.

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

### Heterogeneous lifting (`quilt/src/lift/`)

`↑` is target-directed: `MetaLanguage::lift_str(target)` picks the spelling, where `target` defaults to the language of the enclosing quote (threaded through `build_nodes` as `splice_target`). Rust's spellings (`langs::rust::ops::lift_spelling`) are `qlift()` for rust→rust and `qlift_to::<L>()` for the heterogeneous targets (python, wgsl, zsh, bash, nix, lean, sql). `lift.rs` (always compiled, no parser deps — wasm consumers use it) defines `LiftTo<L>` keyed by marker types (`Rust`, `Python`, `Wgsl`, `Bash`, `Zsh`, `Nix`, `Lean`, `Sql`) plus the `QLiftTo` postfix helper; per-(type, language) impls own the target's tags and spellings (e.g. `LiftTo<Wgsl> for u32` → `leaf("int_literal", "3u")`, `LiftTo<Python> for Vec<T>` → a `list` literal). Those impls live in `lift/gen.rs` and are **generated** (issue #203): `bin/gen-lifts` runs `lift/mk_lifts.rs.quilt`, which parses a sample literal per family with the vendored grammar, lifts it with `↑` — the builder-call source that rebuilds it — and substitutes the sample's text for the runtime expression, so the tags and layout are the parser's rather than a transcription. `bin/check-lifts` is the drift gate. `lift/mod.rs` keeps what is not a shape: markers, the trait, and the `*_dquote_escape` / `sql_squote_escape` rules the generated shapes call.

### Concrete languages (`quilt/src/langs/`)

**Host languages** (rust, python) provide:
- `lang.rs` — implements the `Language` trait. There is no hard dependency on tree-sitter; a language can implement `Language` directly. The tree-sitter-backed languages use the `TSLanguage<P: TSProvider>` helper (`quilt/src/treesitter.rs`), which wraps a tree-sitter parser. `TSProvider` supplies the parser, the hole placeholder string (`{}` for Rust, `__HOLE__` for Python), and an `unwrap` method that squashes the tree-sitter root and infers `InnerKind` (Expr/Stmt/File).
- `meta.rs` — implements `MetaLanguage`. Rust's is **generated** by bootstrap from `mk_meta.rs.quilt`; python's is hand-written. The `expand_*` methods are thin wrappers that delegate to `ops.rs`, and each meta also supplies the operator spellings (`lift_str`/`reduce_str`/`emit_str`/`type_str`/`name_str`) that the `↑ ↓ ← ⟨T⟩ ⟨N⟩` glyphs expand to.
- `ops.rs` — hand-written helpers that build the output `QTerm` **directly** via the builder: `build_tuple_code` / `build_quote_code` / `build_unquote_code` / `build_variadic_block`, plus `name` (and, for rust, `qlift` and `reduce`).

**Target-only languages** (wgsl, html, sql) provide just `lang.rs` — they can be quoted (`wgsl↖...↗`) but have no `MetaLanguage`, so the host's meta drives expansion. **SQL** additionally carries the point of the exercise in its `LiftTo` impls (issue #219): a value spliced with `↑` becomes a `literal` *node*, spelled as standard SQL with every `'` doubled, so it cannot close the literal and continue the statement — see `lift::sql_squote_escape` and the dialect caveat in issue #233. Its `program` holds statements, so a bare expression fragment (`sql↖id = 1↗`) reaches a `QTerm` only through the `SELECT …` wrapper retry in `parse_pre`, the same technique Lean uses with `#check …`.
 **Text** additionally has a `meta.rs`: the **identity** meta. Where the other hosts translate a quoted fragment into code that rebuilds it (builder calls for rust/python, string literals for nix/lean), text has no expressions to translate into, so it *holds the object-level code as unparsed lines* — same tags, same `cmds`, same text. The operator spellings are the other half of that: `↑ ↓ ← ⟨T⟩ ⟨N⟩` each need a host expression, so each returns an error naming a real host. Text is absent from `omni.rs`'s `metas` section, so the meta is reachable only by wiring it into a `Single`/`DictMulti` by hand.

**Lean** (`lean`/`lean4`) is, like Nix, both a quotable target *and* a string-based host: `langs/lean/meta.rs` + `ops.rs` reconstruct fragments as Lean interpolated strings (`s!"…"`), mapping a host unquote `↙x↘` onto Lean's own `{x}` interpolation and `↑` onto `toString` (issue #132). Its hole (`__QUILT_HOLE__`) needs no grammar patch — it is already a valid Lean identifier, so it parses in term, tactic, do-element, name and binder position. Command position is reached by a fallback in `parse_pre`: holes alone on their own line are wrapped in `#check …` and the wrapper stripped from the parsed tree. Emit into a top-level command *sequence* still needs the grammar change in issue #133. A hole's kind comes from its *parent* (`hole_kind`), since the token is spelled the same everywhere. Lean's `module` holds commands rather than terms, so `LeanLanguage` retries a failed parse inside `#check …` and strips the wrapper, which is what lets a bare term fragment (`lean↖n + 1↗`) parse at all.

**Nix** is both a quotable target *and* a host: `langs/nix/meta.rs` + `ops.rs` implement a **string-based** `MetaLanguage` — instead of emitting builder calls into a `QTerm` runtime (which Nix has none of), it reconstructs each fragment as a Nix string literal, mapping a host unquote `↙x↘` onto Nix's own `${x}` antiquotation and `↑` onto `toString`. A `.nix.quilt` file therefore expands to a plain Nix metaprogram that, evaluated (`nix eval`), yields the generated code as a string. The string model is language-agnostic (a Nix host can generate any target), but has no `b_` accumulator, so emit is functional rather than imperative: `←` spells `builtins.concatStringsSep "\n"` and takes the whole list of fragments (built with `map`), joining it into the surrounding container — `nix↖[ ↙← (map f xs)↘ ]↗` (issue #155).

**Bash and zsh** are, like Nix and Lean, quotable targets *and* string-based hosts (issue #151). One `ShellMetaLanguage<D>` in `langs/shell/meta.rs` + `ops.rs` serves both dialects — they double-quote identically, so the dialect is only a marker for error messages, shared for the reason `langs/shell/mod.rs`'s tag tables are. A quote becomes a double-quoted word and a host unquote is spliced into it **verbatim**, not wrapped: unlike a Nix expression, every shell expression that produces a value already carries its own `$` (`$name`, `${arr[0]}`, `$(cmd)`), and wrapping would be wrong — `${$(cmd)}` is a syntax error. So a `.bash.quilt` file expands to a plain script that, run, prints the generated code, which is what makes `hashbang()` reachable through `quilt run`. Escaping is `lift::sh_dquote_escape`, shared with the `LiftTo<Bash>`/`LiftTo<Zsh>` impls. Four of the five glyphs have no spelling — an operator is applied *prefix* to what follows and a shell has no prefix-applied word operators — so `↑ ← ↓ ⟨T⟩ ⟨N⟩` (all but the quote/unquote pair) return errors naming the alternative.

**Bootstrap** (`langs/bootstrap/`) is internal-only: its `lang.rs` re-exports the tree-sitter `RustLanguage` unchanged, and only its *meta* is special — `strlift.rs` lifts to a string and re-parses it, a slower shortcut used only for bootstrapping. (No language currently implements `Language` without tree-sitter; `langs/text/lang.rs` is a `todo!()` stub. The trait itself has no tree-sitter dependency, so one could.)

`langs/omni.rs` defines `Omni` (the default `Multi` used by the CLI) using enum-dispatch over all enabled languages. Registry keys: `rust`/`rs`, `python`/`py`, `text`/`txt`, `wgsl`, `html`, `zsh`, `bash`, `nix`, `lean`/`lean4`, `sql`.

### Feature flags

Each language is gated behind a Cargo feature (see `quilt/Cargo.toml`); all are on by default. The `parse` feature gates tree-sitter (the Quilt-source parser, the `Language` providers, `omni`, and `Multi`'s parse path). The runtime that expanded code targets (the `QTerm` builders, `qlift`, `coparse`) is tree-sitter-free, so consumers like `nanobots-codegen` depend on quilt with `default-features = false, features = ["rust"]` and build for `wasm32-unknown-unknown` without the tree-sitter C runtime.

### Bootstrap (`quilt/src/langs/bootstrap/`)

A two-stage self-hosting process that regenerates `quilt/src/langs/rust/meta.rs`. Both stages run the same program via `quilt`, `mk_meta.rs.quilt`, which produces and writes `meta.rs` (then `cargo fmt`s it):
1. **bootstrap0** — expands it with the `Bootstrap` multi (`BootstrapMetaLanguage`, which works without `meta.rs`)
2. **bootstrap1** — expands it with the `Omni` multi, i.e. the freshly generated `RustMetaLanguage` (self-hosting); a clean run leaves `meta.rs` unchanged

`mk_meta.rs.quilt` is a Rust source file that uses `⟨T⟩` (type placeholder) to refer to `Arc<QTerm>` without hard-coding it.

### Output: `StrCmd` / `PrefixWriter` (`quilt/src/strcmd.rs`)

Serialization is driven by a stack-based `StrCmd` sequence embedded in each `QTerm`. `PrefixWriter` maintains an indentation prefix stack; `StrCmd::NewLine` emits a newline then the current prefix.

### Other crates

- `quilt-lsp` — a multiplexing Language Server for `.quilt` files (tower-lsp). It parses the quilt structure, projects each language into a virtual document, proxies LSP traffic to per-language downstream servers (currently `rust-analyzer` for the ground language), and remaps positions in both directions. See `quilt-lsp/README.md` and `docs/wiki/lsp.md`.
- `quilt-python/` (crate `quilt_python`) — PyO3 bindings exposing quilt's core IR (`QTerm`, the fluent `tb/.c/.w/.n/.p/.x/.e/.b` builder, `leaf/sym/quote/unquote/cmd/write/push/name/qlift`, `NL/POP/HOLE`, and `.coparse()`) to Python. This is the runtime that expanded `.py.quilt` files target (`PythonMetaLanguage` emits calls into it). The Python import name is **`quilt`** (`from quilt import *`): a `quilt/` package whose `__init__.py` re-exports the native `quilt._quilt` module. Built abi3 (one `.so` for CPython ≥3.8) via `bin/build-py` (maturin); `quilt` puts it on `PYTHONPATH` for `python3` runs. See `examples/hello.py.quilt`.
- `quilt-wasm/` — wasm-bindgen bindings exposing the same core IR to JavaScript: the runtime expanded `.ts.quilt` files target, in the browser demos and on the CLI. Built with `bin/build-ts` (`--target nodejs`, into `pkg/`); `examples/web/build.mjs` builds `--target web` into `pkg-web/` for the browser. `quilt-wasm/node/` wraps it as the package `quilt run` binds the bare `quilt` import to, adding `↓` (reduce) — which re-expands a generated stage by shelling out to `$QUILT`, since the expander is not in the runtime crate. `examples/web/quilt-rt.js` is the same wrapper for the browser, calling an in-page WASI expander instead. See `examples/staged_pow.ts.quilt`.
- `tree-sitter-quilt` — the Quilt bracket language (arrow brackets and special symbols). Source in `grammar.js`; regenerate the parser with `ts-gen`.

### Clippy configuration

The workspace enables `clippy::pedantic` but suppresses several lints globally (see `[workspace.lints.clippy]` in the root `Cargo.toml`). Run `cargo clippy` (or `bin/lint`) to check.
