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

use crate::qterm::{leaf, tb, QTerm};
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
fn py_dquote_escape(s: &str) -> String {
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

impl LiftTo<Python> for str {
    fn lift_to(&self) -> Arc<QTerm> {
        leaf("string", &format!("\"{}\"", py_dquote_escape(self)))
    }
}

impl LiftTo<Python> for String {
    fn lift_to(&self) -> Arc<QTerm> {
        LiftTo::<Python>::lift_to(self.as_str())
    }
}

impl<T: LiftTo<Python>> LiftTo<Python> for [T] {
    fn lift_to(&self) -> Arc<QTerm> {
        let mut b = tb("list").w("[");
        for (i, x) in self.iter().enumerate() {
            if i > 0 {
                b = b.w(", ");
            }
            b = b.c(&x.lift_to());
        }
        b.w("]").b()
    }
}

impl<T: LiftTo<Python>> LiftTo<Python> for Vec<T> {
    fn lift_to(&self) -> Arc<QTerm> {
        self.as_slice().lift_to()
    }
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
                leaf("string", &format!("\"{}\"", sh_dquote_escape(self)))
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
        leaf(
            "string_expression",
            &format!("\"{}\"", nix_dquote_escape(self)),
        )
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
        // `true`/`false` are builtins, parsed as `variable_expression`s.
        leaf("variable_expression", if *self { "true" } else { "false" })
    }
}

impl<T: LiftTo<Nix>> LiftTo<Nix> for [T] {
    fn lift_to(&self) -> Arc<QTerm> {
        // Nix list literals are space-separated, not comma-separated: `[ 1 2 ]`.
        let mut b = tb("list_expression").w("[");
        for x in self {
            b = b.w(" ").c(&x.lift_to());
        }
        b.w(" ]").b()
    }
}

impl<T: LiftTo<Nix>> LiftTo<Nix> for Vec<T> {
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
