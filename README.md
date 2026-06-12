# ![Quilt Logo](./docs/quilt.svg) Quilt

> **⚠️ Early alpha.** Quilt is under active development. Expect breaking changes, rough edges, and gaps in the docs — the syntax, APIs, and CLI may all change without notice. Bug reports and feedback are very welcome.

Quilt is a polyglot metaprogramming language. A `.quilt` file is ordinary source code with Unicode arrow-bracket syntax for embedding and manipulating code fragments of other languages at code-generation time.

**[→ Documentation Wiki](docs/wiki/index.md)  ·  [→ Website](https://quiltlang.github.io)**

## Example

A Rust program that generates Python. `python↖…↗` quotes Python source and `↙…↘` splices Rust back in; inside a splice, a postfix `↑` lifts a Rust value into a term of the quoted language. The squares are computed at generation time, in Rust; the emitted Python contains only the results:

```rust
#!/usr/bin/env quilt
use quilt::prelude::*;

fn main() -> Result<()> {
    // Runs at generation time, in Rust.
    let squares: Vec<u64> = (1..=5).map(|n| n * n).collect();

    // Arrow brackets quote Python source and splice Rust back in; the
    // postfix lift operator turns the Rust Vec into a Python list literal.
    let program = python↖
        def main():
            squares = ↙squares.↑↘
            print(squares)

        main()
    ↗;

    println!("{}", program.coparse());
    Ok(())
}
```

Running it prints the generated Python:

```python
def main():
    squares = [1, 4, 9, 16, 25]
    print(squares)

main()
```

## Installation

Quilt is a Rust project, so you'll need a **Rust toolchain** (1.96 or newer) — install one via [rustup](https://rustup.rs) if you don't have it — plus a **C compiler** (`cc`; clang or gcc), which the bundled tree-sitter grammars build against. That's the whole story for the CLI and the LSP: neither needs Python. (The Python interpreter and [`maturin`](https://www.maturin.rs/) are only required to run `.py.quilt` files — see below.)

### CLI

Install the `quilt` command straight from the repo:

```sh
cargo install --git https://github.com/QuiltLang/quilt quilt
```

This builds the `quilt` binary (`expand`, `run`, `check`, `clean`) and drops it in `~/.cargo/bin`. That's all you need to **expand** `.quilt` files into plain source.

To **run** a `.quilt` file as a script, you also need the runtime for its ground language:

- **`.rs.quilt` (Rust ground):** `cargo install rust-script`
- **`.py.quilt` (Python ground):** the `quilt` Python module — see [Python bindings](docs/wiki/python-bindings.md).

### Editor tooling (LSP + VS Code)

`quilt-lsp` is a Language Server that multiplexes per-language servers (currently `rust-analyzer`) across the embedded fragments. Install it the same way:

```sh
cargo install --git https://github.com/QuiltLang/quilt quilt-lsp
```

The VS Code extension (syntax highlighting, glyph keybindings, LSP client) isn't on the Marketplace yet. Clone the repo and run `bin/install_tools`, which cargo-installs `quilt-lsp`, installs the extension's npm dependencies, and symlinks it into `~/.vscode/extensions`. See [Editor setup](docs/wiki/editor-setup.md) for details, including how to type the arrow glyphs.

## Quickstart

1. Make sure the CLI and the Rust runner are installed:

   ```sh
   cargo install --git https://github.com/QuiltLang/quilt quilt
   cargo install rust-script
   ```

2. Save the program from the [Example](#example) above as `squares.py.rs.quilt`. The double extension is the **language chain**, read right-to-left: `rs` is the ground language (the program you run) and `py` is the default language for un-annotated quotes. You can copy the arrow glyphs straight out of this README.

3. Run it:

   ```sh
   quilt run squares.py.rs.quilt
   ```

   `run` is the default subcommand, so `quilt squares.py.rs.quilt` works too — as does `./squares.py.rs.quilt` once it's executable (`chmod +x`), thanks to the `#!/usr/bin/env quilt` shebang. Either way it prints the generated Python:

   ```python
   def main():
       squares = [1, 4, 9, 16, 25]
       print(squares)

   main()
   ```

4. To see the generated *source* instead of running it, expand the brackets:

   ```sh
   quilt expand squares.py.rs.quilt   # writes plain Rust to squares.py.rs
   ```

For more, browse the [`examples/`](examples/) directory and the [Documentation Wiki](docs/wiki/index.md).

## Supported platforms

Quilt is developed on **macOS** and continuously tested on **Linux** (CI runs on `ubuntu-latest`); both are known-good. **Windows is currently untested** — it may work, but it isn't part of CI, so expect rough edges. Reports are welcome.

## Tenets

1. **Code should be generic over representation.** Every language already has a textual syntax, so a metaprogramming system that operates only on terms adds surface area instead of reusing what exists. And there is no single right representation: strings, token trees, or terms; arcs, hash-consing, plain references, or none; arena or heap allocation; red-green trees; eager or lazy properties; source text, spans, or commands; mutable or immutable; untyped or typed. Metaprograms shouldn't be married to any one of these choices.
2. **A language shouldn't need a second language for metaprogramming.** "Meta" is as universal a concept as arithmetic or functions, yet most languages bolt on an ad-hoc macro layer that sacrifices the host language's tooling and guarantees. Bad metaprogramming is everywhere; it deserves to be fixed once, with meta-meta-programming.
3. **Support all languages.** When one system spans many languages, the right tool for the job is always available.

## Development

Contributing requires the full dev environment (Nix + [direnv](https://direnv.net)). Clone the repo and let direnv load the toolchain:

```sh
git clone https://github.com/QuiltLang/quilt
cd quilt
direnv allow   # loads the Nix dev shell: Rust toolchain, rust-script, python3, …
```

The `bin/` scripts then work from anywhere the env is active:

- `cargo test` (or `bin/ctest`) — run the test suite.
- `bin/bootstrap` — regenerate the self-hosted Rust meta-language from `mk_meta.rs.quilt`.
- `bin/install_tools` — build and install the LSP + VS Code extension.

See the [wiki](docs/wiki/index.md) — especially [Pipeline](docs/wiki/pipeline.md), [Bootstrap](docs/wiki/bootstrap.md), and [Adding a language](docs/wiki/adding-a-language.md) — for the full picture.

- [tools/quilt](/tools/quilt/): The VS Code extension (syntax highlighting, glyph keybindings, LSP client).
- [tools/DefaultKeyBinding.dict](./tools/DefaultKeyBinding.dict): Mac keybinding configuration (optional, installed manually).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Quilt by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
