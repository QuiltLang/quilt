// How long one expansion of a few thousand terms takes, against the string
// template that produces the same text (issue #306).
//
// The page is the issue's: 150 items, each the expanded form of
// `html↖<li><b>↙↑(t)↘</b> ↙↑(b)↘</li>↗` — about 3000 terms in all — built
// with the exact calls `quilt expand` emits for it, then coparsed.
//
// Run after `bin/build-ts`:
//   node quilt-wasm/test/bench.cjs            # this checkout's runtime
//   node quilt-wasm/test/bench.cjs path/pkg   # another build, to compare
//
// Not a gate: it only checks that every row produces the same text, and
// prints the timings.

const assert = require("node:assert");
const { join, resolve } = require("node:path");

const pkg = process.argv[2] ? resolve(process.argv[2]) : join(__dirname, "..", "pkg");
const q = require(join(pkg, "quilt_wasm.js"));
const { tb, leaf, sym, qlift_html } = q;

const ITEMS = Array.from({ length: 150 }, (_, i) => [`item ${i}`, `body <${i}> & "${i}"`]);

// `quilt expand` of `html↖<li><b>↙↑(t)↘</b> ↙↑(b)↘</li>↗`, verbatim.
function item(t, b) {
  return tb("element").t(tb("start_tag").t(sym("<")).t(leaf("tag_name", "li")).t(sym(">")).b()).t(tb("element").t(tb("start_tag").t(sym("<")).t(leaf("tag_name", "b")).t(sym(">")).b()).t(qlift_html(t)).t(tb("end_tag").t(sym("</")).t(leaf("tag_name", "b")).t(sym(">")).b()).b()).w(" ").t(qlift_html(b)).t(tb("end_tag").t(sym("</")).t(leaf("tag_name", "li")).t(sym(">")).b()).b();
}

// `html↖<ul>↙ul.b()↘</ul>↗` around an accumulated builder of items.
function page() {
  let ul = tb("element");
  for (const [t, b] of ITEMS) ul = ul.t(item(t, b)).n();
  return tb("element").t(tb("start_tag").t(sym("<")).t(leaf("tag_name", "ul")).t(sym(">")).b()).e(ul.b()).t(tb("end_tag").t(sym("</")).t(leaf("tag_name", "ul")).t(sym(">")).b()).b();
}

const free = (t) => (typeof t.free === "function" ? t.free() : undefined);

// The same text from a string template, escaping as `qlift_html` does.
const ESC = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;" };
const esc = (s) => s.replace(/[&<>"']/g, (c) => ESC[c]);
const template = () => `<ul>${ITEMS.map(([t, b]) => `<li><b>${esc(t)}</b> ${esc(b)}</li>\n`).join("")}</ul>`;

const rows = [
  ["build + coparse", () => { const t = page(); const s = t.coparse(); free(t); return s; }],
  ["build only", () => { const t = page(); free(t); return null; }],
];
if (typeof q.__coparseWasm === "function") {
  // The other way issue #306 weighs: build in JS, coparse in wasm.
  rows.push(["build + coparse in wasm", () => q.__coparseWasm(page())]);
}
rows.push(["string template", template]);

const want = template();
for (const [label, run] of rows) {
  const got = run();
  if (got !== null) assert.strictEqual(got, want, `${label} disagrees with the template`);
}

// The median of BATCHES batches of RUNS runs each, after a warm-up, so one
// garbage collection or a busy machine does not decide a row.
const WARM = 100;
const BATCHES = 21;
const RUNS = 50;
console.log(`quilt-wasm bench: 150 items (~3000 terms), median of ${BATCHES}x${RUNS} runs, node ${process.version}`);
for (const [label, run] of rows) {
  for (let i = 0; i < WARM; i++) run();
  const times = [];
  for (let b = 0; b < BATCHES; b++) {
    const t0 = process.hrtime.bigint();
    for (let i = 0; i < RUNS; i++) run();
    times.push(Number(process.hrtime.bigint() - t0) / 1e6 / RUNS);
  }
  times.sort((a, b) => a - b);
  const ms = times[BATCHES >> 1];
  console.log(`  ${label.padEnd(24)} ${ms.toFixed(3).padStart(8)} ms`);
}
