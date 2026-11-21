# Multi and Omni

**File:** `quilt/src/multi.rs`, `quilt/src/langs/omni.rs`

## `Multi<LS, MS>`

`Multi` is the engine that combines a `Languages` registry with a `MetaLanguages` registry. All parsing and expansion goes through `Multi`.

```rust
pub struct Multi<LS: Languages, MS: MetaLanguages> {
    pub langs: LS,
    pub metas: MS,
    pub machines: crate::machine::Park,   // live default machines, one per language
}
```

### `Languages` and `MetaLanguages`

```rust
pub trait Languages {
    type Language: Language + ?Sized;
    fn get(&self, lang: &str)     -> Result<&Self::Language>;
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::Language>;
    fn canonical<'a>(&'a self, lang: &'a str) -> &'a str { lang }   // resolve an alias
}

pub trait MetaLanguages {
    type MetaLanguage: MetaLanguage + ?Sized;
    fn get(&self, lang: &str)     -> Result<&Self::MetaLanguage>;
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::MetaLanguage>;
    // …plus provided forwarders for every operator spelling and `prelude`,
    // e.g. `lift_str(lang, target)` = `self.get(lang)?.lift_str(target)`.
}
```

`canonical` maps an alias onto its canonical name (`py` → `python` in `Omni`); the machine park keys by it, so every alias of a language shares one machine.

The key entry points on `Multi`:

| Method                     | Description                                                     |
|----------------------------|-----------------------------------------------------------------|
| `parse(s)`                 | Parse `s` in the default language (`"rs"`)                      |
| `parse_lang(lang, s)`      | Parse `s` as a single-language source                           |
| `parse_chain(chain, s)`    | Parse with a language chain (ground + defaults for bare quotes) |
| `expand(qterm)`            | Expand in the default language                                  |
| `expand_lang(lang, qterm)` | Expand using `lang`'s `MetaLanguage`                            |
| `prelude_for(chain, src)`  | The runtime import the expanded file should open with, if any (see [Language Traits](language-traits.md#prelude--the-runtime-import)) |
| `spawn_machine(lang)`      | A fresh [machine](language-traits.md#machines--running-a-language) for `lang`, owned by the caller |
| `machine(lang)`            | `lang`'s default machine: parked in `machines`, spawned on first use, shared by every caller |
| `feed_on(lang, kind, src)` | Parse `src` as `kind` with `lang`'s `Language`, then feed it to the default machine |
| `eval_on(lang, term)`      | Evaluate on the default machine and parse the answered literal back into a term |

`Multi` also forwards each operator spelling (`lift_str(lang, target)`, `emit_str(lang)`, …) to the named meta-language, which is how `build_nodes` spells a glyph while parsing.

### Built-in registry types

Three concrete `Languages`/`MetaLanguages` implementations are provided:

| Type                | Description                                                          |
|---------------------|----------------------------------------------------------------------|
| `DictLanguages`     | `BTreeMap<Box<str>, Box<dyn Language<…>>>` — dynamic, heap-allocated, plus an alias map |
| `DictMetaLanguages` | `BTreeMap<Box<str>, Box<dyn MetaLanguage>>`, plus an alias map       |
| `Singleton<T>`      | Single language; `get(any_key)` always returns it                    |

`DictMulti = Multi<DictLanguages, DictMetaLanguages>` is the fully-dynamic version; useful when building a custom registry at runtime. Register with `add_lang` / `add_meta`, and with `add_alias(alias, canonical)` to make another name resolve to the same instance in both registries.

---

## `Omni`

`Omni = Multi<OmniLanguages, OmniMetaLanguages>` is the *production* multi used by the CLI. It avoids `Box<dyn …>` allocations by using enum dispatch over all enabled languages.

```rust
pub type Omni = Multi<OmniLanguages, OmniMetaLanguages>;
```

None of it is written by hand. The `define_omni!` macro at the bottom of `langs/omni.rs` generates the whole surface from one table — a `languages` section with one line per language, and a `metas` section listing the languages that can act as hosts:

```rust
define_omni! {
    languages {
        bash if "bash"   => Bash(BashLanguage, DynBashLanguage):       ["bash"];
        // …
        py   if "python" => Python(PythonLanguage, DynPythonLanguage): ["python", "py"];
        rs   if "rust"   => Rust(RustLanguage, DynRustLanguage):       ["rust", "rs"];
        // …
    }
    metas {
        bash if "bash"   => Bash(BashMetaLanguage):                    ["bash"];
        // …
    }
}
```

Each line reads `field if "feature" => Variant(LanguageTy, DynLanguageTy): ["canonical", "alias", …]`. From it the macro generates:

- `OmniLanguages` / `OmniMetaLanguages` — the registries, one field per enabled language (`bash`, `html`, `lean`, `nix`, `py`, `rs`, `sql`, `txt`, `ts`, `wgsl`, `zsh`) and per enabled host (`bash`, `html`, `lean`, `nix`, `py`, `rs`, `ts`, `zsh`);
- `OmniLanguage` / `OmniLanguagePost` / `OmniMetaLanguage` — the enum-dispatch wrappers, delegating every trait method to the variant;
- `DynOmniLanguages` / `DynOmniMetaLanguages` — the same registries over `dyn Language` / `dyn MetaLanguage`;
- `dict_omni_language()` — below;
- `langs::line_comment` / `langs::header_comment` — each language's [`Comments`](language-traits.md#comments) consts, looked up by canonical name or alias.

Each field is gated on its Cargo feature, so `Omni` holds only the languages that are enabled. Because `Omni` is the default, `Omni::default()` is the canonical way to create the engine in application code.

### `dict_omni_language()`

A `DictMulti` over all enabled languages, with `Box<dyn …>` entries: each language registered once under its canonical name, its aliases added with `add_alias`. Useful when the language set needs to change at runtime or when compiling code that can't use enum dispatch.

---

## Language lookup

Language names are looked up by short string key. Both short and long forms are accepted:

| Key                               | Language   | Meta-language                 |
|-----------------------------------|------------|-------------------------------|
| `"rs"` or `"rust"`                | Rust       | runtime-backed                |
| `"py"` or `"python"`              | Python     | runtime-backed                |
| `"ts"` or `"typescript"`          | TypeScript | runtime-backed                |
| `"nix"`                           | Nix        | string-based                  |
| `"lean"` or `"lean4"`             | Lean 4     | string-based                  |
| `"bash"`                          | Bash       | string-based                  |
| `"zsh"`                           | Zsh        | string-based                  |
| `"html"`                          | HTML       | identity (holds its quotes)   |
| `"txt"` or `"text"`               | Text       | — (an identity meta exists, but is not registered) |
| `"wgsl"`                          | WGSL       | —                             |
| `"sql"`, `"mysql"` or `"mariadb"` | SQL        | —                             |

Languages without a meta-language entry are parsing targets only: they can be quoted, and the host's meta drives their expansion. `mysql`/`mariadb` are the same grammar as `sql` under another name, which changes only how `↑` escapes a string (see [Concrete Languages](concrete-languages.md#dialects-sql-vs-mysql)).

---

## The `Stage` and `Expander`

The `Expander` struct in `multi.rs` (its fields and its recursive `expand` method are private) holds a reference to the `Languages` registry, the ground `MetaLanguage` and the ground language's name, plus two pieces of state: the metavariables collected while expanding a pattern-let's pattern, and whether the ground code being expanded runs inside a `b_` accumulator (only there can a statement-shaped quote be emitted).

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

`Singleton<T>` implements `Languages` (or `MetaLanguages`) by ignoring the language key and always returning the single wrapped value. Useful in tests and the bootstrap pipeline where only one language is in play: `langs::bootstrap::Bootstrap` is `Single<BootstrapLanguage, BootstrapMetaLanguage>`, the multi `-m bootstrap` selects.
