# Language Traits

**Files:** `quilt/src/lang.rs`, `quilt/src/meta.rs` (plus `quilt/src/treesitter.rs` and `quilt/src/machine.rs`, below)

Two trait families are the extension points for adding language support to Quilt. They are the syntactic half of a square whose semantic half is *machines* (`docs/design/machines.md`): a `Language` says what a fragment *is*, a `Machine` what it *does*. A language declares its machine through `Language` hooks, and a host's spellings for driving one (`m.↓(t)`, `m.⟨T⟩(t)`, `lang⟨M⟩`) live on `MetaLanguage`.

## `Language` / `LanguagePost` — parsing

```rust
pub trait Language {
    type Post: LanguagePost;

    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post>;
    // Convenience wrappers:
    fn parse_with(&mut self, code: &[FlatNode], plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>>;
    fn parse(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>>;
    fn parse_as(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Arc<QTerm>>;
    fn parse_expr/stmt/file/auto(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>>;

    // Facts about node kinds (all defaulted):
    fn arity(&self, tag: &str) -> Arity;
    fn separator(&self, tag: &str) -> Option<&'static str>;
    fn typ(&self, tag: &str) -> InnerKind;
    fn classify_term(&self, term: &QTerm) -> InnerKind;   // typ on the root tag by default
    fn ident_tag(&self) -> &'static str;                  // "identifier"; the shells say "word"

    // Running it (all defaulted to None):
    fn hashbang(&self) -> Option<&'static str>;
    fn machine_spec(&self) -> Option<MachineSpec>;
    fn repl_spec(&self) -> Option<ReplSpec>;
    fn native_machine(&self) -> Option<Box<dyn Machine>>;
}

pub trait LanguagePost: Debug {
    fn holes(&self) -> &[Hole];
    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>>;
}

// Comment syntax rides alongside, as consts rather than `Language` methods —
// see `Comments` below.
pub trait Comments {
    const LINE: Option<&'static str> = Some("//");
    const HEADER: Option<&'static str> = Self::LINE;
}
```

### `FlatNode`

```rust
pub enum FlatNode<'a> {
    Hole,          // a placeholder for a child term
    Splice,        // a hole a one-line ↙…↘ splices its value into
    Str(&'a str),  // ordinary source text
    NewLine,
}
```

The multi-language engine collects the `Node` list for a given fragment and builds a `Vec<FlatNode>` where each `Node::Quote`/`Node::Unquote` becomes a hole. `Splice` is a `Hole` in every respect but one: a column-sensitive language may align the spliced value to the hole's column (see `TSProvider::align_splices` below — Lean turns it on). `FlatNode::is_hole` matches either. This flat list is passed to `parse_pre`.

### `Hole`

```rust
pub struct Hole {
    pub otag: Box<str>,              // the tree-sitter tag of the hole in the outer language
    pub ikind: Option<InnerKind>,    // the kind of term this position demands
    pub prefix: Box<[Box<str>]>,     // accumulated indentation prefixes
}
```

`parse_pre` must return one `Hole` per hole in its input (in order). The `Multi` engine uses `hole.otag` to label the `Quote`/`Unquote` node in the output tree, `hole.ikind` to parse an unquote body with the kind its position demands (a `{ }` in statement position, say) instead of guessing, and `hole.prefix` to strip indentation from nested content.

### `InnerKind`

```rust
pub enum InnerKind { Expr, Stmt, Item, Block, File }   // File is the default
```

The grammatical role a fragment plays: an expression, a statement, an item / top-level declaration, a brace-delimited block *body* (as opposed to a block used as a value, which is an `Expr`), or a whole file. `is_stmt_like()` groups `Stmt` and `Item`; `is_code_like()` adds `File`, for a ground unquote that holds several statements.

Passed as a hint to `parse_pre`. Some parsers use it to try specific grammar entry points instead of guessing. `None` means auto-detect. Languages map their node tags onto kinds with `typ(tag)` and, when the root tag alone is ambiguous, `classify_term(term)` (WGSL's `source_file` holding a statement plus a trailing `;`). A hole's kind comes from `TSProvider::hole_kind`, which sees the node in its parse tree rather than only its tag (below). `InnerKind` is also the message sort a [machine](#machines--running-a-language) is fed in.

### `Arity`

```rust
pub enum Arity { Unknown, Const(u8), Variadic }
```

Returned by `Language::arity(tag)`. `Variadic` tags (e.g. Rust's `"block"` and `"source_file"`) tell the expander to use `expand_tuple` in variadic mode, generating an imperative builder block rather than a single `tb(..).c(..)..b()` call — so that one child can contribute zero-or-many terms.

The tables are **generated, not hand-written** (issue #202): `bin/gen-arity` reads the `REPEAT` rules out of each vendored `quilt/grammars/<lang>/grammar.json` and writes `quilt/src/langs/arity.rs`, and every provider's `arity` is one line:

```rust
fn arity(&self, tag: &str) -> Arity {
    Arity::from_table(crate::langs::arity::RUST, tag)
}
```

`bin/check-arity` regenerates and fails on drift, so a grammar bump that adds, drops or renames a repeat container shows up as a reviewable diff rather than as emit quietly changing where it may splice. The derivation itself — which repeats count as a node's *own* children, and why hidden rules are followed but category rules, aliases and tokens are not — is documented in `quilt-conformance/src/arity.rs`.

Declaring a tag variadic costs nothing in generated output: a variadic node with no unquote among its direct children is built fluently anyway, since only an unquote can contribute a variable number of terms.

### `separator`

The token between the repeated children of a variadic node — `","` for an array, an argument list, a Python list — or `None` when they are not separated (a block, a statement list). The expander asks it for a splice into such a node (`[↙xs↘]`, `f(↙xs↘)`) so the elements the splice contributes come out separated rather than back to back ([#293](https://github.com/QuiltLang/quilt/issues/293)). It is derived like `arity`: `bin/gen-arity` also writes a `<LANG>_COMMA_SEPARATED` table, read with

```rust
fn separator(&self, tag: &str) -> Option<&'static str> {
    crate::lang::comma_separator(crate::langs::arity::RUST_COMMA_SEPARATED, tag)
}
```

### `hashbang`

Returns the shebang line used to run the expanded file as a script, e.g.:
- Rust: `"#!/usr/bin/env rust-script"`
- Python: `"#!/usr/bin/env python3"`
- TypeScript: `"#!/usr/bin/env -S node --experimental-strip-types"`

`quilt run` uses this to determine which runner to invoke. Split it with `lang::parse_hashbang`, which unwraps `env` (and `env -S`) and returns the program plus the arguments that precede the script path — the interpreter is not always the last word.

### Machines — running a language

A `Machine` (`quilt/src/machine.rs`; design in `docs/design/machines.md`) is a stateful evaluator of one language: fed terms over time, with `InnerKind` as the message sort (an `Item` defines, a `Stmt` runs, an `Expr` is a query answered with the value's literal spelling), it remembers what it was fed. Three `Language` hooks declare how to get one, and `machine::spawn_machine` prefers them in this order:

| Hook | Provider | Example |
|---|---|---|
| `native_machine()` | in-process, no interpreter | HTML's `HtmlMachine` — a document whose definitions are its ids |
| `repl_spec()` | `ReplMachine`: one long-lived process fed sentinel-framed chunks on stdin, so state is real process state | `sqlite3` for SQL, the shells, the stdin kernels for Python and TypeScript |
| `machine_spec()` | `ScriptMachine`: the history replayed through a fresh script per feed | Lean (`#eval` / `#check`) |

The specs themselves are data and live in `machine.rs` (`native_machine`, `repl_spec`, `script_spec`, listed in `REGISTERED_SPECS`) rather than behind the `parse` feature, so the `Language` impls delegate into that table — `crate::machine::repl_spec("python")` — and `qspawn`, what `lang⟨M⟩` expands to in a runtime-only Rust program, reads the same one. `Multi::machine(lang)` parks one default machine per language; that is what `quilt repl` and notebook cells share.

### `Comments`

How the language spells comments, implemented beside its `Language` impl:

```rust
impl Comments for RustLanguage {
    const LINE: Option<&'static str> = Some("//");
    const HEADER: Option<&'static str> = Some("//!");   // inner doc comment
}
```

`LINE` is the plain line-comment introducer; `HEADER` is the one the CLI puts on
a generated file's `DO NOT EDIT` banner, and defaults to `LINE` — Rust is the
only language where the two differ. `None` means no *prefix* can express one
(HTML's comments are delimited; plain text has none), and the caller decides
what to do about it.

Look them up by language name with `langs::line_comment` / `langs::header_comment`,
which `define_omni!` generates from the registry table, so aliases agree by
construction. quilt-lsp reads the same functions for its adapters' comment
syntax rather than declaring the spellings a second time
([#194](https://github.com/QuiltLang/quilt/issues/194)).

These are associated consts rather than `Language` methods for two reasons: an
associated const would make `Language` non-dyn-compatible, and `DynOmniLanguages`
dispatches through `dyn Language`; and a const lookup is a `match` returning a
`&'static str`, where a `&self` method would mean constructing a language — a
tree-sitter `Parser` — to read one.

---

## `MetaLanguage` — expansion

```rust
pub trait MetaLanguage {
    fn expand_quote(
        &self, lang1: &str, tag: &str, i: Index,
        lang2: &str, qterm: &Arc<QTerm>, cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;

    fn expand_unquote(
        &self, lang1: &str, tag: &str, i: Index,
        lang2: &str, qterm: &Arc<QTerm>, cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;

    fn expand_tuple(
        &self, lang1: &str, tag: &str,
        qterms: &[Arc<QTerm>], cmds: &[CmdOrHole], arity: Arity,
    ) -> Result<Arc<QTerm>>;

    // Everything below has a default.
    fn wrap_child(&self, qterm: Arc<QTerm>, okind: OuterKind) -> Result<Arc<QTerm>>;
    fn wrap_separated(&self, qterm: Arc<QTerm>, sep: &str) -> Result<Arc<QTerm>>;

    // Pattern-lets (`let ↖pattern↗ = value;`) — `None`/`Err` unless the host has them:
    fn pattern_tag(&self) -> Option<&'static str>;
    fn pattern_binding(&self, terms: &[Arc<QTerm>]) -> Option<(usize, usize)>;
    fn pattern_var(&self, name: &str) -> Result<Arc<QTerm>>;
    fn pattern_let(&self, names: &[Box<str>], pattern: &Arc<QTerm>, value: &Arc<QTerm>)
        -> Result<(Arc<QTerm>, Arc<QTerm>)>;

    // Operator spellings — the string that ↑ / ↓ / ← / ⟨T⟩ / ⟨N⟩ / ⟨M⟩ expand to:
    fn lift_str(&self, target: &str)   -> Result<&'static str>;   // default LIFT
    fn reduce_str(&self, target: &str) -> Result<&'static str>;   // default REDUCE
    fn emit_str(&self)                 -> Result<&'static str>;   // default EMIT
    fn type_str(&self)                 -> Result<&'static str>;   // default TYPE
    fn name_str(&self)                 -> Result<&'static str>;   // default NAME
    fn reduce_method_str(&self, target: &str) -> Result<&'static str>;  // `m.↓(t)`; default Err
    fn type_method_str(&self)                 -> Result<&'static str>;  // `m.⟨T⟩(t)`; default Err
    fn spawn_str(&self, lang: &str)           -> Result<String>;        // `lang⟨M⟩`; default Err

    // Quote representations other than ⟨cst⟩ (`ts⟨str⟩↖…↗`, `ts⟨qt⟩↖…↗`):
    fn lift_str_in(&self, target: &str, repr: Repr) -> Result<&'static str>;
    fn emit_str_in(&self, repr: Repr) -> Result<&'static str>;
    fn expand_flat(&self, repr: Repr, lang2: &str, pieces: &[Piece]) -> Result<Arc<QTerm>>;

    // What must be in scope for the code above: the runtime import an expanded
    // file opens with.
    fn prelude(&self, targets: &[&str]) -> Option<Prelude> { None }
}
```

### How expansion works

When the expander encounters a `Quote { lang2, … }` at Sky depth, it calls:

```
meta.expand_quote(lang1, tag, index, lang2, expanded_term, cmds)
```

This should return a `QTerm` whose code, when executed, constructs the quoted term at runtime. For the **Rust** meta-language this means returning code like:

```
quote("expression_statement", 1, "rs", <term>, &[...cmds...])
```

Similarly for `expand_unquote` and `expand_tuple`.

### `OuterKind`

```rust
pub enum OuterKind { None, Emit, Splice }
```

Passed to `wrap_child`:
- `None` — no wrapping needed.
- `Emit` — the child is a quote inside a variadic context; wrap as `.emit(&mut b_);` (Rust) or `.e(child)` (Python).
- `Splice` — the child is a statement-valued unquote that should be inlined; wrap as a bare statement (Rust: add `;`).

A splice into a comma-separated container (see [`separator`](#separator)) is wrapped once more, by `wrap_separated(child, sep)`: the builder's `sep(..)`/`end_sep()` region, so the elements it contributes are separated. The default leaves the child alone.

### Operator spelling constants

The special glyphs in `.quilt` source are translated to language-specific strings. `lift_str` is *target-directed* — `target` is the language of the enclosing quote, so a lift inside `wgsl↖ … ↗` spells a lift into WGSL — and `reduce_str`'s `target` is the annotation on the glyph (`""` when there is none):

| Glyph | Default constant | Rust                                                        |
|-------|------------------|-------------------------------------------------------------|
| `↑`   | `"__LIFT__"`     | `"qlift()"` into Rust, `"qlift_to::<L>()"` into any other target |
| `↓`   | `"__REDUCE__"`   | `"reduce()"`; `"reduce_py()"` for `py↓`                     |
| `←`   | `"__EMIT__"`     | `"emit(&mut b_)"`                                           |
| `⟨T⟩` | `"__TYPE__"`     | `"Arc<QTerm>"`                                              |
| `⟨N⟩` | `"__NAME__"`     | `"name"`                                                    |
| `m.↓(t)` | (error)       | `"eval"` — `reduce_method_str`, machine-directed reduce     |
| `m.⟨T⟩(t)` | (error)     | `"type_of"` — `type_method_str`, the machine's typing judgment |
| `lang⟨M⟩` | (error)      | `qspawn("lang")` — `spawn_str`, a machine speaking `lang`   |

The method-position spellings apply only when the glyph sits flush against an argument list; a host that returns an error there falls back to the ordinary `↓` / `⟨T⟩` spelling. `spawn_str` refuses, at expansion time, a language no machine is registered for (`meta::spawn_spelling`). The Bootstrap meta-language has its own spellings (`bs_lift()`, `bs_reduce()`, `bs_name`) for bootstrapping `meta.rs`.

The spelling accessors return `Result`, because not every meta-language has a
spelling for every glyph: a **string-based** meta (nix, lean) has no `QTerm`
runtime, so `↓` has none, and untyped Nix has none for `⟨T⟩`. Returning an error
there is what keeps the default `__REDUCE__`/`__TYPE__` placeholders from
leaking into generated code. A meta that *does* have a meaning for a glyph
should spell it rather than fail — in the string model a fragment is a string,
so lean's `⟨T⟩` is `String` and `⟨N⟩` is the host's identity (`id`, and
`toString` for nix). The same applies to `←`: having no `b_` accumulator is not
the same as having no emit, and nix spells it
`builtins.concatStringsSep "\n"` — the functional reading, joining a list of
fragments into the surrounding container (issue #155). Lean's still fails
(issue #133).

Having a `QTerm` runtime is not sufficient either. Python and TypeScript build a
variadic container as a fluent `tb(..).e(child).b()` chain rather than Rust's
imperative block, so there is no name for a ground `←` to append to (issue
#152). Python therefore fails on `←`; TypeScript took the functional reading
instead, as Nix did — `←(items)` spells the runtime's `qseq`, one term splicing
the whole sequence. The rule is the same in every case: spell the glyph where
the host has a meaning for it, fail with actionable guidance where it does not,
and never let the placeholder reach the output.

### Quote representations — `lift_str_in`, `emit_str_in`, `expand_flat`

A quote may name a representation other than the default `⟨cst⟩`
(`ts⟨str⟩↖…↗`, `ts⟨qt⟩↖…↗`; see
[Concepts](concepts.md#representations-cst-str-qt)). The expander does not
rebuild such a quote's tree through `expand_quote`: it flattens it into
`Piece`s — text, newlines, prefix pushes and pops, and a `Hole` holding each
ground unquote's expanded host expression — and hands them to
`expand_flat(repr, lang2, pieces)`. Inside such a quote, `↑` and `←` are spelled
by `lift_str_in` / `emit_str_in`, since the value must be of that
representation. The defaults defer to `lift_str` / `emit_str` for `⟨cst⟩` and
refuse the others (`meta::no_repr`); TypeScript is the host that implements all
three.

### `prelude` — the runtime import

The operator spellings say *how* this host writes a call into the runtime;
`prelude` says what has to be in scope for those calls to resolve — the import
`quilt expand` and `quilt run` open a generated file with (issue #274):

```rust
pub struct Prelude {
    pub text: Cow<'static, str>,        // "use quilt::prelude::*;"
    pub markers: Vec<Cow<'static, str>>, // "already imported by hand" if the source contains one
}
```

It sits on `MetaLanguage` rather than on `Language` because the import exists to
satisfy the *expander's output*, and the output vocabulary is the meta's
business: `ops.rs` is what decides a quote becomes `tb("block").c(…)`. A
quotable-only target (wgsl, sql) has no meta and needs no prelude, and a
string-based meta (nix, lean, bash, zsh) or an identity meta (text, html) calls
no runtime at all — so both get the right answer from the `None` default.

`targets` are the object languages the file may lift into: its language chain
plus every annotation its quotes name. Rust and Python ignore them and return
one glob import; TypeScript, whose `lift_str` is target-directed, builds its
named import list from them — which is the half of this that a human writing the
import by hand gets wrong, and finds out about at run time.

The `markers` are matched against the *source*, not the expansion, because a
ground-stage import is copied through verbatim and the source is in hand before
anything is written. The injection itself belongs to what writes the file, not
to expansion: `Multi::prelude_for(chain, input)` decides whether a file gets
one, and `meta::with_prelude` places it after the leading header (comments, a
shebang, Rust's inner attributes). See
[CLI](cli.md#the-runtime-import-issue-274) for the rules the injection follows
and the `--prelude` / `--no-prelude` escape hatch.

---

## `TSLanguage` — the tree-sitter helper

**File:** `quilt/src/treesitter.rs`

Most concrete language implementations use `TSLanguage<P: TSProvider>` rather than implementing `Language` directly. A `TSProvider` supplies:

```rust
pub trait TSProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser;
    fn hole_str(&self) -> &'static str;   // placeholder: "{}", "__HOLE__" or "__QUILT_HOLE__"

    // Everything below has a default.
    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)>;
    fn arity(&self, tag: &str) -> Arity;                 // Arity::Unknown
    fn separator(&self, tag: &str) -> Option<&'static str>;
    fn align_splices(&self) -> bool;                     // false; Lean says true
    fn typ(&self, tag: &str) -> InnerKind;               // InnerKind::File
    fn hole_kind(&self, node: tree_sitter::Node) -> InnerKind;   // typ(node.kind())
    fn classify_term(&self, term: &QTerm) -> InnerKind;
    fn ident_tag(&self) -> &'static str;
    fn hashbang(&self) -> Option<&'static str>;
    fn machine_spec(&self) -> Option<MachineSpec>;
    fn native_machine(&self) -> Option<Box<dyn Machine>>;
    fn repl_spec(&self) -> Option<ReplSpec>;
    fn fragment_wrappers(&self) -> &'static [FragmentWrapper];   // &[]
}
```

Most of these are the `Language` methods of the same name, which `TSLanguage` forwards.

`TSLanguage::parse_pre` substitutes each hole with `hole_str()`, feeds the resulting string to the tree-sitter parser, and records where it wrote each placeholder. A node whose byte range *equals* a placeholder's is the hole. Where the token around a placeholder swallows it — inside a string, inside a comment, or glued to neighbouring text — no node matches, so the token is split around the placeholder instead and the hole becomes its child (issue #221); that is why a placeholder need only lex as an ordinary identifier. `parse_post` then replaces the holes with real `Arc<QTerm>` children.

`hole_kind` gives each hole its `InnerKind`. Unlike `typ`, it is handed the node in its parse tree, so it can read context the tag alone cannot express: a Rust `block` is an expression by tag (`let x = { … }`), but in body position (`fn f() { … }`) it is a `Block`, and Lean classifies its hole — spelled the same everywhere — by its parent.

`unwrap` is called on the top-level tree-sitter node to strip the outer `source_file` wrapper and infer whether the fragment is an expression or statement. It returns `Result`, so an unsupported parse shape is an error rather than a panic.

When a fragment does not parse as a whole file — a match arm, an enum variant, a Lean constructor — `parse_pre` retries it inside each of the provider's `fragment_wrappers` in turn (`match q { … }`, `enum Q { … }`, …), takes the one fragment node back out of the wrapper's `container`, and keeps it only if it writes back as the quoted text byte for byte. `TSLanguage::parse_plain` is `parse_pre` without that retry, for a language with retries of its own to order (Lean's `#check`).

`DynTSLanguage<P>` is a newtype that wraps `TSLanguage<P>` and boxes its `Post` type so it can be stored as `Box<dyn Language<Post = Box<dyn LanguagePost>>>` in a `DictMulti` (and as `dyn Language` in `DynOmniLanguages`).

Nothing in `Language` requires tree-sitter. `langs/text/lang.rs` implements it directly: text has no grammar, so "parsing" records the literal runs and the holes between them as one `text` tuple.
