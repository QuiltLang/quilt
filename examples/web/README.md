# Browser demo: TypeScript generates HTML

A Quilt meta demo (issue #46): a TypeScript program generates HTML and runs
**in the browser** against the `quilt-wasm` runtime.

- `cards.html.ts.quilt` — the source. Ground language TypeScript; un-annotated
  quotes are HTML (the `.html.ts` chain). It exports `render()`, which builds an
  HTML fragment from quoted `html↖…↗` templates and `coparse()`s it. It is
  deliberately annotation-free, so its expansion is valid JavaScript that a
  browser loads as an ES module with no transpile step.
- `cards.html.ts` — the committed expansion (`quilt expand cards.html.ts.quilt`):
  plain TypeScript whose `html↖…↗` quotes have become `quilt-wasm` builder calls.
- `index.html` — the page shell. An import map resolves the bare `quilt-wasm`
  specifier to the runtime; a module script initialises the WebAssembly and
  injects `render()` into the page.

Expansion happens **ahead of time** here (`quilt expand`). Doing the expansion
*in* the browser too is the meta-meta demo (issue #47).

## Run it

Requires the Quilt dev env (`wasm-pack` + `node`):

```sh
node examples/web/build.mjs                          # → examples/web/dist/
python3 -m http.server -d examples/web/dist 8000     # open http://localhost:8000
```

`build.mjs` builds the runtime (`wasm-pack build quilt-wasm --target web`),
copies it plus the expanded program and `index.html` into `dist/` (git-ignored).

## Verify headlessly

```sh
node examples/web/verify.mjs
```

Builds `dist/`, then loads the runtime and the expanded program exactly as the
page does — but in Node, initialising the WebAssembly from the bundled bytes —
and asserts the rendered HTML (including that lifted `<script>`/`"quotes"` come
out entity-escaped).

## Regenerate the expansion

After editing `cards.html.ts.quilt`:

```sh
quilt expand examples/web/cards.html.ts.quilt
```
