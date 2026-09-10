pub mod glyphs;
#[cfg(feature = "parse")]
pub mod grammars;
pub mod lang;
pub mod langs;
pub mod lift;
// Machines (stateful evaluators; docs/design/machines.md) are, like `lift`,
// part of the runtime-only build: no tree-sitter dependency.
pub mod machine;
pub mod meta;
pub mod multi;
// Notebooks: an HTML page whose quoted cells run on the machines of their
// languages, with the page itself as the HTML machine. Needs the parsers
// (cells are re-parsed as programs) and HTML (the page).
#[cfg(all(feature = "parse", feature = "html"))]
pub mod notebook;
// The Quilt surface parser is hand-written (issue #254), so it needs no
// tree-sitter and is available in the runtime-only build too.
pub mod node;
pub mod prelude;
pub mod qmatch;
pub mod qsnap;
pub mod qterm;
pub mod strcmd;
pub mod term;
#[cfg(feature = "parse")]
pub mod treesitter;
pub mod util;
pub mod validate;
pub mod zipper;
