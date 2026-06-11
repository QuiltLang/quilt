//! Syntactic pattern matching for quoted terms: `let ↖pattern↗ = ↖value↗;`.
//!
//! A pattern quote in the binding position of a ground `let` destructures the
//! value term by matching its shape: the pattern's ground unquotes (`↙name↘`)
//! become *metavariables* that bind the corresponding parts of the value. The
//! expander rewrites such a statement into a destructuring of [`qmatch_n`]
//! (see `Expander::expand_pattern_let` in `multi.rs`); the pattern term it
//! builds carries an [`mvar`] marker at each metavariable position.
//!
//! Matching is *textual*: both sides are flattened to the source text they
//! coparse to, the pattern becoming a sequence of literal segments separated
//! by metavariable gaps. The subject must reproduce the literals exactly
//! (matching is anchored at both ends and whitespace-sensitive); each gap
//! binds the text in between, leftmost-shortest when several positions would
//! match. Bindings are returned as `text` leaf terms, so splicing one into a
//! later quote reproduces the matched source verbatim.
//!
//! Like `lift.rs`, this module is part of the parser-free runtime that
//! expanded code links against, plus the codegen helpers ([`pattern_var_code`],
//! [`pattern_let_code`]) the meta-languages use to spell calls into it.

use crate::prelude::*;
use crate::qterm::QTerm;
use crate::strcmd::StrCmd;
use crate::term::{CmdOrHole, STerm};
use miette::{bail, ensure};

/**************************************************************/

/// The tag marking a metavariable term inside a pattern (see [`mvar`]).
pub const MVAR: &str = "mvar";

/// A metavariable marker: the term spliced where `↙name↘` sits in a pattern
/// quote. Expanded terms otherwise never contain `Unquote` nodes, so the
/// variant doubles as the marker; it coparses back to `↙name↘` for debugging.
pub fn mvar(name: &str) -> Arc<QTerm> {
    unquote(
        MVAR,
        0,
        "",
        sym(name),
        &[cmd(write("↙")), HOLE, cmd(write("↘"))],
    )
}

/**************************************************************/

/// A flattened pattern: literal source text interleaved with metavariables.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Lit(String),
    Var(String),
}

/// Flatten a pattern term to segments. This mirrors `STerm::write` /
/// `PrefixWriter` (same prefix-stack semantics), so the literals are exactly
/// the text the term would coparse to, with [`mvar`] markers cut out as
/// metavariable gaps.
#[derive(Default)]
struct Flattener {
    segs: Vec<Seg>,
    buf: String,
    stack: Vec<String>,
}

impl Flattener {
    fn flush(&mut self) {
        if !self.buf.is_empty() {
            self.segs.push(Seg::Lit(std::mem::take(&mut self.buf)));
        }
    }

    fn term(&mut self, term: &QTerm) {
        if let QTerm::Unquote { tag, term, .. } = term {
            if &**tag == MVAR {
                self.flush();
                self.segs.push(Seg::Var(term.coparse()));
                return;
            }
        }
        match term {
            QTerm::Quote { term, cmds, .. } | QTerm::Unquote { term, cmds, .. } => {
                self.cmds(cmds, &mut std::iter::once(&**term));
            }
            QTerm::Tuple { terms, cmds, .. } => {
                self.cmds(cmds, &mut terms.iter().map(|t| &**t));
            }
        }
    }

    fn cmds(&mut self, cmds: &[CmdOrHole], children: &mut dyn Iterator<Item = &QTerm>) {
        for cmd in cmds {
            match cmd {
                CmdOrHole::Cmd(StrCmd::Write(s)) => self.buf.push_str(s),
                CmdOrHole::Cmd(StrCmd::NewLine) => {
                    self.buf.push('\n');
                    for prefix in &self.stack {
                        self.buf.push_str(prefix);
                    }
                }
                CmdOrHole::Cmd(StrCmd::Push(s)) => self.stack.push(s.to_string()),
                CmdOrHole::Cmd(StrCmd::Pop) => {
                    self.stack.pop();
                }
                CmdOrHole::Hole => {
                    self.term(children.next().expect("flatten: not enough children"));
                }
            }
        }
    }
}

fn flatten(pattern: &QTerm) -> Vec<Seg> {
    let mut f = Flattener::default();
    f.term(pattern);
    f.flush();
    f.segs
}

/// A matched metavariable binding: a leaf that writes the text back verbatim.
fn bind(text: &str) -> Arc<QTerm> {
    leaf("text", text)
}

/**************************************************************/

/// Match `term` against `pattern`, returning the metavariable bindings in
/// the order the metavariables appear in the pattern. See the module docs
/// for the matching semantics.
pub fn qmatch(pattern: &QTerm, term: &QTerm) -> Result<Vec<Arc<QTerm>>> {
    let segs = flatten(pattern);
    let text = term.coparse();

    let mut binds = Vec::new();
    let mut pos = 0;
    let mut i = 0;
    while i < segs.len() {
        match &segs[i] {
            Seg::Lit(lit) => {
                ensure!(
                    text[pos..].starts_with(lit),
                    "qmatch: expected {lit:?} at {:?}",
                    &text[pos..]
                );
                pos += lit.len();
                i += 1;
            }
            Seg::Var(name) => match segs.get(i + 1) {
                // a trailing metavariable takes the rest
                None => {
                    binds.push(bind(&text[pos..]));
                    pos = text.len();
                    i += 1;
                }
                Some(Seg::Lit(lit)) => {
                    // the final literal is anchored at the end; the others
                    // bind leftmost-shortest
                    let end = if i + 2 == segs.len() {
                        text[pos..]
                            .ends_with(&**lit)
                            .then(|| text.len() - lit.len())
                    } else {
                        text[pos..].find(&**lit).map(|d| pos + d)
                    };
                    let end = end.ok_or_else(|| {
                        miette!(
                            "qmatch: metavariable {name} unmatched: expected {lit:?} after {:?}",
                            &text[pos..]
                        )
                    })?;
                    binds.push(bind(&text[pos..end]));
                    pos = end + lit.len();
                    i += 2;
                }
                Some(Seg::Var(next)) => {
                    bail!("qmatch: adjacent metavariables {name} and {next} are ambiguous")
                }
            },
        }
    }
    ensure!(
        pos == text.len(),
        "qmatch: trailing text {:?} after pattern",
        &text[pos..]
    );
    Ok(binds)
}

/// [`qmatch`] with the binding count checked against the destructuring arity:
/// this is what `let ↖pattern↗ = value;` expands to a call of. Panics on a
/// mismatch — the expanded `let` is irrefutable, so there is no place for the
/// error to go.
pub fn qmatch_n<const N: usize>(pattern: &QTerm, term: &QTerm) -> [Arc<QTerm>; N] {
    let binds = qmatch(pattern, term).unwrap_or_else(|e| panic!("qmatch_n: {e}"));
    binds.try_into().unwrap_or_else(|binds: Vec<_>| {
        panic!(
            "qmatch_n: pattern bound {} metavariables, destructuring expects {N}",
            binds.len()
        )
    })
}

/**************************************************************/

// Codegen: the Rust spellings of the runtime above, shared by the Rust and
// Bootstrap meta-languages (see `MetaLanguage::pattern_var`/`pattern_let`).

/// Code for a pattern metavariable splice: `mvar("name")`.
pub fn pattern_var_code(name: &str) -> Arc<QTerm> {
    leaf("_", &format!("mvar(\"{name}\")"))
}

/// The two terms a pattern-let rewrites to: the destructuring binder
/// `[a, b]` that replaces the pattern quote, and the matching call
/// `qmatch_n(&<pattern>, &<value>)` that replaces the initializer.
pub fn pattern_let_code(
    names: &[Box<str>],
    pattern: &Arc<QTerm>,
    value: &Arc<QTerm>,
) -> (Arc<QTerm>, Arc<QTerm>) {
    let binder = leaf("_", &format!("[{}]", names.join(", ")));
    let call = tb("_")
        .w("qmatch_n(&")
        .c(pattern)
        .w(", &")
        .c(value)
        .w(")")
        .b();
    (binder, call)
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact() -> Result<()> {
        let binds = qmatch(&leaf("a", "foo"), &leaf("b", "foo"))?;
        assert!(binds.is_empty());
        assert!(qmatch(&leaf("a", "foo"), &leaf("b", "bar")).is_err());
        Ok(())
    }

    #[test]
    fn single_var() -> Result<()> {
        let pat = tb("_").w("a + ").c(&mvar("x")).b();
        let binds = qmatch(&pat, &leaf("_", "a + b * c"))?;
        assert_eq!(binds, vec![bind("b * c")]);
        Ok(())
    }

    #[test]
    fn two_vars() -> Result<()> {
        let pat = tb("_")
            .w("f(")
            .c(&mvar("x"))
            .w(", ")
            .c(&mvar("y"))
            .w(")")
            .b();
        let binds = qmatch(&pat, &leaf("_", "f(1, g(2, 3))"))?;
        assert_eq!(binds, vec![bind("1"), bind("g(2, 3)")]);
        Ok(())
    }

    #[test]
    fn end_anchored() -> Result<()> {
        // the final literal must reach the end: `)` matches the outer paren
        let pat = tb("_").w("(").c(&mvar("x")).w(")").b();
        let binds = qmatch(&pat, &leaf("_", "(f(x))"))?;
        assert_eq!(binds, vec![bind("f(x)")]);
        // ...and trailing text the pattern doesn't cover is a mismatch
        assert!(qmatch(&pat, &leaf("_", "(f(x)) ")).is_err());
        Ok(())
    }

    #[test]
    fn empty_binding() -> Result<()> {
        let pat = tb("_").w("f(").c(&mvar("x")).w(")").b();
        let binds = qmatch(&pat, &leaf("_", "f()"))?;
        assert_eq!(binds, vec![bind("")]);
        Ok(())
    }

    #[test]
    fn adjacent_vars_ambiguous() {
        let pat = tb("_").w("f(").c(&mvar("x")).c(&mvar("y")).w(")").b();
        assert!(qmatch(&pat, &leaf("_", "f(12)")).is_err());
    }

    #[test]
    fn structured_subject() -> Result<()> {
        // the subject's tree shape is irrelevant: only its coparse text counts
        let subject = tb("binary_expression")
            .c(&leaf("integer_literal", "1"))
            .w(" ")
            .c(&sym("+"))
            .w(" ")
            .c(&leaf("integer_literal", "2"))
            .b();
        let pat = tb("_").w("1 + ").c(&mvar("x")).b();
        let binds = qmatch(&pat, &subject)?;
        assert_eq!(binds, vec![bind("2")]);
        Ok(())
    }

    #[test]
    fn multiline_prefix() -> Result<()> {
        // newlines respect the prefix stack on both sides, as in coparse
        let pat = tb("_")
            .w("{")
            .p("    ")
            .n()
            .c(&mvar("x"))
            .x()
            .n()
            .w("}")
            .b();
        let subject = tb("_").w("{").p("    ").n().w("body();").x().n().w("}").b();
        let binds = qmatch(&pat, &subject)?;
        assert_eq!(binds, vec![bind("body();")]);
        Ok(())
    }

    #[test]
    fn qmatch_n_arity() {
        let pat = tb("_").w("f(").c(&mvar("x")).w(")").b();
        let [x] = qmatch_n(&pat, &leaf("_", "f(1)"));
        assert_eq!(x, bind("1"));
    }

    #[test]
    #[should_panic(expected = "destructuring expects 2")]
    fn qmatch_n_arity_mismatch() {
        let pat = tb("_").w("f(").c(&mvar("x")).w(")").b();
        let [_, _] = qmatch_n(&pat, &leaf("_", "f(1)"));
    }

    #[test]
    fn mvar_coparses_for_debugging() {
        assert_eq!(mvar("x").coparse(), "↙x↘");
    }
}
