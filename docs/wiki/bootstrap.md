# Bootstrap

**Files:** `quilt/src/langs/bootstrap/`, `bin/bootstrap`, `bin/bootstrap0`, `bin/bootstrap1`

Quilt is self-hosting: the Rust `MetaLanguage` implementation (`langs/rust/meta.rs`) is *generated* by a Quilt program (`langs/bootstrap/mk_meta.rs.quilt`), not hand-written. This page explains how that works.

## Why bootstrap?

`RustMetaLanguage` expands Rust `.quilt` files. To expand `mk_meta.rs.quilt` (which contains `⟨T⟩` type placeholders and quote/unquote syntax) we need *some* `MetaLanguage` first. The `Bootstrap` meta-language serves this role: it works without `meta.rs` and uses a slower string-based lifting strategy (`strlift.rs`).

## The two stages

Both stages run the same generator program via `quilt`, `mk_meta.rs.quilt`. The program:
1. Uses the Quilt library — `quilt::prelude`, which `quilt run` imports for it (issue #274) — to build `RustMetaLanguage`'s implementation as a `QTerm`.
2. Uses `⟨T⟩` (expanded to `Arc<QTerm>`) to avoid hard-coding the type.
3. Writes `quilt/src/langs/rust/meta.rs` and runs `cargo fmt` on it.

The stages differ only in which `MetaLanguage` expands the generator.

### Stage 0 — `bootstrap0`

Expand and run `mk_meta.rs.quilt` using the `Bootstrap` multi (`BootstrapMetaLanguage`), with the CLI built `--no-default-features -F bootstrap`:

```sh
cd quilt && cargo run -p quiltlang --no-default-features -F bootstrap -- run -m bootstrap src/langs/bootstrap/mk_meta.rs.quilt
```

This works without an existing `meta.rs` and regenerates it.

### Stage 1 — `bootstrap1`

Expand and run `mk_meta.rs.quilt` again, this time with the `Omni` multi — i.e. the freshly generated `RustMetaLanguage` (self-hosting):

```sh
cd quilt && cargo run -p quiltlang --no-default-features -F rust,parse -- run -m omni src/langs/bootstrap/mk_meta.rs.quilt
```

`bootstrap` (no suffix) runs stage 0 then stage 1. If `meta.rs` is already correct and nothing has changed in `mk_meta.rs.quilt`, both stages leave the file unchanged (idempotent).

## `mk_meta.rs.quilt`

This is a Rust source file (a `rust-script` script) that uses:
- `⟨T⟩` for every occurrence of `Arc<QTerm>` (expanded by bootstrap → `Arc<QTerm>`).
- `↖…↗` to quote the body of `meta.rs` (so the Quilt machinery generates the file's content as a `QTerm`).

The structure is roughly:

```rust
#!/usr/bin/env rust-script
use quilt::term::STerm;

fn main() -> Result<()> {
    let meta: ⟨T⟩ = ↖
        // ... full RustMetaLanguage impl body ...
        // uses ⟨T⟩ for Arc<QTerm> again inside the quote
    ↗;
    meta.dump_with_cmds("src/langs/rust/meta.rs", …)?;
    // cargo fmt
    Ok(())
}
```

## `BootstrapMetaLanguage`

**File:** `langs/bootstrap/meta.rs`

Implements `MetaLanguage` using the string-lift strategy from `strlift.rs` instead of the direct-builder strategy in `langs/rust/ops.rs` (whose builder-call fragments come from the shared fold in `langs/chain/`). Specifically, the `expand_*` methods render the builder code for each term as a string (`StrLift::strlift`, e.g. `format!("b_.write({})", …)`) and re-parse it with the Rust parser the meta holds (`parse_expr` / `parse_stmt`). This is slower and less structured, but avoids the circular dependency on `RustMetaLanguage` — and because it deliberately does *not* share the `langs/chain/` fold, it doubles as an independent oracle: the `expand_both` tests compare its output against `RustMetaLanguage`'s.

The operator spellings for bootstrap differ from the production Rust spellings:

| Glyph | Bootstrap       | Rust (production)      |
|-------|-----------------|------------------------|
| `↑`   | `"bs_lift()"`   | `"qlift()"` (rust→rust; `"qlift_to::<L>()"` into another language) |
| `↓`   | `"bs_reduce()"` | `"reduce()"` (`"reduce_py()"` via Python) |
| `⟨T⟩` | `"Arc<QTerm>"`  | `"Arc<QTerm>"`         |
| `⟨N⟩` | `"bs_name"`     | `"name"`               |

Bootstrap ignores the target: it has no heterogeneous lift or reduce. Both metas emit the same `use quilt::prelude::*;` (`quilt::prelude` re-exports `bs_lift` and `qlift` alike), so `quilt expand` and `quilt expand -m bootstrap` differ only in those calls.

## `Bootstrap` multi type

```rust
pub type Bootstrap = Single<lang::BootstrapLanguage, meta::BootstrapMetaLanguage>;
```

`BootstrapLanguage` (`langs/bootstrap/lang.rs`) is a plain re-export of `RustLanguage` — bootstrap re-uses the production Rust parser; only the meta is special. The CLI argument `-m bootstrap` selects this multi (the default is `-m omni`); it is only available when quilt is built with the `bootstrap` feature.

## Idempotency

A clean run of `bootstrap` leaves `meta.rs` byte-for-byte unchanged. `bin/check-bootstrap` verifies this: it runs both stages and fails if `git diff` shows `meta.rs` modified. It runs in CI (the "Bootstrap (self-host no-op)" job in `.github/workflows/ci.yml`) and in the `main check` pre-commit gate. Like the stages themselves it needs `rust-script` and `rustfmt`. If `meta.rs` diverges, it means a change to `mk_meta.rs.quilt` or a breaking change in `ops.rs` that must be reconciled.

## What else could be bootstrapped

`meta.rs` is no longer the only file Quilt generates about itself. Two more are
produced the same way — a `.rs.quilt` program run with `quilt run -m omni`, which
writes the file and `cargo fmt`s it — each with a drift gate in CI and `main check`:

| Generated file | Generator | Regenerate / gate |
|---|---|---|
| `quilt/src/lift/gen.rs` — every `LiftTo` impl | `quilt/src/lift/mk_lifts.rs.quilt` | `bin/gen-lifts` / `bin/check-lifts` (#203) |
| `quilt/src/langs/chain/gen.rs` — the builder-call fragments | `quilt/src/langs/chain/mk_chain.rs.quilt` | `bin/gen-chain` / `bin/check-chain` (#204) |

Unlike `meta.rs` these need no stage 0: they are expanded by the already-built
`RustMetaLanguage`. (`quilt/src/langs/arity.rs` is generated too, by `bin/gen-arity`,
but from the grammars' `grammar.json` by plain Rust in `quilt-conformance` — not by
a Quilt program.)

The [Dogfooding Audit](dogfooding.md) surveys the rest of the tree for code that
is hand-written but could be produced the same way — and, just as usefully, the
places where it should not be.
