// The three quote representations build the same code at run time.
//
// One writer, expanded three times with only its first quote's tag changed —
// `ts↖` (⟨cst⟩), `ts⟨str⟩↖` and `ts⟨qt⟩↖` — each later quote inheriting it.
// Every representation must produce the same text, including a multi-line
// value spliced into an indented block, lifts, `←` and holes in comments.
// The one place `⟨str⟩` differs is pinned below too: a host string holding a
// newline, spliced under a prefix, is re-indented by `⟨str⟩` and written raw
// by the other two.
//
//   bin/build-ts && QUILT=target/debug/quilt node quilt-wasm/test/repr.mjs

import assert from "node:assert";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const QUILT = process.env.QUILT;
if (!QUILT) throw new Error("set QUILT to the quilt binary (bin/test-ts does)");
const runtime = pathToFileURL(fileURLToPath(new URL("../node/index.mjs", import.meta.url))).href;

const WRITER = `
export function program(items, rest) {
  return TAG↖
    // ↙items.length↘ items, named ↙items.map((it) => it.name).join(", ")↘
    function register() {
      ↙←(items.map(item))↘
      return [↙←(items.map((it) => ↖↙↑(it.name)↘↗), ", ")↘];
    }
    ↙rest↘
  ↗;
}

function item(it) {
  const body = it.big ? ↖{
  big: true,
  n: ↙it.n↘,
}↗ : ↖↙it.n↘↗;
  return ↖table[↙↑(it.name)↘] = ↙body↘;↗;
}

export const literal = TAG↖const s = \`a
b\`;↗;
export function wrapped(x) {
  return ↖{
    ↙x↘
  }↗;
}
`;

const dir = mkdtempSync(join(tmpdir(), "quilt-repr-"));
const prelude =
  `import { tb, leaf, sym, quote, unquote, cmd, write, push, name, qseq, qjoin, qlift_str, ` +
  `qlift, NL, POP, HOLE } from ${JSON.stringify(runtime)};`;
const load = async (tag) => {
  const file = join(dir, `w_${tag.replace(/\W/g, "") || "cst"}.ts.quilt`);
  writeFileSync(file, WRITER.replaceAll("TAG↖", `${tag}↖`));
  execFileSync(QUILT, ["expand", "--prelude", prelude, file]);
  const out = file.replace(/\.quilt$/, ".mjs");
  writeFileSync(out, readFileSync(file.replace(/\.quilt$/, "")));
  return import(pathToFileURL(out).href);
};

const reps = { cst: await load("ts"), str: await load("ts⟨str⟩"), qt: await load("ts⟨qt⟩") };
const items = [
  { name: 'say "hi"', n: 1, big: true },
  { name: "b", n: 2, big: false },
];
const rest = "done();\nreturn;";
const text = (x) => (typeof x === "string" ? x : x.coparse());

const want = text(reps.cst.program(items, rest));
assert.strictEqual(
  want,
  [
    `// 2 items, named say "hi", b`,
    `function register() {`,
    `  table["say \\"hi\\""] = {`,
    `    big: true,`,
    `    n: 1,`,
    `  };`,
    `  table["b"] = 2;`,
    `  return ["say \\"hi\\"", "b"];`,
    `}`,
    `done();\nreturn;`,
  ].join("\n"),
);
for (const r of ["str", "qt"]) {
  assert.strictEqual(text(reps[r].program(items, rest)), want, `⟨${r}⟩ program`);
}
// `⟨str⟩` values are strings, and a string's coparse() is itself.
assert.strictEqual(typeof reps.str.program(items, rest), "string");
assert.strictEqual(reps.str.program(items, rest).coparse(), want);

// A newline inside a token of a *quoted* value (a template literal) is laid
// out like any other in all three: Quilt splits a quote's lines before the
// grammar sees them, so `⟨cst⟩` indents it too, and `⟨str⟩` agrees.
for (const r of ["cst", "str", "qt"]) {
  assert.strictEqual(text(reps[r].wrapped(reps[r].literal)), "{\n  const s = `a\n  b`;\n}", r);
}

// The one difference: a host *string* with a newline, spliced under a prefix.
// `⟨cst⟩` and `⟨qt⟩` write a string as raw text, so its later lines start flush
// left; `⟨str⟩` re-indents every line of what it splices, as hand-written
// string codegen would.
for (const r of ["cst", "qt"]) {
  assert.strictEqual(text(reps[r].wrapped("a();\nb();")), "{\n  a();\nb();\n}", r);
}
assert.strictEqual(reps.str.wrapped("a();\nb();"), "{\n  a();\n  b();\n}");

console.log("quilt-wasm repr test: all assertions passed");
