# Editor Setup

## VS Code extension

**Directory:** `tools/quilt/`

The VS Code extension provides:

- **Syntax highlighting** — TextMate grammar for the Quilt arrow-bracket syntax, with embedded Rust, Python, WGSL, HTML, shell and SQL grammars injected into annotated quotes.
- **Arrow-glyph keybindings** — chord shortcuts for typing all Quilt Unicode operators.
- **Language Server** — launches `quilt-lsp` for `.quilt` files; provides quilt diagnostics and code actions, plus ground-language support proxied to `rust-analyzer` (`.rs.quilt`), pyright (`.py.quilt`) and `lean --server` (`.lean.quilt`), and to wgsl-analyzer / Lean for embedded quotes.

### Installation

The extension is **QuiltLang** (`quiltlang.quiltlang`). Every release publishes it to Open VSX and attaches it to the GitHub release; the [VS Code Marketplace listing](https://marketplace.visualstudio.com/items?itemName=quiltlang.quiltlang) exists but lags behind (it is still at 0.4.1, because Marketplace publishing needs a token the release workflow doesn't have). For the current version:

- **VSCodium and other Open VSX editors:** install it from [Open VSX](https://open-vsx.org/extension/quiltlang/quiltlang) (search for `QuiltLang` in the Extensions view).
- **VS Code:** download `quiltlang-<version>.vsix` from the [latest release](https://github.com/QuiltLang/quilt/releases/latest) and run `code --install-extension quiltlang-<version>.vsix` (or **Extensions: Install from VSIX…**).

You'll still need the `quilt-lsp` server on your `PATH` (plus `rust-analyzer`, `pyright-langserver`, `lean` or `wgsl-analyzer` for the languages you use) — grab the binary from the same release, or `cargo install --git https://github.com/QuiltLang/quilt quilt-lsp`.

#### From source (for development)

To hack on the extension itself, run the install script from the repo root (idempotent — re-run it after updates):

```sh
bin/install_tools
```

It performs the manual steps below: `cargo install --path quilt-lsp`, `npm install` in `tools/quilt`, and symlinking the extension into `~/.vscode/extensions/quiltlang` (removing a stale link from the old `~/.vscode/extensions/quilt` name). It also warns if `rust-analyzer` or `rust-script` is missing from `PATH`.

#### Manual steps (what the script does)

1. Build and install the LSP server:
   ```sh
   cargo install --path quilt-lsp
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
   ln -sfn "$(pwd)/tools/quilt" ~/.vscode/extensions/quiltlang
   ```
   (`-n` matters: without it, re-running with an existing link drops a recursive `quiltlang` symlink *inside* `tools/quilt`.) Or open VS Code and use **Developer: Install Extension from Location…**.

4. Reload VS Code.

5. Rust support also requires `rust-analyzer` (Python support needs `pyright-langserver`, Lean needs `lean`, WGSL needs `wgsl-analyzer`):
   ```sh
   rustup component add rust-analyzer
   ```

### Settings

An empty override means the server's default command on `PATH` (shown in parentheses).

| Setting                      | Default     | Description                                                              |
|------------------------------|-------------|--------------------------------------------------------------------------|
| `quilt-lsp.serverPath`       | `quilt-lsp` | Path to the `quilt-lsp` binary                                           |
| `quilt-lsp.rustAnalyzerPath` | empty       | Override the Rust server command (`rust-analyzer`)                       |
| `quilt-lsp.pythonServerPath` | empty       | Override the Python server command (`pyright-langserver --stdio`)        |
| `quilt-lsp.wgslServerPath`   | empty       | Override the WGSL server command (`wgsl-analyzer`)                       |
| `quilt-lsp.leanServerPath`   | empty       | Override the Lean server command (`lean --server`; `lake serve` inside a Lake package) |
| `quilt-lsp.trace.server`     | `off`       | `off` \| `messages` \| `verbose` LSP tracing                             |

---

## Keybindings

The extension registers chord keybindings for all Quilt Unicode operators so they are convenient to type on a standard keyboard. The full list is in `tools/quilt/package.json`.

### Mac keybinding file

`tools/DefaultKeyBinding.dict` is a macOS `~/Library/KeyBindings/DefaultKeyBinding.dict` snippet for system-level arrow-key input. Copy or merge it into:

```
~/Library/KeyBindings/DefaultKeyBinding.dict
```

Restart apps after installing.

### Typical chord patterns

The VS Code extension uses multi-key chords starting with `cmd+1` or `cmd+2` to insert each glyph. Directions can be typed with the arrow keys or vim-style `h`/`j`/`k`/`l`, with or without `cmd` held on the later keys:

- `cmd+1` then a direction: a single arrow (`cmd+1 cmd+up` → `↑`, `cmd+1 cmd+left` → `←`, …); `cmd+1 cmd+,` / `cmd+1 cmd+.` → `⟨` / `⟩`; `cmd+1 cmd+t` → `⟨T⟩`; `cmd+1 cmd+n` → `⟨N⟩`.
- `cmd+2` then two directions: the combined arrow (`cmd+2 cmd+up cmd+left` → `↖`, `cmd+2 cmd+down cmd+right` → `↘`, `cmd+2 cmd+left cmd+right` → `↔`, `cmd+2 cmd+up cmd+down` → `↕`; a repeated direction gives the single arrow).

The exact bindings are in `package.json` under `"keybindings"`.

---

## Quilt comments in the editor

Quilt line comments (`⟨//⟩ text`) and block comments (`⟨/*⟩ … ⟨*/⟩`) are understood by the LSP server and stripped before the language fragment reaches the downstream server, so they do not appear as syntax errors in the ground language. The LSP translates them to standard ground-language comments in the projected document.

---

## How the LSP is started

The extension's `extension.js` starts `quilt-lsp` (from `quilt-lsp.serverPath`) as a child process communicating over stdio whenever a `.quilt` file is opened, passing the per-language server overrides through as `QUILT_LSP_*` environment variables and defaulting `RUST_LOG` to `info`. Server logs appear in the **Quilt LSP** output channel. See [Quilt LSP](lsp.md) for the server architecture.
