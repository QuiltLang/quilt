# quilt-lsp demo

A minimal cargo project to exercise `quilt-lsp` with full rust-analyzer support.

Open **`src/main.rs.quilt`** in VS Code (with the Quilt extension). Because this
is a real cargo project, rust-analyzer overlays the projection onto `src/main.rs`
inside the crate and you get the semantic features that standalone scripts can't:

- **Hover** over `greeting`, `make_greeting`, `expr`.
- **Go-to-definition** (F12) on the `make_greeting(...)` call → jumps to its `fn`.
- **Completion** after `make_greeting(` or `expr.`.
- **Semantic highlighting**, including the Rust inside `rs↖1 + 2 * 3↗`.
- **Outline** (⇧⌘O) listing `main` and `make_greeting`.

Give rust-analyzer a few seconds to index on first open (watch the "Quilt LSP"
output channel). `src/main.rs` is a placeholder the server overlays in-memory.
