// Assemble the static browser demo into examples/web/dist/.
//
// Requires `wasm-pack` and `node` (the Quilt dev env has both). No bundler:
// the page loads native ES modules and resolves the runtime via an import map.
//
//   node examples/web/build.mjs
//   python3 -m http.server -d examples/web/dist 8000   # then open localhost:8000
import { execSync } from "node:child_process";
import { cpSync, mkdirSync, copyFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..");
const dist = join(here, "dist");

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });

// 1. Build the quilt-wasm runtime for the browser (ESM + .wasm).
execSync("wasm-pack build quilt-wasm --target web --out-dir pkg-web", {
  cwd: repo,
  stdio: "inherit",
});
cpSync(join(repo, "quilt-wasm", "pkg-web"), join(dist, "quilt-wasm"), { recursive: true });

// 2. The expanded TypeScript program is annotation-free, so it is already valid
//    JavaScript — copy it in as the app module. (Regenerate it from the .quilt
//    source with `quilt expand examples/web/cards.html.ts.quilt`.)
copyFileSync(join(here, "cards.html.ts"), join(dist, "app.js"));

// 3. The page shell.
copyFileSync(join(here, "index.html"), join(dist, "index.html"));

console.log("built examples/web/dist — serve it, e.g. `python3 -m http.server -d examples/web/dist 8000`");
