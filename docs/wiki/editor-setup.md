# Editor Setup

## VS Code extension

**Directory:** `tools/quilt/`

The VS Code extension provides:

- **Syntax highlighting** — TextMate grammar for the Quilt arrow-bracket syntax.
- **Arrow-glyph keybindings** — chord shortcuts for typing all Quilt Unicode operators.
- **Language Server** — launches `quilt-lsp` for `.quilt` files; provides quilt diagnostics and, for `.rs.quilt` files, full Rust support proxied through `rust-analyzer`.

### Installation

1. Build and install the LSP server:
   ```sh
   cargo install --path rust/quilt-lsp
   ```
   Or point the extension at the binary you built:
   ```json
   // .vscode/settings.json
   { "quilt-lsp.serverPath": "/path/to/target/debug/quilt-lsp" }
   ```

2. Install the extension's npm dependencies (needed for the extension host):
   ```sh
   cd tools/quilt && npm install
   ```

3. Symlink the extension into VS Code's extensions directory:
   ```sh
   ln -s "$(pwd)/tools/quilt" ~/.vscode/extensions/quilt
   ```
   Or open VS Code and use **Developer: Install Extension from Location…**.

4. Reload VS Code.

5. Rust support also requires `rust-analyzer`:
   ```sh
   rustup component add rust-analyzer
   ```

### Settings

| Setting                      | Default         | Description                                  |
|------------------------------|-----------------|----------------------------------------------|
| `quilt-lsp.serverPath`       | `quilt-lsp`     | Path to the `quilt-lsp` binary               |
| `quilt-lsp.rustAnalyzerPath` | `rust-analyzer` | Override rust-analyzer command               |
| `quilt-lsp.trace.server`     | `off`           | `off` \| `messages` \| `verbose` LSP tracing |

---

## Keybindings

The extension registers chord keybindings for all Quilt Unicode operators so they are convenient to type on a standard keyboard. The full list is in `tools/quilt/package.json`.

### Mac keybinding file

`tools/DefaultKeyBinding.dict` (also at `tools/quilt/DefaultKeyBinding.dict`) is a macOS `~/Library/KeyBindings/DefaultKeyBinding.dict` snippet for system-level arrow-key input. Copy or merge it into:

```
~/Library/KeyBindings/DefaultKeyBinding.dict
```

Restart apps after installing.

### Typical chord patterns

The VS Code extension uses a multi-key chord (e.g. `ctrl+k ctrl+u`) to insert each glyph. The exact bindings are in `package.json` under `"keybindings"`.

---

## Quilt comments in the editor

Quilt line comments (`⟨//⟩ text`) and block comments (`⟨/*⟩ … ⟨*/⟩`) are understood by the LSP server and stripped before the language fragment reaches the downstream server, so they do not appear as syntax errors in the ground language. The LSP translates them to standard ground-language comments in the projected document.

---

## How the LSP is started

The extension's `extension.js` starts `quilt-lsp` as a child process communicating over stdio whenever a `.quilt` file is opened. See [Quilt LSP](lsp.md) for the server architecture.
