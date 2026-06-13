// Headless smoke test for the browser demo. Builds dist/, then loads the
// runtime + the expanded program the way index.html does — except it runs in
// Node and initialises the WebAssembly from the bundled bytes instead of a
// `fetch`. Asserts that render() produces the expected, entity-escaped HTML.
//
//   node examples/web/verify.mjs
//
// Node resolves module identity by realpath, so app.js's bare `quilt`
// import (via the dist/node_modules symlink) is the *same* module instance we
// initialise here — render() sees an initialised runtime.
import { execSync } from "node:child_process";
import { readFileSync, existsSync, mkdirSync, symlinkSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";
import assert from "node:assert";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "dist");

// 1. Build the static demo.
execSync(`node ${join(here, "build.mjs")}`, { stdio: "inherit" });

// 2. Make the bare `quilt` specifier resolve for app.js (import-map
//    equivalent), pointing at the same files the page uses. The dist runtime
//    dir stays `quilt-wasm`; only the node_modules entry is named `quilt`.
const nm = join(dist, "node_modules");
if (!existsSync(join(nm, "quilt"))) {
  mkdirSync(nm, { recursive: true });
  symlinkSync(join("..", "quilt-wasm"), join(nm, "quilt"));
}

// 3. Initialise the WebAssembly from bytes, then run the program's render().
const glue = pathToFileURL(join(dist, "quilt-wasm", "quilt_wasm.js"));
const { default: init } = await import(glue);
await init({ module_or_path: readFileSync(join(dist, "quilt-wasm", "quilt_wasm_bg.wasm")) });

const { render } = await import(pathToFileURL(join(dist, "app.js")));
const html = render();

console.log(html);
assert(html.includes('<section class="cards">'), "section wrapper");
assert(html.includes("<h2>Hello from TypeScript</h2>"), "first card title");
// The lifted value with `<script>` and quotes must be entity-escaped.
assert(html.includes("&lt;script&gt; &amp; &quot;quotes&quot;"), "escaped lift");
assert(!html.includes("<script>"), "no raw script tag leaked");
console.log("\nexamples/web verify: all assertions passed");
