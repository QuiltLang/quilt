# Contributing to Quilt

Thanks for your interest in Quilt! The project is in **early alpha** — syntax, APIs, and the CLI may all change without notice — so the most valuable contributions right now are bug reports, feedback on the language design, and documentation fixes. Code contributions are welcome too; for anything non-trivial, please open an issue first so we can discuss the approach before you invest time in it.

## Reporting bugs and requesting features

Use the [issue tracker](https://github.com/QuiltLang/quilt/issues). For bugs, include:

- the `.quilt` source that triggers the problem (a minimal example if you can),
- what you expected and what happened instead (error output, generated code),
- how you ran it (`quilt expand`, `quilt run`, `quilt check`, the LSP, …) and your platform.

A note on writing Quilt snippets in issues: the arrow brackets (`↖ ↗ ↙ ↘`) and operators (`↑ ↓ ← ⟨T⟩ ⟨N⟩`) are real syntax, including inside comments — please paste examples verbatim in fenced code blocks.

## Development setup

Quilt is a Rust workspace. Two ways to get a working environment:

- **Nix + direnv (recommended):** the flake in `nix/` provides everything (Rust toolchain, rust-script, python3, maturin, tree-sitter, nodejs). With [direnv](https://direnv.net) installed, `direnv allow` in the repo root sets it all up, including the `bin/` helper scripts on your `PATH`. This is exactly what CI uses.
- **Manual:** a Rust toolchain matching `rust-toolchain.toml` (via [rustup](https://rustup.rs)) and a C compiler get you `cargo build` / `cargo test`. Python 3 and [maturin](https://www.maturin.rs/) are additionally needed for the Python bindings and `.py.quilt` tests.

See `CLAUDE.md` for a tour of the workspace layout and architecture, and `docs/wiki/` for the language documentation.

## Before you submit a PR

CI runs every check inside the Nix devShell; you can reproduce them locally with the same scripts:

```sh
bin/ctest              # cargo test across the workspace
bin/fmt-check          # cargo fmt --check
bin/lint -- -D warnings   # cargo clippy (pedantic, --tests)
bin/check-bootstrap    # self-hosting bootstrap must leave meta.rs unchanged
bin/check-examples     # quilt-check + expand-diff every example
bin/build-py && bin/test-py   # Python bindings + .py.quilt runtime
```

(Without the direnv environment, run the scripts from the repo root, e.g. `./bin/ctest`.)

A few project-specific things to know:

- `quilt/src/langs/rust/meta.rs` is **generated** by the bootstrap from `mk_meta.rs.quilt` — don't edit it by hand; edit `mk_meta.rs.quilt` and run `bin/bootstrap`.
- After editing `tree-sitter-quilt/grammar.js`, regenerate the parser with `bin/ts-gen` and commit the result.
- Each example under `examples/` has a committed expansion that CI diffs against — re-expand and commit if your change affects output.

## Pull requests

- Keep PRs focused; separate refactors from behavior changes.
- Reference the issue the PR addresses (`Fixes #NN`).
- Make sure the checks above pass — CI runs them all on every PR.

## License

Quilt is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
