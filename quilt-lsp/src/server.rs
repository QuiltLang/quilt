//! The editor-facing LSP server and the router that proxies to downstream
//! language servers.
//!
//! For a Rust `.quilt` file the server projects the ground language into a
//! virtual `.rs` document (see [`crate::projection`]), opens it to a single
//! lazily-spawned rust-analyzer under the *de-quilted* file URI (so it resolves
//! inside the real project), forwards position requests with coordinates mapped
//! into the virtual doc, and maps results back. Downstream diagnostics are
//! remapped and merged with quilt's own syntax diagnostics.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use dashmap::DashMap;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};
use tower_lsp::jsonrpc::Result;
#[allow(clippy::wildcard_imports)]
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::adapters::{embedded_adapters, ground_lang, language_adapter, meta_adapter};
use crate::child::{ChildNotification, ChildServer};
use crate::document::Document;
use crate::lineindex::{Encoding, LineIndex};
use crate::projection::{project, project_fragments, Projection};
use crate::regions::Region;
use crate::translate::{self, Mapper};

/// Whether the ground language is a host (has a `MetaLanguageAdapter`), so we can
/// project it and drive a downstream server. Non-host grounds get only
/// quilt-level (syntactic) features.
fn is_host_ground(ground: Option<&str>) -> bool {
    ground.and_then(meta_adapter).is_some()
}

/// `foo.rs.quilt` → `file:///…/foo.rs` (string URI), the overlay we open to the
/// downstream server.
fn dequilt_uri(quilt_uri: &Url) -> Option<String> {
    let path = quilt_uri.to_file_path().ok()?;
    let stripped = path.to_str()?.strip_suffix(".quilt")?;
    Url::from_file_path(stripped).ok().map(|u| u.to_string())
}

/// Folding ranges for the quilt regions themselves (each multi-line `↖…↗` /
/// `↙…↘`), recursively.
fn collect_region_folds(
    region: &Region,
    text: &str,
    line_index: &LineIndex,
    enc: Encoding,
    out: &mut Vec<FoldingRange>,
) {
    for child in &region.children {
        let start = line_index.position(text, child.body.start, enc).line;
        let end = line_index.position(text, child.body.end, enc).line;
        if end > start {
            out.push(FoldingRange {
                start_line: start,
                end_line: end,
                kind: Some(FoldingRangeKind::Region),
                ..Default::default()
            });
        }
        collect_region_folds(child, text, line_index, enc, out);
    }
}

/// Remap a downstream folding range (line numbers in virtual coords) to quilt
/// line numbers.
fn remap_folding(
    fr: &Value,
    proj: &Projection,
    text: &str,
    line_index: &LineIndex,
    enc: Encoding,
) -> Option<FoldingRange> {
    let to_quilt_line = |vline: u32| {
        proj.to_quilt(
            text,
            line_index,
            enc,
            Position {
                line: vline,
                character: 0,
            },
        )
        .line
    };
    let start = u32::try_from(fr.get("startLine")?.as_u64()?).ok()?;
    let end = u32::try_from(fr.get("endLine")?.as_u64()?).ok()?;
    let (qs, qe) = (to_quilt_line(start), to_quilt_line(end));
    if qe <= qs {
        return None;
    }
    let kind = fr.get("kind").and_then(Value::as_str).map(|s| match s {
        "comment" => FoldingRangeKind::Comment,
        "imports" => FoldingRangeKind::Imports,
        _ => FoldingRangeKind::Region,
    });
    Some(FoldingRange {
        start_line: qs,
        end_line: qe,
        kind,
        ..Default::default()
    })
}

/// Client capabilities we present to the downstream server.
fn downstream_capabilities() -> Value {
    json!({
        "textDocument": {
            "synchronization": {"dynamicRegistration": false, "didSave": false},
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "definition": {"linkSupport": true},
            "completion": {"completionItem": {"snippetSupport": false}},
            "publishDiagnostics": {"relatedInformation": true},
            // Pull diagnostics (wgsl-analyzer uses this model, not push).
            "diagnostic": {"dynamicRegistration": false, "relatedDocumentSupport": false},
        },
        "workspace": {"configuration": true, "workspaceFolders": false},
        "window": {"workDoneProgress": false},
    })
}

pub struct Backend {
    inner: Arc<Inner>,
}

struct Inner {
    client: Client,
    encoding: OnceLock<Encoding>,
    /// quilt URI → analyzed document.
    docs: DashMap<Url, Document>,
    /// quilt URI → ground projection.
    projections: DashMap<Url, Projection>,
    /// virtual URI (string) → quilt URI.
    virt_to_quilt: DashMap<String, Url>,
    /// quilt URI → latest downstream diagnostics (already in quilt coords).
    child_diags: DashMap<Url, Vec<Diagnostic>>,
    /// workspace root → running downstream server for that workspace. Each
    /// Cargo workspace (determined by `find_root`) gets its own rust-analyzer
    /// so that files from `nanobots/` and `Quilt2/` resolve their crate graphs
    /// independently. `DashMap` allows concurrent lock-free reads on the hot path.
    workspaces: DashMap<PathBuf, Arc<ChildServer>>,
    /// One mutex per workspace root, held only during spawn + initialize. This
    /// prevents concurrent requests (semtok, folding, hover all fire at once on
    /// file open) from each racing to spawn their own rust-analyzer for the same
    /// workspace — which wastes memory, kills two RA processes mid-initialization,
    /// and can corrupt the shared `target/.rust-analyzer/` cache.
    workspace_locks: DashMap<PathBuf, Arc<Mutex<()>>>,
    /// The downstream semantic-token legend, captured at child init and
    /// re-advertised to the editor so token indices line up. Extended (never
    /// reordered) with the tree-sitter fragment highlighter's token types.
    semantic_legend: OnceLock<Value>,
    /// Token-type name → index into the advertised legend, for tokens the
    /// server generates itself for embedded fragments (see [`crate::tshl`]).
    /// Set together with `semantic_legend`.
    ts_token_index: OnceLock<std::collections::HashMap<&'static str, u32>>,

    /* ---- embedded target languages (per-fragment, e.g. WGSL) ---- */
    /// quilt URI → its embedded-language fragments (each a standalone quote).
    embedded_frags: DashMap<Url, Vec<EmbeddedFragment>>,
    /// fragment virtual URI → owning quilt URI (for routing diagnostics back).
    embedded_virt_to_quilt: DashMap<String, Url>,
    /// fragment virtual URI → latest diagnostics in quilt coords.
    embedded_diags: DashMap<String, Vec<Diagnostic>>,
    /// fragment virtual URI → its `languageId`, for the set currently opened to
    /// an embedded server (so we can `didChange` vs `didOpen`).
    embedded_opened: DashMap<String, &'static str>,
    /// `languageId` → its single running downstream server (e.g. wgsl-analyzer).
    /// Each embedded server is standalone and hosts every fragment of its
    /// language across all files.
    embedded: DashMap<String, Arc<ChildServer>>,
    /// One spawn lock per embedded `languageId`.
    embedded_locks: DashMap<String, Arc<Mutex<()>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                encoding: OnceLock::new(),
                docs: DashMap::new(),
                projections: DashMap::new(),
                virt_to_quilt: DashMap::new(),
                child_diags: DashMap::new(),
                workspaces: DashMap::new(),
                workspace_locks: DashMap::new(),
                semantic_legend: OnceLock::new(),
                ts_token_index: OnceLock::new(),
                embedded_frags: DashMap::new(),
                embedded_virt_to_quilt: DashMap::new(),
                embedded_diags: DashMap::new(),
                embedded_opened: DashMap::new(),
                embedded: DashMap::new(),
                embedded_locks: DashMap::new(),
            }),
        }
    }
}

/// Where `uri`'s downstream server is rooted and keyed, with its init options.
/// A Cargo project is keyed by its root dir and shared by every file under it.
/// A workspace-orphan script (`find_root` returned a dir with no manifest) is
/// analyzed as a *detached file*: keyed by the file itself so it gets its own
/// analyzer, and named in `detachedFiles` so semantic features still respond.
struct WorkspaceTarget {
    /// Directory the downstream server is spawned in (its root).
    root: PathBuf,
    /// Key into `workspaces` / `workspace_locks`.
    key: PathBuf,
    /// Downstream `initialize` options (carries `detachedFiles` when detached).
    init_options: Value,
}

fn workspace_target(uri: &Url) -> Option<WorkspaceTarget> {
    let lang = ground_lang(uri)?;
    let meta = meta_adapter(&lang)?;
    let file = uri.to_file_path().ok()?;
    let root = meta.find_root(&file)?;
    // A detached file (workspace orphan) gets its own analyzer named in the init
    // options; a project root is shared by every file beneath it.
    let detached = meta.is_detached_root(&root);
    // The downstream server sees the *de-quilted* overlay path, not the `.quilt`.
    let overlay = file
        .to_str()
        .and_then(|s| s.strip_suffix(".quilt"))
        .map(PathBuf::from);
    let init_options = overlay
        .as_deref()
        .map_or_else(|| json!({}), |p| meta.server_init_options(p, detached));
    let key = if detached { file } else { root.clone() };
    Some(WorkspaceTarget {
        root,
        key,
        init_options,
    })
}

/// One embedded target-language quote (e.g. a `wgsl↖…↗` fragment) opened as its
/// own standalone document to a per-language downstream server (wgsl-analyzer).
struct EmbeddedFragment {
    /// Downstream `languageId` (e.g. `"wgsl"`), also the key into `embedded`.
    lang: &'static str,
    /// Quilt byte range of the quote body, used to route a cursor to it.
    quilt_range: Range<usize>,
    /// This fragment's standalone virtual document + map back to quilt coords.
    proj: Projection,
    /// Synthetic file URI this fragment is opened under to the embedded server.
    virt_uri: String,
}

/// Look up the running server for `uri`'s workspace without spawning.
fn workspace_server_for(
    uri: &Url,
    workspaces: &DashMap<PathBuf, Arc<ChildServer>>,
) -> Option<Arc<ChildServer>> {
    let key = workspace_target(uri)?.key;
    workspaces.get(&key).map(|r| r.value().clone())
}

impl Inner {
    fn enc(&self) -> Encoding {
        self.encoding.get().copied().unwrap_or_default()
    }

    /// Quilt-syntax diagnostics for a document.
    fn syntax_diags(&self, doc: &Document) -> Vec<Diagnostic> {
        let enc = self.enc();
        doc.errors
            .iter()
            .map(|e| Diagnostic {
                range: doc.line_index.range(&doc.text, e.range.clone(), enc),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("quilt".to_string()),
                message: e.message.clone(),
                ..Default::default()
            })
            .collect()
    }

    /// Re-analyze text, store it, publish diagnostics, and sync to the child.
    async fn ingest(self: &Arc<Self>, uri: Url, text: String, version: i32) {
        let doc = Document::new(&uri, text, version);
        // Project only if the ground language is a host; otherwise quilt-only.
        if let Some(key) = doc.ground.as_deref() {
            if let (Some(meta), Some(lang)) = (meta_adapter(key), language_adapter(key)) {
                let chain = crate::document::chain_refs(&doc.chain);
                let proj = project(&doc.text, meta, lang, &chain);
                self.projections.insert(uri.clone(), proj);
            }
        }
        // Embedded target-language fragments (e.g. WGSL → wgsl-analyzer), each a
        // standalone quote opened to its own per-language server.
        self.build_embedded(&uri, &doc);
        // When quilt structure is broken the projection is unreliable; clear
        // any stale downstream diagnostics immediately so old rust-analyzer
        // noise doesn't linger while the user fixes the bracket.
        if !doc.errors.is_empty() {
            self.child_diags.insert(uri.clone(), Vec::new());
            if let Some(frags) = self.embedded_frags.get(&uri) {
                for f in frags.iter() {
                    self.embedded_diags.insert(f.virt_uri.clone(), Vec::new());
                }
            }
        }
        self.docs.insert(uri.clone(), doc);

        self.publish_combined(&uri).await;
        self.child_sync(&uri).await;
        self.embedded_sync(&uri).await;
    }

    /// Publish quilt-syntax diagnostics merged with the latest downstream ones.
    async fn publish_combined(&self, uri: &Url) {
        let (diags, version) = {
            let Some(doc) = self.docs.get(uri) else {
                return;
            };
            let mut diags = self.syntax_diags(&doc);
            if let Some(cd) = self.child_diags.get(uri) {
                diags.extend(cd.clone());
            }
            // Embedded-language (e.g. WGSL) diagnostics, already in quilt coords.
            if let Some(frags) = self.embedded_frags.get(uri) {
                for f in frags.iter() {
                    if let Some(ed) = self.embedded_diags.get(&f.virt_uri) {
                        diags.extend(ed.clone());
                    }
                }
            }
            (diags, doc.version)
        };
        self.client
            .publish_diagnostics(uri.clone(), diags, Some(version))
            .await;
    }

    /// Mirror a Rust document into the downstream server (didOpen first time,
    /// didChange afterwards). No-op for non-Rust or non-file documents.
    async fn child_sync(self: &Arc<Self>, uri: &Url) {
        let (virt, text, version, language_id) = {
            let Some(doc) = self.docs.get(uri) else {
                return;
            };
            let Some(lang) = doc.ground.as_deref().and_then(language_adapter) else {
                return;
            };
            let Some(virt) = dequilt_uri(uri) else {
                return;
            };
            // Present only if the ground is a host (projection was built).
            let Some(proj) = self.projections.get(uri) else {
                return;
            };
            (virt, proj.text.clone(), doc.version, lang.language_id())
        };

        let Some(child) = self.ensure_workspace_child(uri).await else {
            return;
        };

        if self.virt_to_quilt.contains_key(&virt) {
            let _ = child
                .notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": {"uri": virt, "version": version},
                        "contentChanges": [{"text": text}],
                    }),
                )
                .await;
        } else {
            let _ = child
                .notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": virt, "languageId": language_id,
                            "version": version, "text": text,
                        }
                    }),
                )
                .await;
            self.virt_to_quilt.insert(virt, uri.clone());
        }
    }

    /// (Re)build the embedded target-language fragments for `doc` (e.g. each
    /// `wgsl↖…↗` quote as its own standalone document) and refresh their virtual
    /// URI → quilt mapping. Does not talk to any server (see [`Self::embedded_sync`]).
    fn build_embedded(&self, uri: &Url, doc: &Document) {
        // Drop the previous build's mappings + diagnostics for this document.
        if let Some((_, old)) = self.embedded_frags.remove(uri) {
            for f in old {
                self.embedded_virt_to_quilt.remove(&f.virt_uri);
                self.embedded_diags.remove(&f.virt_uri);
            }
        }
        let Some(base) = dequilt_uri(uri) else {
            return;
        };
        let ground_id = doc
            .ground
            .as_deref()
            .and_then(language_adapter)
            .map(|a| a.language_id());
        let chain = crate::document::chain_refs(&doc.chain);
        let mut frags = Vec::new();
        let mut n = 0usize;
        for lang in embedded_adapters() {
            // The ground language's own quotes ride the merged ground projection.
            if Some(lang.language_id()) == ground_id {
                continue;
            }
            for fd in project_fragments(&doc.text, lang, &chain) {
                let virt_uri = format!("{base}.q{n}.{}", lang.virtual_extension());
                n += 1;
                self.embedded_virt_to_quilt
                    .insert(virt_uri.clone(), uri.clone());
                frags.push(EmbeddedFragment {
                    lang: lang.language_id(),
                    quilt_range: fd.quilt_range,
                    proj: fd.proj,
                    virt_uri,
                });
            }
        }
        if !frags.is_empty() {
            self.embedded_frags.insert(uri.clone(), frags);
        }
    }

    /// Mirror a document's embedded fragments into their per-language servers:
    /// `didClose` fragments that vanished, `didOpen`/`didChange` the rest.
    async fn embedded_sync(self: &Arc<Self>, uri: &Url) {
        let Some(base) = dequilt_uri(uri) else {
            return;
        };
        let version = self.docs.get(uri).map_or(1, |d| d.version);
        let items: Vec<(String, &'static str, String)> = match self.embedded_frags.get(uri) {
            Some(frags) => frags
                .iter()
                .map(|f| (f.virt_uri.clone(), f.lang, f.proj.text.clone()))
                .collect(),
            None => Vec::new(),
        };

        // Close fragments that disappeared (e.g. a quote was deleted). Virtual
        // URIs are prefixed by this document's de-quilted base, scoping the sweep.
        let stale: Vec<(String, &'static str)> = self
            .embedded_opened
            .iter()
            .filter(|e| e.key().starts_with(&base) && !items.iter().any(|(u, _, _)| u == e.key()))
            .map(|e| (e.key().clone(), *e.value()))
            .collect();
        for (virt, lang) in stale {
            if let Some(child) = self.embedded.get(lang).map(|r| r.value().clone()) {
                let _ = child
                    .notify(
                        "textDocument/didClose",
                        json!({"textDocument": {"uri": virt}}),
                    )
                    .await;
            }
            self.embedded_opened.remove(&virt);
        }

        for (virt, lang, text) in items {
            let Some(child) = self.ensure_embedded_child(lang).await else {
                continue;
            };
            if self.embedded_opened.contains_key(&virt) {
                let _ = child
                    .notify(
                        "textDocument/didChange",
                        json!({
                            "textDocument": {"uri": virt, "version": version},
                            "contentChanges": [{"text": text}],
                        }),
                    )
                    .await;
            } else {
                let _ = child
                    .notify(
                        "textDocument/didOpen",
                        json!({
                            "textDocument": {
                                "uri": virt, "languageId": lang,
                                "version": version, "text": text,
                            }
                        }),
                    )
                    .await;
                self.embedded_opened.insert(virt, lang);
            }
        }

        // Pull fresh diagnostics now that the fragments are in sync.
        self.refresh_embedded_diagnostics(uri).await;
    }

    /// Lazily spawn + initialize the single downstream server for an embedded
    /// `languageId` (e.g. wgsl-analyzer). Standalone: no project root.
    async fn ensure_embedded_child(
        self: &Arc<Self>,
        lang: &'static str,
    ) -> Option<Arc<ChildServer>> {
        let adapter = embedded_adapters()
            .into_iter()
            .find(|a| a.language_id() == lang)?;
        let (program, args) = adapter.server_command()?;

        if let Some(child) = self.embedded.get(lang).map(|r| r.value().clone()) {
            return Some(child);
        }
        let spawn_lock = self
            .embedded_locks
            .entry(lang.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .value()
            .clone();
        let spawn_guard = spawn_lock.lock().await;
        if let Some(child) = self.embedded.get(lang).map(|r| r.value().clone()) {
            return Some(child);
        }

        tracing::info!("spawning embedded `{program}` for {lang}");
        let (server, rx) = match ChildServer::spawn(&program, &args, None) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("failed to spawn `{program}`: {e}");
                return None;
            }
        };
        if let Err(e) = server
            .initialize(None, downstream_capabilities(), json!({}))
            .await
        {
            tracing::warn!("embedded `{lang}` initialize failed: {e}");
            return None;
        }
        self.embedded.insert(lang.to_string(), server.clone());
        drop(spawn_guard);
        tokio::spawn(self.clone().consume_embedded(rx, server.clone(), lang));
        Some(server)
    }

    /// Drain an embedded server's notifications (chiefly diagnostics). On exit,
    /// clear its entry (guarded by `ptr_eq`) so the next request respawns it.
    async fn consume_embedded(
        self: Arc<Self>,
        mut rx: mpsc::UnboundedReceiver<ChildNotification>,
        server_ref: Arc<ChildServer>,
        lang: &'static str,
    ) {
        while let Some(n) = rx.recv().await {
            if n.method == "textDocument/publishDiagnostics" {
                self.on_embedded_diagnostics(n.params).await;
            }
        }
        let is_current = self
            .embedded
            .get(lang)
            .is_some_and(|s| Arc::ptr_eq(s.value(), &server_ref));
        if is_current {
            self.embedded.remove(lang);
            self.embedded_opened.retain(|_, v| *v != lang);
            tracing::info!("embedded `{lang}` exited; cleared for respawn");
        }
    }

    /// Pull diagnostics for each of `uri`'s embedded fragments (wgsl-analyzer uses
    /// the **pull** model — `textDocument/diagnostic` — not push), remap them to
    /// quilt coordinates, and republish. Called after the fragments are synced.
    async fn refresh_embedded_diagnostics(self: &Arc<Self>, uri: &Url) {
        // The projection is unreliable while quilt brackets are broken.
        if self.docs.get(uri).is_some_and(|d| !d.errors.is_empty()) {
            return;
        }
        let items: Vec<(String, &'static str)> = match self.embedded_frags.get(uri) {
            Some(frags) => frags.iter().map(|f| (f.virt_uri.clone(), f.lang)).collect(),
            None => return,
        };
        let mut changed = false;
        for (virt, lang) in items {
            let Some(child) = self.embedded.get(lang).map(|r| r.value().clone()) else {
                continue;
            };
            let Ok(report) = child
                .request(
                    "textDocument/diagnostic",
                    json!({"textDocument": {"uri": virt}}),
                )
                .await
            else {
                continue;
            };
            let raw = report
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(diags) = self.translate_embedded_diags(uri, &virt, raw) {
                self.embedded_diags.insert(virt, diags);
                changed = true;
            }
        }
        if changed {
            self.publish_combined(uri).await;
        }
    }

    /// Some embedded servers may push diagnostics; remap and merge those too.
    async fn on_embedded_diagnostics(&self, params: Value) {
        let Some(virt) = params.get("uri").and_then(Value::as_str) else {
            return;
        };
        let Some(quilt_uri) = self.embedded_virt_to_quilt.get(virt).map(|r| r.clone()) else {
            return;
        };
        if self
            .docs
            .get(&quilt_uri)
            .is_some_and(|d| !d.errors.is_empty())
        {
            return;
        }
        let raw = params
            .get("diagnostics")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(diags) = self.translate_embedded_diags(&quilt_uri, virt, raw) {
            self.embedded_diags.insert(virt.to_string(), diags);
            self.publish_combined(&quilt_uri).await;
        }
    }

    /// Remap a fragment's raw downstream diagnostics into quilt coordinates,
    /// dropping any that fall on a masked splice placeholder.
    fn translate_embedded_diags(
        &self,
        quilt_uri: &Url,
        virt: &str,
        raw: Vec<Value>,
    ) -> Option<Vec<Diagnostic>> {
        let enc = self.enc();
        let doc = self.docs.get(quilt_uri)?;
        let frags = self.embedded_frags.get(quilt_uri)?;
        let frag = frags.iter().find(|f| f.virt_uri == virt)?;
        let mut out = Vec::with_capacity(raw.len());
        for d in raw {
            let Ok(mut diag) = serde_json::from_value::<Diagnostic>(d) else {
                continue;
            };
            if frag.proj.is_synthetic(enc, diag.range) {
                continue;
            }
            diag.range = frag
                .proj
                .to_quilt_range(&doc.text, &doc.line_index, enc, diag.range);
            diag.related_information = None;
            if diag.source.is_none() {
                diag.source = Some(frag.lang.to_string());
            }
            out.push(diag);
        }
        Some(out)
    }

    /// Tear down a document, closing its overlay downstream.
    async fn close(self: &Arc<Self>, uri: Url) {
        if let Some(virt) = dequilt_uri(&uri) {
            // Find the server for this file's workspace without spawning.
            let server = workspace_server_for(&uri, &self.workspaces);
            if let Some(child) = server {
                let _ = child
                    .notify(
                        "textDocument/didClose",
                        json!({"textDocument": {"uri": virt}}),
                    )
                    .await;
            }
            self.virt_to_quilt.remove(&virt);
        }
        // Close any embedded fragments (e.g. WGSL) to their per-language servers.
        if let Some((_, frags)) = self.embedded_frags.remove(&uri) {
            for f in &frags {
                if let Some(child) = self.embedded.get(f.lang).map(|r| r.value().clone()) {
                    let _ = child
                        .notify(
                            "textDocument/didClose",
                            json!({"textDocument": {"uri": f.virt_uri}}),
                        )
                        .await;
                }
                self.embedded_virt_to_quilt.remove(&f.virt_uri);
                self.embedded_diags.remove(&f.virt_uri);
                self.embedded_opened.remove(&f.virt_uri);
            }
        }
        self.docs.remove(&uri);
        self.projections.remove(&uri);
        self.child_diags.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    /// Lazily spawn + initialize a downstream server for `for_uri`'s workspace.
    /// Returns `None` if it can't be started. Each distinct workspace root gets
    /// its own server, so `nanobots/` and `Quilt2/` resolve independently.
    ///
    /// A per-workspace lock (`workspace_locks`) serializes concurrent spawn
    /// attempts for the same workspace root. Without it, VS Code's burst of
    /// simultaneous requests on file-open (semtok, folding, symbols, didOpen) all
    /// race to spawn rust-analyzer: two extra processes start, are killed, and can
    /// corrupt the shared `target/.rust-analyzer/` cache mid-write.
    async fn ensure_workspace_child(self: &Arc<Self>, for_uri: &Url) -> Option<Arc<ChildServer>> {
        // Resolve adapters and the workspace/detached-file target.
        let lang = ground_lang(for_uri)?;
        let (program, args) = language_adapter(&lang)?.server_command()?;
        let target = workspace_target(for_uri)?;

        // Fast path (no lock): server already running for this key.
        if let Some(child) = self.workspaces.get(&target.key).map(|r| r.value().clone()) {
            return Some(child);
        }

        // Slow path: we may need to spawn. Hold a per-key lock so only one task
        // spawns at a time; others wait here and find the server on their second
        // check below.
        let spawn_lock = self
            .workspace_locks
            .entry(target.key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .value()
            .clone();
        let spawn_guard = spawn_lock.lock().await;

        // Second check: another task may have spawned while we were waiting.
        if let Some(child) = self.workspaces.get(&target.key).map(|r| r.value().clone()) {
            return Some(child);
        }

        let root_uri = Url::from_directory_path(&target.root)
            .ok()
            .map(|u| u.to_string());
        tracing::info!(
            "spawning downstream `{program}` (cwd: {:?}, key: {:?})",
            target.root,
            target.key
        );
        let (server, rx) = match ChildServer::spawn(&program, &args, Some(target.root.as_path())) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("failed to spawn `{program}`: {e}");
                return None;
            }
        };
        let init = match server
            .initialize(
                root_uri.as_deref(),
                downstream_capabilities(),
                target.init_options,
            )
            .await
        {
            Ok(init) => init,
            Err(e) => {
                tracing::warn!("downstream initialize failed: {e}");
                return None;
            }
        };

        self.workspaces.insert(target.key.clone(), server.clone());
        // Release spawn lock before starting background tasks.
        drop(spawn_guard);

        tokio::spawn(
            self.clone()
                .consume(rx, server.clone(), Some(target.key.clone())),
        );

        // Register semantic tokens off the request path: it awaits a reply from
        // the editor and must not block document sync or other requests.
        let this = self.clone();
        tokio::spawn(async move {
            this.register_semantic_tokens(&init).await;
        });
        Some(server)
    }

    /// If the downstream server provides semantic tokens, capture its legend and
    /// dynamically register the same capability with the editor so we can proxy
    /// `textDocument/semanticTokens/full`.
    async fn register_semantic_tokens(&self, init: &Value) {
        let legend = &init["capabilities"]["semanticTokensProvider"]["legend"];
        if !legend.is_object() {
            return;
        }
        // Extend the downstream legend with the tree-sitter fragment
        // highlighter's token types: appended, never reordered, so downstream
        // token indices stay valid as-is while fragment tokens resolve by name.
        let mut legend = legend.clone();
        let mut type_index = std::collections::HashMap::new();
        if let Some(types) = legend.get_mut("tokenTypes").and_then(Value::as_array_mut) {
            for (i, t) in types.iter().enumerate() {
                if let (Some(s), Ok(i)) = (t.as_str(), u32::try_from(i)) {
                    if let Some(name) = crate::tshl::TOKEN_TYPES.iter().find(|n| **n == s) {
                        type_index.insert(*name, i);
                    }
                }
            }
            for name in crate::tshl::TOKEN_TYPES {
                if !type_index.contains_key(name) {
                    if let Ok(i) = u32::try_from(types.len()) {
                        type_index.insert(*name, i);
                        types.push(json!(name));
                    }
                }
            }
        }
        let is_first = self.semantic_legend.set(legend.clone()).is_ok();
        if is_first {
            // First workspace to initialize: register the capability with the
            // editor. The fragment-token index must match the legend actually
            // registered, so it is set only by the same winner.
            let _ = self.ts_token_index.set(type_index);
            let registration = Registration {
                id: "quilt-semantic-tokens".to_string(),
                method: "textDocument/semanticTokens".to_string(),
                register_options: Some(json!({
                    "documentSelector": [{"language": "quilt"}],
                    "legend": legend,
                    "full": true,
                })),
            };
            if let Err(e) = self.client.register_capability(vec![registration]).await {
                tracing::warn!("failed to register semantic tokens: {e}");
                return;
            }
        }
        // Always send a refresh after registering (or if already registered by an
        // earlier workspace). This ensures VS Code re-requests for all open files
        // even if `workspace/semanticTokens/refresh` from the downstream server
        // arrived before our registration round-trip completed (common when RA
        // indexes a small project faster than the editor acks the registration),
        // or when a second workspace finishes indexing and should trigger a re-pull.
        let _ = self.client.semantic_tokens_refresh().await;
    }

    /// Drain downstream notifications for the server at `workspace`. When the
    /// channel closes (child exited) clear the workspace entry — guarding
    /// against erasing a concurrently-spawned replacement with `Arc::ptr_eq`.
    async fn consume(
        self: Arc<Self>,
        mut rx: mpsc::UnboundedReceiver<ChildNotification>,
        server_ref: Arc<crate::child::ChildServer>,
        workspace: Option<std::path::PathBuf>,
    ) {
        while let Some(n) = rx.recv().await {
            match n.method.as_str() {
                "textDocument/publishDiagnostics" => self.on_child_diagnostics(n.params).await,
                // rust-analyzer asks the client to re-pull once analysis is
                // ready; relay so the editor re-requests (initial results are
                // often empty while it loads).
                "workspace/semanticTokens/refresh" => {
                    let _ = self.client.semantic_tokens_refresh().await;
                }
                "window/logMessage" | "window/showMessage" => {
                    tracing::debug!("downstream: {}", n.params);
                }
                _ => {}
            }
        }
        // Channel closed: downstream exited. Clear the entry so the next
        // request triggers a fresh spawn. Guard with ptr_eq so we don't erase
        // a replacement server that was already inserted.
        if let Some(root) = workspace {
            let is_current = self
                .workspaces
                .get(&root)
                .is_some_and(|s| Arc::ptr_eq(s.value(), &server_ref));
            if is_current {
                self.workspaces.remove(&root);
                tracing::info!("downstream server exited; cleared workspace {root:?} for respawn");
            }
        }
    }

    /// Remap and merge downstream diagnostics for a virtual document.
    async fn on_child_diagnostics(&self, params: Value) {
        let Some(virt) = params.get("uri").and_then(Value::as_str) else {
            return;
        };
        let Some(quilt_uri) = self.virt_to_quilt.get(virt).map(|r| r.clone()) else {
            return;
        };
        // A language whose ground projection uses lossy placeholders opts out of
        // diagnostics (e.g. Python: `()` placeholders mistype quote-consuming
        // lines, so pyright errors would be spurious).
        let publishes = self
            .docs
            .get(&quilt_uri)
            .and_then(|d| d.ground.clone())
            .and_then(|g| language_adapter(&g))
            .is_none_or(|a| a.publishes_diagnostics());
        if !publishes {
            return;
        }
        // Suppress downstream noise while quilt structure is broken: the
        // projection is garbled, so remapped diagnostics are meaningless and
        // would flood the file with spurious errors.
        if self
            .docs
            .get(&quilt_uri)
            .is_some_and(|d| !d.errors.is_empty())
        {
            return;
        }
        let enc = self.enc();

        let translated = {
            let Some(doc) = self.docs.get(&quilt_uri) else {
                return;
            };
            let Some(proj) = self.projections.get(&quilt_uri) else {
                return;
            };
            let raw = params
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut out = Vec::with_capacity(raw.len());
            for d in raw {
                let Ok(mut diag) = serde_json::from_value::<Diagnostic>(d) else {
                    continue;
                };
                // Drop diagnostics on placeholder text or inside appended quote
                // fragments (their wrapping makes diagnostics unreliable).
                if proj.is_synthetic(enc, diag.range) || proj.is_in_fragment(enc, diag.range) {
                    continue;
                }
                diag.range = proj.to_quilt_range(&doc.text, &doc.line_index, enc, diag.range);
                // Related info points into the virtual doc; omit rather than
                // show wrong positions (revisited in a later phase).
                diag.related_information = None;
                out.push(diag);
            }
            out
        };

        self.child_diags.insert(quilt_uri.clone(), translated);
        self.publish_combined(&quilt_uri).await;
    }

    /// Forward a position-based request and remap the result. Tries the ground
    /// projection first (Rust/Python → its server); if the position is not in the
    /// ground language (e.g. inside a `wgsl↖…↗` quote), falls through to the
    /// embedded fragment that contains it (WGSL → wgsl-analyzer).
    async fn forward_position(
        self: &Arc<Self>,
        method: &str,
        uri: &Url,
        pos: Position,
    ) -> Option<Value> {
        if let Some(v) = self.forward_ground(method, uri, pos).await {
            return Some(v);
        }
        self.forward_embedded(method, uri, pos).await
    }

    /// Forward to the ground (host) server. `None` when the doc has no host
    /// ground, the position is inside a quoted construct, or downstream errors.
    async fn forward_ground(
        self: &Arc<Self>,
        method: &str,
        uri: &Url,
        pos: Position,
    ) -> Option<Value> {
        let (text, line_index, proj, virt) = {
            let doc = self.docs.get(uri)?;
            if !is_host_ground(doc.ground.as_deref()) {
                return None;
            }
            let virt = dequilt_uri(uri)?;
            let proj = self.projections.get(uri)?.clone();
            (doc.text.clone(), doc.line_index.clone(), proj, virt)
        };

        let enc = self.enc();
        let vpos = proj.to_virtual(&text, &line_index, enc, pos)?;
        let child = self.ensure_workspace_child(uri).await?;

        let result = child
            .request(
                method,
                json!({"textDocument": {"uri": virt}, "position": vpos}),
            )
            .await
            .ok()?;
        if result.is_null() {
            return None;
        }

        let mapper = Mapper {
            enc,
            virt_uri: &virt,
            quilt_uri: uri.as_str(),
            quilt_text: &text,
            quilt_index: &line_index,
            proj: &proj,
        };
        Some(translate::translate_result(method, result, &mapper))
    }

    /// Forward to the embedded fragment containing `pos` (e.g. a WGSL quote →
    /// wgsl-analyzer), remapping the result back to quilt coordinates.
    async fn forward_embedded(
        self: &Arc<Self>,
        method: &str,
        uri: &Url,
        pos: Position,
    ) -> Option<Value> {
        let enc = self.enc();
        let (virt, lang, vpos, proj, text, line_index) = {
            let doc = self.docs.get(uri)?;
            let frags = self.embedded_frags.get(uri)?;
            let qoff = doc.line_index.offset(&doc.text, pos, enc);
            let frag = frags.iter().find(|f| f.quilt_range.contains(&qoff))?;
            let vpos = frag.proj.to_virtual(&doc.text, &doc.line_index, enc, pos)?;
            (
                frag.virt_uri.clone(),
                frag.lang,
                vpos,
                frag.proj.clone(),
                doc.text.clone(),
                doc.line_index.clone(),
            )
        };

        let child = self.ensure_embedded_child(lang).await?;
        let result = child
            .request(
                method,
                json!({"textDocument": {"uri": virt}, "position": vpos}),
            )
            .await
            .ok()?;
        if result.is_null() {
            return None;
        }

        let mapper = Mapper {
            enc,
            virt_uri: &virt,
            quilt_uri: uri.as_str(),
            quilt_text: &text,
            quilt_index: &line_index,
            proj: &proj,
        };
        Some(translate::translate_result(method, result, &mapper))
    }

    /// Document symbols from the downstream server, remapped to quilt coords and
    /// with the synthetic `_quilt_qN` wrapper functions filtered out.
    async fn document_symbols(self: &Arc<Self>, uri: &Url) -> Option<Value> {
        let (text, line_index, proj, virt) = {
            let doc = self.docs.get(uri)?;
            if !is_host_ground(doc.ground.as_deref()) {
                return None;
            }
            let virt = dequilt_uri(uri)?;
            let proj = self.projections.get(uri)?.clone();
            (doc.text.clone(), doc.line_index.clone(), proj, virt)
        };

        let child = self.ensure_workspace_child(uri).await?;
        let result = child
            .request(
                "textDocument/documentSymbol",
                json!({"textDocument": {"uri": virt}}),
            )
            .await
            .ok()?;
        if result.is_null() {
            return None;
        }

        let mapper = Mapper {
            enc: self.enc(),
            virt_uri: &virt,
            quilt_uri: uri.as_str(),
            quilt_text: &text,
            quilt_index: &line_index,
            proj: &proj,
        };
        Some(translate::translate_result(
            "textDocument/documentSymbol",
            result,
            &mapper,
        ))
    }

    /// Whole-document semantic tokens: forward to the downstream server (which
    /// sees the ground projection *and* the appended quote fragments), remap
    /// every token back to quilt coordinates, and merge in tree-sitter tokens
    /// for embedded fragments (whose own servers may provide none — wgsl-analyzer
    /// advertises no semantic tokens).
    async fn semantic_tokens(self: &Arc<Self>, uri: &Url) -> Option<Vec<u32>> {
        let (text, line_index, proj, virt, frags) = {
            let doc = self.docs.get(uri)?;
            if !is_host_ground(doc.ground.as_deref()) {
                return None;
            }
            let virt = dequilt_uri(uri)?;
            let proj = self.projections.get(uri)?.clone();
            // Embedded fragments highlighted in-process: language + projection.
            let frags: Vec<(&'static str, Projection)> = self
                .embedded_frags
                .get(uri)
                .map(|fs| fs.iter().map(|f| (f.lang, f.proj.clone())).collect())
                .unwrap_or_default();
            (doc.text.clone(), doc.line_index.clone(), proj, virt, frags)
        };

        let child = self.ensure_workspace_child(uri).await?;
        let result = child
            .request(
                "textDocument/semanticTokens/full",
                json!({"textDocument": {"uri": virt}}),
            )
            .await
            .ok()?;

        let data: Vec<u32> = result
            .get("data")?
            .as_array()?
            .iter()
            .filter_map(|v| u32::try_from(v.as_u64()?).ok())
            .collect();

        let enc = self.enc();
        let mut toks = crate::semtok::remap(&data, &proj, &text, &line_index, enc);
        if let Some(type_index) = self.ts_token_index.get() {
            for (lang, fproj) in &frags {
                if let Some(hl) = crate::tshl::highlighter(lang) {
                    toks.extend(crate::tshl::fragment_tokens(
                        hl,
                        fproj,
                        &text,
                        &line_index,
                        enc,
                        type_index,
                    ));
                }
            }
        }
        Some(crate::semtok::encode(toks))
    }

    /// Folding ranges: the quilt regions, plus the downstream server's ground
    /// folds remapped into quilt coordinates.
    async fn folding(self: &Arc<Self>, uri: &Url) -> Vec<FoldingRange> {
        let enc = self.enc();
        let mut out = Vec::new();

        let forward = {
            let Some(doc) = self.docs.get(uri) else {
                return out;
            };
            collect_region_folds(&doc.region, &doc.text, &doc.line_index, enc, &mut out);
            if is_host_ground(doc.ground.as_deref()) {
                match (dequilt_uri(uri), self.projections.get(uri)) {
                    (Some(virt), Some(proj)) => {
                        Some((doc.text.clone(), doc.line_index.clone(), virt, proj.clone()))
                    }
                    _ => None,
                }
            } else {
                None
            }
        };

        if let Some((text, line_index, virt, proj)) = forward {
            if let Some(child) = self.ensure_workspace_child(uri).await {
                if let Ok(res) = child
                    .request(
                        "textDocument/foldingRange",
                        json!({"textDocument": {"uri": virt}}),
                    )
                    .await
                {
                    if let Some(arr) = res.as_array() {
                        for fr in arr {
                            if let Some(m) = remap_folding(fr, &proj, &text, &line_index, enc) {
                                out.push(m);
                            }
                        }
                    }
                }
            }
        }

        out
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let offered = params
            .capabilities
            .general
            .as_ref()
            .and_then(|g| g.position_encodings.as_deref());
        let enc = Encoding::negotiate(offered);
        let _ = self.inner.encoding.set(enc);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(enc.as_kind()),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions::default()),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "quilt-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        tracing::info!("quilt-lsp initialized (encoding: {:?})", self.inner.enc());
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.inner.ingest(doc.uri, doc.text, doc.version).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().next_back() else {
            return;
        };
        self.inner
            .ingest(
                params.text_document.uri,
                change.text,
                params.text_document.version,
            )
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.inner.close(params.text_document.uri).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let p = params.text_document_position_params;
        let v = self
            .inner
            .forward_position("textDocument/hover", &p.text_document.uri, p.position)
            .await;
        Ok(v.and_then(|v| serde_json::from_value(v).ok()))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let p = params.text_document_position_params;
        let v = self
            .inner
            .forward_position("textDocument/definition", &p.text_document.uri, p.position)
            .await;
        Ok(v.and_then(|v| serde_json::from_value(v).ok()))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let p = params.text_document_position;
        let v = self
            .inner
            .forward_position("textDocument/completion", &p.text_document.uri, p.position)
            .await;
        Ok(v.and_then(|v| serde_json::from_value(v).ok()))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        Ok(Some(self.inner.folding(&params.text_document.uri).await))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let v = self.inner.document_symbols(&params.text_document.uri).await;
        Ok(v.and_then(|v| serde_json::from_value(v).ok()))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some(data) = self.inner.semantic_tokens(&params.text_document.uri).await else {
            return Ok(None);
        };
        let tokens = data
            .chunks_exact(5)
            .map(|g| SemanticToken {
                delta_line: g[0],
                delta_start: g[1],
                length: g[2],
                token_type: g[3],
                token_modifiers_bitset: g[4],
            })
            .collect();
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        let url = Url::parse("file:///x/foo.rs.quilt").unwrap();
        Document::new(&url, text.to_string(), 1)
    }

    #[test]
    fn region_fold_for_multiline_quote() {
        let d = doc("fn main() {\n    let p = ↖{\n        a\n    }↗;\n}\n");
        let mut out = Vec::new();
        collect_region_folds(&d.region, &d.text, &d.line_index, Encoding::Utf16, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_line, 1); // `↖{`
        assert_eq!(out[0].end_line, 3); // `}↗`
    }

    #[test]
    fn no_region_fold_for_single_line_quote() {
        let d = doc("let x = ↖1↗;\n");
        let mut out = Vec::new();
        collect_region_folds(&d.region, &d.text, &d.line_index, Encoding::Utf16, &mut out);
        assert!(out.is_empty());
    }
}
