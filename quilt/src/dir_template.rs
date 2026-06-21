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
//! File and directory **names** may themselves carry parameter holes (issue
//! #91), in either spelling: the glyph `↙name↘` (uniform with content holes) or
//! the ASCII `{{name}}` (byte-safe for filesystems and tooling — e.g. git's
//! `core.quotePath` octal-escapes glyph filenames). A path hole is a bare
//! parameter reference; after substitution each segment is validated (no `/`,
//! `.`/`..`, etc.).
//!
//! The template's parameter signature is the *union* of the free variables of
//! all its template files *and* path-segment holes ([`dir_params`]); an
//! instantiation must supply every one.
//!
//! Materialization is the sinks' job ([`crate::sink`]): build the `QTree` here,
//! then `write_tree` it through an [`FsSink`](crate::sink::FsSink). This module
//! needs the parser, so it is gated on the `parse` feature.
//!
//! [`Raw`]: crate::tree::Node::Raw

use crate::multi::{ident_name, lang_chain, template_params, Languages, MetaLanguages, Multi};
use crate::prelude::*;
use crate::template::{instantiate, strip_tier_b_marker, ParamEnv, ParamValue};
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
    /// A subdirectory and its (already-pending) children, each keyed by its
    /// pre-substitution output name.
    Dir(Vec<(String, Pending)>),
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
/// `params` of every template file *and* path-segment hole (deduped, first-seen
/// order). Directory entries are sorted by name so the resulting tree — and any
/// materialized output — is deterministic regardless of `read_dir` order.
///
/// Each entry is paired with its *pre-substitution* output name (the on-disk
/// name with the `.tmpl.quilt` marker dropped for files). Path holes (issue #91)
/// are substituted later, in [`build_tree`], because [`dir_params`] walks
/// without an env.
fn walk_dir<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    dir: &Path,
    params: &mut Vec<Box<str>>,
) -> Result<Vec<(String, Pending)>> {
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
        let is_dir = entry.file_type().into_diagnostic()?.is_dir();

        // The output name drops the `.tmpl.quilt` marker from a template file;
        // a directory (or verbatim asset) keeps its name. Either may carry path
        // holes, whose parameter names join the signature.
        let out_name = if is_dir {
            name.to_owned()
        } else {
            name.strip_suffix(TEMPLATE_SUFFIX)
                .unwrap_or(name)
                .to_owned()
        };
        for p in path_hole_params(&out_name)? {
            if !params.contains(&p) {
                params.push(p);
            }
        }

        let pending = if is_dir {
            Pending::Dir(walk_dir(multi, &path, params)?)
        } else {
            file_pending(multi, &path, name, params)?
        };
        out.push((out_name, pending));
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
    pending: Vec<(String, Pending)>,
    env: &ParamEnv,
    render_tier_b: &mut TierBRender<'_>,
) -> Result<QTree> {
    let mut tree = QTree::new();
    for (name, node) in pending {
        let seg = subst_path_segment(&name, env)?;
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

/**************************************************************/
// Templated path segments (issue #91): file/dir names carrying parameter holes.

/// Substitute every path-segment hole in `name` with its parameter value
/// (string-rendered), then validate the result as a single [`Segment`]. Both
/// spellings are accepted — `↙name↘` and `{{name}}` — and a hole is a *bare*
/// parameter reference (a path can't host a Tier-B expression).
fn subst_path_segment(name: &str, env: &ParamEnv) -> Result<Segment> {
    let filled = map_path_holes(name, |inner| {
        let id = ident_name(inner).ok_or_else(|| hole_name_error(name, inner))?;
        let value = env
            .get(&*id)
            .ok_or_else(|| miette!("missing template parameter `{id}` in path segment {name:?}"))?;
        render_path_value(value).wrap_err_with(|| format!("in path segment {name:?}"))
    })?;
    // Post-substitution validation: a value can't smuggle in a `/`, `.`/`..`, …
    Segment::new(filled).wrap_err_with(|| format!("instantiating path segment {name:?}"))
}

/// The bare parameter names referenced by `name`'s path-segment holes (both
/// spellings), in first-seen order, deduped. These join the template's
/// signature so a name-only parameter is still required and checked up front.
fn path_hole_params(name: &str) -> Result<Vec<Box<str>>> {
    let mut names = Vec::new();
    map_path_holes(name, |inner| {
        let id = ident_name(inner).ok_or_else(|| hole_name_error(name, inner))?;
        if !names.contains(&id) {
            names.push(id);
        }
        Ok(String::new())
    })?;
    Ok(names)
}

/// Rewrite `name` by replacing each path-segment hole with `f(inner)`, copying
/// literal text through. Recognizes both the glyph form `↙inner↘` and the ASCII
/// form `{{inner}}`; an unterminated opener is an error. `inner` is the hole's
/// raw (untrimmed) body — `f` decides what counts as a valid name.
fn map_path_holes(name: &str, mut f: impl FnMut(&str) -> Result<String>) -> Result<String> {
    const OPEN: char = '↙';
    const CLOSE: char = '↘';
    let mut out = String::new();
    let mut rest = name;
    loop {
        // The next hole starts at whichever opener comes first: ASCII `{{` or `↙`.
        let (at, braced) = match (rest.find("{{"), rest.find(OPEN)) {
            (None, None) => {
                out.push_str(rest);
                return Ok(out);
            }
            (Some(b), None) => (b, true),
            (None, Some(g)) => (g, false),
            (Some(b), Some(g)) => {
                if b < g {
                    (b, true)
                } else {
                    (g, false)
                }
            }
        };
        out.push_str(&rest[..at]);
        let body = &rest[at + if braced { "{{".len() } else { OPEN.len_utf8() }..];
        let (inner, after) = if braced {
            let end = body
                .find("}}")
                .ok_or_else(|| miette!("unterminated `{{{{…}}}}` path hole in {name:?}"))?;
            (&body[..end], &body[end + "}}".len()..])
        } else {
            let end = body
                .find(CLOSE)
                .ok_or_else(|| miette!("unterminated `↙…↘` path hole in {name:?}"))?;
            (&body[..end], &body[end + CLOSE.len_utf8()..])
        };
        out.push_str(&f(inner)?);
        rest = after;
    }
}

/// Render a parameter value as a path-segment string. Scalars stringify
/// plainly; a list has no path spelling, so it errors.
fn render_path_value(value: &ParamValue) -> Result<String> {
    Ok(match value {
        ParamValue::Str(s) => s.clone(),
        ParamValue::Int(i) => i.to_string(),
        ParamValue::Float(f) => format!("{f:?}"),
        ParamValue::Bool(b) => if *b { "true" } else { "false" }.to_owned(),
        ParamValue::List(_) => bail!("a list parameter has no path-segment spelling"),
    })
}

/// Error for a path hole whose body is not a bare parameter name.
fn hole_name_error(name: &str, inner: &str) -> miette::Report {
    miette!(
        "a path-segment hole must be a bare parameter name; `{}` in {name:?} is not",
        inner.trim()
    )
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

    /**********************************************************/
    // Templated path segments (issue #91).

    #[test]
    fn ascii_path_hole_in_a_file_name() {
        let dir = template_dir(&[("{{module}}.py.tmpl.quilt", "x = ↙v↘\n")]);
        let mut multi = Omni::default();
        let tree = instantiate_dir(
            &mut multi,
            dir.path(),
            &env(&[("module", "widgets".into()), ("v", 1.into())]),
        )
        .unwrap();
        assert_eq!(leaf_text(&tree, "widgets.py"), "x = 1\n");
    }

    #[test]
    fn glyph_path_hole_round_trips() {
        // A file literally named with the `↙name↘` glyph spelling on disk.
        let dir = template_dir(&[("↙module↘.py.tmpl.quilt", "x = ↙v↘\n")]);
        let mut multi = Omni::default();
        let tree = instantiate_dir(
            &mut multi,
            dir.path(),
            &env(&[("module", "shapes".into()), ("v", 2.into())]),
        )
        .unwrap();
        assert_eq!(leaf_text(&tree, "shapes.py"), "x = 2\n");
    }

    #[test]
    fn templated_directory_name() {
        let dir = template_dir(&[("{{pkg}}/app.py.tmpl.quilt", "n = ↙v↘\n")]);
        let mut multi = Omni::default();
        let tree = instantiate_dir(
            &mut multi,
            dir.path(),
            &env(&[("pkg", "myapp".into()), ("v", 3.into())]),
        )
        .unwrap();
        assert_eq!(leaf_text(&tree, "myapp/app.py"), "n = 3\n");
    }

    #[test]
    fn hole_mixed_with_literal_text_and_an_int_value() {
        // A hole need not be the whole segment, and a non-string value (int)
        // stringifies into the name.
        let dir = template_dir(&[("{{name}}_v↙ver↘.py.tmpl.quilt", "x = ↙v↘\n")]);
        let mut multi = Omni::default();
        let tree = instantiate_dir(
            &mut multi,
            dir.path(),
            &env(&[("name", "core".into()), ("ver", 2.into()), ("v", 0.into())]),
        )
        .unwrap();
        assert_eq!(leaf_text(&tree, "core_v2.py"), "x = 0\n");
    }

    #[test]
    fn path_hole_param_joins_the_signature() {
        // `module` appears only in a file *name*, never in content — it is still
        // a required parameter, reported up front when missing.
        let dir = template_dir(&[("{{module}}.py.tmpl.quilt", "x = 1\n")]);
        let mut multi = Omni::default();
        assert_eq!(
            dir_params(&mut multi, dir.path())
                .unwrap()
                .iter()
                .map(|p| &**p)
                .collect::<Vec<_>>(),
            vec!["module"]
        );
        let err = instantiate_dir(&mut multi, dir.path(), &env(&[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("module"), "got: {err}");
    }

    #[test]
    fn substituted_value_with_a_slash_is_rejected() {
        // A value can't smuggle path structure past segment validation.
        let dir = template_dir(&[("{{module}}.py.tmpl.quilt", "x = 1\n")]);
        let mut multi = Omni::default();
        let err = instantiate_dir(&mut multi, dir.path(), &env(&[("module", "a/b".into())]))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("path segment") || err.contains('/'),
            "got: {err}"
        );
    }

    #[test]
    fn non_bare_path_hole_is_rejected() {
        let dir = template_dir(&[("{{a.b}}.py.tmpl.quilt", "x = 1\n")]);
        let mut multi = Omni::default();
        let err = dir_params(&mut multi, dir.path()).unwrap_err().to_string();
        assert!(err.contains("bare parameter name"), "got: {err}");
    }
}
