//! Language adapters: the only language-specific surface of the server.
//!
//! Mirroring quilt's own `Language` / `MetaLanguage` split:
//!
//! * [`LanguageAdapter`] — a language that can appear as a **quoted fragment /
//!   downstream target** (every supported language, including weak ones like
//!   WGSL). It knows how to wrap a fragment so its server can parse it, what
//!   server to talk to, and how to mask holes inside a fragment.
//! * [`MetaLanguageAdapter`] — a language strong enough to be the **ground /
//!   host** that drives a whole `.quilt` file (Rust, Python — *not* WGSL). It
//!   knows how to reabsorb stage-0 splices as ground code and where the project
//!   root is.
//!
//! A host language (Rust) implements both and is registered in both registries;
//! a target-only language (WGSL) implements only [`LanguageAdapter`]. A file's
//! ground language must resolve a [`MetaLanguageAdapter`] to get full support;
//! otherwise it degrades to quilt-only (syntactic) features.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::Url;

/// How a host language reabsorbs inlined stage-0 `↙…↘` splices as a single
/// ground expression: `open` + each body + `terminator`, then `close`. For Rust
/// this is a block expression `{ a; b; }`.
pub struct SpliceBlock {
    pub open: &'static str,
    pub terminator: &'static str,
    pub close: &'static str,
}

/// How a language spells comments, so the projection can translate quilt's
/// `⟨//⟩` / `⟨/*⟩…⟨*/⟩` comment glyphs into delimiters the host parser accepts.
#[derive(Debug, Clone, Copy)]
pub struct CommentSyntax {
    /// Line-comment introducer, e.g. `//`.
    pub line: &'static str,
    /// Block-comment open/close, e.g. `/*` and `*/`.
    pub block_open: &'static str,
    pub block_close: &'static str,
}

/// A language that can be analyzed as an embedded fragment / downstream target.
pub trait LanguageAdapter: Send + Sync {
    /// LSP `languageId` for documents of this language (e.g. `"rust"`).
    fn language_id(&self) -> &'static str;
    /// File extension for this language's virtual documents (e.g. `"rs"`).
    fn virtual_extension(&self) -> &'static str;
    /// Downstream LSP server command, or `None` for highlight-only languages.
    fn server_command(&self) -> Option<(String, Vec<String>)>;
    /// Identifier used to mask holes (nested unquotes/glyphs) inside a fragment
    /// of this language, keeping it parseable (e.g. `"__q__"`).
    fn splice_placeholder(&self) -> &'static str;
    /// Wrap a quoted fragment body so the server parses it; returns
    /// `(prologue, epilogue)`. `n` disambiguates multiple fragments.
    fn wrap_fragment(&self, n: usize) -> (String, String);
    /// How this language spells comments, used to translate quilt's `⟨//⟩` /
    /// `⟨/*⟩…⟨*/⟩` glyphs into something the downstream parser accepts.
    fn comment_syntax(&self) -> CommentSyntax;
    /// Whether downstream diagnostics for this language should be surfaced.
    /// Default `true`; a language whose ground projection uses lossy placeholders
    /// (so ground code that consumes a quoted value mistypes) may opt out to
    /// avoid spurious errors until it has proper type information.
    fn publishes_diagnostics(&self) -> bool {
        true
    }
}

/// A language strong enough to be the ground/host of a `.quilt` file.
pub trait MetaLanguageAdapter: Send + Sync {
    /// Placeholder for a ground-level glyph operator (`↑↓←⟨T⟩⟨N⟩`).
    fn glyph_placeholder(&self) -> &'static str;
    /// How to wrap inlined stage-0 splice bodies as one ground expression.
    fn splice_block(&self) -> SpliceBlock;
    /// Discover the project root for the overlay (e.g. nearest `Cargo.toml`).
    fn find_root(&self, file: &Path) -> Option<PathBuf>;
    /// Downstream initialization options for a freshly-spawned server rooted at a
    /// project. `file` is the de-quilted overlay path; `detached` is true when the
    /// root has no project manifest and the file is analyzed standalone — in which
    /// case rust-analyzer must be told to treat it as a *detached file*, or its
    /// semantic features (hover/definition/completion) return nothing.
    fn server_init_options(&self, file: &Path, detached: bool) -> Value {
        let _ = (file, detached);
        json!({})
    }
    /// Whether `root` (the result of [`Self::find_root`]) lacks a project manifest
    /// so the file must be analyzed as a standalone/detached file. Default `false`
    /// (the server roots anywhere, like pyright). Rust overrides this to detect a
    /// manifest-less directory (a workspace orphan), which it then names in
    /// [`Self::server_init_options`].
    fn is_detached_root(&self, root: &Path) -> bool {
        let _ = root;
        false
    }
}

/// Whether `key` names a language this server recognizes as a filename
/// extension (mirroring the keys registered in quilt's `Omni`). Distinct from
/// having an adapter: a key can be known without one compiled in.
pub fn is_known_lang(key: &str) -> bool {
    matches!(
        key,
        "rs" | "rust" | "py" | "python" | "txt" | "text" | "wgsl" | "html" | "bash" | "zsh"
    )
}

/// Language-extension chain from a `.quilt` URL, ground (host) language first,
/// mirroring `lang_chain` in quilt's `bin.rs`: peel known-language extensions
/// off the stem right-to-left — the rightmost is the ground language and the
/// rest are the default languages for successively deeper un-annotated quotes.
/// `shaders.wgsl.rs.quilt` → `["rs", "wgsl"]`, `foo.rs.quilt` → `["rs"]`,
/// `foo.quilt` → `[]`. The basename never counts, even when it looks like a
/// language; an unknown rightmost extension is kept (`a.b.quilt` → `["b"]`) so
/// callers discover it resolves no adapter.
pub fn lang_chain(uri: &Url) -> Vec<String> {
    let Some(seg) = uri.path_segments().and_then(|mut s| s.next_back()) else {
        return Vec::new();
    };
    let name = seg.replace("%20", " ");
    let Some(stem) = name.strip_suffix(".quilt") else {
        return Vec::new();
    };
    let parts: Vec<&str> = stem.split('.').collect();
    let exts = &parts[1..];
    let mut chain: Vec<String> = exts
        .iter()
        .rev()
        .take_while(|part| is_known_lang(part))
        .map(|s| (*s).to_string())
        .collect();
    if chain.is_empty() {
        chain.extend(exts.last().map(|s| (*s).to_string()));
    }
    chain
}

/// Extract the ground-language key from a `.quilt` URL.
/// `foo.rs.quilt` → `Some("rs")`, `foo.quilt` → `None` (no inner extension).
pub fn ground_lang(uri: &Url) -> Option<String> {
    lang_chain(uri).into_iter().next()
}

/// Resolve the [`LanguageAdapter`] for a language key, if one is compiled in.
pub fn language_adapter(key: &str) -> Option<&'static dyn LanguageAdapter> {
    match key {
        #[cfg(feature = "rust")]
        "rs" | "rust" => Some(&RUST),
        #[cfg(feature = "python")]
        "py" | "python" => Some(&PYTHON),
        #[cfg(feature = "wgsl")]
        "wgsl" => Some(&WGSL),
        #[cfg(feature = "html")]
        "html" => Some(&HTML),
        #[cfg(feature = "bash")]
        "bash" => Some(&BASH),
        #[cfg(feature = "zsh")]
        "zsh" => Some(&ZSH),
        _ => None,
    }
}

/// Resolve the [`MetaLanguageAdapter`] (host capability) for a language key.
pub fn meta_adapter(key: &str) -> Option<&'static dyn MetaLanguageAdapter> {
    match key {
        #[cfg(feature = "rust")]
        "rs" | "rust" => Some(&RUST),
        #[cfg(feature = "python")]
        "py" | "python" => Some(&PYTHON),
        _ => None,
    }
}

/// Target-only languages dispatched **per-fragment** to their own downstream
/// server — each quoted fragment is analyzed as a standalone unit (e.g. a WGSL
/// `wgsl↖…↗` quote is a complete shader module sent to wgsl-analyzer), rather
/// than merged into the ground projection. Host languages (Rust, Python) are
/// *not* here: their same-language quotes ride the merged ground projection.
/// Languages with no downstream server (html, bash, zsh) are still listed:
/// their fragments are projected so the in-process tree-sitter highlighter
/// ([`crate::tshl`]) can produce semantic tokens, but nothing is `didOpen`ed.
pub fn embedded_adapters() -> Vec<&'static dyn LanguageAdapter> {
    let adapters: &[&'static dyn LanguageAdapter] = &[
        #[cfg(feature = "wgsl")]
        &WGSL,
        #[cfg(feature = "html")]
        &HTML,
        #[cfg(feature = "bash")]
        &BASH,
        #[cfg(feature = "zsh")]
        &ZSH,
    ];
    adapters.to_vec()
}

/* --------------------------------- Rust --------------------------------- */

#[cfg(feature = "rust")]
static RUST: RustAdapter = RustAdapter;

/// The Rust adapter — a host language, so it implements both traits.
#[cfg(feature = "rust")]
pub struct RustAdapter;

#[cfg(feature = "rust")]
impl LanguageAdapter for RustAdapter {
    fn language_id(&self) -> &'static str {
        "rust"
    }
    fn virtual_extension(&self) -> &'static str {
        "rs"
    }
    fn server_command(&self) -> Option<(String, Vec<String>)> {
        // Overridable via `QUILT_LSP_RUST_ANALYZER` (whitespace-separated, e.g.
        // a custom path or a test mock); defaults to `rust-analyzer` on `PATH`.
        let cmd = match std::env::var("QUILT_LSP_RUST_ANALYZER") {
            Ok(s) if !s.trim().is_empty() => {
                let mut parts = s.split_whitespace().map(str::to_string);
                let program = parts.next().unwrap_or_else(|| "rust-analyzer".to_string());
                (program, parts.collect())
            }
            _ => ("rust-analyzer".to_string(), Vec::new()),
        };
        Some(cmd)
    }
    fn splice_placeholder(&self) -> &'static str {
        "__q__"
    }
    fn wrap_fragment(&self, n: usize) -> (String, String) {
        (format!("\nfn _quilt_q{n}() {{\n"), "\n}\n".to_string())
    }
    fn comment_syntax(&self) -> CommentSyntax {
        CommentSyntax {
            line: "//",
            block_open: "/*",
            block_close: "*/",
        }
    }
}

#[cfg(feature = "rust")]
impl MetaLanguageAdapter for RustAdapter {
    fn glyph_placeholder(&self) -> &'static str {
        "__q__"
    }
    fn splice_block(&self) -> SpliceBlock {
        // A block expression: valid in expression position, and `{ }` for a
        // quote with no ground splices.
        SpliceBlock {
            open: "{ ",
            terminator: "; ",
            close: "}",
        }
    }
    fn find_root(&self, file: &Path) -> Option<PathBuf> {
        let mut dir = file.parent()?.to_path_buf();
        let fallback = dir.clone();
        let mut nearest: Option<PathBuf> = None;
        loop {
            let cargo_toml = dir.join("Cargo.toml");
            if cargo_toml.exists() {
                let content = std::fs::read_to_string(&cargo_toml).unwrap_or_default();
                if content.contains("[workspace]") {
                    // A workspace root only anchors rust-analyzer when this file is
                    // actually owned by one of its crates: either we passed a member
                    // package on the way up (`nearest`), or the workspace root is
                    // itself a package (`[package]`) that could own a file beneath it.
                    if nearest.is_some() || content.contains("[package]") {
                        return Some(dir);
                    }
                    // A pure virtual workspace that does *not* own this file — e.g. a
                    // standalone `examples/*.rs.quilt` script sitting under the
                    // repo-root workspace. Rooting rust-analyzer at the workspace
                    // would make it reject the file as "not in the module tree"; root
                    // at the file's own directory instead so it is analyzed as a
                    // detached single file (standalone inference).
                    return Some(fallback);
                }
                // Package that uses workspace inheritance (e.g. `edition.workspace =
                // true`) cannot compile without its workspace Cargo.toml, so keep
                // walking up to find the workspace root.
                if !content.contains(".workspace = true") {
                    // Self-contained package — use it directly.
                    return Some(dir);
                }
                if nearest.is_none() {
                    nearest = Some(dir.clone());
                }
            }
            if !dir.pop() {
                break;
            }
        }
        // No workspace root found above a workspace-inheritance package: use the
        // nearest package dir, or the file's own directory as last resort.
        nearest.or(Some(fallback))
    }

    fn server_init_options(&self, file: &Path, detached: bool) -> Value {
        if detached {
            // Tell rust-analyzer to analyze this standalone `.rs` as a detached
            // file so hover/definition/completion work (otherwise it is "not in
            // the module tree" and only syntactic features respond).
            json!({ "detachedFiles": [file.to_string_lossy()] })
        } else {
            json!({})
        }
    }

    fn is_detached_root(&self, root: &Path) -> bool {
        // `find_root` only returns a directory with no `Cargo.toml` when the file
        // is a workspace orphan (e.g. a standalone `examples/*.rs.quilt` script).
        !root.join("Cargo.toml").exists()
    }
}

/* -------------------------------- Python -------------------------------- */

#[cfg(feature = "python")]
static PYTHON: PythonAdapter = PythonAdapter;

/// The Python adapter — both a host (drives a `.py.quilt` file) and a target
/// (its quoted fragments are Python), so it implements both traits. It dispatches
/// to a standalone Python language server (pyright), which analyzes a single file
/// regardless of project layout — matching how `.py.quilt` scripts live anywhere.
///
/// The ground projection replaces each quote with a placeholder *expression*
/// (`()`), so Python is navigation-only for now: hover / go-to-definition /
/// completion work on the host Python, but diagnostics are suppressed (the
/// placeholder mistypes any ground line that consumes a quoted value). See
/// [`LanguageAdapter::publishes_diagnostics`]. Semantic tokens come from the
/// server's in-process tree-sitter highlighter ([`crate::tshl`]), since pyright
/// provides no semantic tokens (a Pylance-only feature).
#[cfg(feature = "python")]
pub struct PythonAdapter;

#[cfg(feature = "python")]
impl LanguageAdapter for PythonAdapter {
    fn language_id(&self) -> &'static str {
        "python"
    }
    fn virtual_extension(&self) -> &'static str {
        "py"
    }
    fn server_command(&self) -> Option<(String, Vec<String>)> {
        // Overridable via `QUILT_LSP_PYTHON_SERVER` (whitespace-separated, e.g. a
        // custom path or a different server); defaults to `pyright-langserver
        // --stdio` on `PATH`.
        let cmd = match std::env::var("QUILT_LSP_PYTHON_SERVER") {
            Ok(s) if !s.trim().is_empty() => {
                let mut parts = s.split_whitespace().map(str::to_string);
                let program = parts
                    .next()
                    .unwrap_or_else(|| "pyright-langserver".to_string());
                (program, parts.collect())
            }
            _ => (
                "pyright-langserver".to_string(),
                vec!["--stdio".to_string()],
            ),
        };
        Some(cmd)
    }
    fn splice_placeholder(&self) -> &'static str {
        "__q__"
    }
    fn wrap_fragment(&self, n: usize) -> (String, String) {
        // A parenthesized assignment: inside `( … )` Python ignores newlines and
        // indentation, so a multi-line quoted *expression* tokenizes without the
        // fragment carrying its own indentation. (Quoted statements are rare in
        // metaprogramming; at worst one fails to tokenize, and fragment
        // diagnostics are suppressed anyway.)
        (format!("\n_quilt_q{n} = (\n"), "\n)\n".to_string())
    }
    fn comment_syntax(&self) -> CommentSyntax {
        // Python has no block comments; the closest syntactically-inert delimiter
        // is a triple-quoted string. Block-comment glyphs are rare in `.py.quilt`.
        CommentSyntax {
            line: "#",
            block_open: "\"\"\"",
            block_close: "\"\"\"",
        }
    }
    fn publishes_diagnostics(&self) -> bool {
        false
    }
}

#[cfg(feature = "python")]
impl MetaLanguageAdapter for PythonAdapter {
    fn glyph_placeholder(&self) -> &'static str {
        "__q__"
    }
    fn splice_block(&self) -> SpliceBlock {
        // A parenthesized tuple, valid in expression position: `()` for a quote
        // with no ground splices, `(a, b, )` when stage-0 `↙…↘` splices are
        // reabsorbed (so pyright resolves the ground names they reference).
        SpliceBlock {
            open: "(",
            terminator: ", ",
            close: ")",
        }
    }
    fn find_root(&self, file: &Path) -> Option<PathBuf> {
        // pyright analyzes a file standalone; root at its own directory so several
        // `.py.quilt` scripts in one folder share a single server.
        file.parent().map(Path::to_path_buf)
    }
}

/* --------------------------------- WGSL --------------------------------- */

#[cfg(feature = "wgsl")]
static WGSL: WgslAdapter = WgslAdapter;

/// The WGSL adapter — a *target-only* language: it can appear as a quoted
/// fragment (`wgsl↖…↗`) inside a host like Rust, but it never drives a `.quilt`
/// file, so it implements only [`LanguageAdapter`] (no [`MetaLanguageAdapter`]).
/// Each WGSL quote is a complete shader module, dispatched per-fragment to
/// wgsl-analyzer (see [`embedded_adapters`]).
#[cfg(feature = "wgsl")]
pub struct WgslAdapter;

#[cfg(feature = "wgsl")]
impl LanguageAdapter for WgslAdapter {
    fn language_id(&self) -> &'static str {
        "wgsl"
    }
    fn virtual_extension(&self) -> &'static str {
        "wgsl"
    }
    fn server_command(&self) -> Option<(String, Vec<String>)> {
        // Overridable via `QUILT_LSP_WGSL_SERVER` (whitespace-separated); defaults
        // to `wgsl-analyzer` on `PATH`.
        let cmd = match std::env::var("QUILT_LSP_WGSL_SERVER") {
            Ok(s) if !s.trim().is_empty() => {
                let mut parts = s.split_whitespace().map(str::to_string);
                let program = parts.next().unwrap_or_else(|| "wgsl-analyzer".to_string());
                (program, parts.collect())
            }
            _ => ("wgsl-analyzer".to_string(), Vec::new()),
        };
        Some(cmd)
    }
    fn splice_placeholder(&self) -> &'static str {
        // A bare `↙…↘` Rust splice (e.g. `= ↙width.↑↘;`) sits in value position in
        // these shaders; `0` is a valid WGSL expression there.
        "0"
    }
    fn wrap_fragment(&self, _n: usize) -> (String, String) {
        // A WGSL quote is already a complete module; no wrapper is needed (and one
        // would be wrong — there is no enclosing function to put it in).
        (String::new(), String::new())
    }
    fn comment_syntax(&self) -> CommentSyntax {
        CommentSyntax {
            line: "//",
            block_open: "/*",
            block_close: "*/",
        }
    }
}

/* --------------------------------- HTML --------------------------------- */

#[cfg(feature = "html")]
static HTML: HtmlAdapter = HtmlAdapter;

/// The HTML adapter — target-only like WGSL, but with no downstream server at
/// all: [`LanguageAdapter::server_command`] is `None`, so `html↖…↗` quotes are
/// never `didOpen`ed anywhere. Their fragments exist purely for the in-process
/// tree-sitter highlighter ([`crate::tshl`]).
#[cfg(feature = "html")]
pub struct HtmlAdapter;

#[cfg(feature = "html")]
impl LanguageAdapter for HtmlAdapter {
    fn language_id(&self) -> &'static str {
        "html"
    }
    fn virtual_extension(&self) -> &'static str {
        "html"
    }
    fn server_command(&self) -> Option<(String, Vec<String>)> {
        None // highlight-only: no downstream HTML server
    }
    fn splice_placeholder(&self) -> &'static str {
        // A nested `↙…↘` splice sits in text or attribute-value position when
        // templating a page; a bare word is valid in both.
        "__q__"
    }
    fn wrap_fragment(&self, _n: usize) -> (String, String) {
        // An HTML quote is already a complete document or fragment.
        (String::new(), String::new())
    }
    fn comment_syntax(&self) -> CommentSyntax {
        // HTML has only `<!-- … -->`, so a `⟨//⟩` line-comment glyph opens a
        // comment that never closes. Tolerable: these fragments are
        // highlight-only, so the worst case is over-colored trailing text.
        CommentSyntax {
            line: "<!--",
            block_open: "<!--",
            block_close: "-->",
        }
    }
}

/* ------------------------------ Bash / Zsh ------------------------------- */

#[cfg(feature = "bash")]
static BASH: ShellAdapter = ShellAdapter { id: "bash" };

#[cfg(feature = "zsh")]
static ZSH: ShellAdapter = ShellAdapter { id: "zsh" };

/// The Bash and Zsh adapters — target-only and highlight-only, like
/// [`HtmlAdapter`]: `bash↖…↗` / `zsh↖…↗` quotes are projected for the
/// in-process tree-sitter highlighter and dispatched to no downstream server.
/// The two shells differ only in name (separate grammars, same adapter shape),
/// so they share one struct.
#[cfg(any(feature = "bash", feature = "zsh"))]
pub struct ShellAdapter {
    id: &'static str,
}

#[cfg(any(feature = "bash", feature = "zsh"))]
impl LanguageAdapter for ShellAdapter {
    fn language_id(&self) -> &'static str {
        self.id
    }
    fn virtual_extension(&self) -> &'static str {
        self.id
    }
    fn server_command(&self) -> Option<(String, Vec<String>)> {
        None // highlight-only: no downstream shell server
    }
    fn splice_placeholder(&self) -> &'static str {
        // A nested `↙…↘` splice sits in word/argument position; `__q__` is a
        // plain word there.
        "__q__"
    }
    fn wrap_fragment(&self, _n: usize) -> (String, String) {
        // A shell quote is already a complete script or command sequence.
        (String::new(), String::new())
    }
    fn comment_syntax(&self) -> CommentSyntax {
        // Shells have no block comments; the closest syntactically-inert
        // delimiter is a single-quoted string (it spans newlines). Same
        // pragmatic trade as Python's triple-quoted string: block-comment
        // glyphs are rare in shell quotes, and fragments are highlight-only.
        CommentSyntax {
            line: "#",
            block_open: "'",
            block_close: "'",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn detects_ground_lang() {
        assert_eq!(
            ground_lang(&url("file:///x/foo.rs.quilt")).as_deref(),
            Some("rs")
        );
        assert_eq!(
            ground_lang(&url("file:///x/a.b.wgsl.quilt")).as_deref(),
            Some("wgsl")
        );
        assert_eq!(ground_lang(&url("file:///x/foo.quilt")), None);
        assert_eq!(ground_lang(&url("file:///x/foo.rs")), None);
    }

    #[test]
    fn detects_extension_chains() {
        assert_eq!(
            lang_chain(&url("file:///x/shaders.wgsl.rs.quilt")),
            ["rs", "wgsl"]
        );
        assert_eq!(lang_chain(&url("file:///x/foo.rs.quilt")), ["rs"]);
        assert!(lang_chain(&url("file:///x/foo.quilt")).is_empty());
        // The basename never counts as a language, even when it looks like one.
        assert_eq!(lang_chain(&url("file:///x/text.rs.quilt")), ["rs"]);
        // An unknown extension stops the chain but is kept as the ground key.
        assert_eq!(lang_chain(&url("file:///x/a.b.quilt")), ["b"]);
        assert_eq!(
            ground_lang(&url("file:///x/shaders.wgsl.rs.quilt")).as_deref(),
            Some("rs")
        );
    }

    #[test]
    #[cfg(feature = "rust")]
    fn rust_is_host_and_target() {
        assert!(language_adapter("rs").is_some());
        assert!(meta_adapter("rust").is_some());
        assert_eq!(language_adapter("rs").unwrap().language_id(), "rust");
    }

    #[test]
    #[cfg(feature = "python")]
    fn python_is_host_and_target() {
        assert!(language_adapter("py").is_some());
        assert!(meta_adapter("python").is_some());
        assert_eq!(language_adapter("py").unwrap().language_id(), "python");
        // Python is a host: its same-language quotes ride the merged ground
        // projection, so it is *not* a per-fragment embedded target.
        assert!(!embedded_adapters()
            .iter()
            .any(|a| a.language_id() == "python"));
    }

    #[test]
    fn unknown_language_has_no_adapter() {
        assert!(language_adapter("cobol").is_none());
        assert!(meta_adapter("cobol").is_none());
    }

    #[test]
    #[cfg(feature = "rust")]
    fn rust_find_root_detaches_workspace_orphans() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // A pure virtual workspace at the root.
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();
        // A real member package (uses workspace inheritance).
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::write(
            root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nedition.workspace = true\n",
        )
        .unwrap();
        // A standalone script dir owned by no crate.
        fs::create_dir_all(root.join("examples")).unwrap();

        let adapter = RustAdapter;
        // A member file anchors at the workspace root.
        assert_eq!(
            adapter.find_root(&root.join("a/src/lib.rs")).as_deref(),
            Some(root)
        );
        // An orphan under the virtual workspace detaches to its own directory,
        // instead of returning the workspace (which RA would reject).
        assert_eq!(
            adapter.find_root(&root.join("examples/foo.rs")).as_deref(),
            Some(root.join("examples").as_path())
        );
    }

    #[test]
    fn wgsl_is_not_a_host() {
        // WGSL is a target-only language: it never resolves a MetaLanguageAdapter
        // (it cannot be the ground/host of a `.quilt` file), even though it now
        // has a LanguageAdapter so its quoted fragments can reach wgsl-analyzer.
        assert!(meta_adapter("wgsl").is_none());
    }

    #[test]
    fn shell_and_html_keys_are_known() {
        // `lang_chain` / quote annotations must recognize the highlight-only
        // languages, mirroring quilt's `Omni` registry keys.
        for key in ["html", "bash", "zsh"] {
            assert!(is_known_lang(key), "{key}");
        }
    }

    #[test]
    #[cfg(all(feature = "html", feature = "bash", feature = "zsh"))]
    fn highlight_only_targets_are_embedded_without_a_server() {
        // html/bash/zsh are per-fragment embedded targets so their quotes get
        // FragmentDoc projections to highlight, but they are never hosts and
        // have no downstream server (`embedded_sync` must skip `didOpen`).
        for key in ["html", "bash", "zsh"] {
            let adapter = language_adapter(key).unwrap_or_else(|| panic!("{key} adapter"));
            assert_eq!(adapter.language_id(), key);
            assert!(adapter.server_command().is_none(), "{key}");
            assert!(meta_adapter(key).is_none(), "{key}");
            assert!(
                embedded_adapters().iter().any(|a| a.language_id() == key),
                "{key}"
            );
        }
    }

    #[test]
    #[cfg(feature = "wgsl")]
    fn wgsl_is_an_embedded_target() {
        // WGSL has a LanguageAdapter (so its quotes reach wgsl-analyzer) and is
        // listed as a per-fragment embedded target.
        assert!(language_adapter("wgsl").is_some());
        assert_eq!(language_adapter("wgsl").unwrap().language_id(), "wgsl");
        assert!(embedded_adapters()
            .iter()
            .any(|a| a.language_id() == "wgsl"));
    }
}
