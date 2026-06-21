//! WebAssembly bindings for quilt's core IR.
//!
//! These expose the real Rust `QTerm`, its builder, and the `coparse`
//! serializer to JavaScript — the browser runtime that expanded `.ts.quilt`
//! files target. A `TypeScriptMetaLanguage` (issue #45) emits TypeScript source
//! like `tb("binary_expression").c(leaf("number", "1")).w(" ")..b()`, and these
//! bindings are exactly the `tb`/`leaf`/`sym`/`quote`/`unquote`/`cmd`/`write`/
//! `push`/`name` functions, the `NL`/`POP`/`HOLE` constructors, and the fluent
//! `Builder` (`.c`/`.w`/`.n`/`.p`/`.x`/`.e`/`.b`) and `QTerm` (`.coparse()`)
//! classes that source calls into. It mirrors the PyO3 runtime in
//! `quilt-python/src/lib.rs`, one-for-one where the two host ABIs allow.

use quilt::prelude::{Arc, QTerm};
use quilt::qterm::{
    leaf as mk_leaf, quote as mk_quote, sym as mk_sym, tb as mk_tb, unquote as mk_unquote,
    QTermBuilder,
};
use quilt::sink::{write_tree as mk_write_tree, TarSink, ZipSink};
use quilt::strcmd::{push as mk_push, write as mk_write, StrCmd};
use quilt::term::{cmd as mk_cmd, CmdOrHole, STerm};
use quilt::tree::{self, Node, QTree};
use wasm_bindgen::prelude::*;

/**************************************************************/

/// A quilt term (`Arc<QTerm>`).
#[wasm_bindgen]
#[derive(Clone)]
pub struct WasmQTerm(Arc<QTerm>);

#[wasm_bindgen]
impl WasmQTerm {
    /// Serialize the term back to source code.
    pub fn coparse(&self) -> String {
        self.0.coparse()
    }

    #[wasm_bindgen(js_name = toString)]
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        self.0.coparse()
    }
}

/// A single string command (`write`/`NL`/`push`/`POP`).
#[wasm_bindgen]
#[derive(Clone)]
pub struct WasmStrCmd(StrCmd);

/// A `StrCmd` or a child placeholder (`HOLE`), used in `quote`/`unquote` cmds.
#[wasm_bindgen]
#[derive(Clone)]
pub struct WasmCmdOrHole(CmdOrHole);

/// A fluent term builder, mirroring the Rust `QTermBuilder` (consuming form:
/// each method takes `self` and returns the next builder, so chaining works
/// from JS exactly as `tb("x").w("a").c(child).b()`).
#[wasm_bindgen]
pub struct WasmBuilder(QTermBuilder);

#[wasm_bindgen]
impl WasmBuilder {
    /// Splice a child term.
    pub fn c(self, child: &WasmQTerm) -> WasmBuilder {
        WasmBuilder(self.0.c(&child.0))
    }

    /// Write literal source text.
    pub fn w(self, s: &str) -> WasmBuilder {
        WasmBuilder(self.0.w(s))
    }

    /// Emit a newline (respecting the current prefix).
    pub fn n(self) -> WasmBuilder {
        WasmBuilder(self.0.n())
    }

    /// Push an indentation prefix.
    pub fn p(self, s: &str) -> WasmBuilder {
        WasmBuilder(self.0.p(s))
    }

    /// Pop an indentation prefix.
    pub fn x(self) -> WasmBuilder {
        WasmBuilder(self.0.x())
    }

    /// Emit a child term (for an `Arc<QTerm>` this is the same as [`c`]).
    pub fn e(self, child: &WasmQTerm) -> WasmBuilder {
        WasmBuilder(self.0.e(child.0.clone()))
    }

    /// Build the term. Consumes the builder.
    pub fn b(self) -> WasmQTerm {
        WasmQTerm(self.0.b())
    }
}

/**************************************************************/

/// Start building a tuple node with the given tag.
#[wasm_bindgen]
pub fn tb(tag: &str) -> WasmBuilder {
    WasmBuilder(mk_tb(tag))
}

/// A leaf node: a tag whose only content is `code`.
#[wasm_bindgen]
pub fn leaf(tag: &str, code: &str) -> WasmQTerm {
    WasmQTerm(mk_leaf(tag, code))
}

/// A symbol: a leaf whose tag and code are the same.
#[wasm_bindgen]
pub fn sym(s: &str) -> WasmQTerm {
    WasmQTerm(mk_sym(s))
}

/// A quoted fragment.
#[wasm_bindgen]
pub fn quote(
    tag: &str,
    index: u8,
    lang: &str,
    term: &WasmQTerm,
    cmds: Vec<WasmCmdOrHole>,
) -> WasmQTerm {
    let cmds: Vec<CmdOrHole> = cmds.into_iter().map(|c| c.0).collect();
    WasmQTerm(mk_quote(tag, index, lang, term.0.clone(), &cmds))
}

/// An unquoted splice.
#[wasm_bindgen]
pub fn unquote(
    tag: &str,
    index: u8,
    lang: &str,
    term: &WasmQTerm,
    cmds: Vec<WasmCmdOrHole>,
) -> WasmQTerm {
    let cmds: Vec<CmdOrHole> = cmds.into_iter().map(|c| c.0).collect();
    WasmQTerm(mk_unquote(tag, index, lang, term.0.clone(), &cmds))
}

/// Wrap a `StrCmd` as a `CmdOrHole`.
#[wasm_bindgen]
pub fn cmd(c: &WasmStrCmd) -> WasmCmdOrHole {
    WasmCmdOrHole(mk_cmd(c.0.clone()))
}

/// A `Write` command.
#[wasm_bindgen]
pub fn write(s: &str) -> WasmStrCmd {
    WasmStrCmd(mk_write(s))
}

/// A `Push` command.
#[wasm_bindgen]
pub fn push(s: &str) -> WasmStrCmd {
    WasmStrCmd(mk_push(s))
}

/// The `NewLine` command (the `NL` constant in the Python runtime).
#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn NL() -> WasmStrCmd {
    WasmStrCmd(StrCmd::NewLine)
}

/// The `Pop` command (the `POP` constant in the Python runtime).
#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn POP() -> WasmStrCmd {
    WasmStrCmd(StrCmd::Pop)
}

/// A child placeholder (the `HOLE` constant in the Python runtime).
#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn HOLE() -> WasmCmdOrHole {
    WasmCmdOrHole(CmdOrHole::Hole)
}

/// An identifier term (the `⟨N⟩` operator).
#[wasm_bindgen]
pub fn name(s: &str) -> WasmQTerm {
    WasmQTerm(mk_leaf("identifier", s))
}

/**************************************************************/

/// Lift a JS value to a term that reconstructs it (the homogeneous `↑`
/// operator, TypeScript into TypeScript). Supports `number`, `string`, and
/// `boolean`. Numbers with no fractional part lift to integer literals;
/// everything is coparse-only, so the tags are advisory.
///
/// Unlike the Python runtime's `qlift`, this does *not* pass an already-built
/// `QTerm` through unchanged: recovering an exported wasm-bindgen type from a
/// polymorphic `JsValue` needs target-specific glue. The demos never lift a
/// term (terms splice via `↙…↘`), so this is sufficient; a JS shim can add
/// pass-through later if needed.
#[wasm_bindgen]
pub fn qlift(value: &JsValue) -> Result<WasmQTerm, JsError> {
    if let Some(b) = value.as_bool() {
        let s = if b { "true" } else { "false" };
        return Ok(WasmQTerm(mk_leaf(s, s)));
    }
    if let Some(n) = value.as_f64() {
        return Ok(WasmQTerm(mk_leaf("number", &fmt_number(n))));
    }
    if let Some(s) = value.as_string() {
        return Ok(WasmQTerm(mk_leaf("string", &ts_string_lit(&s))));
    }
    Err(JsError::new(
        "qlift: unsupported type (expected number, string, boolean, or QTerm)",
    ))
}

/// Lift a JS value to an HTML term (the `↑` operator with an `html` splice
/// target). Strings become entity-escaped `text` leaves — inert as text content
/// or as a double-quoted attribute value — and terms pass through unchanged, so
/// already-built fragments can be lifted too. Mirrors `qlift_html` in the
/// Python runtime, minus the `QTerm` pass-through (see [`qlift`]).
#[wasm_bindgen]
pub fn qlift_html(value: &JsValue) -> Result<WasmQTerm, JsError> {
    if let Some(b) = value.as_bool() {
        return Ok(WasmQTerm(mk_leaf("text", if b { "true" } else { "false" })));
    }
    if let Some(n) = value.as_f64() {
        return Ok(WasmQTerm(mk_leaf("text", &fmt_number(n))));
    }
    if let Some(s) = value.as_string() {
        return Ok(WasmQTerm(mk_leaf("text", &escape_html(&s))));
    }
    Err(JsError::new(
        "qlift_html: unsupported type (expected number, string, boolean, or QTerm)",
    ))
}

/// Format a JS number: drop the decimal point when it is integral (`42`, not
/// `42.0`), so lifted whole numbers read as integer literals.
fn fmt_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Render a JavaScript/TypeScript double-quoted string literal, escaping the
/// characters the lexer would otherwise interpret.
fn ts_string_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escape `& < > " '` so the result is inert HTML wherever a hole can sit.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/**************************************************************/
// The directory layer (issue #97): build a `QTree` in the browser and pack it
// into a `.zip`/`.tar` `Uint8Array` — so the playground can instantiate a
// template and offer the result as a download with no backend. The archive
// sinks touch no filesystem, so they are safe and dependency-free on wasm.

/// A node in a [`QTree`]: a subdirectory, a file, or a verbatim blob. Build one
/// with `file`/`raw`/`rawBytes`/`subdir`/`link`.
#[wasm_bindgen]
pub struct WasmNode(Node);

/// A generated directory tree, the directory analog of `QTerm`. Build it with
/// `.emit(path, node)`, then pack it with `.zip()` / `.tar()` to download.
#[wasm_bindgen]
#[derive(Default, Clone)]
pub struct WasmQTree(QTree);

#[wasm_bindgen]
impl WasmQTree {
    /// An empty tree.
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmQTree {
        WasmQTree(QTree::new())
    }

    /// Insert (or replace) a leaf at `path` (a `/`-joined string), creating
    /// intermediate directories. Throws on an invalid path component.
    pub fn emit(&mut self, path: &str, node: &WasmNode) -> Result<(), JsError> {
        self.0
            .emit(path, node.0.clone())
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// A "find"-style listing of every path, for debugging.
    pub fn listing(&self) -> String {
        self.0.listing()
    }

    /// Pack the tree into a store-only ZIP archive (a `Uint8Array` in JS) —
    /// wrap in a `Blob` to download.
    pub fn zip(&self) -> Result<Vec<u8>, JsError> {
        let mut sink = ZipSink::new();
        mk_write_tree(&mut sink, &self.0).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(sink.into_bytes())
    }

    /// Pack the tree into a ustar TAR archive (a `Uint8Array` in JS).
    pub fn tar(&self) -> Result<Vec<u8>, JsError> {
        let mut sink = TarSink::new();
        mk_write_tree(&mut sink, &self.0).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(sink.into_bytes())
    }
}

/// A generated source-file leaf whose content is a `QTerm` (serialized via
/// `coparse` when packed).
#[wasm_bindgen]
pub fn file(content: &WasmQTerm) -> WasmNode {
    WasmNode(tree::file(content.0.clone()))
}

/// A verbatim text-file leaf (the common case for instantiated templates).
#[wasm_bindgen]
pub fn raw(text: &str) -> WasmNode {
    WasmNode(tree::raw(text.as_bytes().to_vec()))
}

/// A verbatim binary-file leaf (a `Uint8Array` from JS).
#[wasm_bindgen(js_name = rawBytes)]
pub fn raw_bytes(bytes: &[u8]) -> WasmNode {
    WasmNode(tree::raw(bytes.to_vec()))
}

/// A subdirectory node wrapping a `QTree` (the `dir!` analog).
#[wasm_bindgen]
pub fn subdir(t: &WasmQTree) -> WasmNode {
    WasmNode(Node::Dir(t.0.clone()))
}

/// A symlink leaf pointing at a relative path within the tree.
#[wasm_bindgen]
pub fn link(target: &str) -> WasmNode {
    WasmNode(tree::link(target))
}
