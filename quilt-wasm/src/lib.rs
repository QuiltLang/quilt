//! The WebAssembly half of quilt's JavaScript runtime.
//!
//! Expanded `.ts.quilt` programs build terms with `tb`/`leaf`/`sym`/`quote`/…
//! and serialize them with `coparse()`. All of that is plain JavaScript, in
//! `js/runtime.js`: a term is an ordinary object the garbage collector owns.
//! It used to be a wasm-bindgen wrapper around a Rust `Arc<QTerm>`, built a
//! node at a time across the boundary, which cost ~25x what the same builder
//! costs in plain JS, and pinned every term a program kept in wasm memory
//! (issue #306).
//!
//! What is left here is what the JavaScript cannot do without a second copy of
//! Rust logic:
//!
//! * [`lift`]: `qlift` of an already-built term, whose output comes from the
//!   generated tables in `quilt::langs::typescript::ops` (issue #166);
//! * [`fmt_number`]: the numbers JavaScript would print in exponent form.
//!
//! A term crosses in the flat encoding [`decode`] reads and [`encode`] writes
//! (documented beside its JavaScript twin in `js/runtime.js`), once per call.
//! [`coparse`] is here too, so tests and the benchmark can hold the JavaScript
//! `coparse` to the Rust one.
//!
//! `js/assemble.mjs` turns wasm-pack's output into the package: these exports
//! become `quilt_wasm_raw.js`, and `quilt_wasm.js` is the runtime bound to
//! them. See `quilt-wasm/README.md`.

use quilt::prelude::{Arc, QTerm};
use quilt::qterm::{qquote, qtuple, qunquote};
use quilt::strcmd::StrCmd;
use quilt::term::{CmdOrHole, STerm};
use wasm_bindgen::prelude::*;

/**************************************************************/
// The exports.

/// `qlift` of an encoded term: the encoded term whose TypeScript rebuilds it,
/// so `↓(↑(x)) == x` (issue #166). Returns `[ops, strings]`.
#[wasm_bindgen]
pub fn lift(ops: &[u32], strings: Vec<String>) -> Result<js_sys::Array, JsError> {
    let term = decode(ops, &strings).map_err(|e| JsError::new(&format!("lift: {e}")))?;
    let (ops, strings) = encode(&lift_term(&term));
    let out = js_sys::Array::new();
    out.push(&js_sys::Uint32Array::from(&ops[..]));
    out.push(
        &strings
            .into_iter()
            .map(JsValue::from)
            .collect::<js_sys::Array>(),
    );
    Ok(out)
}

/// The Rust `coparse` of an encoded term.
#[wasm_bindgen]
pub fn coparse(ops: &[u32], strings: Vec<String>) -> Result<String, JsError> {
    let term = decode(ops, &strings).map_err(|e| JsError::new(&format!("coparse: {e}")))?;
    Ok(term.coparse())
}

/// Format a JS number: drop the decimal point when it is integral (`42`, not
/// `42.0`), so lifted whole numbers read as integer literals.
#[wasm_bindgen]
pub fn fmt_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// The size of this module's linear memory, in bytes — for tests and
/// benchmarks that check a workload runs in bounded memory (wasm memory only
/// ever grows, so a leak shows up here as growth). Not API.
#[wasm_bindgen(js_name = __memoryBytes)]
pub fn memory_bytes() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        (core::arch::wasm32::memory_size::<0>() * 65536) as f64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0.0
    }
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
/// term again. Rust's `QLift for Arc<QTerm>` has always done this. See issue
/// #166.
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

/**************************************************************/
// The flat encoding.

const TUPLE: u32 = 0;
const QUOTE: u32 = 1;
const UNQUOTE: u32 = 2;

const OP_WRITE: u32 = 0;
const OP_NL: u32 = 1;
const OP_PUSH: u32 = 2;
const OP_POP: u32 = 3;
const OP_HOLE: u32 = 4;

/// Rebuild the term `ops` and `strings` encode.
pub fn decode(ops: &[u32], strings: &[String]) -> Result<Arc<QTerm>, String> {
    let mut reader = Reader {
        ops,
        strings,
        at: 0,
    };
    let term = reader.node()?;
    if reader.at != ops.len() {
        return Err(format!("{} trailing ops", ops.len() - reader.at));
    }
    Ok(term)
}

struct Reader<'a> {
    ops: &'a [u32],
    strings: &'a [String],
    at: usize,
}

impl Reader<'_> {
    fn op(&mut self) -> Result<u32, String> {
        let op = *self.ops.get(self.at).ok_or("truncated term")?;
        self.at += 1;
        Ok(op)
    }

    fn str(&mut self) -> Result<&str, String> {
        let i = self.op()? as usize;
        self.strings
            .get(i)
            .map(String::as_str)
            .ok_or_else(|| format!("no string {i}"))
    }

    fn node(&mut self) -> Result<Arc<QTerm>, String> {
        let kind = self.op()?;
        let tag = self.str()?.to_owned();
        let quoted = match kind {
            TUPLE => None,
            QUOTE | UNQUOTE => {
                let index = u8::try_from(self.op()?).map_err(|e| e.to_string())?;
                let lang = self.str()?.to_owned();
                Some((index, lang, self.node()?))
            }
            k => return Err(format!("unknown term kind {k}")),
        };
        let n = self.op()? as usize;
        let mut cmds = Vec::with_capacity(n);
        let mut children = Vec::new();
        for _ in 0..n {
            cmds.push(match self.op()? {
                OP_WRITE => CmdOrHole::Cmd(StrCmd::Write(self.str()?.into())),
                OP_NL => CmdOrHole::Cmd(StrCmd::NewLine),
                OP_PUSH => CmdOrHole::Cmd(StrCmd::Push(self.str()?.into())),
                OP_POP => CmdOrHole::Cmd(StrCmd::Pop),
                OP_HOLE => {
                    if quoted.is_none() {
                        children.push(self.node()?);
                    }
                    CmdOrHole::Hole
                }
                op => return Err(format!("unknown cmd {op}")),
            });
        }
        Ok(Arc::new(match quoted {
            None => qtuple(&tag, &children, &cmds),
            Some((index, lang, term)) if kind == QUOTE => qquote(&tag, index, &lang, term, &cmds),
            Some((index, lang, term)) => qunquote(&tag, index, &lang, term, &cmds),
        }))
    }
}

/// Encode `term` for the JavaScript `decode`.
pub fn encode(term: &QTerm) -> (Vec<u32>, Vec<String>) {
    let mut writer = Writer::default();
    writer.node(term);
    (writer.ops, writer.strings)
}

#[derive(Default)]
struct Writer {
    ops: Vec<u32>,
    strings: Vec<String>,
}

impl Writer {
    fn str(&mut self, s: &str) -> u32 {
        self.strings.push(s.to_owned());
        (self.strings.len() - 1) as u32
    }

    fn node(&mut self, term: &QTerm) {
        match term {
            QTerm::Tuple { tag, terms, cmds } => {
                let tag = self.str(tag);
                self.ops.extend([TUPLE, tag]);
                self.cmds(cmds, Some(terms));
            }
            QTerm::Quote {
                tag,
                index,
                lang,
                term,
                cmds,
                ..
            } => self.quoted(QUOTE, tag, *index, lang, term, cmds),
            QTerm::Unquote {
                tag,
                index,
                lang,
                term,
                cmds,
                ..
            } => self.quoted(UNQUOTE, tag, *index, lang, term, cmds),
        }
    }

    fn quoted(
        &mut self,
        kind: u32,
        tag: &str,
        index: u8,
        lang: &str,
        term: &QTerm,
        cmds: &[CmdOrHole],
    ) {
        let tag = self.str(tag);
        let lang = self.str(lang);
        self.ops.extend([kind, tag, u32::from(index), lang]);
        self.node(term);
        self.cmds(cmds, None);
    }

    /// A node's cmds; a tuple's `children` follow their holes.
    fn cmds(&mut self, cmds: &[CmdOrHole], children: Option<&[Arc<QTerm>]>) {
        self.ops.push(cmds.len() as u32);
        let mut children = children.unwrap_or_default().iter();
        for cmd in cmds {
            match cmd {
                CmdOrHole::Cmd(StrCmd::Write(s)) => {
                    let s = self.str(s);
                    self.ops.extend([OP_WRITE, s]);
                }
                CmdOrHole::Cmd(StrCmd::NewLine) => self.ops.push(OP_NL),
                CmdOrHole::Cmd(StrCmd::Push(s)) => {
                    let s = self.str(s);
                    self.ops.extend([OP_PUSH, s]);
                }
                CmdOrHole::Cmd(StrCmd::Pop) => self.ops.push(OP_POP),
                CmdOrHole::Hole => {
                    self.ops.push(OP_HOLE);
                    if let Some(child) = children.next() {
                        self.node(child);
                    }
                }
            }
        }
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use quilt::qterm::{leaf, quote, sym, tb, unquote};
    use quilt::term::{cmd, HOLE};

    fn round_trip(t: &Arc<QTerm>) {
        let (ops, strings) = encode(t);
        let back = decode(&ops, &strings).unwrap();
        assert_eq!(&back, t);
        assert_eq!(back.coparse(), t.coparse());
    }

    #[test]
    fn encoding_round_trips() {
        round_trip(&leaf("number", "7"));
        round_trip(&leaf("text", ""));
        round_trip(
            &tb("block")
                .w("{")
                .p("    ")
                .n()
                .c(&sym("x"))
                .x()
                .n()
                .w("}")
                .b(),
        );
        let body = leaf("number", "5");
        let cmds = [
            cmd(StrCmd::Write("[".into())),
            HOLE,
            cmd(StrCmd::Write("]".into())),
        ];
        round_trip(&quote("x", 0, "ts", body.clone(), &cmds));
        round_trip(&tb("p").c(&unquote("y", 1, "ts", body, &[HOLE, HOLE])).b());
    }

    #[test]
    fn decode_rejects_malformed_input() {
        let strings = ["t".to_owned()];
        assert!(decode(&[], &strings).is_err());
        assert!(decode(&[9, 0, 0], &strings).is_err());
        assert!(decode(&[TUPLE, 1, 0], &strings).is_err());
        assert!(decode(&[TUPLE, 0, 1, 7], &strings).is_err());
        assert!(decode(&[TUPLE, 0, 0, 0], &strings).is_err());
    }
}
