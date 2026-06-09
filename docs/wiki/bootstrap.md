# Bootstrap

**Files:** `rust/quilt/src/langs/bootstrap/`, `bin/bootstrap`, `bin/bootstrap0`, `bin/bootstrap1`

Quilt is self-hosting: the Rust `MetaLanguage` implementation (`langs/rust/meta.rs`) is *generated* by a Quilt program (`langs/bootstrap/mk_meta.rs.quilt`), not hand-written. This page explains how that works.

## Why bootstrap?

`RustMetaLanguage` expands Rust `.quilt` files. To expand `mk_meta.rs.quilt` (which contains `⟨T⟩` type placeholders and quote/unquote syntax) we need *some* `MetaLanguage` first. The `Bootstrap` meta-language serves this role: it works without `meta.rs` and uses a slower string-based lifting strategy (`strlift.rs`).

## The two stages

### Stage 0 — `bootstrap0`

Expand `mk_meta.rs.quilt` using the `Bootstrap` multi (which uses `BootstrapMetaLanguage`):

```sh
quilt expand -m bootstrap rust/quilt/src/langs/bootstrap/mk_meta.rs.quilt
```

This produces `mk_meta.rs` — a plain Rust program (a `rust-script` file).

### Stage 1 — `bootstrap1`

Run the generated `mk_meta.rs` with `rust-script`. That program itself:
1. Uses the Quilt library (`use quilt::prelude::*;`) to build `RustMetaLanguage`'s implementation.
2. Uses `⟨T⟩` (which it itself expands to `Arc<QTerm>`) to avoid hard-coding the type.
3. Writes `rust/quilt/src/langs/rust/meta.rs` and runs `cargo fmt` on it.

`bootstrap` (no suffix) runs stage 0 then stage 1. If `meta.rs` is already correct and nothing has changed in `mk_meta.rs.quilt`, both stages leave the file unchanged (idempotent).

## `mk_meta.rs.quilt`

This is a Rust source file (a `rust-script` script) that uses:
- `⟨T⟩` for every occurrence of `Arc<QTerm>` (expanded by bootstrap → `Arc<QTerm>`).
- `↖…↗` to quote the body of `meta.rs` at stage 1 (so the Quilt machinery generates the file's content as a `QTerm`).

The structure is roughly:

```rust
#!/usr/bin/env rust-script
use quilt::prelude::*;
use quilt::term::STerm;

fn main() -> Result<()> {
    let meta: ⟨T⟩ = ↖
        // ... full RustMetaLanguage impl body ...
        // uses ⟨T⟩ for Arc<QTerm> again inside the quote
    ↗;
    meta.dump("rust/quilt/src/langs/rust/meta.rs")?;
    // cargo fmt
    Ok(())
}
```

## `BootstrapMetaLanguage`

**File:** `langs/bootstrap/meta.rs`

Implements `MetaLanguage` using the string-lift strategy from `strlift.rs` instead of the direct-builder strategy in `langs/rust/ops.rs`. Specifically, `expand_tuple` calls `strlift::bs_lift` which renders the entire sub-tree to a string and re-parses it. This is slower and less structured, but avoids the circular dependency on `RustMetaLanguage`.

The operator spellings for bootstrap differ from the production Rust spellings:

| Glyph | Bootstrap       | Rust (production)      |
|-------|-----------------|------------------------|
| `↑`   | `"bs_lift()"`   | `"qlift()"`            |
| `↓`   | `"bs_reduce()"` | `"reduce()"`           |
| `⟨T⟩` | `"Arc<QTerm>"`  | (via bootstrap output) |
| `⟨N⟩` | `"bs_name()"`   | `"name()"`             |

## `Bootstrap` multi type

```rust
pub type Bootstrap = Multi<Singleton<BootstrapRustLanguage>, Singleton<BootstrapMetaLanguage>>;
```

`BootstrapRustLanguage` is `RustLanguage` under the hood — bootstrap re-uses the production Rust parser. The `Multi` CLI argument `-m bootstrap` selects this multi.

## Idempotency

A clean run of `bootstrap` leaves `meta.rs` byte-for-byte unchanged. This is verified in CI by checking that the file has no diff after running bootstrap. If `meta.rs` diverges, it means a change to `mk_meta.rs.quilt` or a breaking change in `ops.rs` that must be reconciled.

## Known issue

`bootstrap1` (stage 1) is currently broken at HEAD because `rust-script` sees a dependency conflict. `bootstrap0` (expanding `mk_meta.rs.quilt` to `mk_meta.rs`) still works. See project memory for context.
