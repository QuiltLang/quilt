// The notebook editor: cells on the left, the live page on the right.
//
// Two things are worth knowing before reading on.
//
// 1. **The page frame is a machine.** `#page` holds the document the server's
//    HTML machine is feeding, and its realm is where TypeScript cells run —
//    so it is patched, never reloaded, for the life of the session. A cell's
//    `const` is still there for the next cell, its event handlers keep
//    firing, and its `document` is the page it is editing.
// 2. **The server's page is the truth.** Every change arrives as a
//    `{kind, id, html}` patch over `/events` and is applied by id. This
//    client never invents page content; even a cell it ran itself comes back
//    through the same events as a cell someone else ran.

const $ = (id) => document.getElementById(id);
const api = async (method, path, body) => {
  const res = await fetch(path, {
    method,
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let value;
  try {
    value = text ? JSON.parse(text) : {};
  } catch {
    value = { error: text };
  }
  if (!res.ok && !value.error) value.error = `${res.status} ${text}`;
  return value;
};

const state = { cells: [], typescript: "browser", runtime: false, dirty: new Set() };

/* ── the cell list ─────────────────────────────────────────────────────── */

// Re-render only what changed: a cell being typed in must not lose its
// caret because another cell finished running.
function renderCells() {
  const list = $("cells");
  const seen = new Set();
  for (const cell of state.cells) {
    seen.add(cell.id);
    let node = list.querySelector(`[data-id="${cell.id}"]`);
    if (!node) {
      node = $("cell").content.firstElementChild.cloneNode(true);
      node.dataset.id = cell.id;
      wire(node, cell.id);
      list.append(node);
    }
    paint(node, cell);
  }
  for (const node of [...list.children]) {
    if (!seen.has(Number(node.dataset.id))) node.remove();
  }
  // Page order is the server's order; the DOM follows it.
  for (const cell of state.cells) {
    list.append(list.querySelector(`[data-id="${cell.id}"]`));
  }
}

function paint(node, cell) {
  node.className =
    "cell" +
    (cell.failed ? " failed" : "") +
    (!cell.ran ? " pending" : "") +
    (cell.generation > 0 ? " generated" : "");
  node.querySelector(".id").textContent = `#${cell.id}`;
  const lang = node.querySelector(".lang");
  if (lang.value !== cell.lang) {
    if (![...lang.options].some((o) => o.value === cell.lang)) {
      lang.append(new Option(cell.lang, cell.lang));
    }
    lang.value = cell.lang;
  }
  const src = node.querySelector(".src");
  if (!state.dirty.has(cell.id) && src.value !== cell.src) {
    src.value = cell.src;
    grow(src);
  }
  const badges = node.querySelector(".badges");
  badges.replaceChildren();
  if (cell.generation > 0) badges.append(badge(`gen ${cell.generation}`));
  for (const id of cell.edits) badges.append(badge(`→ #${id}`));
  if (cell.spawned.length) badges.append(badge(`made ${cell.spawned.map((s) => "#" + s).join(" ")}`));
  paintOutput(node.querySelector(".out"), cell);
}

const badge = (text) => {
  const span = document.createElement("span");
  span.className = "badge";
  span.textContent = text;
  return span;
};

// The editor shows what the cell *said*; what it *drew* is in the page, which
// is the other half of the window.
function paintOutput(out, cell) {
  const parts = [];
  const line = (cls, text) => {
    if (!text || !text.trim()) return;
    const p = document.createElement("p");
    p.className = cls;
    p.textContent = text.replace(/\s+$/, "");
    parts.push(p);
  };
  line("", cell.stdout);
  if (cell.value && !["None", "undefined"].includes(cell.value.trim())) line("value", cell.value);
  line("warn", cell.stderr);
  line("err", cell.error);
  out.replaceChildren(...parts);
  out.hidden = parts.length === 0;
}

function grow(textarea) {
  textarea.rows = Math.min(24, Math.max(2, textarea.value.split("\n").length));
}

/* ── editing ───────────────────────────────────────────────────────────── */

function wire(node, id) {
  const src = node.querySelector(".src");
  src.addEventListener("input", () => {
    state.dirty.add(id);
    grow(src);
  });
  src.addEventListener("blur", () => save(id));
  src.addEventListener("keydown", (ev) => onKey(ev, id, src));
  node.querySelector(".lang").addEventListener("change", () => save(id));
  node.querySelector(".run").addEventListener("click", () => run(id));
  node.querySelector(".expand").addEventListener("click", () => expand(id, node));
  node.querySelector(".after").addEventListener("click", () => addCell(cellById(id)?.element));
  node.querySelector(".remove").addEventListener("click", async () => {
    if (!confirm(`Remove cell #${id}? Its machine keeps what it defined.`)) return;
    take(await api("DELETE", `/cells/${id}`));
  });
}

const cellById = (id) => state.cells.find((c) => c.id === id);
const nodeOf = (id) => $("cells").querySelector(`[data-id="${id}"]`);

async function save(id) {
  if (!state.dirty.has(id)) return;
  const node = nodeOf(id);
  if (!node) return;
  state.dirty.delete(id);
  take(
    await api("PUT", `/cells/${id}`, {
      lang: node.querySelector(".lang").value,
      src: node.querySelector(".src").value,
    }),
  );
}

async function addCell(after) {
  const lang = $("add-lang").value;
  const answer = await api("POST", "/cells", { lang, src: "", after: after ?? null });
  take(answer);
  if (answer.id) nodeOf(answer.id)?.querySelector(".src")?.focus();
}

/* ── running ───────────────────────────────────────────────────────────── */

async function run(id) {
  await save(id);
  const node = nodeOf(id);
  node?.classList.add("running");
  try {
    const answer = await api("POST", `/cells/${id}/run`, {});
    if (answer.error) return note(answer.error, true);
    // A cell whose machine is this browser: the server sent the program
    // instead of an answer, and wants the answer back.
    if (answer.ran === "browser") {
      take(await api("POST", `/cells/${id}/result`, await evaluate(answer.program)));
    } else {
      take(answer);
    }
  } finally {
    node?.classList.remove("running");
  }
}

async function runAll() {
  $("run-all").disabled = true;
  try {
    // Re-read the list as we go: a cell's output can create cells, and those
    // have already run by the time the creator's answer comes back.
    const done = new Set();
    for (;;) {
      const next = state.cells.find((c) => !done.has(c.id));
      if (!next) break;
      done.add(next.id);
      await run(next.id);
    }
  } finally {
    $("run-all").disabled = false;
  }
}

async function expand(id, node) {
  await save(id);
  const panel = node.querySelector(".program");
  if (!panel.hidden) {
    panel.hidden = true;
    return;
  }
  const answer = await api("GET", `/cells/${id}/expand`);
  if (answer.error) return note(answer.error, true);
  // The expansion is the program the machine is fed. When the cell quotes,
  // that is builder calls and reads nothing like the cell — so the resolved
  // source is shown above it when the two differ.
  const same = answer.resolved.trim() === answer.program.trim();
  panel.textContent = same
    ? answer.program
    : `${answer.resolved}\n\n— becomes (${answer.kind}) —\n\n${answer.program}`;
  panel.hidden = false;
}

/* ── the page frame, and the machine inside it ─────────────────────────── */

const frame = () => $("page").contentWindow;

// Install the quilt runtime in the page's realm, once. A TypeScript cell that
// quotes expands to builder calls (`tb(…).c(…)`), which have to be names the
// page knows; `/runtime.js` is that module.
function installRuntime() {
  const win = frame();
  if (!win || win.__quilt) return Promise.resolve();
  const doc = win.document;
  win.__quilt = new Promise((resolve) => {
    const script = doc.createElement("script");
    script.type = "module";
    script.textContent =
      `import * as RT from "/runtime.js";\n` +
      `await RT.install(window);\n` +
      `window.__quiltReady = RT.RUNTIME_BUILT;\n` +
      `window.dispatchEvent(new Event("quilt-ready"));\n`;
    win.addEventListener("quilt-ready", () => resolve(win.__quiltReady), { once: true });
    doc.head.append(script);
    // A page with no runtime built still runs plain TypeScript; do not wait
    // forever for a module that may have failed to import.
    setTimeout(() => resolve(false), 3000);
  });
  return win.__quilt;
}

// Evaluate a cell's program in the page's realm and answer as a machine
// would: a value in the language's own spelling, plus whatever it printed.
async function evaluate(program) {
  const win = frame();
  if (!win) return { error: "the page frame is not ready" };
  await installRuntime();
  const logs = [];
  const console0 = win.console;
  const capture = (...args) => logs.push(args.map(show).join(" "));
  win.console = Object.assign(Object.create(console0), { log: capture, info: capture });
  try {
    let value = win.eval(program);
    // A cell that fetches is a cell that waits; the notebook waits with it.
    if (value && typeof value.then === "function") value = await value;
    return { stdout: logs.join("\n"), value: literal(value) };
  } catch (e) {
    return { stdout: logs.join("\n"), error: e && e.stack ? e.stack : String(e) };
  } finally {
    win.console = console0;
  }
}

// What the machine answers with: the value as a literal of its own language.
// A quilt term answers with its *code*, which is what makes a TypeScript cell
// that builds HTML land in the page like every other cell's markup.
function literal(value) {
  if (value === undefined) return "undefined";
  if (value && typeof value.coparse === "function") return value.coparse();
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

const show = (v) => (typeof v === "string" ? v : literal(v));

/* ── patches from the server ───────────────────────────────────────────── */

function applyChanges(changes) {
  const doc = frame()?.document;
  if (!doc) return;
  for (const change of changes || []) {
    const anchor = change.id ? doc.getElementById(change.id) : null;
    if (change.kind === "define" && anchor) anchor.outerHTML = change.html;
    else if (change.kind === "remove") anchor?.remove();
    else if (change.kind === "insert") {
      const after = doc.getElementById(change.after);
      if (after) after.insertAdjacentHTML("afterend", change.html);
      else append(doc, change.html);
    } else append(doc, change.html);
  }
}

function append(doc, html) {
  (doc.body || doc.documentElement).insertAdjacentHTML("beforeend", html);
}

// Everything a request answers: the cells, the page patches, or both.
function take(answer) {
  if (!answer) return;
  if (answer.error) note(answer.error, true);
  if (answer.cells) {
    state.cells = answer.cells;
    renderCells();
  }
  applyChanges(answer.changes);
}

function note(message, bad = false) {
  const el = $("note");
  el.textContent = String(message).split("\n")[0];
  el.style.color = bad ? "var(--bad)" : "var(--mute)";
}

/* ── the event stream ──────────────────────────────────────────────────── */

function listen() {
  const events = new EventSource("/events");
  const status = $("status");
  events.addEventListener("open", () => {
    status.textContent = "live";
    status.className = "chip live";
  });
  events.addEventListener("error", () => {
    status.textContent = "reconnecting";
    status.className = "chip lost";
  });
  events.addEventListener("hello", (ev) => {
    const hello = JSON.parse(ev.data);
    state.cells = hello.cells;
    state.typescript = hello.typescript;
    state.runtime = hello.runtime;
    $("engine").textContent = `ts: ${hello.typescript}`;
    if (hello.typescript === "browser" && !hello.runtime) {
      note("TypeScript cells that quote need the browser runtime: wasm-pack build quilt-wasm --target web --out-dir pkg-web");
    }
    renderCells();
  });
  events.addEventListener("cells", (ev) => {
    state.cells = JSON.parse(ev.data).cells;
    renderCells();
  });
  events.addEventListener("page", (ev) => applyChanges(JSON.parse(ev.data).changes));
  events.addEventListener("app", (ev) => {
    const { path, stdout } = JSON.parse(ev.data);
    note(`${path}: ${stdout.trim().split("\n").pop()}`);
  });
}

/* ── glyphs: the chord scheme of the VS Code extension and the playground ── */

const DIR = { ArrowLeft: "L", KeyH: "L", ArrowRight: "R", KeyL: "R", ArrowUp: "U", KeyK: "U", ArrowDown: "D", KeyJ: "D" };
const SINGLE = { L: "←", R: "→", U: "↑", D: "↓", Comma: "⟨", Period: "⟩", KeyT: "⟨T⟩", KeyN: "⟨N⟩", KeyM: "⟨M⟩" };
const DIAG = {
  UL: "↖", LU: "↖", UR: "↗", RU: "↗", DL: "↙", LD: "↙", DR: "↘", RD: "↘",
  UU: "↑", DD: "↓", LL: "←", RR: "→",
};

let chord = null;
let chordTimer = null;
const resetChord = () => {
  chord = null;
  clearTimeout(chordTimer);
};
const armChord = (c) => {
  chord = c;
  clearTimeout(chordTimer);
  chordTimer = setTimeout(resetChord, 1500);
};

function insert(field, text) {
  const { selectionStart: a, selectionEnd: b, value } = field;
  field.value = value.slice(0, a) + text + value.slice(b);
  field.selectionStart = field.selectionEnd = a + text.length;
  field.dispatchEvent(new Event("input"));
}

function onKey(ev, id, field) {
  if ((ev.metaKey || ev.ctrlKey) && ev.key === "Enter") {
    ev.preventDefault();
    resetChord();
    run(id);
    return;
  }
  if ((ev.metaKey || ev.ctrlKey) && (ev.code === "Digit1" || ev.code === "Digit2")) {
    ev.preventDefault();
    armChord(ev.code === "Digit1" ? "1" : "2");
    return;
  }
  if (chord === "1") {
    const glyph = SINGLE[DIR[ev.code] || ev.code];
    if (glyph) {
      ev.preventDefault();
      insert(field, glyph);
    }
    resetChord();
    return;
  }
  if (chord === "2") {
    const d = DIR[ev.code];
    if (d) {
      ev.preventDefault();
      armChord("2:" + d);
    } else resetChord();
    return;
  }
  if (chord?.startsWith("2:")) {
    const d = DIR[ev.code];
    const glyph = d && DIAG[chord.slice(2) + d];
    if (glyph) {
      ev.preventDefault();
      insert(field, glyph);
    }
    resetChord();
    return;
  }
  if (ev.key === "Tab" && !ev.shiftKey && !ev.metaKey && !ev.ctrlKey && !ev.altKey) {
    ev.preventDefault();
    insert(field, "  ");
  }
}

/* ── start ─────────────────────────────────────────────────────────────── */

$("add").addEventListener("click", () => addCell());
$("run-all").addEventListener("click", runAll);
$("stop").addEventListener("click", async () => {
  if (!confirm("End the session? The machines and the page go with it.")) return;
  await api("POST", "/shutdown", {});
  note("the session is over — the page above is still yours to export");
});

for (const tab of document.querySelectorAll(".tabs button")) {
  tab.addEventListener("click", () => {
    for (const other of document.querySelectorAll(".tabs button")) other.classList.toggle("on", other === tab);
    const site = tab.dataset.tab === "site";
    // The site frame is reloaded each time it is shown; the page frame never
    // is — a reload would take the TypeScript machine with it.
    if (site) $("site").src = "/site?at=" + Date.now();
    $("site").hidden = !site;
    $("page").hidden = site;
  });
}

$("page").addEventListener("load", () => installRuntime());
listen();
