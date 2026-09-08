//! A tour of machines (`docs/design/machines.md`): stateful evaluators of a
//! language behind one trait, driven here across three languages from one
//! Rust program — which makes this program a *meta-machine*: a machine whose
//! values include machines.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p quiltlang --example machines
//! ```
//!
//! Needs `python3`, `bash` and `sqlite3` on `PATH` (all present in the dev
//! shell and on CI images).

use quilt::lang::{InnerKind, Language as _};
use quilt::langs::omni::Omni;
use quilt::machine::{Machine, ScriptMachine, SnapshotMachine};
use quilt::prelude::*;

fn main() -> Result<()> {
    let mut multi = Omni::default();

    // ── python: a ScriptMachine — state as replayed history ──────────────
    // Feed a definition; it persists to later queries. A query's answer is
    // the value's own literal (`repr`), the inverse of lift.
    println!("── python (script machine)");
    multi
        .machine("py")?
        .feed_str(InnerKind::Item, "def area(r):\n    return 3.14159 * r * r")?;
    let term = multi.parse_lang("py", "area(10)")?;
    println!(
        "  area(10)          = {}",
        multi.eval_on("py", &term)?.coparse()
    );
    let typed = multi.machine("py")?.type_of("area(10)")?;
    println!("  type_of(area(10)) = {}", typed.value.unwrap_or_default());

    // ── bash: a ReplMachine — state is a live process ────────────────────
    // One long-lived shell; functions and variables are real process state,
    // so effects run once and nothing is replayed.
    println!("── bash (persistent repl machine)");
    let sh = multi.machine("bash")?;
    sh.feed_str(InnerKind::Item, "double() { echo $(( $1 * 2 )); }")?;
    sh.feed_str(InnerKind::Item, "x=$(double 21)")?;
    println!(
        "  x                 = {}",
        sh.feed_str(InnerKind::Expr, "x")?.value.unwrap_or_default()
    );

    // ── sql: a database connection is the machine ────────────────────────
    // Temp tables are its definitions; `typeof` is its typing judgment.
    println!("── sql (a live sqlite3 connection)");
    let db = multi.machine("sql")?;
    db.feed_str(
        InnerKind::Item,
        "CREATE TEMP TABLE menu(item TEXT, price REAL);",
    )?;
    db.feed_str(
        InnerKind::Stmt,
        "INSERT INTO menu VALUES ('espresso', 2.5), ('latte', 3.75);",
    )?;
    let total = "(SELECT SUM(price) FROM menu)";
    println!(
        "  total             = {}",
        db.feed_str(InnerKind::Expr, total)?
            .value
            .unwrap_or_default()
    );
    println!(
        "  type_of(total)    = {}",
        db.type_of(total)?.value.unwrap_or_default()
    );

    // ── the laws ─────────────────────────────────────────────────────────
    // Isolation: a freshly spawned machine shares nothing with the park.
    println!("── laws");
    let mut fresh = multi.spawn_machine("py")?;
    println!(
        "  isolation: fresh python rejects area(1)? {}",
        fresh.feed_str(InnerKind::Expr, "area(1)").is_err()
    );

    // Snapshots: a ScriptMachine's whole state is its history, so rollback
    // is exact.
    let spec = multi
        .get_lang("py")?
        .machine_spec()
        .expect("python registers a machine spec");
    let mut draft = ScriptMachine::new("py", spec);
    draft.feed_str(InnerKind::Item, "mode = 'draft'")?;
    let saved = draft.snapshot()?;
    draft.feed_str(InnerKind::Item, "mode = 'final'")?;
    draft.restore(&saved)?;
    println!(
        "  snapshot: mode after restore = {}",
        draft
            .feed_str(InnerKind::Expr, "mode")?
            .value
            .unwrap_or_default()
    );
    Ok(())
}
