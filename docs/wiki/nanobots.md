# Nanobots

**Directory:** `rust/nanobots/` (separate Cargo workspace)

Nanobots is a gas-metered IR toolchain for building resumable, GPU-friendly state machines. It uses Quilt (`html↖…↗`, `wgsl↖…↗`) for code generation in some of its crates.

## Concept

Programs are compiled into state machines where execution can be paused and resumed. Each *run* is capped by a **gas limit** — the maximum abstract work units before the machine must yield. This makes it safe to run many agents concurrently (including on the GPU).

Gas is checked at every basic-block boundary. `yield` is an explicit suspension point.

## Languages

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
# Interpret a .nb or .nbir file (tree-walking, no compilation)
nanobots interpret demos/counter.nb

# Lower a .nb file to .nbir text (prints generated IR)
nanobots lower demos/counter.nb

# Compile to a rust-script file
nanobots build demos/counter.nb MyBot . --output bot.rs

# Compile and run in one step
nanobots run demos/counter.nb MyBot .
```

## Crate structure

```
nanobots/crates/
├── nanobots-hir/      # HIR types + .nb parser + HIR → IR lowering
├── nanobots-ir/       # IR types + text parser
├── nanobots-vm/       # StateMachine trait + tree-walking interpreter
├── nanobots-lang/     # Combined pipeline (lex/parse/codegen)
├── nanobots-codegen/  # Code generator (Program → Rust) — uses wgsl↖…↗ quotes
├── nanobots-game/     # Game logic (agent scheduling)
├── nanobots-wasm/     # WASM bindings
├── nanobots-web/      # Web pages — uses html↖…↗ quotes
└── nanobots/          # CLI entry point
```

## `StateMachine` trait

```rust
pub trait StateMachine {
    fn max_gas_to_resume(&self) -> u32;
    fn add_gas(&mut self, amount: u32);
    fn run(&mut self, max_gas: u32) -> RunResult;
}
```

Scheduler loop:

```rust
loop {
    machine.add_gas(gas_per_round);
    match machine.run(max_gas_cap) {
        RunResult::Finished  => break,
        RunResult::Suspended => {} // next round
    }
}
```

## Use of Quilt in nanobots

`nanobots-web/src/html.rs.quilt` generates HTML page templates using `html↖…↗` quotes. `nanobots-codegen/src/wgsl.rs.quilt` generates WGSL shader code using `wgsl↖…↗` quotes inside Rust. These are pre-expanded; the checked-in `.rs` files are the generated output.

## Building

Nanobots lives in its own Cargo workspace (`rust/nanobots/`), separate from the main quilt workspace.

```sh
cd rust/nanobots
cargo build
cargo run -- interpret demos/counter.nb
```
