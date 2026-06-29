#[cfg(feature = "parse")]
pub mod dir_template;
#[cfg(feature = "parse")]
pub mod grammars;
pub mod lang;
pub mod langs;
pub mod lift;
pub mod manifest;
pub mod meta;
pub mod multi;
// Dev tooling, not part of the user-facing surface: the `new-lang` scaffold
// (issue #108) that stubs out a new `langs/<lang>/` module. Always compiled (it
// only needs `tree`) so the `bin/new-lang` rust-script tool can import it.
pub mod new_lang;
#[cfg(feature = "parse")]
pub mod node;
pub mod prelude;
pub mod qmatch;
pub mod qterm;
pub mod sink;
pub mod strcmd;
pub mod template;
pub mod term;
pub mod tree;
#[cfg(feature = "parse")]
pub mod treesitter;
pub mod util;
pub mod validate;
pub mod zipper;
