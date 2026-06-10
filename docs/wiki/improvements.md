# Possible Improvements — Bang for Buck

A prioritized list of improvements to the Quilt codebase. "Bang for buck" means: impact on correctness, usability, or maintainability relative to implementation effort. Items near the top are high-reward and achievable in a single sitting; items near the bottom are either more speculative or require broader design work.

- 🔲 = Not Started
- 🚧 = In Progress
- ✅ = Completed

---

## Tier 1 — Quick wins, high payoff

### 🔲 1. Fix the `parse_chain` newline-padding FIXME

**File:** `multi.rs:143`

```rust
// FIXME: this is temporary
let s = &format!("\n{s}\n");
```

The parser wraps every source string in leading/trailing newlines to work around some edge case in the tree-sitter grammar. This is a latent correctness bug: it silently offsets all parse positions by one line, which means error positions and source-map data are wrong by ±1. The right fix is to find and patch the grammar edge case that required the hack, or explicitly adjust positions at the boundary.

---

### 🔲 2. Remove the `ikind` dead path in `build_nodes`

**File:** `multi.rs:260`

```rust
let post = self
    .get_lang_mut(lang)?
    .parse_pre(/*hole.ikind*/ None, &code)?;
```

The hole's `InnerKind` was designed to propagate into child parses (so a hole known to be in expression position forces an expression parse). The call always passes `None`, so `parse_pre` never receives context. The commented-out parameter and the un-removed `ikind: Option<InnerKind>` signature on `Language::parse_pre` both accumulate confusion. Either wire the value through or delete the parameter.

---

### 🔲 3. Fix expander Emit-inference TODO

**File:** `multi.rs:373`

```rust
// TODO: only infer Emit if the outer TermKind was Stmt
```

In the `Stage::Ground` path, the expander unconditionally sets `okind = Default::default()` (i.e. `OuterKind::None`) for every child. The TODO says it should set `OuterKind::Emit` only when the hole's outer term is a statement, not an expression. Without the fix, quote splices in expression position may produce ill-typed code silently. The information to make the decision (the `tag`/`arity` of the parent tuple) is available at that point in the code.

---

### 🔲 4. Wire up `Hole.itag` for better hole typing

**File:** `lang.rs:30`

```rust
pub struct Hole {
    pub otag: Box<str>,
    // pub itag: Option<Box<str>>, // inner tag: what kind of thing can fill this hole
    pub prefix: Box<[Box<str>]>,
}
```

The `itag` field was stubbed and then commented out. It was meant to carry the expected inner kind of each hole (Expr, Stmt, Block) so nested parses could be coerced to the right shape. Implementing it would let `TSLanguage::parse_pre` pass a real `InnerKind` for each hole instead of `None`, which in turn would let the Rust parser correctly distinguish `{ }` (block) holes from expression holes, eliminating a class of parse ambiguity.

---

### 🔲 5. Macro-generate `OmniLanguages` boilerplate

**File:** `langs/omni.rs`

Every language added to the system requires editing at minimum six match arms spread across `OmniLanguages`, `OmniMetaLanguages`, `OmniLanguage`, `OmniLanguagePost`, `OmniMetaLanguage`, and the two `DynOmni*` structs — plus updating `Cargo.toml` feature flags, `dict_omni_language`, and the `Languages`/`MetaLanguages` `get`/`get_mut` implementations. All of this is mechanical.

A declarative macro like `define_omni! { rust => (RustLanguage, RustMetaLanguage, "rs", "rust"), ... }` could generate the entire block. Alternatively a proc-macro `#[derive(OmniLanguage)]` on a marker enum. Either approach means adding a new language is a one-line change in one place.

---

### 🔲 6. Dedup `DictMulti` language alias registration

**File:** `langs/omni.rs:491-527`

```rust
ret.add_lang("python", bx(DynPythonLanguage::default()));
ret.add_lang("py",     bx(DynPythonLanguage::default()));
```

Language aliases (`"py"`/`"python"`, `"rs"`/`"rust"`) are registered as separate `Box<dyn Language>` instances. This means there are two independent parser states per alias pair — not a correctness bug today (parsers are stateless), but it's wasteful and will become one if parsers ever accumulate state. A registry that maps aliases to a canonical key would fix this without changing the API.

---

## Tier 2 — Medium effort, high value

### 🔲 7. Expansion errors should carry source spans

Currently, when the expander fails (e.g. `unquote depth too high`), the error gives no source location. The `QTerm` type has no span field; spans are lost after parsing.

The simplest approach: add an `Option<Range<usize>>` to `QTerm::Quote` and `QTerm::Unquote` (populated during `build_nodes`) and thread it into error messages. A larger approach is a side table (source map) from `QTerm` node identity to source range, kept external to the IR so it doesn't bloat every node. Either would transform cryptic panics into editor-actionable diagnostics.

---

### 🔲 8. LSP: incremental document sync

**File:** `quilt-lsp/src/server.rs:608`

```rust
text_document_sync: Some(TextDocumentSyncCapability::Kind(
    TextDocumentSyncKind::FULL,
)),
```

The server currently requests full-text replacement on every keystroke. For large files this means re-projecting, re-parsing (tree-sitter), and re-syncing the entire virtual document to rust-analyzer on every change. Switching to `INCREMENTAL` and using tree-sitter's existing incremental parse API (`parse_with_options` + old tree) would reduce the work to only changed regions.

---

### 🔲 9. LSP: add Python as a host language

**File:** `quilt-lsp/src/adapters.rs:118-133`

```rust
pub fn language_adapter(key: &str) -> Option<...> {
    match key {
        #[cfg(feature = "rust")]
        "rs" | "rust" => Some(&RUST),
        _ => None,
    }
}
```

Python is a first-class quilt host (it has a `PythonMetaLanguage`), but the LSP only knows how to drive rust-analyzer. Adding a `PythonAdapter` that wraps Pyright or Pylsp (via `QUILT_LSP_PYTHON_SERVER`) would give `.py.quilt` files the same hover/goto-def/completion support that `.rs.quilt` files have today. The adapter interface is already well-abstracted for this.

---

### 🔲 10. LSP: multi-server support

**File:** `quilt-lsp/src/server.rs:139`

```rust
/// The single rust-analyzer connection, spawned on first Rust document.
rust: Mutex<Option<Arc<ChildServer>>>,
```

There is one shared child server for all Rust documents. This breaks if you open `.quilt` files from two different Cargo workspaces, because rust-analyzer is initialized with one root and can't serve the other. The fix: change this to `DashMap<PathBuf, Arc<ChildServer>>` keyed by the workspace root, spawning a new child per root. Also needed before Python (item 9) can share the server struct cleanly.

---

### 🔲 11. LSP: code actions

The server advertises no `codeActionProvider`. The most useful quilt-specific code actions would be:

- **Wrap selection in `↖…↗`** — the single most common editing gesture
- **Inline unquote** — move a `↙expr↘` body out of the quote into a local binding
- **Extract to named fragment** — pull a large quote body into a named item

These don't require downstream server cooperation; they operate purely on the quilt CST (which the server already has in `Document::region`).

---

### 🔲 12. Add `quilt check` subcommand

**File:** `bin.rs`

`quilt expand` always writes an output file; there is no way to validate a `.quilt` file without producing output. A `check` subcommand would parse and attempt expansion (including bootstrap validation), reporting errors without writing anything. Useful for CI pipelines and pre-commit hooks where you want to catch mistakes but don't want to check generated files into git.

---

## Tier 3 — Architectural improvements worth planning

### 🔲 13. Extend `InnerKind` with `Block` (and language-specific kinds)

**File:** `lang.rs:10`

```rust
pub enum InnerKind {
    Expr,
    Stmt,
    #[default]
    File,
    // TODO: rename or also add `Block`
}
```

The TODO note is correct. Rust distinguishes expressions, statements, items, and block bodies; Python distinguishes expressions, statements, and suites. The current three-way enum forces the `TSProvider::unwrap` implementation to guess from tree node tags, which is brittle. Adding `Block` at minimum, and possibly language-specific variants via a separate enum or trait method, would make hole typing precise enough to eliminate the `unimplemented!` panics in the Rust provider.

---

### 🔲 14. Eliminate `unimplemented!` panics in language providers

**File:** `langs/rust/lang.rs`

```rust
_ => unimplemented!("{}", qterm.sexp()),
```

The Rust `TSProvider::unwrap` and related helpers use `unimplemented!` for tree-sitter node kinds that weren't anticipated. These become user-visible panics when unusual but valid Rust code appears in a `.quilt` file. The fix is to replace each `unimplemented!` with a `Result::Err` that surfaces a clear diagnostic. Requires also changing signatures from returning bare values to returning `Result`.

---

### 🔲 15. Fix bootstrap self-hosting (bootstrap1 broken at HEAD)

From the project's own notes: `bootstrap1` (the fully self-hosted expansion of `mk_meta.rs.quilt` using the generated `RustMetaLanguage`) is broken at HEAD. This means the project is not genuinely self-hosting: edits to `mk_meta.rs.quilt` can only be validated via the slower bootstrap0 path. Fixing this unlocks the real self-hosting invariant and makes the test suite's bootstrap tests meaningful.

---

### 🔲 16. Heterogeneous reduction: meta-lang A reducing lang B

**File:** `meta.rs:63`

```rust
// TODO: support heterogenous reduction: meta-lang A reducing lang B
```

Currently `↓` (reduce) always reduces to the same meta-language. Heterogeneous reduction would mean `py↓` inside a Rust meta-program could reduce a WGSL quote — the Rust meta-language would invoke Python evaluation logic on the inner term. This is the key to cross-language staged computation and is probably the most theoretically interesting missing piece, though it requires a protocol between `MetaLanguage` implementations.

---

### 🔲 17. QTerm pattern matching / rewriting

**File:** `qterm.rs:166`

```rust
pub fn rewrite_naive(&self, find: &Self, replace: &Self) -> Arc<Self> {
```

`rewrite_naive` does structural equality matching only. A proper pattern language (with metavariables, `?x` that binds to any subtree) would make QTerm-level rewriting useful for refactoring tools, optimizations in the expander, and tests. The existing `Zipper` in `zipper.rs` is already the right data structure for cursor-based rewriting; it just needs a pattern matcher layered on top.

---

### 🔲 18. Caching / incremental expansion

`QTerm` already derives `Serialize`/`Deserialize` (via `postcard` in workspace deps). A file-based parse cache keyed by `(path, mtime, feature flags)` would let `quilt expand` skip re-parsing unchanged files. This matters most for the bootstrap workflow, where `mk_meta.rs.quilt` is expanded on every `bootstrap` run. The cache invalidation logic is simple since `.quilt` files have no transitive imports.

---

### 🔲 19. Remove or promote `ArcSTerm`

**File:** `term.rs`

`ArcSTerm<T>` and `ArcSTermBuilder<T>` are fully implemented generic versions of `QTerm`/`QTermBuilder` without the `Quote`/`Unquote` distinction. They don't appear to be used outside their own unit test. Either:

- **Promote**: make `QTerm` a type alias or newtype over `ArcSTerm<QTermTag>`, removing the duplication
- **Remove**: delete the dead code to reduce cognitive overhead

---

### 🔲 20. Grammar ergonomics for comment syntax

The `⟨//⟩`, `⟨/*⟩`, `⟨*/⟩` comment glyphs require three keystrokes each (compose sequence + two characters) and are easy to confuse with the `⟨T⟩` / `⟨N⟩` type/name glyphs. Some directions:

- Allow plain `//` and `/* */` to pass through in contexts where the host language parser would accept them (most `.rs.quilt` files), and only require the glyph form in embedded languages that don't have `//` syntax
- Explore whether the tree-sitter grammar can hide ordinary comments alongside the existing hidden newline handling, making comment glyphs optional for the common case
