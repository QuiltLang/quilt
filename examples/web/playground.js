// In-browser staged demo: a self-specializing live dashboard. The whole "code
// generates code generates HTML" pipeline runs client-side —
//
//   source ──(wasi-shim + quilt-expand.wasm)──▶ Stage 1: makeRenderer (TS)
//   makeRenderer(schema) ──(↓ reduce: re-expand + eval)──▶ Stage 2: render (TS)
//   render(values) ──(called every second)──────────────▶ Stage 3: HTML
//
// `quilt-expand.wasm` is the Quilt parser+expander (wasm32-wasip1); the runtime
// is `quilt-wasm` (wasm32-unknown-unknown). The `↓` operator (reduce) is what
// runs a generated stage; the runtime has no reduce of its own (it would need to
// re-expand, and the expander is a separate WASI module), so quilt-rt.js adds it
// in JS — coparse → expand → eval — and we register the page's expander into it.
//
// Editing the source or pressing Reconfigure reruns Stage 1 (the expensive,
// rare step: two passes through the expander). The per-second tick only calls
// the already-built render() with fresh numbers — no expansion, no looping.

import initRuntime, { setExpander, reduceTrace, clearReduceTrace } from "quilt";
import { WASI } from "./wasi-shim.js";

const $ = (id) => document.getElementById(id);
const enc = new TextEncoder();
const dec = new TextDecoder();

const CHAIN = ["ts", "html"]; // ground TypeScript; un-annotated quotes are HTML

let expanderModule; // compiled WebAssembly.Module for the expander
let demo; // the imported Stage-1 module (makeRenderer, schema, opts)
let schema; // the active layout (Reconfigure mutates this)
let render; // the current Stage-3 render(values) → HTML term
let sim = {}; // simulated live readings, per metric key
let timer = null; // the once-a-second tick
let ticks = 0;

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

// Stage 3, once a second: just call render() with fresh readings. No expansion.
function tick() {
  step();
  const t0 = performance.now();
  const html = render(sim).coparse();
  const ms = performance.now() - t0;
  $("preview").srcdoc = previewDoc(html);
  ticks++;
  $("tick-info").textContent = `tick #${ticks} · render() ${ms.toFixed(2)} ms · no expansion`;
}

// Stage 1 → Stage 2: the expensive step. makeRenderer() unrolls the schema and
// reduces (↓) the result, so a pass runs through the wasm expander. We time it
// and show the render() it generated.
function restage() {
  clearReduceTrace();
  const t0 = performance.now();
  render = demo.makeRenderer(schema, demo.opts);
  const ms = performance.now() - t0;
  const stage2 = reduceTrace.length ? reduceTrace[reduceTrace.length - 1].generated : "(no reduce ran)";
  $("stage2").innerHTML = highlight(stage2);
  $("restage-info").textContent =
    `restaged ${schema.length} gauge(s) in ${ms.toFixed(1)} ms · ${reduceTrace.length} expansion(s)`;
  tick(); // paint immediately
}

// Reconfigure = a user interaction that triggers the expensive outer loop:
// shuffle and drop/add metrics, then rerun Stage 1.
function reconfigure() {
  const all = demo.schema;
  const shuffled = [...all].sort(() => Math.random() - 0.5);
  const n = 2 + Math.floor(Math.random() * (all.length - 1)); // keep 2..all
  schema = shuffled.slice(0, n);
  sim = {};
  restage();
}

async function expandAndRun() {
  $("run").disabled = true;
  if (timer) { clearInterval(timer); timer = null; }
  try {
    setStatus("expanding Stage 1…");
    const ts = expand(src.value);
    $("expanded").innerHTML = highlight(ts);
    setStatus("staging…");
    demo = await importModule(ts);
    if (typeof demo.makeRenderer !== "function") throw new Error("source must export makeRenderer()");
    schema = [...demo.schema];
    sim = {};
    ticks = 0;
    restage();
    timer = setInterval(tick, 1000);
    setStatus("live — bars update every second.");
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

async function main() {
  const [source, , expanderBytes] = await Promise.all([
    fetch("./dashboard.html.ts.quilt").then((r) => r.text()),
    initRuntime(),
    fetch("./quilt-expand.wasm").then((r) => r.arrayBuffer()),
  ]);
  src.value = source;
  refreshSource();
  expanderModule = await WebAssembly.compile(expanderBytes);
  setExpander(expand); // so `term.reduce()` (↓) can re-expand generated stages

  src.addEventListener("input", refreshSource);
  src.addEventListener("scroll", refreshSource);
  setupGlyphs();

  $("run").disabled = false;
  $("run").addEventListener("click", expandAndRun);
  $("reconfigure").addEventListener("click", reconfigure);
  setStatus("ready — press Expand & run.");
  expandAndRun();
}

main().catch((e) => setStatus(String(e.message || e), true));
