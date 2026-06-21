//! Materializing a [`QTree`] to a destination.
//!
//! A [`TreeSink`] is the one abstraction every backend implements; the
//! [`write_tree`] walker turns each `QTree` leaf into bytes **once** (a `File`
//! leaf via [`STerm::coparse`](crate::term::STerm::coparse), a `Raw` leaf
//! verbatim) and feeds every sink the same way. [`FsSink`] is the primary
//! backend: it writes under a fixed root and is the trusted place where the
//! **path-traversal sandbox** lives — segments are validated by the tree, and
//! the sink additionally refuses to traverse or overwrite symlinks and checks
//! every created directory descends from the root.
//!
//! This module is always compiled (no tree-sitter dependency), like
//! [`tree`](crate::tree) and [`lift`](crate::lift).
//!
//! See `docs/design/directory-scaffolding.md` §6.5, §10.

// `coparse` is a provided method on `STerm`; bring the trait into scope.
use crate::term::STerm;
use crate::tree::{FileMeta, Node, QTree, Segment};
use miette::{miette, IntoDiagnostic, Result};
use std::path::{Path, PathBuf};

/**************************************************************/

/// A relative path built from validated [`Segment`]s — the only kind of path a
/// sink ever receives. The walker constructs it as it descends, so a sink can
/// trust that no component is `.`/`..`, absolute, or contains a separator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelPath(Vec<Segment>);

impl RelPath {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Extend the path by one validated segment (returns a new path).
    #[must_use]
    pub fn join(&self, seg: &Segment) -> Self {
        let mut segs = self.0.clone();
        segs.push(seg.clone());
        Self(segs)
    }

    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The path's final component, if any.
    #[must_use]
    pub fn file_name(&self) -> Option<&Segment> {
        self.0.last()
    }

    /// The path with its final component removed (the tree root for a
    /// single-component path).
    #[must_use]
    pub fn parent(&self) -> Self {
        let n = self.0.len().saturating_sub(1);
        Self(self.0[..n].to_vec())
    }

    /// The corresponding [`PathBuf`] (each segment is one component).
    #[must_use]
    pub fn to_path(&self) -> PathBuf {
        self.0.iter().map(Segment::as_str).collect()
    }
}

impl std::fmt::Display for RelPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            f.write_str(seg.as_str())?;
        }
        Ok(())
    }
}

/**************************************************************/

/// A backend that materializes a [`QTree`]. The walker calls `dir`/`file`/`link`
/// in insertion order as it descends, then `finish`.
pub trait TreeSink {
    /// Create a (possibly empty) directory at `path`.
    fn dir(&mut self, path: &RelPath) -> Result<()>;
    /// Write a file leaf (`bytes` already serialized) at `path`.
    fn file(&mut self, path: &RelPath, bytes: &[u8], meta: &FileMeta) -> Result<()>;
    /// Create a symlink at `path` pointing at `target` (relative to the link's
    /// own directory).
    fn link(&mut self, path: &RelPath, target: &Path) -> Result<()>;
    /// Flush any deferred work (e.g. a manifest). Called once after the walk.
    fn finish(self) -> Result<()>
    where
        Self: Sized;
}

/// Walk `tree` depth-first in insertion order, driving `sink`. Each `File` leaf
/// is serialized **once** via `coparse()`; `Raw` leaves pass bytes through.
/// (Header stamping is the write-policy layer's job — issue #93.)
pub fn write_tree<S: TreeSink>(sink: &mut S, tree: &QTree) -> Result<()> {
    walk(sink, &RelPath::new(), tree)
}

fn walk<S: TreeSink>(sink: &mut S, base: &RelPath, tree: &QTree) -> Result<()> {
    for (seg, node) in tree.entries() {
        let path = base.join(seg);
        match node {
            Node::Dir(sub) => {
                sink.dir(&path)?;
                walk(sink, &path, sub)?;
            }
            Node::File { content, meta } => {
                let bytes = content.coparse().into_bytes();
                sink.file(&path, &bytes, meta)?;
            }
            Node::Raw { bytes, meta } => {
                sink.file(&path, bytes, meta)?;
            }
            Node::Link { target } => {
                sink.link(&path, target)?;
            }
        }
    }
    Ok(())
}

/**************************************************************/

/// The filesystem backend: writes a [`QTree`] under a fixed root, applying unix
/// modes and creating symlinks. All writes are sandboxed (see §10).
pub struct FsSink {
    /// Canonicalized destination root; every write stays under it.
    root: PathBuf,
}

impl FsSink {
    /// Root the sink at `out`, creating it if needed. The root is canonicalized
    /// so descendant checks are robust against symlinks in the path leading to
    /// it.
    pub fn new(out: impl AsRef<Path>) -> Result<Self> {
        let out = out.as_ref();
        std::fs::create_dir_all(out).into_diagnostic()?;
        let root = std::fs::canonicalize(out).into_diagnostic()?;
        Ok(Self { root })
    }

    /// The canonicalized output root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create the directory chain `rel` *one level at a time*, never following a
    /// symlink: each existing level must be a real directory (a symlink there is
    /// rejected, since it could redirect the write outside the root). Returns the
    /// absolute path of the deepest directory.
    fn ensure_dir_chain(&self, rel: &RelPath) -> Result<PathBuf> {
        let mut cur = self.root.clone();
        for seg in rel.segments() {
            cur.push(seg.as_str());
            match std::fs::symlink_metadata(&cur) {
                Ok(md) => {
                    if md.file_type().is_symlink() {
                        return Err(miette!(
                            "refusing to traverse symlink {} inside the output root",
                            cur.display()
                        ));
                    }
                    if !md.is_dir() {
                        return Err(miette!("{} exists and is not a directory", cur.display()));
                    }
                }
                Err(_) => std::fs::create_dir(&cur).into_diagnostic()?,
            }
        }
        Ok(cur)
    }

    /// Reject overwriting through an existing symlink at a leaf path.
    fn refuse_symlink(full: &Path) -> Result<()> {
        if let Ok(md) = std::fs::symlink_metadata(full) {
            if md.file_type().is_symlink() {
                return Err(miette!("refusing to overwrite symlink {}", full.display()));
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    fn apply_mode(full: &Path, meta: &FileMeta) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = meta.mode {
            std::fs::set_permissions(full, std::fs::Permissions::from_mode(mode))
                .into_diagnostic()?;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn apply_mode(_full: &Path, _meta: &FileMeta) -> Result<()> {
        Ok(())
    }
}

impl TreeSink for FsSink {
    fn dir(&mut self, path: &RelPath) -> Result<()> {
        self.ensure_dir_chain(path)?;
        Ok(())
    }

    fn file(&mut self, path: &RelPath, bytes: &[u8], meta: &FileMeta) -> Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| miette!("cannot write a file at the tree root"))?;
        let dir = self.ensure_dir_chain(&path.parent())?;
        let full = dir.join(name.as_str());
        Self::refuse_symlink(&full)?;
        std::fs::write(&full, bytes).into_diagnostic()?;
        Self::apply_mode(&full, meta)?;
        Ok(())
    }

    fn link(&mut self, path: &RelPath, target: &Path) -> Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| miette!("cannot create a link at the tree root"))?;
        // The target is resolved relative to the link's own directory; check
        // lexically that it can never escape the root, then create it.
        link_target_within_root(&path.parent(), target)?;
        let dir = self.ensure_dir_chain(&path.parent())?;
        let full = dir.join(name.as_str());
        Self::refuse_symlink(&full)?;
        symlink(target, &full)
    }

    fn finish(self) -> Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
fn symlink(target: &Path, full: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, full).into_diagnostic()
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _full: &Path) -> Result<()> {
    Err(miette!("symlinks are only supported on unix targets"))
}

/// Lexically verify that a relative symlink `target`, resolved against the link's
/// directory `link_dir` (relative to the output root), stays within the root —
/// i.e. its `..` components never pop above the root. Absolute targets are
/// rejected. Purely lexical (the target may legitimately be dangling), so it is
/// safe to run before touching the filesystem.
fn link_target_within_root(link_dir: &RelPath, target: &Path) -> Result<()> {
    use std::path::Component;
    if target.is_absolute() {
        return Err(miette!(
            "symlink target must be relative, got {}",
            target.display()
        ));
    }
    let mut depth: usize = link_dir.segments().len();
    for comp in target.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    miette!(
                        "symlink target {} escapes the output root",
                        target.display()
                    )
                })?;
            }
            Component::Normal(_) => depth += 1,
            Component::RootDir | Component::Prefix(_) => {
                return Err(miette!("symlink target must be relative"));
            }
        }
    }
    Ok(())
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qterm::leaf;
    use crate::tree::{file, link as link_node, raw};
    use crate::{dir, tree};
    use std::sync::Arc;

    fn seg(s: &str) -> Segment {
        Segment::new(s).unwrap()
    }

    fn rel(parts: &[&str]) -> RelPath {
        let mut p = RelPath::new();
        for s in parts {
            p = p.join(&seg(s));
        }
        p
    }

    #[test]
    fn writes_files_dirs_and_serializes_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("proj");
        let main_rs: Arc<_> = leaf("source_file", "fn main() {}");
        let t = tree! {
            "Cargo.toml" => raw(b"[package]\n".to_vec()),
            "src" => dir! { "main.rs" => file(main_rs) },
            "tests" => dir! {},   // empty dir kept
        };
        let mut sink = FsSink::new(&out).unwrap();
        write_tree(&mut sink, &t).unwrap();
        sink.finish().unwrap();

        assert_eq!(
            std::fs::read_to_string(out.join("Cargo.toml")).unwrap(),
            "[package]\n"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
        assert!(out.join("tests").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn applies_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let t = tree! { "run" => raw(b"#!/bin/sh\n".to_vec()).mode(0o755) };
        let mut sink = FsSink::new(out).unwrap();
        write_tree(&mut sink, &t).unwrap();
        let mode = std::fs::metadata(out.join("run"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn creates_symlink_within_tree() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let t = tree! {
            "versions" => dir! { "v2.txt" => raw(b"two".to_vec()) },
            "latest.txt" => link_node("versions/v2.txt"),
        };
        let mut sink = FsSink::new(out).unwrap();
        write_tree(&mut sink, &t).unwrap();
        let link = out.join("latest.txt");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "two");
    }

    #[test]
    fn rejects_absolute_symlink_target() {
        assert!(link_target_within_root(&rel(&["a"]), Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn rejects_symlink_escaping_root() {
        // From dir "a", "../../etc" escapes (pops a, then root).
        assert!(link_target_within_root(&rel(&["a"]), Path::new("../../etc")).is_err());
        // From dir "a/b", "../c" stays inside (-> a/c).
        assert!(link_target_within_root(&rel(&["a", "b"]), Path::new("../c")).is_ok());
        // From the root, any "../" escapes.
        assert!(link_target_within_root(&RelPath::new(), Path::new("../x")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn link_sink_rejects_escaping_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = FsSink::new(dir.path()).unwrap();
        let err = sink.link(&rel(&["escape"]), Path::new("../../etc/passwd"));
        assert!(err.is_err());
        assert!(!dir.path().join("escape").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_write_through_preexisting_symlink_dir() {
        // out/evil -> outside; writing out/evil/x must not land outside.
        let base = tempfile::tempdir().unwrap();
        let out = base.path().join("out");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, out.join("evil")).unwrap();

        let mut sink = FsSink::new(&out).unwrap();
        let err = sink.file(&rel(&["evil", "x.txt"]), b"pwned", &FileMeta::asset());
        assert!(err.is_err(), "writing through a symlinked dir should fail");
        assert!(
            !outside.join("x.txt").exists(),
            "must not write outside root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_overwrite_through_preexisting_symlink_file() {
        let base = tempfile::tempdir().unwrap();
        let out = base.path().join("out");
        let outside = base.path().join("target.txt");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(&outside, "original").unwrap();
        std::os::unix::fs::symlink(&outside, out.join("evil.txt")).unwrap();

        let mut sink = FsSink::new(&out).unwrap();
        let err = sink.file(&rel(&["evil.txt"]), b"pwned", &FileMeta::asset());
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "original");
    }

    #[test]
    fn relpath_parent_and_display() {
        let p = rel(&["a", "b", "c.txt"]);
        assert_eq!(p.to_string(), "a/b/c.txt");
        assert_eq!(p.parent().to_string(), "a/b");
        assert_eq!(p.file_name().unwrap().as_str(), "c.txt");
        assert_eq!(RelPath::new().parent(), RelPath::new());
    }
}
