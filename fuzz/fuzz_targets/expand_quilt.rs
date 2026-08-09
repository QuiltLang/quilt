//! The whole pipeline — parse, then expand — must not panic either.
//!
//! `parse_quilt` covers the surface grammar. This covers what runs *after* it:
//! `build_nodes` walking the bracket structure, the expander's `Stage`
//! arithmetic (which is where "unquote depth too high" lives), and the host
//! meta-language building output terms. Those paths index, slice and unwrap
//! their way through a tree they assume the parser vouched for, and the
//! assumption is only as good as the parser's own error handling.
//!
//! Both stages may return `Err`; only unwinding fails.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quilt::langs::omni::Omni;
use std::cell::RefCell;

thread_local! {
    /// `Omni::default()` builds every enabled language's tree-sitter parser.
    /// Rebuilding that per iteration would make the fuzzer spend its budget on
    /// parser construction instead of on inputs.
    static OMNI: RefCell<Omni> = RefCell::new(Omni::default());
}

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    OMNI.with(|omni| {
        let mut omni = omni.borrow_mut();
        // Rust is the ground language: the generated host, the one the
        // bootstrap runs through, and the one with the most expander surface.
        if let Ok(term) = omni.parse_lang("rust", src) {
            let _ = omni.expand_lang("rust", &term);
        }
    });
});
