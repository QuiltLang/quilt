//! Arbitrary input is a diagnostic, never a panic.
//!
//! This is the contract `quilt check` rests on: its whole job is to report
//! malformed input, so a panic there is not a wrong answer, it is no answer —
//! the process aborts mid-report and the contributor gets a backtrace instead
//! of a caret under the offending bracket. `Node::parse` was made to return
//! `Err` rather than hit an `unreachable!` for exactly this reason; nothing
//! held it to that.
//!
//! `Err` is a pass. Only unwinding fails.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quilt::node::Node;

fuzz_target!(|data: &[u8]| {
    // Quilt sources are text. Non-UTF-8 is the caller's problem (the CLI reads
    // with `read_to_string`, which rejects it before we ever get here), so
    // spending fuzz budget on invalid byte sequences would test the standard
    // library rather than Quilt.
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let _ = Node::parse(src);
});
