#[cfg(feature = "bootstrap")]
pub use crate::langs::bootstrap::meta::{bs_lift, bs_name, bs_reduce, BsLift};
#[cfg(feature = "rust")]
pub use crate::langs::rust::ops::{name, qlift, reduce, QLift};
pub use crate::lift::{Bash, LiftTo, Nix, Python, QLiftTo, Rust, Wgsl, Zsh};
pub use crate::manifest::{content_hash, Manifest, ManifestEntry};
pub use crate::qmatch::{mvar, qmatch, qmatch_n, sinstantiate, smatch, smvar, SMVAR};
pub use crate::qterm::{leaf, qb, quote, sym, tb, tuple, ub, unquote, Emit, QTerm};
pub use crate::sink::{
    has_marker, header_line, write_tree, Action, FsSink, NixSink, OnConflict, RelPath, TreeSink,
    WriteOptions, WriteReport, GENERATED_MARKER,
};
pub use crate::strcmd::{newline, pop, push, write, StrCmd, NL, POP};
pub use crate::term::STerm;
pub use crate::term::{cmd, hole, HOLE};
pub use crate::tree::{
    emit_tree, file, link, raw, scaffold_param, FileMeta, HeaderPolicy, Node, QTree, Segment,
    PARAM_ENV_PREFIX, TREE_OUT_ENV,
};
// The `tree!` / `dir!` builder macros are `#[macro_export]`ed at the crate root.
pub use crate::util::{arc, bx, sep, Index, Span, SEP};
pub use crate::{dir, tree};
pub use miette::{miette, Result};
pub use std::sync::Arc;
