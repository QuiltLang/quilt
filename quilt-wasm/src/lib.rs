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
//!
//! Where they *don't* allow it, the difference is recorded rather than papered
//! over — see "Divergences from the Python runtime" in `quilt-wasm/README.md`.
//! The one that shows up in every generated module is `NL`/`POP`/`HOLE`, which
//! are constants there and functions here; [`NL`] and [`HOLE`] carry the two
//! reasons why (issue #167).

use quilt::prelude::{Arc, QTerm};
use quilt::qterm::{
    leaf as mk_leaf, quote as mk_quote, sym as mk_sym, tb as mk_tb, unquote as mk_unquote,
    QTermBuilder,
};
use quilt::strcmd::{push as mk_push, write as mk_write, StrCmd};
use quilt::term::{cmd as mk_cmd, CmdOrHole, STerm};
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

    /// `↑` on this term: the TypeScript source that reconstructs it.
    ///
    /// Exists as a *method* because `&self` borrows. The free functions take a
    /// polymorphic `JsValue`, and the only way to get a `WasmQTerm` back out of
    /// one is `TryFromJsValue`, which **takes** it — nulling the caller's handle
    /// so the term is unusable afterwards. `call_self` therefore dispatches to
    /// this method rather than unwrapping. Prefixed `__` because it is plumbing,
    /// not API: callers use `qlift`.
    #[wasm_bindgen(js_name = __liftSelf)]
    pub fn lift_self(&self) -> WasmQTerm {
        WasmQTerm(lift_term(&self.0))
    }

    /// A non-consuming copy, for the same reason.
    #[wasm_bindgen(js_name = __copySelf)]
    pub fn copy_self(&self) -> WasmQTerm {
        self.clone()
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

/// The `NewLine` command — `NL()` here, the `NL` *constant* in the Python
/// runtime.
///
/// A function because wasm-bindgen has no way to export a module-scope
/// constant: `#[wasm_bindgen]` on a `const` is a hard compile error ("will not
/// work on constants unless you are defining a
/// `#[wasm_bindgen(typescript_custom_section)]`"), and the only items that
/// reach JS are functions, structs, enums and impls. A `static get` on an
/// exported class would give `Consts.NL`, not the bare `NL` the Python runtime
/// spells — reaching that would mean replacing wasm-pack's generated package
/// with a hand-maintained JS entry point, on the npm publish path, for an
/// ergonomics wart. [`HOLE`] documents the second, independent reason. Issue
/// #167 weighed both and kept the divergence, documented.
#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn NL() -> WasmStrCmd {
    WasmStrCmd(StrCmd::NewLine)
}

/// The `Pop` command — `POP()` here, the `POP` constant in the Python runtime.
/// See [`NL`].
#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn POP() -> WasmStrCmd {
    WasmStrCmd(StrCmd::Pop)
}

/// A child placeholder — `HOLE()` here, the `HOLE` constant in the Python
/// runtime.
///
/// A function for [`NL`]'s reason and one of its own, which would survive even
/// if the export problem were solved: [`quote`] and [`unquote`] take
/// `Vec<WasmCmdOrHole>`, and wasm-bindgen **moves** each element out of its JS
/// wrapper (`__unwrap` → `__destroy_into_raw`), nulling the caller's handle.
/// A module-level `HOLE` singleton would be freed by the first `quote(..)` that
/// used it and throw "array contains a value of the wrong type" on the second —
/// the same move-semantics trap `call_self` exists to dodge. Calling it hands
/// out a fresh value each time, which is exactly what makes reuse safe.
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
/// An already-built `QTerm` lifts to the TypeScript source that reconstructs it
/// (see [`lift_term`]), satisfying `↓(↑(x)) == x` — issue #166.
#[wasm_bindgen]
pub fn qlift(value: &JsValue) -> Result<WasmQTerm, JsError> {
    if let Some(q) = lift_if_term(value) {
        return Ok(q);
    }
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
/// Python runtime.
///
/// The term case is a genuine pass-through here, unlike `qlift`: an
/// already-built HTML fragment is already escaped, and re-escaping it would
/// double-encode. The `↓(↑(x)) == x` law that governs `qlift` (issue #166) does
/// not bite, because HTML has no reduce to round-trip through.
///
/// This branch was missing while the doc comment and the error message both
/// advertised it, so lifting a fragment failed at run time with a message
/// naming the very type it was given. Found by the shared runtime corpus (#159).
#[wasm_bindgen]
pub fn qlift_html(value: &JsValue) -> Result<WasmQTerm, JsError> {
    if let Some(q) = copy_if_term(value) {
        return Ok(q);
    }
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

/// Build the TypeScript source that reconstructs `term`, recursively.
///
/// This is what `↑` on an already-built `QTerm` must produce, because `↑` is
/// governed by
///
/// ```text
/// ↓(↑(x)) == x
/// ```
///
/// `↑` maps a value to a term *whose code evaluates back to that value*, and `↓`
/// evaluates a term's code. For `42` that code is `42`; for a term it has to be
/// a constructor call — `leaf("integer", "7")` — so evaluating it yields the
/// term again. Rust's `QLift for Arc<QTerm>` has always done this; the term case
/// was simply missing here. See issue #166.
fn lift_term(term: &Arc<QTerm>) -> Arc<QTerm> {
    use quilt::langs::typescript::ops;

    match &**term {
        QTerm::Tuple { tag, terms, cmds } => {
            let children: Vec<Arc<QTerm>> = terms.iter().map(lift_term).collect();
            ops::build_tuple_code(tag, cmds, &children)
        }
        QTerm::Quote {
            tag,
            index,
            lang,
            term,
            cmds,
            ..
        } => ops::build_quote_code(tag, *index, lang, &lift_term(term), cmds),
        QTerm::Unquote {
            tag,
            index,
            lang,
            term,
            cmds,
            ..
        } => ops::build_unquote_code(tag, *index, lang, &lift_term(term), cmds),
    }
}

/// Call one of `WasmQTerm`'s `__`-prefixed self methods on `value`, if it is one
/// of our terms.
///
/// Deliberately *not* `TryFromJsValue`, which is the obvious choice and is
/// wrong: it **takes** the value out of the JS object, nulling the caller's
/// handle, so `qlift(t)` left `t` unusable and the next `t.coparse()` failed
/// with "null pointer passed to rust". A `&WasmQTerm` *parameter* borrows
/// (wasm-bindgen emits `_assertClass` + `__wbg_ptr`), and so does a `&self`
/// method — so the lift is implemented as a method and reached from here.
fn call_self(value: &JsValue, method: &str) -> Option<WasmQTerm> {
    use wasm_bindgen::convert::TryFromJsValue as _;
    let f = js_sys::Reflect::get(value, &JsValue::from_str(method)).ok()?;
    let f = f.dyn_ref::<js_sys::Function>()?;
    let out = f.call0(value).ok()?;
    WasmQTerm::try_from_js_value(out).ok()
}

/// The lifted form of `value`, if it is one of our terms — leaving `value`
/// itself untouched.
fn lift_if_term(value: &JsValue) -> Option<WasmQTerm> {
    call_self(value, "__liftSelf")
}

/// A copy of `value`, if it is one of our terms — leaving `value` untouched.
/// Used where a lift must pass an existing term straight through.
fn copy_if_term(value: &JsValue) -> Option<WasmQTerm> {
    call_self(value, "__copySelf")
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

/// Direct coverage for `escape_html` (issue #192).
///
/// It had none: the shared runtime corpus reaches it only through `qlift_html`,
/// on one string, which pins the composite answer and not the rules. This is
/// the function that decides whether a lifted value can close an attribute and
/// open a tag, so each rule is worth its own line.
///
/// `quilt-python` carries a byte-identical copy of `escape_html` and this
/// module, because the two bindings share no crate to put it in — the core has
/// no `LiftTo<Html>` at all (issue #149, which is also where a de-duplicated
/// home for it belongs). Keeping the tables identical is the point: the corpus
/// checks the two agree on one string, and these check they agree on the rules.
#[cfg(test)]
mod tests {
    use super::escape_html;

    #[test]
    fn escapes_each_markup_character() {
        assert_eq!(escape_html("&"), "&amp;");
        assert_eq!(escape_html("<"), "&lt;");
        assert_eq!(escape_html(">"), "&gt;");
        assert_eq!(escape_html("\""), "&quot;");
        assert_eq!(escape_html("'"), "&#x27;");
    }

    /// The ordering bug this shape of function invites: escaping `&` *after*
    /// the others turns `<` into `&amp;lt;`, which renders as literal `&lt;`.
    #[test]
    fn does_not_double_encode_its_own_output() {
        assert_eq!(escape_html("<a>"), "&lt;a&gt;");
        assert_eq!(escape_html("a & b < c"), "a &amp; b &lt; c");
    }

    /// Both quote styles, so the result is inert in either attribute spelling.
    #[test]
    fn neutralises_an_attribute_value() {
        assert_eq!(
            escape_html("\" onload='x()'"),
            "&quot; onload=&#x27;x()&#x27;"
        );
    }

    /// Everything else is passed through byte for byte — an escaper that
    /// mangled target text would be caught here and nowhere else.
    #[test]
    fn leaves_everything_else_alone() {
        assert_eq!(escape_html(""), "");
        assert_eq!(escape_html("a/b=c;\n\t"), "a/b=c;\n\t");
        assert_eq!(escape_html("héllo ← ↖ 世界"), "héllo ← ↖ 世界");
    }

    /// The rule set itself, not just the rules that happen to have a test: a
    /// sixth escape added without a line above shows up right here.
    #[test]
    fn escapes_exactly_five_characters() {
        let changed: String = (0u8..=127)
            .map(char::from)
            .filter(|c| {
                let s = c.to_string();
                escape_html(&s) != s
            })
            .collect();
        assert_eq!(changed, "\"&'<>");
    }

    /// Not idempotent, deliberately — which is *why* `qlift_html` passes an
    /// already-built term through instead of re-escaping it.
    #[test]
    fn is_not_idempotent() {
        assert_eq!(escape_html(&escape_html("&")), "&amp;amp;");
    }
}
