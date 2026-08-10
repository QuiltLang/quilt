# Adding a Language

This guide walks through adding a new language to Quilt. It uses the pattern established by the existing concrete languages (`rust`, `python`, `html`, `wgsl`, `zsh`, `bash`, `nix`, `lean`, `text`).

## 1. Decide the role

A language can be:

- **Host language** — the ground language in a `.quilt` file. Requires both a `Language` *and* a `MetaLanguage` implementation. Example: Rust, Python.
- **Target language** — only appears inside `lang↖…↗` quotes. Requires only a `Language`. Example: HTML, WGSL.
- **Both.** Nix, Lean, bash and zsh are quotable targets *and* hosts.

If you want a host, decide which kind of `MetaLanguage` it is:

- **Runtime-backed** (Rust, Python) — the expanded code calls into a `QTerm` builder library that Quilt ships for that language. Highest fidelity (structural manipulation, pattern matching), but you have to *write and distribute that runtime*.
- **String-based** (Nix, Lean, bash, zsh) — no runtime at all: `meta.rs`/`ops.rs` reconstruct each fragment as a string expression in the host, mapping Quilt's unquote onto the host's own string interpolation. Check whether the host's expressions carry their own interpolation sigil before reaching for a wrapper: Nix and Lean need one (`${x}`, `{x}`) because a bare expression has none, while a shell expansion already does, so `langs/shell/ops.rs` splices verbatim and wrapping would be a syntax error. Far less work, language-agnostic in what it can generate, but there is no `b_` accumulator, so emit/splice in *ground* loops is unsupported — sequences must be built functionally. That is a limit on how the sequence is *built*, not proof the host cannot emit: give `emit_str` the functional reading if the host has one, joining a list of fragments into the surrounding container (nix spells it `builtins.concatStringsSep "\n"` — see issue #155). Only where there is no meaning at all should a spelling return an error, so the glyph is rejected rather than leaking a `__EMIT__` placeholder into the output — as `reduce_str` still does in every string host (`↓` needs a `QTerm` runtime to evaluate a fragment). Override the remaining spellings with whatever the string model *does* mean: a fragment is a string, so `type_str` is the host's string type and `name_str` its identity function — unless the host has no way to *write* either, as the shells do not, in which case those refuse too. Every spelling accessor returns `Result` precisely so "this host has no such thing" is expressible. See `langs/nix/ops.rs`, `langs/lean/ops.rs` and `langs/shell/ops.rs`.

## 2. Grammar

If the language needs tree-sitter parsing (recommended):

1. Fork or adapt an existing tree-sitter grammar for the language.
2. Add a **hole node** to the grammar. Rust uses `{}` and Python uses `__HOLE__` as hole tokens; your grammar needs a token that is syntactically valid in expression/statement position and uniquely recognizable. Most forks spell it `__QUILT_HOLE__`.

   **Check first whether you need a patch at all.** Nix and Lean both spell the hole `__QUILT_HOLE__`, which already matches their identifier regexes, so it parses in every position an identifier may appear — no grammar change, and the range-based hole detection in `treesitter.rs` recognises it by byte range regardless of node kind. This is much the cheapest option; see issue #133 for exactly how far it gets you in Lean (everywhere except top-level command position).

   It gets you further than it used to. A hole no longer has to *be* a node: where the surrounding token swallows the marker — inside a string, inside a comment, or glued to neighbouring text — `build_nodes` splits that token around it (issue #221). So "the marker is a valid identifier" now covers string interiors too, for free, in every language. Do not add a grammar rule to buy that.

   If you do patch, where you *reference* the token is the real design decision. Reaching it from more positions is more expressive, but a hole viable in two different roles at once creates genuine parse ambiguity — the first Lean attempt made the hole both an identifier operand and a command, which conflicted with every command ending in a greedy identifier list, cost `prec.right` on eight rules, and blew the parse table past 2.5 GB during generation. Prefer the smallest set of positions that covers your use cases.

   Also check whether your language's *root* node accepts the fragments you want to quote. Lean's `module` holds commands, not terms, so a bare term (`lean↖n + 1↗`) does not parse as a whole file the way it does in Rust or Python. `LeanLanguage` handles this by retrying a failed parse inside a synthetic `#check …` command and stripping the wrapper off the resulting `QTerm` — worth copying if your language has the same shape.
3. Host the grammar as its own repo under the [QuiltLang](https://github.com/QuiltLang) GitHub org, following the same structure as the existing forks.
4. Add it to `[workspace.dependencies]` in the root `Cargo.toml`, pinned to an explicit `rev`, like the existing `tree-sitter-*` forks.
5. **Vendor the generated parser.** `quiltlang` does not depend on the forks as crates — that would pull git dependencies crates.io rejects (issue #32). Instead it compiles the generated C directly:
   - Add the language to the loop in `bin/sync-grammars` (and to its `highlights.scm` case list if `quilt-lsp` should highlight it), then run it. This clones the fork at its pinned rev and copies `parser.c`/`scanner.c` into `quilt/grammars/<lang>/`. Commit that directory.
   - Add the language to the feature loop in `quilt/build.rs`, which compiles it.
   - Add a `grammar!` line in `quilt/src/grammars.rs` exposing `LANGUAGE` (add the `highlights` variant to also expose `HIGHLIGHTS_QUERY`).

   `bin/check-grammars` (CI) fails if the vendored copies drift from the pins.

If the language doesn't need tree-sitter, implement `Language` directly (see the `bootstrap/lang.rs` approach).

## 3. Create the language module

Create `quilt/src/langs/<lang>/`:

```
langs/<lang>/
├── mod.rs
├── lang.rs     # Language implementation
└── meta.rs     # MetaLanguage (only for host languages)
```

### `lang.rs`

For a tree-sitter-backed language, use `TSLanguage<YourProvider>`:

```rust
pub struct YourProvider(tree_sitter::Parser);

impl Default for YourProvider {
    fn default() -> Self {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_your_lang::LANGUAGE.into()).unwrap();
        Self(parser)
    }
}

impl TSProvider for YourProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser { &mut self.0 }

    fn hole_str(&self) -> &'static str {
        "__HOLE__"  // must match your grammar's hole token
    }

    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        // Strip the root "source_file" wrapper and infer whether the
        // content is an expression, statement, or file.
        // See rust/lang.rs or python/lang.rs for reference.
        todo!()
    }

    fn arity(&self, tag: &str) -> Arity {
        // Don't write this table. Add your language to `derive_all` in
        // quilt-conformance/src/arity.rs, run `bin/gen-arity`, and point at the
        // table it generates from your grammar's REPEAT rules (issue #202).
        Arity::from_table(crate::langs::arity::YOUR_LANG, tag)
    }

    fn hashbang(&self) -> Option<&'static str> {
        // Only needed for host languages that are runnable via `quilt`.
        None
    }
}

pub type YourLanguage = TSLanguage<YourProvider>;
pub type DynYourLanguage = DynTSLanguage<YourProvider>;
```

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

    // Override operator spellings if needed:
    fn lift_str(&self)   -> &'static str { "your_lift()" }
    fn reduce_str(&self) -> &'static str { "your_reduce()" }
}
```

You can reuse `langs::rust::ops` or `langs::python::ops` helpers if your meta-language emits Rust or Python constructor code.

## 4. Add a Cargo feature

In the root `Cargo.toml`:

```toml
[workspace.dependencies]
tree-sitter-your-lang = { git = "https://github.com/QuiltLang/tree-sitter-your-lang.git" }
```

In `quilt/Cargo.toml` (tree-sitter languages must also enable `parse`):

```toml
[features]
your_lang = ["dep:tree-sitter-your-lang", "parse"]
default = [..., "your_lang"]

[dependencies]
tree-sitter-your-lang = { workspace = true, optional = true }
```

## 5. Register in `langs/mod.rs`

```rust
#[cfg(feature = "your_lang")]
pub mod your_lang;
```

## 6. Add to `Omni`

Edit `langs/omni.rs`:

**In `OmniLanguages`:**

```rust
pub struct OmniLanguages {
    // …existing fields…
    #[cfg(feature = "your_lang")]
    your_lang: OmniLanguage,
}

impl Default for OmniLanguages {
    fn default() -> Self {
        Self {
            // …
            #[cfg(feature = "your_lang")]
            your_lang: OmniLanguage::YourLang(YourLanguage::default()),
        }
    }
}
```

**In `Languages for OmniLanguages`:** add a `"your_lang"` match arm to `get` and `get_mut`.

**If it's a host language, in `OmniMetaLanguages`:** add a field and a match arm for the `MetaLanguage`.

**In `OmniLanguage` enum:** add `YourLang(YourLanguage)` variant and implement the `Language` delegation methods.

**In `dict_omni_language()`:** add `ret.add_lang("your_lang", …)` (and `ret.add_meta` for host languages).

## 7. Update `DynOmniLanguages` (optional)

If you want the language accessible via `dict_omni_language()` (needed for tests and the LSP), also add it to `DynOmniLanguages` and `DynOmniMetaLanguages` in `omni.rs`.

## 8. Add to the LSP adapters (for host languages)

If the language will be a host ground language in the LSP:

1. Add a `LanguageAdapter` impl in `quilt-lsp/src/adapters.rs` defining:
   - `comment_syntax()` — how to write placeholder comments.
   - `splice_block()` — the placeholder for a quote in the projected document.
   - `wrap_fragment(body)` — how to wrap a quoted fragment so the downstream server tokenizes it.
2. Add a `MetaLanguageAdapter` impl for the projection logic.
3. Register the new adapter in `language_adapter()` and `meta_adapter()`.

## 9. Declare it in the conformance matrix

**This is the step that produces your tests.** Rather than hand-writing an
`expand_<lang>.rs`, you declare what your language can do in
`conformance/spec/<lang>.toml`, and the shared battery in `quilt-conformance/`
turns each claim into a probe. See [Support Matrix](support-matrix.md) for the
rendered result and [issue #144](https://github.com/QuiltLang/quilt/issues/144)
for the design.

```sh
cp conformance/spec/wgsl.toml conformance/spec/your_lang.toml   # nearest template
$EDITOR conformance/spec/your_lang.toml
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
  nothing asked the question when Lean landed.

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
