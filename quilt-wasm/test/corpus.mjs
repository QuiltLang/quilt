// The Node runner for the shared runtime corpus (issue #159).
//
// `conformance/runtime/cases.json` describes builder programs and the text each
// must coparse to. The Rust and Python runners execute the same file, so a
// divergence between the three published runtimes — quiltlang, quilt-python,
// quilt-wasm — is a test failure rather than something a user discovers.
//
//   bin/build-ts && node quilt-wasm/test/corpus.mjs
//
// Built --target nodejs (not --target web, which the npm package ships) purely
// so this runner can require() it without a fetch/init dance. The API surface
// is the same either way.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..");
const require = createRequire(import.meta.url);

const RUNTIME = "wasm";
const q = require(join(here, "..", "pkg", "quilt_wasm.js"));

const corpus = JSON.parse(
  readFileSync(join(repoRoot, "conformance", "runtime", "cases.json"), "utf8"),
);
const cases = corpus.cases.filter((c) => (c.runtimes ?? [RUNTIME]).includes(RUNTIME));

// The corpus-declared invariants this runner knows how to check (issue #192).
// An invariant that applies to wasm and is not in here is a *failure*, not a
// skip: the corpus decides what a runtime owes, so a runner must not quietly
// opt out of the property it has no coverage for.
const IMPLEMENTED_INVARIANTS = new Set(["stringify"]);
const invariants = (corpus.invariants ?? [])
  .filter((i) => (i.runtimes ?? [RUNTIME]).includes(RUNTIME))
  .map((i) => i.name);

function buildCmds(cmds) {
  return cmds.map((c) => {
    // The corpus spells a cmd abstractly ("NL", "HOLE", {write: s}) and every
    // runner maps it to its own API, so this is the same mapping the Rust and
    // Python runners do — not a special case for wasm. It just happens to be
    // where the one recorded divergence shows: NL/POP/HOLE are functions here
    // and constants in Python (issue #167; quilt-wasm/README.md has the
    // history).
    if (c === "HOLE") return q.HOLE();
    if (c === "NL") return q.cmd(q.NL());
    if (c === "POP") return q.cmd(q.POP());
    if ("write" in c) return q.cmd(q.write(c.write));
    if ("push" in c) return q.cmd(q.push(c.push));
    throw new Error(`unknown cmd ${JSON.stringify(c)}`);
  });
}

// A nested term means "lift an already-built term" — the case that exposes the
// ↓(↑(x)) == x divergence (issue #166).
const buildValue = (v) =>
  Array.isArray(v) ? v.map(buildValue) : v !== null && typeof v === "object" ? build(v) : v;

function build(t) {
  if ("leaf" in t) return q.leaf(t.leaf.tag, t.leaf.text);
  if ("sym" in t) return q.sym(t.sym);
  if ("name" in t) return q.name(t.name);
  if ("qlift" in t) return q.qlift(buildValue(t.qlift));
  if ("qlift_html" in t) return q.qlift_html(buildValue(t.qlift_html));
  if ("qlift_text" in t) return q.qlift_text(buildValue(t.qlift_text));
  if ("qlift_text_in_quotes" in t) return q.qlift_text_in_quotes(buildValue(t.qlift_text_in_quotes));
  if ("tb" in t) {
    let b = q.tb(t.tb.tag);
    for (const step of t.tb.steps) {
      if (step === "n") b = b.n();
      else if (step === "x") b = b.x();
      else if ("w" in step) b = b.w(step.w);
      else if ("c" in step) b = b.c(build(step.c));
      else if ("e" in step) b = b.e(build(step.e));
      else if ("p" in step) b = b.p(step.p);
      else throw new Error(`unknown step ${JSON.stringify(step)}`);
    }
    return b.b();
  }
  for (const [kind, ctor] of [["quote", q.quote], ["unquote", q.unquote]]) {
    if (kind in t) {
      const x = t[kind];
      return ctor(x.tag, x.index, x.lang, build(x.term), buildCmds(x.cmds));
    }
  }
  throw new Error(`unknown term ${JSON.stringify(t)}`);
}

if (cases.length === 0) {
  console.error(`no corpus cases applied to the ${RUNTIME} runtime`);
  process.exit(1);
}

const failures = [];

for (const name of invariants) {
  if (!IMPLEMENTED_INVARIANTS.has(name)) {
    failures.push(
      `invariant "${name}" applies to ${RUNTIME} but this runner does not implement it — implement it or narrow its \`runtimes\``,
    );
  }
}
const checkStringify = invariants.includes("stringify");

let stringifyChecked = 0;
for (const c of cases) {
  try {
    const term = build(c.term);
    const got = term.coparse();
    if (got !== c.coparse) {
      failures.push(`${c.name}: coparse is ${JSON.stringify(got)}, corpus says ${JSON.stringify(c.coparse)}`);
    }
    // `toString` and `coparse` are both exported and a caller may reasonably
    // use either, so they must not drift. quilt-python's `__str__` is the same
    // promise on the other runtime, checked against these same cases.
    // Going through String() rather than .toString() is deliberate: it is what
    // string concatenation and template literals do, so it also proves the
    // `js_name = toString` export is actually wired to the prototype.
    if (checkStringify) {
      stringifyChecked++;
      const str = String(term);
      if (str !== got) {
        failures.push(`${c.name}: String(term) is ${JSON.stringify(str)} but coparse() is ${JSON.stringify(got)}`);
      }
      if (`${term}` !== got) {
        failures.push(`${c.name}: template interpolation is ${JSON.stringify(`${term}`)} but coparse() is ${JSON.stringify(got)}`);
      }
    }
  } catch (e) {
    failures.push(`${c.name}: build failed: ${e.message ?? e}`);
  }
}

// ── the lift law ──────────────────────────────────────────────────────────
//
// reduce(lift(x)) == x. The corpus can only compare coparsed text; proving the
// law needs the generated code to actually be *evaluated*, which this runner can
// do and a JSON corpus cannot. Issue #166.
//
// The eval sees exactly the runtime's public names, as an expanded `.ts.quilt`
// module does after `import * from "quilt-wasm"`.
const { tb, leaf, sym, name, quote, unquote, cmd, write, push, NL, POP, HOLE } = q;
void [tb, leaf, sym, name, quote, unquote, cmd, write, push, NL, POP, HOLE];

const lawTerms = {
  leaf: () => q.leaf("number", "7"),
  sym: () => q.sym("+"),
  name: () => q.name("f"),
  binary: () =>
    q.tb("binary_expression").c(q.leaf("number", "1")).w(" ").c(q.sym("+")).w(" ")
      .c(q.leaf("number", "2")).b(),
  newline: () => q.tb("block").w("a").n().w("b").b(),
  prefix: () => q.tb("block").w("{").p("    ").n().w("body").x().n().w("}").b(),
  quote: () =>
    q.quote("x", 0, "ts", q.leaf("number", "5"),
      [q.cmd(q.write("[")), q.HOLE(), q.cmd(q.write("]"))]),
  unquote: () => q.unquote("x", 1, "ts", q.leaf("number", "5"), [q.HOLE()]),
};

let lawChecked = 0;
for (const [label, build_] of Object.entries(lawTerms)) {
  lawChecked++;
  const x = build_();
  const want = x.coparse();
  const code = q.qlift(x).coparse();
  try {
    // eval is the point: this *is* the reduce step.
    const back = eval(code);
    if (back.coparse() !== want) {
      failures.push(`lift law [${label}]: reduce(lift(x)) is ${JSON.stringify(back.coparse())}, x is ${JSON.stringify(want)} (via ${code})`);
    }
  } catch (e) {
    failures.push(`lift law [${label}]: evaluating ${code} threw: ${e.message ?? e}`);
  }
}

// A lift must not consume its argument: recovering a term from a polymorphic
// JsValue via TryFromJsValue *takes* it, nulling the caller's handle, so
// `qlift(t)` left `t` unusable. See issue #166.
for (const [label, fn] of [
  ["qlift", q.qlift],
  ["qlift_html", q.qlift_html],
  ["qlift_text", q.qlift_text],
]) {
  const t = q.leaf("text", "x");
  fn(t);
  try {
    t.coparse();
  } catch (e) {
    failures.push(`${label} consumed its argument: ${e.message ?? e}`);
  }
}

// Text dialects are code, so they cannot be corpus cases; this is the same
// check the Python runtime's test_main.py and the core's `lift::text` tests
// make. A dialect overrides only what differs, and what it leaves out is JSON's.
const conf = {
  list_open: "",
  list_sep: "\n",
  list_close: "",
  string: (s) => (/^\w+$/.test(s) ? s : JSON.stringify(s)),
  boolean: (b) => (b ? "on" : "off"),
};
for (const [got, want] of [
  [() => q.qlift_text("web", conf), "web"],
  [() => q.qlift_text("a b", conf), '"a b"'],
  [() => q.qlift_text(true, conf), "on"],
  [() => q.qlift_text(["x", "y z"], conf), 'x\n"y z"'],
  [() => q.qlift_text(1.5, conf), "1.5"],
  [() => q.qlift_text_in_quotes('q"', conf), 'q\\"'],
]) {
  const text = got().coparse();
  if (text !== want) failures.push(`text dialect: ${got} gave ${JSON.stringify(text)}, want ${JSON.stringify(want)}`);
}
for (const [label, bad] of [
  ["NaN in JSON", () => q.qlift_text(NaN)],
  ["a throwing rule", () => q.qlift_text("x", { string: () => { throw new Error("no"); } })],
]) {
  try {
    bad();
    failures.push(`text dialect: ${label} should have thrown`);
  } catch (_) {}
}

console.log(`${RUNTIME}: ${cases.length} corpus cases, ${stringifyChecked} stringify checks, ${lawChecked} lift-law checks, 3 non-consumption checks — ${failures.length} failure(s)`);
if (failures.length > 0) {
  console.error(`\n${failures.length} failure(s):`);
  for (const f of failures) console.error(`  • ${f}`);
  console.error("\nThe corpus is conformance/runtime/cases.json.");
  process.exit(1);
}
