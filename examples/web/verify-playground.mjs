// Headless smoke test for the meta-meta playground (#47): runs the *whole*
// pipeline the browser runs, but in Node —
//
//   source --(wasi-shim + quilt-expand.wasm)--> TS --(import + runtime)--> HTML
//
// Builds dist/ first. Skips (exit 0) if no WASI sdk produced the expander.
//
//   node examples/web/verify-playground.mjs
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdirSync, symlinkSync } from "node:fs";
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

// Resolve the bare `quilt-wasm` specifier for the expanded module (import-map
// equivalent), pointing at the same runtime files we initialise here.
const nm = join(dist, "node_modules");
if (!existsSync(join(nm, "quilt-wasm"))) {
  mkdirSync(nm, { recursive: true });
  symlinkSync(join("..", "quilt-wasm"), join(nm, "quilt-wasm"));
}

// 1. Expand the source through the wasm expander + WASI shim.
const source = readFileSync(join(dist, "cards.html.ts.quilt"));
const wasi = new WASI({ args: ["quilt-expand", "ts", "html"], stdin: new Uint8Array(source) });
const expander = await WebAssembly.compile(readFileSync(join(dist, "quilt-expand.wasm")));
const code = wasi.start(new WebAssembly.Instance(expander, { wasi_snapshot_preview1: wasi.wasiImport }));
const ts = new TextDecoder().decode(wasi.stdoutBytes);
assert.strictEqual(code, 0, new TextDecoder().decode(wasi.stderrBytes) || "expander failed");
assert(ts.includes('tb("element")') && ts.includes("qlift_html("), "expanded to runtime calls");

// 2. Initialise the runtime, then import the expansion and run render().
const { default: init } = await import(pathToFileURL(join(dist, "quilt-wasm", "quilt_wasm.js")));
await init({ module_or_path: readFileSync(join(dist, "quilt-wasm", "quilt_wasm_bg.wasm")) });

// The expanded module imports "quilt-wasm" (bare) → dist/node_modules → same
// runtime instance. Write it inside dist so that resolution works.
const modPath = join(dist, "__expanded.mjs");
writeFileSync(modPath, ts);
const { render } = await import(pathToFileURL(modPath));
const html = render();

console.log(html);
assert(html.includes('<section class="cards">'), "section wrapper");
assert(html.includes("&lt;script&gt; &amp; &quot;quotes&quot;"), "escaped lift");
assert(!html.includes("<script>"), "no raw script leaked");
console.log("\nmeta-meta playground verify: source → wasm-expand → run → HTML ✓");
