//! A tour of machines (`docs/design/machines.md`): stateful evaluators of a
//! language behind one trait, driven here across three languages from one
//! Rust program — which makes this program a *meta-machine*: a machine whose
//! values include machines.
//!
//! Machines traffic in terms; raw text enters through `Multi::feed_on`,
//! where the `Language` that can parse (and so validate) lives, and answers
//! come back out as terms through `Multi::eval_on`.
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
    // Feed a definition; it persists to later queries. Answers are terms:
    // the value's own literal, re-read by the Language at the rim.
    println!("── python (script machine)");
    multi.feed_on(
        "py",
        InnerKind::Item,
        "def area(r):\n    return 3.14159 * r * r",
    )?;
    let query = multi.parse_lang("py", "area(10)")?;
    println!(
        "  area(10)          = {}",
        multi.eval_on("py", &query)?.coparse()
    );
    let typed = multi.machine("py")?.type_of(&query)?;
    println!("  type_of(area(10)) = {}", typed.value.unwrap_or_default());

    // ── bash: a ReplMachine — state is a live process ────────────────────
    // One long-lived shell; functions and variables are real process state,
    // so effects run once and nothing is replayed.
    println!("── bash (persistent repl machine)");
    multi.feed_on("bash", InnerKind::Item, "double() { echo $(( $1 * 2 )); }")?;
    multi.feed_on("bash", InnerKind::Item, "x=$(double 21)")?;
    let x = multi.parse_lang("bash", "x")?;
    println!(
        "  x                 = {}",
        multi.eval_on("bash", &x)?.coparse()
    );

    // ── sql: a database connection is the machine ────────────────────────
    // Temp tables are its definitions; `typeof` is its typing judgment.
    println!("── sql (a live sqlite3 connection)");
    multi.feed_on(
        "sql",
        InnerKind::File,
        "CREATE TEMP TABLE menu(item TEXT, price REAL);",
    )?;
    multi.feed_on(
        "sql",
        InnerKind::File,
        "INSERT INTO menu VALUES ('espresso', 2.5), ('latte', 3.75);",
    )?;
    let total = multi.parse_lang("sql", "(SELECT SUM(price) FROM menu)")?;
    println!(
        "  total             = {}",
        multi.eval_on("sql", &total)?.coparse()
    );
    println!(
        "  type_of(total)    = {}",
        multi
            .machine("sql")?
            .type_of(&total)?
            .value
            .unwrap_or_default()
    );

    // ── the laws ─────────────────────────────────────────────────────────
    // Isolation: a freshly spawned machine shares nothing with the park.
    println!("── laws");
    let probe = multi.parse_lang("py", "area(1)")?;
    let mut fresh = multi.spawn_machine("py")?;
    println!(
        "  isolation: fresh python rejects area(1)? {}",
        fresh.eval(&probe).is_err()
    );

    // Snapshots: a ScriptMachine's whole state is its history, so rollback
    // is exact. (Constructed directly here to reach the concrete type; its
    // inherent `feed_str` is the provider's raw door.)
    let spec = multi
        .get_lang("py")?
        .machine_spec()
        .expect("python registers a machine spec");
    let mut draft = ScriptMachine::new("py", spec);
    draft.feed_str(InnerKind::Item, "mode = 'draft'")?;
    let saved = draft.snapshot()?;
    draft.feed_str(InnerKind::Item, "mode = 'final'")?;
    draft.restore(&saved)?;
    let mode = multi.parse_lang("py", "mode")?;
    println!(
        "  snapshot: mode after restore = {}",
        draft.eval(&mode)?.value.unwrap_or_default()
    );
    Ok(())
}
