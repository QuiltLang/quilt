// The quilt-wasm runtime: the builder API expanded `.ts.quilt` programs call,
// as plain JavaScript (issue #306).
//
// Terms used to be wasm-bindgen wrappers around a Rust `Arc<QTerm>`, so every
// `tb(..)`, `.t(..)`, `.w(..)` and `.b()` crossed into wasm, allocated a
// wrapper and registered it with a FinalizationRegistry. A page of a few
// thousand terms cost ~25x what the same builder in plain JS did, and terms a
// program kept pinned their subtrees in wasm memory until a finalizer ran.
//
// Here a term is an ordinary object the garbage collector owns, and `coparse`
// is the short loop over write / newline / push / pop that Rust's
// `PrefixWriter` runs. The shared corpus (conformance/runtime/cases.json) pins
// it against the Rust and Python runtimes, and test/differential.cjs against
// the Rust `coparse` itself.
//
// Wasm is used only at the edges, where reimplementing would mean a second
// copy of Rust logic that changes with the grammars:
//
//   * `qlift(term)` — the TypeScript that rebuilds a term comes from the
//     generated tables in `quilt::langs::typescript::ops`;
//   * `qlift(n)` for a number JavaScript would print in exponent form, where
//     Rust's `{}` does not.
//
// `qlift(term)` sends the term across once, in the flat encoding below.
//
// This file is the source; `assemble.mjs` copies it into the wasm-pack
// package as `quilt_runtime.js` (CommonJS for `--target nodejs`) and writes
// the `quilt_wasm.js` entry that binds it to the wasm. Nothing here may run
// wasm at import time: in the browser the wasm exists only after `init()`.

/**************************************************************/
// Edges into wasm.

let wasm = null;

/** Called once by the package entry with the wasm-bindgen module. Not API. */
function __bindWasm(raw) {
  wasm = raw;
}

function needWasm(what) {
  if (wasm === null) throw new Error(`quilt-wasm: ${what} needs the wasm module, which is not loaded`);
  return wasm;
}

/**************************************************************/
// Commands.

const K_TERM = 0;
const K_NL = 1;
const K_POP = 2;
const K_PUSH = 3;
const K_WRITE = 4;
const K_HOLE = 5;

const TUPLE = 0;
const QUOTE = 1;
const UNQUOTE = 2;

/**
 * A single string command: `write(s)`, `NL()`, `push(s)` or `POP()`.
 *
 * Inside a term only newline, push and pop are stored as these; a write is
 * stored as the bare string, which is most of what a term holds.
 */
class WasmStrCmd {
  constructor(k, s) {
    this.k = k;
    this.s = s;
  }
}

/** A `StrCmd` or a child placeholder (`HOLE()`), for `quote`/`unquote` cmds. */
class WasmCmdOrHole {
  constructor(k, s) {
    this.k = k;
    this.s = s;
  }
}

const NL_CMD = new WasmStrCmd(K_NL, "");
const POP_CMD = new WasmStrCmd(K_POP, "");
const HOLE_CMD = new WasmCmdOrHole(K_HOLE, "");

/**************************************************************/
// Terms.

/**
 * A quilt term. Immutable once built, so it can be spliced any number of
 * times, kept, and shared between trees.
 *
 * `seq` is the term's commands in order, with each child *in* its hole: a
 * string is a write, a `WasmQTerm` is a child, anything else a `WasmStrCmd`.
 * For a quote or unquote, `term` is the quoted child, and `seq` holds it at
 * every `HOLE` in the cmds it was built with.
 */
class WasmQTerm {
  constructor(k, tag, seq, index, lang, term) {
    this.k = K_TERM;
    this.kind = k;
    this.tag = tag;
    this.seq = seq;
    this.index = index;
    this.lang = lang;
    this.term = term;
  }

  /** Serialize the term back to source code. */
  coparse() {
    return coparse(this);
  }

  toString() {
    return coparse(this);
  }

  /**
   * Nothing to free: the garbage collector owns a term. Kept so code written
   * for the wasm-bindgen classes (`t.free()`, `using t = …`) still runs.
   */
  free() {}

  [Symbol.dispose]() {}
}

/**
 * A fluent term builder. Each method appends to it and returns it, so
 * `tb("x").w("a").t(child).b()` reads as it does in Rust.
 */
class WasmBuilder {
  constructor(tag) {
    this.tag = tag;
    this.seq = [];
  }

  /** Splice a child term. */
  c(child) {
    this.seq.push(asTerm(child));
    return this;
  }

  /**
   * Splice a child term. The same as `c` here; it once *took* the child, which
   * mattered when a term was a wasm handle (#297), and expanded code still
   * spells a child it has just built this way.
   */
  t(child) {
    this.seq.push(asTerm(child));
    return this;
  }

  /** Emit a child term (for a single term, the same as `c`). */
  e(child) {
    this.seq.push(asTerm(child));
    return this;
  }

  /** Write literal source text. Like Rust's builder, an empty write is dropped. */
  w(s) {
    if (s !== "") this.seq.push(s);
    return this;
  }

  /** Emit a newline (respecting the current prefix). */
  n() {
    this.seq.push(NL_CMD);
    return this;
  }

  /** Push an indentation prefix. */
  p(s) {
    this.seq.push(new WasmStrCmd(K_PUSH, s));
    return this;
  }

  /** Pop an indentation prefix. */
  x() {
    this.seq.push(POP_CMD);
    return this;
  }

  /** Build the term. */
  b() {
    return new WasmQTerm(TUPLE, this.tag, this.seq, 0, "", null);
  }

  free() {}

  [Symbol.dispose]() {}
}

function asTerm(x) {
  if (x instanceof WasmQTerm) return x;
  throw new TypeError(`expected a WasmQTerm, got ${describe(x)}`);
}

function describe(x) {
  if (x === null) return "null";
  if (typeof x === "object") return x.constructor?.name ?? "object";
  return typeof x;
}

/**************************************************************/
// Constructors.

/** Start building a tuple node with the given tag. */
function tb(tag) {
  return new WasmBuilder(tag);
}

/** A leaf node: a tag whose only content is `code`. */
function leaf(tag, code) {
  return new WasmQTerm(TUPLE, tag, [code], 0, "", null);
}

/** A symbol: a leaf whose tag and code are the same. */
function sym(s) {
  return new WasmQTerm(TUPLE, s, [s], 0, "", null);
}

/** An identifier term (the `⟨N⟩` operator). */
function name(s) {
  return new WasmQTerm(TUPLE, "identifier", [s], 0, "", null);
}

/** A quoted fragment. */
function quote(tag, index, lang, term, cmds) {
  return quoted(QUOTE, tag, index, lang, term, cmds);
}

/** An unquoted splice. */
function unquote(tag, index, lang, term, cmds) {
  return quoted(UNQUOTE, tag, index, lang, term, cmds);
}

function quoted(kind, tag, index, lang, term, cmds) {
  asTerm(term);
  const seq = [];
  for (const c of cmds) {
    if (!(c instanceof WasmStrCmd || c instanceof WasmCmdOrHole)) {
      throw new TypeError(`expected a cmd(..) or HOLE(), got ${describe(c)}`);
    }
    if (c.k === K_HOLE) seq.push(term);
    else if (c.k === K_WRITE) seq.push(c.s);
    else seq.push(c);
  }
  // wasm-bindgen passed `index` as a u8.
  return new WasmQTerm(kind, tag, seq, (index | 0) & 0xff, lang, term);
}

/** Wrap a `StrCmd` as a `CmdOrHole`. */
function cmd(c) {
  if (c instanceof WasmStrCmd) return c;
  throw new TypeError(`expected a WasmStrCmd, got ${describe(c)}`);
}

/** A `Write` command. */
function write(s) {
  return new WasmStrCmd(K_WRITE, s);
}

/** A `Push` command. */
function push(s) {
  return new WasmStrCmd(K_PUSH, s);
}

/**
 * The `NewLine` command. A function, like `POP()` and `HOLE()`, because the
 * wasm-bindgen runtime could not export a constant (#167); generated code
 * calls it, so it stays one.
 */
function NL() {
  return NL_CMD;
}

/** The `Pop` command. */
function POP() {
  return POP_CMD;
}

/** A child placeholder. */
function HOLE() {
  return HOLE_CMD;
}

/**************************************************************/
// Coparse.

/**
 * Write `t` as Rust's `PrefixWriter` does: a newline writes `\n` and then
 * every pushed prefix, and popping an empty stack does nothing. (Escaping and
 * bracketing, the writer's other state, only matter to `coparse_quilt`.)
 */
function coparse(t) {
  out = "";
  prefix = "";
  stack = [];
  writeTerm(t);
  // Rust receives every string as UTF-8, so a lone surrogate came back as
  // U+FFFD; keep that.
  const s = out;
  out = "";
  return typeof s.toWellFormed === "function" ? s.toWellFormed() : s;
}

// The writer's state. Module-level rather than an object passed down, which
// is measurably faster; `coparse` calls nothing it could re-enter through.
let out = "";
let prefix = "";
let stack = [];

function writeTerm(t) {
  const seq = t.seq;
  for (let i = 0; i < seq.length; i++) {
    const x = seq[i];
    if (typeof x === "string") {
      out += x;
      continue;
    }
    switch (x.k) {
      case K_TERM:
        writeTerm(x);
        break;
      case K_NL:
        out += "\n" + prefix;
        break;
      case K_PUSH:
        stack.push(prefix);
        prefix += x.s;
        break;
      case K_POP:
        if (stack.length > 0) prefix = stack.pop();
        break;
    }
  }
}

/**************************************************************/
// Lifts.

/**
 * Lift a JS value to a term that reconstructs it (the homogeneous `↑`,
 * TypeScript into TypeScript): `number`, `string`, `boolean`, and a term,
 * which lifts to the TypeScript that rebuilds it, so `↓(↑(x)) == x` (#166).
 */
function qlift(value) {
  if (value instanceof WasmQTerm) return liftTerm(value);
  if (typeof value === "boolean") {
    const s = value ? "true" : "false";
    return leaf(s, s);
  }
  if (typeof value === "number") return leaf("number", fmtNumber(value));
  if (typeof value === "string") return leaf("string", tsStringLit(value));
  throw new Error("qlift: unsupported type (expected number, string, boolean, or QTerm)");
}

/**
 * Lift a JS value to an HTML term (the `↑` operator with an `html` splice
 * target). Strings become entity-escaped `text` leaves; a term passes through
 * unchanged, since an HTML fragment is already escaped.
 */
function qlift_html(value) {
  if (value instanceof WasmQTerm) return value;
  if (typeof value === "string") return leaf("text", escapeHtml(value));
  if (typeof value === "boolean") return leaf("text", value ? "true" : "false");
  if (typeof value === "number") return leaf("text", fmtNumber(value));
  throw new Error("qlift_html: unsupported type (expected number, string, boolean, or QTerm)");
}

/**
 * Lift a JS value into a text quote (the `↑` operator with a `text` splice
 * target), in the rules of `quilt::lift::text`: by default JSON's spelling —
 * a string as a `"…"` literal, a number as its digits, a boolean as
 * `true`/`false`, an array as `[a, b]`, and a term as the text it stands for.
 *
 * `dialect` overrides any of `string(s)`, `in_quotes(s)`, `boolean(b)`,
 * `float(f)` (functions returning a string) and `list_open`, `list_sep`,
 * `list_close` (strings), the names the Python runtime's `TextDialect` has;
 * what it leaves out stays JSON's. A number with no fractional part is an
 * integer here, as in `qlift`: JS has one number type, so `1.0` is `1`.
 */
function qlift_text(value, dialect) {
  return leaf("text", renderText(value, dialect ?? null, false));
}

/**
 * `qlift_text` for a hole between two `"` the author wrote: a string renders
 * as its escaped contents, with no quotes of its own, and an array as its
 * elements joined by the separator.
 */
function qlift_text_in_quotes(value, dialect) {
  return leaf("text", renderText(value, dialect ?? null, true));
}

function dialectRule(dialect, rule, arg) {
  const f = dialect?.[rule];
  if (typeof f !== "function") return null;
  const out = f.call(dialect, arg);
  if (typeof out !== "string") throw new TypeError(`qlift_text: dialect.${rule} must return a string`);
  return out;
}

function dialectStr(dialect, key, fallback) {
  const v = dialect?.[key];
  return typeof v === "string" ? v : fallback;
}

function renderText(value, dialect, inQuotes) {
  if (value instanceof WasmQTerm) return coparse(value);
  if (typeof value === "boolean") return dialectRule(dialect, "boolean", value) ?? String(value);
  if (typeof value === "number") {
    if (Number.isInteger(value) && Math.abs(value) < 9007199254740992) return String(value);
    const custom = dialectRule(dialect, "float", value);
    if (custom !== null) return custom;
    if (!Number.isFinite(value)) {
      throw new Error(
        `cannot lift ${value} into text: JSON has no spelling for NaN or infinity ` +
          "(give the dialect a `float` rule that has one)",
      );
    }
    return fmtNumber(value);
  }
  if (typeof value === "string") {
    const custom = dialectRule(dialect, inQuotes ? "in_quotes" : "string", value);
    if (custom !== null) return custom;
    // `quilt::lift::json_escape` is `ts_dquote_escape`'s table.
    const lit = tsStringLit(value);
    return inQuotes ? lit.slice(1, -1) : lit;
  }
  if (Array.isArray(value)) {
    const body = value.map((x) => renderText(x, dialect, inQuotes)).join(dialectStr(dialect, "list_sep", ", "));
    return inQuotes ? body : dialectStr(dialect, "list_open", "[") + body + dialectStr(dialect, "list_close", "]");
  }
  throw new Error("qlift_text: unsupported type (expected number, string, boolean, array, or QTerm)");
}

// `quilt::lift::escape_html`.
const HTML_ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;" };
const HTML_SPECIAL = /[&<>"']/;
const HTML_SPECIALS = /[&<>"']/g;
const htmlEscape = (c) => HTML_ESCAPES[c];

function escapeHtml(s) {
  return HTML_SPECIAL.test(s) ? s.replace(HTML_SPECIALS, htmlEscape) : s;
}

/**
 * Format a number as the wasm runtime's `fmt_number` does: an integral value
 * below 2^53 as an integer, anything else as Rust's `{}` prints an f64. In
 * [1e-6, 1e21) JavaScript prints the same shortest round-trip digits Rust
 * does; outside it JavaScript switches to exponent form, so ask Rust.
 */
function fmtNumber(n) {
  if (Number.isInteger(n) && Math.abs(n) < 9007199254740992) return String(n);
  const a = Math.abs(n);
  if (a >= 1e-6 && a < 1e21) return String(n);
  return needWasm("qlift of this number").fmt_number(n);
}

// `str_lit` in `quilt::langs::typescript::ops`: a double-quoted literal with
// `"` and `\` escaped and control characters (Unicode Cc) spelled out.
const TS_SPECIAL = /["\\\u0000-\u001f\u007f-\u009f]/;

function tsStringLit(s) {
  if (!TS_SPECIAL.test(s)) return `"${s}"`;
  let out = '"';
  for (const c of s) {
    const code = c.codePointAt(0);
    if (c === '"' || c === "\\") out += "\\" + c;
    else if (c === "\n") out += "\\n";
    else if (c === "\r") out += "\\r";
    else if (c === "\t") out += "\\t";
    else if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) out += "\\u" + code.toString(16).padStart(4, "0");
    else out += c;
  }
  return out + '"';
}

/**************************************************************/
// The flat encoding a term crosses into wasm in, and comes back in.
//
// Pre-order, as u32 ops plus a string table:
//
//   tuple:    0 tag n  cmd*n          cmd: 0 s        write
//   quote:    1 tag index lang term n cmd*n     1          newline
//   unquote:  2 tag index lang term n cmd*n     2 s        push
//                                               3          pop
//                                               4 [child]  hole
//
// A tuple's hole is followed by its child; a quote's is not, its child being
// `term`. Mirrored by `decode`/`encode` in src/lib.rs.

const OP_WRITE = 0;
const OP_NL = 1;
const OP_PUSH = 2;
const OP_POP = 3;
const OP_HOLE = 4;

function encode(t) {
  const ops = [];
  const strings = [];
  const str = (s) => strings.push(String(s)) - 1;
  const node = (t) => {
    const quoted = t.kind !== TUPLE;
    ops.push(t.kind, str(t.tag));
    if (quoted) {
      ops.push(t.index, str(t.lang));
      node(t.term);
    }
    ops.push(t.seq.length);
    for (const x of t.seq) {
      if (typeof x === "string") ops.push(OP_WRITE, str(x));
      else if (x.k === K_TERM) {
        ops.push(OP_HOLE);
        if (!quoted) node(x);
      } else if (x.k === K_NL) ops.push(OP_NL);
      else if (x.k === K_POP) ops.push(OP_POP);
      else ops.push(OP_PUSH, str(x.s));
    }
  };
  node(t);
  return [Uint32Array.from(ops), strings];
}

function decode(ops, strings) {
  let i = 0;
  const node = () => {
    const kind = ops[i++];
    const tag = strings[ops[i++]];
    let index = 0;
    let lang = "";
    let term = null;
    if (kind !== TUPLE) {
      index = ops[i++];
      lang = strings[ops[i++]];
      term = node();
    }
    const n = ops[i++];
    const seq = new Array(n);
    for (let j = 0; j < n; j++) {
      const op = ops[i++];
      if (op === OP_WRITE) seq[j] = strings[ops[i++]];
      else if (op === OP_NL) seq[j] = NL_CMD;
      else if (op === OP_POP) seq[j] = POP_CMD;
      else if (op === OP_PUSH) seq[j] = new WasmStrCmd(K_PUSH, strings[ops[i++]]);
      else seq[j] = kind === TUPLE ? node() : term;
    }
    return new WasmQTerm(kind, tag, seq, index, lang, term);
  };
  return node();
}

function liftTerm(t) {
  const [ops, strings] = encode(t);
  const [outOps, outStrings] = needWasm("qlift of a term").lift(ops, strings);
  return decode(outOps, outStrings);
}

/**
 * `term.coparse()` done by the Rust `QTerm` instead: one crossing for the
 * whole tree. For tests and the benchmark, which hold the two to the same
 * output. Not API.
 */
function __coparseWasm(t) {
  const [ops, strings] = encode(asTerm(t));
  return needWasm("__coparseWasm").coparse(ops, strings);
}

/**************************************************************/

export {
  HOLE,
  NL,
  POP,
  WasmBuilder,
  WasmCmdOrHole,
  WasmQTerm,
  WasmStrCmd,
  __bindWasm,
  __coparseWasm,
  cmd,
  leaf,
  name,
  push,
  qlift,
  qlift_html,
  qlift_text,
  qlift_text_in_quotes,
  quote,
  sym,
  tb,
  unquote,
  write,
};
