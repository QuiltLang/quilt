//! Translating downstream LSP *results* from virtual-document coordinates back
//! into `.quilt` coordinates.
//!
//! Results carry ranges (and sometimes URIs) that point into the virtual
//! document we sent the child. Ranges in the *request* document (hover range,
//! completion edits, a definition link's `originSelectionRange`) always belong
//! to our virtual doc and are remapped. A *target* location is only ours when
//! its URI equals the virtual URI; targets in other real files pass through
//! untouched.

use crate::lineindex::{Encoding, LineIndex};
use crate::projection::Projection;
use serde_json::{json, Value};
use tower_lsp::lsp_types::Range;

pub struct Mapper<'a> {
    pub enc: Encoding,
    pub virt_uri: &'a str,
    pub quilt_uri: &'a str,
    pub quilt_text: &'a str,
    pub quilt_index: &'a LineIndex,
    pub proj: &'a Projection,
}

impl Mapper<'_> {
    fn map_range(&self, r: &mut Value) {
        if let Ok(range) = serde_json::from_value::<Range>(r.clone()) {
            let q = self
                .proj
                .to_quilt_range(self.quilt_text, self.quilt_index, self.enc, range);
            if let Ok(v) = serde_json::to_value(q) {
                *r = v;
            }
        }
    }

    fn map_field(&self, obj: &mut Value, key: &str) {
        if let Some(r) = obj.get_mut(key) {
            self.map_range(r);
        }
    }

    fn map_location(&self, loc: &mut Value) {
        if loc.get("uri").and_then(Value::as_str) == Some(self.virt_uri) {
            loc["uri"] = json!(self.quilt_uri);
            self.map_field(loc, "range");
        }
    }

    fn map_location_link(&self, link: &mut Value) {
        // The origin selection is in the request (virtual) document.
        self.map_field(link, "originSelectionRange");
        if link.get("targetUri").and_then(Value::as_str) == Some(self.virt_uri) {
            link["targetUri"] = json!(self.quilt_uri);
            self.map_field(link, "targetRange");
            self.map_field(link, "targetSelectionRange");
        }
    }

    fn map_location_like(&self, v: &mut Value) {
        if v.get("targetUri").is_some() {
            self.map_location_link(v);
        } else if v.get("uri").is_some() {
            self.map_location(v);
        }
    }

    fn map_locations(&self, v: &mut Value) {
        match v {
            Value::Array(arr) => {
                for el in arr {
                    self.map_location_like(el);
                }
            }
            Value::Object(_) => self.map_location_like(v),
            _ => {}
        }
    }

    /// Whether a symbol with this `range` should be dropped from results: it
    /// sits on placeholder text or inside an appended quote fragment (e.g. the
    /// synthetic `_quilt_qN` wrapper functions).
    fn should_drop(&self, range: &Value) -> bool {
        serde_json::from_value::<Range>(range.clone()).is_ok_and(|r| {
            self.proj.is_synthetic(self.enc, r) || self.proj.is_in_fragment(self.enc, r)
        })
    }

    fn map_symbol(&self, mut el: Value) -> Option<Value> {
        // SymbolInformation { location: { uri, range }, ... }
        if let Some(loc) = el.get("location") {
            if loc.get("range").is_some_and(|r| self.should_drop(r)) {
                return None;
            }
            if let Some(loc) = el.get_mut("location") {
                self.map_location(loc);
            }
            return Some(el);
        }
        // DocumentSymbol { range, selectionRange, children }
        if el.get("range").is_some_and(|r| self.should_drop(r)) {
            return None;
        }
        self.map_field(&mut el, "range");
        self.map_field(&mut el, "selectionRange");
        if let Some(children) = el.get_mut("children").and_then(Value::as_array_mut) {
            let kept: Vec<Value> = std::mem::take(children)
                .into_iter()
                .filter_map(|c| self.map_symbol(c))
                .collect();
            el["children"] = Value::Array(kept);
        }
        Some(el)
    }

    fn map_completion(&self, v: &mut Value) {
        let items = match v {
            Value::Array(arr) => Some(arr),
            Value::Object(o) => o.get_mut("items").and_then(Value::as_array_mut),
            _ => None,
        };
        if let Some(items) = items {
            for item in items {
                self.map_completion_item(item);
            }
        }
    }

    fn map_completion_item(&self, item: &mut Value) {
        if let Some(te) = item.get_mut("textEdit") {
            // Either TextEdit { range } or InsertReplaceEdit { insert, replace }.
            self.map_field(te, "range");
            self.map_field(te, "insert");
            self.map_field(te, "replace");
        }
        if let Some(edits) = item
            .get_mut("additionalTextEdits")
            .and_then(Value::as_array_mut)
        {
            for e in edits {
                self.map_field(e, "range");
            }
        }
    }
}

/// Remap a downstream result for `method` from virtual to quilt coordinates.
pub fn translate_result(method: &str, mut value: Value, m: &Mapper) -> Value {
    match method {
        "textDocument/hover" => {
            if value.is_object() {
                m.map_field(&mut value, "range");
            }
        }
        "textDocument/definition"
        | "textDocument/declaration"
        | "textDocument/typeDefinition"
        | "textDocument/implementation"
        | "textDocument/references" => m.map_locations(&mut value),
        "textDocument/completion" => m.map_completion(&mut value),
        "textDocument/documentSymbol" => {
            if let Value::Array(arr) = value {
                value = Value::Array(arr.into_iter().filter_map(|el| m.map_symbol(el)).collect());
            }
        }
        _ => {}
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{language_adapter, meta_adapter};
    use crate::projection::project;

    fn project_ground(src: &str) -> Projection {
        project(
            src,
            meta_adapter("rs").unwrap(),
            language_adapter("rs").unwrap(),
            &["rs"],
        )
    }

    fn mapper_ctx() -> (String, LineIndex, Projection) {
        let src = "let x = ↖1↗;\nlet y = 0;\n";
        let proj = project_ground(src);
        let li = LineIndex::new(src);
        (src.to_string(), li, proj)
    }

    #[test]
    fn hover_range_remapped() {
        let (text, li, proj) = mapper_ctx();
        let m = Mapper {
            enc: Encoding::Utf16,
            virt_uri: "file:///x/foo.rs",
            quilt_uri: "file:///x/foo.rs.quilt",
            quilt_text: &text,
            quilt_index: &li,
            proj: &proj,
        };
        // A hover whose range is line 1 (the `let y` line) in the virtual doc.
        let hover = json!({
            "contents": "stuff",
            "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 5}}
        });
        let out = translate_result("textDocument/hover", hover, &m);
        // Line 1 maps straight through (ground line, unaffected by the quote).
        assert_eq!(out["range"]["start"]["line"], 1);
        assert_eq!(out["range"]["start"]["character"], 4);
    }

    #[test]
    fn definition_in_other_file_passes_through() {
        let (text, li, proj) = mapper_ctx();
        let m = Mapper {
            enc: Encoding::Utf16,
            virt_uri: "file:///x/foo.rs",
            quilt_uri: "file:///x/foo.rs.quilt",
            quilt_text: &text,
            quilt_index: &li,
            proj: &proj,
        };
        let loc = json!({
            "uri": "file:///other/lib.rs",
            "range": {"start": {"line": 9, "character": 0}, "end": {"line": 9, "character": 3}}
        });
        let out = translate_result("textDocument/definition", loc.clone(), &m);
        assert_eq!(out, loc, "locations in other files are untouched");
    }

    #[test]
    fn document_symbols_drop_wrapper_fns() {
        // A file with a quote -> an appended `_quilt_q0` wrapper fragment.
        let src = "fn main() {}\nlet x = ↖1↗;\n";
        let proj = project_ground(src);
        let li = LineIndex::new(src);
        let m = Mapper {
            enc: Encoding::Utf16,
            virt_uri: "file:///x/foo.rs",
            quilt_uri: "file:///x/foo.rs.quilt",
            quilt_text: src,
            quilt_index: &li,
            proj: &proj,
        };

        // The wrapper symbol's range sits on the fragment's first line.
        let frag_line = proj
            .line_index
            .position(&proj.text, proj.fragment_ranges[0].start, Encoding::Utf16)
            .line;
        let rng = |l: u32, c0: u32, c1: u32| json!({"start": {"line": l, "character": c0}, "end": {"line": l, "character": c1}});
        let symbols = json!([
            {"name": "main", "kind": 12, "range": rng(0, 0, 12), "selectionRange": rng(0, 3, 7)},
            {"name": "_quilt_q0", "kind": 12,
             "range": rng(frag_line, 0, 5), "selectionRange": rng(frag_line, 3, 5)},
        ]);

        let out = translate_result("textDocument/documentSymbol", symbols, &m);
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 1, "wrapper fn should be filtered out");
        assert_eq!(arr[0]["name"], "main");
    }

    #[test]
    fn definition_in_our_file_remaps_uri() {
        let (text, li, proj) = mapper_ctx();
        let m = Mapper {
            enc: Encoding::Utf16,
            virt_uri: "file:///x/foo.rs",
            quilt_uri: "file:///x/foo.rs.quilt",
            quilt_text: &text,
            quilt_index: &li,
            proj: &proj,
        };
        let loc = json!({
            "uri": "file:///x/foo.rs",
            "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 5}}
        });
        let out = translate_result("textDocument/definition", loc, &m);
        assert_eq!(out["uri"], "file:///x/foo.rs.quilt");
        assert_eq!(out["range"]["start"]["line"], 1);
    }
}
