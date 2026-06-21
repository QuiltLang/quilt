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

use crate::manifest::{content_hash, Manifest, ManifestEntry};
// `coparse` is a provided method on `STerm`; bring the trait into scope.
use crate::term::STerm;
use crate::tree::{FileMeta, HeaderPolicy, Node, QTree, Segment};
use miette::{miette, IntoDiagnostic, Result};
use std::fmt::Write as _;
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

    /// Parse a `/`-joined string into a `RelPath`, validating every component
    /// via [`Segment::new`]. Used to safely re-derive a path from an untrusted
    /// manifest key before touching the filesystem.
    pub fn parse(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Ok(Self::new());
        }
        let segs = s.split('/').map(Segment::new).collect::<Result<Vec<_>>>()?;
        Ok(Self(segs))
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
    /// Header guard for idempotent regen (issue #94): under
    /// [`OnConflict::Overwrite`], refuse to overwrite a file the user has taken
    /// ownership of (marker removed) or hand-edited (manifest hash mismatch) —
    /// such a file is skipped instead. No effect under other conflict policies.
    pub guard: bool,
    /// After writing, delete paths that the previous manifest recorded as
    /// machine-owned but that are absent from the new tree.
    pub prune: bool,
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
    /// Manifest from the previous generation (empty on a first run); drives the
    /// header guard and `--prune`.
    old: Manifest,
    /// Manifest accumulated during this run; written by `finish`.
    next: Manifest,
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
        let old = Manifest::load(&root);
        Ok(Self {
            root,
            opts,
            report: WriteReport::default(),
            old,
            next: Manifest::default(),
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

    /// Classify the action for a leaf at absolute path `full`, given the policy.
    /// `guarded` enables the idempotent-regen header guard (file leaves only):
    /// under [`OnConflict::Overwrite`] a user-owned or hand-edited file is
    /// demoted to [`Action::Skip`] instead of being clobbered.
    fn plan_action(&self, path: &RelPath, full: &Path, guarded: bool) -> Action {
        if full.symlink_metadata().is_err() {
            return Action::Create;
        }
        match self.opts.on_conflict {
            OnConflict::Error => Action::Conflict,
            OnConflict::Skip => Action::Skip,
            OnConflict::Backup => Action::Backup,
            OnConflict::Overwrite => {
                if guarded && self.opts.guard && !self.may_overwrite(&path.to_string(), full) {
                    Action::Skip
                } else {
                    Action::Overwrite
                }
            }
        }
    }

    /// May the file at `full` be overwritten under the header guard? Yes when the
    /// previous manifest recorded it as machine-owned *and* it is unchanged on
    /// disk (hash matches); for a path unknown to the manifest, only when the
    /// on-disk file still carries the DO-NOT-EDIT marker. A user-owned or
    /// hand-edited file returns false.
    fn may_overwrite(&self, path_str: &str, full: &Path) -> bool {
        let on_disk = std::fs::read(full).unwrap_or_default();
        match self.old.get(path_str) {
            Some(e) if e.managed => content_hash(&on_disk) == e.hash,
            Some(_) => false,
            None => has_marker(&on_disk),
        }
    }

    /// Whether this leaf is machine-owned: a source-file header policy *and* an
    /// active stamp option (so a recognizable marker is actually written).
    fn is_managed(&self, meta: &FileMeta) -> bool {
        self.opts.stamp.is_some() && matches!(meta.header, HeaderPolicy::Stamp(_))
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

    /// Delete paths the previous manifest recorded as machine-owned that are
    /// absent from the new tree, then tidy up directories left empty. Only
    /// managed (stamped) regular files are removed — user assets and links are
    /// never auto-deleted. Each manifest key is re-validated as a `RelPath`
    /// before use, so a tampered manifest can't reach outside the root.
    fn prune(&self) -> Result<()> {
        for (path_str, entry) in &self.old.entries {
            if !entry.managed || self.next.get(path_str).is_some() {
                continue;
            }
            let Ok(rel) = RelPath::parse(path_str) else {
                continue;
            };
            if rel.is_empty() {
                continue;
            }
            let full = self.root.join(rel.to_path());
            // Only remove a plain file (never a symlink or directory).
            if let Ok(md) = std::fs::symlink_metadata(&full) {
                if md.file_type().is_file() {
                    std::fs::remove_file(&full).into_diagnostic()?;
                    if let Some(parent) = full.parent() {
                        self.remove_empty_parents(parent);
                    }
                }
            }
        }
        Ok(())
    }

    /// Remove now-empty directories from `dir` upward, stopping at the root.
    fn remove_empty_parents(&self, dir: &Path) {
        let mut cur = dir.to_path_buf();
        while cur.starts_with(&self.root) && cur != self.root {
            let empty = std::fs::read_dir(&cur).is_ok_and(|mut d| d.next().is_none());
            if !empty || std::fs::remove_dir(&cur).is_err() {
                break;
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent.to_path_buf();
        }
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
        let action = self.plan_action(path, &full, true);
        self.report.actions.push((path.to_string(), action));

        let final_bytes = self.stamped(bytes, meta);
        let managed = self.is_managed(meta);
        // Record this path as present in the new tree so `--prune` keeps it. A
        // skipped (user-owned) file is recorded by its on-disk content and as
        // not machine-owned, so a later prune/guard leaves it alone.
        let entry = match action {
            Action::Skip | Action::Conflict => ManifestEntry {
                hash: content_hash(&std::fs::read(&full).unwrap_or_default()),
                managed: false,
            },
            _ => ManifestEntry {
                hash: content_hash(&final_bytes),
                managed,
            },
        };
        self.next.insert(path.to_string(), entry);

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
        // Links carry no DO-NOT-EDIT marker, so the header guard never applies.
        let action = self.plan_action(path, &full, false);
        self.report.actions.push((path.to_string(), action));
        // Record the link as present in the new tree but never machine-owned, so
        // `--prune` won't delete it and the guard won't touch it.
        self.next.insert(
            path.to_string(),
            ManifestEntry {
                hash: content_hash(target.to_string_lossy().as_bytes()),
                managed: false,
            },
        );

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
        if self.opts.dry_run {
            return Ok(());
        }
        if self.opts.prune {
            self.prune()?;
        }
        // Persist the new manifest so the next run can guard/prune against it.
        self.next.save(&self.root)
    }
}

/// Whether `bytes` carries the DO-NOT-EDIT marker near its start (a
/// machine-owned file). Only the head is scanned so a large asset that happens
/// to contain the phrase deep inside isn't misclassified.
#[must_use]
pub fn has_marker(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    String::from_utf8_lossy(head).contains(GENERATED_MARKER)
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

/// A [`TreeSink`] that lowers a [`QTree`] to a **Nix expression** instead of
/// writing files: a `pkgs.linkFarm` that materializes the tree as a derivation
/// in the Nix store, so `nix build` on the emitted `.nix` produces a
/// reproducible directory. It composes with the string-based Nix meta — one
/// tree, many lowerings (issue #98).
///
/// File content becomes a `pkgs.writeText` (or `pkgs.writeTextFile { …;
/// executable = true; }` when the leaf's mode carries the execute bit), and each
/// `/`-joined path is a `linkFarm` entry name (linkFarm creates the parent
/// directories). There is no filesystem and no traversal risk — the output is
/// just text — so this sink is wasm-friendly too.
///
/// Because the Nix store models *content*, two limitations apply: an empty
/// directory is dropped (linkFarm has no entry for one), and an in-tree symlink
/// ([`Node::Link`](crate::tree::Node::Link)) is unsupported (it errors). File
/// content must be valid UTF-8.
///
/// The walker fills the sink; the result is taken with [`NixSink::into_source`]
/// (`finish` is a no-op):
///
/// ```
/// use quilt::prelude::*;
///
/// let t = tree! { "hello.txt" => raw(b"hi\n".to_vec()) };
/// let mut sink = NixSink::new("demo");
/// write_tree(&mut sink, &t).unwrap();
/// let nix = sink.into_source();
/// assert!(nix.contains("pkgs.linkFarm \"demo\""));
/// assert!(nix.contains("name = \"hello.txt\""));
/// ```
///
/// See `docs/design/directory-scaffolding.md` §6.5.
pub struct NixSink {
    /// The derivation name (`pkgs.linkFarm "<name>" …`).
    name: String,
    /// One linkFarm entry per file leaf, in walk order.
    entries: Vec<NixEntry>,
}

/// One materialized file: its `/`-joined tree path, UTF-8 content, and whether
/// it should be executable.
struct NixEntry {
    path: String,
    text: String,
    executable: bool,
}

impl NixSink {
    /// A sink that emits a `linkFarm` derivation named `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    /// The Nix expression for the accumulated tree: a function of `pkgs`
    /// (defaulting to `<nixpkgs>`) returning the `linkFarm` derivation.
    #[must_use]
    pub fn into_source(self) -> String {
        let mut s = String::new();
        s.push_str("# DO NOT EDIT. Generated by quilt's NixSink.\n");
        s.push_str("{ pkgs ? import <nixpkgs> { } }:\n");
        s.push_str("pkgs.linkFarm ");
        s.push_str(&nix_string(&self.name));
        s.push_str(" [\n");
        for entry in &self.entries {
            let store = if entry.executable {
                format!(
                    "pkgs.writeTextFile {{ name = {}; text = {}; executable = true; }}",
                    nix_string(&store_name(&entry.path)),
                    nix_string(&entry.text),
                )
            } else {
                format!(
                    "pkgs.writeText {} {}",
                    nix_string(&store_name(&entry.path)),
                    nix_string(&entry.text),
                )
            };
            // `s` is a String, whose `fmt::Write` is infallible.
            let _ = writeln!(
                s,
                "  {{ name = {}; path = {store}; }}",
                nix_string(&entry.path)
            );
        }
        s.push_str("]\n");
        s
    }
}

impl TreeSink for NixSink {
    /// Directories are implied by file paths; an empty one has no `linkFarm`
    /// entry and is dropped.
    fn dir(&mut self, _path: &RelPath) -> Result<()> {
        Ok(())
    }

    fn file(&mut self, path: &RelPath, bytes: &[u8], meta: &FileMeta) -> Result<()> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| {
                miette!("NixSink: {path} is not valid UTF-8 (the Nix store sink lowers text only)")
            })?
            .to_owned();
        // A file is executable iff its mode carries any execute bit — all the
        // Nix store can express about a single file's permissions.
        let executable = meta.mode.is_some_and(|m| m & 0o111 != 0);
        self.entries.push(NixEntry {
            path: path.to_string(),
            text,
            executable,
        });
        Ok(())
    }

    fn link(&mut self, path: &RelPath, _target: &Path) -> Result<()> {
        Err(miette!(
            "NixSink: in-tree symlink at {path} is unsupported (the Nix store sink lowers files only)"
        ))
    }

    /// The emitted source is taken with [`NixSink::into_source`]; `finish` does
    /// nothing.
    fn finish(self) -> Result<()> {
        Ok(())
    }
}

/// A Nix double-quoted string literal for `s`, escaping `\`, `"`, and the
/// antiquotation opener `${` (to `\${`) so interpolation stays literal. Other
/// characters — including newlines and tabs — pass through, which a Nix `"…"`
/// string takes verbatim. Mirrors `langs::nix::ops`'s escaping (duplicated here
/// because this sink is always compiled, with no parser dependency).
fn nix_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Nix store-safe name derived from a path's final component: keep
/// `[A-Za-z0-9._+-]`, map anything else to `_`, never start with `.`, and fall
/// back to `leaf` when nothing usable is left. (`writeText`'s name only labels
/// the store path; collisions are harmless since the hash is content-derived.)
fn store_name(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    let mut out = String::with_capacity(base.len());
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.starts_with('.') {
        out.insert(0, 'q');
    }
    out
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

    // --- #94: idempotent regen (manifest, guard, prune) --------------------

    use crate::manifest::Manifest;

    /// Regen policy: stamp markers, overwrite machine-owned files, header guard.
    fn regen() -> WriteOptions {
        WriteOptions {
            on_conflict: OnConflict::Overwrite,
            guard: true,
            stamp: Some("scaffold demo".to_string()),
            ..WriteOptions::default()
        }
    }

    fn gen(out: &Path, t: &QTree, opts: WriteOptions) {
        let mut sink = FsSink::with_options(out, opts).unwrap();
        write_tree(&mut sink, t).unwrap();
        sink.finish().unwrap();
    }

    fn src_leaf(code: &str) -> Node {
        file(leaf("source_file", code))
    }

    #[test]
    fn manifest_written_and_records_managed_flag() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        let t = tree! {
            "main.rs" => src_leaf("fn main() {}"),
            "data.bin" => raw(b"asset".to_vec()),
        };
        let mut sink = FsSink::with_options(
            out,
            WriteOptions {
                stamp: Some("scaffold".to_string()),
                ..WriteOptions::default()
            },
        )
        .unwrap();
        let root = sink.root().to_path_buf();
        write_tree(&mut sink, &t).unwrap();
        sink.finish().unwrap();

        let m = Manifest::load(&root);
        assert!(m.get("main.rs").unwrap().managed, "source leaf is managed");
        assert!(
            !m.get("data.bin").unwrap().managed,
            "raw asset is not managed"
        );
    }

    #[test]
    fn guard_skips_user_edited_file() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        gen(out, &tree! { "main.rs" => src_leaf("v1") }, regen());
        // User takes over the file.
        std::fs::write(out.join("main.rs"), "hand written\n").unwrap();
        // Regen with new content — the guard must not clobber the edit.
        let mut sink = FsSink::with_options(out, regen()).unwrap();
        write_tree(&mut sink, &tree! { "main.rs" => src_leaf("v2") }).unwrap();
        assert_eq!(sink.report().actions[0].1, Action::Skip);
        sink.finish().unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join("main.rs")).unwrap(),
            "hand written\n"
        );
    }

    #[test]
    fn guard_overwrites_unedited_managed_file() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        gen(out, &tree! { "main.rs" => src_leaf("v1") }, regen());
        let mut sink = FsSink::with_options(out, regen()).unwrap();
        write_tree(&mut sink, &tree! { "main.rs" => src_leaf("v2") }).unwrap();
        assert_eq!(sink.report().actions[0].1, Action::Overwrite);
        sink.finish().unwrap();
        let main = std::fs::read_to_string(out.join("main.rs")).unwrap();
        assert!(main.contains("v2"));
        assert!(main.contains(GENERATED_MARKER));
    }

    #[test]
    fn guard_uses_marker_when_manifest_absent() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        // A pre-existing machine-owned file (marker present), no manifest.
        std::fs::write(
            out.join("owned.rs"),
            format!("{}\n\nfn old() {{}}", header_line("//", "x")),
        )
        .unwrap();
        // And a user-owned file (no marker).
        std::fs::write(out.join("user.rs"), "fn user() {}").unwrap();

        let mut sink = FsSink::with_options(out, regen()).unwrap();
        write_tree(
            &mut sink,
            &tree! { "owned.rs" => src_leaf("new"), "user.rs" => src_leaf("new") },
        )
        .unwrap();
        assert_eq!(sink.report().actions[0].1, Action::Overwrite); // marker -> ok
        assert_eq!(sink.report().actions[1].1, Action::Skip); // no marker -> skip
        sink.finish().unwrap();
        assert!(std::fs::read_to_string(out.join("owned.rs"))
            .unwrap()
            .contains("new"));
        assert_eq!(
            std::fs::read_to_string(out.join("user.rs")).unwrap(),
            "fn user() {}"
        );
    }

    #[test]
    fn prune_deletes_managed_absent_keeps_unmanaged() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path();
        // First gen: a managed source file in a subdir + an unmanaged asset.
        gen(
            out,
            &tree! {
                "src" => dir! { "gen.rs" => src_leaf("generated") },
                "user.txt" => raw(b"mine".to_vec()),
            },
            regen(),
        );
        assert!(out.join("src/gen.rs").exists());

        // Second gen drops both leaves; prune should remove only the managed one.
        let mut opts = regen();
        opts.prune = true;
        gen(out, &QTree::new(), opts);

        assert!(!out.join("src/gen.rs").exists(), "managed file pruned");
        assert!(!out.join("src").exists(), "now-empty dir tidied up");
        assert!(out.join("user.txt").exists(), "unmanaged asset kept");
    }

    // --- #98: NixSink ------------------------------------------------------

    fn lower(name: &str, t: &QTree) -> String {
        let mut sink = NixSink::new(name);
        write_tree(&mut sink, t).unwrap();
        sink.into_source()
    }

    #[test]
    fn nix_sink_emits_linkfarm_with_nested_paths() {
        let t = tree! {
            "Cargo.toml" => raw(b"[package]\nname = \"x\"\n".to_vec()),
            "src" => dir! { "main.rs" => file(leaf("source_file", "fn main() {}")) },
        };
        let nix = lower("demo", &t);
        // A pkgs.linkFarm derivation named after the sink.
        assert!(nix.contains("pkgs.linkFarm \"demo\" ["), "{nix}");
        // Each leaf is a writeText store entry keyed by its `/`-joined path.
        assert!(nix.contains(r#"name = "Cargo.toml""#), "{nix}");
        assert!(nix.contains(r#"name = "src/main.rs""#), "{nix}");
        assert!(nix.contains("pkgs.writeText"), "{nix}");
        assert!(nix.contains("fn main() {}"), "{nix}");
    }

    #[test]
    fn nix_sink_escapes_content() {
        // Backslash, double-quote, and the `${` antiquotation opener must all be
        // escaped so the content stays literal.
        let t = tree! { "f" => raw(br#"a"b\c ${x}"#.to_vec()) };
        let nix = lower("e", &t);
        assert!(nix.contains(r#"a\"b\\c \${x}"#), "{nix}");
    }

    #[test]
    fn nix_sink_marks_executable_files() {
        let t = tree! {
            "run.sh" => raw(b"#!/bin/sh\necho hi\n".to_vec()).mode(0o755),
            "data.txt" => raw(b"plain\n".to_vec()),
        };
        let nix = lower("e", &t);
        // The 0o755 leaf uses writeTextFile { … executable = true; }; the plain
        // one uses writeText.
        assert!(nix.contains("executable = true;"), "{nix}");
        assert_eq!(nix.matches("executable = true;").count(), 1, "{nix}");
    }

    #[test]
    fn nix_sink_rejects_links_and_non_utf8() {
        // In-tree symlinks have no Nix-store spelling here.
        let t = tree! { "latest" => link_node("versions/v2") };
        let mut sink = NixSink::new("e");
        assert!(write_tree(&mut sink, &t).is_err());

        // Non-UTF-8 file content is rejected up front.
        let t = tree! { "blob" => raw(vec![0xff, 0xfe, 0x00]) };
        let mut sink = NixSink::new("e");
        assert!(write_tree(&mut sink, &t).is_err());
    }

    #[test]
    fn store_name_is_sanitized() {
        assert_eq!(store_name("src/main.rs"), "main.rs");
        assert_eq!(store_name(".gitignore"), "q.gitignore"); // never starts with '.'
        assert_eq!(store_name("a b!c"), "a_b_c");
    }
}
