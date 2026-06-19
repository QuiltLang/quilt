// Reduce-enabled wrapper around the quilt-wasm runtime — this is what the page
// import map binds the bare `quilt` specifier to (it re-exports the whole
// runtime, plus the missing `↓` operator).
//
// `↓` (reduce) is the engine behind staging: it takes a term whose coparse() is
// *still Quilt source* — a generated stage that itself quotes — expands it, and
// runs it, yielding the value (here, the next stage's function). The TypeScript
// meta already spells `↓` as `term.reduce()`; the wasm runtime just never had a
// `reduce()` to call. It cannot: reduce must re-expand, and the expander is a
// separate WASI module (the C tree-sitter grammars need a libc). So reduce is
// JS-orchestrated, exactly like the Python runtime's `_reduce_src` shells out to
// the `quilt` binary — see quilt-python/quilt/__init__.py. Here the "shell out"
// is a call into the in-page wasm expander, registered via `setExpander`.

import * as RT from "quilt-wasm";
export * from "quilt-wasm";
export { default } from "quilt-wasm"; // the runtime's init(); callers `await` it

// The runtime functions the expanded stage code calls. Passed into the reduce
// sandbox by name so a stage's `tb(...).c(...)` etc. resolve without an import.
const RT_NAMES = [
  "tb", "leaf", "sym", "quote", "unquote", "cmd", "write", "push",
  "name", "qlift", "qlift_html", "NL", "POP", "HOLE",
];

// Presence of any glyph means coparse() returned Quilt source (a generated stage
// that itself quotes), so it must be expanded before it can run.
const GLYPHS = "↖↗↙↘↑↓←⟨⟩";
const hasGlyph = (s) => [...GLYPHS].some((g) => s.includes(g));

// The expander (source string → expanded TypeScript). Injected by the page,
// which owns the WASI shim + expander wasm. Kept here so `term.reduce()` works
// anywhere the term flows, including inside other expanded modules.
let expander = null;
export function setExpander(fn) {
  expander = fn;
}

// A record of what each reduce ran, newest last: `{ generated, expanded }`,
// where `generated` is the stage's Quilt source (what the previous stage wrote)
// and `expanded` is the plain TypeScript it became. The playground reads this to
// show "the program this program wrote" at each stage. Cleared per restage.
export const reduceTrace = [];
export function clearReduceTrace() {
  reduceTrace.length = 0;
}

// `quilt expand` prepends a `//! DO NOT EDIT` banner; drop it so the remainder
// is a bare expression we can wrap and evaluate.
function stripBanner(src) {
  const lines = src.split("\n");
  while (lines.length && lines[0].startsWith("//!")) lines.shift();
  return lines.join("\n").replace(/^\n+/, "");
}

// Reduce Quilt/target source to a value — the engine behind `↓`. If the source
// still holds glyphs it is expanded first; the result is evaluated as a single
// expression (our stages are arrow functions) with the runtime in scope. This
// mirrors quilt-python's `_reduce_src`, minus the block-with-trailing-expression
// case, which JS can't do without a parser — stages here are expressions.
function reduceSrc(src) {
  const generated = src;
  let code = src;
  if (hasGlyph(code)) {
    if (!expander) {
      throw new Error("reduce (↓): no expander registered — call setExpander() first");
    }
    code = stripBanner(expander(code));
  }
  reduceTrace.push({ generated, expanded: code });
  const fn = new Function(...RT_NAMES, `"use strict";\nreturn (\n${code}\n);`);
  return fn(...RT_NAMES.map((n) => RT[n]));
}

// Install the `↓` operator. `term.↓` expands (TS meta) to `term.reduce()`.
RT.WasmQTerm.prototype.reduce = function reduce() {
  return reduceSrc(this.coparse());
};
