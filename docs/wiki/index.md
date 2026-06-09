# Quilt Documentation Wiki

Quilt is a multi-stage, multi-language metaprogramming system. A `.quilt` file is an ordinary source file (Rust, Python, …) with Unicode arrow-bracket syntax spliced in to embed and manipulate code fragments of other languages — or the same language — at code-generation time.

## Pages

| Page                                        | What it covers                                                     |
|---------------------------------------------|--------------------------------------------------------------------|
| [Concepts](concepts.md)                     | The quilt file format, operator glyphs, quasi-quoting, and staging |
| [QTerm IR](qterm.md)                        | The central `QTerm` data type and the `QTermBuilder` fluent API    |
| [Parse → Expand Pipeline](pipeline.md)      | How a `.quilt` file becomes ordinary source code                   |
| [Language Traits](language-traits.md)       | `Language`, `LanguagePost`, `MetaLanguage` — the extension points  |
| [Concrete Languages](concrete-languages.md) | Rust, Python, HTML, WGSL, Text implementations                     |
| [Multi and Omni](multi-omni.md)             | The `Multi<LS,MS>` engine and the `Omni` production registry       |
| [Bootstrap](bootstrap.md)                   | Self-hosting: generating `meta.rs` from `mk_meta.rs.quilt`         |
| [CLI & Scripts](cli.md)                     | `quilt expand`, `quilt run`, and the `bin/` helper scripts         |
| [Python Bindings](python-bindings.md)       | `quilt_python` — the PyO3 runtime for `.py.quilt` files            |
| [Quilt LSP](lsp.md)                         | `quilt-lsp` — the multiplexing Language Server                     |
| [Editor Setup](editor-setup.md)             | VS Code extension, keybindings, and `tools/quilt`                  |
| [Nanobots](nanobots.md)                     | The gas-metered nanobot IR toolchain                               |
| [Adding a Language](adding-a-language.md)   | Step-by-step guide for supporting a new language                   |
| [Possible Improvements](improvements.md)    | Prioritized list of correctness / usability improvements           |

## Quick orientation

```
Quilt/
├── rust/
│   ├── quilt/          # Core library + CLI (cargo workspace member)
│   │   └── src/
│   │       ├── qterm.rs        # QTerm IR
│   │       ├── node.rs         # Surface AST (tree-sitter-quilt output)
│   │       ├── lang.rs         # Language / LanguagePost traits
│   │       ├── meta.rs         # MetaLanguage trait
│   │       ├── multi.rs        # Multi<LS,MS> engine
│   │       ├── strcmd.rs       # StrCmd serialization
│   │       └── langs/          # Concrete language implementations
│   │           ├── rust/       # Rust language + generated meta.rs
│   │           ├── python/     # Python language + meta
│   │           ├── html/       # HTML language (target only)
│   │           ├── wgsl/       # WGSL language (target only)
│   │           ├── text/       # Plain-text language (target only)
│   │           ├── bootstrap/  # Bootstrap meta + mk_meta.rs.quilt
│   │           └── omni.rs     # Omni (production Multi)
│   ├── quilt-lsp/      # Language Server (cargo workspace member)
│   ├── quilt_python/   # PyO3 bindings (cargo workspace member)
│   ├── tree-sitter-quilt/   # Grammar for the quilt bracket language
│   ├── tree-sitter-rust/    # Forked, with {} hole nodes
│   ├── tree-sitter-python/  # Forked, with __HOLE__ nodes
│   ├── tree-sitter-html/    # HTML grammar
│   └── nanobots/       # Nanobot toolchain (separate workspace)
├── bin/                # Shell scripts: quilt, bootstrap, build-py, ts-gen
├── tools/quilt/        # VS Code extension
└── examples/           # .quilt example files
```

## Key concepts in one paragraph

A `.quilt` file lives in a *ground language* (determined by the inner extension: `foo.rs.quilt` → Rust). Inside it, `↖…↗` opens a *quote* — a code fragment to be treated as data — and `↙…↘` opens an *unquote* — a spliced value. The `↑` glyph *lifts* a runtime value into a `QTerm`, `↓` *reduces* a `QTerm` by evaluating it, and `←` *emits* a term into the surrounding variadic context. The Quilt compiler parses the file into a `QTerm` tree, then calls the ground language's `MetaLanguage` to expand that tree into ordinary source code, which is written to disk.
