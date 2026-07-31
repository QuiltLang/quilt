# Dogfooding Audit

**Issue:** [#180](https://github.com/QuiltLang/quilt/issues/180) · **Audited at:** `b1301ae`

Quilt exists to stop people writing `format!("…{}…", x)` when they mean "generate
code". This page audits Quilt's own source for places where it does exactly that,
and asks — for each one — whether a Quilt program (run at bootstrap time, like
`mk_meta.rs.quilt`) would be an improvement.

The bar is not "could this technically be a `.quilt` file". It is: *does routing
this through Quilt make it shorter, or make it correct by construction, or both?*
Three of the candidates below fail that bar, and the reasons they fail are as
useful as the ones that pass.

## Method

Every finding here was checked against a running build, not read off. The
technique the first two findings rely on is a one-liner you can run yourself:

```rust
// probe.rs.quilt — what does the real grammar say this literal looks like?
println!("{}", wgsl↖3u↗.↑.coparse());
```

`wgsl↖3u↗` parses `3u` with the vendored WGSL grammar; `.↑` lifts the resulting
term into the Rust source that rebuilds it. The expander is an oracle for "what
shape does a parsed literal actually have" — which is precisely the question the
hand-written tables in `lift.rs` answer from memory.

## Findings

| # | Candidate | Today | Verdict |
|---|---|---|---|
| 1 | [Heterogeneous lift impls](#1-heterogeneous-lift-impls-liftrs) | `lift.rs`, 726 lines, 28 hand-written `LiftTo` impls | **Do it** — and it fixes [#174](https://github.com/QuiltLang/quilt/issues/174) as a side effect |
| 2 | [Builder-call emitters](#2-builder-call-emitters-langsops) | `langs/{rust,python,typescript}/ops.rs`, ~925 lines, ~90% cloned | **Do it, partially** — generate the fragment shapes, keep the fold |
| 3 | [Arity tables from the grammar](#3-arity-tables-from-the-grammar) | 9 hand-written `match tag` allowlists | **Do it** — but from `REPEAT`, not `children.multiple`. Two calls left ([#202](https://github.com/QuiltLang/quilt/issues/202)) |
| 4 | [String-based emitters](#4-string-based-emitters-nix-lean) | `langs/{nix,lean}/ops.rs`, 331 lines | **Don't** — the output is a string, not a term; Quilt adds nothing |
| 5 | [`omni.rs` registry](#5-the-omni-registry) | 528 lines, already `macro_rules!` | **Don't** — `macro_rules!` is the right tool; Quilt would be worse |
| 6 | [`strlift.rs`](#6-strliftrs-and-the-bootstrap-floor) | 200 lines of `format!` | **Can't** — it is the bootstrap's trusted base case |
| 7 | [Conformance harness / matrix](#7-what-is-not-a-candidate) | `battery.rs`, `matrix.rs` | **Not applicable** — the `format!`s are diagnostics and Markdown, not code |

---

## 1. Heterogeneous lift impls (`lift.rs`)

### What is there today

`lift.rs` hand-writes the shape of a literal in each target language:

```rust
impl LiftTo<Wgsl> for u32 {
    fn lift_to(&self) -> Arc<QTerm> {
        leaf("int_literal", &format!("{self}u"))
    }
}
```

28 impls across five targets, most of them behind `macro_rules!` so the *type*
list is generated but the *shape* — the tags, the nesting, the punctuation — is
typed in by hand for every target.

### Why that is a correctness problem, not a style problem

[#174](https://github.com/QuiltLang/quilt/issues/174) established that most of
these shapes were wrong: the term `lift.rs` built was not the term the parser
builds. Only plain scalars were faithful — every string, every container, every
negative number and both WGSL cases built a tree the grammar would never produce.

Two rounds of hand-fixing have since closed most of it (#176 for the Python
string, `fed278b` for every case whose *root tag* does not move), with
`quilt/tests/lift_fidelity.rs` as a structural guard. What remains hand-written
is the *method*: a person reads parser output and retypes it into `lift.rs`. That
is what generation replaces, and it is why the survey had to be run twice.

The survey itself was always mechanical — the expander will just tell you:

```console
$ quilt expand probe.rs.quilt
wgsl 3u    => tb("const_literal").c(&leaf("int_literal", "3u")).b()
wgsl true  => tb("const_literal").c(&tb("bool_literal").c(&sym("true")).b()).b()
py  -7     => tb("unary_operator").c(&sym("-")).c(&leaf("integer", "7")).b()
py  "ab"   => tb("string").c(&leaf("string_start", "\"")).c(&leaf("string_content", "ab")).c(&leaf("string_end", "\"")).b()
py  [1, 4] => tb("list").c(&sym("[")).c(&leaf("integer", "1")).c(&sym(",")).w(" ").c(&leaf("integer", "4")).c(&sym("]")).b()
nix true   => tb("variable_expression").c(&leaf("identifier", "true")).b()
lean -2    => tb("unary_op").c(&sym("-")).c(&leaf("num_lit", "2")).b()
```

Compare against `lift.rs` and you reproduce #174's table exactly — `leaf("int_literal", …)`
where the grammar wants a `const_literal` wrapper, `leaf("string", …)` where the
grammar wants three children, `.w("[")` where the grammar wants `.c(&sym("["))`.

### The generator

The trick is already in the tree: `mk_meta.rs.quilt` generates the numeric `Lift`
impls by lifting a *parsed* sample and rewriting the literal out of it
(`code_int_0.rewrite_naive(&str_0, &format)`). The heterogeneous case is the same
move with a target-annotated quote. **This runs today** — the following is a
working `.quilt` file, not a sketch:

```rust
#!/usr/bin/env quilt
use quilt::prelude::*;
use quilt::term::STerm;

fn main() -> Result<()> {
    let out: ⟨T⟩ = ↖
        use crate::lift::{LiftTo, Python, Wgsl};

        ↙
            ⟨//⟩ One sample literal per (target, shape), parsed by the real grammar.
            ⟨//⟩ `.↑` turns the parsed term into the Rust source that rebuilds it.
            let wgsl_u: ⟨T⟩ = wgsl↖3u↗.↑;
            let wgsl_u_lit: ⟨T⟩ = ↖"3u"↗;
            let py_str: ⟨T⟩ = py↖"s"↗.↑;
            let py_str_lit: ⟨T⟩ = ↖"s"↗;

            for &ty in &["u8", "u16", "u32", "usize"] {
                let fmt: ⟨T⟩ = ↖&format!(↙"{self}u".↑↘)↗;
                ↖
                    impl LiftTo<Wgsl> for ↙⟨N⟩(ty)↘ {
                        fn lift_to(&self) -> ⟨T⟩ {
                            ↙wgsl_u.rewrite_naive(&wgsl_u_lit, &fmt)↘
                        }
                    }
                ↗.←;
                NL.←;
            }

            for &ty in &["str", "String"] {
                let body: ⟨T⟩ = ↖&py_dquote_escape(self)↗;
                ↖
                    impl LiftTo<Python> for ↙⟨N⟩(ty)↘ {
                        fn lift_to(&self) -> ⟨T⟩ {
                            ↙py_str.rewrite_naive(&py_str_lit, &body)↘
                        }
                    }
                ↗.←;
                NL.←;
            }
        ↘
    ↗;
    println!("{}", out.coparse());
    Ok(())
}
```

Its actual output:

```rust
use crate::lift::{LiftTo, Python, Wgsl};

impl LiftTo<Wgsl> for u8 {
    fn lift_to(&self) -> Arc<QTerm> {
        tb("const_literal").c(&leaf("int_literal", &format!("{self}u"))).b()
    }
}
…
impl LiftTo<Python> for str {
    fn lift_to(&self) -> Arc<QTerm> {
        tb("string").c(&leaf("string_start", "\"")).c(&leaf("string_content", &py_dquote_escape(self))).c(&leaf("string_end", "\"")).b()
    }
}
```

Both shapes are the faithful ones. Note the Python `str` case: that three-child
form is exactly what [#176](https://github.com/QuiltLang/quilt/issues/176) had to
fix by hand after the core and the PyO3 binding drifted apart. A generated
`lift.rs` could not have drifted, because neither copy would have been written by
a human.

### What this buys

* **Faithfulness by construction.** The shape comes from the same parser
  `lift_fidelity.rs` compares against, so its assertion becomes true by
  construction rather than by vigilance — and nobody has to run the survey a
  third time.
* **Adding a target gets cheap.** Today it is ~60 lines of shape-guessing per
  language. It becomes a table of sample literals.
* **`lift.rs` shrinks.** Of its 726 lines, 493 precede the test module, and the
  28 impls plus their `macro_rules!` scaffolding are the bulk of those. What
  stays behind is the four `*_dquote_escape` helpers (~90 lines of genuine
  runtime logic) and the marker types; what replaces the rest is a table of
  sample literals plus the loop above.

### What it costs, and what has to be decided first

What is left is the call #174 flagged, which the audit does not resolve and
`fed278b` deliberately skipped: the remaining shapes **move declared conformance
claims**.
`wgsl.toml` pins `tag = "int_literal"` but the parser wraps literals in
`const_literal`; `python.toml` pins `tag = "integer"` for `i32:-7` but `-7`
parses as `unary_operator`. Either the specs follow the parser, or the invariant
is deliberately weakened and the reason written down.

Two further limits worth knowing before starting:

* **Containers need a loop, not a rewrite.** `Vec<T>` cannot be produced by
  substituting into a fixed sample. But the sample still supplies the three
  things that vary by language — opener, separator, closer. From `py↖[1, 4]↗` you
  read off `sym("[")` / `sym(",") + w(" ")` / `sym("]")`; from `nix↖[ 1 4 ]↗` you
  read off `sym("[")` / `w(" ")` / `sym("]")`. Generate the loop body from those,
  keep the loop hand-written.
* **Escapes remain a floor.** Python strings containing escapes parse with nested
  `escape_sequence` children inside `string_content`, which no sample-and-rewrite
  scheme reproduces. That needs escape-aware lifting or an explicit exemption —
  same conclusion #174 reached.

---

## 2. Builder-call emitters (`langs/*/ops.rs`)

`rust/ops.rs`, `python/ops.rs` and `typescript/ops.rs` each contain the same four
functions — `build_tuple_code`, `build_quote_code`, `build_unquote_code`,
`build_variadic_block` — differing only in three axes:

| | Rust | Python | TypeScript |
|---|---|---|---|
| child splice | `.c(&x)` | `.c(x)` | `.c(x)` |
| cmd list | `&[…]` | `[…]` | `[…]` |
| `NL`/`POP`/`HOLE` | constants | constants | calls: `NL()` |
| variadic | imperative `b_` block | `.e()` chain | `.e()` chain |

`diff python/ops.rs typescript/ops.rs` is 86 lines out of 374. The
Rust↔Python diff over the shared region is a dozen one-character changes.

The duplicated part is a fold over `cmds` that assembles a target-language method
chain out of string fragments:

```rust
CmdOrHole::Cmd(StrCmd::Write(s)) => { b.write(&format!(".w({})", str_lit(s)));  }
CmdOrHole::Cmd(StrCmd::NewLine)  => { b.write(".n()");                          }
CmdOrHole::Hole                  => { b.write(".c(&"); b.child(…); b.write(")"); }
```

### Verdict: generate the fragments, keep the fold

The chain is dynamic — its length depends on runtime `cmds` — so the *fold* has
to stay ordinary Rust. What does not have to stay hand-written is the eight
fragment shapes each language spells out as string literals.

Every prefix of a builder chain is a complete expression in all three languages,
which is what makes this expressible at all: `tb("x")`, `tb("x").w("a")` and
`tb("x").w("a").c(&child)` each parse. So a step can be written as a quote over
the accumulator —

```rust
acc = rs↖↙acc↘.w(↙lit↘)↗;      // instead of  b.write(&format!(".w({})", str_lit(s)))
```

— and generated once per (language, cmd kind) at bootstrap time. The payoff is
that `.c(&x)` vs `.c(x)`, `&[…]` vs `[…]`, and `NL` vs `NL()` stop being facts a
maintainer has to remember in three files: they are whatever that language's
parser said when the sample was quoted.

**Constraint that shapes the design:** the `rust`, `python` and `typescript`
features deliberately do *not* imply `parse` (`quilt-wasm` generates TypeScript on
`wasm32` with no C runtime). So the quoting must happen at bootstrap time and the
committed `ops.rs` must remain parser-free — exactly the `mk_meta.rs.quilt` model,
not a runtime call into `parse_lang`.

Sequence this **after** finding 1. It touches the expander's own output path, so
every snapshot in `quilt/tests/snapshots/` is in the blast radius; doing it second
means the technique is already proven on a lower-risk file.

---

## 3. Arity tables from the grammar

Suggested on the issue: scrape `Arity::Variadic` out of the tree-sitter grammar
JSON instead of hand-maintaining nine `match tag` allowlists.

**This works — but not from the field the obvious scrape reads.** The first
version of this page said "don't", on the grounds that it changes behaviour
across nine languages. That was the wrong test. The right one is whether the
derived tables are *correct*, and running them says they largely are.

### `children.multiple` is the wrong signal

`node-types.json` marks a node `children.multiple` when the grammar lets it hold
more than one child. That sounds like "variadic" and is not: it is true for any
node with several distinct child *slots*. Under it, Rust's `function_item`
becomes variadic — which `conformance/spec/rust.toml` explicitly declares it must
not be, and rightly, since emitting a sequence into a function item is
meaningless. Scraping this field takes Rust from 2 tags to 57 and fails the
conformance battery for rust, wgsl, typescript and lean.

### `REPEAT` is the right one

`grammar.json` records the actual rule structure, so a genuine sequence container
is a node whose rule contains a `REPEAT`/`REPEAT1` over a symbol. Two refinements,
both found by iterating against the test suite rather than by reading:

1. **Follow hidden (`_`-prefixed) rules.** bash's `program` reaches its repeat via
   `_statements`; without inlining, the file root — the single most important
   variadic container for a shell host — drops out of the set.
2. **But not *category* rules.** A hidden rule that is only a `CHOICE` of symbols
   (`_expression`) is a category, not structure. Following it inherits the repeats
   of every alternative and inflates the set (python 30 → 56 spuriously).

With both, the derived set is a strict **superset** of what is declared today for
six of nine languages, and matches html exactly:

| lang | declared | derived | declared but *not* derived |
|---|---|---|---|
| html | 7 | 7 | — |
| rust | 2 | 35 | — |
| python | 2 | 56 | — |
| wgsl | 5 | 18 | — |
| lean | 3 | 57 | — |
| typescript | 10 | 35 | — |
| nix | 5 | 9 | `source_code` |
| bash | 47 | 28 | 19 tags |
| zsh | 42 | 40 | 15 tags |

Nothing currently declared is lost except in bash, zsh and nix — and those losses
are exactly the entries that should go: `raw_string` (which the grammar gives *no*
children at all), `number`, `command_name`, `binary_expression`,
`variable_assignment`, and nix's `source_code`, whose only field is a single
optional expression. A Nix file *is* one expression; emitting a sequence into it
produces invalid Nix.

### What made this look impossible, and why it no longer is

The objection was output size, not semantics. `string_literal` and `token_tree`
really are repeat containers, so scraping turned every string literal in generated
Rust into a six-line accumulator block. That is why the tables were hand-curated:
declaring a tag variadic was *expensive*.

It no longer is. A variadic node with no unquote among its direct children now
builds fluently — same term, none of the accumulator — so the table is free to
follow the grammar. Semantics were never the problem: with the derived tables
installed across all nine languages, `cargo test -p quiltlang` passes end to end,
including the Omni-vs-Bootstrap differential.

### The two calls left

Tracked in [#202](https://github.com/QuiltLang/quilt/issues/202), because they are
design decisions rather than mechanical work:

1. **Adopt the ~150 newly-derived tags?** rust 2 → 35, python 2 → 56, lean 3 → 57.
   This is an expressiveness gain — emitting into `arguments`, `array_expression`,
   `parameters`, `match_block` — but it also widens where the emit heuristic fires.
2. **Drop bash/zsh/nix's non-repeat entries?** `conformance/spec/bash.toml`
   currently *requires* `list`, `function_definition` and `variable_assignment` to
   be variadic, so dropping them moves declared claims.

Three tags stay contested under any rule: `wgsl::function_declaration` and
`typescript::lexical_declaration` are real repeats (of attributes / declarators)
that the specs say must not be variadic, and `lean::by` depends on which hidden
rules get inlined.

### Not a Quilt-dogfooding target

Worth saying, since this page is about dogfooding: the *generator* here should not
be a `.quilt` file. It is a JSON-to-table transform with no interesting structure —
the same reasoning that leaves `omni.rs` as a `macro_rules!`. It belongs next to
`gen-matrix` in `quilt-conformance`, which already depends on `serde_json`.

---

## 4. String-based emitters (nix, lean)

`nix/ops.rs` and `lean/ops.rs` reconstruct a fragment as a host *string literal*,
mapping Quilt's unquote onto the host's own interpolation (`${x}` / `{x}`). They
are near-clones of each other (`diff` = 157 lines of 331).

**Not a Quilt candidate.** Their output is a string, not a term: there is no
target-language AST for a quote to be checked against, and the escaping rules
(`\${`, `\"`) are exactly the part a quote would have to bypass anyway. The
shared-ness is real but it is ordinary Rust deduplication — one
`StringMeta { escape, interp_open, interp_close }` parameterisation — not a
metaprogramming problem.

---

## 5. The Omni registry

`omni.rs` is 528 lines, of which ~330 are one `macro_rules! define_omni` that
generates six registries and three enum-dispatch impls from a ten-line table.

**Leave it.** This is repetition-with-substitution over a *static* table, with
`#[cfg(feature = …)]` interleaved at every expansion site — the case
`macro_rules!` handles natively and cheaply. Routing it through Quilt would add a
generated file, a bootstrap step, and a `cargo fmt` pass to buy nothing; Quilt's
edge is *computation* during generation (reading files, arithmetic, cross-language
output), and there is none here. Worth stating explicitly, because "we have a
metaprogramming system, so use it for all the metaprogramming" is the obvious
wrong conclusion to draw from this page.

---

## 6. `strlift.rs` and the bootstrap floor

`langs/bootstrap/strlift.rs` is a fourth copy of the Rust emitter, written with
`format!` and a string round-trip.

**It has to stay hand-written.** It is the base case: `bootstrap0` expands
`mk_meta.rs.quilt` with `BootstrapMetaLanguage`, which is what exists *before*
`meta.rs` does. Generating it with Quilt would close a loop with no floor. It is
correctly hand-written and correctly slow.

---

## 7. What is not a candidate

Two files rank high on a naive `grep -c 'format!'` and should be ruled out
explicitly so they are not re-audited later:

* **`quilt-conformance/src/battery.rs`** (71 hits) — every one is a diagnostic
  message (`"arity({tag:?}) is {other:?}, spec says Variadic"`). No generated code.
* **`quilt-conformance/src/matrix.rs`** (14 hits) — Markdown table assembly for
  `docs/wiki/support-matrix.md`. Quilt could only reach this through the `text`
  language, and a Markdown table is not a tree anyone benefits from typing.

One layering note that is adjacent but not a dogfooding item:
`qmatch::pattern_var_code` / `pattern_let_code` emit **Rust** source
(`mvar("x")`, `qmatch_n(&p, &v)`) from the language-agnostic core rather than from
`langs/rust/ops.rs`. That is why pattern matching is Rust-only, and it is a
plain move-the-function refactor rather than anything Quilt is needed for.

---

## Found while auditing: a ground unquote holding statements was emitted, not spliced

Building the finding-1 prototype turned up an expander bug, since fixed. A ground
unquote whose body holds **several** statements was wrapped as a value to emit
rather than spliced as code to run, appending `.emit(&mut b_);` after the body's
last statement:

```rust
let out: ⟨T⟩ = ↖
    fn keep() {}

    ↙
        let n = 3;
        ↖fn made() {}↗.←;
    ↘
↗;
```

```rust
tb("function_item")…b().emit(&mut b_);.emit(&mut b_);   // does not parse
```

Two things were wrong in the same decision. The body was classified with the
*quoted* language rather than the ground one — `html↖<input value="↙w↘">↗` was
asking HTML whether the Rust expression `w` is a statement — and the test was
`is_stmt_like`, a single-node question, so a statement *sequence* (which parses
with the file root) read as "not code".

It went unnoticed because it is invisible whenever the last statement is
block-shaped (`for`, `if`, `{…}`): those are valid method receivers evaluating to
`()`, so the stray call landed on the no-op `impl Emit for ()`. **`mk_meta.rs.quilt`
ends its unquote with a `for`**, so the bootstrap had been generating
`}.emit(&mut b_);` and bootstrapping straight past it. Anyone whose unquote ends
in anything else got a `rustc` parse failure on a temp file with no Quilt
diagnostic.

The two halves are not separable: `Language::typ` defaults to `InnerKind::File`,
so widening the test while still asking the quoted language reads every
non-classifying target (html, wgsl, bash, zsh) as "always code".

## Status

| | |
|---|---|
| Ground-unquote splice bug | fixed — [#197](https://github.com/QuiltLang/quilt/pull/197) |
| Hole-free variadic nodes build fluently | [#201](https://github.com/QuiltLang/quilt/pull/201) — prerequisite for finding 3 |
| Derive the arity tables (finding 3) | [#202](https://github.com/QuiltLang/quilt/issues/202) — two calls left |
| Lift shapes match the parsers | mostly fixed by hand — #176, `fed278b`, guarded by `lift_fidelity.rs` |
| Generate `lift.rs` (finding 1) | [#203](https://github.com/QuiltLang/quilt/issues/203) — stops the shapes re-drifting; two cases still need the #174 call |
| Generate the `ops.rs` fragments (finding 2) | [#204](https://github.com/QuiltLang/quilt/issues/204) — do last |

Suggested order from here:

1. **Decide the remaining #174 faithfulness question** — the two cases `fed278b`
   skipped because they move a declared conformance tag (`wgsl::int_literal` →
   `const_literal`, `python::integer` → `unary_operator` for `-7`).
2. **Make the two calls in [#202](https://github.com/QuiltLang/quilt/issues/202).**
   The derivation is written and verified; what is left is whether to adopt the
   newly-derived tags and whether to move the bash/zsh spec claims.
3. **Generate `lift.rs`** (finding 1). Highest value: it removes the largest
   hand-written table *and* makes a class of #174 bug unrepresentable.
4. **Generate the `ops.rs` fragment shapes** (finding 2). Last: it is the
   expander's own output path, so every snapshot is in scope.
