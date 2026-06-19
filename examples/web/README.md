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

## 2. Staged playground (`/playground.html`, issue #47)

**TypeScript that writes TypeScript that writes HTML** — three staged passes, in
the browser and driven by a clock. Everything runs client-side:

```
source ──(WASI shim + quilt-expand.wasm)──▶ Stage 1: makeRenderer  (TypeScript)
makeRenderer(schema) ──(↓ reduce: re-expand + eval)──▶ Stage 2: start()  (TypeScript)
start(setHtml, read) ──(its own baked setInterval)───▶ Stage 3: HTML, looping
```

`dashboard.html.ts.ts.quilt` is a self-specializing live dashboard. **Stage 1**
(`makeRenderer`) runs once: it loops over the chosen metrics and *unrolls* them
into a flat, branch-free **Stage 2** (`start()`) — there is no loop and no schema
left in the generated code (the metric loop is fully unrolled). `start()` carries
its own update loop, with the interval baked in from `opts.intervalMs`. **Stage
3** is the HTML that loop sets every tick — no further expansion, no
interpretation; the page only supplies the HTML sink and the readings feed.
Editing the source or pressing *Reconfigure* reruns the expensive Stage 1; the
codegened loop stays cheap. The page shows the timings so the contrast is visible.

- `quilt-expand.wasm` — the Quilt parser+expander. Unlike the runtime, expansion
  needs the `parse` feature (tree-sitter + the C grammars), which needs a libc,
  so it is built for `wasm32-wasip1` (see `bin/build-expand-wasm`, the
  `quilt-expand-wasm` crate).
- `quilt-rt.js` — the reduce-enabled `quilt` wrapper, and the new piece that
  makes staging work in the browser. The TypeScript meta already spells `↓` as
  `term.reduce()`, but the wasm runtime has no `reduce()`: reduce must *re-expand*
  a generated stage (it still quotes HTML), and the expander is a separate WASI
  module. So, exactly like the Python runtime's `_reduce_src` shells out to the
  `quilt` binary, `quilt-rt.js` adds `reduce()` in JS — coparse → expand → eval —
  re-exporting the runtime and accepting the page's expander via `setExpander`.
  The import map binds the bare `quilt` specifier to it (and `quilt-wasm` to the
  runtime).
- `wasi-shim.js` — a tiny hand-rolled WASI preview1 shim (zero deps) that runs
  the expander command, wiring argv / stdin / stdout to in-memory buffers.
- `playground.js` / `playground.html` — host the pipeline: expand Stage 1, import
  it, call `makeRenderer` (which reduces to `start()`), then hand `start()` the
  HTML sink + readings feed and let *its* codegened loop drive the preview. The
  source editor is a zero-dependency highlighter (a
  coloured `<pre>` behind a transparent `<textarea>`) that also colours the Quilt
  arrow glyphs. Since those glyphs can't be typed, a button row inserts them
  (`↖↗` `↙↘` `↑` `↓` `←`), and the keyboard uses the same chord scheme as the VS
  Code extension (`tools/quilt`): leader `⌘`/`Ctrl`+`1` then a direction (`↑↓←→`
  or `hjkl`) for a single glyph, leader `⌘`/`Ctrl`+`2` then two directions for a
  diagonal (e.g. up-then-left → `↖`). `⌘`/`Ctrl`+`Enter` expands & runs.
- `theme.css` — the shared site theme (brand palette + per-glyph syntax colours,
  mirroring the docs site's `custom.css`). Both demo pages link it, and the
  playground links it from the *rendered preview* by a relative href (the live
  dashboard styles are scoped to `.preview`), so the generated HTML is themed like
  the site without any inlined CSS.

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
node examples/web/verify.mjs             # demo 1: runtime renders the expansion
node examples/web/verify-dashboard.mjs   # demo 2: source → expand → ↓ reduce → render → HTML
```

Both load the same modules the page does but in Node (initialising the
WebAssembly from bytes). `verify.mjs` asserts the cards demo's escaped output;
`verify-dashboard.mjs` runs the whole staged pipeline and asserts the generated
`start()` is unrolled (one gauge per metric, no schema loop), codegens its own
`setInterval` with the interval baked in, that running frames triggers no
expansion, and that the live values land in the HTML.

## Regenerate the committed expansion

After editing `cards.html.ts.quilt` (the ahead-of-time demo):

```sh
quilt expand examples/web/cards.html.ts.quilt
```
