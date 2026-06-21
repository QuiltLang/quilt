//! The scaffold manifest: `.quilt/manifest.json` under a generated tree's root.
//!
//! It records, per generated path, a content hash and whether the path is
//! *header-managed* (machine-owned: written with a DO-NOT-EDIT marker). This is
//! what makes re-running a generator safe ([`FsSink`](crate::sink::FsSink)):
//!
//! - **edit detection** — if a managed file's on-disk hash no longer matches the
//!   recorded one, the user edited it, so regen skips it instead of clobbering;
//! - **`--prune`** — paths that were managed last time but are absent from the
//!   new tree are deleted;
//! - **header guard** — only files still carrying the marker are overwritten.
//!
//! Always compiled (no tree-sitter), like [`tree`](crate::tree) and
//! [`sink`](crate::sink). See `docs/design/directory-scaffolding.md` §6.6, §9.

use miette::{IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Current on-disk manifest schema version.
pub const MANIFEST_VERSION: u32 = 1;

/// One generated path's record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Content hash of the bytes written (see [`content_hash`]).
    pub hash: String,
    /// Whether the path is machine-owned (written with a DO-NOT-EDIT marker).
    /// Only managed paths are header-guarded and pruned.
    pub managed: bool,
}

/// The set of paths a previous generation wrote under a root, keyed by their
/// `/`-joined relative path. Ordered (`BTreeMap`) for stable, diff-friendly
/// output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub entries: BTreeMap<String, ManifestEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

impl Manifest {
    /// The manifest path under a tree root: `<root>/.quilt/manifest.json`.
    #[must_use]
    pub fn path(root: &Path) -> PathBuf {
        root.join(".quilt").join("manifest.json")
    }

    /// Load the manifest under `root`, or an empty one if it is missing or
    /// unreadable (a corrupt manifest degrades to "nothing was generated before"
    /// rather than failing the run).
    #[must_use]
    pub fn load(root: &Path) -> Self {
        std::fs::read(Self::path(root))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Write the manifest under `root` (creating `.quilt/`).
    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = root.join(".quilt");
        std::fs::create_dir_all(&dir).into_diagnostic()?;
        let json = serde_json::to_vec_pretty(self).into_diagnostic()?;
        std::fs::write(Self::path(root), json).into_diagnostic()?;
        Ok(())
    }

    #[must_use]
    pub fn get(&self, path: &str) -> Option<&ManifestEntry> {
        self.entries.get(path)
    }

    pub fn insert(&mut self, path: String, entry: ManifestEntry) {
        self.entries.insert(path, entry);
    }
}

/// A stable, non-cryptographic content hash (FNV-1a, 64-bit, hex). Stable across
/// runs and platforms — good enough to detect user edits; not a security
/// primitive.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_distinguishes() {
        assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
        assert_ne!(content_hash(b"hello"), content_hash(b"hellp"));
        assert_eq!(content_hash(b"").len(), 16);
    }

    #[test]
    fn round_trips_through_disk() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let mut m = Manifest::default();
        m.insert(
            "src/main.rs".to_string(),
            ManifestEntry {
                hash: content_hash(b"x"),
                managed: true,
            },
        );
        m.save(root).unwrap();
        assert!(Manifest::path(root).exists());

        let loaded = Manifest::load(root);
        assert_eq!(loaded.version, MANIFEST_VERSION);
        assert_eq!(loaded.entries, m.entries);
    }

    #[test]
    fn missing_manifest_loads_empty() {
        let d = tempfile::tempdir().unwrap();
        let m = Manifest::load(d.path());
        assert!(m.entries.is_empty());
    }
}
