# Machines and meta-machines

Status: **proposal** (phase 1 implemented — see [Phases](#phases)).

Quilt's two trait families are both about programs *as data*. `Language` /
`LanguagePost` (`quilt/src/lang.rs`) reads text into a `QTerm`; `MetaLanguage`
(`quilt/src/meta.rs`) turns terms into host code that rebuilds them, plus the
operator spellings. Everything is stateless by design: every `MetaLanguage`
method takes `&self`, the only `&mut` in the engine is tree-sitter parser
scratch, and the only context threaded through parsing is the language-chain
zipper.

Execution, by contrast, is not an abstraction — it is a handful of hard-coded
escape hatches:

- Rust `↓` forks a fresh `rust-script` process per reduce and shuttles the
  value back as postcard bytes (`langs/rust/ops.rs`). It cannot re-expand a
  staged term.
- `py↓` writes a one-shot python3 script; Python's own `↓` does `exec` into a
  fresh `ns = {}` every time (`quilt-python/quilt/__init__.py`); Node's runs a
  fresh `vm` context and shells back out to `$QUILT` to expand
  (`quilt-wasm/node/index.mjs`, issue #153).
- `quilt run` is expand-then-shebang-exec; every temp artifact is deleted
  before exit. `$QUILT`/`$QUILT_CHAIN` is the only channel into the running
  program, and it exists only so `↓` can find the expander again.

Nothing anywhere holds definitions between evaluations. Two `↓`s in one
meta-program each rebuild the world from nothing; a definition made by one
reduce is invisible to the next. And the conformance matrix already *names*
the missing noun without having it: SQL's `runnable` axis is unsupported
because "a query is executed by **a database server**, not by an interpreter
reading a shebang"; Nix's because a file "is evaluated with `nix eval`, not
executed"; bash/zsh/nix/lean cannot spell `↓` because "the string-based meta
has no `QTerm` runtime to evaluate against". All four notes describe the same
absence: there is no first-class thing that executes a language and remembers
what it was told. That thing is a **machine**.

Two useful precedents already in the tree: `quilt-python`'s `run()` is the one
API that hands back an evaluated environment (`return ns`), and `quilt-lsp`'s
`ChildServer` is the one long-lived, request-multiplexed child process — a
machine in all but name, for the *tooling* protocol instead of the
*evaluation* protocol.

## The square

The existing pair is the syntactic half of a 2×2. The proposal adds the
semantic half:

|                  | programs as data (syntax)                        | programs as behavior (semantics)                  |
|------------------|--------------------------------------------------|---------------------------------------------------|
| **object level** | `Language` — text → term                         | `Machine` — term → answer, *stateful*             |
| **meta level**   | `MetaLanguage` — how a host spells term-building | `MetaMachine` — how a host spells machine driving |

A `Language` says what a fragment *is*; a `Machine` says what it *does*, and
remembers. A `MetaLanguage` generates programs; a `MetaMachine` generates
programs *that drive machines* — including machines of other languages, and
including other machines as values.

## The `Machine` trait

```rust
/// A stateful evaluator of one language: fed programs over time, it
/// accumulates definitions and effects, and answers queries with the
/// literal spelling of the value.
pub trait Machine {
    fn lang(&self) -> &str;

    /// Feed one fragment. `kind` is the message sort:
    ///   Item — a definition: extends the machine's environment
    ///   Stmt — an effect: runs against the current environment
    ///   Expr — a query: returns the value's denotation
    ///   File — a batch of the above
    fn feed_str(&mut self, kind: InnerKind, src: &str) -> Result<Answer>;

    // Term-level conveniences layered on the textual primitive:
    fn feed(&mut self, kind: InnerKind, term: &QTerm) -> Result<Answer> { … }
    fn eval(&mut self, term: &QTerm) -> Result<Answer> { … } // = feed(Expr)
}

pub struct Answer {
    /// The value as a literal of the machine's own language — the inverse
    /// of lift — when the value has a literal spelling.
    pub value: Option<Box<str>>,
    /// Otherwise a residue: a fresh name the machine bound the value to.
    /// The value stays in the machine; the text to splice is a reference.
    pub name: Option<Box<str>>,
    /// Captured output, for display and diagnostics.
    pub stdout: Box<str>,
    pub stderr: Box<str>,
}
```

Three deliberate choices, each anchored in a tenet:

**The trait traffics in terms; text is the end user's door** (tenet 2, read
correctly — an earlier draft made `feed_str` the trait primitive, which
misread the tenet: it is about the *end user* writing plain source text
rather than builder calls, and says nothing about internal APIs). So
`Machine::feed`/`eval`/`type_of` take `&QTerm` like every other Quilt
interface, and the validated textual doors live at the rim, where the
`Language` that can parse lives: `Multi::feed_on(lang, kind, src)` parses
user text into a term before any machine sees it, and `Multi::eval_on`
re-reads the answered literal into a term. A provider whose *wire* is text —
a subprocess's stdin — coparses internally, which is its business and no
caller's; the raw door survives as inherent `feed_str`/`type_of_str` on the
concrete providers. `machine.rs` still lives beside `lift.rs` in the
tree-sitter-free runtime half of the crate, which is exactly why the parsing
side of the loop belongs to `Multi`.

**Answers are literals** (tenets 2+3). What does "the value of `21 + 21`" mean
language-agnostically? The one representation every language already has: its
own literal for it — `42`. This makes `Answer::value` the inverse of `↑`: lift
takes a host value to the term denoting it; eval takes a term to the literal
denoting its value. The law the runtime corpus already pins —
`reduce(lift(x)) == x` in `conformance/runtime/cases.json` — becomes the
defining equation of the pair, now stated per machine and testable for every
language with a machine, not just the three with `QTerm` runtimes. Parsing the
literal back into a term is the `Multi` rim's job (`Multi::eval_on`), because
that is where the `Language` lives. Values with no literal spelling (an open
file, a closure) do not break the model: the machine binds them to a fresh
name and answers with the reference — which is precisely "programs referencing
definitions the machine keeps track of".

**`InnerKind` is already the message vocabulary.** `lang.rs` calls the kinds
"sorts of messages for communicating between parsers". They are equally the
sorts of messages for communicating with machines: Item = define, Stmt = do,
Expr = ask, File = batch. No new enum; the parse-side classification
(`classify_term`) tells you how to feed a term.

### Laws

Conformance properties, in the spirit of `cross.rs` turning tenet 3 into a
check:

- **Denotation**: `m.eval(lift_to::<L>(v)).value` coparses equal to
  `lift_to::<L>(v)` — evaluating a literal answers that literal.
- **Sequencing**: `m.feed(p1); m.feed(p2)` behaves as feeding `p1; p2` to a
  fresh machine. This is the soundness criterion van Binsbergen et al. give
  for REPL interpreters over "sequential languages", and it is what makes
  definitions-then-references work.
- **Isolation**: two spawned machines share nothing (until explicitly
  connected).

### Capabilities as subtraits (implemented)

Mirroring how the capability matrix refuses to pretend all languages are
equal, `machine.rs` now carries:

```rust
pub trait SnapshotMachine: Machine {          // fork/rollback (Racket
    fn snapshot(&self) -> Result<Snapshot>;   // namespaces, Smalltalk images,
    fn restore(&mut self, s: &Snapshot) -> Result<()>; // a solver's push/pop)
}
pub trait MeteredMachine: Machine {           // nanobots' StateMachine shape
    fn add_gas(&mut self, amount: u32);       // and wasmtime fuel
    fn run(&mut self, max_gas: u32) -> Result<RunResult>; // Finished | Suspended
}
pub trait IntrospectMachine: Machine {        // what's defined — for the LSP
    fn defs(&self) -> Result<Vec<Def>>;       // to project machine state
}
```

`ScriptMachine` implements `SnapshotMachine` exactly and for free — its
whole state *is* its history, the replay model's one honest advantage over a
live process. The typing judgment sits on `Machine` itself as `type_of`
(default: an honest error), spelled per language by an optional `type_wrap`
in the spec — python answers `int`; `Metered` and `Introspect` await their
providers (nanobots-shaped runners; the LSP).

## Why `Machine` is not a better `Language`

A fair question: is the machine just what `Language` should have been? No —
they are duals, and merging them would cost both their strengths.

`Language` is *denotational and stateless*: text in, term out, the same
answer every time, no resources owned, safe to share behind one registry
entry, compiled for wasm. `Machine` is *operational and stateful*: it owns a
process or connection, its answers depend on everything fed before, and two
of them deliberately diverge. The cardinality alone settles it: there is one
Rust `Language` and there are as many Rust machines as you spawn — a
singleton registry versus a park of instances. A language also has *many*
machine shapes (script, REPL, kernel, DB connection), so machine-ness is a
1:N relation off `Language`, not an is-a.

The seam between them is the factory: `Language::machine_spec` /
`repl_spec`, the same pattern as `hashbang` — the language *knows how to
start* its machines, and nothing more. Everything stateful lives behind the
`Machine` trait. (In the algebra/coalgebra reading: `Language` is the
algebra of syntax, `Machine` the coalgebra of behavior; `coparse` and `feed`
are the two directions.)

## Who else can implement `Machine`

The trait is deliberately the *smallest* interface over "a stateful thing
that accepts programs", so a surprising amount of existing tooling already
has the right shape. Beyond the interpreters:

- **Language servers.** `quilt-lsp`'s `ChildServer` already *is* a
  request-multiplexed persistent child process; an `LspMachine` wraps one:
  feeding an Item is `didOpen`/`didChange` on the virtual document, and the
  machine-level queries are the tooling judgments — diagnostics, hover
  types, completions. This is not `eval` (an LSP must not run code), which
  is exactly why the *judging* generalization below matters: an LSP is a
  machine whose answers are judgments about programs rather than values of
  them. The payoff runs both directions — `IntrospectMachine::defs()` feeds
  the LSP's projections (issue #193's missing module context is "the
  definitions the ground program will have fed by fragment-run time"), and
  the LSP's own downstream servers become machines in the park.
- **Debug adapters (DAP).** A debugger session is a machine whose `feed` is
  setting breakpoints and stepping and whose queries (`evaluate`, watch
  expressions) answer in the debuggee's literal syntax. DAP is, like LSP and
  Jupyter, one adapter for many languages.
- **SMT solvers.** SMT-LIB is almost embarrassingly this protocol already:
  `declare-const`/`assert` are Item feeds, `check-sat`/`get-model` are
  queries, `push`/`pop` are `SnapshotMachine`. A `z3 -in` process is a
  `ReplMachine` with `print_wrap = "(eval {})"` and nothing else new.
- **Proof assistants.** Lean and Coq servers — see the judging section
  below.
- **Databases.** A connection is the SQL machine (temp tables and prepared
  statements are its definitions); `sqlite3` on stdin is a `ReplMachine`
  today, a real driver is a native provider tomorrow.
- **Notebook kernels.** One Jupyter adapter is ~100 languages of maintained
  machines, with `execute_request` as `feed` and rich MIME answers as
  display.
- **Browsers.** The Chrome DevTools Protocol's `Runtime.evaluate` makes a
  page a JS/TS machine — which is where quilt-wasm's in-browser `↓` wants
  to land anyway.
- **Downstream: nanobots.** Its `StateMachine` is `MeteredMachine` minus the
  language coupling; convergence means quilt supplies the trait nanobots
  hand-rolls.

## Machines that judge: proof languages

The evaluator framing ("execute statements, reduce expressions") is one
species of a more general thing: a machine *maintains a context and renders
judgments over fed programs*. Lean makes the general shape unavoidable — a
proof checker's job is not to run your theorem but to *accept or reject*
it — and the design accommodates it without a new `InnerKind`:

- **No new `InnerKind`.** The kinds are *syntactic roles*, and they already
  fit: a `theorem`/`def` is an [`Item`], a `#check`/`#eval` command is
  [`Stmt`]-like, a term is an [`Expr`]. What a proof language adds is not a
  new syntactic role but a new *judgment* over the same roles — and
  judgments are the machine's side of the protocol, not the parser's. Adding
  a `Theorem` kind would repeat the mistake `InnerKind` was designed to
  avoid: encoding one language's semantics in the shared vocabulary.
- **Checking is what feeding an Item already means.** Feeding
  `theorem t : P := proof` to a Lean machine extends the context iff the
  proof checks; rejection is the `Err` of the feed, carrying the
  diagnostic. This is the same contract every machine has — python rejects
  an Item with a syntax error the same way — proof languages just have a
  much richer rejection judgment. (A refinement worth making while alpha:
  give `Answer` an explicit `verdict` so a *rejection with output* is not
  squeezed through `Err` — accepted-with-warnings (`sorry`!) is a verdict,
  not a failure. See the API-changes section.)
- **The missing query is `type_of`, not a new kind.** An evaluator's query
  is "what value"; a checker's is "what type / does it hold". So `Machine`
  gains a second query with the same literal-answer contract:
  `type_of(expr)` answers the literal of the *type* — `int` from python's
  `type({}).__name__`, `42 : ℕ` from Lean's `#check {}`, `sat`/`unsat` from
  a solver's `check-sat`. Spelled per language as an optional `type_wrap` in
  the machine spec, defaulting to an honest error, exactly like the operator
  spellings on `MetaLanguage`. `⟨T⟩` foreshadowed this: quilt already knows
  types are language-relative spellings.

A Lean `ScriptMachine` needs nothing beyond a spec (`lean` on a replayed
file of commands; effects-free by nature, so the replay model is *exact*
for it, not degenerate) — it is unimplemented only because the toolchain is
not in the dev shell yet.

## Providers: adapters, not interpreters

Machines are instances, not singletons — you spawn them, they diverge, you may
want several. So the third registry is a factory table plus a pool (the
**park**) of live per-language defaults.

The crucial point for the "nearly all languages" tenet: providers are adapters
over machines the world already has, exactly as `Language` is an adapter over
tree-sitter grammars the world already has. Nobody writes an interpreter:

- **`ScriptMachine`** — built from a small per-language `MachineSpec` (runner
  command derived the same way `quilt run` derives it from
  `Language::hashbang`, plus the spelling for "print this value as a
  literal"). It buffers Items/Stmts and on each Expr feed replays
  `history + print(expr)` through one fresh process. This is today's `↓`
  recovered as the degenerate (memoryless-process, replay-based) machine — and
  it already upgrades today's semantics: definitions accumulate across reduces
  even though each feed forks anew (state as replayed history — the Futamura
  end of the spectrum). Its cost: effects in the history re-run on every feed,
  so it is only honest for effect-free preludes; a persistent provider lifts
  that restriction.
- **`ReplMachine`** — drives a persistent subprocess REPL from a declarative
  spec: command, prompt pattern, how to submit each `InnerKind`, how to read
  an answer back. Declared in the language's `conformance/spec/<lang>.toml`
  under a new `[machine]` section — the same file that already declares
  `meta_kind` and per-axis claims. `python3 -i`, `node -i`, `nix repl`,
  `sqlite3`, `psql`, `lean --server`, and — the paradigm case — `bash`/`zsh`,
  where a long-lived shell process holding functions, variables and a cwd *is*
  the canonical stateful machine.
- **`JupyterMachine`** — one adapter speaking the Jupyter kernel wire protocol
  buys ~100 languages' worth of maintained, stateful, answer-returning
  machines for free. The machine-side analog of vendoring tree-sitter
  grammars.
- **Native machines** where bindings exist: embedded CPython for the Python
  runtime (its `run()` returning `ns` is the seed), a retained `vm` context
  for Node, a `wasmtime::Store` for wasm targets. **(implemented for HTML)**:
  `Language::native_machine` is the hook, preferred over both subprocess
  providers, and `HtmlMachine` (`machine/html.rs`) is the first one — a
  document held as a term, in-process, whose *definitions are its ids*:
  feeding an element with a known `id` replaces it, `#id` queries it back as
  its own markup, and the typing judgment is the tag name. No interpreter
  runs; the state is the term. It is what a `quilt notebook` page is
  (`docs/wiki/notebook.md`), and the reason a language with nothing to
  *execute* can still have a machine: a machine is a thing that remembers.
- **Persistent kernels (implemented for python and typescript)**: where a
  language's own REPL cannot be sentinel-framed (Python's `...`
  continuations), a twenty-line stdin kernel can — `python3 -c` /
  `node -e` reading chunks up to the echo line and exec-ing each in one
  namespace. The `ReplMachine` protocol gained two bits for them: an
  optional `echo_err_wrap` frames stderr per feed (so a traceback is exactly
  this feed's), and an interpreter that can tell echoes `sentinel !` for a
  rejected feed, which is the `Err` the replay machine got from a non-zero
  exit. The shells spell the latter off `$?`.
- **`DbMachine`** for SQL — a connection. This flips SQL's saddest matrix
  cell: `runnable = unsupported ("executed by a database server")` becomes
  `machine = supported (provider = "db")`. Schemas, temp tables and prepared
  statements are its definitions; a `SELECT` is an Expr feed whose answer is a
  literal row-set. Combined with #219's injection-proof `LiftTo<Sql>`, Quilt
  becomes a safely-parameterized, stateful query layer with no new concepts.

## `↓` re-grounded: reduce runs on a machine, not a language

Today `↓`'s annotation names a *meta-language* (`reduce_str(target)`, issue
`#13`). Re-read it as naming a **machine**:

- **Bare `↓t`** — evaluate `t` on the ambient default machine for `t`'s
  language, drawn from the run's park. New semantics for free: two `↓`s in one
  meta-program now share definitions (`↓↖x = 5↗; … ↓↖x * 2↗` works), because
  the default machine persists for the run. Today's behavior remains as the
  `ScriptMachine` default where no better provider exists.
- **`py↓t`** — the default machine for that language, as now, but uniformly
  for every language with a provider rather than the two hand-wired pairs
  (`reduce_py()`, `reduce_rs()`).
- **`db.↓(t)` (implemented)** — machine-directed reduce, method position:
  the glyph flush against an argument list spells the host's machine-eval
  method (`db.eval(t)` in Python and Rust), completed by the source's own
  parentheses. The machine value, not an annotation, knows its language, and
  `eval` classifies what it is fed — definitions and statements feed,
  expressions answer — so one glyph serves the whole session. The operator
  forms (`t.↓`, `py↓`) are untouched, and `↓ (t)` with a space stays the
  operator.
- **`m↓t` (future)** — the annotation slot resolving machine bindings; with
  `db.↓(t)` landed this may never be needed, which answers the namespace
  question on #262 by construction.
- **Typed convenience survives**: `reduce::<T>` = eval + parse-the-literal
  back (the inverse-lift direction), with the postcard shuttle demoted from
  "the definition of reduce" to one transport a provider may use.

The biggest structural win: this dissolves the "no `QTerm` runtime" blocker
recorded in issues #132 (Lean) and #155 (Nix) and in the shell metas'
`reduce_str` errors. Those hosts lack a term *runtime* — but `↓` never needed
one; it needed a machine *client*, and "send text, splice the answer's text
back" is expressible in any host that has strings. Bash's `↓` can spell as
command substitution against a machine daemon (`$(quilt machine eval py …)`);
Lean's and Nix's as their string-interpolated equivalents (Nix's purity may
argue for keeping it an error — that becomes a policy choice per meta, not a
structural impossibility). The `Result`-returning spelling accessors were
designed for exactly this kind of honest partiality, and `MetaMachine` keeps
that shape.

## `MetaMachine`: machines as values, and the tower

`MetaMachine` is to machine operations what `MetaLanguage` is to term
construction — the host's spellings:

```rust
pub trait MetaMachine {
    fn spawn_str(&self, lang: &str) -> Result<String>;   // "spawn::<Py>()" / "quilt.spawn('py')"
    fn eval_str(&self, machine: &str) -> Result<&'static str>;  // what ↓ expands to
    fn machine_type_str(&self) -> Result<&'static str>;  // ⟨M⟩ — the ⟨T⟩ of machines
    fn machine_ref_lift(&self, target: &str) -> Result<&'static str>; // lift a handle across stages
}
```

"Meta-machines that deal with values which are themselves machines" then
falls out at two levels:

**Within a stage**, a machine is an ordinary host value (a Rust
`Box<dyn Machine>`, a Python `quilt.Machine` object), so the ground program —
the process `quilt run` launches — is already a meta-machine: a machine whose
values include machines it spawns and feeds. The runner process was always a
machine; it was just anonymous and amnesiac. The proposal reifies what
`quilt run` already does.

**Across stages** is where it gets genuinely new: a **`MachineRef`** — a
serializable address (endpoint URI for a daemonized machine) with `LiftTo`
impls. Stage N spawns a machine, feeds it definitions, then lifts the handle
itself into the stage-N+1 code it generates; when stage N+1 runs (its deferred
`↓` glyphs now live), it reconnects and evaluates against the same
environment. That is cross-stage persistence — MetaOCaml's CSP, but for
machine state rather than values — and it is what "programs referencing
definitions" means when the reference crosses a stage boundary. The plumbing
generalizes the existing `$QUILT`/`$QUILT_CHAIN` channel (add the park's
endpoint); the deferral machinery in `build_nodes` (operators at
`sky_depth > 0` surviving as glyphs) already stages `m↓` correctly, unchanged.
This mirrors what .NET Interactive does for cross-kernel variable sharing and
what Kernel-FFI does over the Jupyter protocol — and the `Stage` enum gains a
semantic reading: `Sky` depth counts quote nesting; the machine tower is its
runtime shadow, one level per live `↓` (the reflective-tower picture, in the
collapsing-towers sense that adjacent levels can fuse when the machine is a
`ScriptMachine` replay).

## Homogeneous vs heterogeneous machines

The homogeneous/heterogeneous distinction (meta-language == language, or
not) matters in exactly two places, and both are about *what can cross the
boundary*:

1. **Value transport.** Heterogeneous answers must go through the literal
   channel: text a `Language` can re-read. That is the whole design — it is
   what makes the protocol language-agnostic — but it caps fidelity at "what
   has a literal", governed by the same grid as `LiftTo`. Homogeneous pairs
   may upgrade the transport: rust↔rust already ships arbitrary
   `Serialize` values over postcard (today's `reduce::<T>`), and a
   same-process homogeneous machine can share memory outright. The rule:
   *the literal channel is the portable floor; a provider may negotiate a
   richer channel only when both ends speak the same language.* This is why
   `reduce::<T>` survives as a homogeneous fast path rather than being
   replaced.
2. **Residues and cross-stage persistence.** An `Answer::name` residue is a
   name *in one machine's environment* — meaningful only to later feeds of
   that same machine. Lifting a residue across languages is meaningless;
   lifting a `MachineRef` (the address of the machine) is how state crosses
   a stage or language boundary, and it is heterogeneous-safe precisely
   because a URI is a string in every language. MetaOCaml's cross-stage
   persistence is the homogeneous special case (same language, adjacent
   stages, values flow directly); Quilt's general case must route through
   ref + re-query.

The homogeneous case also has one structural gift: the ground program is
itself a machine of its own language, so a homogeneous `↓` *could* evaluate
in-process (the identity machine — no spawn, collapsing one tower level, in
the Amin–Rompf sense). Worth doing eventually; it must stay
indistinguishable from the subprocess semantics up to isolation.

## Examples

Two runnable ones ship with the repo:

- **`cargo run -p quiltlang --example machines`** — the machine tour: a
  python `ScriptMachine` (definitions, literal answers, the `type_of`
  judgment), a persistent bash `ReplMachine` (functions and variables as
  real process state), a live sqlite3 connection as the SQL machine
  (`typeof` as its typing judgment), and the isolation + snapshot laws
  demonstrated. One Rust program driving machines of three languages — a
  meta-machine.
- **`quilt run examples/db_menu.py.rs.quilt`** — machines at generation
  time: the ground Rust meta-program feeds a sqlite machine rows built as
  SQL *terms* (`↑` lifting Rust values into injection-safe SQL literals,
  #219), asks it aggregates, and generates a Python report with the answers
  lifted into *Python* literals. Three languages, one machine, no database
  needed by the emitted program.
- **`quilt run examples/sql_session.sql.py.quilt`** — the machine syntax
  itself: a Python ground program `spawn("sql")`s a live sqlite machine
  through the quilt-python bindings (#271) and drives it with `db.↓(↖…↗)` —
  method-position reduce — feeding schema and rows as terms, splicing one
  predicate term into two queries, and asking the `typeof` judgment. No
  `coparse` anywhere: terms go in, answers come back.

What else works on this branch today:

```console
$ quilt repl py
quilt repl — ground language py; ctrl-D to exit
py> x = 5
py> x * 8 + 2
42
py> def area(r): return 3.14159 * r * r
py> area(10)
314.159
```

A shell machine is *persistent* (state is the process), so effects run once
and definitions include functions and cwd:

```console
$ quilt repl bash
bash> y=40
bash> y + 2        # a query is an arithmetic expression
42
```

Machines as a library — the park on `Multi` is what `↓` will ride:

```rust
use quilt::lang::InnerKind;
use quilt::langs::omni::Omni;
use quilt::prelude::*;

fn main() -> Result<()> {
    let mut multi = Omni::default();

    // Feed a definition to the default python machine; it persists.
    multi.machine("py")?.feed_str(InnerKind::Item, "y = 40")?;

    // Later — different call site, same park — evaluate a *term* against it
    // and get a term back: the literal, re-read by the Language at the rim.
    let term = multi.parse_lang("py", "y + 2")?;
    let answer = multi.eval_on("py", &term)?;
    assert_eq!(answer.coparse(), "42");

    // A spawned machine is isolated from the park (the isolation law).
    let mut fresh = multi.spawn_machine("py")?;
    assert!(fresh.feed_str(InnerKind::Expr, "y").is_err());
    Ok(())
}
```

The syntax as it stands (#273). A machine is *acquired* with `⟨M⟩`, held as
an ordinary host binding, and asked the two judgments in method position:

```rust
// .py.rs.quilt — a Rust meta-program driving a live python machine.
let mut py = python⟨M⟩?;                  // = qspawn("python")
py.↓(&↖import numpy as np↗)?;             // a definition: feed
let norm = py.↓(&↖np.linalg.norm([3, 4])↗)?;  // a query: answer
let ty = py.⟨T⟩(&↖np.linalg.norm([3, 4])↗)?;  // the typing judgment
```

The annotation is optional, because the file stem has already said which
language the fragments are: a bare `⟨M⟩` resolves the way a bare `↖…↗` does,
through the chain. So a `.sql.py.quilt` file names neither the language nor
the provider —

```python
db = ⟨M⟩                                  # = spawn("sql")
db.↓(↖CREATE TEMP TABLE menu(item TEXT, price REAL);↗)
```

— and each occurrence is one machine: sharing is ordinary binding, so two
`⟨M⟩`s are two isolated machines. Which *provider* answers is the registry's
question (repl before script), which is what keeps `sqlite3` out of the
program. Like `↑ ↓ ←`, a `⟨M⟩` at sky depth > 0 is deferred, so a generated
generator can spawn machines of its own.

Where the syntax is still heading (design, not yet implemented):

```rust
// The ambient machine — a bare `↓` on a per-language default, shared by
// every reduce in the run, complementing `⟨M⟩`'s named-and-isolated handles:
↓↖import numpy as np↗;                  // feed the ambient py machine
let v: f64 = ↓↖np.linalg.norm([3, 4])↗; // 5.0 — numpy is still imported

// A machine handle lifted into the next stage:
let m = python⟨M⟩?;
let stage2 = python↖
    m = quilt.connect(↙m.reference().↑↘)   // the same machine, next stage
    print(m.eval("model.score(test)"))
↗;

// Configured spawns, on the same method-position shape (#273):
db = sql⟨M⟩("file:menu.db")             // = spawn("sql", "file:menu.db")
```

## API changes worth making while alpha

Asked directly: yes, some existing surface should move, and early alpha is
the time.

- **`hashbang()` should fold into the machine family.** It is a one-shot
  run spec wearing older clothes; `MachineSpec` already subsumes it
  (program + args came *from* it via `from_hashbang`). End state:
  `quilt run` derives its runner from the machine spec, `hashbang` becomes a
  derived convenience or disappears, and the `runnable` axis folds into
  `machine` (a language is runnable iff a script-shaped machine exists).
- **`Answer` should grow a `verdict`.** Today rejection is `Err`, which
  conflates "the machine judged this program wrong" with "the transport
  died", and cannot express accepted-with-warnings (Lean's `sorry`). A small
  `Verdict { Accepted, Rejected }` plus diagnostics on `Answer` fixes the
  checker story without touching the trait shape.
- **`typ()`/`classify_term` quality becomes semantic, not just cosmetic.**
  The REPL and every machine feed route on classification, so issue #191
  (four languages classify everything as `File`) graduates from
  matrix-cell blemish to behavior bug. Fixing it upgrades the shells' REPL
  for free.
- **`InnerKind` stays.** No `Theorem`, no `Query` variant — the machine-side
  judgments (`feed` vs `eval` vs `type_of`) carry the semantic load, and the
  syntactic vocabulary stays shared. This is the same division of labor the
  parser already lives by.
- **`Multi`'s park keying should canonicalize aliases.** `machine("py")` and
  `machine("python")` currently spawn two machines; the registries already
  have the alias maps to fix this.
- **One assumption worth loosening later: one machine, one language.** The
  trait pins `Machine::lang` to a single registry key, which fits every
  provider here but excludes genuinely polyglot machines (a GraalVM
  `Context`, a .NET Interactive session, a Jupyter kernel with magics). If
  those matter, `lang()` becomes "the default dialect" and `feed` gains an
  optional language, at no cost to current providers. This is also the one
  place the original "machines that speak *those languages*" framing was
  (mildly) narrower than the tool landscape — nothing else in the brief
  precluded a cleaner result.

## Conformance, CLI, LSP

- **New axes** in `Axis::ALL` (every language forced to answer, per the
  existing rule): `machine` (with provider kind: `script` / `repl` /
  `jupyter` / `native` / `db`), `machine-defs`, `machine-snapshot`,
  `machine-metered`. The battery gains the three laws as properties driven by
  each spec's existing sample corpus; the parity harness (`test-runtimes`)
  gains "same expr, two machines of one language, same answer" — machines
  become each other's oracle, like hosts in `cross.rs`. `runnable` narrows to
  what it really measures (shebang script-ness) or folds into `machine`.
- **CLI**: `quilt run` keeps a warm park for the run's duration (so `↓`s share
  state), `quilt machine serve/eval` exposes the daemon that string hosts and
  cross-stage refs dial, and `quilt repl` becomes nearly free: a REPL is just
  the ground machine with the expander in front — the sequencing law is
  exactly what makes it sound.
- **LSP**: `IntrospectMachine::defs()` gives `quilt-lsp` something it cannot
  get today — the machine-side symbol table — to inject into projections
  (issue #193's missing-module-context problem is a special case: the context
  is definitions the ground program will have fed by the time the fragment
  runs).

## Related work

- **REPL semantics**: van Binsbergen, Verano Merino, Jeanjean, van der Storm,
  Combemale, Barais, *A Principled Approach to REPL Interpreters* (Onward!
  2020) — defines the "sequential languages" whose sound REPLs are exactly the
  sequencing law. <https://ltvanbinsbergen.nl/publications/onward2020-repls.pdf>
- **Language-agnostic machine protocols**: the Jupyter kernel protocol (100+
  stateful kernels behind one wire format); .NET Interactive / Polyglot
  Notebooks (multiple kernels + cross-kernel variable sharing = `MachineRef`);
  MetaCall's polyglot REPL (shared scope across language REPLs); Kernel-FFI
  (transparent cross-language calls over Jupyter messaging); CodePod
  (namespaced evaluation protocols); Bacatá (notebooks for DSLs).
- **Multi-stage programming**: MetaML / MetaOCaml — `run` is `↓`, cross-stage
  persistence is `↑`; environment classifiers are the type-discipline answer
  to "can this open term run on that machine" if Quilt ever wants static
  guarantees.
- **Machines as values**: GraalVM's polyglot `Context`/`Value` (one host
  object owning many-language state, language-agnostic value views);
  wasmtime's `Engine`/`Store`/`Linker` split (provider / machine state /
  definition-wiring — and fuel = `MeteredMachine`); Racket namespaces and
  sandboxed evaluators (`make-evaluator` is literally a machine value);
  Smalltalk/Lisp images (snapshot/restore).
- **Meta-machines**: reflective towers (3-Lisp, Brown, Black) and Amin &
  Rompf, *Collapsing Towers of Interpreters* (POPL 2018) — the theory of when
  a tower of machines-running-machines can be fused; the Futamura projections
  are the `ScriptMachine`-replay end of that spectrum.
- **In-repo**: nanobots' `StateMachine` trait is `MeteredMachine` minus the
  language coupling — convergence means Quilt eventually supplies the
  abstraction nanobots hand-rolls, closing the loop with its gas-metered,
  resumable, GPU-scheduled machines.

## Phases

Each step useful alone:

1. **(implemented by this document's branch)** `Machine` + `Answer` +
   `ScriptMachine` + `MachineSpec` (`quilt/src/machine.rs`, always compiled,
   no tree-sitter deps); `Language::machine_spec` (the `hashbang` pattern);
   the `Park` of live per-language defaults on `Multi`, with
   `Multi::machine(lang)` and the term-level `Multi::eval_on(lang, term)`
   closing the text→term loop at the rim.
2. **(implemented)** `[machine]` spec sections + the `machine` axis + the
   three laws in the battery (`probe_machine`). The warm park in `quilt run`
   waits for phase 4's `↓` respelling — until `↓` expands to machine evals,
   a CLI-side park has no consumer.
3. **(first slice implemented)** `ReplMachine` — a persistent interpreter
   with sentinel-framed feeds, driving bash and zsh (flipped to
   `machine = supported`); still to come: sqlite/nix providers, then
   `JupyterMachine`.
4. **(`quilt repl`, the method-position judgments, the machine glyph, and
   python bindings implemented)** — each REPL line parsed, expanded,
   classified and fed to the ground language's park machine; `db.↓(term)`
   and `db.⟨T⟩(term)` spell the machine's value and typing judgments in
   Python and Rust (`reduce_method_str` / `type_method_str`, pinned in the
   specs); quilt-python exposes `spawn`/`Machine` (#271); and `⟨M⟩` — the
   machine glyph, chain-defaulted like bare quotes — is machine
   *acquisition*, so `db = ⟨M⟩` replaces `spawn("sql")` entirely (#273) and
   a `.sql.py.quilt` program drives a live sqlite machine with quilt syntax
   end to end, naming neither the language nor the provider. Still to come:
   `MachineRef` + `LiftTo` impls, `quilt machine serve`.
5. **(traits implemented)** Capability subtraits (`Snapshot` / `Metered` /
   `Introspect`, plus `type_of` on `Machine`); still to come: the LSP and
   nanobots integrations behind them.
6. **(implemented)** The first meta-machine *artifact*: notebooks
   (`quilt/src/notebook.rs`, `docs/wiki/notebook.md`). An `.html.quilt`
   page is a program whose quotes are cells; HTML's meta is the identity
   (so `quilt check` validates every cell without running one) and its
   machine is native (the page, definitions by id); each cell is expanded
   by its own meta and fed to its language's park machine, its output is
   fed back to the page — redefining ids elsewhere on it — and an unquote
   that reaches the page (`↙#id↘`) reads an element as a literal of the
   cell's language, which is how machines that share nothing exchange
   values. Output is read as Quilt-in-HTML, so cells create cells (capped
   by generation). The python and typescript kernels above were built for
   it, and `quilt repl html` is the same session a line at a time.
7. **(implemented)** The notebook, *live* (`quilt/src/serve.rs`,
   `quilt notebook --serve`): the same session behind a socket, with an
   editor and the page in front of it. Two items from the lists above land
   here. **"The browser as a JavaScript machine"** — a TypeScript cell is
   expanded on the server and evaluated in the *page's own realm*, so its
   `document` is the page it is editing and its definitions outlive the
   cell; its answer crosses back as an ordinary `Answer`, which is what
   makes *where* a cell ran invisible to the notebook. And the **`quilt
   machine serve` daemon**, in the shape this artifact needs: the page is
   one of the machines, an `/app/…` request is dispatched to the Python
   machine *as a feed* (no second port, no thread in the kernel, every
   request serialized with cell execution), and the SQL machine is parked on
   a session file whose path the Python kernel is given — so SQL cells, a
   Python backend and a TypeScript view build one small website inside one
   session (`examples/notebook/cafe.html.quilt`). Still to come:
   `MachineRef`, so a cell can hold a handle to another machine
   (`/machines/sql/eval`) instead of going through the page.

## Open questions

- The name: `Machine` fits the framing and nanobots; `Session` / `Kernel` are
  the alternatives.
- Whether `m↓`'s shared namespace between machine bindings and language names
  wants a resolution rule or a distinct glyph.
- Sandbox policy: `↓` already executes code at expansion time, but persistent
  machines widen the blast radius, so machine spawning should probably be
  capability-gated per invocation (deny-by-default in `quilt check`).
- Whether Nix's `↓` should exist at all, given evaluation purity.
- Rust's machine: the flat replay of `ScriptMachine` does not fit Rust's
  item/`fn main` split; Rust wants a structured provider (items outside,
  stmts/expr inside `main`) or the existing one-shot `reduce` until then.
