// Assemble the static browser demos into examples/web/dist/:
//   /index.html       — ahead-of-time demo (#46): TS→HTML, expanded offline
//   /playground.html  — meta-meta demo (#47): expansion runs in the browser too
//
// Requires `wasm-pack` and `node`. The playground also needs the expander wasm
// (`bin/build-expand-wasm`, which needs a WASI sdk — see that script); if no
// WASI sdk is found it is skipped with a warning and only the #46 demo builds.
// No bundler: the pages load native ES modules and resolve the runtime via an
// import map.
//
//   node examples/web/build.mjs
//   python3 -m http.server -d examples/web/dist 8000   # open localhost:8000
import { execSync } from "node:child_process";
import { cpSync, mkdirSync, copyFileSync, rmSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..");
const dist = join(here, "dist");
const copyInto = (name) => copyFileSync(join(here, name), join(dist, name));

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });

// 1. The quilt-wasm runtime for the browser (ESM + .wasm), shared by both demos.
execSync("wasm-pack build quilt-wasm --target web --out-dir pkg-web", {
  cwd: repo,
  stdio: "inherit",
});
cpSync(join(repo, "quilt-wasm", "pkg-web"), join(dist, "quilt-wasm"), { recursive: true });

// 2. Ahead-of-time demo (#46). The expanded program is annotation-free, so it
//    is valid JS — copy it in as the app module. (Regenerate from the .quilt
//    source with `quilt expand examples/web/cards.html.ts.quilt`.)
copyFileSync(join(here, "cards.html.ts"), join(dist, "app.js"));
copyInto("index.html");

// 3. Meta-meta playground (#47): the expander wasm + the page that drives it.
const WASI_SDK = process.env.WASI_SDK_PATH || join(process.env.HOME || "", "wasi-sdk-33.0-arm64-macos");
if (existsSync(join(WASI_SDK, "bin", "clang"))) {
  execSync("bin/build-expand-wasm", { cwd: repo, stdio: "inherit", env: { ...process.env, WASI_SDK_PATH: WASI_SDK } });
  copyFileSync(
    join(repo, "target", "wasm32-wasip1", "release", "quilt-expand-wasm.wasm"),
    join(dist, "quilt-expand.wasm"),
  );
  copyInto("playground.html");
  copyInto("playground.js");
  copyInto("wasi-shim.js");
  copyInto("cards.html.ts.quilt"); // playground's initial source
  console.log("included the meta-meta playground (/playground.html)");
} else {
  console.warn(`! no WASI sdk at ${WASI_SDK} — skipping the playground (#47); set WASI_SDK_PATH to include it`);
}

console.log("built examples/web/dist — serve it, e.g. `python3 -m http.server -d examples/web/dist 8000`");
