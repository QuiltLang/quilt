//! The builder-call fold shared by the Rust, Python and TypeScript metas.
//!
//! All three metas answer the same question — *what source rebuilds this
//! `QTerm`?* — and all three answered it with their own copy of the same fold
//! over `cmds`, assembling a method chain out of string fragments:
//!
//! ```text
//! CmdOrHole::Cmd(StrCmd::Write(s)) => b.write(&format!(".w({})", str_lit(s)))
//! CmdOrHole::Cmd(StrCmd::NewLine)  => b.write(".n()")
//! CmdOrHole::Hole                  => b.write(".c(&"); b.child(..); b.write(")")
//! ```
//!
//! The fold is the same in every language; only the *fragments* differ, and
//! only in small ways — `.c(&x)` against `.c(x)`, `&[..]` against `[..]`, `NL`
//! against `NL()`. Issue #204: those are three facts a maintainer had to
//! remember in three files, and the fold around them was cloned four times
//! (five, counting `bootstrap::strlift`, which is deliberately left alone — see
//! below).
//!
//! So the fold lives here, once, and the fragments are [`Shapes`] — a table
//! **generated** by `bin/gen-chain` from samples quoted in each target
//! language and parsed with that language's own grammar. `.c(&x)` is not
//! Python because `py↖ACC.c(&LIT)↗` does not parse, and `&[..]` is not Python
//! for the same reason; the generator fails rather than emitting them. What a
//! sample cannot settle is a fact about the *runtime* rather than the syntax —
//! `NL` and `NL()` both parse as TypeScript — so that divergence is still a
//! declared one, just declared once (see `quilt-wasm`'s README and #167).
//!
//! Only *text* is contractual here. The emitted terms are flat, `"_"`-tagged
//! layout — dump-only, serialized with `coparse` and never matched against —
//! so this fold is free to group its writes differently from the four hand-
//! written folds it replaced, and does. The expander snapshots
//! (`quilt/tests/snapshots/`) pin the text, and `expand_both` in
//! `quilt/tests/expand_rust.rs` re-derives it independently through
//! `BootstrapMetaLanguage`, which is *not* built on this fold.
//!
//! ## What is deliberately not here
//!
//! * **Rust's variadic block.** Python and TypeScript splice into a variadic
//!   container with a fluent `.e(..)` chain, so they share [`Chain::variadic_code`]
//!   with a different hole fragment. Rust emits an imperative
//!   `{ let mut b_ = ..; ..; b_.b() }` block instead — not a chain at all, and
//!   the only shape that supports statement-context splicing. It stays
//!   hand-written in `langs::rust::ops`, in one copy.
//! * **String escaping.** Which characters a literal must escape is runtime
//!   logic, not a shape a sample can carry, so each meta still supplies its own
//!   escaper through [`Lit`].
//! * **`bootstrap::strlift`.** It folds to `String` rather than to `QTerm` and
//!   exists to bootstrap `meta.rs` without a `MetaLanguage`. Keeping it
//!   independent is what makes `expand_both` an oracle rather than a tautology.

mod gen;

pub use gen::{PYTHON, RUST, TYPESCRIPT};

use crate::prelude::*;
use crate::qterm::QTermBuilder;
use crate::term::CmdOrHole;

/**************************************************************/

/// The target-language spelling of every builder-call fragment the fold
/// splices together.
///
/// Each field is the source text *around* the arguments of one call, so an
/// n-argument shape is n+1 pieces: `tb: ["tb(", ")"]` renders `tb(TAG)`, and
/// `leaf: ["leaf(", ", ", ")"]` renders `leaf(TAG, CODE)`. A shape with no
/// arguments is a single piece and is spelled as a plain `&str`.
///
/// Generated — see `mk_chain.rs.quilt` for the samples each field is read off.
pub struct Shapes {
    /// `tb(TAG)` — opens a builder chain.
    pub tb: [&'static str; 2],
    /// `.w(LIT)` — a write command.
    pub w: [&'static str; 2],
    /// `.n()` — a newline command.
    pub n: &'static str,
    /// `.p(LIT)` — a push-prefix command.
    pub p: [&'static str; 2],
    /// `.x()` — a pop-prefix command.
    pub x: &'static str,
    /// `.c(CHILD)` — splice a child. The one fragment that carries Rust's
    /// borrow (`.c(&x)`) and the two dynamic languages' plain `.c(x)`.
    pub c: [&'static str; 2],
    /// `.e(CHILD)` — emit into a variadic container (Python and TypeScript).
    pub e: [&'static str; 2],
    /// `.b()` — closes a builder chain.
    pub b: &'static str,
    /// `sym(LIT)` — shorthand for a childless node whose text is its own tag.
    pub sym: [&'static str; 2],
    /// `leaf(TAG, CODE)` — shorthand for any other childless single-write node.
    pub leaf: [&'static str; 3],
    /// `quote(TAG, INDEX, LANG, TERM, CMDS)`.
    pub quote: [&'static str; 6],
    /// `unquote(TAG, INDEX, LANG, TERM, CMDS)`.
    pub unquote: [&'static str; 6],
    /// A cmds list — `[A, B]`, or `&[A, B]` in Rust. Pieces are open,
    /// separator, close; the borrow, where there is one, belongs to the open.
    pub list: [&'static str; 3],
    /// `cmd(X)` — wraps one `StrCmd` for the list.
    pub cmd: [&'static str; 2],
    /// `write(LIT)`.
    pub write: [&'static str; 2],
    /// `push(LIT)`.
    pub push: [&'static str; 2],
    /// The newline command as a value: `NL`, or `NL()` where the runtime cannot
    /// export a constant.
    pub nl: &'static str,
    /// The pop command as a value: `POP` / `POP()`.
    pub pop: &'static str,
    /// A hole in a cmds list: `HOLE` / `HOLE()`.
    pub hole: &'static str,
}

/// How a string literal is rendered into the emitted source.
///
/// The two modes differ in what the caller can do with the result, not in what
/// it serializes to.
#[derive(Clone, Copy)]
pub enum Lit {
    /// As source text, escaped by the given function. The dump-only path that
    /// the three `build_*_code` helpers take.
    Flat(fn(&str) -> String),
    /// As a structured literal subterm, so the emitted code can be matched and
    /// rewritten as AST afterwards. Rust's `qlift` takes this path, which is
    /// what `rewrite_naive` relies on.
    Term(fn(&str) -> Arc<QTerm>),
}

/// A [`Shapes`] table paired with a way to render string literals: everything
/// the fold needs to emit one language's builder calls.
#[derive(Clone, Copy)]
pub struct Chain {
    shapes: &'static Shapes,
    lit: Lit,
}

impl Chain {
    #[must_use]
    pub const fn new(shapes: &'static Shapes, lit: Lit) -> Self {
        Self { shapes, lit }
    }

    /// Append a string literal in whichever form this chain was built for.
    fn lit(&self, b: &mut QTermBuilder, s: &str) {
        match self.lit {
            Lit::Flat(esc) => b.write(&esc(s)),
            Lit::Term(term) => b.child(&term(s)),
        };
    }

    /// Append `f[0] LIT f[1]`.
    fn call1(&self, b: &mut QTermBuilder, f: [&'static str; 2], s: &str) {
        b.write(f[0]);
        self.lit(b, s);
        b.write(f[1]);
    }

    /// Append `f[0] CHILD f[1]`.
    fn splice(b: &mut QTermBuilder, f: [&'static str; 2], child: &Arc<QTerm>) {
        b.write(f[0]).child(child).write(f[1]);
    }

    /// `sym(TAG)` / `leaf(TAG, CODE)` — the shorthand a childless node with a
    /// single write collapses to, or `None` if this node is not one.
    fn shorthand(
        &self,
        tag: &str,
        cmds: &[CmdOrHole],
        children: &[Arc<QTerm>],
    ) -> Option<Arc<QTerm>> {
        if !children.is_empty() || cmds.len() != 1 {
            return None;
        }
        let CmdOrHole::Cmd(StrCmd::Write(code)) = &cmds[0] else {
            return None;
        };
        let mut b = tb("_");
        if tag == &**code {
            self.call1(&mut b, self.shapes.sym, tag);
        } else {
            let f = self.shapes.leaf;
            b.write(f[0]);
            self.lit(&mut b, tag);
            b.write(f[1]);
            self.lit(&mut b, code);
            b.write(f[2]);
        }
        Some(b.b())
    }

    /// The fold itself: `tb(tag)` followed by one fragment per cmd, closed with
    /// `.b()`. `hole` is the fragment each child is spliced with — `.c(..)` for
    /// an ordinary node, `.e(..)` for a variadic one.
    fn chain(
        &self,
        tag: &str,
        cmds: &[CmdOrHole],
        children: &[Arc<QTerm>],
        hole: [&'static str; 2],
        who: &str,
    ) -> Arc<QTerm> {
        if let Some(short) = self.shorthand(tag, cmds, children) {
            return short;
        }
        let s = self.shapes;
        let mut b = tb("_");
        self.call1(&mut b, s.tb, tag);
        let mut it = children.iter();
        for c in cmds {
            match c {
                CmdOrHole::Cmd(StrCmd::Write(t)) => self.call1(&mut b, s.w, t),
                CmdOrHole::Cmd(StrCmd::NewLine) => {
                    b.write(s.n);
                }
                CmdOrHole::Cmd(StrCmd::Push(t)) => self.call1(&mut b, s.p, t),
                CmdOrHole::Cmd(StrCmd::Pop) => {
                    b.write(s.x);
                }
                CmdOrHole::Hole => {
                    let child = it
                        .next()
                        .unwrap_or_else(|| panic!("{who}: not enough children"));
                    Self::splice(&mut b, hole, child);
                }
            }
        }
        b.write(s.b);
        b.b()
    }

    /// Code that reconstructs a tuple: `tb(tag).w(..).c(child)..b()`, using the
    /// `sym`/`leaf` shorthands when possible. `children` are the already-built
    /// child expressions, spliced at hole positions.
    #[must_use]
    pub fn tuple_code(&self, tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
        self.chain(tag, cmds, children, self.shapes.c, "build_tuple_code")
    }

    /// Code that reconstructs a variadic node as a fluent emit chain:
    /// `tb(tag).e(child)..b()`. Each child is emitted with `.e(..)`, which —
    /// like Rust's `.emit(&mut b_)` — appends one *or more* children.
    #[must_use]
    pub fn variadic_code(
        &self,
        tag: &str,
        cmds: &[CmdOrHole],
        children: &[Arc<QTerm>],
    ) -> Arc<QTerm> {
        self.chain(tag, cmds, children, self.shapes.e, "build_variadic_block")
    }

    /// A cmds list: `[cmd(write(..)), HOLE, ..]`.
    #[must_use]
    pub fn cmds_code(&self, cmds: &[CmdOrHole]) -> Arc<QTerm> {
        let s = self.shapes;
        let mut b = tb("_");
        b.write(s.list[0]);
        for (i, c) in cmds.iter().enumerate() {
            if i > 0 {
                b.write(s.list[1]);
            }
            match c {
                CmdOrHole::Hole => {
                    b.write(s.hole);
                }
                CmdOrHole::Cmd(cmd) => {
                    b.write(s.cmd[0]);
                    match cmd {
                        StrCmd::Write(t) => self.call1(&mut b, s.write, t),
                        StrCmd::NewLine => {
                            b.write(s.nl);
                        }
                        StrCmd::Push(t) => self.call1(&mut b, s.push, t),
                        StrCmd::Pop => {
                            b.write(s.pop);
                        }
                    }
                    b.write(s.cmd[1]);
                }
            }
        }
        b.write(s.list[2]);
        b.b()
    }

    /// `quote(tag, index, lang, <term>, [..cmds..])`, splicing `term`.
    #[must_use]
    pub fn quote_code(
        &self,
        tag: &str,
        index: Index,
        lang: &str,
        term: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Arc<QTerm> {
        self.wrapper(self.shapes.quote, tag, index, lang, term, cmds)
    }

    /// `unquote(tag, index, lang, <term>, [..cmds..])`, splicing `term`.
    #[must_use]
    pub fn unquote_code(
        &self,
        tag: &str,
        index: Index,
        lang: &str,
        term: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Arc<QTerm> {
        self.wrapper(self.shapes.unquote, tag, index, lang, term, cmds)
    }

    /// The shared body of [`Self::quote_code`] and [`Self::unquote_code`]: the
    /// two differ only in the name they open with.
    fn wrapper(
        &self,
        f: [&'static str; 6],
        tag: &str,
        index: Index,
        lang: &str,
        term: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Arc<QTerm> {
        let mut b = tb("_");
        b.write(f[0]);
        self.lit(&mut b, tag);
        b.write(&format!("{}{index}{}", f[1], f[2]));
        self.lit(&mut b, lang);
        b.write(f[3]);
        b.child(term);
        b.write(f[4]);
        b.child(&self.cmds_code(cmds));
        b.write(f[5]);
        b.b()
    }
}
