//! Heterogeneous lifting: turn host-language (Rust) values into `QTerm`s of a
//! *target* object language, selected by a zero-sized marker type.
//!
//! The homogeneous `↑` (Rust lifting into Rust) is `QLift`/`qlift()` in
//! `langs::rust::ops`; this module generalizes it to `LiftTo<L>`, indexed by
//! the target language. Inside `wgsl↖ … ↙x.↑↘ … ↗` the `↑` expands to
//! `qlift_to::<Wgsl>()` (see `langs::rust::ops::lift_spelling`), so a Rust
//! `3u32` lifts to the WGSL term `3u` instead of the Rust term `3`.
//!
//! This module is deliberately *not* gated behind the per-language parser
//! features: lift impls are runtime code for expanded programs, which may not
//! enable the target language's parser (e.g. `nanobots-web` builds quilt with
//! only the `rust` feature but splices WGSL terms). The markers index lifting;
//! they don't need the parser.

use crate::qterm::{leaf, sym, tb, QTerm};
use std::sync::Arc;

/**************************************************************/

/// Marker: the Rust object language (the homogeneous case; see `QLift`).
pub struct Rust;

/// Marker: the Python object language.
pub struct Python;

/// Marker: the WGSL object language.
pub struct Wgsl;

/// Marker: the Zsh object language.
pub struct Zsh;

/// Marker: the Bash object language.
pub struct Bash;

/// Marker: the Nix object language.
pub struct Nix;

/// Marker: the Lean 4 object language.
pub struct Lean;

/**************************************************************/

/// Lift a value to a `QTerm` of the object language `L` (the `↑` operator).
///
/// Implement this per (Rust type, target language) pair; the impl owns the
/// target language's spelling and tags (e.g. WGSL `u32` literals are
/// `int_literal`s spelled `3u`).
pub trait LiftTo<L> {
    fn lift_to(&self) -> Arc<QTerm>;
}

/// Postfix sugar for [`LiftTo`]: `x.qlift_to::<Wgsl>()`. Blanket-implemented,
/// so it is always in scope via the prelude; the bound is only required at the
/// call site.
pub trait QLiftTo {
    fn qlift_to<L>(&self) -> Arc<QTerm>
    where
        Self: LiftTo<L>,
    {
        LiftTo::<L>::lift_to(self)
    }
}

impl<T: ?Sized> QLiftTo for T {}

/**************************************************************/
// WGSL lifts. WGSL has no 64-bit integers, so u64/i64/etc. get no impl —
// lifting one is a compile error in the expanded program, not a silent
// truncation.

macro_rules! wgsl_lift_int {
    ($suffix:literal: $($t:ty),* $(,)?) => {$(
        impl LiftTo<Wgsl> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                leaf("int_literal", &format!(concat!("{}", $suffix), self))
            }
        }
    )*};
}

wgsl_lift_int!("u": u8, u16, u32, usize);
wgsl_lift_int!("i": i8, i16, i32, isize);

impl LiftTo<Wgsl> for f32 {
    fn lift_to(&self) -> Arc<QTerm> {
        leaf("float_literal", &format!("{self}f"))
    }
}

impl LiftTo<Wgsl> for bool {
    fn lift_to(&self) -> Arc<QTerm> {
        leaf("bool_literal", &self.to_string())
    }
}

/**************************************************************/
// Python lifts. Python integers are arbitrary-precision, so every Rust
// integer width lifts losslessly. Strings lift to double-quoted `string`
// literals with the characters Python interprets backslash-escaped; slices
// and `Vec`s of liftable values lift element-wise to `list` literals.

macro_rules! python_lift_int {
    ($($t:ty),* $(,)?) => {$(
        impl LiftTo<Python> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                leaf("integer", &self.to_string())
            }
        }
    )*};
}

python_lift_int!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

macro_rules! python_lift_float {
    ($($t:ty),* $(,)?) => {$(
        impl LiftTo<Python> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                // `{:?}` keeps the decimal point (`1.0`, not `1`), so the
                // lifted literal stays a Python float.
                leaf("float", &format!("{self:?}"))
            }
        }
    )*};
}

python_lift_float!(f32, f64);

impl LiftTo<Python> for bool {
    fn lift_to(&self) -> Arc<QTerm> {
        if *self {
            leaf("true", "True")
        } else {
            leaf("false", "False")
        }
    }
}

/// Escape a string for inclusion in a Python double-quoted literal.
///
/// Public because the `quilt-python` runtime's own `qlift` needs exactly this
/// rule: there were two implementations of "lift a string into Python" — this
/// one, and the binding's, which did no escaping at all, so `qlift('a"b')`
/// produced an unparseable literal and `qlift('a\\b')` silently changed the
/// value (`\\b` is a backspace escape in Python). Sharing the function is what
/// keeps them from drifting apart again. Found by the shared runtime corpus
/// (#159), which runs the same cases against all three published runtimes.
pub fn py_dquote_escape(s: &str) -> String {
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
            c if c.is_control() => write!(out, "\\x{:02x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out
}

/// Build the Python `string` term for `s`, structured the way the Python parser
/// structures a string literal: a `string` tuple over `string_start`,
/// `string_content`, `string_end`.
///
/// This used to be a single flat `leaf("string", "\"…\"")`. That coparses to the
/// right text, so every text-level test passed — but it is a shape
/// tree-sitter-python never produces, which matters because `smatch` /
/// `sinstantiate` / [`QTerm::rewrite`] compare tree structure. It also disagreed
/// with `quilt-python`'s own `qlift`, which already built the three-child form:
/// two implementations of one documented operation, differing in the shape they
/// hand back. `rust::ops::strlit_term` makes the same argument for the Rust
/// target in its own doc comment.
///
/// Fidelity is exact for an escape-free string. A string *with* escapes parses
/// with nested `escape_sequence` children inside `string_content`
/// (`"a\"b"` → `(string_content "a" (escape_sequence) "b")`), which neither this
/// nor `quilt-python`'s `qlift` reproduces; see issue #174 (finding A2) for the
/// general "a lift must equal the parse of its own text" guard.
/// The empty string is spelled `(string (string_start) (string_end))` — the
/// parser emits no `string_content` child when there is no content — so this
/// mirrors that rather than emitting an empty one.
pub fn py_string_term(s: &str) -> Arc<QTerm> {
    let mut b = tb("string");
    b.child(&leaf("string_start", "\""));
    if !s.is_empty() {
        b.child(&leaf("string_content", &py_dquote_escape(s)));
    }
    b.child(&leaf("string_end", "\""));
    b.b()
}

impl LiftTo<Python> for str {
    fn lift_to(&self) -> Arc<QTerm> {
        py_string_term(self)
    }
}

impl LiftTo<Python> for String {
    fn lift_to(&self) -> Arc<QTerm> {
        LiftTo::<Python>::lift_to(self.as_str())
    }
}

impl<T: LiftTo<Python>> LiftTo<Python> for [T] {
    fn lift_to(&self) -> Arc<QTerm> {
        // `[1, 4]` parses as `(list "[" (integer) "," (integer) "]")`: the
        // brackets and commas are child tokens, not literal text. Only the
        // space after a comma is layout.
        let mut b = tb("list").c(&sym("["));
        for (i, x) in self.iter().enumerate() {
            if i > 0 {
                b = b.c(&sym(",")).w(" ");
            }
            b = b.c(&x.lift_to());
        }
        b.c(&sym("]")).b()
    }
}

impl<T: LiftTo<Python>> LiftTo<Python> for Vec<T> {
    fn lift_to(&self) -> Arc<QTerm> {
        self.as_slice().lift_to()
    }
}

/**************************************************************/
// HTML. There is no `LiftTo<Html>` marker yet (issue #149), so this is a free
// function rather than an impl — but the escaping rule already has two callers,
// and that is what it is here for.

/// Escape `& < > " '` so `s` is inert HTML wherever a hole can sit: as text
/// content, and inside a single- or double-quoted attribute value.
///
/// Public for the same reason as [`py_dquote_escape`]: the `quilt-python` and
/// `quilt-wasm` runtimes each need exactly this rule for their `qlift_html`,
/// and each had grown its own byte-identical copy. Three copies of an escaping
/// table is three chances to fix a hole in one of them — the class of bug the
/// conformance epic (#144) exists to close — so the rule lives here and the
/// bindings call it. Tested directly here; issue #192 is where the two copies
/// having no direct test at all was written up.
///
/// Not idempotent, and deliberately so: escaping `&amp;` yields `&amp;amp;`,
/// because the input is a *value*, not markup. A caller lifting an
/// already-escaped fragment wants the term pass-through both `qlift_html`
/// implementations do, not a second trip through this function.
#[must_use]
pub fn escape_html(s: &str) -> String {
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
// Shell lifts (Zsh, Bash). A Rust string lifts to a double-quoted `string`
// literal so the value is treated as inert data: characters the shell would
// interpret inside `"…"` (`"`, `\`, `$`, `` ` ``) are backslash-escaped.
// Integers lift to bare `number` words. The two shells share a grammar lineage,
// so the tags and escaping are identical.

/// Escape a string for inclusion in a POSIX shell double-quoted literal.
fn sh_dquote_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Generate the string and integer `LiftTo` impls for a shell marker.
macro_rules! shell_lifts {
    ($marker:ty; $($t:ty),* $(,)?) => {
        impl LiftTo<$marker> for str {
            fn lift_to(&self) -> Arc<QTerm> {
                // `"s"` parses as `(string "\"" (string_content) "\"")`; an
                // empty string has no `string_content` child.
                let mut b = tb("string").c(&sym("\""));
                if !self.is_empty() {
                    b = b.c(&leaf("string_content", &sh_dquote_escape(self)));
                }
                b.c(&sym("\"")).b()
            }
        }
        impl LiftTo<$marker> for String {
            fn lift_to(&self) -> Arc<QTerm> {
                LiftTo::<$marker>::lift_to(self.as_str())
            }
        }
        $(
            impl LiftTo<$marker> for $t {
                fn lift_to(&self) -> Arc<QTerm> {
                    leaf("number", &self.to_string())
                }
            }
        )*
    };
}

shell_lifts!(Zsh; u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);
shell_lifts!(Bash; u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

/**************************************************************/
// Nix lifts. Nix is a pure expression language, so every value lifts to its
// literal expression: integers to `integer_expression`, floats to
// `float_expression`, booleans to the `true`/`false` builtins, strings to
// double-quoted `string_expression`s, and slices/`Vec`s to space-separated
// `list_expression`s. Nix integers are 64-bit, matching the shell width set.

/// Escape a string for inclusion in a Nix double-quoted literal. Besides `"`
/// and `\`, the antiquotation opener `${` is escaped (to `\${`) so the value
/// stays inert data rather than triggering interpolation.
fn nix_dquote_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out
}

impl LiftTo<Nix> for str {
    fn lift_to(&self) -> Arc<QTerm> {
        // `"s"` parses as `(string_expression "\"" (string_fragment) "\"")`.
        // An empty string has no `string_fragment` child at all.
        let mut b = tb("string_expression").c(&sym("\""));
        if !self.is_empty() {
            b = b.c(&leaf("string_fragment", &nix_dquote_escape(self)));
        }
        b.c(&sym("\"")).b()
    }
}

impl LiftTo<Nix> for String {
    fn lift_to(&self) -> Arc<QTerm> {
        LiftTo::<Nix>::lift_to(self.as_str())
    }
}

macro_rules! nix_lift_int {
    ($($t:ty),* $(,)?) => {$(
        impl LiftTo<Nix> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                leaf("integer_expression", &self.to_string())
            }
        }
    )*};
}

nix_lift_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

macro_rules! nix_lift_float {
    ($($t:ty),* $(,)?) => {$(
        impl LiftTo<Nix> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                // `{:?}` keeps the decimal point (`1.0`, not `1`) so the lifted
                // literal parses back as a Nix float, not an integer.
                leaf("float_expression", &format!("{self:?}"))
            }
        }
    )*};
}

nix_lift_float!(f32, f64);

impl LiftTo<Nix> for bool {
    fn lift_to(&self) -> Arc<QTerm> {
        // `true`/`false` are builtins, parsed as
        // `(variable_expression (identifier))` — the name is its own node.
        tb("variable_expression")
            .c(&leaf("identifier", if *self { "true" } else { "false" }))
            .b()
    }
}

impl<T: LiftTo<Nix>> LiftTo<Nix> for [T] {
    fn lift_to(&self) -> Arc<QTerm> {
        // `[ 1 4 ]` parses as `(list_expression "[" (…) (…) "]")`: the brackets
        // are child tokens and the separators really are just whitespace, since
        // Nix lists are space-separated rather than comma-separated.
        let mut b = tb("list_expression").c(&sym("[")).w(" ");
        for x in self {
            b = b.c(&x.lift_to()).w(" ");
        }
        b.c(&sym("]")).b()
    }
}

impl<T: LiftTo<Nix>> LiftTo<Nix> for Vec<T> {
    fn lift_to(&self) -> Arc<QTerm> {
        self.as_slice().lift_to()
    }
}

/**************************************************************/
// Lean lifts. Lean's `Nat`/`Int` are arbitrary-precision, so every Rust integer
// width lifts losslessly to a `num_lit`. Negative values lift as a `unary_op`
// (`-3`), since `num_lit` itself is unsigned. Floats lift to `scientific_lit`,
// booleans to Lean's `true`/`false` constants, strings to `str_lit`s, and
// slices/`Vec`s to comma-separated `list_lit`s.

/// Escape a string for inclusion in a Lean double-quoted literal. Lean's string
/// escapes are the familiar C-like set; note `{` is *not* escaped here — that is
/// only special inside an interpolated `s!"…"`, which a lifted literal is not.
fn lean_dquote_escape(s: &str) -> String {
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

impl LiftTo<Lean> for str {
    fn lift_to(&self) -> Arc<QTerm> {
        // `"s"` parses as `(str_lit "\"" "\"")` — the quote tokens are children
        // and the body sits between them as raw text, not as its own node.
        let mut b = tb("str_lit").c(&sym("\""));
        if !self.is_empty() {
            b = b.w(&lean_dquote_escape(self));
        }
        b.c(&sym("\"")).b()
    }
}

impl LiftTo<Lean> for String {
    fn lift_to(&self) -> Arc<QTerm> {
        LiftTo::<Lean>::lift_to(self.as_str())
    }
}

macro_rules! lean_lift_uint {
    ($($t:ty),* $(,)?) => {$(
        impl LiftTo<Lean> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                leaf("num_lit", &self.to_string())
            }
        }
    )*};
}

lean_lift_uint!(u8, u16, u32, u64, u128, usize);

macro_rules! lean_lift_int {
    ($($t:ty),* $(,)?) => {$(
        impl LiftTo<Lean> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                // `num_lit` is unsigned in the grammar, so a negative value is
                // the `unary_op` `-` applied to its magnitude.
                if *self < 0 {
                    // `-2` parses as `(unary_op "-" (num_lit))`: the sign is a
                    // child token, not literal text.
                    return tb("unary_op")
                        .c(&sym("-"))
                        .c(&leaf("num_lit", &self.unsigned_abs().to_string()))
                        .b();
                }
                leaf("num_lit", &self.to_string())
            }
        }
    )*};
}

lean_lift_int!(i8, i16, i32, i64, i128, isize);

macro_rules! lean_lift_float {
    ($($t:ty),* $(,)?) => {$(
        impl LiftTo<Lean> for $t {
            fn lift_to(&self) -> Arc<QTerm> {
                // `{:?}` keeps the decimal point (`1.0`, not `1`), so the lifted
                // literal stays a Lean float rather than a `Nat`.
                let s = format!("{self:?}");
                if *self < 0.0 {
                    return tb("unary_op")
                        .c(&sym("-"))
                        .c(&leaf("scientific_lit", s.trim_start_matches('-')))
                        .b();
                }
                leaf("scientific_lit", &s)
            }
        }
    )*};
}

lean_lift_float!(f32, f64);

impl LiftTo<Lean> for bool {
    fn lift_to(&self) -> Arc<QTerm> {
        if *self {
            leaf("true_const", "true")
        } else {
            leaf("false_const", "false")
        }
    }
}

impl<T: LiftTo<Lean>> LiftTo<Lean> for [T] {
    fn lift_to(&self) -> Arc<QTerm> {
        // `[1, 2]` parses as `(list_lit "[" (num_lit) "," (num_lit) "]")`.
        let mut b = tb("list_lit").c(&sym("["));
        for (i, x) in self.iter().enumerate() {
            if i > 0 {
                b = b.c(&sym(",")).w(" ");
            }
            b = b.c(&x.lift_to());
        }
        b.c(&sym("]")).b()
    }
}

impl<T: LiftTo<Lean>> LiftTo<Lean> for Vec<T> {
    fn lift_to(&self) -> Arc<QTerm> {
        self.as_slice().lift_to()
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::STerm;

    #[test]
    fn wgsl_ints() {
        assert_eq!(3u32.qlift_to::<Wgsl>().coparse(), "3u");
        assert_eq!(7usize.qlift_to::<Wgsl>().coparse(), "7u");
        assert_eq!((-2i32).qlift_to::<Wgsl>().coparse(), "-2i");
    }

    #[test]
    fn wgsl_float_bool() {
        assert_eq!(1.5f32.qlift_to::<Wgsl>().coparse(), "1.5f");
        assert_eq!(true.qlift_to::<Wgsl>().coparse(), "true");
    }

    #[test]
    fn wgsl_tags() {
        let QTerm::Tuple { tag, .. } = &*3u32.qlift_to::<Wgsl>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "int_literal");
    }

    #[test]
    fn python_scalars() {
        assert_eq!(42u64.qlift_to::<Python>().coparse(), "42");
        assert_eq!((-7i32).qlift_to::<Python>().coparse(), "-7");
        assert_eq!(1.0f64.qlift_to::<Python>().coparse(), "1.0");
        assert_eq!(2.5f32.qlift_to::<Python>().coparse(), "2.5");
        assert_eq!(true.qlift_to::<Python>().coparse(), "True");
        assert_eq!(false.qlift_to::<Python>().coparse(), "False");
    }

    #[test]
    fn python_strings() {
        // Characters Python interprets inside "…" are backslash-escaped.
        let owned = String::from("hi there");
        assert_eq!(owned.qlift_to::<Python>().coparse(), "\"hi there\"");
        assert_eq!(
            "say \"hi\\\"\nbye".qlift_to::<Python>().coparse(),
            "\"say \\\"hi\\\\\\\"\\nbye\""
        );
        assert_eq!("".qlift_to::<Python>().coparse(), "\"\"");
    }

    /// A lifted Python string is the term the Python parser produces for the same
    /// text — not merely something that coparses to it.
    ///
    /// The lift used to be a single flat `leaf("string", "\"…\"")`, a shape
    /// tree-sitter-python never emits, which the text-level assertions above
    /// could not see. Structure matters because `smatch`/`rewrite` compare it.
    ///
    /// Only escape-free strings are checked: with escapes the parser nests
    /// `escape_sequence` children inside `string_content`, which neither this
    /// lift nor `quilt-python`'s `qlift` reproduces (issue #174, finding A2).
    #[cfg(feature = "parse")]
    #[test]
    fn python_string_lift_matches_the_parser() -> crate::prelude::Result<()> {
        use crate::lang::{flat_nodes, Language};
        use crate::langs::python::lang::PythonLanguage;

        let mut py = PythonLanguage::default();
        for s in ["", "hi there", "/usr/bin", "no escapes at all"] {
            let lifted = s.qlift_to::<Python>();
            let text = lifted.coparse();
            let parsed = py.parse_as(None, &flat_nodes(&text))?;
            assert_eq!(
                &*parsed,
                &*lifted,
                "lift of {s:?} is {:?} but the parser reads {text} as {:?}",
                lifted.sexp(),
                parsed.sexp()
            );
        }
        Ok(())
    }

    #[test]
    fn python_lists() {
        let squares: Vec<u64> = (1..=5).map(|n| n * n).collect();
        assert_eq!(squares.qlift_to::<Python>().coparse(), "[1, 4, 9, 16, 25]");
        let nested = vec![vec![1u8], vec![2, 3]];
        assert_eq!(nested.qlift_to::<Python>().coparse(), "[[1], [2, 3]]");
        let empty: Vec<u8> = Vec::new();
        assert_eq!(empty.qlift_to::<Python>().coparse(), "[]");
    }

    #[test]
    fn python_tags() {
        let QTerm::Tuple { tag, .. } = &*3u32.qlift_to::<Python>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "integer");
        let QTerm::Tuple { tag, .. } = &*vec![1u8].qlift_to::<Python>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "list");
    }

    #[test]
    fn zsh_strings() {
        // String, &str (via deref to `str`), and a literal all lift to a
        // double-quoted zsh string.
        let owned = String::from("hi there");
        let borrowed: &str = "/var/log";
        assert_eq!(owned.qlift_to::<Zsh>().coparse(), "\"hi there\"");
        assert_eq!(borrowed.qlift_to::<Zsh>().coparse(), "\"/var/log\"");
        assert_eq!("plain".qlift_to::<Zsh>().coparse(), "\"plain\"");
    }

    #[test]
    fn zsh_escaping() {
        // Characters zsh interprets inside "…" are backslash-escaped.
        assert_eq!(
            "say \"$x\" `now`".qlift_to::<Zsh>().coparse(),
            "\"say \\\"\\$x\\\" \\`now\\`\""
        );
    }

    #[test]
    fn zsh_ints() {
        assert_eq!(42u32.qlift_to::<Zsh>().coparse(), "42");
        assert_eq!((-7i32).qlift_to::<Zsh>().coparse(), "-7");
    }

    #[test]
    fn bash_lifts() {
        // Bash shares the shell lift behaviour with zsh.
        let p: &str = "/var/log";
        assert_eq!(p.qlift_to::<Bash>().coparse(), "\"/var/log\"");
        assert_eq!(
            "echo `id`".qlift_to::<Bash>().coparse(),
            "\"echo \\`id\\`\""
        );
        assert_eq!(42u32.qlift_to::<Bash>().coparse(), "42");
    }

    #[test]
    fn nix_scalars() {
        assert_eq!(42u64.qlift_to::<Nix>().coparse(), "42");
        assert_eq!((-7i32).qlift_to::<Nix>().coparse(), "-7");
        assert_eq!(1.0f64.qlift_to::<Nix>().coparse(), "1.0");
        assert_eq!(2.5f32.qlift_to::<Nix>().coparse(), "2.5");
        assert_eq!(true.qlift_to::<Nix>().coparse(), "true");
        assert_eq!(false.qlift_to::<Nix>().coparse(), "false");
    }

    #[test]
    fn nix_strings() {
        let owned = String::from("/etc/nixos");
        assert_eq!(owned.qlift_to::<Nix>().coparse(), "\"/etc/nixos\"");
        // `"`, `\` are escaped, and the antiquotation opener `${` becomes `\${`.
        assert_eq!(
            "say \"hi\" ${x}".qlift_to::<Nix>().coparse(),
            "\"say \\\"hi\\\" \\${x}\""
        );
    }

    #[test]
    fn nix_lists() {
        let squares: Vec<u64> = (1..=3).map(|n| n * n).collect();
        // Nix lists are space-separated, not comma-separated.
        assert_eq!(squares.qlift_to::<Nix>().coparse(), "[ 1 4 9 ]");
        let empty: Vec<u8> = Vec::new();
        assert_eq!(empty.qlift_to::<Nix>().coparse(), "[ ]");
    }

    #[test]
    fn lean_ints() {
        assert_eq!(3u32.qlift_to::<Lean>().coparse(), "3");
        assert_eq!(7usize.qlift_to::<Lean>().coparse(), "7");
        // `num_lit` is unsigned, so a negative lifts as a `unary_op`.
        assert_eq!((-2i32).qlift_to::<Lean>().coparse(), "-2");
        assert_eq!(i32::MIN.qlift_to::<Lean>().coparse(), "-2147483648");
    }

    #[test]
    fn lean_floats_and_bools() {
        assert_eq!(1.5f64.qlift_to::<Lean>().coparse(), "1.5");
        // Keeps the decimal point so it stays a float, not a `Nat`.
        assert_eq!(2.0f32.qlift_to::<Lean>().coparse(), "2.0");
        assert_eq!((-0.5f64).qlift_to::<Lean>().coparse(), "-0.5");
        assert_eq!(true.qlift_to::<Lean>().coparse(), "true");
        assert_eq!(false.qlift_to::<Lean>().coparse(), "false");
    }

    #[test]
    fn lean_strings() {
        let owned = String::from("Nat.succ");
        assert_eq!(owned.qlift_to::<Lean>().coparse(), "\"Nat.succ\"");
        assert_eq!(
            "say \"hi\"\n".qlift_to::<Lean>().coparse(),
            r#""say \"hi\"\n""#
        );
        // A brace is *not* escaped: a lifted literal is a plain string, not an
        // interpolated `s!"…"`.
        assert_eq!("{x}".qlift_to::<Lean>().coparse(), "\"{x}\"");
    }

    #[test]
    fn lean_lists() {
        let squares: Vec<u64> = (1..=3).map(|n| n * n).collect();
        assert_eq!(squares.qlift_to::<Lean>().coparse(), "[1, 4, 9]");
        let empty: Vec<u8> = Vec::new();
        assert_eq!(empty.qlift_to::<Lean>().coparse(), "[]");
    }

    #[test]
    fn lean_tags() {
        let QTerm::Tuple { tag, .. } = &*3u32.qlift_to::<Lean>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "num_lit");
        let QTerm::Tuple { tag, .. } = &*vec![1u8].qlift_to::<Lean>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "list_lit");
    }

    /// The whole escape table, one character at a time. The runtime corpus
    /// reaches `escape_html` only through `qlift_html`, and only with a string
    /// containing `& < > "` — so `'` was the one entry in the table with no
    /// coverage anywhere, in either binding (issue #192).
    #[test]
    fn html_escape_table() {
        assert_eq!(escape_html("&"), "&amp;");
        assert_eq!(escape_html("<"), "&lt;");
        assert_eq!(escape_html(">"), "&gt;");
        assert_eq!(escape_html("\""), "&quot;");
        assert_eq!(escape_html("'"), "&#x27;");
    }

    /// A single-quoted attribute value is why `'` is in the table: without it,
    /// `<a title='…'>` breaks out of the attribute the same way `"` does.
    #[test]
    fn html_escape_closes_both_attribute_quotings() {
        assert_eq!(
            escape_html("\" onclick='x'"),
            "&quot; onclick=&#x27;x&#x27;"
        );
        assert_eq!(
            escape_html("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
    }

    /// `&` is escaped first in the sense that matters: the replacements
    /// themselves introduce `&`, and a rule that rescanned its own output would
    /// turn `<` into `&amp;lt;`. One pass over the *input* is the fix, so this
    /// pins that a mixed string escapes each character exactly once.
    #[test]
    fn html_escape_does_not_rescan_its_own_output() {
        assert_eq!(escape_html("a & b < c"), "a &amp; b &lt; c");
        // Not idempotent, by design: the input is a value, not markup.
        assert_eq!(escape_html("&amp;"), "&amp;amp;");
    }

    /// Everything outside the table is passed through byte-for-byte — the
    /// runtime must not apply Quilt's own glyph escaping to target text.
    #[test]
    fn html_escape_leaves_everything_else_alone() {
        assert_eq!(escape_html(""), "");
        assert_eq!(escape_html("plain text"), "plain text");
        assert_eq!(escape_html("héllo ← ↖ 世界"), "héllo ← ↖ 世界");
        assert_eq!(escape_html("a\nb\t\\c"), "a\nb\t\\c");
    }

    #[test]
    fn nix_tags() {
        let QTerm::Tuple { tag, .. } = &*3u32.qlift_to::<Nix>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "integer_expression");
        let QTerm::Tuple { tag, .. } = &*vec![1u8].qlift_to::<Nix>() else {
            panic!("expected tuple");
        };
        assert_eq!(&**tag, "list_expression");
    }
}
