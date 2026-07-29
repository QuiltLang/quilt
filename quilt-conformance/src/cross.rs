//! The cross-language grid: every host against every target (issue #158).
//!
//! The per-language battery checks each language on its own. This checks the
//! *grid* — 5 hosts × 10 targets — which is where the interesting corner cases
//! live and where nothing existed before.
//!
//! Three properties, in increasing order of what they'd catch:
//!
//! 1. **Splice** — every host can quote every target and splice a ground term
//!    into it, and the quoted fragment survives the round trip through the
//!    host's parser unchanged.
//!
//! 2. **Identical output across hosts** — the same target fragment, quoted from
//!    any host, produces an *equal* `QTerm`. This is the "meta-programming
//!    should be language-agnostic" tenet turned into a check, and it needs no
//!    golden file: the hosts are each other's oracle. It generalises
//!    `expand_both`, which compares two engines for Rust only, to the whole
//!    grid.
//!
//! 3. **Lift** — which (host, target) pairs support `↑`, verified *through real
//!    source* rather than by calling `lift_str` directly. The battery's
//!    `lift-from` probe asks the API; this one writes `host↖…↙v.↑↘…↗` and
//!    parses it, so the two agree only if the spelling is actually reachable
//!    from the surface syntax. Where a pair is unsupported the error must name
//!    both the host and the target, because "cannot lift" without saying
//!    between what is not actionable.
//!
//! Deliberately not run through the host runtimes: proving the *generated*
//! target text is identical would need rust-script, python3 and node. The
//! parse-level term equality in (2) is the same property one stage earlier, and
//! costs nothing.

use crate::spec::Spec;
use miette::{bail, Result};
use quilt::langs::omni::Omni;
use quilt::qterm::QTerm;
use quilt::term::{STerm as _, Term as _};

/// One grid cell that did not hold.
#[derive(Debug, Clone)]
pub struct Failure {
    pub host: String,
    pub target: String,
    pub probe: String,
    pub detail: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{} → {}] {}: {}",
            self.host, self.target, self.probe, self.detail
        )
    }
}

/// Find the first quote of `lang` anywhere in the tree.
fn find_quote<'a>(t: &'a QTerm, lang: &str) -> Option<&'a QTerm> {
    if let QTerm::Quote { lang: l, .. } = t {
        if &**l == lang {
            return Some(t);
        }
    }
    t.children().find_map(|c| find_quote(c, lang))
}

/// The payload each target contributes to the grid: its first declared
/// fragment, so the grid reuses the corpus the battery already verifies rather
/// than inventing a second one.
fn payload(spec: &Spec) -> Option<(&str, &str)> {
    spec.fragments
        .first()
        .map(|f| (spec.name.as_str(), f.code.as_str()))
}

/// Run the whole grid. `specs` must be every language's spec.
pub fn run(specs: &[Spec]) -> Result<(Vec<Failure>, usize)> {
    let hosts: Vec<&Spec> = specs.iter().filter(|s| s.cross.wrapper.is_some()).collect();
    let targets: Vec<(&str, &str)> = specs.iter().filter_map(payload).collect();

    if hosts.is_empty() || targets.is_empty() {
        bail!("cross-language grid needs at least one host and one target");
    }

    let mut omni = Omni::default();
    let mut failures = Vec::new();
    let mut cells = 0;

    // (target, fragment) → the term the first host produced, for the
    // identical-output oracle.
    let mut reference: std::collections::BTreeMap<&str, (String, QTerm)> =
        std::collections::BTreeMap::new();

    for host in &hosts {
        let wrapper = host.cross.wrapper.as_deref().expect("filtered");
        let lift = host.cross.lift.as_deref();

        for (target, fragment) in &targets {
            cells += 1;
            let mut fail = |probe: &str, detail: String| {
                failures.push(Failure {
                    host: host.name.clone(),
                    target: (*target).to_string(),
                    probe: probe.into(),
                    detail,
                });
            };

            // ── 1. splice ────────────────────────────────────────────────
            let src = wrapper.replace('@', &format!("{target}↖{fragment}↗"));
            match omni.parse_chain(&[&host.name], &src) {
                Ok(parsed) => {
                    match find_quote(&parsed, target) {
                        Some(q) => {
                            let inner =
                                q.children().next().map(|c| c.coparse()).unwrap_or_default();
                            if inner != *fragment {
                                fail(
                                    "splice",
                                    format!(
                                        "fragment did not survive the host's parser:\n  in:  {fragment:?}\n  out: {inner:?}"
                                    ),
                                );
                            }

                            // ── 2. identical across hosts ────────────────
                            match reference.get(*target) {
                                None => {
                                    if let Some(inner) = q.children().next() {
                                        reference
                                            .insert(target, (host.name.clone(), inner.clone()));
                                    }
                                }
                                Some((first_host, first_term)) => {
                                    if q.children().next() != Some(first_term) {
                                        fail(
                                            "identical-across-hosts",
                                            format!(
                                                "the same {target} fragment quoted from {first_host} and from {} \
                                                 produced different terms — meta-programming is supposed to be \
                                                 language-agnostic",
                                                host.name,
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                        None => fail("splice", format!("no {target} quote in the parsed tree")),
                    }

                    if let Err(e) = omni.expand_lang(&host.name, &parsed) {
                        fail("splice", format!("expansion failed: {e}"));
                    }
                }
                Err(e) => fail("splice", format!("parse failed: {e}")),
            }

            // ── 3. lift ─────────────────────────────────────────────────
            let Some(lift) = lift else { continue };
            let src = wrapper.replace('@', &format!("{target}↖↙{lift}↘↗"));
            let claimed = host.lift_from.iter().any(|t| t == target)
                || host.aliases.iter().any(|a| a == target) && host.name == *target;
            let got = omni.parse_chain(&[&host.name], &src);

            match (got, claimed) {
                (Ok(_), true) | (Err(_), false) => {}
                (Ok(_), false) => fail(
                    "lift",
                    format!(
                        "lifting into {target} works from real source, but the spec's `lift_from` \
                         does not list it — promote it in conformance/spec/{}.toml",
                        host.name
                    ),
                ),
                (Err(e), true) => fail(
                    "lift",
                    format!(
                        "the spec says this host can lift into {target}, but parsing \
                         `{src}` failed: {e}"
                    ),
                ),
            }
        }
    }

    Ok((failures, cells))
}

/// The lift-failure message must name both ends. "cannot lift" without saying
/// between *what* sends the reader to the source.
pub fn check_lift_errors(specs: &[Spec]) -> Result<Vec<Failure>> {
    let mut omni = Omni::default();
    let mut failures = Vec::new();

    for host in specs.iter().filter(|s| s.cross.wrapper.is_some()) {
        let (Some(wrapper), Some(lift)) =
            (host.cross.wrapper.as_deref(), host.cross.lift.as_deref())
        else {
            continue;
        };
        for target in &host.lift_from_unsupported {
            let src = wrapper.replace('@', &format!("{target}↖↙{lift}↘↗"));
            if let Err(e) = omni.parse_chain(&[&host.name], &src) {
                let msg = format!("{e}");
                let names_host = msg.contains(&host.name);
                let names_target = msg.contains(target.as_str());
                if !names_host || !names_target {
                    failures.push(Failure {
                        host: host.name.clone(),
                        target: target.clone(),
                        probe: "lift-error-quality".into(),
                        detail: format!(
                            "error names host={names_host} target={names_target}; both are needed \
                             to be actionable. Got: {msg}"
                        ),
                    });
                }
            }
        }
    }
    Ok(failures)
}

/// A language is a usable *chain member* when a two-element chain makes a bare
/// `↖…↗` default to it — the `shaders.wgsl.rs.quilt` case, where the ground
/// language is Rust and un-annotated quotes are WGSL.
///
/// This is what the `chain-member` matrix axis claims, and it was
/// declaration-only until now: the per-language battery only ever parses a
/// single language, so nothing exercised the zipper that walks the chain.
pub fn check_chain_members(specs: &[Spec]) -> Result<(Vec<Failure>, usize)> {
    let hosts: Vec<&Spec> = specs.iter().filter(|s| s.cross.wrapper.is_some()).collect();
    let Some(host) = hosts.first() else {
        bail!("no host with a `[cross] wrapper` to drive the chain check");
    };
    let wrapper = host.cross.wrapper.as_deref().expect("filtered");

    let mut omni = Omni::default();
    let mut failures = Vec::new();
    let mut checked = 0;

    for spec in specs {
        // The host's own language as a chain member is degenerate (a bare quote
        // already defaults to it), so it proves nothing.
        if spec.name == host.name {
            continue;
        }
        let Some((target, fragment)) = payload(spec) else {
            continue;
        };
        checked += 1;

        // A *bare* quote: no annotation, so the chain is the only thing that can
        // decide which language parses it.
        let src = wrapper.replace('@', &format!("↖{fragment}↗"));
        match omni.parse_chain(&[&host.name, target], &src) {
            Ok(parsed) => match find_quote(&parsed, target) {
                Some(q) => {
                    let inner = q.children().next().map(|c| c.coparse()).unwrap_or_default();
                    if inner != fragment {
                        failures.push(Failure {
                            host: host.name.clone(),
                            target: target.to_string(),
                            probe: "chain-member".into(),
                            detail: format!(
                                "bare quote in chain [{}, {target}] did not round-trip:\n  in:  \
                                 {fragment:?}\n  out: {inner:?}",
                                host.name
                            ),
                        });
                    }
                }
                None => failures.push(Failure {
                    host: host.name.clone(),
                    target: target.to_string(),
                    probe: "chain-member".into(),
                    detail: format!(
                        "a bare `↖…↗` in chain [{}, {target}] did not parse as {target} — the \
                         chain's default language was not applied",
                        host.name
                    ),
                }),
            },
            Err(e) => failures.push(Failure {
                host: host.name.clone(),
                target: target.to_string(),
                probe: "chain-member".into(),
                detail: format!(
                    "parsing a bare quote in chain [{}, {target}] failed: {e}",
                    host.name
                ),
            }),
        }
    }

    Ok((failures, checked))
}
