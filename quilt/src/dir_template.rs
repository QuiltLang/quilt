//! Directory instantiation (issue #90): walk a hand-authored template directory
//! and instantiate every file against one shared, inferred parameter
//! environment, producing a [`QTree`]. The directory *is* the template — no
//! manifest or control file.
//!
//! A file named `*.tmpl.quilt` is a **sky-first template**: it is parsed
//! sky-first (the file *is* the body of one `target↖ … ↗`, see
//! [`Multi::parse_template`]) and its `↙name↘` holes are filled via Tier A
//! ([`crate::template::instantiate`]) — or, when the file opens with a
//! [`#!tier-b`](crate::template::TIER_B_MARKER) marker, via a host-backed Tier B
//! render the caller supplies. The output path drops the `.tmpl.quilt` marker
//! (`main.py.tmpl.quilt` → `main.py`); the remaining extensions give its
//! language chain. Every other file is copied **verbatim** into a [`Raw`] leaf,
//! so binary assets pass through untouched.
//!
//! The template's parameter signature is the *union* of the free variables of
//! all its template files ([`dir_params`]); an instantiation must supply every
//! one. Templated file/dir *names* (path-segment holes) are issue #91.
//!
//! Materialization is the sinks' job ([`crate::sink`]): build the `QTree` here,
//! then `write_tree` it through an [`FsSink`](crate::sink::FsSink). This module
//! needs the parser, so it is gated on the `parse` feature.
//!
//! [`Raw`]: crate::tree::Node::Raw

use crate::multi::{lang_chain, template_params, Languages, MetaLanguages, Multi};
use crate::prelude::*;
use crate::template::{instantiate, strip_tier_b_marker, ParamEnv};
use miette::{bail, Context, IntoDiagnostic};
use std::path::Path;

/// Suffix marking a directory entry as a sky-first template file. The output
/// drops it: `Cargo.toml.tmpl.quilt` → `Cargo.toml`, `main.py.tmpl.quilt` →
/// `main.py`.
const TEMPLATE_SUFFIX: &str = ".tmpl.quilt";

/// A directory entry parsed but **not yet** instantiated. Building the whole
/// pending tree first lets [`instantiate_dir_with`] report *every* missing
/// parameter up front (across all files) before doing any work.
enum Pending {
    /// A subdirectory and its (already-pending) children.
    Dir(Vec<(Segment, Pending)>),
    /// A Tier A template: its sky-first parse, instantiated against the env in
    /// the build phase. `display` is the on-disk path, for error context.
    TierA {
        template: Arc<QTerm>,
        display: String,
    },
    /// A Tier B template: its language chain and raw body, rendered by the
    /// caller's host in the build phase. `display` is the on-disk path.
    TierB {
        chain: Vec<String>,
        body: String,
        display: String,
    },
    /// A non-template file, copied through verbatim.
    Raw(Vec<u8>),
}

/// The signature of a caller-supplied Tier B renderer: turn one template file —
/// its language `chain`, raw `body`, and the parameter `env` — into finished
/// output. Running a host language is a process side effect that the parser-free
/// `template` module deliberately avoids, so directory instantiation takes the
/// renderer as a callback. The CLI (`bin.rs`) wires it to a real Python run.
pub type TierBRender<'a> = dyn FnMut(&[&str], &str, &ParamEnv) -> Result<String> + 'a;

/// Collect the parameter signature of a template directory: the union of the
/// free variables of every `*.tmpl.quilt` file under `dir`, in first-seen order
/// with duplicates removed. This is the set of values an instantiation must
/// supply. (Tier B files contribute their *bare-name* holes; host-expression
/// holes are the running host's concern, exactly as in the single-file path.)
pub fn dir_params<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    dir: &Path,
) -> Result<Vec<Box<str>>> {
    let mut params = Vec::new();
    walk_dir(multi, dir, &mut params)?;
    Ok(params)
}

/// Instantiate the template directory `dir` against `env`, Tier A only:
/// errors if any file opts into Tier B (which needs a running host — use
/// [`instantiate_dir_with`] and supply a renderer). See the module docs.
pub fn instantiate_dir<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    dir: &Path,
    env: &ParamEnv,
) -> Result<QTree> {
    instantiate_dir_with(multi, dir, env, &mut no_tier_b)
}

/// Instantiate the template directory `dir` against `env`, rendering any
/// `#!tier-b` file through `render_tier_b`. Reports *all* missing parameters up
/// front (clearer than failing at the first file), then builds the [`QTree`].
pub fn instantiate_dir_with<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    dir: &Path,
    env: &ParamEnv,
    render_tier_b: &mut TierBRender<'_>,
) -> Result<QTree> {
    let mut params = Vec::new();
    let pending = walk_dir(multi, dir, &mut params)?;

    let missing: Vec<&str> = params
        .iter()
        .filter(|p| !env.contains_key(&***p))
        .map(|p| &**p)
        .collect();
    if !missing.is_empty() {
        bail!("missing template parameter(s): {}", missing.join(", "));
    }

    build_tree(pending, env, render_tier_b)
}

/// The default Tier B renderer for [`instantiate_dir`]: refuse, pointing at the
/// CLI path that can actually run a host.
fn no_tier_b(_chain: &[&str], _body: &str, _env: &ParamEnv) -> Result<String> {
    bail!(
        "this template directory contains a `#!tier-b` file, which needs a running host; \
         instantiate it through the `quilt instantiate` CLI"
    )
}

/// Recursively parse `dir` into a [`Pending`] tree, accumulating the free-var
/// `params` of every template file (deduped, first-seen order). Directory
/// entries are sorted by name so the resulting tree — and any materialized
/// output — is deterministic regardless of `read_dir` order.
fn walk_dir<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    dir: &Path,
    params: &mut Vec<Box<str>>,
) -> Result<Vec<(Segment, Pending)>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading template directory {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .into_diagnostic()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let raw_name = entry.file_name();
        let name = raw_name
            .to_str()
            .ok_or_else(|| miette!("non-UTF-8 file name in template dir: {raw_name:?}"))?;
        let path = entry.path();
        let ftype = entry.file_type().into_diagnostic()?;

        if ftype.is_dir() {
            let children = walk_dir(multi, &path, params)?;
            out.push((path_segment(name)?, Pending::Dir(children)));
        } else {
            let pending = file_pending(multi, &path, name, params)?;
            out.push((output_segment(name)?, pending));
        }
    }
    Ok(out)
}

/// Parse one file into a [`Pending`], adding any template file's params to
/// `params`. `*.tmpl.quilt` files are parsed sky-first (Tier A or, behind the
/// `#!tier-b` marker, Tier B); everything else becomes verbatim bytes.
fn file_pending<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    path: &Path,
    name: &str,
    params: &mut Vec<Box<str>>,
) -> Result<Pending> {
    let Some(stem) = name.strip_suffix(TEMPLATE_SUFFIX) else {
        // Not a template: copy through verbatim (works for binary assets too).
        return Ok(Pending::Raw(std::fs::read(path).into_diagnostic()?));
    };

    let raw = std::fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading template {}", path.display()))?;
    let (tier_b, body) = match strip_tier_b_marker(&raw) {
        Some(body) => (true, body.to_owned()),
        None => (false, raw),
    };

    // The chain comes from the output name (`main.py` from `main.py.tmpl.quilt`),
    // owned because it must outlive the borrowed `name`/`stem`.
    let chain: Vec<String> = lang_chain(multi, stem)
        .into_iter()
        .map(str::to_owned)
        .collect();
    let chain_refs: Vec<&str> = chain.iter().map(String::as_str).collect();

    let template = multi
        .parse_template(&chain_refs, &body)
        .wrap_err_with(|| format!("parsing template {}", path.display()))?;
    for p in template_params(&template) {
        if !params.contains(&p) {
            params.push(p);
        }
    }

    let display = path.display().to_string();
    Ok(if tier_b {
        Pending::TierB {
            chain,
            body,
            display,
        }
    } else {
        Pending::TierA { template, display }
    })
}

/// Build the final [`QTree`] from the pending tree: instantiate Tier A templates
/// against `env`, render Tier B ones through `render_tier_b`, and pass raw files
/// through.
fn build_tree(
    pending: Vec<(Segment, Pending)>,
    env: &ParamEnv,
    render_tier_b: &mut TierBRender<'_>,
) -> Result<QTree> {
    let mut tree = QTree::new();
    for (seg, node) in pending {
        let node = match node {
            Pending::Dir(children) => Node::Dir(build_tree(children, env, render_tier_b)?),
            Pending::TierA { template, display } => {
                let out = instantiate(&template, env)
                    .wrap_err_with(|| format!("instantiating {display}"))?;
                file(out)
            }
            Pending::TierB {
                chain,
                body,
                display,
            } => {
                let chain_refs: Vec<&str> = chain.iter().map(String::as_str).collect();
                let rendered = render_tier_b(&chain_refs, &body, env)
                    .wrap_err_with(|| format!("rendering {display}"))?;
                raw(rendered.into_bytes())
            }
            Pending::Raw(bytes) => raw(bytes),
        };
        // `seg` is a validated single component; `emit` re-splits on '/' (a no-op
        // here) and inserts it.
        tree.emit(seg.as_str(), node)?;
    }
    Ok(tree)
}

/// The output path segment for a non-template entry (a directory or a verbatim
/// file): the name as-is, validated. Issue #91 layers `↙name↘`/`{{name}}` hole
/// substitution on top of this.
fn path_segment(name: &str) -> Result<Segment> {
    Segment::new(name)
}

/// The output path segment for a file, stripping the `.tmpl.quilt` template
/// marker (`main.py.tmpl.quilt` → `main.py`) before validating.
fn output_segment(name: &str) -> Result<Segment> {
    path_segment(name.strip_suffix(TEMPLATE_SUFFIX).unwrap_or(name))
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs::omni::Omni;
    use crate::template::ParamValue;
    use crate::tree::Node;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    /// A template directory laid out from `(relative path, contents)` pairs.
    /// Intermediate directories are created as needed.
    fn template_dir(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (rel, contents) in files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        dir
    }

    fn env(pairs: &[(&str, ParamValue)]) -> ParamEnv {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), v.clone()))
            .collect::<BTreeMap<_, _>>()
    }

    /// The instantiated text of a `File`/`Raw` leaf at `path` (slash-joined).
    fn leaf_text(tree: &QTree, path: &str) -> String {
        let mut cur = tree;
        let segs: Vec<_> = path.split('/').collect();
        for seg in &segs[..segs.len() - 1] {
            let Some(Node::Dir(sub)) = cur.get(&Segment::new(*seg).unwrap()) else {
                panic!("missing dir {seg} in {path}");
            };
            cur = sub;
        }
        match cur.get(&Segment::new(segs[segs.len() - 1]).unwrap()) {
            Some(Node::File { content, .. }) => content.coparse(),
            Some(Node::Raw { bytes, .. }) => String::from_utf8(bytes.clone()).unwrap(),
            other => panic!("expected a file leaf at {path}, got {other:?}"),
        }
    }

    #[test]
    fn instantiates_templates_and_copies_assets() {
        let dir = template_dir(&[
            ("greeting.py.tmpl.quilt", "msg = ↙greeting↘\nn = ↙count↘\n"),
            ("src/app.py.tmpl.quilt", "name = ↙who↘\n"),
            ("README.md", "# A fixed asset, no holes\n"),
        ]);
        let mut multi = Omni::default();
        let tree = instantiate_dir(
            &mut multi,
            dir.path(),
            &env(&[
                ("greeting", "hi".into()),
                ("count", 3.into()),
                ("who", "bob".into()),
            ]),
        )
        .unwrap();

        // The `.tmpl.quilt` marker is dropped from output names.
        assert_eq!(leaf_text(&tree, "greeting.py"), "msg = \"hi\"\nn = 3\n");
        assert_eq!(leaf_text(&tree, "src/app.py"), "name = \"bob\"\n");
        // Non-template files are copied verbatim, name unchanged.
        assert_eq!(leaf_text(&tree, "README.md"), "# A fixed asset, no holes\n");
    }

    #[test]
    fn asset_is_a_raw_leaf_and_template_is_a_file_leaf() {
        let dir = template_dir(&[("x.py.tmpl.quilt", "v = ↙v↘\n"), ("logo.bin", "asset")]);
        let mut multi = Omni::default();
        let tree = instantiate_dir(&mut multi, dir.path(), &env(&[("v", 1.into())])).unwrap();
        assert!(matches!(
            tree.get(&Segment::new("x.py").unwrap()),
            Some(Node::File { .. })
        ));
        assert!(matches!(
            tree.get(&Segment::new("logo.bin").unwrap()),
            Some(Node::Raw { .. })
        ));
    }

    #[test]
    fn params_are_the_union_across_files() {
        let dir = template_dir(&[
            ("a.py.tmpl.quilt", "x = ↙one↘\ny = ↙two↘\n"),
            ("b.py.tmpl.quilt", "z = ↙two↘\nw = ↙three↘\n"),
        ]);
        let mut multi = Omni::default();
        let params = dir_params(&mut multi, dir.path()).unwrap();
        // First-seen order across the (name-sorted) files, deduped.
        assert_eq!(
            params.iter().map(|p| &**p).collect::<Vec<_>>(),
            vec!["one", "two", "three"]
        );
    }

    #[test]
    fn missing_params_are_all_reported() {
        let dir = template_dir(&[("t.py.tmpl.quilt", "a = ↙p↘\nb = ↙q↘\n")]);
        let mut multi = Omni::default();
        let err = instantiate_dir(&mut multi, dir.path(), &env(&[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing template parameter"), "got: {err}");
        assert!(err.contains('p') && err.contains('q'), "got: {err}");
    }

    #[test]
    fn empty_subdir_is_kept() {
        let dir = template_dir(&[("keep.py.tmpl.quilt", "x = ↙v↘\n")]);
        fs::create_dir(dir.path().join("emptydir")).unwrap();
        let mut multi = Omni::default();
        let tree = instantiate_dir(&mut multi, dir.path(), &env(&[("v", 1.into())])).unwrap();
        assert!(matches!(
            tree.get(&Segment::new("emptydir").unwrap()),
            Some(Node::Dir(d)) if d.is_empty()
        ));
    }

    #[test]
    fn tier_b_file_without_a_renderer_errors() {
        let dir = template_dir(&[("t.py.tmpl.quilt", "#!tier-b\nx = ↙n↘\n")]);
        let mut multi = Omni::default();
        let report = instantiate_dir(&mut multi, dir.path(), &env(&[("n", 1.into())])).unwrap_err();
        // The refusal is the error's cause (wrapped with the file's path), so
        // check the whole rendered chain, not just the top message.
        let full = format!("{report:?}");
        assert!(full.contains("running host"), "got: {full}");
    }

    #[test]
    fn tier_b_render_callback_supplies_content() {
        let dir = template_dir(&[("t.py.tmpl.quilt", "#!tier-b\ng = ↙greeting↘\n")]);
        let mut multi = Omni::default();
        // A stand-in host: echoes a fixed rendering instead of running Python.
        let mut render =
            |_chain: &[&str], _body: &str, _env: &ParamEnv| Ok("g = rendered\n".into());
        let tree = instantiate_dir_with(
            &mut multi,
            dir.path(),
            &env(&[("greeting", "hi".into())]),
            &mut render,
        )
        .unwrap();
        assert_eq!(leaf_text(&tree, "t.py"), "g = rendered\n");
    }
}
