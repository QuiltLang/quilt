// Repeated expansion must run in flat wasm memory.
//
// A quilt-wasm term used to be a JS wrapper around a Rust `Arc`, released when
// the garbage collector ran the wrapper's finalizer — which a synchronous loop
// never lets it do, so every expansion pinned a copy of its tree in wasm
// memory (tens of MB per expansion of a few thousand nodes). #297 had expanded
// code *take* the children it builds (`.t(..)`), so freeing the root gave the
// tree back; terms a program kept, and never freed, still stayed pinned.
//
// Terms are plain JavaScript objects now (#306), so wasm memory is not
// involved at all, freed or not.
//
// Run after `bin/build-ts`:
//   node quilt-wasm/test/memory.cjs

const assert = require("node:assert");
const { tb, leaf, sym, qlift_html, __memoryBytes } = require("../pkg/quilt_wasm.js");

// A host value the program keeps and splices into every page, as `↙footer↘`
// does. It is borrowed (`.e`), so it must survive every page being freed.
const footer = tb("element").t(sym("<hr>")).b();

// The shape `html↖<li><b>↙↑(title)↘</b> ↙↑(body)↘</li>↗` expands to.
function item(title, body) {
  return tb("element")
    .t(tb("start_tag").t(sym("<")).t(leaf("tag_name", "li")).t(sym(">")).b())
    .t(
      tb("element")
        .t(tb("start_tag").t(sym("<")).t(leaf("tag_name", "b")).t(sym(">")).b())
        .t(qlift_html(title))
        .t(tb("end_tag").t(sym("</")).t(leaf("tag_name", "b")).t(sym(">")).b())
        .b(),
    )
    .w(" ")
    .t(qlift_html(body))
    .t(tb("end_tag").t(sym("</")).t(leaf("tag_name", "li")).t(sym(">")).b())
    .b();
}

// A page of `n` items: about 20 terms each.
function page(n) {
  let b = tb("element").t(tb("start_tag").t(sym("<")).t(leaf("tag_name", "ul")).t(sym(">")).b());
  for (let i = 0; i < n; i++) b = b.t(item(`item ${i}`, `body <${i}>`)).n();
  return b.e(footer).t(tb("end_tag").t(sym("</")).t(leaf("tag_name", "ul")).t(sym(">")).b()).b();
}

function expandOnce() {
  const t = page(250);
  const html = t.coparse();
  t.free();
  return html;
}

// Warm up so the allocator reaches its working size, then measure.
let html;
for (let i = 0; i < 5; i++) html = expandOnce();
assert.ok(html.startsWith("<ul>") && html.endsWith("<hr></ul>"), html.slice(0, 80));
assert.ok(html.includes("<li><b>item 7</b> body &lt;7&gt;</li>"));

const before = __memoryBytes();
for (let i = 0; i < 200; i++) expandOnce();
const grown = __memoryBytes() - before;

// Each page is ~5000 terms; 200 pinned pages were well over 50 MB.
const limit = 2 * 1024 * 1024;
assert.ok(grown < limit, `wasm memory grew by ${(grown / 1e6).toFixed(1)} MB over 200 expansions`);

// Terms a program keeps and never frees cost no wasm memory either: they are
// the garbage collector's, like any other object.
const kept = [];
const before2 = __memoryBytes();
for (let i = 0; i < 50; i++) kept.push(page(250));
const grown2 = __memoryBytes() - before2;
assert.ok(grown2 < limit, `wasm memory grew by ${(grown2 / 1e6).toFixed(1)} MB holding 50 unfreed pages`);
assert.ok(kept.every((t) => t.coparse() === html));

// The borrowed host value is still usable after every page that held it.
assert.strictEqual(footer.coparse(), "<hr>");

console.log(
  `quilt-wasm memory test: grew ${(grown / 1e6).toFixed(1)} MB over 200 expansions, ` +
    `${(grown2 / 1e6).toFixed(1)} MB holding 50 unfreed pages`,
);
