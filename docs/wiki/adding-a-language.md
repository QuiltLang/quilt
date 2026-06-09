# Adding a Language

This guide walks through adding a new language to Quilt. It uses the pattern established by the existing concrete languages (`rust`, `python`, `html`, `wgsl`, `text`).

## 1. Decide the role

A language can be:

- **Host language** — the ground language in a `.quilt` file. Requires both a `Language` *and* a `MetaLanguage` implementation. Example: Rust, Python.
- **Target language** — only appears inside `lang↖…↗` quotes. Requires only a `Language`. Example: HTML, WGSL.

## 2. Grammar

If the language needs tree-sitter parsing (recommended):

1. Fork or adapt an existing tree-sitter grammar for the language.
2. Add a **hole node** to the grammar. Rust uses `{}` and Python uses `__HOLE__` as hole tokens; your grammar needs a token that is syntactically valid in expression/statement position and uniquely recognizable.
3. Place the grammar in `rust/tree-sitter-<lang>/` following the same structure as the existing ones (Cargo bindings in `bindings/rust/`).
4. Add it to the workspace in `rust/Cargo.toml` if it needs to be a workspace member.

If the language doesn't need tree-sitter, implement `Language` directly (see the `bootstrap/lang.rs` approach).

## 3. Create the language module

Create `rust/quilt/src/langs/<lang>/`:

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
        // Return Variadic for nodes that accept arbitrarily many children
        // (e.g. block-like constructs). Default: Unknown.
        Arity::Unknown
    }

    fn hashbang(&self) -> Option<&'static str> {
        // Only needed for host languages that support `quilt run`.
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

In `rust/quilt/Cargo.toml`:

```toml
[features]
your_lang = ["dep:tree-sitter-your-lang"]

[dependencies]
tree-sitter-your-lang = { path = "../tree-sitter-your-lang", optional = true }
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

1. Add a `LanguageAdapter` impl in `rust/quilt-lsp/src/adapters.rs` defining:
   - `comment_syntax()` — how to write placeholder comments.
   - `splice_block()` — the placeholder for a quote in the projected document.
   - `wrap_fragment(body)` — how to wrap a quoted fragment so the downstream server tokenizes it.
2. Add a `MetaLanguageAdapter` impl for the projection logic.
3. Register the new adapter in `language_adapter()` and `meta_adapter()`.

## 9. Write tests

Add tests alongside your implementation:

```sh
cargo test -p quilt your_lang
```

At minimum test round-tripping: parse a fragment, serialize it back, and check it matches the input.
