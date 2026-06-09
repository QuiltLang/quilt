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

use crate::qterm::{leaf, QTerm};
use std::sync::Arc;

/**************************************************************/

/// Marker: the Rust object language (the homogeneous case; see `QLift`).
pub struct Rust;

/// Marker: the WGSL object language.
pub struct Wgsl;

/// Marker: the Zsh object language.
pub struct Zsh;

/// Marker: the Bash object language.
pub struct Bash;

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
}
