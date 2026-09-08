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

**The wire is text, not trees** (tenet 2). Almost every real machine — a REPL
subprocess, a Jupyter kernel, a DB connection, `nix repl` — accepts source
text. Making `feed_str` the primitive and `feed(term)` a coparse-wrapper means
a machine implementation never touches `QTerm` internals, exactly as tenet 2
demands ("languages already expose textual syntax as their primary
interface"). It also means `machine.rs` lives beside `lift.rs` in the
tree-sitter-free runtime half of the crate.

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

### Capabilities as subtraits

Mirroring how the capability matrix refuses to pretend all languages are
equal:

```rust
pub trait SnapshotMachine: Machine {          // fork/rollback (Racket
    fn snapshot(&self) -> Result<Snapshot>;   // namespaces, Smalltalk images)
    fn restore(&mut self, s: &Snapshot) -> Result<()>;
}
pub trait MeteredMachine: Machine {           // nanobots' StateMachine shape
    fn add_gas(&mut self, amount: u32);       // and wasmtime fuel
    fn run(&mut self, max_gas: u32) -> Result<RunResult>; // Finished | Suspended
}
pub trait IntrospectMachine: Machine {        // what's defined — for the LSP
    fn defs(&self) -> Result<Vec<Def>>;       // to project machine state
}
```

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
  for Node, a `wasmtime::Store` for wasm targets.
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
- **`m↓t` (new)** — `m` resolves first as an in-scope host binding of machine
  type, then as a language name. In host code this is just sugar: `↓` expands
  to `m.eval(&t)` / `machine("py").eval(&t)` instead of today's `reduce()`.
  The annotation grammar (`([a-z][a-z0-9]*)?↓`) already parses it.
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
4. **(`quilt repl` implemented)** — each line parsed, expanded, classified
   and fed to the ground language's park machine. Still to come:
   `MetaMachine` spellings, `⟨M⟩`, `m↓` resolution, `MachineRef` + `LiftTo`
   impls, `quilt machine serve`.
5. Capability subtraits (`Snapshot` / `Metered` / `Introspect`) and the
   LSP / nanobots integrations.

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
