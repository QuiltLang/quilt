# Adding a Language

This guide walks through adding a new language to Quilt. It uses the pattern established by the existing concrete languages (`rust`, `python`, `typescript`, `html`, `wgsl`, `sql`, `zsh`, `bash`, `nix`, `lean`, `text`).

## 1. Decide the role

A language can be:

- **Host language** — the ground language in a `.quilt` file. Requires both a `Language` *and* a `MetaLanguage` implementation. Example: Rust, Python, TypeScript.
- **Target language** — only appears inside `lang↖…↗` quotes. Requires only a `Language`. Example: WGSL, SQL.
- **Both.** Nix, Lean, bash and zsh are quotable targets *and* hosts; HTML is a quotable target and an identity host.

Independently of either, a language can have a **machine** — a stateful evaluator that `quilt repl`, notebook cells and `lang⟨M⟩` drive (step 8).

If you want a host, decide which kind of `MetaLanguage` it is:

- **Runtime-backed** (Rust, Python, TypeScript) — the expanded code calls into a `QTerm` builder library that Quilt ships for that language. Highest fidelity (structural manipulation, pattern matching), but you have to *write and distribute that runtime*.
- **String-based** (Nix, Lean, bash, zsh) — no runtime at all: `meta.rs`/`ops.rs` reconstruct each fragment as a string expression in the host, mapping Quilt's unquote onto the host's own string interpolation. Check whether the host's expressions carry their own interpolation sigil before reaching for a wrapper: Nix and Lean need one (`${x}`, `{x}`) because a bare expression has none, while a shell expansion already does, so `langs/shell/ops.rs` splices verbatim and wrapping would be a syntax error. Far less work, language-agnostic in what it can generate, but there is no `b_` accumulator, so emit/splice in *ground* loops is unsupported — sequences must be built functionally. That is a limit on how the sequence is *built*, not proof the host cannot emit: give `emit_str` the functional reading if the host has one, joining a list of fragments into the surrounding container (nix spells it `builtins.concatStringsSep "\n"` — see issue #155). Only where there is no meaning at all should a spelling return an error, so the glyph is rejected rather than leaking a `__EMIT__` placeholder into the output — as `reduce_str` still does in every string host (`↓` needs a `QTerm` runtime to evaluate a fragment). Override the remaining spellings with whatever the string model *does* mean: a fragment is a string, so `type_str` is the host's string type and `name_str` its identity function — unless the host has no way to *write* either, as the shells do not, in which case those refuse too. Every spelling accessor returns `Result` precisely so "this host has no such thing" is expressible. See `langs/nix/ops.rs`, `langs/lean/ops.rs` and `langs/shell/ops.rs`.
- **Identity** (text, HTML) — the meta *holds* the object-level code rather than translating it, so a file expands to itself with its quotes verbatim, and every operator glyph is refused. Use `langs::hold::HoldMetaLanguage<D>` with a two-line `Holds` dialect marker (`langs/html/meta.rs` is the whole of it). Worth it when the file's computation lives somewhere else — HTML's lives in its machine.

## 2. Grammar

If the language needs tree-sitter parsing (recommended):

1. Fork or adapt an existing tree-sitter grammar for the language.
2. Add a **hole node** to the grammar. Rust uses `{}` and Python uses `__HOLE__` as hole tokens; your grammar needs a token that is syntactically valid in expression/statement position and uniquely recognizable. Most forks spell it `__QUILT_HOLE__`.

   **Check first whether you need a patch at all.** Nix, Lean and SQL all spell the hole `__QUILT_HOLE__`, which already matches their identifier regexes, so it parses in every position an identifier may appear — no grammar change, and the range-based hole detection in `treesitter.rs` recognises it by byte range regardless of node kind. This is much the cheapest option; see issue #133 for exactly how far it gets you in Lean (everywhere except top-level command position).

   It gets you further than it used to. A hole no longer has to *be* a node: where the surrounding token swallows the marker — inside a string, inside a comment, or glued to neighbouring text — `TSLanguage` (`treesitter.rs`) splits that token around it (issue #221). So "the marker is a valid identifier" now covers string interiors too, for free, in every language. Do not add a grammar rule to buy that.

   If you do patch, where you *reference* the token is the real design decision. Reaching it from more positions is more expressive, but a hole viable in two different roles at once creates genuine parse ambiguity — the first Lean attempt made the hole both an identifier operand and a command, which conflicted with every command ending in a greedy identifier list, cost `prec.right` on eight rules, and blew the parse table past 2.5 GB during generation. Prefer the smallest set of positions that covers your use cases.

   Also check whether your language's *root* node accepts the fragments you want to quote. Lean's `module` holds commands, not terms, so a bare term (`lean↖n + 1↗`) does not parse as a whole file the way it does in Rust or Python. `LeanLanguage` handles this by retrying a failed parse inside a synthetic `#check …` command and stripping the wrapper off the resulting `QTerm` (SQL does the same with `SELECT …`) — worth copying if your language has the same shape. For pieces that are valid only *inside* a construct (a match arm, an enum variant, a constructor) you need no code of your own: list the constructs in `TSProvider::fragment_wrappers` and `TSLanguage` retries the fragment inside each (see `RustProvider` and `LeanProvider`).
3. Host the grammar as its own repo under the [QuiltLang](https://github.com/QuiltLang) GitHub org, following the same structure as the existing forks.
4. Add it to `[workspace.dependencies]` in the root `Cargo.toml`, pinned to an explicit `rev`, like the existing `tree-sitter-*` forks. Nothing depends on that entry as a crate; it is the pin `bin/sync-grammars` reads.
5. **Vendor the generated parser.** `quiltlang` does not depend on the forks as crates — that would pull git dependencies crates.io rejects (issue #32). Instead it compiles the generated C directly:
   - Add the language to the loop in `bin/sync-grammars` (and to its `highlights.scm` case list if `quilt-lsp` should highlight it), then run it. This clones the fork at its pinned rev and copies `parser.c`/`scanner.c`, the headers and `grammar.json` into `quilt/grammars/<lang>/`. Commit that directory. (`grammar.json` is not compiled; it is what `bin/gen-arity` derives your tables from, in step 3.)
   - Add the language to the feature loop in `quilt/build.rs`, which compiles it.
   - Add a `grammar!` line in `quilt/src/grammars.rs` exposing `LANGUAGE` — `grammar!("your_lang", your_lang, LANGUAGE, tree_sitter_your_lang)` — with a trailing `highlights` to also expose `HIGHLIGHTS_QUERY`.

   `bin/check-grammars` (CI, and `main check --all`) fails if the vendored copies drift from the pins.

If the language doesn't need tree-sitter, implement `Language` directly, as `langs/text/lang.rs` does: it records the literal runs and holes between them as one `text` tuple.

## 3. Create the language module

Create `quilt/src/langs/<lang>/`:

```
langs/<lang>/
├── mod.rs
├── lang.rs     # Language implementation
├── meta.rs     # MetaLanguage (only for host languages)
└── ops.rs      # the code-building helpers meta.rs delegates to (hosts, usually)
```

### `lang.rs`

For a tree-sitter-backed language, use `TSLanguage<YourProvider>`. `langs/nix/lang.rs` is a short, complete example:

```rust
pub struct YourProvider(tree_sitter::Parser);

impl Default for YourProvider {
    fn default() -> Self {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&crate::grammars::your_lang::LANGUAGE.into())
            .expect("Error loading YourLang parser");
        Self(parser)
    }
}

impl TSProvider for YourProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser { &mut self.0 }

    fn hole_str(&self) -> &'static str {
        "__QUILT_HOLE__"  // must lex as an identifier (or match your grammar's hole token)
    }

    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        // Strip the root "source_file" wrapper and infer whether the
        // content is an expression, statement, or file. Return an error,
        // never panic, on a shape you do not handle.
        // See nix/lang.rs, rust/lang.rs or python/lang.rs for reference.
        todo!()
    }

    // Don't write these tables. Once your language is in the conformance
    // registry (step 10), `bin/gen-arity` derives both from your grammar's
    // REPEAT rules and writes them to quilt/src/langs/arity.rs (issue #202).
    fn arity(&self, tag: &str) -> Arity {
        Arity::from_table(crate::langs::arity::YOUR_LANG, tag)
    }
    fn separator(&self, tag: &str) -> Option<&'static str> {
        crate::lang::comma_separator(crate::langs::arity::YOUR_LANG_COMMA_SEPARATED, tag)
    }

    fn typ(&self, tag: &str) -> InnerKind {
        // Map node tags onto Expr / Stmt / Item / Block / File. Override
        // `hole_kind` too if a hole's kind depends on where it sits rather
        // than on its tag (Rust's body-position `block`, Lean's holes).
        todo!()
    }

    fn hashbang(&self) -> Option<&'static str> {
        // Only needed for host languages that are runnable via `quilt`.
        None
    }
}

pub type YourLanguage = TSLanguage<YourProvider>;
pub type DynYourLanguage = DynTSLanguage<YourProvider>;

// How your language spells comments. Required: the registry lookups
// `langs::line_comment` / `langs::header_comment` are generated from the `Omni`
// table below and will not compile without it (issue #194). Both default to
// `//`; override `LINE` for anything else, and `HEADER` only where a generated
// file's banner wants a different form — Rust's is `//!`, an inner doc comment.
impl Comments for YourLanguage {
    const LINE: Option<&'static str> = Some("#");
}
```

If your comments are *delimited* rather than prefixed, say so with `const LINE:
Option<&'static str> = None`: a prefix alone would open a comment that never
closes, and every caller has a fallback for the languages that decline. HTML is
the worked example. The same answer feeds quilt-lsp, which takes each adapter's
`CommentSyntax::line` from here rather than declaring its own.

### `meta.rs` (host languages only)

Implement `MetaLanguage`. The three required methods build *code* that reconstructs the term at runtime:

```rust
#[derive(Default)]
pub struct YourMetaLanguage;

impl MetaLanguage for YourMetaLanguage {
    fn expand_quote(&self, lang1, tag, i, lang2, qterm, cmds) -> Result<Arc<QTerm>> {
        // Build code: quote(tag, i, lang2, <term>, [..cmds..])
        Ok(build_quote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_unquote(&self, lang1, tag, i, lang2, qterm, cmds) -> Result<Arc<QTerm>> {
        Ok(build_unquote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_tuple(&self, lang1, tag, qterms, cmds, arity) -> Result<Arc<QTerm>> {
        Ok(if arity == Arity::Variadic {
            build_variadic_block(tag, cmds, qterms)
        } else {
            build_tuple_code(tag, cmds, qterms)
        })
    }

    // Override operator spellings. Every one returns `Result`: spell the glyph
    // where your host has a meaning for it and return an actionable error where
    // it does not, so no `__LIFT__`-style placeholder reaches the output.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "your_lang" => Ok("your_lift"),
            _ => miette::bail!("your_lang can't lift into {target:?}"),
        }
    }
    fn reduce_str(&self, target: &str) -> Result<&'static str> { … }
    fn emit_str(&self) -> Result<&'static str> { … }
    fn type_str(&self) -> Result<&'static str> { … }
    fn name_str(&self) -> Result<&'static str> { … }

    // The runtime import an expanded file opens with, if your host calls one
    // (`use quilt::prelude::*;`, `from quilt import *`). Injected by
    // `quilt expand` / `quilt run`, so authors no longer type it.
    fn prelude(&self, targets: &[&str]) -> Option<Prelude> { … }
}
```

See [Language Traits](language-traits.md#metalanguage--expansion) for the rest of the trait — method-position spellings and `⟨M⟩` for driving machines, pattern-lets, and the `⟨str⟩`/`⟨qt⟩` quote representations — all of which default to declining, so a host need only implement what it supports.

You can reuse `langs::rust::ops` or `langs::python::ops` helpers if your meta-language emits Rust or Python constructor code. A runtime-backed host whose builder calls are a method chain can also share `langs::chain`, the builder-call fold Rust, Python and TypeScript use: its per-language fragments (`.c(&x)` against `.c(x)`, …) are a table `bin/gen-chain` generates from samples parsed by each language's grammar, and `bin/check-chain` is the drift gate.

## 4. Add a Cargo feature

Name the feature after the language's canonical name: `bin/gen-arity` gates each generated table on `#[cfg(feature = "<name>")]`. In `quilt/Cargo.toml` (tree-sitter languages must also enable `parse`; there is no grammar crate to depend on, since the parser is vendored):

```toml
[features]
your_lang = ["parse"]
default = [..., "your_lang"]
```

A language whose *runtime* half needs no parser — `rust`, `python`, `typescript` — leaves `parse` out, so the runtime-only build (`default-features = false`, used on `wasm32`) can still enable it; `bin/check-features` builds each language feature on its own.

## 5. Register in `langs/mod.rs`

```rust
#[cfg(feature = "your_lang")]
pub mod your_lang;
```

## 6. Add to `Omni`

The whole `Omni` surface — the registries, the enum-dispatch wrappers, `DynOmniLanguages`/`DynOmniMetaLanguages`, `dict_omni_language()` and the `line_comment`/`header_comment` lookups — is generated by `define_omni!` from one table at the bottom of `langs/omni.rs` (see [Multi and Omni](multi-omni.md#omni)). Import your types at the top of the file, then add one line to the `languages` section:

```rust
your_lang if "your_lang" => YourLang(YourLanguage, DynYourLanguage): ["your_lang", "yl"];
```

The first string is the canonical name, the rest are aliases; all of them resolve to the one instance. For a host, add the matching line to the `metas` section:

```rust
your_lang if "your_lang" => YourLang(YourMetaLanguage): ["your_lang", "yl"];
```

Leave a language out of `metas` and it is a target only. The `Comments` impl from step 3 is what makes the table compile.

## 7. Make it a lift target (optional)

For `↑` to lift a host value into a quote of your language (`your_lang↖ … ↙x.↑↘ … ↗`):

1. Add a marker type in `quilt/src/lift/mod.rs` (`pub struct YourLang;`) and re-export it from `quilt/src/prelude.rs`, where the other markers are.
2. Add sample-literal rows for it to `quilt/src/lift/mk_lifts.rs.quilt` and run `bin/gen-lifts`, which writes the `LiftTo<YourLang>` impls into `quilt/src/lift/gen.rs`. Don't write the shapes by hand: the generator parses each sample with your vendored grammar and lifts the parsed term, so the tags and layout are the parser's (issue #203). Escaping rules the shapes call (like `sh_dquote_escape`) stay hand-written in `lift/mod.rs`. `bin/check-lifts` is the drift gate.
3. Teach each host to spell the lift: Rust's spellings are `langs::rust::ops::lift_spelling` (`qlift_to::<YourLang>()`); Python's and TypeScript's are their `lift_str`, backed by a function in their runtime.

## 8. Give it a machine (optional)

A machine lets `quilt repl`, notebook cells and `lang⟨M⟩` run your language (see [Language Traits](language-traits.md#machines--running-a-language) and `docs/design/machines.md`). The specs are data, kept in `quilt/src/machine.rs`, not in your module:

1. Add your language to the table that fits: `repl_spec` for an interpreter that can be kept alive on stdin (it needs a "print this value" and a sentinel-echo spelling), `script_spec` for one that must be replayed per feed, or `native_machine` for an in-process evaluator. Add its names to `REGISTERED_SPECS`.
2. Delegate the matching `Language` hook to it — `fn repl_spec(&self) -> Option<ReplSpec> { crate::machine::repl_spec("your_lang") }` — on your provider.
3. Add a `[machine]` probe to your conformance spec (step 10). The battery fails if a language registers a machine and the spec has no probe, or the reverse.

## 9. Add to the LSP adapters

`quilt-lsp` projects each language of a `.quilt` file into a virtual document. Add a `your_lang = ["quilt/your_lang"]` feature to `quilt-lsp/Cargo.toml` (and to its `default`); the adapter needs the matching quilt language enabled, or the lookups below return `None` at runtime. Then, in `quilt-lsp/src/adapters.rs`:

1. Add a `LanguageAdapter` impl defining:
   - `language_id()`, `virtual_extension()` and `server_command()` — the downstream server, or `None` for a highlight-only language.
   - `splice_placeholder()` — the identifier that masks holes inside a fragment.
   - `wrap_fragment(n)` — the prologue/epilogue that wraps a quoted fragment so the downstream server parses it.
   - `comment_syntax()` — how to write placeholder comments. Take `line` from
     `quilt::langs::line_comment(<your language>)` rather than spelling it
     again; only the block delimiters are the adapter's own, because for a
     language with no block comment they are a projection workaround rather
     than a fact about the language.
2. For a host, add a `MetaLanguageAdapter` impl for the ground projection (`glyph_placeholder()`, `splice_block()`, `find_root()`).
3. Register the adapters in `language_adapter()` (and `meta_adapter()` for a host), and add a target to `embedded_adapters()` so its fragments are projected per-fragment. Highlighting comes from the grammar's `HIGHLIGHTS_QUERY` (step 2).

## 10. Declare it in the conformance matrix

**This is the step that produces your tests.** Rather than hand-writing an
`expand_<lang>.rs`, you declare what your language can do in
`conformance/spec/<lang>.toml`, and the shared battery in `quilt-conformance/`
turns each claim into a probe. See [Support Matrix](support-matrix.md) for the
rendered result and [issue #144](https://github.com/QuiltLang/quilt/issues/144)
for the design.

First register the language with the harness, in `quilt-conformance/src/registry.rs`: its canonical name in `LANGUAGES` (and in `HOSTS` if it has a meta), and an arm in `language()`, `grammar()` and, for a host, `meta()`. The registry is also what `bin/gen-arity` walks, so this is what gets your `arity`/`separator` tables generated (step 3). Then add it to the `language_tests!` list in `quilt-conformance/tests/conformance.rs`, which gives it its `#[test]`; `every_language_has_a_spec` and `host_list_matches_registry` fail until the pieces agree.

```sh
cp conformance/spec/wgsl.toml conformance/spec/your_lang.toml   # nearest template
$EDITOR conformance/spec/your_lang.toml
bin/gen-arity         # derive your variadic / comma-separated tables
bin/gen-matrix        # verify the claims, regenerate the matrix
```

The spec is required to answer **every** axis in `Axis::ALL` — a missing key is a
hard error. That is deliberate: it is what makes the checklist below unskippable
rather than something a new language can quietly omit, which is how `bash`, `zsh`
and `text` previously ended up with no coverage at all.

What you declare, and what the battery does with it:

| Spec key | What the probe checks |
|---|---|
| `[[fragments]]` | parses, round-trips to identical source, produces the declared root tag, is structurally sound (every child has a hole to be written into), and reparses idempotently |
| `[[holes]]` | each `@` marker lands in a hole with the declared `InnerKind` |
| `[kinds]` | `Language::typ` classifies each tag as declared |
| `variadic` / `not_variadic` | `Language::arity` agrees — including the negative cases, since over-declaring variadicity silently changes emit behaviour. Both are claims about your *grammar*, since the table is derived from it: `variadic` says the rule has a repeat over direct children, `not_variadic` says it does not |
| `lift_marker` + `[[lift]]` | values lift to the declared tag and text, **and the lifted literal reparses in your grammar** — the check that catches escaping bugs |
| `lift_from` / `lift_from_unsupported` | your `MetaLanguage::lift_str` spells exactly the targets you claim, and refuses the rest |
| `[[glyphs]]` | each Quilt glyph you declare as your language's own syntax is one Quilt actually reserves, `\<glyph>` escapes it, and the fragment you supply parses in your language with the declared tag — checked again end-to-end through a real quote, from every host |
| `comment_prefix` | the `DO NOT EDIT` header for a generated file of your language — your `Comments::HEADER`, read back through `quilt::langs::header_comment` — is what you declared, and every alias of your language agrees. Separately, `comment_tags_are_recognised` builds a comment from your `Comments::LINE` and checks your grammar tags it `comment` or `*_comment`, the convention `lang::is_comment_tag` relies on |
| `[meta]` (hosts) | your operator spellings — `emit`, `type_str`, `name_str`, `reduce` per target, the pattern-let tag — are exactly these, and each refusal (`emit_error`, `type_error`, …) contains the text you give, so it is checked for being *actionable* |
| `[cross]` (hosts) | how your host embeds a quote (`wrapper`) and spells a lift (`lift`), so the cross-language grid can drive every target from it |
| `[machine]` | the machine your `Language` declares, spawned the way users get it, obeys the three machine laws: the fed `define` is visible to the `query`, which answers `answer`; that literal, queried, answers itself; and a fresh machine does not see the definition. Present exactly when you registered a machine (step 8) |
| `[capabilities]` | each claim matches reality; `partial`/`unsupported` must carry a `note`, and `partial`/`planned` a tracking `issue` |

Three rules worth knowing before you start:

- **A panic is always a failure**, whatever the claimed status. An `unsupported`
  capability must return a clean `Err`, never `todo!()`. (This is what finally
  surfaced the `TextLanguage` stub that issue #11 left behind.)
- **Over-claiming and under-claiming both fail.** Declaring `supported` for
  something broken fails; so does declaring `unsupported` for something that
  works, which keeps the spec from rotting after you fix a gap.
- **Answer the glyph question honestly.** `glyph-collisions` asks which Quilt
  glyphs are also your language's own syntax. Lean spells monadic bind `←` —
  the same glyph as emit — and that cost a real bug
  ([#141](https://github.com/QuiltLang/quilt/issues/141)) precisely because
  nothing asked the question when Lean landed. Half the answer is a declaration
  no probe can check: whether `↑` *means* anything in your language is a fact
  about your language. The other half is `[[glyphs]]`, one entry per colliding
  glyph with a real fragment using it, and the status has to agree with the
  list — `supported` means the list is empty, `partial` means it is not
  ([#195](https://github.com/QuiltLang/quilt/issues/195)).

### Your spec also generates property tests

The same file drives `quilt-conformance/tests/properties.rs`
([#161](https://github.com/QuiltLang/quilt/issues/161)), so declaring a
`lift_marker` and `[[lift]]` probes buys more than the six literals you wrote
down: each probe declares a *row* of the lift grid — a Rust type plus, where it
changes the spelling, the sign or value — and the property then generates
arbitrary values of that row and requires every one of them to lift to the
declared tag and reparse in your grammar. That is the escaping net, and it is
where a target's quoting rules meet input nobody thought to write by hand.

Two consequences when you add a language:

- **Add a `lift_arbitrary` arm** in `properties.rs` for every row your spec
  declares. A declared row with no arm is a test failure, not a silent skip —
  the spec is what says the cell exists.
- **Declare both spellings when the value picks one.** Lean gives `false` its
  own tag (`false_const`, not `true_const`) and lifts a negative integer as a
  `unary_op`; Rust lifts a negative float as a `unary_expression`. If your
  language does something similar, write the second `[[lift]]` probe —
  otherwise half that row's domain goes unchecked.

Nothing here needs a nightly toolchain: the properties run in the ordinary
`cargo test`. The `bin/fuzz` targets are separate and are not per-language.

Add ordinary `#[test]`s too, for anything the battery's shape does not cover
(unusual recovery paths, language-specific expander behaviour):

```sh
cargo test -p quiltlang your_lang
cargo test -p quilt-conformance your_lang
```

### Pin your refusals in `quilt/tests/ui/`

The matrix records *that* an operator is unsupported; `quilt/tests/ui/` records
what the user is told when they try it. Every `unsupported` capability you
declared above should have a case here — the message is the only thing standing
between a contributor and a mystery, and it is the part that rots silently.

A case is one file holding the smallest input that provokes the error, named
`<what>.<chain>.quilt` so the extensions are the language chain, exactly as on
the command line:

```sh
printf 'def gen : String := lean↖[↙←frags↘]↗\n' > quilt/tests/ui/lean_emit.lean.quilt
cargo insta review                     # accept the rendered diagnostic
$EDITOR quilt/tests/ui.rs              # add the file to the `corpus_is_complete` roster
```

The rendered `miette` output — message, source snippet, caret position and help
— is snapshotted, so improving an error message is a reviewable diff rather than
an invisible change. A case that stops failing is itself a test failure.
