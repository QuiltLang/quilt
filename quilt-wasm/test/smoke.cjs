// Smoke test for the quilt-wasm runtime: build QTerms the way expanded
// `.ts.quilt` code does (tuples + qlift/qlift_html) and check `coparse` output.
//
// Run after `wasm-pack build --target nodejs`:
//   node quilt-wasm/test/smoke.cjs

const assert = require("node:assert");
const q = require("../pkg/quilt_wasm.js");

// 1. A leaf reconstructs to its code verbatim.
assert.strictEqual(q.leaf("text", "Hello").coparse(), "Hello");

// 2. A tuple interleaves writes and spliced children — an HTML <li>.
const li = q.tb("element").w("<li>").c(q.leaf("text", "Hello")).w("</li>").b();
assert.strictEqual(li.coparse(), "<li>Hello</li>");

// 3. qlift_html entity-escapes strings so spliced values are inert HTML.
assert.strictEqual(
  q.qlift_html('a <b> & "c"').coparse(),
  "a &lt;b&gt; &amp; &quot;c&quot;",
);
// numbers and booleans lift to plain text.
assert.strictEqual(q.qlift_html(42).coparse(), "42");
assert.strictEqual(q.qlift_html(true).coparse(), "true");

// 4. homogeneous qlift produces TypeScript literals.
assert.strictEqual(q.qlift(42).coparse(), "42");
assert.strictEqual(q.qlift(3.5).coparse(), "3.5");
assert.strictEqual(q.qlift("hi").coparse(), '"hi"');
assert.strictEqual(q.qlift('say "hi"').coparse(), '"say \\"hi\\""');
assert.strictEqual(q.qlift(false).coparse(), "false");

// 5. A nested fragment built by splicing a lifted value into a tuple —
//    exactly the shape `html↖<li>↙↑(title)↘</li>↗` expands to.
const item = q
  .tb("element")
  .w("<li>")
  .c(q.qlift_html("Fix <bug> #1"))
  .w("</li>")
  .b();
assert.strictEqual(item.coparse(), "<li>Fix &lt;bug&gt; #1</li>");

// 6. quote/unquote + cmd/HOLE round-trip (the meta-reconstruction path used at
//    higher quasi-quote levels). We only assert it builds and the spliced child
//    text survives in the serialized output.
const quoted = q.quote("element", 0, "html", q.leaf("text", "x"), [
  q.cmd(q.write("<li>")),
  q.HOLE(),
  q.cmd(q.write("</li>")),
]);
const out = quoted.coparse();
assert.ok(typeof out === "string" && out.includes("x"), `quote coparse: ${out}`);

console.log("quilt-wasm smoke test: all assertions passed");
