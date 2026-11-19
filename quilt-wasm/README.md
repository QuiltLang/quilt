# quilt-wasm

Quilt's core IR for JavaScript — the runtime that expanded `.ts.quilt` programs
target, in a browser and under Node alike. It is the JS analog of the `quilt`
Python module (`quilt-python/`): the same `QTerm` builder, `qlift`/`qlift_html`,
and `coparse` serializer.

The builder and `coparse` are plain JavaScript (`js/runtime.js`): a term is an
ordinary object the garbage collector owns. The wasm-bindgen crate (`src/`) is
only the edges that need Rust — `qlift` of an already-built term, whose output
comes from the generated tables in `quilt::langs::typescript::ops`, and the
numbers JavaScript would print in exponent form. Those cross once per call, the
term sent as a flat buffer (issue #306). See [Performance](#performance).

Like `nanobots-codegen`, it depends on `quilt` with
`default-features = false, features = ["rust"]`, so it uses only the
tree-sitter-free runtime path and builds for `wasm32-unknown-unknown` with no C
runtime. (Compiling the *parser/expander* to wasm — for the meta-meta demo — is
tracked separately as Phase 2 of issue #43.)

## Build

```sh
# from the repo root
bin/build-ts                     # for `quilt run` and the tests (= bin/build-wasm nodejs pkg)
bin/build-wasm web pkg-web       # for the browser demos and `quilt notebook --serve` (ESM)
bin/build-wasm web pkg           # the npm package
```

`bin/build-wasm` runs `wasm-pack build --out-name quilt_wasm_raw`, then
`js/assemble.mjs`, which writes the package entry `quilt_wasm.js` (the runtime,
bound to the wasm), its `.d.ts`, and `quilt_runtime.js` beside wasm-pack's
output, and points `package.json` at them. A bare `wasm-pack build` is *not*
the package: it has only the edge functions.

The artifact lands in `quilt-wasm/pkg/` (git-ignored); `examples/web/build.mjs`
puts its `web` build in `pkg-web/`, so the two never clobber each other.

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
passes): it does `bin/build-wasm web pkg` and `npm publish`es
the resulting `pkg/`. The package version tracks the workspace version in
`Cargo.toml`.

Auth is **npm Trusted Publishing (OIDC)** — no secret to manage; GitHub's
`id-token` authenticates the publish and npm records build provenance. One-time
setup, because npm can only attach a trusted publisher to a package that already
exists:

1. Publish the first version manually: `bin/build-wasm web pkg`,
   then `npm login` and `npm publish` from `quilt-wasm/pkg/`.
2. On npmjs.com → the package → **Settings → Trusted Publisher**, add a GitHub
   Actions publisher for repo `QuiltLang/quilt`, workflow `ci.yml`.

After that every `v*` tag publishes with no token. A version already on npm is a
no-op, not a failure.

## Tests

```sh
bin/build-ts
node quilt-wasm/test/smoke.cjs               # the builder API
node quilt-wasm/test/memory.cjs              # expansion costs no wasm memory (bin/test-ts)
node quilt-wasm/test/differential.cjs        # JS coparse/lifts = the Rust QTerm's (bin/test-ts)
node quilt-wasm/test/corpus.mjs              # the shared runtime corpus (bin/test-runtimes)
node quilt-wasm/test/bench.cjs [other/pkg]   # timings against a string template; not a gate
cargo test -p quilt-wasm --lib               # the flat encoding
QUILT=bin/quilt node quilt-wasm/test/reduce.mjs   # `↓`, in node/ (bin/test-ts)
```

## API

Mirrors the Python runtime one-for-one where the ABIs allow:

| Python runtime        | quilt-wasm                                  |
| --------------------- | ------------------------------------------- |
| `tb(tag)` + `.c/.w/.n/.p/.x/.e/.b` | same fluent `WasmBuilder`      |
| (none)                             | `.t(child)`: the same as `.c`; what expanded code spells (see Memory) |
| `leaf/sym/quote/unquote/name`      | same free functions            |
| `cmd/write/push`                   | same free functions            |
| `NL`, `POP`, `HOLE` (constants)    | `NL()`, `POP()`, `HOLE()` (**functions** — see below) |
| `qlift`, `qlift_html`              | same; lift `number`/`string`/`boolean`, and a `QTerm` |
| `term.coparse()`                   | `term.coparse()` / `term.toString()` |
| `reduce(term)` / `term.↓`          | `node/` only: `reduce(term)` / `term.reduce()` |
| `expand(src, lang)`                | `node/` only: `expand(src, chain)` |

A term is immutable once built, so it can be spliced any number of times and
kept. A builder's methods append to it and return it, so chain in one
expression or reassign (`b = b.e(x)`), as you would in Rust.

## Memory

Terms are plain JavaScript objects, freed by the garbage collector like any
other, whether a quote built them or the program keeps them in a variable.
Nothing is held in wasm memory between calls. `free()` and `using` still work,
as no-ops, for code written when a term was a wasm handle and had to be freed
(#297); `.t(child)` is from then too.

`test/memory.cjs` checks that repeated expansion, and holding the results,
costs no wasm memory.

## Performance

Until #306 a term was a wasm-bindgen wrapper around a Rust `Arc<QTerm>`. Every
builder call crossed into wasm, allocated a wrapper, and registered it with a
`FinalizationRegistry`, so most of an expansion's time went to the boundary,
not to building. `test/bench.cjs` builds the page from the issue — 150 items of
`html↖<li><b>↙↑(t)↘</b> ↙↑(b)↘</li>↗`, about 3000 terms — and compares it with
a string template that produces the same text:

| | ms per page |
| --- | ---: |
| wasm-bindgen terms (before #306) | ~17 |
| JavaScript terms, `coparse` in JavaScript | ~0.25 |
| JavaScript terms, `coparse` in wasm (one crossing) | ~2.3 |
| string template | ~0.07 |

(Node 22, Apple silicon; run it for your own numbers.) The third row is the
other design #306 weighed: building in JavaScript and crossing once to coparse
in Rust. Copying the tree across costs more than writing it, so `coparse` is in
JavaScript, held to the Rust one by `test/differential.cjs` and the corpus.

## Divergences from the Python runtime

One, deliberate. The shared corpus (`conformance/runtime/cases.json`) drives all
three runtimes through the same cases, so a change to it shows up as a test
failure rather than as something a user discovers.

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

Issue #167 kept the functions for two reasons, both about wasm-bindgen: it
cannot export a module-scope constant, and it *moved* each element of a
`quote(..)`'s cmds out of its JS wrapper, so a shared `HOLE` would have been
freed by its first use. Neither holds since #306 — the package entry is
hand-written JavaScript, and `NL()`, `POP()` and `HOLE()` return shared
immutable values — but the TypeScript meta still emits the calls, and changing
that is a change to every expansion, so they stay functions for now.

Move semantics, the other divergence the wasm-bindgen classes had (a term
passed by value was consumed), went with them: nothing here consumes its
argument.
