//! Lifting into plain text (`text↖…↗` / `txt↖…↗`), issue #149.
//!
//! Every other target has a grammar, and a grammar decides what a lifted value
//! looks like: Python's `"s"`, SQL's `'s'`, Nix's `[ 1 2 ]`. Text has none, so
//! the spelling has to come from somewhere else, and that somewhere is a
//! **dialect**: a small set of lift rules, chosen by the host. Quilt ships two
//! and knows no config format by name. A Caddyfile, a YAML fragment or an
//! in-house DSL is a dialect its author writes in a few lines, by overriding
//! only the rules that differ from the default.
//!
//! # What `↑` produces
//!
//! The default dialect, [`Json`], spells every value the way JSON does:
//!
//! | value | `↑` in `text↖…↗` |
//! | --- | --- |
//! | string | `"…"`, with `"`, `\` and control characters escaped |
//! | integer | its decimal digits |
//! | float | shortest round-trip form, decimal point kept (`1.0`) |
//! | bool | `true` / `false` |
//! | list | `[a, b, c]`, each element lifted the same way |
//!
//! JSON is the default because it is the closest thing config formats have to
//! a common literal syntax: the same spelling is a valid YAML scalar or flow
//! sequence, a TOML value, a JavaScript or HCL literal, and a quoted token in
//! most line-oriented formats. Where it is not, a dialect says so.
//!
//! # Quote-aware lifting
//!
//! Text does not parse, so a lift cannot ask the tree what position it is in.
//! It can be told by the characters around it. When the hole sits directly
//! between two double quotes the author wrote — `"↙x.↑↘"` — the expander
//! spells the lift [`InQuotes`], and a string then renders as its *escaped
//! contents*, without quotes of its own. So
//!
//! ```text
//! text↖name: "↙name.↑↘"↗   ==   text↖name: ↙name.↑↘↗
//! ```
//!
//! and both are safe. Numbers and booleans render the same in either position,
//! and a list inside quotes is its elements joined by the dialect's separator.
//!
//! # Picking a dialect
//!
//! `↑` in a Rust host spells `qlift_to::<Text>()` (or
//! `qlift_to::<InQuotes<Text>>()`), naming `Text` *unqualified*. The prelude's
//! `Text` is `Text<Json>`; a module that declares its own
//!
//! ```ignore
//! type Text = quilt::lift::Text<Caddy>;
//! ```
//!
//! shadows the glob import, and every `↑` in a text quote in that module uses
//! `Caddy`. A single site can name its dialect outright instead:
//! `↙x.qlift_to::<Text<Raw>>()↘`.

use super::LiftTo;
use crate::qterm::{leaf, QTerm};
use crate::term::STerm as _;
use std::marker::PhantomData;
use std::sync::Arc;

/// The splice target the expander passes a host's `lift_str` for a hole in a
/// text quote that sits between two `"` (see the module docs). Not a language
/// name — nothing can annotate a quote with it — but a position *within* one.
pub const TEXT_IN_QUOTES: &str = "text-in-quotes";

/// The tag of every lifted text value — the one tag the text language has, so
/// a lifted value is the term the text "parser" builds for the same
/// characters.
const TEXT_TAG: &str = "text";

/**************************************************************/

/// The lift rules of one text dialect.
///
/// Every method has a default, and the defaults are [`Json`]'s, so a dialect
/// overrides only what it spells differently. The methods take no `self`: a
/// dialect is a type, chosen where the lift is spelled.
///
/// A dialect with no safe rendering for a value — a newline in a bare token, a
/// NaN in JSON — should **panic** with a message saying which value and why.
/// `↑` cannot return an error, and a panic at the lift site is the failure the
/// author can see; text that silently means something else is not.
pub trait TextDialect {
    /// Written before a list's first element.
    const LIST_OPEN: &'static str = "[";
    /// Written between two elements of a list.
    const LIST_SEP: &'static str = ", ";
    /// Written after a list's last element.
    const LIST_CLOSE: &'static str = "]";

    /// A string standing on its own: a complete token, quotes included.
    fn string(s: &str) -> String {
        json_string(s)
    }

    /// A string inside a `"…"` the author wrote: its contents only, escaped
    /// for that position.
    fn in_quotes(s: &str) -> String {
        json_escape(s)
    }

    /// A boolean.
    fn boolean(b: bool) -> String {
        b.to_string()
    }

    /// A float. Integers are not routed through here: decimal digits are the
    /// one spelling every dialect agrees on.
    fn float(f: f64) -> String {
        json_float(f)
    }
}

/// The default dialect: JSON's literal syntax. See the module docs for why.
pub struct Json;

impl TextDialect for Json {}

/// Verbatim: a string is its own characters, with nothing escaped, and a list
/// is its elements separated by spaces.
///
/// This is what splicing a `&str` with `↙s↘` already does; the dialect exists
/// so a whole module can opt into it, and so numbers and lists get the same
/// treatment. Floats keep Rust's `{:?}` spelling, NaN and infinities included,
/// since there is no grammar to violate.
pub struct Raw;

impl TextDialect for Raw {
    const LIST_OPEN: &'static str = "";
    const LIST_SEP: &'static str = " ";
    const LIST_CLOSE: &'static str = "";

    fn string(s: &str) -> String {
        s.to_string()
    }

    fn in_quotes(s: &str) -> String {
        s.to_string()
    }

    fn float(f: f64) -> String {
        format!("{f:?}")
    }
}

/**************************************************************/

/// Marker: the text object language, in dialect `D` (default [`Json`]).
///
/// What a plain `↑` inside `text↖…↗` lifts with. Never constructed.
pub struct Text<D = Json>(PhantomData<D>);

/// Marker: a lift into a text hole the author already wrapped in `"…"`.
///
/// The expander spells `↑` with this when the characters on both sides of the
/// hole are double quotes; see the module docs.
pub struct InQuotes<M>(PhantomData<M>);

/// What a text dialect makes of one host value, in either position. The
/// `LiftTo` impls below are this plus a `text` leaf; keeping the rendering as a
/// `String` is what lets a list join its elements without building a tree that
/// the text parser would never produce.
pub trait TextValue {
    fn render<D: TextDialect>(&self, in_quotes: bool) -> String;
}

impl<D: TextDialect, T: TextValue + ?Sized> LiftTo<Text<D>> for T {
    fn lift_to(&self) -> Arc<QTerm> {
        leaf(TEXT_TAG, &self.render::<D>(false))
    }
}

impl<D: TextDialect, T: TextValue + ?Sized> LiftTo<InQuotes<Text<D>>> for T {
    fn lift_to(&self) -> Arc<QTerm> {
        leaf(TEXT_TAG, &self.render::<D>(true))
    }
}

macro_rules! text_ints {
    ($($t:ty),*) => {$(
        impl TextValue for $t {
            fn render<D: TextDialect>(&self, _in_quotes: bool) -> String {
                self.to_string()
            }
        }
    )*};
}

text_ints!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

impl TextValue for f64 {
    fn render<D: TextDialect>(&self, _in_quotes: bool) -> String {
        D::float(*self)
    }
}

impl TextValue for f32 {
    fn render<D: TextDialect>(&self, _in_quotes: bool) -> String {
        // Through the `f32`'s own shortest spelling, so `0.1f32` is `0.1` and
        // not the `0.10000000149011612` a plain widening would print.
        D::float(format!("{self:?}").parse().unwrap_or(f64::from(*self)))
    }
}

impl TextValue for bool {
    fn render<D: TextDialect>(&self, _in_quotes: bool) -> String {
        D::boolean(*self)
    }
}

impl TextValue for str {
    fn render<D: TextDialect>(&self, in_quotes: bool) -> String {
        if in_quotes {
            D::in_quotes(self)
        } else {
            D::string(self)
        }
    }
}

impl TextValue for String {
    fn render<D: TextDialect>(&self, in_quotes: bool) -> String {
        self.as_str().render::<D>(in_quotes)
    }
}

impl TextValue for char {
    fn render<D: TextDialect>(&self, in_quotes: bool) -> String {
        self.to_string().render::<D>(in_quotes)
    }
}

/// A list lifts each element and joins them with the dialect's separator. On
/// its own it is bracketed (`[a, b]` in JSON); inside quotes there is no list
/// syntax to be in, so it is the elements and separators alone.
impl<T: TextValue> TextValue for [T] {
    fn render<D: TextDialect>(&self, in_quotes: bool) -> String {
        let items: Vec<String> = self.iter().map(|x| x.render::<D>(in_quotes)).collect();
        let body = items.join(D::LIST_SEP);
        if in_quotes {
            body
        } else {
            format!("{}{body}{}", D::LIST_OPEN, D::LIST_CLOSE)
        }
    }
}

impl<T: TextValue> TextValue for Vec<T> {
    fn render<D: TextDialect>(&self, in_quotes: bool) -> String {
        self.as_slice().render::<D>(in_quotes)
    }
}

impl<T: TextValue + ?Sized> TextValue for &T {
    fn render<D: TextDialect>(&self, in_quotes: bool) -> String {
        (**self).render::<D>(in_quotes)
    }
}

/// An already-built term splices as the text it stands for, so a fragment
/// built elsewhere can be lifted alongside plain values (a list of terms, say).
impl TextValue for Arc<QTerm> {
    fn render<D: TextDialect>(&self, _in_quotes: bool) -> String {
        self.coparse()
    }
}

/**************************************************************/
// The JSON rules, public because the Python runtime's text lift calls exactly
// these, and a user's dialect builds on them. The JavaScript runtime keeps its
// own copy, which the shared runtime corpus holds to these.

/// Escape `s` for the inside of a JSON string literal.
///
/// `"` and `\` are backslash-escaped, `\n` `\r` `\t` get their short escapes,
/// and every other control character (C0, DEL and C1, i.e. `char::is_control`)
/// becomes `\uXXXX`. That is stricter than JSON needs — it only requires C0 —
/// and exactly what YAML needs, which forbids DEL and C1 unescaped in a
/// double-quoted scalar. Everything else, non-ASCII included, passes through.
#[must_use]
pub fn json_escape(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out
}

/// A JSON string literal: `"` + [`json_escape`] + `"`.
#[must_use]
pub fn json_string(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

/// A JSON number for a float: the shortest round-trip spelling, with the
/// decimal point kept so `1.0` stays a float in formats that distinguish.
///
/// # Panics
///
/// On NaN and the infinities, which JSON has no spelling for. A dialect that
/// has one (YAML's `.nan`, `.inf`) overrides [`TextDialect::float`].
#[must_use]
pub fn json_float(f: f64) -> String {
    assert!(
        f.is_finite(),
        "cannot lift {f:?} into text: JSON has no spelling for NaN or infinity \
         (override `TextDialect::float` in a dialect that has one)"
    );
    format!("{f:?}")
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lift::QLiftTo as _;

    fn text<T: LiftTo<Text> + ?Sized>(x: &T) -> String {
        x.lift_to().coparse()
    }

    fn quoted<T: LiftTo<InQuotes<Text>> + ?Sized>(x: &T) -> String {
        x.lift_to().coparse()
    }

    #[test]
    fn scalars_spell_as_json() {
        assert_eq!(text(&42u64), "42");
        assert_eq!(text(&-7i32), "-7");
        assert_eq!(text(&u128::MAX), u128::MAX.to_string());
        assert_eq!(text(&1.0f64), "1.0");
        assert_eq!(text(&-1.5f32), "-1.5");
        assert_eq!(text(&0.1f32), "0.1");
        assert_eq!(text(&true), "true");
        assert_eq!(text(&false), "false");
    }

    #[test]
    fn strings_are_json_literals() {
        assert_eq!(text("hi"), "\"hi\"");
        assert_eq!(text(""), "\"\"");
        assert_eq!(text(&String::from("a b")), "\"a b\"");
        assert_eq!(text("say \"hi\"\\"), r#""say \"hi\"\\""#);
        assert_eq!(text("a\nb\tc\r"), r#""a\nb\tc\r""#);
        assert_eq!(text("\u{0}\u{7f}\u{85}"), r#""\u0000\u007f\u0085""#);
        // Not escaped: non-ASCII, and characters that only mean something in
        // some *other* format. A dialect for that format escapes them.
        assert_eq!(text("héllo ${x} {y} 'z' #"), "\"héllo ${x} {y} 'z' #\"");
    }

    #[test]
    fn in_quotes_is_the_escaped_contents() {
        assert_eq!(quoted("hi"), "hi");
        assert_eq!(quoted("say \"hi\"\n"), r#"say \"hi\"\n"#);
        // Not a string: the same spelling in either position.
        assert_eq!(quoted(&3u32), "3");
        assert_eq!(quoted(&true), "true");
        assert_eq!(quoted(&2.5f64), "2.5");
    }

    #[test]
    fn lists_join_with_the_dialect_separator() {
        assert_eq!(text(&vec![1u32, 2, 3]), "[1, 2, 3]");
        assert_eq!(text(&Vec::<u8>::new()), "[]");
        assert_eq!(text(&vec!["a", "b\""]), r#"["a", "b\""]"#);
        assert_eq!(text(&vec![vec![1u8], vec![]]), "[[1], []]");
        // Inside quotes there is no list syntax: elements and separators only.
        assert_eq!(quoted(&vec!["a", "b"]), "a, b");
    }

    #[test]
    fn raw_is_verbatim() {
        let s = "a \"b\" ${c}\n";
        assert_eq!(s.qlift_to::<Text<Raw>>().coparse(), s);
        assert_eq!(s.qlift_to::<InQuotes<Text<Raw>>>().coparse(), s);
        assert_eq!(
            vec!["a", "b", "c"].qlift_to::<Text<Raw>>().coparse(),
            "a b c"
        );
        assert_eq!(f64::NAN.qlift_to::<Text<Raw>>().coparse(), "NaN");
    }

    #[test]
    #[should_panic(expected = "JSON has no spelling for NaN")]
    fn json_refuses_nan_at_the_lift_site() {
        let _ = f64::NAN.qlift_to::<Text>();
    }

    /// A dialect overrides only what differs. This one is a line-oriented
    /// format with bare words: strings are unquoted when they can be, lists
    /// are one per line, and booleans are `on`/`off`.
    #[test]
    fn a_user_dialect_overrides_only_what_differs() {
        struct Conf;
        impl TextDialect for Conf {
            const LIST_OPEN: &'static str = "";
            const LIST_SEP: &'static str = "\n";
            const LIST_CLOSE: &'static str = "";
            fn string(s: &str) -> String {
                if !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "._-/".contains(c)) {
                    s.to_string()
                } else {
                    json_string(s)
                }
            }
            fn boolean(b: bool) -> String {
                (if b { "on" } else { "off" }).to_string()
            }
        }
        assert_eq!("web-1".qlift_to::<Text<Conf>>().coparse(), "web-1");
        assert_eq!("a b".qlift_to::<Text<Conf>>().coparse(), "\"a b\"");
        assert_eq!(true.qlift_to::<Text<Conf>>().coparse(), "on");
        assert_eq!(
            vec!["x", "y z"].qlift_to::<Text<Conf>>().coparse(),
            "x\n\"y z\""
        );
        // Untouched rules are the default's.
        assert_eq!(1.5f64.qlift_to::<Text<Conf>>().coparse(), "1.5");
        assert_eq!("q\"".qlift_to::<InQuotes<Text<Conf>>>().coparse(), "q\\\"");
    }

    #[test]
    fn a_lift_is_one_text_leaf() {
        let QTerm::Tuple { tag, terms, .. } = &*vec![1u8, 2].qlift_to::<Text>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "text");
        assert!(
            terms.is_empty(),
            "a lifted list is flat text, as the parser reads it"
        );
    }
}
