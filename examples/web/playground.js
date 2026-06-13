// In-browser meta-meta demo (issue #47): expand `.html.ts.quilt` source live,
// then run the expansion — all client-side.
//
//   source --(WASI shim + quilt-expand.wasm)--> TypeScript --(import + runtime)--> HTML
//
// `quilt-expand.wasm` is the Quilt parser+expander (wasm32-wasip1); the runtime
// is the same `quilt-wasm` (wasm32-unknown-unknown) used by the ahead-of-time
// demo. Both are WebAssembly; only the expander needs WASI (it links the C
// grammars), so it runs through the small hand-rolled shim in wasi-shim.js.

import initRuntime from "quilt";
import { WASI } from "./wasi-shim.js";

const $ = (id) => document.getElementById(id);
const enc = new TextEncoder();
const dec = new TextDecoder();

const CHAIN = ["ts", "html"]; // .html.ts: ground TypeScript, quotes default to HTML

let expanderModule; // compiled WebAssembly.Module for the expander

function setStatus(msg, isError = false) {
  const el = $("status");
  el.textContent = msg;
  el.className = "status" + (isError ? " err" : "");
}

// Run the expander wasm once: stdin = source, argv = chain, returns stdout.
function expand(source) {
  const wasi = new WASI({ args: ["quilt-expand", ...CHAIN], stdin: enc.encode(source) });
  const instance = new WebAssembly.Instance(expanderModule, {
    wasi_snapshot_preview1: wasi.wasiImport,
  });
  const code = wasi.start(instance);
  if (code !== 0) {
    throw new Error(dec.decode(wasi.stderrBytes) || `expander exited ${code}`);
  }
  return dec.decode(wasi.stdoutBytes);
}

// Import the expanded TypeScript as a module and call its render(). The blob
// module's bare `quilt` import resolves through the page import map to the
// already-initialised runtime, so it shares the same wasm instance.
async function run(tsSource) {
  const url = URL.createObjectURL(new Blob([tsSource], { type: "text/javascript" }));
  try {
    const mod = await import(url);
    if (typeof mod.render !== "function") {
      throw new Error("expanded program does not export render()");
    }
    return mod.render();
  } finally {
    URL.revokeObjectURL(url);
  }
}

// Wrap a rendered HTML fragment in a minimal document so the preview iframe
// styles the cards like the ahead-of-time demo.
function previewDoc(fragment) {
  return `<!DOCTYPE html><meta charset="utf-8"><style>
    body { font-family: system-ui, sans-serif; margin: 1rem; color: #222; }
    .cards { display: grid; gap: 1rem; }
    .card { border: 1px solid #ddd; border-radius: 10px; padding: 1rem 1.25rem; }
    .card h2 { font-size: 1.05rem; margin: 0 0 .4rem; }
    .card p { margin: 0; color: #444; }
  </style>${fragment}`;
}

async function expandAndRun() {
  $("run").disabled = true;
  try {
    setStatus("expanding…");
    const ts = expand($("src").value);
    $("expanded").textContent = ts;
    setStatus("running…");
    const html = await run(ts);
    $("preview").srcdoc = previewDoc(html);
    setStatus("done.");
  } catch (e) {
    setStatus(String(e.message || e), true);
  } finally {
    $("run").disabled = false;
  }
}

async function main() {
  // Load the default source, the runtime, and the expander in parallel.
  const [src, , expanderBytes] = await Promise.all([
    fetch("./cards.html.ts.quilt").then((r) => r.text()),
    initRuntime(),
    fetch("./quilt-expand.wasm").then((r) => r.arrayBuffer()),
  ]);
  $("src").value = src;
  expanderModule = await WebAssembly.compile(expanderBytes);

  $("run").disabled = false;
  $("run").addEventListener("click", expandAndRun);
  setStatus("ready — press Expand & run.");
  expandAndRun(); // show output immediately
}

main().catch((e) => setStatus(String(e.message || e), true));
