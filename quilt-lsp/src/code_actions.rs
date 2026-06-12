//! Quilt-specific code actions.
//!
//! All three actions operate purely on the quilt CST (the [`Region`] tree from
//! [`Document::region`]); no downstream server cooperation is needed.
//!
//! * **Wrap in `↖…↗`** — available whenever the editor selection is non-empty;
//!   inserts the bracket glyphs around the selection.
//! * **Inline unquote** — when the cursor is inside a stage-0 `↙…↘` (an
//!   unquote that's a direct child of a stage-1 quote), extracts the unquoted
//!   expression to a local binding before the enclosing quote and replaces it
//!   with `↙<name>↘`.
//! * **Extract to named fragment** — when the cursor is inside a stage-1
//!   `↖…↗`, extracts the whole quote to a local binding before its line and
//!   replaces it with `↙<name>↘`.

use std::cmp::Reverse;
use std::collections::HashMap;

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Range, TextEdit, Url, WorkspaceEdit,
};

use crate::document::Document;
use crate::lineindex::Encoding;
use crate::regions::{Region, RegionKind, ARROW_LEN};

/// Compute all applicable code actions for the given selection range.
pub fn code_actions(
    uri: &Url,
    doc: &Document,
    enc: Encoding,
    range: Range,
) -> Vec<CodeActionOrCommand> {
    let mut out = Vec::new();

    let start_offset = doc.line_index.offset(&doc.text, range.start, enc);
    let end_offset = doc.line_index.offset(&doc.text, range.end, enc);

    // Action 1: wrap selection in ↖…↗
    if let Some(ca) = wrap_in_quotes(uri, doc, enc, range, start_offset, end_offset) {
        out.push(CodeActionOrCommand::CodeAction(ca));
    }

    // Walk the region tree once to find the deepest region(s) containing the
    // cursor.  We keep the two innermost levels so we can identify:
    //   - inline unquote: leaf = Unquote, parent = Quote at stage 1
    //   - extract fragment: leaf = Quote at stage 1
    let path = region_path(&doc.region, start_offset);

    // Action 2: inline unquote
    if let Some(ca) = inline_unquote(uri, doc, enc, start_offset, &path) {
        out.push(CodeActionOrCommand::CodeAction(ca));
    }

    // Action 3: extract to named fragment
    if let Some(ca) = extract_fragment(uri, doc, enc, start_offset, &path) {
        out.push(CodeActionOrCommand::CodeAction(ca));
    }

    out
}

// ---------------------------------------------------------------------------
// Action 1: wrap selection
// ---------------------------------------------------------------------------

fn wrap_in_quotes(
    uri: &Url,
    _doc: &Document,
    _enc: Encoding,
    range: Range,
    start: usize,
    end: usize,
) -> Option<CodeAction> {
    if start >= end {
        return None;
    }
    let edits = vec![
        TextEdit {
            range: Range {
                start: range.start,
                end: range.start,
            },
            new_text: "↖".to_string(),
        },
        TextEdit {
            range: Range {
                start: range.end,
                end: range.end,
            },
            new_text: "↗".to_string(),
        },
    ];
    Some(CodeAction {
        title: "Wrap in ↖…↗".to_string(),
        kind: Some(CodeActionKind::REFACTOR_REWRITE),
        edit: Some(workspace_edit(uri, edits)),
        is_preferred: Some(true),
        diagnostics: None,
        command: None,
        disabled: None,
        data: None,
    })
}

// ---------------------------------------------------------------------------
// Action 2: inline unquote
// ---------------------------------------------------------------------------

fn inline_unquote(
    uri: &Url,
    doc: &Document,
    enc: Encoding,
    _offset: usize,
    path: &[&Region],
) -> Option<CodeAction> {
    // Need at least root + quote + unquote.
    if path.len() < 3 {
        return None;
    }
    let leaf = *path.last()?;
    let parent = path[path.len() - 2];

    if leaf.kind != RegionKind::Unquote || leaf.stage != 0 {
        return None;
    }
    if parent.kind != RegionKind::Quote || parent.stage != 1 {
        return None;
    }

    let unquote_body = &doc.text[leaf.body.clone()];
    let name = suggested_name(unquote_body, "_u");

    // Full unquote span: anno↙body↘ — anno on unquotes doesn't change the
    // language but the text still contains it.
    let unquote_start = leaf.body.start.saturating_sub(leaf.anno.len() + ARROW_LEN);
    let unquote_end = leaf.body.end + ARROW_LEN;

    // Insertion point: beginning of the line that contains the opening ↖ of
    // the parent quote.
    let quote_open = parent
        .body
        .start
        .saturating_sub(parent.anno.len() + ARROW_LEN);
    let insert_byte = line_start_before(&doc.text, quote_open);
    let insert_pos = doc.line_index.position(&doc.text, insert_byte, enc);

    let binding_line = ground_binding(doc, &name, unquote_body);
    let new_unquote = format!("↙{name}↘");

    // Build edits — apply last-to-first so earlier offsets are stable.
    let replace_range = Range {
        start: doc.line_index.position(&doc.text, unquote_start, enc),
        end: doc.line_index.position(&doc.text, unquote_end, enc),
    };
    // Sort descending by start position so non-overlapping edits are safe.
    let mut sorted = vec![
        TextEdit {
            range: replace_range,
            new_text: new_unquote,
        },
        TextEdit {
            range: Range {
                start: insert_pos,
                end: insert_pos,
            },
            new_text: binding_line,
        },
    ];
    sorted.sort_by_key(|e| Reverse(e.range.start));

    Some(CodeAction {
        title: "Inline unquote as local binding".to_string(),
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        edit: Some(workspace_edit(uri, sorted)),
        is_preferred: None,
        diagnostics: None,
        command: None,
        disabled: None,
        data: None,
    })
}

// ---------------------------------------------------------------------------
// Action 3: extract to named fragment
// ---------------------------------------------------------------------------

fn extract_fragment(
    uri: &Url,
    doc: &Document,
    enc: Encoding,
    _offset: usize,
    path: &[&Region],
) -> Option<CodeAction> {
    // Find the innermost Quote at stage 1 in the path.
    let quote = path
        .iter()
        .rev()
        .find(|r| r.kind == RegionKind::Quote && r.stage == 1)?;

    let name = "_frag";

    // Full quote span: anno↖body↗
    let quote_start = quote
        .body
        .start
        .saturating_sub(quote.anno.len() + ARROW_LEN);
    let quote_end = quote.body.end + ARROW_LEN;
    let full_quote = &doc.text[quote_start..quote_end];

    // Insertion point: beginning of the line containing the ↖.
    let insert_byte = line_start_before(&doc.text, quote_start);
    let insert_pos = doc.line_index.position(&doc.text, insert_byte, enc);

    let binding_line = ground_binding(doc, name, full_quote);

    let replace_range = Range {
        start: doc.line_index.position(&doc.text, quote_start, enc),
        end: doc.line_index.position(&doc.text, quote_end, enc),
    };

    let mut edits = vec![
        TextEdit {
            range: replace_range,
            new_text: format!("↙{name}↘"),
        },
        TextEdit {
            range: Range {
                start: insert_pos,
                end: insert_pos,
            },
            new_text: binding_line,
        },
    ];
    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));

    Some(CodeAction {
        title: "Extract to named fragment".to_string(),
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        edit: Some(workspace_edit(uri, edits)),
        is_preferred: None,
        diagnostics: None,
        command: None,
        disabled: None,
        data: None,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk the region tree and return the path from root to the deepest region
/// whose body contains `offset`.
fn region_path(root: &Region, offset: usize) -> Vec<&Region> {
    let mut path = vec![root];
    descend(root, offset, &mut path);
    path
}

fn descend<'a>(region: &'a Region, offset: usize, path: &mut Vec<&'a Region>) {
    for child in &region.children {
        let child_start = child
            .body
            .start
            .saturating_sub(child.anno.len() + ARROW_LEN);
        let child_end = child.body.end + ARROW_LEN;
        if child_start <= offset && offset <= child_end {
            path.push(child);
            descend(child, offset, path);
            return;
        }
    }
}

/// Byte offset of the start of the line that contains `byte_offset`.
fn line_start_before(text: &str, byte_offset: usize) -> usize {
    let safe = byte_offset.min(text.len());
    text[..safe].rfind('\n').map_or(0, |i| i + 1)
}

/// Generate a local-variable binding line appropriate for the ground language.
fn ground_binding(doc: &Document, name: &str, rhs: &str) -> String {
    match doc.ground.as_deref() {
        Some("py") => format!("{name} = {rhs}\n"),
        _ => format!("let {name} = {rhs};\n"),
    }
}

/// Derive a short variable name from `body`.  If `body` is a plain identifier,
/// reuse it with a `_` prefix; otherwise fall back to `default`.
fn suggested_name(body: &str, default: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !trimmed.is_empty()
        && !trimmed.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        format!("_{trimmed}")
    } else {
        default.to_string()
    }
}

fn workspace_edit(uri: &Url, edits: Vec<TextEdit>) -> WorkspaceEdit {
    WorkspaceEdit {
        changes: Some(HashMap::from([(uri.clone(), edits)])),
        document_changes: None,
        change_annotations: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::lineindex::Encoding;

    fn doc(text: &str) -> Document {
        Document::new(
            &"file:///test.rs.quilt".parse().unwrap(),
            text.to_string(),
            1,
            None,
        )
    }

    fn lsp_range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Range {
        Range {
            start: tower_lsp::lsp_types::Position {
                line: start_line,
                character: start_char,
            },
            end: tower_lsp::lsp_types::Position {
                line: end_line,
                character: end_char,
            },
        }
    }

    fn apply_edits(text: &str, edits: &[TextEdit], enc: Encoding) -> String {
        let li = crate::lineindex::LineIndex::new(text);
        // Edits must be sorted high-to-low (no overlaps) before applying.
        let mut sorted = edits.to_vec();
        sorted.sort_by_key(|e| Reverse(e.range.start));
        let mut result = text.to_string();
        for edit in &sorted {
            let start = li.offset(text, edit.range.start, enc);
            let end = li.offset(text, edit.range.end, enc);
            result.replace_range(start..end, &edit.new_text);
        }
        result
    }

    fn first_code_action(text: &str, range: Range) -> Option<CodeAction> {
        let uri: Url = "file:///test.rs.quilt".parse().unwrap();
        let d = doc(text);
        let actions = code_actions(&uri, &d, Encoding::Utf8, range);
        actions.into_iter().find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => Some(ca),
            CodeActionOrCommand::Command(_) => None,
        })
    }

    fn find_action(text: &str, range: Range, title: &str) -> Option<CodeAction> {
        let uri: Url = "file:///test.rs.quilt".parse().unwrap();
        let d = doc(text);
        code_actions(&uri, &d, Encoding::Utf8, range)
            .into_iter()
            .find_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) if ca.title == title => Some(ca),
                _ => None,
            })
    }

    // ---- wrap in quotes ----

    #[test]
    fn wrap_available_for_non_empty_selection() {
        let text = "fn main() { let x = 1; }\n";
        // Select "1"
        let range = lsp_range(0, 20, 0, 21);
        let ca = first_code_action(text, range).unwrap();
        assert_eq!(ca.title, "Wrap in ↖…↗");
        assert!(ca.is_preferred == Some(true));
    }

    #[test]
    fn wrap_not_available_for_empty_selection() {
        let text = "fn main() {}\n";
        let range = lsp_range(0, 5, 0, 5); // empty
                                           // Should not return a wrap action (but may return others)
        let uri: Url = "file:///test.rs.quilt".parse().unwrap();
        let d = doc(text);
        let actions = code_actions(&uri, &d, Encoding::Utf8, range);
        let has_wrap = actions.iter().any(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title == "Wrap in ↖…↗",
            CodeActionOrCommand::Command(_) => false,
        });
        assert!(!has_wrap, "wrap should not be offered for empty selection");
    }

    #[test]
    fn wrap_produces_correct_edits() {
        let text = "let x = 1 + 2;\n";
        // Select "1 + 2" (bytes 8..13)
        let range = lsp_range(0, 8, 0, 13);
        let ca = find_action(text, range, "Wrap in ↖…↗").unwrap();
        let edits = ca.edit.unwrap().changes.unwrap();
        let uri: Url = "file:///test.rs.quilt".parse().unwrap();
        let edit_list = edits.get(&uri).unwrap();
        let result = apply_edits(text, edit_list, Encoding::Utf8);
        assert_eq!(result, "let x = ↖1 + 2↗;\n");
    }

    // ---- inline unquote ----

    #[test]
    fn inline_unquote_extracts_to_local() {
        let text = "let r = ↖foo + ↙bar↘ + baz↗;\n";
        // Cursor inside the unquote body "bar" — byte offset of 'b' in "bar":
        // "let r = ↖foo + ↙bar↘ + baz↗;\n"
        //  0123456789...
        // ↖ = bytes 8..11, ↙ = bytes 15..18, "bar" starts at 18
        let range = lsp_range(0, 18, 0, 18);
        let ca = find_action(text, range, "Inline unquote as local binding").unwrap();
        let edits = ca.edit.unwrap().changes.unwrap();
        let uri: Url = "file:///test.rs.quilt".parse().unwrap();
        let edit_list = edits.get(&uri).unwrap();
        let result = apply_edits(text, edit_list, Encoding::Utf8);
        // Should insert `let _bar = bar;\n` before the quote's line
        // and replace ↙bar↘ with ↙_bar↘
        assert!(result.contains("let _bar = bar;\n"), "got: {result:?}");
        assert!(result.contains("↙_bar↘"), "got: {result:?}");
    }

    // ---- extract fragment ----

    #[test]
    fn extract_fragment_available_inside_quote() {
        let text = "let r = ↖1 + 2↗;\n";
        // Cursor inside the quote body — byte 11 (somewhere inside "1 + 2")
        let range = lsp_range(0, 11, 0, 11);
        let ca = find_action(text, range, "Extract to named fragment");
        assert!(
            ca.is_some(),
            "extract fragment should be offered inside a quote"
        );
    }

    #[test]
    fn extract_fragment_not_available_in_ground() {
        let text = "let x = 1;\n";
        // Cursor in pure ground code
        let range = lsp_range(0, 4, 0, 4);
        let ca = find_action(text, range, "Extract to named fragment");
        assert!(
            ca.is_none(),
            "extract fragment should not fire in ground code"
        );
    }

    #[test]
    fn extract_fragment_produces_correct_edits() {
        let text = "let r = ↖1 + 2↗;\n";
        let range = lsp_range(0, 11, 0, 11);
        let ca = find_action(text, range, "Extract to named fragment").unwrap();
        let edits = ca.edit.unwrap().changes.unwrap();
        let uri: Url = "file:///test.rs.quilt".parse().unwrap();
        let edit_list = edits.get(&uri).unwrap();
        let result = apply_edits(text, edit_list, Encoding::Utf8);
        assert!(result.contains("let _frag = ↖1 + 2↗;\n"), "got: {result:?}");
        assert!(result.contains("↙_frag↘"), "got: {result:?}");
    }

    // ---- suggested_name ----

    #[test]
    fn suggested_name_plain_ident() {
        assert_eq!(suggested_name("foo", "_u"), "_foo");
        assert_eq!(suggested_name("  bar  ", "_u"), "_bar");
    }

    #[test]
    fn suggested_name_complex_expr() {
        assert_eq!(suggested_name("foo + bar", "_u"), "_u");
        assert_eq!(suggested_name("123abc", "_u"), "_u");
    }
}
