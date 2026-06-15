# Quilt — VS Code extension

Editor support for Quilt (`.quilt`) files:

- **Syntax highlighting** for the Quilt arrow-bracket language (TextMate grammar).
- **Arrow-glyph keybindings** for typing `← → ↑ ↓ ↖ ↗ ↙ ↘ ↕ ↔ ⟨ ⟩ ⟨T⟩ ⟨N⟩`
  (see the chords in `package.json`).
- **Language Server** (`quilt-lsp`): quilt syntax diagnostics, and — for the
  ground language of a `.rs.quilt` file — full Rust support (hover,
  go-to-definition, completion, diagnostics) proxied to `rust-analyzer`.

## Setup

Install **QuiltLang** from the VS Code Marketplace: open the Extensions view
(`⇧⌘X` / `Ctrl+Shift+X`), search for `QuiltLang`, and click Install — or run
`code --install-extension quiltlang.quiltlang`. Then reload VS Code.

For diagnostics and Rust support the extension needs the `quilt-lsp` language
server on your PATH. Grab the binary from the [latest
release](https://github.com/QuiltLang/quilt/releases/latest), or build it from
source:

```sh
cargo install --path quilt-lsp   # puts `quilt-lsp` on your PATH
```

(or point `quilt-lsp.serverPath` at the binary). `rust-analyzer` must be
available for Rust support (`rustup component add rust-analyzer`).

### Developing the extension

From the repo root, `bin/install_tools` does everything for a live local build:
installs `quilt-lsp`, runs `npm install` here, and symlinks this directory into
`~/.vscode/extensions`. Then reload VS Code.

## Settings

- `quilt-lsp.serverPath` — path to the `quilt-lsp` binary (default `quilt-lsp`).
- `quilt-lsp.rustAnalyzerPath` — override the downstream rust-analyzer command
  (sets `QUILT_LSP_RUST_ANALYZER`).
- `quilt-lsp.trace.server` — `off` | `messages` | `verbose`.

## How it works

A `.quilt` file is one ground-language program with other-language fragments in
`↖…↗`/`↙…↘`. The server projects the ground language into a virtual document,
opens it to `rust-analyzer` under the de-quilted URI, and maps positions back
and forth. See [`quilt-lsp`](../../quilt-lsp) for details.
