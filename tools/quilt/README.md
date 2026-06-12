# Quilt — VS Code extension

Editor support for Quilt (`.quilt`) files:

- **Syntax highlighting** for the Quilt arrow-bracket language (TextMate grammar).
- **Arrow-glyph keybindings** for typing `← → ↑ ↓ ↖ ↗ ↙ ↘ ↕ ↔ ⟨ ⟩ ⟨T⟩ ⟨N⟩`
  (see the chords in `package.json`).
- **Language Server** (`quilt-lsp`): quilt syntax diagnostics, and — for the
  ground language of a `.rs.quilt` file — full Rust support (hover,
  go-to-definition, completion, diagnostics) proxied to `rust-analyzer`.

## Setup

For the alpha the extension is installed manually (it is not on the
Marketplace yet). From the repo root, `bin/install_tools` does everything:
installs `quilt-lsp`, runs `npm install` here, and symlinks this directory
into `~/.vscode/extensions`. Then reload VS Code.

Or, step by step:

1. Build/install the server:
   ```sh
   cargo install --path quilt-lsp   # puts `quilt-lsp` on your PATH
   ```
   (or set `quilt-lsp.serverPath` to the built binary).
2. Install the client deps in this folder:
   ```sh
   npm install
   ```
3. Symlink this directory into `~/.vscode/extensions` (see the top-level
   README) and reload VS Code.

`rust-analyzer` must be available for Rust support (`rustup component add
rust-analyzer`).

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
