# Bootstrap

**Files:** `quilt/src/langs/bootstrap/`, `bin/bootstrap`, `bin/bootstrap0`, `bin/bootstrap1`

Quilt is self-hosting: the Rust `MetaLanguage` implementation (`langs/rust/meta.rs`) is *generated* by a Quilt program (`langs/bootstrap/mk_meta.rs.quilt`), not hand-written. This page explains how that works.

## Why bootstrap?

`RustMetaLanguage` expands Rust `.quilt` files. To expand `mk_meta.rs.quilt` (which contains `⟨T⟩` type placeholders and quote/unquote syntax) we need *some* `MetaLanguage` first. The `Bootstrap` meta-language serves this role: it works without `meta.rs` and uses a slower string-based lifting strategy (`strlift.rs`).

## The two stages

Both stages `quilt run` the same generator program, `mk_meta.rs.quilt`. The program:
1. Uses the Quilt library (`use quilt::prelude::*;`) to build `RustMetaLanguage`'s implementation as a `QTerm`.
2. Uses `⟨T⟩` (expanded to `Arc<QTerm>`) to avoid hard-coding the type.
3. Writes `quilt/src/langs/rust/meta.rs` and runs `cargo fmt` on it.

The stages differ only in which `MetaLanguage` expands the generator.

### Stage 0 — `bootstrap0`

Expand and run `mk_meta.rs.quilt` using the `Bootstrap` multi (`BootstrapMetaLanguage`), with the CLI built `--no-default-features -F bootstrap`:

```sh
cd quilt && cargo run -p quilt --no-default-features -F bootstrap -- run -m bootstrap src/langs/bootstrap/mk_meta.rs.quilt
```

This works without an existing `meta.rs` and regenerates it.

### Stage 1 — `bootstrap1`

Expand and run `mk_meta.rs.quilt` again, this time with the `Omni` multi — i.e. the freshly generated `RustMetaLanguage` (self-hosting):

```sh
cd quilt && cargo run -p quilt --no-default-features -F rust,parse -- run -m omni src/langs/bootstrap/mk_meta.rs.quilt
```

`bootstrap` (no suffix) runs stage 0 then stage 1. If `meta.rs` is already correct and nothing has changed in `mk_meta.rs.quilt`, both stages leave the file unchanged (idempotent).

## `mk_meta.rs.quilt`

This is a Rust source file (a `rust-script` script) that uses:
- `⟨T⟩` for every occurrence of `Arc<QTerm>` (expanded by bootstrap → `Arc<QTerm>`).
- `↖…↗` to quote the body of `meta.rs` (so the Quilt machinery generates the file's content as a `QTerm`).

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
    meta.dump_with_cmds("src/langs/rust/meta.rs", …)?;
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
