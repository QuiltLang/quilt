//! The Tier-1 battery: one fixed set of probes, run against every language.
//!
//! The maintenance property this exists for: adding a language is adding a spec
//! file, and the ~100 assertions below come for free. Nothing here is
//! per-language code, so a language cannot be registered and then quietly skip
//! the checks the others pass — which is how `bash`, `zsh` and `text` ended up
//! with no coverage at all.
//!
//! ## Outcomes
//!
//! Every probe returns one of three outcomes, and the distinction matters:
//!
//! * **Ok** — the capability works.
//! * **Err** — it fails cleanly, with a diagnostic. This is what an
//!   `unsupported` cell must do.
//! * **Panic** — it fails dirtily. This is *never* acceptable, for any status,
//!   which is what turns issue #11's "no `unimplemented!` panics" goal from a
//!   statement into an enforced invariant.
//!
//! ## Performance
//!
//! Each language's `Language` is constructed once (see `registry`) and reused
//! across its whole corpus, and the expensive `catch_unwind` boundary wraps a
//! whole probe rather than each assertion inside it.

use crate::matrix::{Axis, Cell, Row, Status};
use crate::qsnap::{qhead, qsnap, structural_violations};
use crate::registry::{self, BoxLang};
use crate::spec::{Kind, Spec};
use miette::Result;
use quilt::lang::{flat_nodes, Arity, FlatNode, Language as _, LanguagePost as _};
use quilt::prelude::*;
use quilt::term::Term as _;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// A probe that did not hold. Collected rather than asserted immediately so one
/// run reports every problem in a language, not just the first.
#[derive(Debug, Clone)]
pub struct Failure {
    pub language: String,
    pub axis: Axis,
    pub probe: String,
    pub detail: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} / {}: {}",
            self.language,
            self.axis.key(),
            self.probe,
            self.detail
        )
    }
}

/// Everything one language's run produced.
pub struct Outcome {
    pub row: Row,
    pub failures: Vec<Failure>,
}

/// What a probe body reported.
enum Ran<T> {
    Ok(T),
    Err(String),
    Panicked(String),
}

/// Run `f`, converting a panic into a reportable outcome rather than unwinding
/// out of the harness.
fn run<T>(f: impl FnOnce() -> Result<T>) -> Ran<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ran::Ok(v),
        Ok(Err(e)) => Ran::Err(format!("{e}")),
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic>".into());
            Ran::Panicked(msg)
        }
    }
}

/// Accumulates cells and failures for one language.
struct Ctx<'a> {
    spec: &'a Spec,
    failures: Vec<Failure>,
    cells: Vec<Cell>,
}

impl Ctx<'_> {
    fn fail(&mut self, axis: Axis, probe: &str, detail: impl Into<String>) {
        self.failures.push(Failure {
            language: self.spec.name.clone(),
            axis,
            probe: probe.into(),
            detail: detail.into(),
        });
    }

    /// Record a cell whose claim the harness actually checked.
    fn verified(&mut self, axis: Axis, detail: Vec<String>) {
        let claim = self.spec.claim(axis).expect("validated");
        self.cells.push(Cell {
            axis,
            status: claim.status,
            note: claim.note.clone(),
            issue: claim.issue,
            verified_by: Some(format!("{}/{}", axis.key(), self.spec.name)),
            detail,
        });
    }

    /// Record a cell the harness cannot yet check. Rendered distinctly so the
    /// website never shows a declaration and a proof as the same thing.
    fn declared(&mut self, axis: Axis) {
        let claim = self.spec.claim(axis).expect("validated");
        self.cells.push(Cell {
            axis,
            status: claim.status,
            note: claim.note.clone(),
            issue: claim.issue,
            verified_by: None,
            detail: Vec::new(),
        });
    }
}

/// Run the whole battery for one language.
pub fn run_language(spec: &Spec) -> Result<Outcome> {
    let mut ctx = Ctx {
        spec,
        failures: Vec::new(),
        cells: Vec::new(),
    };

    // One construction, reused by every probe below (see `registry`).
    let lang = match run(|| registry::language(&spec.name)) {
        Ran::Ok(l) => Some(l),
        Ran::Err(e) => {
            ctx.fail(
                Axis::Quotable,
                "construct",
                format!("Language::default: {e}"),
            );
            None
        }
        Ran::Panicked(p) => {
            ctx.fail(
                Axis::Quotable,
                "construct",
                format!("Language::default PANICKED: {p}"),
            );
            None
        }
    };

    if let Some(mut lang) = lang {
        probe_quotable(&mut ctx, &mut lang);
        probe_holes(&mut ctx, &mut lang);
        probe_kinds(&mut ctx, &lang);
        probe_variadic(&mut ctx, &lang);
        probe_runnable(&mut ctx, &lang);
        probe_lift_into(&mut ctx, &mut lang);
    }

    probe_host(&mut ctx);
    probe_lift_from(&mut ctx);
    probe_highlights(&mut ctx);

    // Axes no tier reaches yet. Listing them explicitly (rather than letting
    // them fall through) is what keeps the matrix rectangular and makes the
    // unverified set an obvious, countable backlog.
    for axis in [
        Axis::Reduce,
        Axis::Emit,
        Axis::PatternMatch,
        Axis::RuntimeBinding,
        Axis::GlyphCollisions,
        Axis::ChainMember,
        Axis::HeaderComment,
        Axis::Lsp,
    ] {
        ctx.declared(axis);
    }

    ctx.cells.sort_by_key(|c| c.axis);

    Ok(Outcome {
        row: Row {
            name: spec.name.clone(),
            display: spec.display.clone(),
            aliases: spec.aliases.clone(),
            feature: spec.feature.clone(),
            blurb: spec.blurb.clone(),
            meta_kind: spec.meta_kind.clone(),
            lang_src: spec.lang_src.clone(),
            meta_src: spec.meta_src.clone(),
            cells: ctx.cells,
        },
        failures: ctx.failures,
    })
}

/// Round-trip, root tag, structural soundness and parse idempotence for every
/// declared fragment.
fn probe_quotable(ctx: &mut Ctx, lang: &mut BoxLang) {
    let axis = Axis::Quotable;
    let claim = ctx.spec.claim(axis).expect("validated").status;
    let mut detail = Vec::new();

    if ctx.spec.fragments.is_empty() && claim == Status::Supported {
        ctx.fail(
            axis,
            "corpus",
            "declared `supported` but the spec has no `[[fragments]]` to prove it",
        );
    }

    for frag in &ctx.spec.fragments {
        let ikind = frag.kind.and_then(Kind::to_inner);
        let code = frag.code.clone();

        let parsed = run(|| lang.parse_as(ikind, &flat_nodes(&code)));
        let term = match parsed {
            Ran::Ok(t) => t,
            Ran::Err(e) => {
                // An `unsupported` language is *expected* to fail cleanly.
                if claim != Status::Unsupported {
                    ctx.fail(axis, &frag.name, format!("parse failed: {e}"));
                }
                continue;
            }
            Ran::Panicked(p) => {
                ctx.fail(
                    axis,
                    &frag.name,
                    format!("parse PANICKED (must return Err instead): {p}"),
                );
                continue;
            }
        };

        if claim == Status::Unsupported {
            ctx.fail(
                axis,
                &frag.name,
                "declared `unsupported` but the fragment parsed successfully — \
                 promote the claim or remove the fragment",
            );
            continue;
        }

        // 1. Round-trip: the text must survive parse → coparse untouched.
        let back = term.coparse();
        if back != frag.code {
            ctx.fail(
                axis,
                &frag.name,
                format!(
                    "round-trip differs:\n  in:  {:?}\n  out: {back:?}",
                    frag.code
                ),
            );
        }

        // 2. Structure, not just text: the root tag must be the declared one.
        let tag = term.tag();
        let want = quilt::qterm::QTermTag::tuple(&frag.tag);
        if tag != want {
            ctx.fail(
                axis,
                &frag.name,
                format!("root tag is {tag:?}, spec says {:?}", frag.tag),
            );
        }

        // 3. Structural soundness: every child has a hole to be written into.
        for v in structural_violations(&term) {
            ctx.fail(
                axis,
                &frag.name,
                format!("unsound term: {v}\n{}", qsnap(&term)),
            );
        }

        // 4. Idempotence: reparsing the coparsed text yields an equal term.
        //    This is the check that catches a parser that normalises on the way
        //    in — the term would round-trip once and drift on the second pass.
        if back == frag.code {
            match run(|| lang.parse_as(ikind, &flat_nodes(&back))) {
                Ran::Ok(again) => {
                    if again != term {
                        ctx.fail(
                            axis,
                            &frag.name,
                            format!(
                                "parse is not idempotent:\n--- first ---\n{}\n--- second ---\n{}",
                                qsnap(&term),
                                qsnap(&again)
                            ),
                        );
                    }
                }
                Ran::Err(e) => ctx.fail(axis, &frag.name, format!("reparse failed: {e}")),
                Ran::Panicked(p) => {
                    ctx.fail(axis, &frag.name, format!("reparse PANICKED: {p}"));
                }
            }
        }

        detail.push(format!("{} → {}", frag.name, qhead(&term)));
    }

    ctx.verified(axis, detail);
}

/// Each `@` in a hole probe becomes a hole; the `InnerKind` the language
/// assigns it must match the spec. This is the axis that says *where* a splice
/// can go — the difference between "Lean is quotable" and "Lean is quotable in
/// term, tactic and do-element position but not at command position" (#133).
fn probe_holes(ctx: &mut Ctx, lang: &mut BoxLang) {
    let axis = Axis::HolePositions;
    let mut detail = Vec::new();

    for probe in &ctx.spec.holes {
        let nodes = split_holes(&probe.code);
        let want: Vec<Option<quilt::lang::InnerKind>> =
            probe.ikinds.iter().map(|k| k.to_inner()).collect();

        match run(|| {
            let post = lang.parse_pre(None, &nodes)?;
            Ok(post.holes().iter().map(|h| h.ikind).collect::<Vec<_>>())
        }) {
            Ran::Ok(got) => {
                if got == want {
                    detail.push(format!(
                        "{}: {}",
                        probe.name,
                        probe
                            .ikinds
                            .iter()
                            .map(|k| format!("{k:?}").to_lowercase())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                } else {
                    ctx.fail(
                        axis,
                        &probe.name,
                        format!("hole kinds are {got:?}, spec says {want:?}"),
                    );
                }
            }
            Ran::Err(e) => ctx.fail(axis, &probe.name, format!("parse_pre failed: {e}")),
            Ran::Panicked(p) => ctx.fail(axis, &probe.name, format!("parse_pre PANICKED: {p}")),
        }
    }

    ctx.verified(axis, detail);
}

/// Split a probe string on `@` into literal runs and holes, preserving newlines
/// the way `flat_nodes` does.
fn split_holes(code: &str) -> Vec<FlatNode<'_>> {
    let mut out = Vec::new();
    for (i, chunk) in code.split('@').enumerate() {
        if i > 0 {
            out.push(FlatNode::Hole);
        }
        out.extend(flat_nodes(chunk));
    }
    out
}

fn probe_kinds(ctx: &mut Ctx, lang: &BoxLang) {
    let axis = Axis::KindClassification;
    let mut detail = Vec::new();

    for (tag, want) in &ctx.spec.kinds {
        // `typ` always answers with a concrete kind, so `any` is not a
        // meaningful expectation here.
        let Some(want) = want.to_inner() else {
            ctx.fail(axis, tag, "`any` is not a valid expectation for `[kinds]`");
            continue;
        };
        match run(|| Ok(lang.typ(tag))) {
            Ran::Ok(got) => {
                if got == want {
                    detail.push(format!("{tag} = {got:?}"));
                } else {
                    ctx.fail(
                        axis,
                        tag,
                        format!("typ({tag:?}) is {got:?}, spec says {want:?}"),
                    );
                }
            }
            Ran::Err(e) => ctx.fail(axis, tag, format!("typ failed: {e}")),
            Ran::Panicked(p) => ctx.fail(axis, tag, format!("typ PANICKED: {p}")),
        }
    }

    ctx.verified(axis, detail);
}

fn probe_variadic(ctx: &mut Ctx, lang: &BoxLang) {
    let axis = Axis::VariadicContainers;
    let mut detail = Vec::new();

    for tag in &ctx.spec.variadic {
        match run(|| Ok(lang.arity(tag))) {
            Ran::Ok(Arity::Variadic) => detail.push(tag.clone()),
            Ran::Ok(other) => ctx.fail(
                axis,
                tag,
                format!("arity({tag:?}) is {other:?}, spec says Variadic"),
            ),
            Ran::Err(e) => ctx.fail(axis, tag, format!("arity failed: {e}")),
            Ran::Panicked(p) => ctx.fail(axis, tag, format!("arity PANICKED: {p}")),
        }
    }

    // Over-declaring variadicity silently changes emit behaviour, so the spec
    // pins the negative cases too.
    for tag in &ctx.spec.not_variadic {
        match run(|| Ok(lang.arity(tag))) {
            Ran::Ok(Arity::Variadic) => ctx.fail(
                axis,
                tag,
                format!("arity({tag:?}) is Variadic, spec says it must not be"),
            ),
            Ran::Ok(_) => {}
            Ran::Err(e) => ctx.fail(axis, tag, format!("arity failed: {e}")),
            Ran::Panicked(p) => ctx.fail(axis, tag, format!("arity PANICKED: {p}")),
        }
    }

    ctx.verified(axis, detail);
}

fn probe_runnable(ctx: &mut Ctx, lang: &BoxLang) {
    let axis = Axis::Runnable;
    let claim = ctx.spec.claim(axis).expect("validated").status;

    match run(|| Ok(lang.hashbang())) {
        Ran::Ok(hb) => {
            let has = hb.is_some();
            let claimed = matches!(claim, Status::Supported | Status::Partial);
            if has != claimed {
                ctx.fail(
                    axis,
                    "hashbang",
                    format!(
                        "hashbang() is {hb:?} but the spec claims {:?}",
                        claim.label()
                    ),
                );
            }
            ctx.verified(axis, hb.map(|h| vec![h.to_string()]).unwrap_or_default());
        }
        Ran::Err(e) => {
            ctx.fail(axis, "hashbang", format!("failed: {e}"));
            ctx.declared(axis);
        }
        Ran::Panicked(p) => {
            ctx.fail(axis, "hashbang", format!("PANICKED: {p}"));
            ctx.declared(axis);
        }
    }
}

fn probe_host(ctx: &mut Ctx) {
    let axis = Axis::Host;
    let claim = ctx.spec.claim(axis).expect("validated").status;
    let has_meta = registry::meta(&ctx.spec.name).is_some();
    let claimed = matches!(claim, Status::Supported | Status::Partial);

    if has_meta != claimed {
        ctx.fail(
            axis,
            "registry",
            format!(
                "MetaLanguage {} registered, but the spec claims {:?}",
                if has_meta { "is" } else { "is not" },
                claim.label()
            ),
        );
    }

    // A host must declare which kind of meta it is; a target must not claim one.
    let kind_ok = if has_meta {
        matches!(ctx.spec.meta_kind.as_str(), "runtime" | "string")
    } else {
        ctx.spec.meta_kind == "none"
    };
    if !kind_ok {
        ctx.fail(
            axis,
            "meta_kind",
            format!(
                "meta_kind is {:?} but MetaLanguage {} registered",
                ctx.spec.meta_kind,
                if has_meta { "is" } else { "is not" }
            ),
        );
    }

    ctx.verified(axis, vec![format!("meta_kind = {}", ctx.spec.meta_kind)]);
}

/// Which targets this host's `lift_str` can spell. This is the host half of the
/// lift grid, read straight off the implementation, so a target that gains a
/// spelling (or loses one in a refactor) moves a matrix cell.
fn probe_lift_from(ctx: &mut Ctx) {
    let axis = Axis::LiftFrom;
    let Some(meta) = registry::meta(&ctx.spec.name) else {
        // Not a host: nothing to lift *from*.
        if !ctx.spec.lift_from.is_empty() {
            ctx.fail(
                axis,
                "spec",
                "declares `lift_from` targets but has no MetaLanguage",
            );
        }
        ctx.verified(axis, Vec::new());
        return;
    };

    let mut detail = Vec::new();
    for target in &ctx.spec.lift_from {
        match run(|| meta.lift_str(target)) {
            Ran::Ok(spelling) => detail.push(format!("{target} → {spelling}")),
            Ran::Err(e) => ctx.fail(
                axis,
                target,
                format!("spec says this host can lift into {target:?}, but: {e}"),
            ),
            Ran::Panicked(p) => ctx.fail(axis, target, format!("lift_str PANICKED: {p}")),
        }
    }

    for target in &ctx.spec.lift_from_unsupported {
        match run(|| meta.lift_str(target)) {
            Ran::Ok(spelling) => ctx.fail(
                axis,
                target,
                format!(
                    "spec says lifting into {target:?} is unsupported, but it spells {spelling:?} \
                     — promote it in the spec"
                ),
            ),
            Ran::Err(_) => {}
            Ran::Panicked(p) => ctx.fail(
                axis,
                target,
                format!("lift_str PANICKED (must return Err): {p}"),
            ),
        }
    }

    ctx.verified(axis, detail);
}

/// Lift Rust values into this language and check both the spelling *and* that
/// the result reparses in this language's own grammar. The reparse is the point:
/// it is what catches an escaping bug (Nix `${`, Lean `{`, shell `$`), which a
/// pure string comparison against an expected literal cannot.
fn probe_lift_into(ctx: &mut Ctx, lang: &mut BoxLang) {
    let axis = Axis::LiftInto;
    let mut detail = Vec::new();

    for probe in &ctx.spec.lift {
        let Some(marker) = ctx.spec.lift_marker.as_deref() else {
            ctx.fail(
                axis,
                &probe.value,
                "spec has `[[lift]]` probes but no `lift_marker`",
            );
            continue;
        };

        let lifted = match run(|| lift_value(marker, &probe.value)) {
            Ran::Ok(t) => t,
            Ran::Err(e) => {
                ctx.fail(axis, &probe.value, format!("lift failed: {e}"));
                continue;
            }
            Ran::Panicked(p) => {
                ctx.fail(axis, &probe.value, format!("lift PANICKED: {p}"));
                continue;
            }
        };

        let text = lifted.coparse();
        if text != probe.text {
            ctx.fail(
                axis,
                &probe.value,
                format!("lifted to {text:?}, spec says {:?}", probe.text),
            );
        }

        let got_tag = lifted.tag();
        let want_tag = quilt::qterm::QTermTag::tuple(&probe.tag);
        if got_tag != want_tag {
            ctx.fail(
                axis,
                &probe.value,
                format!("lifted tag is {got_tag:?}, spec says {:?}", probe.tag),
            );
        }

        // The escaping check: feed the lifted literal back through this
        // language's parser. If lifting produced something that does not parse,
        // the generated program is broken at the point of use.
        match run(|| lang.parse_as(None, &flat_nodes(&text))) {
            Ran::Ok(reparsed) => {
                let again = reparsed.coparse();
                if again != text {
                    ctx.fail(
                        axis,
                        &probe.value,
                        format!("lifted literal {text:?} reparsed as {again:?}"),
                    );
                }
            }
            Ran::Err(e) => ctx.fail(
                axis,
                &probe.value,
                format!(
                    "lifted literal {text:?} does not parse in {}: {e}",
                    ctx.spec.name
                ),
            ),
            Ran::Panicked(p) => ctx.fail(
                axis,
                &probe.value,
                format!("reparsing lifted literal PANICKED: {p}"),
            ),
        }

        detail.push(format!("{} → {text}", probe.value));
    }

    ctx.verified(axis, detail);
}

/// A deliberately hostile string: a double quote, a backslash, and the
/// interpolation openers of the shells, Nix (`${`) and Lean (`{`). Every
/// target's escaping must render it inert, and the reparse in `probe_lift_into`
/// is what proves it did.
const TRICKY: &str = r#"a"b\c${d}"#;

/// Build one lifted value.
///
/// The `LiftTo` grid is genuinely ragged — WGSL has no string impl, the shells
/// have no float or bool impl — and it is ragged for good reasons documented in
/// `lift.rs`. So this is written out per marker rather than through one macro:
/// the arms *are* the (Rust type × target language) matrix, and a missing arm is
/// a compile error rather than a silently skipped probe. Adding a `LiftTo` impl
/// upstream means adding its arm here, and the spec's `[[lift]]` entry then
/// pins the spelling.
fn lift_value(marker: &str, value: &str) -> Result<Arc<QTerm>> {
    use miette::miette;
    use quilt::lift::{Bash, Lean, Nix, Python, QLiftTo as _, Rust, Wgsl, Zsh};

    let unknown = || miette!("{marker}: no lift probe for value {value:?}");

    Ok(match marker {
        // The homogeneous case. `lift.rs` blanket-implements
        // `LiftTo<Rust> for T: QLift`, so the same probe shape works, but the
        // set is `QLift`'s (ints, char, str, `Arc<QTerm>`) — no float or bool.
        "Rust" => match value {
            "u32:3" => 3u32.qlift_to::<Rust>(),
            "i32:-7" => (-7i32).qlift_to::<Rust>(),
            "str:plain" => "hi".qlift_to::<Rust>(),
            "str:tricky" => TRICKY.qlift_to::<Rust>(),
            _ => return Err(unknown()),
        },
        // Full set: arbitrary-precision ints, floats, bools, strings.
        "Python" => match value {
            "u32:3" => 3u32.qlift_to::<Python>(),
            "i32:-7" => (-7i32).qlift_to::<Python>(),
            "f32:1.5" => 1.5f32.qlift_to::<Python>(),
            "bool:true" => true.qlift_to::<Python>(),
            "str:plain" => "hi".qlift_to::<Python>(),
            "str:tricky" => TRICKY.qlift_to::<Python>(),
            _ => return Err(unknown()),
        },
        "Nix" => match value {
            "u32:3" => 3u32.qlift_to::<Nix>(),
            "i32:-7" => (-7i32).qlift_to::<Nix>(),
            "f32:1.5" => 1.5f32.qlift_to::<Nix>(),
            "bool:true" => true.qlift_to::<Nix>(),
            "str:plain" => "hi".qlift_to::<Nix>(),
            "str:tricky" => TRICKY.qlift_to::<Nix>(),
            _ => return Err(unknown()),
        },
        "Lean" => match value {
            "u32:3" => 3u32.qlift_to::<Lean>(),
            "i32:-7" => (-7i32).qlift_to::<Lean>(),
            "f32:1.5" => 1.5f32.qlift_to::<Lean>(),
            "bool:true" => true.qlift_to::<Lean>(),
            "str:plain" => "hi".qlift_to::<Lean>(),
            "str:tricky" => TRICKY.qlift_to::<Lean>(),
            _ => return Err(unknown()),
        },
        // WGSL has numeric and bool literals but no string type, so no `str`
        // impl exists — lifting one is a compile error in the generated program
        // rather than a silent truncation (see `lift.rs`).
        "Wgsl" => match value {
            "u32:3" => 3u32.qlift_to::<Wgsl>(),
            "i32:-7" => (-7i32).qlift_to::<Wgsl>(),
            "f32:1.5" => 1.5f32.qlift_to::<Wgsl>(),
            "bool:true" => true.qlift_to::<Wgsl>(),
            _ => return Err(unknown()),
        },
        // The shells are word-oriented: strings and integers only.
        "Zsh" => match value {
            "u32:3" => 3u32.qlift_to::<Zsh>(),
            "i32:-7" => (-7i32).qlift_to::<Zsh>(),
            "str:plain" => "hi".qlift_to::<Zsh>(),
            "str:tricky" => TRICKY.qlift_to::<Zsh>(),
            _ => return Err(unknown()),
        },
        "Bash" => match value {
            "u32:3" => 3u32.qlift_to::<Bash>(),
            "i32:-7" => (-7i32).qlift_to::<Bash>(),
            "str:plain" => "hi".qlift_to::<Bash>(),
            "str:tricky" => TRICKY.qlift_to::<Bash>(),
            _ => return Err(unknown()),
        },
        _ => return Err(miette!("unknown lift marker {marker:?}")),
    })
}

/// Whether a vendored `highlights.scm` is exposed for this grammar. Compile-time
/// information, so it is a table rather than a probe — but keeping it in the
/// matrix means the LSP's highlighting story is visible per language.
fn probe_highlights(ctx: &mut Ctx) {
    let axis = Axis::Highlights;
    let has = highlights_query(&ctx.spec.name).is_some();
    let claim = ctx.spec.claim(axis).expect("validated").status;
    let claimed = matches!(claim, Status::Supported | Status::Partial);

    if has != claimed {
        ctx.fail(
            axis,
            "grammars",
            format!(
                "HIGHLIGHTS_QUERY {} exposed, but the spec claims {:?}",
                if has { "is" } else { "is not" },
                claim.label()
            ),
        );
    }

    let detail = if has {
        vec![format!(
            "quilt::grammars::{}::HIGHLIGHTS_QUERY",
            ctx.spec.name
        )]
    } else {
        Vec::new()
    };
    ctx.verified(axis, detail);
}

fn highlights_query(name: &str) -> Option<&'static str> {
    use quilt::grammars;
    Some(match name {
        "bash" => grammars::bash::HIGHLIGHTS_QUERY,
        "html" => grammars::html::HIGHLIGHTS_QUERY,
        "lean" => grammars::lean::HIGHLIGHTS_QUERY,
        "nix" => grammars::nix::HIGHLIGHTS_QUERY,
        "python" => grammars::python::HIGHLIGHTS_QUERY,
        "zsh" => grammars::zsh::HIGHLIGHTS_QUERY,
        _ => return None,
    })
}
