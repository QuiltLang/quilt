//! The two round-trip laws, checked against adversarial input rather than a
//! curated corpus.
//!
//! 1. `unescape ∘ escape = id`. If it does not hold, expanding a file changes
//!    parts of it Quilt is not responsible for. #141 is the shape of the bug
//!    this catches: `←` became a glyph and the escape class did not follow.
//!
//! 2. Parsing is idempotent on what it prints. `coparse` renders a parsed tree
//!    back to source, and that source must parse again to the same text. The
//!    generated corpus in `quilt-conformance/tests/properties.rs` states the
//!    same law, but only over trees a generator builds; libFuzzer gets to
//!    approach it from the text side, where the interesting inputs are the ones
//!    that sit exactly on the boundary between an escape and a bracket.
//!
//! Note the asymmetry in (2): the *first* parse is allowed to fail — plenty of
//! byte strings are not Quilt. The *second* is not: whatever `coparse` prints
//! is by construction something the parser just produced, so refusing to read
//! it back is a defect no matter what the input was.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quilt::node::{escape, unescape, Node};

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };

    assert_eq!(
        &*unescape(&escape(src)),
        src,
        "escape/unescape are not inverse"
    );

    let Ok(nodes) = Node::parse(src) else {
        return;
    };
    let printed = Node::coparse(&nodes);
    let reparsed = Node::parse(&printed)
        .unwrap_or_else(|e| panic!("coparse produced source that will not parse: {printed:?}: {e}"));
    assert_eq!(
        &*Node::coparse(&reparsed),
        &*printed,
        "parse/coparse is not idempotent"
    );
});
