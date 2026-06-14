// In-browser meta-meta demo (issue #47): expand `.html.ts.quilt` source live,
// then run the expansion — all client-side.
//
//   source --(WASI shim + quilt-expand.wasm)--> TypeScript --(import + runtime)--> HTML
//
// `quilt-expand.wasm` is the Quilt parser+expander (wasm32-wasip1); the runtime
// is the same `quilt-wasm` (wasm32-unknown-unknown) used by the ahead-of-time
// demo. Both are WebAssembly; only the expander needs WASI (it links the C
// grammars), so it runs through the small hand-rolled shim in wasi-shim.js.
//
// The editor is a zero-dependency syntax highlighter: a coloured <pre> sits
// behind a transparent <textarea>, both sharing the same box metrics, so you
// type into the textarea (caret only) while the pre shows the colours. The same
// tokenizer colours the read-only expanded TypeScript.

import initRuntime from "quilt";
import { WASI } from "./wasi-shim.js";

const $ = (id) => document.getElementById(id);
const enc = new TextEncoder();
const dec = new TextDecoder();

const CHAIN = ["ts", "html"]; // .html.ts: ground TypeScript, quotes default to HTML

let expanderModule; // compiled WebAssembly.Module for the expander

// ── Syntax highlighting (zero deps) ───────────────────────────────────────────
// A small TypeScript-flavoured tokenizer that also colours the Quilt arrow
// glyphs (↖↗ quote, ↙↘ unquote, ↑ lift, ↓ reduce, ← emit). The colours come
// from theme.css, matching the site's `.token.quilt-*` palette.

const KEYWORDS = new Set(
  ("import from export default as const let var function return if else for while do switch " +
   "case break continue new class extends interface type enum implements public private " +
   "protected readonly static async await yield typeof instanceof in of void delete this " +
   "super try catch finally throw true false null undefined").split(" "),
);
const TYPES = new Set(
  ("string number boolean any unknown never object symbol bigint Array Promise Record Map " +
   "Set Readonly Partial").split(" "),
);
const GLYPH_CLASS = {
  "↖": "glyph-quote", "↗": "glyph-quote", "↙": "glyph-unquote", "↘": "glyph-unquote",
  "↑": "glyph-lift", "↓": "glyph-reduce", "←": "glyph-emit",
};
const escHtml = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

function highlight(src) {
  // Order matters: comments, then (possibly unterminated) strings, glyphs,
  // numbers, identifiers. Everything else passes through escaped.
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
  // Trailing newline keeps the pre as tall as the textarea's last (empty) line.
  srcHl.innerHTML = highlight(src.value) + "\n";
  srcHl.scrollTop = src.scrollTop;
  srcHl.scrollLeft = src.scrollLeft;
}

// Insert text at the caret (wrapping the selection if `close` is given), then
// re-highlight. Used by both the glyph buttons and their keyboard shortcuts.
function insert(open, close = "") {
  src.focus();
  const { selectionStart: a, selectionEnd: b, value } = src;
  const sel = value.slice(a, b);
  src.value = value.slice(0, a) + open + sel + close + value.slice(b);
  // No selection → caret lands just after `open` (between a wrap's glyphs);
  // with a selection → caret lands after the whole inserted run.
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
  const instance = new WebAssembly.Instance(expanderModule, {
    wasi_snapshot_preview1: wasi.wasiImport,
  });
  const code = wasi.start(instance);
  if (code !== 0) {
    throw new Error(dec.decode(wasi.stderrBytes) || `expander exited ${code}`);
  }
  return dec.decode(wasi.stdoutBytes);
}

// Import the expanded TypeScript as a module, call its render() to get a Quilt
// term, then coparse() it here (the harness) into an HTML string. The blob
// module's bare `quilt` import resolves through the page import map to the
// already-initialised runtime, so it shares the same wasm instance.
async function run(tsSource) {
  const url = URL.createObjectURL(new Blob([tsSource], { type: "text/javascript" }));
  try {
    const mod = await import(url);
    if (typeof mod.render !== "function") {
      throw new Error("expanded program does not export render()");
    }
    return mod.render().coparse();
  } finally {
    URL.revokeObjectURL(url);
  }
}

// Wrap a rendered HTML fragment in a minimal document that links the shared
// site theme by a relative href, so the preview is styled like the rest of the
// site without inlining any CSS here.
function previewDoc(fragment) {
  return `<!DOCTYPE html><html><head><meta charset="utf-8">` +
    `<link rel="stylesheet" href="./theme.css"></head><body>${fragment}</body></html>`;
}

async function expandAndRun() {
  $("run").disabled = true;
  try {
    setStatus("expanding…");
    const ts = expand(src.value);
    $("expanded").innerHTML = highlight(ts);
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

// ── Arrow-glyph buttons + keyboard chords ─────────────────────────────────────
// The arrow glyphs can't be typed on a normal keyboard. The buttons insert them
// (wrapping the selection for the bracket pairs), and the keyboard uses the same
// chord scheme as the VS Code extension (tools/quilt): leader ⌘/Ctrl+1 then a
// direction inserts a single glyph; leader ⌘/Ctrl+2 then two directions inserts
// the diagonal that combines them. Directions are the arrow keys or vim h/j/k/l.
const DIR = { ArrowLeft: "L", KeyH: "L", ArrowRight: "R", KeyL: "R", ArrowUp: "U", KeyK: "U", ArrowDown: "D", KeyJ: "D" };
const SINGLE = { L: "←", R: "→", U: "↑", D: "↓", Comma: "⟨", Period: "⟩", KeyT: "⟨T⟩", KeyN: "⟨N⟩" };
const DIAG = {
  UL: "↖", LU: "↖", UR: "↗", RU: "↗", DL: "↙", LD: "↙", DR: "↘", RD: "↘",
  LR: "↔", RL: "↔", UD: "↕", DU: "↕", UU: "↑", DD: "↓", LL: "←", RR: "→",
};

let chord = null, chordTimer = null; // null | "1" | "2" | "2:<dir>"
function resetChord() { chord = null; clearTimeout(chordTimer); }
function armChord(c) { chord = c; clearTimeout(chordTimer); chordTimer = setTimeout(resetChord, 1500); }

function onKey(ev) {
  if ((ev.metaKey || ev.ctrlKey) && ev.key === "Enter") { ev.preventDefault(); resetChord(); expandAndRun(); return; }
  // Leaders. (Mid-chord direction keys are accepted with or without the
  // modifier, so both `⌘1 ←` and `⌘1 ⌘←` work, like the extension.)
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
  // Load the default source, the runtime, and the expander in parallel.
  const [source, , expanderBytes] = await Promise.all([
    fetch("./cards.html.ts.quilt").then((r) => r.text()),
    initRuntime(),
    fetch("./quilt-expand.wasm").then((r) => r.arrayBuffer()),
  ]);
  src.value = source;
  refreshSource();
  expanderModule = await WebAssembly.compile(expanderBytes);

  src.addEventListener("input", refreshSource);
  src.addEventListener("scroll", refreshSource);
  setupGlyphs();

  $("run").disabled = false;
  $("run").addEventListener("click", expandAndRun);
  setStatus("ready — press Expand & run.");
  expandAndRun(); // show output immediately
}

main().catch((e) => setStatus(String(e.message || e), true));
