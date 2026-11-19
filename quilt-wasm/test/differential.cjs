// The JavaScript runtime against the Rust one it replaced (issue #306).
//
// `coparse` and the scalar lifts are plain JavaScript now, and the Rust
// `QTerm` is reached only at the edges. Hold the two to the same output:
//
//   * every term in the shared corpus, and a few thousand random ones, coparse
//     the same in JavaScript as through the Rust `coparse` (one crossing, in
//     the flat encoding `qlift` of a term also uses);
//   * that encoding round-trips: lifting a term and evaluating the lift gives
//     the term back, for random terms too;
//   * numbers lift as the Rust `fmt_number` prints them.
//
// Run after `bin/build-ts`:
//   node quilt-wasm/test/differential.cjs

const assert = require("node:assert");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const q = require("../pkg/quilt_wasm.js");
const raw = require("../pkg/quilt_wasm_raw.js");
const { tb, leaf, sym, name, quote, unquote, cmd, write, push, NL, POP, HOLE, qlift, qlift_html, qlift_text, qlift_text_in_quotes } = q;

let checked = 0;
function same(t, label) {
  checked++;
  assert.strictEqual(t.coparse(), q.__coparseWasm(t), `${label}: JavaScript and Rust coparse differ`);
}

// ── the corpus ────────────────────────────────────────────────────────────

const corpus = JSON.parse(readFileSync(join(__dirname, "..", "..", "conformance", "runtime", "cases.json"), "utf8"));

function buildCmds(cmds) {
  return cmds.map((c) => {
    if (c === "HOLE") return HOLE();
    if (c === "NL") return cmd(NL());
    if (c === "POP") return cmd(POP());
    if ("write" in c) return cmd(write(c.write));
    return cmd(push(c.push));
  });
}

const value = (v) =>
  Array.isArray(v) ? v.map(value) : v !== null && typeof v === "object" ? build(v) : v;

function build(t) {
  if ("leaf" in t) return leaf(t.leaf.tag, t.leaf.text);
  if ("sym" in t) return sym(t.sym);
  if ("name" in t) return name(t.name);
  if ("qlift" in t) return qlift(value(t.qlift));
  if ("qlift_html" in t) return qlift_html(value(t.qlift_html));
  if ("qlift_text" in t) return qlift_text(value(t.qlift_text));
  if ("qlift_text_in_quotes" in t) return qlift_text_in_quotes(value(t.qlift_text_in_quotes));
  if ("tb" in t) {
    const b = tb(t.tb.tag);
    for (const step of t.tb.steps) {
      if (step === "n") b.n();
      else if (step === "x") b.x();
      else if ("w" in step) b.w(step.w);
      else if ("c" in step) b.c(build(step.c));
      else if ("e" in step) b.e(build(step.e));
      else b.p(step.p);
    }
    return b.b();
  }
  const x = t.quote ?? t.unquote;
  return (t.quote ? quote : unquote)(x.tag, x.index, x.lang, build(x.term), buildCmds(x.cmds));
}

for (const c of corpus.cases) {
  if (!(c.runtimes ?? ["wasm"]).includes("wasm")) continue;
  same(build(c.term), c.name);
}

// ── random terms ──────────────────────────────────────────────────────────

// A small deterministic PRNG, so a failure reproduces.
let seed = 306;
const rand = (n) => {
  seed = (seed * 1103515245 + 12345) & 0x7fffffff;
  return seed % n;
};
const TEXT = ["", "a", " ", "\n", "  ", "x y", '"', "\\", "<&>", "↑", "é", "\u0007", "\u{1F600}", "\ud800"];
const text = () => TEXT[rand(TEXT.length)] + (rand(3) === 0 ? TEXT[rand(TEXT.length)] : "");

function randomCmds(holes) {
  const cmds = [];
  for (let i = 0, n = rand(5); i < n; i++) {
    const k = rand(4);
    cmds.push(k === 0 ? cmd(write(text())) : k === 1 ? cmd(NL()) : k === 2 ? cmd(push(text())) : cmd(POP()));
  }
  for (let i = 0; i < holes; i++) cmds.splice(rand(cmds.length + 1), 0, HOLE());
  return cmds;
}

function randomTerm(depth) {
  const k = depth > 4 ? rand(3) : rand(8);
  if (k === 0) return leaf(text(), text());
  if (k === 1) return sym(text());
  if (k === 2) return [qlift_html, qlift][rand(2)]([text(), rand(1000) - 500, rand(2) === 0][rand(3)]);
  if (k === 3) {
    const f = rand(2) === 0 ? quote : unquote;
    return f(text(), rand(4), "ts", randomTerm(depth + 1), randomCmds(rand(3)));
  }
  const b = tb(text());
  for (let i = 0, n = rand(7); i < n; i++) {
    const s = rand(6);
    if (s === 0) b.w(text());
    else if (s === 1) b.n();
    else if (s === 2) b.p(text());
    else if (s === 3) b.x();
    else b.t(randomTerm(depth + 1));
  }
  return b.b();
}

const randoms = Array.from({ length: 2000 }, () => randomTerm(0));
randoms.forEach((t, i) => same(t, `random term ${i}`));

// ── the lift law, over the flat encoding ──────────────────────────────────

// `↓(↑(x)) == x`: what qlift of a term returns is TypeScript that rebuilds it,
// so evaluating it with the runtime in scope gives back the same text.
const scope = { tb, leaf, sym, name, quote, unquote, cmd, write, push, NL, POP, HOLE };
const evaluate = new Function(...Object.keys(scope), "code", "return eval(code);").bind(null, ...Object.values(scope));
let lifted = 0;
for (const t of randoms.slice(0, 500)) {
  const code = qlift(t).coparse();
  assert.strictEqual(evaluate(code).coparse(), t.coparse(), `reduce(lift(x)) != x via ${code}`);
  lifted++;
}

// ── numbers ───────────────────────────────────────────────────────────────

const NUMBERS = [
  0, -0, 1, -1, 42, 3.5, -2.25, 0.1, 0.1 + 0.2, 1 / 3, 1e-6, 1.5e-6, 9.99e-7, 1e-7, 1e-300, 5e-324,
  2 ** 53 - 1, 2 ** 53, 2 ** 53 + 2, 2 ** 60, 1e20, 1e21, 1.5e21, 1e300, Number.MAX_VALUE,
  -1e21, -(2 ** 53), NaN, Infinity, -Infinity, 123456789.123, 0.000123,
];
for (const n of NUMBERS) {
  assert.strictEqual(qlift(n).coparse(), raw.fmt_number(n), `qlift(${n})`);
  assert.strictEqual(qlift_html(n).coparse(), raw.fmt_number(n), `qlift_html(${n})`);
}

console.log(
  `quilt-wasm differential test: ${checked} terms coparse alike, ${lifted} lifts round-trip, ${NUMBERS.length} numbers format alike`,
);
