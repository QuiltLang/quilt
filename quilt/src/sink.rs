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
use crate::tree::{FileMeta, HeaderPolicy, Node, QTree, Segment};
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

/// The phrase stamped into generated source files. The single-file generator
/// (`bin.rs::generate`) and the tree stamper share it so idempotent regen
/// (#94) can recognize a machine-owned file regardless of the comment leader.
pub const GENERATED_MARKER: &str = "DO NOT EDIT. GENERATED BY";

/// Build the DO-NOT-EDIT header line stamped into generated source: the comment
/// `leader`, then [`GENERATED_MARKER`], then the `quilt <args>` invocation in
/// backticks. `leader` is `"//!"` for the single-file path and `"//"`/`"#"` for
/// tree leaves.
#[must_use]
pub fn header_line(leader: &str, args: &str) -> String {
    format!("{leader} {GENERATED_MARKER} `quilt {args}`.")
}

/// What to do when a write target already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnConflict {
    /// Refuse to touch an existing file (the safe default, like `cargo new`
    /// declining a non-empty directory).
    #[default]
    Error,
    /// Overwrite existing files.
    Overwrite,
    /// Leave existing files untouched (scaffold-once / `init`).
    Skip,
    /// Rename the existing file to `<path>.orig`, then write the new one.
    Backup,
}

/// Write-time policy for [`FsSink`].
#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    /// How to handle a pre-existing target. Default [`OnConflict::Error`].
    pub on_conflict: OnConflict,
    /// Compute the plan but write nothing.
    pub dry_run: bool,
    /// When `Some(args)`, stamp the DO-NOT-EDIT header on source-file leaves
    /// whose [`FileMeta`] header policy is [`HeaderPolicy::Stamp`]. `args` is the
    /// generator invocation recorded in the header (see [`header_line`]).
    pub stamp: Option<String>,
}

/// The action [`FsSink`] took (or, under `dry_run`, *would* take) for one leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Path did not exist; written fresh.
    Create,
    /// Path existed; replaced.
    Overwrite,
    /// Path existed; left untouched (`--on-conflict skip`).
    Skip,
    /// Path existed; backed up to `<path>.orig`, then replaced.
    Backup,
    /// Path existed and `--on-conflict error` would abort. Only reported under
    /// `dry_run`; a real run errors instead.
    Conflict,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Create => "create",
            Self::Overwrite => "overwrite",
            Self::Skip => "skip",
            Self::Backup => "backup",
            Self::Conflict => "conflict",
        })
    }
}

/// The per-leaf plan accumulated by [`FsSink`], in walk order. Printed for
/// `--dry-run`.
#[derive(Debug, Clone, Default)]
pub struct WriteReport {
    pub actions: Vec<(String, Action)>,
}

impl WriteReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

impl std::fmt::Display for WriteReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (path, action) in &self.actions {
            writeln!(f, "  {action:>9}  {path}")?;
        }
        Ok(())
    }
}

/**************************************************************/

/// The filesystem backend: writes a [`QTree`] under a fixed root, applying unix
/// modes and creating symlinks, governed by a [`WriteOptions`] policy
/// (dry-run, conflict handling, header stamping). All writes are sandboxed
/// (see §10).
pub struct FsSink {
    /// Canonicalized destination root; every write stays under it.
    root: PathBuf,
    opts: WriteOptions,
    report: WriteReport,
}

impl FsSink {
    /// Root the sink at `out` with the default policy ([`OnConflict::Error`], no
    /// dry-run, no header stamping). The root is canonicalized so descendant
    /// checks are robust against symlinks in the path leading to it.
    pub fn new(out: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(out, WriteOptions::default())
    }

    /// Root the sink at `out` with an explicit [`WriteOptions`] policy. Under
    /// `dry_run` the output root is still resolved (created if needed) so
    /// existence checks are accurate, but no leaves are written.
    pub fn with_options(out: impl AsRef<Path>, opts: WriteOptions) -> Result<Self> {
        let out = out.as_ref();
        std::fs::create_dir_all(out).into_diagnostic()?;
        let root = std::fs::canonicalize(out).into_diagnostic()?;
        Ok(Self {
            root,
            opts,
            report: WriteReport::default(),
        })
    }

    /// The canonicalized output root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The plan of actions taken (or, under `dry_run`, that would be taken), in
    /// walk order.
    #[must_use]
    pub fn report(&self) -> &WriteReport {
        &self.report
    }

    /// Prepend the DO-NOT-EDIT header to `bytes` when the policy asks for it and
    /// this leaf's [`FileMeta`] says [`HeaderPolicy::Stamp`]; otherwise return
    /// the bytes unchanged.
    fn stamped(&self, bytes: &[u8], meta: &FileMeta) -> Vec<u8> {
        if let (Some(args), HeaderPolicy::Stamp(leader)) = (&self.opts.stamp, &meta.header) {
            let header = header_line(leader, args);
            let mut out = Vec::with_capacity(header.len() + 2 + bytes.len());
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(b"\n\n");
            out.extend_from_slice(bytes);
            out
        } else {
            bytes.to_vec()
        }
    }

    /// Classify the action for a leaf at absolute path `full`, given the policy,
    /// and record it in the report.
    fn classify(&mut self, path: &RelPath, full: &Path) -> Action {
        let action = if full.symlink_metadata().is_ok() {
            match self.opts.on_conflict {
                OnConflict::Error => Action::Conflict,
                OnConflict::Overwrite => Action::Overwrite,
                OnConflict::Skip => Action::Skip,
                OnConflict::Backup => Action::Backup,
            }
        } else {
            Action::Create
        };
        self.report.actions.push((path.to_string(), action));
        action
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

    /// Rename `full` to `<full>.orig` (preserving any existing extension).
    fn backup(full: &Path) -> Result<()> {
        let mut bak = full.as_os_str().to_owned();
        bak.push(".orig");
        std::fs::rename(full, PathBuf::from(bak)).into_diagnostic()
    }
}

impl TreeSink for FsSink {
    fn dir(&mut self, path: &RelPath) -> Result<()> {
        // Directories carry no conflict semantics (creation is idempotent) and
        // need not exist under dry-run, where nothing is written.
        if self.opts.dry_run {
            return Ok(());
        }
        self.ensure_dir_chain(path)?;
        Ok(())
    }

    fn file(&mut self, path: &RelPath, bytes: &[u8], meta: &FileMeta) -> Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| miette!("cannot write a file at the tree root"))?;
        let full = self.root.join(path.to_path());
        let action = self.classify(path, &full);
        if self.opts.dry_run {
            return Ok(());
        }
        match action {
            Action::Conflict => {
                return Err(miette!(
                    "{path} already exists (use --on-conflict overwrite|skip|backup)"
                ))
            }
            Action::Skip => return Ok(()),
            Action::Create | Action::Overwrite | Action::Backup => {}
        }
        let dir = self.ensure_dir_chain(&path.parent())?;
        let full = dir.join(name.as_str());
        // Never write *through* a symlink at the leaf (it would follow the link
        // outside the root).
        Self::refuse_symlink(&full)?;
        if action == Action::Backup {
            Self::backup(&full)?;
        }
        let final_bytes = self.stamped(bytes, meta);
        std::fs::write(&full, &final_bytes).into_diagnostic()?;
        Self::apply_mode(&full, meta)?;
        Ok(())
    }

    fn link(&mut self, path: &RelPath, target: &Path) -> Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| miette!("cannot create a link at the tree root"))?;
        // The target is resolved relative to the link's own directory; check
        // lexically that it can never escape the root.
        link_target_within_root(&path.parent(), target)?;
        let full = self.root.join(path.to_path());
        let action = self.classify(path, &full);
        if self.opts.dry_run {
            return Ok(());
        }
        match action {
            Action::Conflict => {
                return Err(miette!(
                    "{path} already exists (use --on-conflict overwrite|skip|backup)"
                ))
            }
            Action::Skip => return Ok(()),
            Action::Create | Action::Overwrite | Action::Backup => {}
        }
        let dir = self.ensure_dir_chain(&path.parent())?;
        let full = dir.join(name.as_str());
        // Replacing a link is safe (we recreate it via `symlink`, not by writing
        // through it): drop the old entry (Overwrite) or rename it (Backup).
        match action {
            Action::Overwrite => {
                let _ = std::fs::remove_file(&full);
            }
            Action::Backup if full.symlink_metadata().is_ok() => Self::backup(&full)?,
            _ => {}
        }
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

    // --- #93: write policy -------------------------------------------------

    fn opts(on_conflict: OnConflict) -> WriteOptions {
        WriteOptions {
            on_conflict,
            ..WriteOptions::default()
        }
    }

    #[test]
    fn dry_run_writes_nothing_but_plans() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("proj");
        let t = tree! {
            "a.txt" => raw(b"a".to_vec()),
            "src" => dir! { "b.txt" => raw(b"b".to_vec()) },
        };
        let mut sink = FsSink::with_options(
            &out,
            WriteOptions {
                dry_run: true,
                ..WriteOptions::default()
            },
        )
        .unwrap();
        write_tree(&mut sink, &t).unwrap();
        assert!(!out.join("a.txt").exists());
        assert!(!out.join("src").exists());
        let actions = &sink.report().actions;
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().all(|(_, a)| *a == Action::Create));
        assert_eq!(actions[0], ("a.txt".to_string(), Action::Create));
    }

    #[test]
    fn on_conflict_error_aborts_on_existing() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        std::fs::write(out.join("a.txt"), "old").unwrap();
        let t = tree! { "a.txt" => raw(b"new".to_vec()) };
        let mut sink = FsSink::with_options(out, opts(OnConflict::Error)).unwrap();
        assert!(write_tree(&mut sink, &t).is_err());
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "old");
    }

    #[test]
    fn on_conflict_skip_leaves_existing() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        std::fs::write(out.join("a.txt"), "old").unwrap();
        let t = tree! { "a.txt" => raw(b"new".to_vec()), "b.txt" => raw(b"fresh".to_vec()) };
        let mut sink = FsSink::with_options(out, opts(OnConflict::Skip)).unwrap();
        write_tree(&mut sink, &t).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(out.join("b.txt")).unwrap(), "fresh");
        assert_eq!(sink.report().actions[0].1, Action::Skip);
        assert_eq!(sink.report().actions[1].1, Action::Create);
    }

    #[test]
    fn on_conflict_overwrite_replaces() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        std::fs::write(out.join("a.txt"), "old").unwrap();
        let t = tree! { "a.txt" => raw(b"new".to_vec()) };
        let mut sink = FsSink::with_options(out, opts(OnConflict::Overwrite)).unwrap();
        write_tree(&mut sink, &t).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "new");
        assert_eq!(sink.report().actions[0].1, Action::Overwrite);
    }

    #[test]
    fn on_conflict_backup_renames_then_writes() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        std::fs::write(out.join("a.txt"), "old").unwrap();
        let t = tree! { "a.txt" => raw(b"new".to_vec()) };
        let mut sink = FsSink::with_options(out, opts(OnConflict::Backup)).unwrap();
        write_tree(&mut sink, &t).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "new");
        assert_eq!(
            std::fs::read_to_string(out.join("a.txt.orig")).unwrap(),
            "old"
        );
        assert_eq!(sink.report().actions[0].1, Action::Backup);
    }

    #[test]
    fn header_stamped_on_source_leaves_only() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        let main_rs: Arc<_> = leaf("source_file", "fn main() {}");
        let t = tree! {
            "main.rs" => file(main_rs),          // source leaf -> Stamp
            "data.bin" => raw(b"binary".to_vec()), // asset -> Skip
        };
        let mut sink = FsSink::with_options(
            out,
            WriteOptions {
                stamp: Some("scaffold x.tree.rs.quilt".to_string()),
                ..WriteOptions::default()
            },
        )
        .unwrap();
        write_tree(&mut sink, &t).unwrap();

        let main = std::fs::read_to_string(out.join("main.rs")).unwrap();
        assert!(main.starts_with("// "), "leader on the source leaf");
        assert!(main.contains(GENERATED_MARKER));
        assert!(main.trim_end().ends_with("fn main() {}"));

        // Raw asset is untouched (Skip policy on its header).
        assert_eq!(
            std::fs::read_to_string(out.join("data.bin")).unwrap(),
            "binary"
        );
    }

    #[test]
    fn no_stamp_when_option_unset() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        let main_rs: Arc<_> = leaf("source_file", "fn main() {}");
        let t = tree! { "main.rs" => file(main_rs) };
        // Default options: stamp = None.
        let mut sink = FsSink::new(out).unwrap();
        write_tree(&mut sink, &t).unwrap();
        let main = std::fs::read_to_string(out.join("main.rs")).unwrap();
        assert!(!main.contains(GENERATED_MARKER));
        assert_eq!(main, "fn main() {}");
    }

    #[test]
    fn header_line_format() {
        assert_eq!(
            header_line("//!", "expand foo.rs.quilt"),
            "//! DO NOT EDIT. GENERATED BY `quilt expand foo.rs.quilt`."
        );
        assert!(header_line("#", "scaffold").contains(GENERATED_MARKER));
    }
}
