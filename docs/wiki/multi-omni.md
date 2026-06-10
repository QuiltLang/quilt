# Multi and Omni

**File:** `quilt/src/multi.rs`, `quilt/src/langs/omni.rs`

## `Multi<LS, MS>`

`Multi` is the engine that combines a `Languages` registry with a `MetaLanguages` registry. All parsing and expansion goes through `Multi`.

```rust
pub struct Multi<LS: Languages, MS: MetaLanguages> {
    pub langs: LS,
    pub metas: MS,
}
```

### `Languages` and `MetaLanguages`

```rust
pub trait Languages {
    type Language: Language + ?Sized;
    fn get(&self, lang: &str)     -> Result<&Self::Language>;
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::Language>;
}

pub trait MetaLanguages {
    type MetaLanguage: MetaLanguage + ?Sized;
    fn get(&self, lang: &str)     -> Result<&Self::MetaLanguage>;
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::MetaLanguage>;
}
```

The key entry points on `Multi`:

| Method                     | Description                                                     |
|----------------------------|-----------------------------------------------------------------|
| `parse(s)`                 | Parse `s` in the default language (`"rs"`)                      |
| `parse_lang(lang, s)`      | Parse `s` as a single-language source                           |
| `parse_chain(chain, s)`    | Parse with a language chain (ground + defaults for bare quotes) |
| `expand(qterm)`            | Expand in the default language                                  |
| `expand_lang(lang, qterm)` | Expand using `lang`'s `MetaLanguage`                            |

### Built-in registry types

Three concrete `Languages`/`MetaLanguages` implementations are provided:

| Type                | Description                                                          |
|---------------------|----------------------------------------------------------------------|
| `DictLanguages`     | `BTreeMap<Box<str>, Box<dyn Language<…>>>` — dynamic, heap-allocated |
| `DictMetaLanguages` | `BTreeMap<Box<str>, Box<dyn MetaLanguage>>`                          |
| `Singleton<T>`      | Single language; `get(any_key)` always returns it                    |

`DictMulti = Multi<DictLanguages, DictMetaLanguages>` is the fully-dynamic version; useful when building a custom registry at runtime.

---

## `Omni`

`Omni = Multi<OmniLanguages, OmniMetaLanguages>` is the *production* multi used by the CLI. It avoids `Box<dyn …>` allocations by using enum dispatch over all enabled languages.

```rust
pub type Omni = Multi<OmniLanguages, OmniMetaLanguages>;
```

`OmniLanguages` holds one field per language, e.g.:

```rust
pub struct OmniLanguages {
    html: OmniLanguage,   // OmniLanguage::Html(HtmlLanguage)
    py:   OmniLanguage,   // OmniLanguage::Python(PythonLanguage)
    rs:   OmniLanguage,   // OmniLanguage::Rust(RustLanguage)
    txt:  OmniLanguage,
    wgsl: OmniLanguage,
}
```

`OmniMetaLanguages` holds only the languages that have meta-language support:

```rust
pub struct OmniMetaLanguages {
    py: OmniMetaLanguage,  // OmniMetaLanguage::Python(PythonMetaLanguage)
    rs: OmniMetaLanguage,  // OmniMetaLanguage::Rust(RustMetaLanguage)
}
```

Because `Omni` is the default, `Omni::default()` is the canonical way to create the engine in application code.

### `DynOmni`

A `DictMulti`-based version with `Box<dyn …>` is also available via `dict_omni_language()` — useful when the language set needs to change at runtime or when compiling code that can't use enum dispatch.

---

## Language lookup

Language names are looked up by short string key. Both short and long forms are accepted:

| Key                  | Language |
|----------------------|----------|
| `"rs"` or `"rust"`   | Rust     |
| `"py"` or `"python"` | Python   |
| `"txt"` or `"text"`  | Text     |
| `"wgsl"`             | WGSL     |
| `"html"`             | HTML     |

Only `"rs"` and `"py"` have meta-language entries; the others are parsing targets only.

---

## The `Stage` and `Expander`

The inner `Expander` struct (private to `multi.rs`) holds a reference to the `Languages` registry and the current ground `MetaLanguage`, and implements the recursive `expand` method.

`Stage` tracks quasi-quote depth:

```rust
pub enum Stage {
    Ground,
    Sky(Box<str>, Index),  // Sky(lang, depth)
}
```

The invariant: `Sky(lang, d)` with `d > 0`. An `Unquote { index }` with `index == d` escapes to Ground; one with `index < d` stays in Sky but at reduced depth.

---

## The `Singleton` adapter

```rust
pub type Single<L, M> = Multi<Singleton<L>, Singleton<M>>;
```

`Singleton<T>` implements `Languages` (or `MetaLanguages`) by ignoring the language key and always returning the single wrapped value. Useful in tests and the bootstrap pipeline where only one language is in play.
