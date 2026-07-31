// Tests for the Node runtime's `↓` (reduce) operator — issue #153, where `↓`
// worked in the browser playground and had no native backend at all.
//
// These drive quilt-wasm/node/index.mjs directly, so they cover the runtime
// rather than the CLI wiring that binds it to the bare `quilt` specifier;
// `bin/test-ts` runs the .ts.quilt examples for the end-to-end path.
//
//   bin/build-ts && QUILT=bin/quilt node quilt-wasm/test/reduce.mjs
//
// $QUILT is the expander reduce shells out to when a generated stage still
// holds Quilt glyphs (`quilt run` sets it automatically).

import assert from "node:assert";
import * as q from "../node/index.mjs";

// A term whose coparse() is exactly `code` — the shape reduce consumes.
const src = (code) => q.leaf("_", code);

// 1. A plain expression reduces to its value.
assert.strictEqual(src("1 + 2").reduce(), 3);
assert.strictEqual(q.reduce(src('"a" + "b"')), "ab");

// 2. Block semantics: leading statements run, the trailing expression is the
//    value — as in quilt-python, and unlike the browser shim, which can only
//    evaluate a single expression.
assert.strictEqual(src("const a = 2; a * 21").reduce(), 42);
assert.strictEqual(src("let n = 0; for (let i = 1; i <= 4; i++) n += i; n").reduce(), 10);

// 3. A script ending in a statement has no value.
assert.strictEqual(src("const unused = 1;").reduce(), undefined);

// 4. The runtime is in scope, so a reduced stage can build terms — and they are
//    real terms from this same wasm instance, so they splice back in.
const built = src('tb("binary_expression").c(leaf("number", "1")).w(" + ").c(leaf("number", "2")).b()').reduce();
assert.strictEqual(built.coparse(), "1 + 2");
assert.strictEqual(q.tb("_").w("(").c(built).w(")").b().coparse(), "(1 + 2)");

// 5. A stage that is annotated TypeScript, not merely annotation-free
//    JavaScript: the types are stripped before evaluation.
assert.strictEqual(src("((x: number): number => x * 2)(21)").reduce(), 42);

// 6. Errors from the stage propagate rather than being swallowed.
assert.throws(() => src("throw new Error('boom')").reduce(), /boom/);
assert.throws(() => src("this is not javascript").reduce(), { name: "SyntaxError" });

// --- The expander path: source that is still Quilt ------------------------
//
// Skipped without $QUILT (and no `quilt` on PATH), so the runtime tests above
// still run in a checkout that has not built the binary.
const haveExpander = Boolean(process.env.QUILT);

if (!haveExpander) {
  console.log("quilt-wasm reduce test: runtime assertions passed (set $QUILT for the expander cases)");
} else {
  // 7. expand() turns Quilt source into plain TypeScript, banner stripped.
  const expanded = q.expand("const x = ts↖1 + 2↗;");
  assert.ok(!expanded.includes("DO NOT EDIT"), `banner not stripped:\n${expanded}`);
  assert.ok(expanded.includes('tb("binary_expression")'), `expanded:\n${expanded}`);

  // 8. reduce() on glyph-bearing source expands first, then runs. The value
  //    here is itself a term — a generated stage that quotes.
  const staged = src("ts↖1 + 2↗").reduce();
  assert.strictEqual(staged.coparse(), "1 + 2");

  // 9. The staging chain: a stage whose own quotes and lifts fire when it runs.
  //    This is `↑` inside a generated stage — the case that needs the expander,
  //    because plain evaluation cannot parse a glyph.
  const gen = src("(a) => ts↖(x) => ↙↑(a)↘ * x↗").reduce();
  assert.strictEqual(gen(7).coparse(), "(x) => 7 * x");
  assert.strictEqual(gen(7).reduce()(6), 42);

  console.log("quilt-wasm reduce test: all assertions passed");
}
