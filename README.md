# ![Quilt Logo](./docs/quilt.svg) Quilt

Quilt is a multi-stage, multi-language metaprogramming system. A `.quilt` file is ordinary source code with Unicode arrow-bracket syntax for embedding and manipulating code fragments of other languages at code-generation time.

**[→ Documentation Wiki](docs/wiki/index.md)**

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

## Tenets

1. **Code should be generic over representation.** Every language already has a textual syntax, so a metaprogramming system that operates only on terms adds surface area instead of reusing what exists. And there is no single right representation: strings, token trees, or terms; arcs, hash-consing, plain references, or none; arena or heap allocation; red-green trees; eager or lazy properties; source text, spans, or commands; mutable or immutable; untyped or typed. Metaprograms shouldn't be married to any one of these choices.
2. **A language shouldn't need a second language for metaprogramming.** "Meta" is as universal a concept as arithmetic or functions, yet most languages bolt on an ad-hoc macro layer that sacrifices the host language's tooling and guarantees. Bad metaprogramming is everywhere; it deserves to be fixed once, with meta-meta-programming.
3. **Support all languages.** When one system spans many languages, the right tool for the job is always available.

## Development

### Bootstrapping

Run `bin/bootstrap` from the repo root (or just `bootstrap` from anywhere once the direnv env is active).

### Tools

Run `bin/install_tools` to build and install the editor tooling: it cargo-installs `quilt-lsp`, installs the VS Code extension's npm dependencies, and symlinks [tools/quilt](/tools/quilt/) into `~/.vscode/extensions`.

- [tools/quilt](/tools/quilt/): The VS Code extension (syntax highlighting, glyph keybindings, LSP client).
- [tools/DefaultKeyBinding.dict](./tools/DefaultKeyBinding.dict): Mac keybinding configuration (optional, installed manually).
