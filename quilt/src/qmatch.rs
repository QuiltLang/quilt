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
//! ## Structural pattern matching ([`smatch`] / [`sinstantiate`] / [`QTerm::rewrite`])
//!
//! A separate, tree-level pattern language sits alongside the textual one.
//! Patterns are ordinary [`QTerm`]s except that [`smvar`] nodes act as
//! wildcards that match **any** single subtree and bind it by name.  Matching
//! is purely structural — the tree shapes must agree at every non-wildcard
//! position — and the cmds (layout) of a node are not compared.  See
//! [`smatch`] for the full semantics.
//!
//! Like `lift.rs`, this module is part of the parser-free runtime that
//! expanded code links against, plus the codegen helpers ([`pattern_var_code`],
//! [`pattern_let_code`]) the meta-languages use to spell calls into it.

use crate::prelude::*;
use crate::qterm::{qquote_at, qunquote_at, QTerm};
use crate::strcmd::StrCmd;
use crate::term::{CmdOrHole, STerm};
use miette::{bail, ensure};
use std::collections::HashMap;
use std::hash::BuildHasher;

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
// Structural pattern matching

/// The tag marking a structural metavariable (see [`smvar`]).
pub const SMVAR: &str = "smvar";

/// A structural metavariable: matches any single [`QTerm`] subtree and binds
/// it to `name`. Coparses to `?name` for debugging.
pub fn smvar(name: &str) -> Arc<QTerm> {
    unquote(SMVAR, 0, "", sym(name), &[cmd(write("?")), HOLE])
}

/// Try to merge `from` into `into`, failing if the same name is bound to two
/// structurally unequal terms.
fn merge_bindings(
    into: &mut HashMap<Box<str>, Arc<QTerm>>,
    from: HashMap<Box<str>, Arc<QTerm>>,
) -> Option<()> {
    for (k, v) in from {
        if let Some(existing) = into.get(&k) {
            if existing.as_ref() != v.as_ref() {
                return None;
            }
        } else {
            into.insert(k, v);
        }
    }
    Some(())
}

/// Extract `(tag, index, lang, inner_term)` from a Quote or Unquote node,
/// together with a flag that is `true` for Quote and `false` for Unquote.
/// Returns `None` for Tuple.
fn quote_fields(t: &QTerm) -> Option<(&str, crate::util::Index, &str, &QTerm, bool)> {
    match t {
        QTerm::Quote {
            tag,
            index,
            lang,
            term,
            ..
        } => Some((tag, *index, lang, term, true)),
        QTerm::Unquote {
            tag,
            index,
            lang,
            term,
            ..
        } => Some((tag, *index, lang, term, false)),
        QTerm::Tuple { .. } => None,
    }
}

/// Match `subject` against `pattern` structurally, returning a map from
/// metavariable name to bound subtree on success, or `None` on mismatch.
///
/// - An [`smvar`] node in the pattern matches any single [`QTerm`] and binds
///   it by name. If the same name appears more than once in the pattern, all
///   occurrences must match structurally equal subtrees.
/// - Everything else requires the same variant, tag, index, and lang; children
///   are matched pairwise and recursively.
/// - The `cmds` (layout instructions) of non-metavariable nodes are **not**
///   compared, so a constructed term and a parsed one that differ only in
///   whitespace still match.
pub fn smatch(pattern: &QTerm, subject: &QTerm) -> Option<HashMap<Box<str>, Arc<QTerm>>> {
    if let QTerm::Unquote { tag, term, .. } = pattern {
        if &**tag == SMVAR {
            let name: Box<str> = term.coparse().into_boxed_str();
            return Some(HashMap::from([(name, Arc::new(subject.clone()))]));
        }
    }
    if let (
        QTerm::Tuple {
            tag: pt,
            terms: pterms,
            ..
        },
        QTerm::Tuple {
            tag: st,
            terms: sterms,
            ..
        },
    ) = (pattern, subject)
    {
        if pt != st || pterms.len() != sterms.len() {
            return None;
        }
        let mut bindings = HashMap::new();
        for (pc, sc) in pterms.iter().zip(sterms.iter()) {
            let sub = smatch(pc, sc)?;
            merge_bindings(&mut bindings, sub)?;
        }
        return Some(bindings);
    }
    let (pt, pi, pl, pterm, pq) = quote_fields(pattern)?;
    let (st, si, sl, sterm, sq) = quote_fields(subject)?;
    if pq != sq || pt != st || pi != si || pl != sl {
        return None;
    }
    smatch(pterm, sterm)
}

/// Substitute [`smvar`] markers in `template` with their bound values from
/// `bindings`. Variables absent from `bindings` are left in place.
pub fn sinstantiate<S: BuildHasher>(
    template: &QTerm,
    bindings: &HashMap<Box<str>, Arc<QTerm>, S>,
) -> Arc<QTerm> {
    if let QTerm::Unquote { tag, term, .. } = template {
        if &**tag == SMVAR {
            let name: Box<str> = term.coparse().into_boxed_str();
            if let Some(bound) = bindings.get(&name) {
                return bound.clone();
            }
        }
    }
    match template {
        QTerm::Quote {
            tag,
            index,
            lang,
            term,
            cmds,
            span,
        } => {
            let term = sinstantiate(term, bindings);
            Arc::new(qquote_at(tag, *index, lang, term, cmds, span.clone()))
        }
        QTerm::Unquote {
            tag,
            index,
            lang,
            term,
            cmds,
            span,
        } => {
            let term = sinstantiate(term, bindings);
            Arc::new(qunquote_at(tag, *index, lang, term, cmds, span.clone()))
        }
        QTerm::Tuple { tag, terms, cmds } => {
            let terms: Vec<Arc<QTerm>> = terms.iter().map(|t| sinstantiate(t, bindings)).collect();
            tuple(tag, &terms, cmds)
        }
    }
}

impl QTerm {
    /// Rewrite every subtree that matches the `find` pattern, replacing it with
    /// `replace` (with metavariable bindings substituted in). Traversal is
    /// outermost-first: if a node matches the pattern, its children are **not**
    /// recursed into — the replacement is returned as-is. This differs from
    /// [`rewrite_naive`][QTerm::rewrite_naive], which uses structural equality
    /// and recurses unconditionally.
    pub fn rewrite(&self, find: &Self, replace: &Self) -> Arc<Self> {
        if let Some(bindings) = smatch(find, self) {
            return sinstantiate(replace, &bindings);
        }
        match self {
            QTerm::Quote {
                tag,
                index,
                lang,
                term,
                cmds,
                span,
            } => {
                let term = term.rewrite(find, replace);
                Arc::new(qquote_at(tag, *index, lang, term, cmds, span.clone()))
            }
            QTerm::Unquote {
                tag,
                index,
                lang,
                term,
                cmds,
                span,
            } => {
                let term = term.rewrite(find, replace);
                Arc::new(qunquote_at(tag, *index, lang, term, cmds, span.clone()))
            }
            QTerm::Tuple { tag, terms, cmds } => {
                let terms: Vec<Arc<QTerm>> =
                    terms.iter().map(|t| t.rewrite(find, replace)).collect();
                tuple(tag, &terms, cmds)
            }
        }
    }
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

    // --- structural pattern matching ---

    #[test]
    fn smvar_coparses_for_debugging() {
        assert_eq!(smvar("x").coparse(), "?x");
    }

    #[test]
    fn smatch_exact_leaf() {
        let t = leaf("k", "hello");
        let binds = smatch(&t, &t).unwrap();
        assert!(binds.is_empty());
    }

    #[test]
    fn smatch_tag_mismatch() {
        assert!(smatch(&leaf("a", "x"), &leaf("b", "x")).is_none());
    }

    #[test]
    fn smatch_single_metavar() {
        let pat = smvar("x");
        let subject = leaf("integer_literal", "42");
        let binds = smatch(&pat, &subject).unwrap();
        assert_eq!(binds.len(), 1);
        assert_eq!(binds["x"].as_ref(), subject.as_ref());
    }

    #[test]
    fn smatch_metavar_in_tuple() {
        // pattern: add(?x, ?y)  subject: add(1, 2)
        let pat = tb("add").c(&smvar("x")).c(&smvar("y")).b();
        let subject = tb("add").c(&leaf("int", "1")).c(&leaf("int", "2")).b();
        let binds = smatch(&pat, &subject).unwrap();
        assert_eq!(binds["x"].as_ref(), leaf("int", "1").as_ref());
        assert_eq!(binds["y"].as_ref(), leaf("int", "2").as_ref());
    }

    #[test]
    fn smatch_arity_mismatch() {
        let pat = tb("f").c(&smvar("x")).b();
        let subject = tb("f").c(&leaf("a", "1")).c(&leaf("b", "2")).b();
        assert!(smatch(&pat, &subject).is_none());
    }

    #[test]
    fn smatch_repeated_var_consistent() {
        // pattern: f(?x, ?x) — both positions must match the same subtree
        let x = smvar("x");
        let pat = tb("f").c(&x).c(&x).b();
        let same = leaf("int", "1");
        let binds = smatch(&pat, &tb("f").c(&same).c(&same).b()).unwrap();
        assert_eq!(binds["x"].as_ref(), same.as_ref());
    }

    #[test]
    fn smatch_repeated_var_conflict() {
        let x = smvar("x");
        let pat = tb("f").c(&x).c(&x).b();
        let subject = tb("f").c(&leaf("int", "1")).c(&leaf("int", "2")).b();
        assert!(smatch(&pat, &subject).is_none());
    }

    #[test]
    fn smatch_cmds_ignored() {
        // Two tuples with the same tag/children but different layout (cmds)
        // should still match: cmds are not compared.
        let pat = tb("x").w("  ").c(&smvar("v")).b();
        let subject = tb("x").c(&leaf("n", "1")).b();
        let binds = smatch(&pat, &subject).unwrap();
        assert_eq!(binds["v"].as_ref(), leaf("n", "1").as_ref());
    }

    #[test]
    fn sinstantiate_replaces_var() {
        let tmpl = tb("add").c(&smvar("x")).c(&smvar("y")).b();
        let bindings: HashMap<Box<str>, Arc<QTerm>> = HashMap::from([
            ("x".into(), leaf("int", "10")),
            ("y".into(), leaf("int", "20")),
        ]);
        let result = sinstantiate(&tmpl, &bindings);
        let expected = tb("add").c(&leaf("int", "10")).c(&leaf("int", "20")).b();
        assert_eq!(result.as_ref(), expected.as_ref());
    }

    #[test]
    fn sinstantiate_missing_var_left_in_place() {
        // A var not in bindings should remain as an smvar node
        let tmpl = smvar("x");
        let bindings: HashMap<Box<str>, Arc<QTerm>> = HashMap::new();
        let result = sinstantiate(&tmpl, &bindings);
        assert_eq!(result.as_ref(), smvar("x").as_ref());
    }

    #[test]
    fn rewrite_no_match() {
        let find = tb("f").c(&leaf("a", "1")).b();
        let replace = leaf("b", "2");
        let tree = tb("g").c(&leaf("c", "3")).b();
        let result = tree.rewrite(&find, &replace);
        assert_eq!(result.as_ref(), tree.as_ref());
    }

    #[test]
    fn rewrite_root_match() {
        let x = smvar("x");
        let find = tb("neg").c(&x).b();
        let replace = tb("neg").c(&tb("neg").c(&smvar("x")).b()).b();
        let subject = tb("neg").c(&leaf("int", "5")).b();
        let result = subject.rewrite(&find, &replace);
        let expected = tb("neg").c(&tb("neg").c(&leaf("int", "5")).b()).b();
        assert_eq!(result.as_ref(), expected.as_ref());
    }

    #[test]
    fn rewrite_deep_match() {
        // replace every `zero` leaf with `0`
        let find = sym("zero");
        let replace = sym("0");
        let tree = tb("add")
            .c(&sym("zero"))
            .c(&tb("mul").c(&sym("zero")).c(&leaf("int", "3")).b())
            .b();
        let result = tree.rewrite(&find, &replace);
        let expected = tb("add")
            .c(&sym("0"))
            .c(&tb("mul").c(&sym("0")).c(&leaf("int", "3")).b())
            .b();
        assert_eq!(result.as_ref(), expected.as_ref());
    }

    #[test]
    fn rewrite_outermost_first() {
        // A match at the root suppresses recursion into children.
        // find = f(?x),  replace = ?x  (strip one level of `f`)
        // subject = f(f(a))  → should give f(a), not a
        let find = tb("f").c(&smvar("x")).b();
        let replace = smvar("x");
        let subject = tb("f").c(&tb("f").c(&leaf("a", "a")).b()).b();
        let result = subject.rewrite(&find, &replace);
        let expected = tb("f").c(&leaf("a", "a")).b();
        assert_eq!(result.as_ref(), expected.as_ref());
    }
}
