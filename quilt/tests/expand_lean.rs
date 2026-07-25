//! Lean 4 as a *target* language: `lean↖ … ↗` fragments embedded in a Rust
//! host, expanded by the Rust `MetaLanguage` (the `roundtrip_*` / `expand_*`
//! tests), and Lean as a *host* (meta) language driving generation with its
//! string-based meta (the `host_*` tests — see `langs::lean::meta`).
//!
//! Lean's grammar is layered `module` → command → term, with tactics and
//! do-elements modeled as ordinary terms in a `by` / `do` body. Holes therefore
//! reach term, tactic and whole-command position (see issue #133).

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/// Parse `code` and assert `coparse` reproduces it exactly.
fn roundtrip(code: &str) -> Result<()> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    assert_eq!(code, q.coparse());
    Ok(())
}

/// Parse + expand `code`, returning the coparsed builder source.
fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
}

/// Parse + expand `code` with **Lean as the host** (ground language), returning
/// the coparsed Lean metaprogram (string-based meta — see `langs::lean::meta`).
fn host_expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(&["lean"], code)?;
    Ok(omni.expand_lang("lean", &q)?.coparse())
}

/* ----------------------------- Lean as target ---------------------------- */

/// A Lean term, a whole declaration, and a tactic proof all round-trip through
/// parse → coparse unchanged.
#[test]
fn roundtrip_fragments() -> Result<()> {
    roundtrip("let x = lean↖n + 1↗;")?;
    roundtrip("let d = lean↖def double (n : Nat) : Nat := 2 * n↗;")?;
    roundtrip("let p = lean↖theorem t (n : Nat) : n = n := by rfl↗;")?;
    // Implicit binders and structure instances — the brace-heavy syntax the
    // string meta has to escape.
    roundtrip("let s = lean↖instance : Foo Nat := { n := 1 }↗;")?;
    Ok(())
}

/// Multi-line Lean fragments keep their newlines and indentation.
#[test]
fn roundtrip_multiline() -> Result<()> {
    roundtrip(indoc! {r#"
        let thm = lean↖theorem add_zero (n : Nat) : n + 0 = n := by
          induction n with
          | zero => rfl
          | succ k ih => simp↗;"#})?;
    Ok(())
}

/// A `↙…↘` unquote inside a Lean quote splices a Rust-side term into Lean
/// *term* position — the hole reaches there via `_term_atom`.
#[test]
fn expand_term_unquote() -> Result<()> {
    let out = expand("let e = lean↖n + ↙rhs↘↗;")?;
    // The expansion is Rust builder calls; the spliced child is `rhs` itself.
    assert!(out.contains("rhs"), "{out}");
    Ok(())
}

/// **Known limitation** (issue #133): a bare `↙…↘` at Lean *command* position
/// does not parse.
///
/// The hole is spelled `__QUILT_HOLE__`, which is a plain Lean identifier — and
/// no Lean command starts with a bare identifier, so `module` rejects it. Every
/// other position works, because an identifier is a term. Lifting this needs a
/// `quilt_hole` alternative in the grammar's `_command`, which needs the fork
/// regenerated.
///
/// Splice the enclosing construct instead, or emit into a `by` / `do` body.
///
/// If this test starts failing, the limitation has been lifted — replace it
/// with the positive assertion that the splice works.
#[test]
fn command_position_hole_is_unsupported() {
    let err = expand(indoc! {r#"
        let m = lean↖namespace Demo
        ↙decl↘
        end Demo↗;"#})
    .expect_err("a bare hole at command position should not parse");
    assert!(format!("{err:?}").contains("Parsed with errors"), "{err:?}");
}

/// The supported way to generate a declaration: quote the whole thing, with
/// holes in the *term* positions inside it.
#[test]
fn expand_whole_declaration() -> Result<()> {
    let out = expand("let d = lean↖def f (n : Nat) : Nat := ↙body↘↗;")?;
    assert!(out.contains("body"), "{out}");
    Ok(())
}

/// A hole in *tactic* position inside a `by` block. Tactics are ordinary terms
/// in the grammar, so `_term_atom` covers them with no extra rule.
#[test]
fn expand_tactic_unquote() -> Result<()> {
    let out = expand("let p = lean↖theorem t : True := by ↙tac↘↗;")?;
    assert!(out.contains("tac"), "{out}");
    Ok(())
}

/* ------------------------------ lifting `↑` ------------------------------ */

/// `↑` inside a Lean quote lifts a Rust value into a *Lean* literal via
/// `LiftTo<Lean>`, not a Rust one.
#[test]
fn lift_into_lean() {
    assert_eq!(3u32.qlift_to::<Lean>().coparse(), "3");
    assert_eq!((-2i32).qlift_to::<Lean>().coparse(), "-2");
    assert_eq!(true.qlift_to::<Lean>().coparse(), "true");
    assert_eq!("Nat".qlift_to::<Lean>().coparse(), "\"Nat\"");
    assert_eq!(vec![1u8, 2, 3].qlift_to::<Lean>().coparse(), "[1, 2, 3]");
}

/// The Rust host spells `↑`-into-Lean as `qlift_to::<Lean>()`.
#[test]
fn expand_lift_spelling() -> Result<()> {
    let out = expand("let e = lean↖x + ↙n.↑↘↗;")?;
    assert!(out.contains("qlift_to::<Lean>()"), "{out}");
    Ok(())
}

/* ------------------------------ Lean as host ----------------------------- */

/// Lean-as-host, homogeneous: a `lean↖…↗` quote becomes an interpolated Lean
/// string and a host unquote `↙x↘` becomes Lean's own `{x}` interpolation.
#[test]
fn host_term_splice() -> Result<()> {
    let out = host_expand("def e := lean↖n + ↙rhs↘↗")?;
    assert_eq!(out, r#"def e := s!"n + {rhs}""#);
    Ok(())
}

/// A fully literal fragment flattens to a single flat string — no tower of
/// `{s!"…"}` from the nested tuple structure.
#[test]
fn host_literal_flattens() -> Result<()> {
    assert_eq!(
        host_expand("def d := lean↖def double (n : Nat) : Nat := 2 * n↗")?,
        r#"def d := s!"def double (n : Nat) : Nat := 2 * n""#
    );
    Ok(())
}

/// Braces in the *generated* Lean are escaped as `\{` so they stay literal
/// rather than opening an interpolation in the host's `s!"…"`.
#[test]
fn host_escapes_braces() -> Result<()> {
    let out = host_expand("def s := lean↖instance : Foo := { n := 1 }↗")?;
    assert_eq!(out, r#"def s := s!"instance : Foo := \{ n := 1 }""#);
    Ok(())
}

/// `↑` in a host unquote spells Lean's `toString`.
#[test]
fn host_lift_to_string() -> Result<()> {
    let out = host_expand("def e := lean↖x + ↙↑ n↘↗")?;
    assert_eq!(out, r#"def e := s!"x + {toString n}""#);
    Ok(())
}

/// The string model is language-agnostic: a Lean host can generate *any*
/// target, reconstructing it the same way.
#[test]
fn host_generates_other_language() -> Result<()> {
    assert_eq!(
        host_expand("def e := bash↖echo ↙msg↘↗")?,
        r#"def e := s!"echo {msg}""#
    );
    Ok(())
}

/// Multi-line fragments keep their newlines and indentation inside the Lean
/// string literal (raw newlines are legal in a Lean string).
#[test]
fn host_multiline() -> Result<()> {
    let out = host_expand(indoc! {r##"
        def thm := lean↖theorem t (n : Nat) : n = n := by
          rfl↗"##})?;
    assert_eq!(
        out,
        indoc! {r##"
            def thm := s!"theorem t (n : Nat) : n = n := by
              rfl""##}
    );
    Ok(())
}
