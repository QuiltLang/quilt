// Headless verify for the staged dashboard playground: runs the whole pipeline
// the browser runs, but in Node —
//
//   source --(wasi-shim + quilt-expand.wasm)--> Stage 1 (makeRenderer)
//   makeRenderer(schema) --(↓ reduce: re-expand + eval)--> Stage 2 (render)
//   render(values) --------------------------------------> Stage 3 (HTML)
//
// Builds dist/ first. Skips (exit 0) if no WASI sdk produced the expander.
//
//   node examples/web/verify-dashboard.mjs
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdirSync, symlinkSync, copyFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";
import assert from "node:assert";
import { WASI } from "./wasi-shim.js";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "dist");

execSync(`node ${join(here, "build.mjs")}`, { stdio: "inherit" });
if (!existsSync(join(dist, "quilt-expand.wasm"))) {
  console.warn("playground not built (no WASI sdk) — skipping verify");
  process.exit(0);
}

// Resolve the bare specifiers the same way the page import map does: `quilt` →
// the reduce-enabled wrapper, `quilt-wasm` → the runtime. In Node that means
// node_modules entries; the wrapper file is copied (not symlinked) so its bare
// `quilt-wasm` import resolves from inside node_modules.
const nm = join(dist, "node_modules");
mkdirSync(join(nm, "quilt"), { recursive: true });
if (!existsSync(join(nm, "quilt-wasm"))) symlinkSync(join("..", "quilt-wasm"), join(nm, "quilt-wasm"));
copyFileSync(join(dist, "quilt-rt.js"), join(nm, "quilt", "index.js"));
writeFileSync(join(nm, "quilt", "package.json"), '{"name":"quilt","type":"module","main":"index.js"}\n');

const enc = new TextEncoder(), dec = new TextDecoder();
const expander = await WebAssembly.compile(readFileSync(join(dist, "quilt-expand.wasm")));
function expand(source, chain = ["ts", "html"]) {
  const wasi = new WASI({ args: ["quilt-expand", ...chain], stdin: enc.encode(source) });
  const code = wasi.start(new WebAssembly.Instance(expander, { wasi_snapshot_preview1: wasi.wasiImport }));
  assert.strictEqual(code, 0, dec.decode(wasi.stderrBytes) || "expander failed");
  return dec.decode(wasi.stdoutBytes);
}

// 1. Initialise the runtime + register the expander, on the shared wrapper.
const quilt = await import(pathToFileURL(join(nm, "quilt", "index.js")));
await quilt.default({ module_or_path: readFileSync(join(dist, "quilt-wasm", "quilt_wasm_bg.wasm")) });
quilt.setExpander((s) => expand(s));

// 2. Expand Stage 1 (makeRenderer) and import it.
const ts = expand(readFileSync(join(dist, "dashboard.html.ts.quilt"), "utf8"));
assert(ts.includes("function makeRenderer") && ts.includes(".reduce()"), "Stage 1 exports makeRenderer + uses ↓");
const modPath = join(dist, "__dash.mjs");
writeFileSync(modPath, ts);
const mod = await import(pathToFileURL(modPath));

// 3. Stage 1 → Stage 2: makeRenderer reduces (↓) to a render function.
quilt.clearReduceTrace();
const render = mod.makeRenderer(mod.schema, mod.opts);
assert.strictEqual(typeof render, "function", "makeRenderer returns render()");
const stage2 = quilt.reduceTrace.at(-1).generated;
console.log("=== Stage 2: the render() Stage 1 generated ===\n" + stage2 + "\n");
// It must be unrolled: one gauge statement per metric, no loop left.
assert(!/\bfor\b/.test(stage2), "Stage 2 is unrolled (no for-loop)");
assert((stage2.match(/class="bar"/g) || []).length === mod.schema.length, "one gauge per metric");

// 4. Stage 3: render fills live readings. No expansion happens here.
const before = quilt.reduceTrace.length;
const html = render({ cpu: 37, mem: 64, net: 12.5, disk: 8 }).coparse();
console.log("=== Stage 3: render(values) → HTML ===\n" + html + "\n");
assert.strictEqual(quilt.reduceTrace.length, before, "rendering a tick triggers no expansion");
assert(html.includes("<h1>") && html.includes(mod.opts.title), "title baked in");
assert(html.includes("█"), "ascii meter bars rendered");
assert(html.includes("37") && html.includes("64"), "live values plugged in");
assert(!html.includes("↙") && !html.includes("↑"), "no unexpanded glyphs leaked");

console.log("staged dashboard verify: source → expand → ↓ reduce → render → HTML ✓");
