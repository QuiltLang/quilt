# quilt-wasm

WebAssembly bindings for Quilt's core IR — the runtime that expanded
`.ts.quilt` programs target, in a browser and under Node alike. It is the
JS/WASM analog of the `quilt` Python module (`quilt-python/`): the same `QTerm`
builder, `qlift`/`qlift_html`, and `coparse` serializer, exposed to JavaScript
via `wasm-bindgen`.

Like `nanobots-codegen`, it depends on `quilt` with
`default-features = false, features = ["rust"]`, so it uses only the
tree-sitter-free runtime path and builds for `wasm32-unknown-unknown` with no C
runtime. (Compiling the *parser/expander* to wasm — for the meta-meta demo — is
tracked separately as Phase 2 of issue #43.)

## Build

```sh
# from the repo root
bin/build-ts                                 # for `quilt run` (the nodejs target)
wasm-pack build quilt-wasm --target web      # for the browser demos (ESM)
wasm-pack build quilt-wasm --target nodejs   # for Node (CommonJS) + tests
```

The artifact lands in `quilt-wasm/pkg/` (git-ignored); `examples/web/build.mjs`
puts its `--target web` build in `pkg-web/`, so the two never clobber each other.

## `node/` — the Node runtime

`quilt run foo.ts.quilt` binds the bare `quilt` import to `node/index.mjs`, which
re-exports everything below and adds the one operator the wasm runtime cannot
provide by itself: `↓` (reduce). Reduce has to *re-expand* — a generated stage's
`coparse()` is still Quilt source — and the expander is not in this crate, so
`node/` shells out to the `quilt` binary (`$QUILT`), exactly as quilt-python's
`expand()` does. It is the CLI twin of `examples/web/quilt-rt.js`, which does the
same job in-page against the WASI expander. See issue #153.

```sh
bin/build-ts && bin/test-ts          # build + run the ↓ tests and .ts.quilt examples
```

Beyond `↓` it exports `expand(src, chain)` (Quilt source → TypeScript, via the
binary), `reduce(term)`, and a default export that is a no-op `init()` — so a
program written against the browser's `--target web` build, which awaits one,
also runs under `quilt run`.

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

## Tests

```sh
wasm-pack build quilt-wasm --target nodejs   # or: bin/build-ts
node quilt-wasm/test/smoke.cjs               # the builder API
node quilt-wasm/test/corpus.mjs              # the shared runtime corpus (bin/test-runtimes)
cargo test -p quilt-wasm --lib               # in-crate units (escape_html)
QUILT=bin/quilt node quilt-wasm/test/reduce.mjs   # `↓`, in node/ (bin/test-ts)
```

## API

Mirrors the Python runtime one-for-one where the ABIs allow:

| Python runtime        | quilt-wasm                                  |
| --------------------- | ------------------------------------------- |
| `tb(tag)` + `.c/.w/.n/.p/.x/.e/.b` | same fluent `WasmBuilder`      |
| `leaf/sym/quote/unquote/name`      | same free functions            |
| `cmd/write/push`                   | same free functions            |
| `NL`, `POP`, `HOLE` (constants)    | `NL()`, `POP()`, `HOLE()` (**functions** — see below) |
| `qlift`, `qlift_html`              | same; lift `number`/`string`/`boolean`, and a `QTerm` |
| `term.coparse()`                   | `term.coparse()` / `term.toString()` |
| `reduce(term)` / `term.↓`          | `node/` only: `reduce(term)` / `term.reduce()` |
| `expand(src, lang)`                | `node/` only: `expand(src, chain)` |

Builder and term methods that take `self`/a child by value **consume** the JS
object (wasm-bindgen move semantics), so chain in one expression and don't reuse
a spliced term.

## Divergences from the Python runtime

Two, both deliberate. The shared corpus (`conformance/runtime/cases.json`)
drives all three runtimes through the same cases, so a change to either of these
shows up as a test failure rather than as something a user discovers.

### `NL`/`POP`/`HOLE` are functions here, constants in Python

```js
cmd(NL())                                       // quilt-wasm
[cmd(write("[")), HOLE(), cmd(write("]"))]
```

```python
cmd(NL)                                         # quilt-python
[cmd(write("[")), HOLE, cmd(write("]"))]
```

Generated code is always correct for its own target — `TypeScriptMetaLanguage`
and `PythonMetaLanguage` each emit their runtime's spelling — so this costs
readers, not programs: porting a metaprogram between the two hosts means
adding or dropping the parens.

Making wasm match Python was investigated and rejected (issue #167). Two
independent things block it:

1. **wasm-bindgen cannot export a module-scope constant.** `#[wasm_bindgen]`
   on a `const` is a hard compile error — *"will not work on constants unless
   you are defining a `#[wasm_bindgen(typescript_custom_section)]`"* — and the
   only items that reach JS are functions, structs, enums and impls. A
   `static get` on an exported class yields `Consts.NL`, not the bare `NL`
   Python spells. Getting real bare constants means replacing wasm-pack's
   generated `pkg/` with a hand-maintained JS entry point and patching the
   package manifest the npm publish job ships — and in the `--target web` build
   that npm actually publishes, such constants could only be filled in *after*
   `await init()`, so they would read as `undefined` beforehand. That trades a
   visible divergence for a subtler one.
2. **A singleton would not survive reuse.** `quote`/`unquote` take
   `Vec<WasmCmdOrHole>`, and wasm-bindgen *moves* each element out of its JS
   wrapper, nulling the handle. One shared `HOLE` object is freed by the first
   `quote(..)` that uses it and throws `array contains a value of the wrong
   type` on the second. Calling `HOLE()` per use is what makes reuse safe.

### Move semantics

The `.c(&child)` / `.c(child)` split that separates the Rust and Python
runtimes shows up here too, one level deeper: a JS object passed by value is
**consumed**. Build in one chain, and don't reuse a term you have already
spliced.
