use crate::lang::Arity;
use crate::prelude::*;
use crate::qterm::QTerm;
use crate::term::CmdOrHole;
use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::Arc;

/**************************************************************/

pub const LIFT: &str = "__LIFT__";
pub const REDUCE: &str = "__REDUCE__";
pub const EMIT: &str = "__EMIT__";
pub const TYPE: &str = "__TYPE__";
pub const NAME: &str = "__NAME__";

/// The body every host's [`MetaLanguage::spawn_str`] shares: refuse a
/// language no provider backs, then let the host spell the rest.
///
/// The refusal is here rather than in generated code so that `wgsl⟨M⟩` is a
/// diagnostic at expansion time — pointing at the glyph in the `.quilt`
/// source, where the mistake is — instead of a failure inside an artifact
/// whose reader has no idea a glyph produced it.
pub fn spawn_spelling(
    host: &str,
    lang: &str,
    spell: impl FnOnce(&str) -> String,
) -> Result<String> {
    if !crate::machine::has_spec(lang) {
        miette::bail!(
            "no machine is registered for {lang:?}, so `{lang}⟨M⟩` gives {host} nothing to \
             spawn; quilt knows: {}",
            crate::machine::REGISTERED_SPECS.join(", ")
        );
    }
    Ok(spell(lang))
}

/// The runtime import an expanded file must open with — `use quilt::prelude::*;`
/// for Rust, `from quilt import *` for Python — together with how to tell that
/// the author already wrote it by hand.
///
/// It exists because expanded code *calls a runtime*: `tb(..)`, `qlift(..)`,
/// `name(..)` are names that have to be in scope, and until now every author of
/// a `.quilt` file typed the import that brings them there. See
/// [`MetaLanguage::prelude`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prelude {
    /// The text to inject, with no trailing newline. May be several lines.
    pub text: Cow<'static, str>,
    /// Substrings whose presence in the *source* means the runtime is already
    /// imported by hand; any one match suppresses injection. A doubled glob
    /// import is harmless in Rust and Python, but a doubled *named* import is a
    /// hard error in TypeScript, so this is not merely tidiness.
    ///
    /// They are matched against the source rather than the expansion because a
    /// ground-stage import is copied through verbatim, and the source is in
    /// hand before anything is written.
    pub markers: Vec<Cow<'static, str>>,
}

impl Prelude {
    pub fn new(
        text: impl Into<Cow<'static, str>>,
        markers: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Prelude {
            text: text.into(),
            markers: markers.into_iter().map(Cow::Borrowed).collect(),
        }
    }

    /// Whether `src` already imports this runtime, so injecting would duplicate
    /// it.
    #[must_use]
    pub fn present_in(&self, src: &str) -> bool {
        self.markers.iter().any(|m| src.contains(&**m))
    }
}

/// `body` — code generated for `lang` — opened with `prelude`, placed after
/// whatever has to stay at the top (see [`prelude_offset`]). The injection
/// [`Multi::prelude_for`](crate::multi::Multi::prelude_for) decides on, issue
/// #274.
///
/// `lang` names the language (any alias) whose line comments make up the
/// header the prelude goes after; `//` for one the registry does not know.
/// Needs `parse`, which is what the language registry it asks lives behind.
#[cfg(feature = "parse")]
#[must_use]
pub fn with_prelude(body: &str, prelude: &str, lang: &str) -> String {
    let line_comment = crate::langs::line_comment(lang).unwrap_or("//");
    let at = prelude_offset(body, line_comment);
    let (before, after) = body.split_at(at);
    if after.is_empty() {
        format!("{before}{prelude}\n")
    } else {
        format!("{before}{prelude}\n\n{after}")
    }
}

/// The byte offset in `body` at which a runtime import can be inserted: after
/// the leading header — blank lines, whole-line comments, a shebang and Rust's
/// inner attributes — and before the first item.
///
/// Not cosmetic. Rust's `//!` doc comments and `#![…]` attributes are *inner*,
/// and inner means "before any item", so an import written above a file's `//!`
/// header turns the header itself into a syntax error (`expected outer doc
/// comment`) — which is what nine of this repo's own examples do. Python and
/// TypeScript are laxer, but a header is what a reader expects first there too,
/// and a `#!` shebang is only a shebang on line 1.
///
/// A line comment is recognised by `line_comment`, the ground language's own
/// introducer, so `//!` and `//` both count for Rust and `#`-anything for
/// Python (its shebang included).
#[cfg(feature = "parse")]
fn prelude_offset(body: &str, line_comment: &str) -> usize {
    let mut at = 0;
    let mut rest = body;
    while !rest.is_empty() {
        let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let line = rest[..line_end].trim_end();
        let take = if line.is_empty() || line.starts_with(line_comment) {
            line_end
        } else if line.starts_with("#![") {
            // An inner attribute, which may wrap across lines: take it whole,
            // by balancing its brackets. Splitting one is worse than placing
            // the import above it.
            match attribute_end(rest) {
                Some(end) => end,
                None => break,
            }
        } else if at == 0 && line.starts_with("#!") {
            line_end // a shebang, which is only one on the first line
        } else {
            break;
        };
        at += take;
        rest = &rest[take..];
    }
    at
}

/// The byte length of the `#![…]` inner attribute `s` starts with, counting to
/// the `]` that balances its `[` (so a nested `[..]` inside does not end it) and
/// through the newline after it. `None` if the brackets never balance.
#[cfg(feature = "parse")]
fn attribute_end(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    let after = i + c.len_utf8();
                    return Some(s[after..].find('\n').map_or(s.len(), |j| after + j + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// One element of a quote flattened for a non-`cst` representation (see
/// [`MetaLanguage::expand_flat`]): the quote's text with its layout commands,
/// and a hole wherever a ground unquote sits.
#[derive(Debug, Clone)]
pub enum Piece {
    /// Literal text of the quoted code (never containing a layout newline).
    Text(Box<str>),
    /// A newline, after which the pushed prefixes are written.
    NewLine,
    /// Push an indentation prefix.
    Push(Box<str>),
    /// Pop the last prefix.
    Pop,
    /// A ground unquote's value: the *expanded* host expression.
    Hole(Arc<QTerm>),
}

/// The error a host gives for a representation it does not implement.
#[must_use]
pub fn no_repr(repr: crate::qterm::Repr) -> miette::Report {
    miette!(
        "this host has no {repr} quotes yet: only `⟨cst⟩` (the default) is implemented \
         for it — TypeScript has all three"
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub enum OuterKind {
    #[default]
    None,
    Emit,
    Splice,
}

pub trait MetaLanguage {
    fn expand_quote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;
    fn expand_unquote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;
    fn expand_tuple(
        &self,
        lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity, // should we split expand_tuple based on variadic vs not?
    ) -> Result<Arc<QTerm>>;

    /// Wrap an expanded child of a tuple.
    /// * `qterm` - the child term to wrap
    /// * `okind` - an outer kind like splice/emit/none
    fn wrap_child(&self, qterm: Arc<QTerm>, _okind: OuterKind) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// Wrap an already-[wrapped](Self::wrap_child) splice into a
    /// comma-separated list (`[↙xs↘]`, `f(↙xs↘)`) so that the elements it
    /// contributes are separated by `sep` (#293): the builder's
    /// `sep(..)`/`end_sep()` region around the child.
    ///
    /// The default leaves the child alone, for a meta whose builder has no
    /// such region.
    fn wrap_separated(&self, qterm: Arc<QTerm>, _sep: &str) -> Result<Arc<QTerm>> {
        Ok(qterm)
    }

    /// The ground-tuple tag that introduces a pattern-let (e.g. Rust's
    /// `let_declaration`), or `None` if this meta-language has no pattern
    /// matching. A ground tuple with this tag whose binding position holds a
    /// quote is expanded as `let ↖pattern↗ = value;` (see
    /// `Expander::expand_pattern_let` and `crate::qmatch`).
    fn pattern_tag(&self) -> Option<&'static str> {
        None
    }

    /// Where the pattern and value sit among the children of a
    /// [`pattern_tag`](Self::pattern_tag) tuple, as `(pattern_ix, value_ix)`.
    ///
    /// The expander used to hardcode this: it scanned for a child tagged
    /// literally `"="` and took the pattern from `terms[1]`. Both assumptions are
    /// Rust's, sitting in the language-agnostic core — so a host whose binding
    /// form uses `:=` or `<-` could not have pattern-lets even after supplying a
    /// `pattern_tag`, and `let mut ↖p↗ = v` (where `mutable_specifier` shifts the
    /// pattern off index 1) silently expanded to Rust that does not compile
    /// (issue #174, finding E3).
    ///
    /// `None` means this tuple is not a pattern-let, so the expander treats it as
    /// an ordinary ground tuple. Implementations will usually delegate to
    /// [`crate::qmatch::pattern_binding_at`] with their separator token.
    fn pattern_binding(&self, terms: &[Arc<QTerm>]) -> Option<(usize, usize)> {
        let _ = terms;
        None
    }

    /// Code for a pattern metavariable: the expression spliced where `↙name↘`
    /// sits inside a pattern quote (e.g. `mvar("name")` for Rust).
    fn pattern_var(&self, name: &str) -> Result<Arc<QTerm>> {
        let _ = name;
        Err(miette!(
            "this meta-language does not support pattern matching"
        ))
    }

    /// The two terms a pattern-let rewrites to: the destructuring binder that
    /// replaces the pattern quote (e.g. `[a, b]`) and the matching call that
    /// replaces the initializer (e.g. `qmatch_n(&*<pattern>, &<value>)`).
    fn pattern_let(
        &self,
        names: &[Box<str>],
        pattern: &Arc<QTerm>,
        value: &Arc<QTerm>,
    ) -> Result<(Arc<QTerm>, Arc<QTerm>)> {
        let _ = (names, pattern, value);
        Err(miette!(
            "this meta-language does not support pattern matching"
        ))
    }

    /// The spelling `↑` expands to when lifting into the object language
    /// `target` (e.g. Rust lifting a value into a WGSL term). The homogeneous
    /// case is `target` == the meta-language's own language; the default
    /// ignores `target`, preserving that behavior for metas without
    /// heterogeneous support.
    #[inline]
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        let _ = target;
        Ok(LIFT)
    }
    /// The spelling `↓` expands to when reducing with meta-language `target`
    /// (e.g. `py↓` inside a Rust meta-program invokes Python evaluation).
    /// The homogeneous case is `target` == `""` (no annotation); the default
    /// ignores `target`, preserving existing behavior for metas without
    /// heterogeneous support.
    #[inline]
    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        let _ = target;
        Ok(REDUCE)
    }
    /// The spelling `↓` expands to in *method position* — the glyph flush
    /// against an argument list, `recv.↓(term)`: a bare method name, which
    /// the source's own parentheses complete. This is machine-directed
    /// reduce (`db.↓(schema)` evaluates a term on a machine value, issue
    /// #268), so hosts spell it as their machine-eval method.
    ///
    /// The default is an error, but unlike the other spellings it is not the
    /// end of the line: a host that returns one has not *opted into* the
    /// method-position reading, so the expander falls back to the operator
    /// spelling — the meaning a flush `(` had before this rule existed, and
    /// still the right one for `gen(7).↓(6)` in TypeScript, which reduces a
    /// generated function and calls it. Only a host that answers the
    /// un-annotated case and then refuses an annotated one gets its error
    /// surfaced.
    #[inline]
    fn reduce_method_str(&self, target: &str) -> Result<&'static str> {
        miette::bail!(
            "this meta-language has no machine-eval spelling for `{target}↓(…)` in method position"
        )
    }
    /// The spelling `←` expands to. Like [`Self::lift_str`] this returns a
    /// `Result` because not every meta-language *has* an emit: a string-based
    /// meta (nix, lean) has no `b_` accumulator to emit into, and must fail
    /// loudly rather than leak the [`EMIT`] placeholder into generated code.
    #[inline]
    fn emit_str(&self) -> Result<&'static str> {
        Ok(EMIT)
    }
    /// The spelling `↑` expands to inside an unquote of a quote with
    /// representation `repr` (`ts⟨str⟩↖ … ↙↑(x)↘ … ↗`), where the lifted value
    /// must be of that representation. `Repr::Cst` is [`Self::lift_str`].
    fn lift_str_in(&self, target: &str, repr: crate::qterm::Repr) -> Result<&'static str> {
        match repr {
            crate::qterm::Repr::Cst => self.lift_str(target),
            _ => Err(no_repr(repr)),
        }
    }
    /// The spelling `←` expands to inside an unquote of a quote with
    /// representation `repr`. `Repr::Cst` is [`Self::emit_str`].
    fn emit_str_in(&self, repr: crate::qterm::Repr) -> Result<&'static str> {
        match repr {
            crate::qterm::Repr::Cst => self.emit_str(),
            _ => Err(no_repr(repr)),
        }
    }
    /// Code that builds a quote's value in representation `repr` — `⟨str⟩` or
    /// `⟨qt⟩`; `⟨cst⟩` quotes go through [`Self::expand_quote`] and friends —
    /// from its flattened `pieces`. `lang2` is the quoted language.
    fn expand_flat(
        &self,
        repr: crate::qterm::Repr,
        lang2: &str,
        pieces: &[Piece],
    ) -> Result<Arc<QTerm>> {
        let _ = (lang2, pieces);
        Err(no_repr(repr))
    }
    /// The spelling `⟨T⟩` expands to: the type of a quilt term in this host's
    /// meta-code (`Arc<QTerm>` for Rust). `Result` because a host language
    /// without a way to write types has no spelling for it.
    #[inline]
    fn type_str(&self) -> Result<&'static str> {
        Ok(TYPE)
    }
    /// The spelling `⟨N⟩` expands to: the function taking a string to an
    /// identifier term (`name` for Rust). In a string-based meta a name is its
    /// own text, so this is the host's identity function.
    #[inline]
    fn name_str(&self) -> Result<&'static str> {
        Ok(NAME)
    }
    /// The spelling `⟨T⟩` expands to in *method position* — the glyph flush
    /// against an argument list, `db.⟨T⟩(term)`: the machine's *typing*
    /// judgment, the companion of [`Self::reduce_method_str`]'s value
    /// judgment (issue #273). A bare method name, completed by the source's
    /// own parentheses; `⟨T⟩` anywhere else keeps meaning the term type.
    ///
    /// Falls back to [`Self::type_str`] on `Err`, for the same reason
    /// [`Self::reduce_method_str`] falls back to the operator spelling.
    #[inline]
    fn type_method_str(&self) -> Result<&'static str> {
        miette::bail!("this meta-language has no type-of spelling for `⟨T⟩(…)` in method position")
    }
    /// The spelling `lang⟨M⟩` expands to: an expression obtaining a machine
    /// speaking `lang` (issue #273). The annotation is already resolved by
    /// the expander, so `lang` is a concrete language name — which is why
    /// this is the one spelling that is not a `&'static str`.
    ///
    /// The default is an honest error naming what would unblock it, like
    /// every other spelling a host may lack: a string-based meta (nix, lean)
    /// has no runtime to hold a machine handle, so it needs the out-of-process
    /// daemon from issue #268 rather than a spelling.
    #[inline]
    fn spawn_str(&self, lang: &str) -> Result<String> {
        miette::bail!(
            "this meta-language has no spelling for `{lang}⟨M⟩`: it has no runtime to hold a \
             machine handle. Drive the machine from a host that does (rust, python), or wait on \
             the `quilt machine serve` daemon (issue #268)"
        )
    }

    /// What must be in scope for the code this meta generates: the runtime
    /// import an expanded file opens with (issue #274). `None` — the default —
    /// means this host needs none.
    ///
    /// It belongs here rather than on [`Language`](crate::lang::Language)
    /// because the import exists to satisfy the *expander's output*, and the
    /// output vocabulary is the meta's business: `ops.rs` is what decides a
    /// quote becomes `tb("block").c(…)`. A quotable-only target (wgsl, html,
    /// sql, the shells) has no meta and needs no prelude, and a string-based
    /// meta (nix, lean, text) calls no runtime at all, so both get the right
    /// answer from this default.
    ///
    /// `targets` are the object languages the file may lift into — the file's
    /// language chain plus every annotation its quotes name. Rust and Python
    /// ignore them and return one glob import; TypeScript, whose lift spelling
    /// is target-directed (see [`Self::lift_str`]), builds its named import
    /// list from them, which is the half of this that a human gets wrong.
    ///
    /// The injection itself belongs to what writes the file — `quilt expand`,
    /// `quilt run` and the in-browser expander, through
    /// [`Multi::prelude_for`](crate::multi::Multi::prelude_for) and
    /// [`with_prelude`] — not to expansion: a prelude is a property of the
    /// *file written*, not of the term, so `quilt check` stays a pure
    /// parse-and-expand and the expander snapshots do not carry it.
    #[inline]
    fn prelude(&self, targets: &[&str]) -> Option<Prelude> {
        let _ = targets;
        None
    }
}

/**************************************************************/

impl MetaLanguage for Box<dyn MetaLanguage> {
    fn expand_quote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        (**self).expand_quote(lang1, tag, i, lang2, qterm, cmds)
    }

    fn expand_unquote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        (**self).expand_unquote(lang1, tag, i, lang2, qterm, cmds)
    }

    fn expand_tuple(
        &self,
        lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity, // should we split expand_tuple based on variadic vs not?
    ) -> Result<Arc<QTerm>> {
        (**self).expand_tuple(lang1, tag, qterms, cmds, arity)
    }

    fn wrap_child(&self, qterm: Arc<QTerm>, okind: OuterKind) -> Result<Arc<QTerm>> {
        (**self).wrap_child(qterm, okind)
    }

    fn wrap_separated(&self, qterm: Arc<QTerm>, sep: &str) -> Result<Arc<QTerm>> {
        (**self).wrap_separated(qterm, sep)
    }

    fn pattern_tag(&self) -> Option<&'static str> {
        (**self).pattern_tag()
    }

    fn pattern_binding(&self, terms: &[Arc<QTerm>]) -> Option<(usize, usize)> {
        (**self).pattern_binding(terms)
    }

    fn pattern_var(&self, name: &str) -> Result<Arc<QTerm>> {
        (**self).pattern_var(name)
    }

    fn pattern_let(
        &self,
        names: &[Box<str>],
        pattern: &Arc<QTerm>,
        value: &Arc<QTerm>,
    ) -> Result<(Arc<QTerm>, Arc<QTerm>)> {
        (**self).pattern_let(names, pattern, value)
    }

    fn lift_str(&self, target: &str) -> Result<&'static str> {
        (**self).lift_str(target)
    }

    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        (**self).reduce_str(target)
    }

    fn reduce_method_str(&self, target: &str) -> Result<&'static str> {
        (**self).reduce_method_str(target)
    }

    fn emit_str(&self) -> Result<&'static str> {
        (**self).emit_str()
    }

    fn lift_str_in(&self, target: &str, repr: crate::qterm::Repr) -> Result<&'static str> {
        (**self).lift_str_in(target, repr)
    }

    fn emit_str_in(&self, repr: crate::qterm::Repr) -> Result<&'static str> {
        (**self).emit_str_in(repr)
    }

    fn expand_flat(
        &self,
        repr: crate::qterm::Repr,
        lang2: &str,
        pieces: &[Piece],
    ) -> Result<Arc<QTerm>> {
        (**self).expand_flat(repr, lang2, pieces)
    }

    fn type_str(&self) -> Result<&'static str> {
        (**self).type_str()
    }

    fn name_str(&self) -> Result<&'static str> {
        (**self).name_str()
    }

    fn type_method_str(&self) -> Result<&'static str> {
        (**self).type_method_str()
    }

    fn spawn_str(&self, lang: &str) -> Result<String> {
        (**self).spawn_str(lang)
    }

    fn prelude(&self, targets: &[&str]) -> Option<Prelude> {
        (**self).prelude(targets)
    }
}

#[cfg(all(test, feature = "parse"))]
mod tests {
    use super::*;

    /// The injected runtime import goes *after* the file's header, because
    /// Rust's `//!` and `#![…]` are inner and inner means "before any item" —
    /// an import above them is a syntax error, not a style choice.
    #[test]
    fn a_prelude_goes_after_the_header() {
        // Each case names where the first *item* starts, so the expected
        // offset is read off the fixture rather than counted by hand.
        let at = |body: &str, comment: &str, item: &str| {
            assert_eq!(
                prelude_offset(body, comment),
                body.find(item).unwrap(),
                "{body:?}"
            );
        };
        // Nothing to skip.
        at("fn main() {}\n", "//", "fn");
        // A doc header, its trailing blank line included.
        at("//! docs\n//! more\n\nfn main() {}\n", "//", "fn");
        // A shebang, and only on the first line.
        at("#!/usr/bin/env quilt\nfn main() {}\n", "//", "fn");
        at("fn main() {}\n#!/usr/bin/env quilt\n", "//", "fn");
        // An inner attribute is taken whole, brackets balanced across lines.
        at("#![allow(dead_code)]\nfn f() {}\n", "//", "fn");
        at("#![allow(\n    dead_code\n)]\nfn f() {}\n", "//", "fn");
        // An unbalanced one stops the scan rather than splitting it.
        at("#![allow(\nfn f() {}\n", "//", "#![");
        // Python's introducer covers its shebang and its comments alike.
        at("#!/usr/bin/env quilt\n# hi\nx = 1\n", "#", "x =");
    }
}
