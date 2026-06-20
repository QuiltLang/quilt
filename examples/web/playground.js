// In-browser staged demo: a self-specializing live dashboard. The whole "code
// generates code generates HTML" pipeline runs client-side —
//
//   source ──(wasi-shim + quilt-expand.wasm)──▶ Stage 1: makeRenderer (TS)
//   makeRenderer(schema) ──(↓ reduce: re-expand + eval)──▶ Stage 2: start() (TS)
//   start(setHtml, read) ──(its own baked setInterval)───▶ Stage 3: HTML, looping
//
// `quilt-expand.wasm` is the Quilt parser+expander (wasm32-wasip1); the runtime
// is `quilt-wasm` (wasm32-unknown-unknown). The `↓` operator (reduce) is what
// runs a generated stage; the runtime has no reduce of its own (it would need to
// re-expand, and the expander is a separate WASI module), so quilt-rt.js adds it
// in JS — coparse → expand → eval — and we register the page's expander into it.
//
// Editing the source or pressing Reconfigure reruns Stage 1 once (the expensive,
// rare step). It generates a start() whose own loop — interval baked in — paints
// the HTML; this page only supplies the HTML sink and the readings feed.

import initRuntime, { setExpander, reduceTrace, clearReduceTrace } from "quilt";
import { WASI } from "./wasi-shim.js";

const $ = (id) => document.getElementById(id);
const enc = new TextEncoder();
const dec = new TextDecoder();

const CHAIN = ["ts", "html"]; // ground TypeScript; un-annotated quotes are HTML

let expanderModule; // compiled WebAssembly.Module for the expander
let demo; // the imported Stage-1 module (just makeRenderer now)
let fullSchema = []; // the schema parsed from the config panel
let schema = []; // the active layout (Reconfigure may use a subset of fullSchema)
let opts = {}; // the opts parsed from the config panel
let stopLoop = null; // interval id returned by the generated loop (to stop it)
let sim = {}; // simulated live readings, per metric key
let frames = 0;

// ── Syntax highlighting (zero deps) ───────────────────────────────────────────
// A small TypeScript-flavoured tokenizer that also colours the Quilt arrow
// glyphs (↖↗ quote, ↙↘ unquote, ↑ lift, ↓ reduce, ← emit). Colours come from
// theme.css, matching the site's `.token.quilt-*` palette.
const KEYWORDS = new Set(
  ("import from export default as const let var function return if else for while do switch " +
   "case break continue new class extends interface type enum implements public private " +
   "protected readonly static async await yield typeof instanceof in of void delete this " +
   "super try catch finally throw true false null undefined").split(" "),
);
const TYPES = new Set(
  ("string number boolean any unknown never object symbol bigint Array Promise Record Map " +
   "Set Readonly Partial Math").split(" "),
);
const GLYPH_CLASS = {
  "↖": "glyph-quote", "↗": "glyph-quote", "↙": "glyph-unquote", "↘": "glyph-unquote",
  "↑": "glyph-lift", "↓": "glyph-reduce", "←": "glyph-emit",
};
const escHtml = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

function highlight(src) {
  const re =
    /(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?|`(?:[^`\\]|\\.)*`?)|([↖↗↙↘↑↓←])|(\d[\d_]*(?:\.\d+)?)|([A-Za-z_$][\w$]*)/g;
  let out = "", last = 0, m;
  while ((m = re.exec(src)) !== null) {
    out += escHtml(src.slice(last, m.index));
    if (m[1]) out += `<span class="tok-comment">${escHtml(m[1])}</span>`;
    else if (m[2]) out += `<span class="tok-string">${escHtml(m[2])}</span>`;
    else if (m[3]) out += `<span class="${GLYPH_CLASS[m[3]]}">${m[3]}</span>`;
    else if (m[4]) out += `<span class="tok-number">${m[4]}</span>`;
    else {
      const w = m[5];
      const cls = KEYWORDS.has(w) ? "tok-keyword" : (TYPES.has(w) || /^[A-Z]/.test(w)) ? "tok-type" : null;
      out += cls ? `<span class="${cls}">${escHtml(w)}</span>` : escHtml(w);
    }
    last = re.lastIndex;
  }
  return out + escHtml(src.slice(last));
}

// ── Editor overlay (highlighted <pre> kept in sync with the <textarea>) ───────
const src = $("src");
const srcHl = $("src-hl");

function refreshSource() {
  srcHl.innerHTML = highlight(src.value) + "\n";
  srcHl.scrollTop = src.scrollTop;
  srcHl.scrollLeft = src.scrollLeft;
}

function insert(open, close = "") {
  src.focus();
  const { selectionStart: a, selectionEnd: b, value } = src;
  const sel = value.slice(a, b);
  src.value = value.slice(0, a) + open + sel + close + value.slice(b);
  src.selectionStart = src.selectionEnd = sel ? a + open.length + sel.length + close.length : a + open.length;
  refreshSource();
}

function setStatus(msg, isError = false) {
  const el = $("status");
  el.textContent = msg;
  el.className = "status" + (isError ? " err" : "");
}

// Run the expander wasm once: stdin = source, argv = chain, returns stdout.
function expand(source) {
  const wasi = new WASI({ args: ["quilt-expand", ...CHAIN], stdin: enc.encode(source) });
  const instance = new WebAssembly.Instance(expanderModule, { wasi_snapshot_preview1: wasi.wasiImport });
  const code = wasi.start(instance);
  if (code !== 0) throw new Error(dec.decode(wasi.stderrBytes) || `expander exited ${code}`);
  return dec.decode(wasi.stdoutBytes);
}

// Import expanded Stage-1 TypeScript as a module. Its bare `quilt` import
// resolves through the page import map to quilt-rt.js (runtime + reduce).
async function importModule(tsSource) {
  const url = URL.createObjectURL(new Blob([tsSource], { type: "text/javascript" }));
  try {
    return await import(url);
  } finally {
    URL.revokeObjectURL(url);
  }
}

function previewDoc(fragment) {
  return `<!DOCTYPE html><html><head><meta charset="utf-8">` +
    `<link rel="stylesheet" href="./theme.css"></head><body class="preview">${fragment}</body></html>`;
}

// A gentle random walk so the bars move like real telemetry.
function step() {
  for (const m of schema) {
    const v = sim[m.key] ?? m.max * 0.4;
    const next = v + (Math.random() - 0.5) * m.max * 0.35;
    sim[m.key] = Math.max(0, Math.min(m.max, Math.round(next * 10) / 10));
  }
}

// The generated loop calls read() each frame for fresh readings, and setHtml()
// with the HTML it built. The loop and its interval live in the generated code
// now, not here.
function read() {
  step();
  return sim;
}
function setHtml(html) {
  $("preview").srcdoc = previewDoc(html);
  frames++;
  $("tick-info").textContent = `frame #${frames} · the loop is codegened`;
}

// Parse the config panel (raw JSON: { schema, opts }) into values.
function parseConfig() {
  const { schema: s, opts: o } = JSON.parse($("config").value);
  if (!Array.isArray(s) || !s.length) throw new Error("`schema` must be a non-empty array");
  if (!o || typeof o !== "object") throw new Error("`opts` must be an object");
  return { schema: s, opts: o };
}

// Read the config panel and (re)stage. Called on load, on config edits, and
// after the source is re-expanded.
function loadConfig() {
  if (!demo) return;
  let parsed;
  try {
    parsed = parseConfig();
    $("config").parentElement.classList.remove("err");
  } catch (e) {
    $("config").parentElement.classList.add("err");
    setStatus("config JSON: " + (e.message || e), true);
    return;
  }
  fullSchema = parsed.schema;
  opts = parsed.opts;
  schema = fullSchema;
  sim = {};
  restage();
}

// Stage 1 → Stage 2: the expensive step, run once. makeRenderer() unrolls the
// schema and reduces (↓) to a start() that contains its own update loop. Stop
// any previous loop, stage the new one, and let it drive the preview.
function restage() {
  if (stopLoop != null) { clearInterval(stopLoop); stopLoop = null; }
  clearReduceTrace();
  const t0 = performance.now();
  const start = demo.makeRenderer(schema, opts);
  const ms = performance.now() - t0;
  const stage2 = reduceTrace.length ? reduceTrace[reduceTrace.length - 1].generated : "(no reduce ran)";
  $("stage2").innerHTML = highlight(stage2);
  $("restage-info").textContent =
    `staged ${schema.length} gauge(s) in ${ms.toFixed(1)} ms · loop @ ${opts.intervalMs} ms baked in`;
  frames = 0;
  stopLoop = start(setHtml, read); // the GENERATED loop now drives updates
  setStatus(`live — the generated loop updates every ${opts.intervalMs} ms.`);
}

// Reconfigure = a user interaction that restages with a shuffled subset of the
// metrics from the config panel.
function reconfigure() {
  if (!demo || !fullSchema.length) return;
  const shuffled = [...fullSchema].sort(() => Math.random() - 0.5);
  const n = 2 + Math.floor(Math.random() * Math.max(1, fullSchema.length - 1));
  schema = shuffled.slice(0, n);
  sim = {};
  restage();
}

async function expandAndRun() {
  $("run").disabled = true;
  if (stopLoop != null) { clearInterval(stopLoop); stopLoop = null; }
  try {
    setStatus("expanding Stage 1…");
    const ts = expand(src.value);
    $("expanded").innerHTML = highlight(ts);
    setStatus("staging…");
    demo = await importModule(ts);
    if (typeof demo.makeRenderer !== "function") throw new Error("source must export makeRenderer()");
    loadConfig(); // read the config panel and stage
  } catch (e) {
    setStatus(String(e.message || e), true);
  } finally {
    $("run").disabled = false;
  }
}

// ── Arrow-glyph buttons + keyboard chords (same scheme as the VS Code ext) ────
const DIR = { ArrowLeft: "L", KeyH: "L", ArrowRight: "R", KeyL: "R", ArrowUp: "U", KeyK: "U", ArrowDown: "D", KeyJ: "D" };
const SINGLE = { L: "←", R: "→", U: "↑", D: "↓", Comma: "⟨", Period: "⟩", KeyT: "⟨T⟩", KeyN: "⟨N⟩" };
const DIAG = {
  UL: "↖", LU: "↖", UR: "↗", RU: "↗", DL: "↙", LD: "↙", DR: "↘", RD: "↘",
  LR: "↔", RL: "↔", UD: "↕", DU: "↕", UU: "↑", DD: "↓", LL: "←", RR: "→",
};

let chord = null, chordTimer = null;
function resetChord() { chord = null; clearTimeout(chordTimer); }
function armChord(c) { chord = c; clearTimeout(chordTimer); chordTimer = setTimeout(resetChord, 1500); }

function onKey(ev) {
  if ((ev.metaKey || ev.ctrlKey) && ev.key === "Enter") { ev.preventDefault(); resetChord(); expandAndRun(); return; }
  if ((ev.metaKey || ev.ctrlKey) && ev.code === "Digit1") { ev.preventDefault(); armChord("1"); return; }
  if ((ev.metaKey || ev.ctrlKey) && ev.code === "Digit2") { ev.preventDefault(); armChord("2"); return; }
  if (chord === "1") {
    const g = SINGLE[DIR[ev.code] || ev.code];
    if (g) { ev.preventDefault(); insert(g); }
    resetChord();
    return;
  }
  if (chord === "2") {
    const d = DIR[ev.code];
    if (d) { ev.preventDefault(); armChord("2:" + d); } else resetChord();
    return;
  }
  if (chord?.startsWith("2:")) {
    const d2 = DIR[ev.code];
    if (d2 && DIAG[chord.slice(2) + d2]) { ev.preventDefault(); insert(DIAG[chord.slice(2) + d2]); }
    resetChord();
    return;
  }
  if (ev.key === "Tab" && !ev.shiftKey && !ev.metaKey && !ev.ctrlKey && !ev.altKey) {
    ev.preventDefault(); insert("  ");
  }
}

function setupGlyphs() {
  $("glyphs").addEventListener("click", (ev) => {
    const btn = ev.target.closest("button");
    if (!btn) return;
    if (btn.dataset.wrap) { const [o, c] = [...btn.dataset.wrap]; insert(o, c); }
    else if (btn.dataset.ins) insert(btn.dataset.ins);
  });
  src.addEventListener("keydown", onKey);
}

// Regenerate the TypeScript a short while after config edits settle.
// Keep the config overlay highlighter in sync with its textarea (same trick as
// the source editor): a coloured <pre> behind a transparent <textarea>.
function refreshConfig() {
  const c = $("config");
  $("config-hl").innerHTML = highlight(c.value) + "\n";
  $("config-hl").scrollTop = c.scrollTop;
  $("config-hl").scrollLeft = c.scrollLeft;
}
function setupConfig() {
  const c = $("config");
  let cfgTimer = null;
  c.addEventListener("input", () => {
    refreshConfig();
    clearTimeout(cfgTimer);
    setStatus("editing config…");
    cfgTimer = setTimeout(loadConfig, 500);
  });
  c.addEventListener("scroll", refreshConfig);
}

async function main() {
  const [source, , expanderBytes] = await Promise.all([
    fetch("./dashboard.html.ts.ts.quilt").then((r) => r.text()),
    initRuntime(),
    fetch("./quilt-expand.wasm").then((r) => r.arrayBuffer()),
  ]);
  src.value = source;
  refreshSource();
  refreshConfig();
  expanderModule = await WebAssembly.compile(expanderBytes);
  setExpander(expand); // so `term.reduce()` (↓) can re-expand generated stages

  src.addEventListener("input", refreshSource);
  src.addEventListener("scroll", refreshSource);
  setupGlyphs();
  setupConfig();

  $("run").disabled = false;
  $("run").addEventListener("click", expandAndRun);
  $("reconfigure").addEventListener("click", reconfigure);
  setStatus("ready — press Expand & run.");
  expandAndRun();
}

main().catch((e) => setStatus(String(e.message || e), true));
