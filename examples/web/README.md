# Browser demos: TypeScript generates HTML

Two demos that run Quilt in the browser against the `quilt-wasm` runtime.

## 1. Ahead-of-time (`/index.html`, issue #46)

A TypeScript program generates HTML; expansion is done offline, the result runs
in the browser.

- `cards.html.ts.quilt` — the source. Ground language TypeScript; un-annotated
  quotes are HTML (the `.html.ts` chain). It exports `render()`, which builds an
  HTML fragment from quoted `html↖…↗` templates and returns it as a Quilt term;
  the harness that calls `render()` is what `coparse()`s the term into a string.
  It is deliberately annotation-free, so its expansion is valid JavaScript that a
  browser loads as an ES module with no transpile step.
- `cards.html.ts` — the committed expansion (`quilt expand cards.html.ts.quilt`):
  plain TypeScript whose `html↖…↗` quotes have become `quilt-wasm` builder calls.
- `index.html` — the page shell. An import map resolves the bare `quilt`
  specifier to the runtime (published on npm as `quilt-wasm`); a module script
  initialises the WebAssembly and injects `render().coparse()` into the page.

## 2. Meta-meta playground (`/playground.html`, issue #47)

The **expansion itself** runs in the browser: edit the `.html.ts.quilt` source,
press *Expand & run*, and the whole pipeline runs client-side —

```
source --(WASI shim + quilt-expand.wasm)--> TypeScript --(import + runtime)--> HTML
```

- `quilt-expand.wasm` — the Quilt parser+expander. Unlike the runtime, expansion
  needs the `parse` feature (tree-sitter + the C grammars), which needs a libc,
  so it is built for `wasm32-wasip1` (see `bin/build-expand-wasm`, the
  `quilt-expand-wasm` crate). 
- `wasi-shim.js` — a tiny hand-rolled WASI preview1 shim (zero deps) that runs
  the expander command, wiring argv / stdin / stdout to in-memory buffers.
- `playground.js` / `playground.html` — drive the loop: run the expander, show
  the expanded TypeScript, import it as a module (its bare `quilt` import
  resolves through the page import map to the runtime), and render the result.
  The source editor is a zero-dependency highlighter (a coloured `<pre>` behind
  a transparent `<textarea>`) that also colours the Quilt arrow glyphs. Since
  those glyphs can't be typed, a button row inserts them (`↖↗` `↙↘` `↑` `↓` `←`),
  and the keyboard uses the same chord scheme as the VS Code extension
  (`tools/quilt`): leader `⌘`/`Ctrl`+`1` then a direction (`↑↓←→` or `hjkl`)
  for a single glyph, leader `⌘`/`Ctrl`+`2` then two directions for a diagonal
  (e.g. up-then-left → `↖`). `⌘`/`Ctrl`+`Enter` expands & runs.
- `theme.css` — the shared site theme (brand palette + per-glyph syntax colours,
  mirroring the docs site's `custom.css`). Both demo pages link it, and the
  playground links it from the *rendered preview* by a relative href, so the
  generated HTML is themed like the site without any inlined CSS.

The *run* step imports the expansion as a module, so it needs the expansion to
be valid JS (annotation-free); the default source is.

## Run them

```sh
node examples/web/build.mjs                          # → examples/web/dist/
python3 -m http.server -d examples/web/dist 8000     # open http://localhost:8000
```

`build.mjs` builds the runtime (`wasm-pack build quilt-wasm --target web`) and
assembles `dist/` (git-ignored). The playground also needs the expander wasm,
which needs a **WASI sdk** for the C grammars: install one from
<https://github.com/WebAssembly/wasi-sdk/releases> and set `WASI_SDK_PATH` (the
default is `$HOME/wasi-sdk-33.0-arm64-macos`). Without it, `build.mjs` skips the
playground and builds only demo 1.

## Verify headlessly

```sh
node examples/web/verify.mjs              # demo 1: runtime renders the expansion
node examples/web/verify-playground.mjs   # demo 2: full source → expand → run loop
```

Both load the same modules the page does but in Node (initialising the
WebAssembly from bytes), and assert the rendered HTML — including that lifted
`<script>`/`"quotes"` come out entity-escaped.

## Regenerate the committed expansion

After editing `cards.html.ts.quilt`:

```sh
quilt expand examples/web/cards.html.ts.quilt
```
