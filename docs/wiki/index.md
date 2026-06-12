# Quilt Documentation

Quilt lets metaprograms in any language generate and manipulate code in any other language using five Unicode arrow glyphs.

A `.quilt` file is ordinary source code with Quilt brackets spliced in. Running `quilt expand` strips the brackets and writes plain source — no special build system required.

```rust
// squares.py.rs.quilt  —  a Rust program that generates Python
let squares: Vec<u64> = (1..=5).map(|n| n * n).collect();

let program = python↖
    def main():
        squares = ↙squares.↑↘
        print(squares)

    main()
↗;

println!("{}", program.coparse());
```

```python
# generated squares.py
def main():
    squares = [1, 4, 9, 16, 25]
    print(squares)

main()
```

The five operators — quote `↖↗`, unquote `↙↘`, lift `↑`, reduce `↓`, emit `←` — compose across any combination of supported languages. See [Concepts](concepts.md) for a full walkthrough.

## Start here

1. **[Concepts](concepts.md)** — the file format, all five operators, and how staging works
2. **[CLI & Scripts](cli.md)** — `quilt expand`, `quilt run`, and the helper scripts
3. **[Editor Setup](editor-setup.md)** — VS Code extension with glyph keybindings and LSP support

## Language

| Page | What it covers |
|------|----------------|
| [Concepts](concepts.md) | The `.quilt` file format, operator glyphs, quasi-quoting, and staging |

## Tooling

| Page | What it covers |
|------|----------------|
| [CLI & Scripts](cli.md) | `quilt expand`, `quilt run`, and the `bin/` helper scripts |
| [Python Bindings](python-bindings.md) | `quilt_python` — the PyO3 runtime for `.py.quilt` files |
| [Quilt LSP](lsp.md) | `quilt-lsp` — the multiplexing Language Server |
| [Editor Setup](editor-setup.md) | VS Code extension, keybindings, and `tools/quilt` |

## Internals

Reference material for contributors and anyone extending Quilt.

| Page | What it covers |
|------|----------------|
| [QTerm IR](qterm.md) | The central `QTerm` data type and the `QTermBuilder` fluent API |
| [Parse → Expand Pipeline](pipeline.md) | How a `.quilt` file becomes ordinary source code |
| [Language Traits](language-traits.md) | `Language`, `LanguagePost`, `MetaLanguage` — the extension points |
| [Concrete Languages](concrete-languages.md) | Rust, Python, HTML, WGSL, Zsh, Bash, Text implementations |
| [Multi and Omni](multi-omni.md) | The `Multi<LS,MS>` engine and the `Omni` production registry |
| [Bootstrap](bootstrap.md) | Self-hosting: generating `meta.rs` from `mk_meta.rs.quilt` |
| [Adding a Language](adding-a-language.md) | Step-by-step guide for supporting a new language |
| [Nanobots](nanobots.md) | The gas-metered nanobot IR toolchain (sibling repo) |

## Repository layout

```
quilt/
├── quilt/              # Core library + CLI
├── quilt-lsp/          # Language Server
├── quilt-python/       # PyO3 bindings (Python runtime)
├── tree-sitter-quilt/  # Grammar for the Quilt bracket syntax
├── bin/                # Helper scripts: quilt, bootstrap, build-py, …
├── tools/quilt/        # VS Code extension
├── docs/wiki/          # This wiki
└── examples/           # Annotated .quilt examples
```
