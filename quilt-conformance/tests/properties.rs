//! Tier 6 — the invariants, re-stated as properties (issue #161, phase 5 of #144).
//!
//! The battery in `tests/conformance.rs` asserts each invariant for the handful
//! of values a spec file names. Those invariants are not facts about six
//! literals, they are *properties*:
//!
//! * `unescape(escape(s)) == s` for **every** string, not for the nine glyphs a
//!   corpus case happens to contain;
//! * a lifted value reparses in the target grammar for **every** value, not for
//!   `3`, `-7` and one hostile string — this is the escaping net, and it is the
//!   generalisation of the bug that put `${` and `\` into `TRICKY` in the first
//!   place;
//! * expansion erases **every** quote, not the ones a snapshot happens to cover.
//!
//! ## What drives the generators
//!
//! The same `conformance/spec/*.toml` files the battery reads. A language
//! declares `lift_marker` plus its `[[lift]]` probes, and this file turns those
//! declarations into generators — so a new language gets property coverage from
//! its spec, with no new test code, which is the maintenance property the whole
//! epic exists for.
//!
//! ## Cost
//!
//! Every property builds its parsers once, outside the proptest loop
//! (`Omni::default()` alone constructs ten tree-sitter parsers, Lean's from a
//! ~44 MB `parser.c` — #134), and there is one `#[test]` per language so the
//! work fans out across libtest's threads. Case counts are deliberately modest
//! on the PR path and raised by the nightly job via `PROPTEST_CASES`, which
//! `cases()` below honours.

use proptest::prelude::*;
use quilt::lang::{flat_nodes, Language as _};
use quilt::langs::omni::Omni;
use quilt::node::{escape, unescape, Node};
use quilt::prelude::*;
use quilt::term::Term as _;
use quilt_conformance::registry::{self, BoxLang};
use quilt_conformance::spec::Spec;
use quilt_conformance::{qsnap, spec_dir};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Case count for one property: `default`, overridden by `PROPTEST_CASES`.
///
/// `ProptestConfig::default()` already reads that variable, but naming an
/// explicit `cases` would then silently win over it — and the whole point of
/// the split is that the PR path stays fast while the nightly job runs the same
/// properties two orders of magnitude harder.
fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn config(default: u32) -> ProptestConfig {
    ProptestConfig {
        cases: cases(default),
        // Pin the regression file. The default (`SourceParallel`) looks for a
        // `lib.rs`/`main.rs` beside the test, finds none in `tests/`, and warns
        // on every failure before falling back — noise on exactly the run
        // someone is trying to read.
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct(
                "tests/proptest-regressions/properties.txt",
            ),
        )),
        ..ProptestConfig::default()
    }
}

fn specs() -> Vec<Spec> {
    Spec::load_all(&spec_dir()).expect("specs load")
}

fn spec_for(name: &str) -> Spec {
    specs()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no spec for {name}"))
}

/* ══════════════════════════ surface syntax ══════════════════════════ */

/// Ordinary characters — everything Quilt gives no meaning to.
///
/// `/` and `*` are out, deliberately: a generated `/*` with no closing `*/` is
/// genuinely malformed Quilt, so including them would make the generator
/// produce inputs the property must then exempt. Comments are covered as their
/// own well-formed node kinds below instead. Newlines are out for the same
/// reason — `Node::NewLine` is the node that spells one.
const ORDINARY: &[char] = &[
    'a', 'b', 'z', 'A', 'Z', '0', '9', ' ', '_', '"', '\'', '`', '$', '#', '(', ')', '{', '}', '[',
    ']', '<', '>', ';', ':', ',', '.', '=', '+', '-', '!', '?', '&', '|', '~', '^', '%', '@', 'é',
    'λ',
];

/// One unit of generated content.
///
/// Content is built from *tokens* rather than from a flat character pool,
/// because the two interesting cases are shaped, not incidental:
///
/// * a **glyph** in content — which is what [`escape`] exists for. A
///   `Node::Content` holding a bare `↖` that coparsed without its `\` would
///   reparse as an opening quote: a structure-changing round-trip failure, not
///   a cosmetic one.
/// * a **backslash**, always paired with an ordinary character. The grammar
///   lexes `\X` as one unit, so a `\` in content is only ever the first half of
///   such a pair: `↖a\↗` is not "content `a\`, then a closer", it is content
///   `a` followed by an escaped `↗` and an unclosed quote. Generating a lone or
///   glyph-adjacent `\` would be asking the round trip to invert a tree no
///   parse can produce.
fn arb_content_token() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => prop::sample::select(ORDINARY).prop_map(|c| c.to_string()),
        3 => prop::sample::select(&quilt::node::GLYPHS[..]).prop_map(|c| c.to_string()),
        1 => prop::sample::select(ORDINARY).prop_map(|c| format!("\\{c}")),
    ]
}

fn arb_content_str() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_content_token(), 0..8).prop_map(|ts| ts.concat())
}

/// A language annotation. The grammar spells an opener `[a-z]*↖`, so the empty
/// annotation (an un-annotated quote, which takes its language from the chain)
/// is as important a case as a named one.
fn arb_anno() -> impl Strategy<Value = String> {
    prop::sample::select(&["", "rs", "py", "wgsl", "nix", "lean", "bash", "text"][..])
        .prop_map(str::to_owned)
}

/// Comment bodies are generated well-formed rather than assembled from
/// [`ORDINARY`]: the interesting property is that a comment passes through
/// coparse verbatim, not that a random `*/` terminates one early.
fn arb_comment() -> impl Strategy<Value = Node> {
    prop_oneof![
        "[a-z ]{0,10}".prop_map(|s| Node::PlainLineComment(format!("// {s}").into())),
        "[a-z ]{0,10}".prop_map(|s| Node::PlainBlockComment(format!("/* {s} */").into())),
    ]
}

/// A whole Quilt document, as `Node`s.
///
/// Generating the AST and rendering it is what keeps the corpus *valid*: a
/// generator over raw text would spend nearly every case on unbalanced
/// brackets, which is the fuzz target's job (`fuzz/fuzz_targets/`), not this
/// one's. Here the interesting question is whether valid syntax survives a
/// round trip.
fn arb_nodes() -> impl Strategy<Value = Vec<Node>> {
    let leaf = prop_oneof![
        6 => arb_content_str().prop_map(|s| Node::Content(s.into())),
        3 => Just(Node::NewLine),
        1 => Just(Node::Lift),
        1 => arb_anno().prop_map(|a| Node::Reduce { anno: a.into() }),
        1 => Just(Node::Emit),
        1 => Just(Node::Type),
        1 => Just(Node::Name),
        1 => arb_comment(),
    ];
    let node = leaf.prop_recursive(3, 32, 3, |inner| {
        let kids = prop::collection::vec(inner, 0..4)
            .prop_map(|ns| ns.into_iter().map(Arc::new).collect::<Box<[Arc<Node>]>>());
        prop_oneof![
            (arb_anno(), kids.clone()).prop_map(|(anno, nodes)| Node::Quote {
                anno: anno.into(),
                nodes,
                span: 0..0,
            }),
            (arb_anno(), kids).prop_map(|(anno, nodes)| Node::Unquote {
                anno: anno.into(),
                nodes,
                span: 0..0,
            }),
        ]
    });
    prop::collection::vec(node, 0..6).prop_map(|ns| terminate_line_comments(&ns))
}

/// Put a newline after every `// …` comment that lacks one.
///
/// A line comment consumes the rest of its line as raw text, closing brackets
/// included — so `↙↙// x↘↘` is genuinely malformed Quilt, not a round-trip bug.
/// The constraint belongs to the generator rather than to an exemption in the
/// property: what is being tested is that *valid* syntax round-trips.
fn terminate_line_comments(nodes: &[Node]) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let fixed = match n {
            Node::Quote { anno, nodes, span } => Node::Quote {
                anno: anno.clone(),
                nodes: rewrap(nodes),
                span: span.clone(),
            },
            Node::Unquote { anno, nodes, span } => Node::Unquote {
                anno: anno.clone(),
                nodes: rewrap(nodes),
                span: span.clone(),
            },
            other => other.clone(),
        };
        let is_line_comment = matches!(fixed, Node::PlainLineComment(_));
        out.push(fixed);
        if is_line_comment {
            out.push(Node::NewLine);
        }
    }
    out
}

fn rewrap(nodes: &[Arc<Node>]) -> Box<[Arc<Node>]> {
    let flat: Vec<Node> = nodes.iter().map(|n| (**n).clone()).collect();
    terminate_line_comments(&flat)
        .into_iter()
        .map(Arc::new)
        .collect()
}

proptest! {
    #![proptest_config(config(512))]

    /// `escape` and `unescape` are inverse. `escape` exists so that a glyph a
    /// `\` put into `Node::Content` survives coparse; if the pair is not
    /// inverse, the round trip changes the program. #141 is what this generalises:
    /// `←` was a glyph the escape class did not cover.
    #[test]
    fn escape_unescape_are_inverse(s in ".{0,64}") {
        prop_assert_eq!(&*unescape(&escape(&s)), s.as_str());
    }
}

proptest! {
    #![proptest_config(config(256))]

    /// Valid Quilt surface syntax survives `coparse ∘ parse` unchanged.
    ///
    /// Stated as a fixpoint on the *text* rather than on the `Node` tree: the
    /// tree carries source spans, which a re-parse necessarily renumbers, and
    /// the property people rely on is that expanding a file does not perturb
    /// the parts of it Quilt is not responsible for.
    #[test]
    fn quilt_source_round_trips(nodes in arb_nodes()) {
        let src = Node::coparse(&nodes);
        let parsed = Node::parse(&src)
            .map_err(|e| TestCaseError::fail(format!("generated source {src:?} did not parse: {e}")))?;
        prop_assert_eq!(&*Node::coparse(&parsed), &*src);
    }

    /// Arbitrary glyph soup is a diagnostic, never a panic.
    ///
    /// The in-process cousin of the fuzz target: it costs nothing on the PR
    /// path and covers the shapes a structured generator cannot reach — stray
    /// closers, unbalanced openers, a `\` at end of input. `Node::parse` may
    /// return `Err`; a panic fails the test, which is exactly the contract
    /// `quilt check` depends on (its whole job is to report malformed input).
    #[test]
    fn malformed_source_never_panics(s in prop::collection::vec(
        prop::sample::select(&['↖', '↗', '↙', '↘', '↑', '↓', '←', '⟨', '⟩', '\\', 'a', ' ', '\n', 'T', 'N'][..]),
        0..24,
    )) {
        let src: String = s.into_iter().collect();
        // Either outcome is fine. What is not fine is unwinding out of here.
        let _ = Node::parse(&src);
    }
}

/* ═══════════════════════════════ lift ═══════════════════════════════ */

/// One row of the lift grid: a Rust type, plus — where the target spells them
/// differently — the sign or value that selects the spelling.
///
/// The sign is not a detail. Rust lifts `-1.5` as a `unary_expression` over a
/// positive `float_literal` (matching how its parser sees the source, so the
/// lifted term can be matched and rewritten like a parsed one), Lean does the
/// same for negative integers, and Lean and Python both give `false` its own
/// tag. A row that ignored the sign would be asserting the wrong tag for half
/// its domain.
///
/// Which rows exist for a language is *declared*, not assumed: the rows are
/// read off that language's `[[lift]]` probes, so the grid's raggedness — WGSL
/// has no strings, the shells have no floats or bools — stays stated in one
/// place.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Row {
    U32,
    I32Pos,
    I32Neg,
    F32Pos,
    F32Neg,
    True,
    False,
    Str,
}

impl Row {
    /// The row a `[[lift]]` probe declares. The sign comes from the declared
    /// `text` — which is where the specs already write it (`i32:-7` → `-7`) —
    /// so this needs no new spec syntax.
    fn from_probe(value: &str, text: &str) -> Option<Row> {
        let neg = text.starts_with('-');
        Some(match value.split(':').next()? {
            "u32" => Row::U32,
            "i32" if neg => Row::I32Neg,
            "i32" => Row::I32Pos,
            "f32" if neg => Row::F32Neg,
            "f32" => Row::F32Pos,
            "bool" => {
                if value == "bool:false" {
                    Row::False
                } else {
                    Row::True
                }
            }
            "str" => Row::Str,
            _ => return None,
        })
    }
}

/// A generated value of one row.
#[derive(Clone, Debug)]
enum Val {
    U32(u32),
    I32(i32),
    F32(f32),
    Bool(bool),
    Str(String),
}

impl Val {
    /// The row this value belongs to — and hence whose declared tag it must
    /// produce. The `< 0.0` test matches the one the lift impls use, so `-0.0`
    /// lands in the same row here as it does there.
    fn row(&self) -> Row {
        match self {
            Val::U32(_) => Row::U32,
            Val::I32(x) if *x < 0 => Row::I32Neg,
            Val::I32(_) => Row::I32Pos,
            Val::F32(x) if *x < 0.0 => Row::F32Neg,
            Val::F32(_) => Row::F32Pos,
            Val::Bool(true) => Row::True,
            Val::Bool(false) => Row::False,
            Val::Str(_) => Row::Str,
        }
    }
}

/// The float domain: finite, |x| < 1000, three decimals.
///
/// Both `{}` and `{:?}` render these without an exponent, which keeps the
/// property about *escaping and tagging*. Whether each target's grammar accepts
/// the exponent form a `f32` near the extremes produces (`1e-38`) is a real
/// question, but a different one — see the PR.
fn tame_float(n: i32) -> f32 {
    // The generator's range is ±1_000_000, well inside f32's exactly-representable
    // integers (±2^24), so this conversion is lossless for every value it sees.
    #[allow(clippy::cast_precision_loss)]
    let n = n as f32;
    n / 1000.0
}

fn arb_val(row: Row) -> BoxedStrategy<Val> {
    match row {
        Row::U32 => any::<u32>().prop_map(Val::U32).boxed(),
        Row::I32Pos => (0..=i32::MAX).prop_map(Val::I32).boxed(),
        Row::I32Neg => (i32::MIN..0).prop_map(Val::I32).boxed(),
        Row::F32Pos => (0i32..=1_000_000)
            .prop_map(|n| Val::F32(tame_float(n)))
            .boxed(),
        Row::F32Neg => (-1_000_000i32..0)
            .prop_map(|n| Val::F32(tame_float(n)))
            .boxed(),
        // Two-element domains. Kept in the property rather than left to the
        // battery so every row is checked by the same code path; the cost of
        // re-testing a constant is nil.
        Row::True => Just(Val::Bool(true)).boxed(),
        Row::False => Just(Val::Bool(false)).boxed(),
        Row::Str => prop::collection::vec(prop::sample::select(LIFT_STR_CHARS), 0..16)
            .prop_map(|cs| Val::Str(cs.into_iter().collect()))
            .boxed(),
    }
}

/// The alphabet lifted strings are drawn from: every metacharacter the targets'
/// escapers exist for, at high density.
///
/// `"` and `\` (all of them), `$`/`` ` `` (the shells), `${` (Nix
/// antiquotation), `{`/`}` (Lean interpolation), `#`/`'` (shell quoting), plus
/// ordinary letters so the generator also produces plain words.
///
/// Control characters are excluded: the shells deliberately do not escape them
/// (a newline inside `"…"` is legal bash), so their treatment is per-language
/// rather than a single property. Worth its own axis — see the PR discussion.
const LIFT_STR_CHARS: &[char] = &[
    'a', 'b', 'Z', '0', '9', ' ', '_', '"', '\\', '$', '{', '}', '`', '\'', '#', '(', ')', '*',
    '/', ';', '&', '|', '<', '>', '!', '%', 'é', '中',
];

/// Inputs whose lifted literal is known not to parse, with the issue tracking
/// the fix: `(language, substring, issue)`.
///
/// A quarantine, not an alphabet edit. Quietly dropping a character from
/// [`LIFT_STR_CHARS`] would make the property green and lose the finding; this
/// keeps generating the input, keeps the reason next to it, and makes deleting
/// the entry the regression test for the fix.
///
/// Empty, and worth keeping empty. Its one entry was zsh's `((` (issue #212):
/// the grammar offered the bare `((…))` arithmetic *command* opener inside a
/// double-quoted string, so the lexer preferred that token over
/// `string_content` and every lifted string containing `((` was rejected. Fixed
/// in the `QuiltLang/tree-sitter-zsh` fork; `shell_lifts_match_the_parser` in
/// `quilt/tests/lift_fidelity.rs` pins the case deterministically.
const KNOWN_UNPARSEABLE: &[(&str, &str, u32)] = &[];

fn quarantined(language: &str, text: &str) -> Option<u32> {
    KNOWN_UNPARSEABLE
        .iter()
        .find(|(l, sub, _)| *l == language && text.contains(sub))
        .map(|(_, _, issue)| *issue)
}

/// Lift `v` into the language `marker` names.
///
/// Written out per marker rather than through a macro, for the same reason
/// `battery::lift_value` is: the arms **are** the (Rust type × target language)
/// grid, and the grid is genuinely ragged — WGSL has no string impl, the shells
/// have no float or bool impl. A missing arm has to be a compile error, not a
/// silently skipped property.
fn lift_arbitrary(marker: &str, v: &Val) -> Option<Arc<QTerm>> {
    use quilt::lift::{Bash, Lean, Nix, Python, QLiftTo as _, Rust, Sql, Wgsl, Zsh};
    Some(match (marker, v) {
        ("Rust", Val::U32(x)) => x.qlift_to::<Rust>(),
        ("Rust", Val::I32(x)) => x.qlift_to::<Rust>(),
        ("Rust", Val::F32(x)) => x.qlift_to::<Rust>(),
        ("Rust", Val::Bool(x)) => x.qlift_to::<Rust>(),
        ("Rust", Val::Str(x)) => x.as_str().qlift_to::<Rust>(),

        ("Python", Val::U32(x)) => x.qlift_to::<Python>(),
        ("Python", Val::I32(x)) => x.qlift_to::<Python>(),
        ("Python", Val::F32(x)) => x.qlift_to::<Python>(),
        ("Python", Val::Bool(x)) => x.qlift_to::<Python>(),
        ("Python", Val::Str(x)) => x.as_str().qlift_to::<Python>(),

        ("Nix", Val::U32(x)) => x.qlift_to::<Nix>(),
        ("Nix", Val::I32(x)) => x.qlift_to::<Nix>(),
        ("Nix", Val::F32(x)) => x.qlift_to::<Nix>(),
        ("Nix", Val::Bool(x)) => x.qlift_to::<Nix>(),
        ("Nix", Val::Str(x)) => x.as_str().qlift_to::<Nix>(),

        ("Lean", Val::U32(x)) => x.qlift_to::<Lean>(),
        ("Lean", Val::I32(x)) => x.qlift_to::<Lean>(),
        ("Lean", Val::F32(x)) => x.qlift_to::<Lean>(),
        ("Lean", Val::Bool(x)) => x.qlift_to::<Lean>(),
        ("Lean", Val::Str(x)) => x.as_str().qlift_to::<Lean>(),

        // WGSL has numeric and bool literals but no string type.
        ("Wgsl", Val::U32(x)) => x.qlift_to::<Wgsl>(),
        ("Wgsl", Val::I32(x)) => x.qlift_to::<Wgsl>(),
        ("Wgsl", Val::F32(x)) => x.qlift_to::<Wgsl>(),
        ("Wgsl", Val::Bool(x)) => x.qlift_to::<Wgsl>(),

        // The shells are word-oriented: strings and integers only.
        ("Zsh", Val::U32(x)) => x.qlift_to::<Zsh>(),
        ("Zsh", Val::I32(x)) => x.qlift_to::<Zsh>(),
        ("Zsh", Val::Str(x)) => x.as_str().qlift_to::<Zsh>(),

        // SQL's `literal` covers every constant, strings included.
        ("Sql", Val::U32(x)) => x.qlift_to::<Sql>(),
        ("Sql", Val::I32(x)) => x.qlift_to::<Sql>(),
        ("Sql", Val::F32(x)) => x.qlift_to::<Sql>(),
        ("Sql", Val::Bool(x)) => x.qlift_to::<Sql>(),
        ("Sql", Val::Str(x)) => x.as_str().qlift_to::<Sql>(),

        ("Bash", Val::U32(x)) => x.qlift_to::<Bash>(),
        ("Bash", Val::I32(x)) => x.qlift_to::<Bash>(),
        ("Bash", Val::Str(x)) => x.as_str().qlift_to::<Bash>(),

        _ => return None,
    })
}

/// row → the tag that row's `[[lift]]` probes declare.
///
/// Two probes of the same row must agree (both of Rust's `str` probes say
/// `string_literal`); a spec that disagreed with itself would be declaring two
/// tags for one row, which is a spec bug and fails here rather than silently
/// picking one.
fn declared_tags(spec: &Spec) -> BTreeMap<Row, String> {
    let mut m: BTreeMap<Row, String> = BTreeMap::new();
    for probe in &spec.lift {
        let Some(row) = Row::from_probe(&probe.value, &probe.text) else {
            continue;
        };
        if let Some(prev) = m.insert(row, probe.tag.clone()) {
            assert_eq!(
                prev, probe.tag,
                "{}: probes for row {row:?} declare two different tags",
                spec.name
            );
        }
    }
    m
}

/// The whole lift property for one language.
///
/// Three things must hold for *every* value, not just the spec's six:
///
/// 1. lifting does not panic and produces the tag the spec declares for that
///    row — the tag is what `smatch`/`rewrite` dispatch on, so a term that
///    coparses correctly under the wrong tag is still broken;
/// 2. the lifted text parses in the target's own grammar — the escaping net;
/// 3. reparsing it yields the same text, so the value survives the trip into
///    the generated program unchanged.
fn lift_property(language: &str) {
    let spec = spec_for(language);
    let Some(marker) = spec.lift_marker.clone() else {
        panic!("{language}: spec has no lift_marker, so this test should not exist");
    };
    let tags = declared_tags(&spec);
    let rows: Vec<Row> = tags.keys().copied().collect();
    assert!(
        !rows.is_empty(),
        "{language}: spec declares a lift_marker but no [[lift]] probes"
    );

    // Built once, outside the loop: this is the expensive part. `RefCell`
    // because proptest hands the body to a `Fn` closure but `parse_as` needs
    // `&mut self`.
    let lang: RefCell<BoxLang> =
        RefCell::new(registry::language(language).expect("registry has the language"));

    let strategy = prop::sample::select(rows).prop_flat_map(arb_val);
    proptest!(config(256), |(v in strategy)| {
        let Some(lifted) = lift_arbitrary(&marker, &v) else {
            // The grid is ragged by design, but a row the spec declares must
            // have an impl: the spec is what says the cell exists.
            return Err(TestCaseError::fail(format!(
                "{language}: no lift impl for {v:?} via marker {marker} — the spec declares the \
                 row, so `lift_arbitrary` is missing an arm"
            )));
        };

        let want = &tags[&v.row()];
        let got = lifted.tag();
        prop_assert_eq!(
            &got,
            &quilt::qterm::QTermTag::tuple(want),
            "{}: lifting {:?} produced tag {:?}, spec declares {:?}",
            language, v, got, want
        );

        let text = lifted.coparse();
        if let Some(issue) = quarantined(language, &text) {
            // Known-bad input, tracked. Skipped rather than asserted, so the
            // property stays green without the generator pretending the input
            // does not exist. See `KNOWN_UNPARSEABLE`, issue #{issue}.
            let _ = issue;
            return Ok(());
        }
        let reparsed = lang
            .borrow_mut()
            .parse_as(None, &flat_nodes(&text))
            .map_err(|e| TestCaseError::fail(format!(
                "{language}: lifted {v:?} to {text:?}, which does not parse: {e}"
            )))?;
        prop_assert_eq!(
            &*reparsed.coparse(), &*text,
            "{}: lifted {:?} to {:?}, which reparsed as something else", language, v, text
        );
    });
}

macro_rules! lift_properties {
    ($($name:ident => $lang:literal),* $(,)?) => {$(
        /// One `#[test]` per language so each builds a single parser and the
        /// languages run in parallel under libtest.
        #[test]
        fn $name() {
            lift_property($lang);
        }
    )*};
}

lift_properties! {
    lift_rust => "rust",
    lift_python => "python",
    lift_nix => "nix",
    lift_lean => "lean",
    lift_wgsl => "wgsl",
    lift_bash => "bash",
    lift_zsh => "zsh",
    lift_sql => "sql",
}

/* ══════════════════════════ SQL dialects ═══════════════════════════ */

/// What a *database* makes of a lifted literal — the claim `lift_sql` cannot
/// make on its own.
///
/// `lift_sql` reparses each lifted literal in the vendored grammar, which
/// proves it is one well-formed token. That is necessary and not sufficient:
/// the question issue #233 is about is whether the value *comes back*, and a
/// token can be well-formed and still be read as a different string. So this
/// models each dialect's own reading of a single-quoted literal and asserts it
/// inverse to the escaper.
///
/// The readers below are written from the dialects' rules, deliberately *not*
/// from what the escapers emit — a reader derived from the writer would agree
/// with any bug they shared.
mod sql_dialects {
    /// Read a standard-SQL literal: strip the quotes, and `''` is one `'`.
    /// Nothing else is special — notably not the backslash, which is the whole
    /// difference from `MySQL`.
    pub fn read_standard(lit: &str) -> Option<String> {
        let body = lit.strip_prefix('\'')?.strip_suffix('\'')?;
        let mut out = String::with_capacity(body.len());
        let mut cs = body.chars().peekable();
        while let Some(c) = cs.next() {
            if c == '\'' {
                // A lone `'` inside the body would have ended the literal, so
                // the only legal occurrence is a doubled one.
                if cs.next() != Some('\'') {
                    return None;
                }
                out.push('\'');
            } else {
                out.push(c);
            }
        }
        Some(out)
    }

    /// Read a `MySQL`/`MariaDB` literal in the **default** `sql_mode`
    /// (`NO_BACKSLASH_ESCAPES` off): `''` is one `'`, and a backslash starts an
    /// escape sequence.
    ///
    /// The sequence table is `MySQL`'s, not ours: `\0 \' \" \b \n \r \t \Z \\`
    /// map to their characters, `\%` and `\_` keep the backslash (they are LIKE
    /// metacharacters), and a backslash before anything else is dropped. That
    /// last rule is why escaping only `'` is unsafe here — it is also what
    /// makes a trailing backslash swallow the closing quote, which shows up
    /// below as `None`.
    pub fn read_mysql(lit: &str) -> Option<String> {
        let body = lit.strip_prefix('\'')?;
        let mut out = String::new();
        let mut cs = body.chars().peekable();
        loop {
            let Some(c) = cs.next() else {
                // Ran out of input before the closing quote: the literal was
                // never terminated, which is the breakout this exists to catch.
                return None;
            };
            match c {
                '\'' => {
                    if cs.peek() == Some(&'\'') {
                        cs.next();
                        out.push('\'');
                    } else {
                        // Closing quote; nothing may follow.
                        return cs.next().is_none().then_some(out);
                    }
                }
                '\\' => match cs.next()? {
                    '0' => out.push('\0'),
                    '\'' => out.push('\''),
                    '"' => out.push('"'),
                    'b' => out.push('\u{8}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'Z' => out.push('\u{1a}'),
                    '\\' => out.push('\\'),
                    e @ ('%' | '_') => {
                        out.push('\\');
                        out.push(e);
                    }
                    e => out.push(e),
                },
                c => out.push(c),
            }
        }
    }
}

proptest! {
    #![proptest_config(config(512))]

    /// A value lifted for a dialect is read back as itself *by that dialect*.
    #[test]
    fn sql_lifts_survive_their_own_dialects_reader(
        cs in prop::collection::vec(prop::sample::select(LIFT_STR_CHARS), 0..16)
    ) {
        use quilt::lift::{MySql, QLiftTo as _, Sql};
        let v: String = cs.into_iter().collect();

        let standard = v.as_str().qlift_to::<Sql>().coparse();
        let read_standard = sql_dialects::read_standard(&standard);
        prop_assert_eq!(
            read_standard.as_deref(),
            Some(v.as_str()),
            "standard SQL reader did not recover {:?} from {:?}", v, standard
        );

        let mysql = v.as_str().qlift_to::<MySql>().coparse();
        let read_mysql = sql_dialects::read_mysql(&mysql);
        prop_assert_eq!(
            read_mysql.as_deref(),
            Some(v.as_str()),
            "MySQL reader did not recover {:?} from {:?}", v, mysql
        );
    }
}

/// The negative half, and the reason the `mysql` annotation exists: a value
/// containing a backslash, escaped for the *standard*, is not what `MySQL` reads.
///
/// Without this the property above would be satisfied by two escapers that were
/// secretly the same. `a\` is the sharpest case — the standard spelling leaves
/// `MySQL` with an unterminated literal.
#[test]
fn standard_escaping_is_not_mysql_safe() {
    use quilt::lift::{QLiftTo as _, Sql};

    let unterminated = r"a\".qlift_to::<Sql>().coparse();
    assert_eq!(&*unterminated, r"'a\'");
    assert_eq!(
        sql_dialects::read_mysql(&unterminated),
        None,
        "MySQL should find {unterminated:?} unterminated — if it does not, the \
         reader has stopped modelling MySQL and this whole axis is vacuous"
    );
    // The standard reads it as the value it was.
    assert_eq!(
        sql_dialects::read_standard(&unterminated).as_deref(),
        Some(r"a\")
    );
}

/// …and the mirror: `MySQL` escaping is not standard-safe, so neither dialect can
/// simply adopt the other's escaper.
#[test]
fn mysql_escaping_is_not_standard_safe() {
    use quilt::lift::{MySql, QLiftTo as _};

    let doubled = r"a\".qlift_to::<MySql>().coparse();
    assert_eq!(&*doubled, r"'a\\'");
    // Well-formed under the standard, but two characters instead of one.
    assert_eq!(
        sql_dialects::read_standard(&doubled).as_deref(),
        Some(r"a\\"),
        "the standard has no backslash escape, so doubling corrupts the value"
    );
}

/* ════════════════════════════ expansion ═════════════════════════════ */

/// Every span in a parsed tree, with the path that reached it.
fn spans(term: &QTerm, path: &str, out: &mut Vec<(String, Span)>) {
    if let QTerm::Quote { span, .. } | QTerm::Unquote { span, .. } = term {
        if let Some(s) = span {
            out.push((path.to_string(), s.clone()));
        }
    }
    for (i, child) in term.children().enumerate() {
        spans(child, &format!("{path}.{i}"), out);
    }
}

/// Quotes and unquotes left in a term, by path. Expansion's whole job is to
/// remove them; one that survives is a fragment of Quilt leaking into the
/// generated program.
fn residual_brackets(term: &QTerm, path: &str, out: &mut Vec<String>) {
    match term {
        QTerm::Quote { lang, .. } => out.push(format!("{path}: quote of {lang}")),
        QTerm::Unquote { lang, .. } => out.push(format!("{path}: unquote of {lang}")),
        QTerm::Tuple { .. } => {}
    }
    for (i, child) in term.children().enumerate() {
        residual_brackets(child, &format!("{path}.{i}"), out);
    }
}

/// One generated Quilt program: a host wrapper with a target quote in it.
///
/// `lifted` picks the second shape — `target↖↙v.↑↘↗`, an unquote *inside* the
/// quote — which is what exercises the unquote and lift paths. (An unquote in
/// the wrapper itself would be at ground level, where "unquote depth too high"
/// is the correct answer, not a bug.) It only applies where the host declares
/// it can lift into that target.
#[derive(Clone, Debug)]
struct Program {
    host: usize,
    target: usize,
    fragment: usize,
    lifted: bool,
}

proptest! {
    #![proptest_config(config(96))]

    /// Expansion erases every bracket, and the parse's spans point into the
    /// source that produced them.
    ///
    /// The corpus is the cross-language grid's, generated rather than
    /// enumerated: `cross.rs` runs each (host, target) pair against the
    /// target's *first* fragment, and this reaches all of them, in the host
    /// wrappers the specs declare.
    ///
    /// Spans are diagnostic metadata — they are what makes "unquote depth too
    /// high" point at the offending bracket (#4) — so an out-of-bounds one is a
    /// panic waiting to happen in whichever renderer slices the source with it.
    #[test]
    fn expansion_erases_brackets_and_spans_are_in_bounds(p in program_strategy()) {
        let (specs, hosts, targets, omni) = &mut *fixture().lock().unwrap();
        let host = &specs[hosts[p.host % hosts.len()]];
        let target = &specs[targets[p.target % targets.len()]];
        let fragment = &target.fragments[p.fragment % target.fragments.len()];
        let wrapper = host.cross.wrapper.as_deref().expect("hosts are filtered on this");

        let can_lift = host.lift_from.contains(&target.name);
        let inner = match (p.lifted && can_lift, host.cross.lift.as_deref()) {
            (true, Some(lift)) => format!("{}↖↙{lift}↘↗", target.name),
            _ => format!("{}↖{}↗", target.name, fragment.code),
        };
        let src = wrapper.replace('@', &inner);

        let parsed = omni.parse_chain(&[&host.name], &src).map_err(|e| {
            TestCaseError::fail(format!("parsing {src:?} as {} failed: {e}", host.name))
        })?;

        let mut found = Vec::new();
        spans(&parsed, "root", &mut found);
        for (path, span) in &found {
            prop_assert!(
                span.start <= span.end && span.end <= src.len(),
                "{}: span {:?} at {} is not inside the {}-byte source {:?}",
                host.name, span, path, src.len(), src
            );
        }

        let expanded = omni.expand_lang(&host.name, &parsed).map_err(|e| {
            TestCaseError::fail(format!("expanding {src:?} as {} failed: {e}", host.name))
        })?;

        let mut residual = Vec::new();
        residual_brackets(&expanded, "root", &mut residual);
        prop_assert!(
            residual.is_empty(),
            "{}: expanding {:?} left Quilt brackets in the output: {:?}",
            host.name, src, residual
        );

        // Free while we are here: every child must have a hole to be written
        // into, or it silently vanishes at serialization time.
        let violations = qsnap::structural_violations(&expanded);
        prop_assert!(
            violations.is_empty(),
            "{}: expanding {:?} produced a structurally broken term: {:?}",
            host.name, src, violations
        );
    }
}

fn program_strategy() -> impl Strategy<Value = Program> {
    (any::<u8>(), any::<u8>(), any::<u8>(), any::<bool>()).prop_map(
        |(host, target, fragment, lifted)| Program {
            host: host as usize,
            target: target as usize,
            fragment: fragment as usize,
            lifted,
        },
    )
}

/// Specs, the host/target index lists, and one `Omni`.
///
/// `Omni::default()` builds ten tree-sitter parsers; doing that per case would
/// dominate the run. A `OnceLock<Mutex<…>>` builds it once for the whole test
/// binary — `Multi::parse_chain` needs `&mut self`, hence the mutex rather than
/// a plain `OnceLock`.
type Fixture = (Vec<Spec>, Vec<usize>, Vec<usize>, Omni);

fn fixture() -> &'static std::sync::Mutex<Fixture> {
    static F: std::sync::OnceLock<std::sync::Mutex<Fixture>> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        let specs = specs();
        let hosts: Vec<usize> = (0..specs.len())
            .filter(|&i| specs[i].cross.wrapper.is_some())
            .collect();
        let targets: Vec<usize> = (0..specs.len())
            .filter(|&i| !specs[i].fragments.is_empty())
            .collect();
        assert!(!hosts.is_empty() && !targets.is_empty());
        std::sync::Mutex::new((specs, hosts, targets, Omni::default()))
    })
}
