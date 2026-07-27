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

/// A `↙…↘` at Lean *command* position splices a whole declaration.
///
/// The hole is spelled `__QUILT_HOLE__`, a plain Lean identifier, and no Lean
/// command starts with a bare identifier — so `module` rejects it and the
/// fragment does not parse as written. `LeanLanguage::parse_pre` recovers by
/// wrapping each hole that sits alone on its own line in `#check …` (the
/// smallest command taking a term) and stripping the wrapper back out of the
/// parsed tree. See issue #133 for the grammar change that makes this
/// unnecessary.
#[test]
fn expand_command_unquote() -> Result<()> {
    let out = expand(indoc! {r#"
        let m = lean↖namespace Demo
        ↙decl↘
        end Demo↗;"#})?;
    assert!(out.contains("decl"), "{out}");
    // The synthetic wrapper must not survive into the generated code.
    assert!(!out.contains("#check"), "{out}");
    Ok(())
}

/// The command fragment round-trips: the `#check` wrapper is invisible in the
/// serialized output, so `coparse` still reproduces the source exactly.
#[test]
fn roundtrip_command_hole() -> Result<()> {
    roundtrip(indoc! {r#"
        let m = lean↖namespace Demo
        ↙decl↘
        end Demo↗;"#})?;
    Ok(())
}

/// Only the wrappers Quilt introduces are stripped — a `#check ↙x↘` the author
/// actually wrote survives, because that fragment parses on the first attempt
/// and never reaches the recovery path.
#[test]
fn genuine_check_command_is_preserved() -> Result<()> {
    roundtrip("let c = lean↖#check ↙e↘↗;")?;
    let out = expand("let c = lean↖#check ↙e↘↗;")?;
    assert!(out.contains("#check"), "{out}");
    Ok(())
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

/// Emit into a Lean fragment still works when the *host* is Rust: `←` is then
/// the Rust meta's `b_` accumulator, so a generation-time loop can append
/// lifted values to a variadic Lean list (`[ … ]`).
#[test]
fn expand_list_emit() -> Result<()> {
    let out = expand(indoc! {r#"
        fn names(items: &[String]) -> Arc<QTerm> {
            lean↖[ ↙{ for s in items { s.↑.←; } }↘ ]↗
        }
    "#})?;
    println!("{out}");
    assert!(out.contains("qlift_to::<Lean>()"));
    assert!(out.contains(".emit(&mut b_)"));
    Ok(())
}

/// Lean-as-host has no `b_` accumulator to emit into (see `wrap_child` in
/// `langs::lean::meta`), so a *ground* `←` fails loudly instead of leaking the
/// `__EMIT__` placeholder into the generated Lean, and the message points at
/// the functional alternative.
#[test]
fn host_emit_unsupported() {
    for code in [
        // Inside a host unquote — the case that used to expand to
        // `s!"a {v.__EMIT__} b"` with no error at all.
        indoc! {r#"
            def v : String := "x"
            def gen : String := lean↖a ↙v.←↘ b↗"#},
        // …and at plain ground position.
        "def gen : String := ←",
    ] {
        let msg = host_expand(code).unwrap_err().to_string();
        assert!(msg.contains("lean can't emit"), "{msg}");
        assert!(msg.contains("String.intercalate"), "{msg}");
    }
}

/// A `←` at sky depth belongs to a *later* stage, so it is still deferred as
/// its glyph — rejecting emit for this host must not over-fire on quoted code
/// the host merely passes through.
#[test]
fn host_emit_deferred_in_quote() -> Result<()> {
    assert_eq!(
        host_expand("def gen : String := lean↖def f := ←↗")?,
        r#"def gen : String := s!"def f := ←""#
    );
    Ok(())
}
