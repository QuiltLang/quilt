//! `Node::parse` before and after issue #254: tree-sitter versus the
//! hand-written scanner, on the same input, producing the same terms.
//!
//! Deliberately not criterion. The question this answers is "which order of
//! magnitude", the two implementations are both in-tree
//! ([`Node::parse_ts`] is kept as the differential oracle), and a dev-dependency
//! that pulls forty crates to tell us that is a bad trade. Run it with:
//!
//! ```sh
//! cargo bench -p quiltlang --bench parse
//! ```
//!
//! Both halves are checked to produce identical terms before either is timed,
//! so a number here is never bought by parsing less.

use quilt::node::Node;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn main() {
    let corpus = corpus();
    let total: usize = corpus.iter().map(|(_, s)| s.len()).sum();
    println!("corpus: {} inputs, {total} bytes\n", corpus.len());

    // A number bought by parsing less is not a number. Check first.
    for (name, src) in &corpus {
        assert_eq!(
            Node::parse(src).map_err(|e| e.to_string()),
            Node::parse_ts(src).map_err(|e| e.to_string()),
            "{name}: the two parsers disagree — benchmark is meaningless"
        );
    }

    let hand = time(&corpus, Node::parse);
    let ts = time(&corpus, Node::parse_ts);

    println!("{:<14}{:>12}{:>14}{:>12}", "", "total", "per KiB", "vs");
    row("tree-sitter", ts, total, ts);
    row("hand-written", hand, total, ts);
}

// A corpus is never big enough for `usize -> f64` to lose anything that
// matters to a µs-per-KiB figure.
#[allow(clippy::cast_precision_loss)]
fn row(name: &str, d: Duration, bytes: usize, base: Duration) {
    let per_kib = d.as_secs_f64() / (bytes as f64 / 1024.0) * 1e6;
    println!(
        "{name:<14}{:>10.2?}{per_kib:>12.2} µs{:>11.2}×",
        d,
        base.as_secs_f64() / d.as_secs_f64()
    );
}

/// Time `parse` over the whole corpus, best-of-N so a stray scheduling hiccup
/// does not become the headline.
fn time<T>(corpus: &[(String, String)], parse: fn(&str) -> T) -> Duration {
    let run = || {
        let start = Instant::now();
        for (_, src) in corpus {
            black_box(parse(black_box(src)));
        }
        start.elapsed()
    };
    for _ in 0..3 {
        run(); // warm the caches and let the CPU clock up
    }
    (0..10).map(|_| run()).min().expect("10 runs")
}

/// Every `.quilt` file in the repo, plus one synthetic file big enough that
/// per-call overhead stops dominating.
fn corpus() -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(root) = repo_root() {
        let mut files = Vec::new();
        collect(&root, &mut files);
        files.sort();
        for path in files {
            if let Ok(src) = std::fs::read_to_string(&path) {
                out.push((path.display().to_string(), src));
            }
        }
    }
    out.push(("<synthetic>".into(), synthetic()));
    out
}

/// ~100 KiB with something of every node kind in it, so no single construct
/// dominates the profile.
fn synthetic() -> String {
    let unit = concat!(
        "fn main() {\n",
        "    // an ordinary line comment\n",
        "    ⟨//⟩ a quilt comment, which vanishes\n",
        "    let a = rs↖let x = ↙name↘ + ↑1↑;↗;\n",
        "    let b = py↖def f(): return ↙body↘↗;\n",
        "    let c = ↖ wgsl↖ ↙expr↘ ↗ ↗;\n",
        "    /* a block comment */\n",
        "    let d: ⟨T⟩ = ⟨N⟩;\n",
        "    let e = ← items;\n",
        "    let f = \\↖ escaped \\↗;\n",
        "}\n",
    );
    unit.repeat(100_000 / unit.len() + 1)
}

fn repo_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .to_path_buf();
    root.join("examples").is_dir().then_some(root)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !matches!(&*name, "target" | "node_modules" | ".git") {
                collect(&path, out);
            }
        } else if name.ends_with(".quilt") {
            out.push(path);
        }
    }
}
