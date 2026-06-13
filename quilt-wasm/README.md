# quilt-wasm

WebAssembly bindings for Quilt's core IR — the **browser runtime** that expanded
`.ts.quilt` programs target. It is the JS/WASM analog of the `quilt` Python
module (`quilt-python/`): the same `QTerm` builder, `qlift`/`qlift_html`, and
`coparse` serializer, exposed to JavaScript via `wasm-bindgen`.

Like `nanobots-codegen`, it depends on `quilt` with
`default-features = false, features = ["rust"]`, so it uses only the
tree-sitter-free runtime path and builds for `wasm32-unknown-unknown` with no C
runtime. (Compiling the *parser/expander* to wasm — for the meta-meta demo — is
tracked separately as Phase 2 of issue #43.)

## Build

```sh
# from the repo root
wasm-pack build quilt-wasm --target web      # for the browser demos (ESM)
wasm-pack build quilt-wasm --target nodejs   # for Node (CommonJS) + tests
```

The artifact lands in `quilt-wasm/pkg/` (git-ignored).

## Releasing to npm

Published to npm as [`quilt-wasm`](https://www.npmjs.com/package/quilt-wasm) — the
same bare specifier expanded `.ts.quilt` programs import. The `publish-npm` job
in `.github/workflows/ci.yml` runs on every `v*` tag (after the check matrix
passes): it does `wasm-pack build quilt-wasm --target web` and `npm publish`es
the resulting `pkg/`. The package version tracks the workspace version in
`Cargo.toml`.

Auth is **npm Trusted Publishing (OIDC)** — no secret to manage; GitHub's
`id-token` authenticates the publish and npm records build provenance. One-time
setup, because npm can only attach a trusted publisher to a package that already
exists:

1. Publish the first version manually: `wasm-pack build quilt-wasm --target web`,
   then `npm login` and `npm publish` from `quilt-wasm/pkg/`.
2. On npmjs.com → the package → **Settings → Trusted Publisher**, add a GitHub
   Actions publisher for repo `QuiltLang/quilt`, workflow `ci.yml`.

After that every `v*` tag publishes with no token. A version already on npm is a
no-op, not a failure.

## Smoke test

```sh
wasm-pack build quilt-wasm --target nodejs
node quilt-wasm/test/smoke.cjs
```

## API

Mirrors the Python runtime one-for-one where the ABIs allow:

| Python runtime        | quilt-wasm                                  |
| --------------------- | ------------------------------------------- |
| `tb(tag)` + `.c/.w/.n/.p/.x/.e/.b` | same fluent `WasmBuilder`      |
| `leaf/sym/quote/unquote/name`      | same free functions            |
| `cmd/write/push`                   | same free functions            |
| `NL`, `POP`, `HOLE` (constants)    | `NL()`, `POP()`, `HOLE()` (functions — wasm-bindgen can't export struct-valued constants) |
| `qlift`, `qlift_html`              | same; lift `number`/`string`/`boolean` (no `QTerm` pass-through yet) |
| `term.coparse()`                   | `term.coparse()` / `term.toString()` |

Builder and term methods that take `self`/a child by value **consume** the JS
object (wasm-bindgen move semantics), so chain in one expression and don't reuse
a spliced term.
