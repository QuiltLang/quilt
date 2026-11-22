# Nanobots

**Repo:** `../nanobots` ([QuiltLang/nanobots](https://github.com/QuiltLang/nanobots)) — a sibling repository with its own Cargo workspace · **Demo:** [quiltlang.github.io/nanobots](https://quiltlang.github.io/nanobots/)

Nanobots is a gas-metered IR toolchain for building resumable, GPU-friendly state machines. It uses Quilt (`html↖…↗`, `wgsl↖…↗`) for code generation in some of its crates.

## Concept

Programs are compiled into state machines where execution can be paused and resumed. Each *run* is capped by a **gas limit** — the maximum abstract work units before the machine must yield. This makes it safe to run many agents concurrently (including on the GPU).

Gas is checked at every basic-block boundary. `yield` is an explicit suspension point.

## Languages

There are three source formats, all accepted by the CLI: `.nbl` (a Python-like language with structured control flow — `while`, `if`, `for _ in range(n)` — compiled by `nanobots-lang`), `.nb` (below) and the `.nbir` IR.

### High-level (`.nb`)

The nanobot source language. Adds named variables, IO register declarations, if-goto, and mid-block yield on top of the IR.

```nb
in  r0: nearby_resource
out r4: move_cmd

idle:
  if nearby_resource != 0: goto hunting
  move_cmd = 0
  yield idle

hunting:
  let dist = r1
  if dist < 2: goto collecting
  move_cmd = 1
  yield hunting
```

### Low-level IR (`.nbir`)

The compilation target; can also be written by hand.

```ir
entry:
  r0 = const 10
  r1 = fconst 1.5
  r2 = r0 + r1
  branch r2 -> yes / no
  yield -> next
  goto label
  return
```

All registers are `[u32; 4]`; component `[0]` holds scalar values.

Integer ops: `+`, `-`, `*`, `/`, `%`, `==`, `<`
Float ops: `f+`, `f-`, `f*`, `f/`, `f==`, `f<`

## CLI

```sh
# Interpret a .nbl, .nb or .nbir file (tree-walking, no compilation)
nanobots interpret demos/counter.nb

# Lower a .nb or .nbl file to .nbir text (prints generated IR)
nanobots lower demos/counter.nb

# Compile to a rust-script file
nanobots build demos/counter.nb MyBot . --output bot.rs

# Compile and run in one step
nanobots run demos/counter.nb MyBot .

# Generate the self-contained WebGPU HTML viewer (the default subcommand)
nanobots web --editor --output build/index.html --no-open demos/wander.nbl
```

Also `game` (the grid game in a TUI) and `check` (verify the CPU simulation is deterministic across runs).

## Crate structure

```
nanobots/
├── nanobots-hir/      # HIR types + .nb parser + HIR → IR lowering
├── nanobots-ir/       # IR types + text parser
├── nanobots-vm/       # StateMachine trait + tree-walking interpreter
├── nanobots-lang/     # The Python-like .nbl language (lex/parse/codegen → IR)
├── nanobots-codegen/  # Code generator (Program → Rust, WGSL) — uses wgsl↖…↗ quotes
├── nanobots-game/     # Game logic (agent scheduling)
├── nanobots-wasm/     # WASM bindings (uses quilt only through nanobots-codegen)
├── nanobots-web/      # WebGPU HTML viewer — uses html↖…↗ and wgsl↖…↗ quotes
└── nanobots/          # CLI entry point
```

## `StateMachine` trait

```rust
pub trait StateMachine {
    fn max_gas_to_resume(&self) -> u32;   // gas needed for the current block
    fn run(&mut self) -> RunResult;       // run within the baked-in per-round budget
}
```

The per-round gas budget is baked in when the machine is built (`--gas-per-round`), and gas does not carry over between rounds. `IoMachine: StateMachine` adds `read_register` / `write_register` for game I/O. Scheduler loop:

```rust
loop {
    match machine.run() {
        RunResult::Finished  => break,
        RunResult::Suspended => {} // next round, fresh gas
    }
}
```

## Use of Quilt in nanobots

Three `.rs.quilt` sources, each named with its language chain (the middle extension is the default language of un-annotated quotes):

- `nanobots-web/src/html.html.rs.quilt` — the viewer/editor HTML pages, using `html↖…↗` quotes.
- `nanobots-web/src/shaders.wgsl.rs.quilt` — the WGSL render/compositing shaders.
- `nanobots-codegen/src/wgsl.wgsl.rs.quilt` — the WGSL compute shaders (run + resolve passes) generated per program.

The generated `html.html.rs` / `shaders.wgsl.rs` / `wgsl.wgsl.rs` siblings are **not** checked in (gitignored); `bin/expand` produces them, and the crates pull them in with `#[path = "….rs"] mod …;`.

## Building

Nanobots lives in its own repository and Cargo workspace, separate from the quilt workspace. It depends on quilt as a library — a git dependency on package `quiltlang`, declared once in the workspace `Cargo.toml` with `default-features = false`, plus `features = ["rust"]` in the two crates that use it (`nanobots-codegen`, `nanobots-web`). That is quilt's tree-sitter-free runtime, so `nanobots-wasm` can build for `wasm32-unknown-unknown`. It also uses the quilt CLI from the sibling checkout (`../quilt/bin/quilt`, override with `$QUILT`) to expand its `.rs.quilt` sources (`bin/expand`, run automatically by `bin/build`).

The dependency is **pinned to tag `v0.1.0-alpha`**: the expander's output is version-specific, and later quilt (the InnerKind Block/Item emit changes, #58) is incompatible with these sources. The sibling checkout used by `bin/expand` must be on the same tag; bump both deliberately, re-expanding when you do.

```sh
cd ../nanobots
bin/expand                            # expand *.rs.quilt → *.rs (gitignored)
cargo build
cargo run -- interpret demos/counter.nb
bin/build && bin/run                  # wasm-pack + release build → build/index.html, then open it
```

The hosted demo is built the same way: the [`quiltlang.github.io`](https://github.com/QuiltLang/quiltlang.github.io) site's deploy CI runs `bin/build` and serves the result at [quiltlang.github.io/nanobots](https://quiltlang.github.io/nanobots/) (the old `/demo/nanobots/` URL redirects there).

On the quilt side, `bin/check-downstream` (nightly, informational — issue #189) clones nanobots, rewrites its quilt dependency to a path pointing at the current checkout, expands its `.rs.quilt` sources with that expander, and builds the two consuming crates; `bin/check-features` separately proves `quiltlang` builds with `default-features = false` and no tree-sitter.

Nanobots has its own `bin/install_tools`, which delegates to quilt's installer for the editor tooling (quilt-lsp + the VS Code extension) and warns if `wasm-pack` is missing.
