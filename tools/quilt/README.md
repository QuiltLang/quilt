# Quilt — VS Code extension

Editor support for Quilt (`.quilt`) files:

- **Syntax highlighting** for the Quilt arrow-bracket language (TextMate grammar).
- **Arrow-glyph keybindings** for typing `← → ↑ ↓ ↖ ↗ ↙ ↘ ↕ ↔ ⟨ ⟩ ⟨T⟩ ⟨N⟩`
  (see the chords in `package.json`).
- **Language Server** (`quilt-lsp`): quilt syntax diagnostics, code actions,
  and language support proxied to downstream servers — Rust via
  `rust-analyzer` and Python via `pyright` for the ground language of
  `.rs.quilt` / `.py.quilt` files (hover, go-to-definition, completion,
  diagnostics), Lean via `lean --server` as ground or quoted fragments, and
  WGSL via `wgsl-analyzer` for `wgsl↖…↗` quotes. HTML and shell quotes are
  highlighted in-process.

## Setup

Install **QuiltLang** (`quiltlang.quiltlang`) from
[Open VSX](https://open-vsx.org/extension/quiltlang/quiltlang), or download
`quiltlang-<version>.vsix` from the [latest
release](https://github.com/QuiltLang/quilt/releases/latest) and run
`code --install-extension quiltlang-<version>.vsix`. (The VS Code Marketplace
listing is still at an older 0.4.1.) Then reload VS Code.

For diagnostics and language support the extension needs the `quilt-lsp` language
server on your PATH. Grab the binary from the [latest
release](https://github.com/QuiltLang/quilt/releases/latest), or build it from
source:

```sh
cargo install --git https://github.com/QuiltLang/quilt quilt-lsp
```

(or point `quilt-lsp.serverPath` at the binary). Each downstream server must be
on your PATH for its language: `rust-analyzer` (`rustup component add
rust-analyzer`), `pyright-langserver` (`npm i -g pyright`), `lean`, and
`wgsl-analyzer`.

### Developing the extension

From the repo root, `bin/install_tools` does everything for a live local build:
installs `quilt-lsp`, runs `npm install` here, and symlinks this directory into
`~/.vscode/extensions`. Then reload VS Code.

## Settings

- `quilt-lsp.serverPath` — path to the `quilt-lsp` binary (default `quilt-lsp`).
- `quilt-lsp.rustAnalyzerPath` — override the downstream rust-analyzer command
  (sets `QUILT_LSP_RUST_ANALYZER`).
- `quilt-lsp.pythonServerPath` — override the Python server command
  (sets `QUILT_LSP_PYTHON_SERVER`).
- `quilt-lsp.wgslServerPath` — override the WGSL server command
  (sets `QUILT_LSP_WGSL_SERVER`).
- `quilt-lsp.leanServerPath` — override the Lean server command; inside a Lake
  package `lake serve` is usually better (sets `QUILT_LSP_LEAN_SERVER`).
- `quilt-lsp.trace.server` — `off` | `messages` | `verbose`.

## How it works

A `.quilt` file is one ground-language program with other-language fragments in
`↖…↗`/`↙…↘`. The server projects the ground language into a virtual document,
opens it to that language's server (`rust-analyzer`, `pyright`, `lean`) under
the de-quilted URI, gives each other-language fragment its own virtual
document, and maps positions back and forth. See
[the LSP docs](https://github.com/QuiltLang/quilt/blob/main/docs/wiki/lsp.md)
for details.
