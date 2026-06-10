# ![Quilt Logo](./docs/quilt.svg) Quilt

Quilt is a multi-stage, multi-language metaprogramming system. A `.quilt` file is ordinary source code with Unicode arrow-bracket syntax for embedding and manipulating code fragments of other languages at code-generation time.

**[→ Documentation Wiki](docs/wiki/index.md)**

## Development

### Bootstrapping

Run `bin/bootstrap` from the repo root (or just `bootstrap` from anywhere once the direnv env is active).

### Tools

Run `bin/install_tools` to build and install the editor tooling: it cargo-installs `quilt-lsp`, installs the VS Code extension's npm dependencies, and symlinks [tools/quilt](/tools/quilt/) into `~/.vscode/extensions`.

- [tools/quilt](/tools/quilt/): The VS Code extension (syntax highlighting, glyph keybindings, LSP client).
- [tools/DefaultKeyBinding.dict](./tools/DefaultKeyBinding.dict): Mac keybinding configuration (optional, installed manually).
