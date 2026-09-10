//! The HTML machine: a document as machine state.
//!
//! Every other provider in this crate wraps an interpreter, because "what a
//! program *does*" for Python or SQL means running it. HTML does nothing when
//! run — but a *document* is still a stateful thing that accepts fragments
//! over time and remembers them, which is all [`Machine`] asks for. So the
//! HTML machine is native and in-process: its state is one document term, and
//! the three message sorts read as a DOM would read them.
//!
//! * A **definition** (`Item`, `Stmt`, `File` — HTML has no statements, so
//!   the sorts collapse) is a fragment. Each top-level element that carries
//!   an `id` the document already has *replaces* the element of that id, in
//!   place; everything else is appended (into `<body>` when there is one).
//!   Ids are the document's own namespace, so they are the machine's
//!   definitions — the way `y = 40` is a Python machine's.
//! * A **query** (`Expr`) is a selector `#id`, answered with the element's
//!   own markup — the value's literal, as the denotation law wants. Any
//!   other fragment fed as a query answers itself: a fragment of markup *is*
//!   its own value.
//! * The **typing judgment** answers an element's tag name: the type of an
//!   HTML value is what kind of element it is.
//!
//! This is what `quilt notebook` runs on (see `crate::notebook`): the page
//! is the HTML machine, each cell's output is fed to it, and a cell that
//! prints `<p id="summary">…</p>` redefines the summary wherever the author
//! put it. But the machine is independent of notebooks — `html⟨M⟩` in a
//! Python or Rust program spawns one, `quilt repl html` talks to one, and
//! the conformance battery holds it to the same three laws as the rest.

use super::{Answer, Def, IntrospectMachine, Machine, Snapshot, SnapshotMachine};
use crate::lang::InnerKind;
use crate::prelude::*;
use crate::qterm::QTerm;
use crate::strcmd::StrCmd;
use crate::term::CmdOrHole;
use miette::{bail, IntoDiagnostic};

/**************************************************************/

/// A document, held as a term, driven as a machine. See the module docs.
pub struct HtmlMachine {
    /// Always a `document` tuple (possibly empty).
    doc: Arc<QTerm>,
}

impl Default for HtmlMachine {
    fn default() -> Self {
        HtmlMachine {
            doc: tuple("document", &[], &[]),
        }
    }
}

/// What [`HtmlMachine::define`] did with a fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defined {
    /// The fragment carried an id the document had: the old element is gone
    /// and the fragment stands in its place.
    Replaced,
    /// No such id (or no id): the fragment was appended.
    Appended,
}

impl HtmlMachine {
    /// Adopt a parsed document. A whole `document` is taken as is (its layout
    /// included); a single element or text node becomes a document of one.
    #[must_use]
    pub fn new(doc: &QTerm) -> Self {
        let root = unwrap_root(doc);
        let doc = match root {
            QTerm::Tuple { tag, .. } if &**tag == "document" => arc(root.clone()),
            _ => tuple("document", &[arc(root.clone())], &[CmdOrHole::Hole]),
        };
        HtmlMachine { doc }
    }

    /// The document as it stands.
    #[must_use]
    pub fn document(&self) -> &Arc<QTerm> {
        &self.doc
    }

    /// The element with this id, if the document has one.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<Arc<QTerm>> {
        find_first(&self.doc, &|t| element_id(t).as_deref() == Some(id))
    }

    /// What the element with this id *says*: its text content, entities
    /// decoded, markup dropped. `None` when there is no such element.
    #[must_use]
    pub fn text(&self, id: &str) -> Option<String> {
        self.find(id).map(|el| text_content(&el))
    }

    /// Every id the document defines, in document order.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        collect_ids(&self.doc, &mut ids);
        ids
    }

    /// Define one node: replace the element that has its id, else append it.
    ///
    /// The node's layout is baked first ([`bake_layout`]), so it renders
    /// exactly as fed wherever it lands: the indentation of its new
    /// surroundings cannot leak into it, which inside a `<pre>` would be a
    /// change of content, not of layout.
    pub fn define(&mut self, node: &Arc<QTerm>) -> Defined {
        let node = bake_layout(node);
        if let Some(id) = element_id(&node) {
            if let Some(doc) = map_first(
                &self.doc,
                &|t| element_id(t).as_deref() == Some(id.as_str()),
                &|_| node.clone(),
            ) {
                self.doc = doc;
                return Defined::Replaced;
            }
        }
        self.append_baked(&node);
        Defined::Appended
    }

    /// Append a node (layout baked, as for [`define`](Self::define)): into
    /// `<body>` when the document has one, else at the end of the document.
    pub fn append(&mut self, node: &Arc<QTerm>) {
        self.append_baked(&bake_layout(node));
    }

    fn append_baked(&mut self, node: &Arc<QTerm>) {
        let into_body = map_first(
            &self.doc,
            &|t| tag_name(t).as_deref() == Some("body"),
            &|body| append_child(body, node),
        );
        self.doc = into_body.unwrap_or_else(|| append_child(&self.doc, node));
    }
}

impl Machine for HtmlMachine {
    fn lang(&self) -> &'static str {
        "html"
    }

    fn feed(&mut self, kind: InnerKind, term: &QTerm) -> Result<Answer> {
        let nodes = top_level(term);
        // A selector is a query whatever sort it was sent as: `#id` is not a
        // fragment anyone could mean to define.
        if let [one] = nodes.as_slice() {
            if let Some(id) = selector(one) {
                let el = self
                    .find(id)
                    .ok_or_else(|| miette!("the html machine has no element with id {id:?}"))?;
                return Ok(Answer {
                    value: Some(el.coparse().into()),
                    ..Answer::default()
                });
            }
        }
        if kind == InnerKind::Expr {
            // Denotation: a fragment queried answers itself.
            return Ok(Answer {
                value: Some(term.coparse().trim().into()),
                ..Answer::default()
            });
        }
        for node in &nodes {
            self.define(node);
        }
        Ok(Answer::default())
    }

    /// The type of an HTML value is its element name: `#id` answers the tag
    /// of the element it names, an element its own tag, text `text`.
    fn type_of(&mut self, term: &QTerm) -> Result<Answer> {
        let nodes = top_level(term);
        let [one] = nodes.as_slice() else {
            bail!(
                "the html machine types one node at a time; got {} top-level nodes",
                nodes.len()
            );
        };
        let node = match selector(one) {
            Some(id) => self
                .find(id)
                .ok_or_else(|| miette!("the html machine has no element with id {id:?}"))?,
            None => one.clone(),
        };
        let ty = match tag_name(&node) {
            Some(tag) => tag,
            None => match &*node {
                QTerm::Tuple { tag, .. } => tag.to_string(),
                QTerm::Quote { .. } => "quote".to_string(),
                QTerm::Unquote { .. } => "unquote".to_string(),
            },
        };
        Ok(Answer {
            value: Some(ty.into()),
            ..Answer::default()
        })
    }
}

/// The whole document is the state, so a snapshot is the document.
impl SnapshotMachine for HtmlMachine {
    fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot(
            postcard::to_stdvec(&*self.doc).into_diagnostic()?.into(),
        ))
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<()> {
        let doc: QTerm = postcard::from_bytes(&snapshot.0).into_diagnostic()?;
        self.doc = arc(doc);
        Ok(())
    }
}

/// The definitions are the ids.
impl IntrospectMachine for HtmlMachine {
    fn defs(&self) -> Result<Vec<Def>> {
        Ok(self
            .ids()
            .into_iter()
            .map(|id| Def {
                name: id.into(),
                kind: InnerKind::Item,
            })
            .collect())
    }
}

/**************************************************************/
// Term helpers. Public because the notebook driver reads documents with the
// same eyes the machine does — one notion of "the element with this id".

/// See through the tagless root `parse_chain` wraps a parsed fragment in.
fn unwrap_root(term: &QTerm) -> &QTerm {
    match term {
        QTerm::Tuple { tag, terms, .. } if tag.is_empty() && terms.len() == 1 => {
            unwrap_root(&terms[0])
        }
        _ => term,
    }
}

/// The top-level nodes of a fragment: a `document`'s children, else the
/// fragment itself (a single element or text node).
#[must_use]
pub fn top_level(term: &QTerm) -> Vec<Arc<QTerm>> {
    match unwrap_root(term) {
        QTerm::Tuple { tag, terms, .. } if &**tag == "document" => terms.to_vec(),
        other => vec![arc(other.clone())],
    }
}

/// `#id`, when `node` is a text node holding exactly one selector.
#[must_use]
pub fn selector(node: &QTerm) -> Option<&str> {
    let QTerm::Tuple { tag, terms, cmds } = node else {
        return None;
    };
    if &**tag != "text" || !terms.is_empty() {
        return None;
    }
    let [CmdOrHole::Cmd(StrCmd::Write(text))] = &**cmds else {
        return None;
    };
    let text = text.trim();
    let id = text.strip_prefix('#')?;
    (!id.is_empty() && !id.contains(char::is_whitespace)).then_some(id)
}

fn children(term: &QTerm) -> &[Arc<QTerm>] {
    match term {
        QTerm::Tuple { terms, .. } => terms,
        QTerm::Quote { .. } | QTerm::Unquote { .. } => &[],
    }
}

fn tag_is(term: &QTerm, want: &str) -> bool {
    matches!(term, QTerm::Tuple { tag, .. } if &**tag == want)
}

/// The text of a leaf (a tuple with no children and one `Write`).
fn leaf_text(term: &QTerm) -> Option<&str> {
    match term {
        QTerm::Tuple { terms, cmds, .. } if terms.is_empty() => match &**cmds {
            [CmdOrHole::Cmd(StrCmd::Write(s))] => Some(s),
            _ => None,
        },
        _ => None,
    }
}

/// The `start_tag` / `self_closing_tag` of an element. `<script>` and
/// `<style>` are elements too, under their own node kinds.
fn opening_tag(term: &QTerm) -> Option<&QTerm> {
    if !(tag_is(term, "element") || tag_is(term, "script_element") || tag_is(term, "style_element"))
    {
        return None;
    }
    children(term)
        .first()
        .map(AsRef::as_ref)
        .filter(|t| tag_is(t, "start_tag") || tag_is(t, "self_closing_tag"))
}

/// An element's tag name (`p` for `<p>…</p>`).
#[must_use]
pub fn tag_name(term: &QTerm) -> Option<String> {
    let open = opening_tag(term)?;
    children(open)
        .iter()
        .find(|t| tag_is(t, "tag_name"))
        .and_then(|t| leaf_text(t))
        .map(str::to_string)
}

/// An element's `id` attribute value, when it has one.
#[must_use]
pub fn element_id(term: &QTerm) -> Option<String> {
    let open = opening_tag(term)?;
    children(open)
        .iter()
        .filter(|t| tag_is(t, "attribute"))
        .find_map(|attr| {
            let parts = children(attr);
            let name = parts.first().and_then(|n| leaf_text(n))?;
            if name != "id" {
                return None;
            }
            let value = parts.iter().skip(1).find_map(|p| {
                if tag_is(p, "attribute_value") {
                    leaf_text(p).map(str::to_string)
                } else if tag_is(p, "quoted_attribute_value") {
                    children(p)
                        .iter()
                        .find(|q| tag_is(q, "attribute_value"))
                        .and_then(|q| leaf_text(q))
                        .map(str::to_string)
                } else {
                    None
                }
            });
            // `<p id>` (no value) defines the empty id, which nothing can
            // reference; treat it as no id.
            value.filter(|v| !v.is_empty())
        })
}

/// The text content of a node: text and entities, markup dropped, script
/// and style bodies skipped. The whitespace between nodes is layout the
/// parser keeps as `Write`/`NewLine` commands on the parent rather than as
/// text nodes, so the walk follows the commands, not just the children.
#[must_use]
pub fn text_content(term: &QTerm) -> String {
    fn collect(term: &QTerm, out: &mut String) {
        let QTerm::Tuple { tag, terms, cmds } = term else {
            out.push_str(&term.coparse());
            return;
        };
        match &**tag {
            "entity" => out.push_str(&decode_entity(leaf_text(term).unwrap_or_default())),
            "start_tag" | "end_tag" | "self_closing_tag" | "doctype" | "comment"
            | "script_element" | "style_element" | "raw_text" => {}
            _ => {
                let mut children = terms.iter();
                for cmd in cmds {
                    match cmd {
                        CmdOrHole::Hole => {
                            if let Some(child) = children.next() {
                                collect(child, out);
                            }
                        }
                        CmdOrHole::Cmd(StrCmd::Write(s)) => out.push_str(s),
                        CmdOrHole::Cmd(StrCmd::NewLine) => out.push('\n'),
                        CmdOrHole::Cmd(StrCmd::Push(_) | StrCmd::Pop) => {}
                    }
                }
            }
        }
    }
    let mut out = String::new();
    collect(term, &mut out);
    out
}

/// Decode one HTML entity (`&amp;`, `&#39;`, `&#x27;`, …); unknown ones are
/// kept as written.
fn decode_entity(entity: &str) -> String {
    let inner = entity
        .strip_prefix('&')
        .and_then(|s| s.strip_suffix(';'))
        .unwrap_or(entity);
    let decoded = match inner {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => inner
            .strip_prefix('#')
            .and_then(|n| match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => n.parse().ok(),
            })
            .and_then(char::from_u32),
    };
    decoded.map_or_else(|| entity.to_string(), String::from)
}

fn collect_ids(term: &QTerm, out: &mut Vec<String>) {
    if let Some(id) = element_id(term) {
        out.push(id);
    }
    for child in children(term) {
        collect_ids(child, out);
    }
}

/// Bake a fragment's layout into its text: every `NewLine` becomes a
/// literal newline carrying the prefix the fragment's *own* pushes had built
/// at that point, and the pushes and pops are dropped. A `Write` ignores the
/// prefix stack, so a baked fragment renders the same wherever it is
/// grafted — the enclosing element's indentation cannot reach it. The cost
/// is cosmetic: a grafted fragment is not re-indented to match its
/// neighbours.
#[must_use]
pub fn bake_layout(term: &Arc<QTerm>) -> Arc<QTerm> {
    fn go(term: &Arc<QTerm>, stack: &mut Vec<Box<str>>) -> Arc<QTerm> {
        let (cmds, terms): (&[CmdOrHole], &[Arc<QTerm>]) = match &**term {
            QTerm::Tuple { terms, cmds, .. } => (cmds, terms),
            QTerm::Quote { term, cmds, .. } | QTerm::Unquote { term, cmds, .. } => {
                (cmds, std::slice::from_ref(term))
            }
        };
        let mut new_cmds = Vec::with_capacity(cmds.len());
        let mut new_terms = Vec::with_capacity(terms.len());
        let mut children = terms.iter();
        for cmd in cmds {
            match cmd {
                CmdOrHole::Hole => {
                    if let Some(child) = children.next() {
                        let depth = stack.len();
                        new_terms.push(go(child, stack));
                        stack.truncate(depth);
                    }
                    new_cmds.push(CmdOrHole::Hole);
                }
                CmdOrHole::Cmd(StrCmd::NewLine) => {
                    let mut line = String::from("\n");
                    for prefix in stack.iter() {
                        line.push_str(prefix);
                    }
                    new_cmds.push(CmdOrHole::Cmd(StrCmd::Write(line.into())));
                }
                CmdOrHole::Cmd(StrCmd::Push(prefix)) => stack.push(prefix.clone()),
                CmdOrHole::Cmd(StrCmd::Pop) => {
                    stack.pop();
                }
                CmdOrHole::Cmd(write @ StrCmd::Write(_)) => {
                    new_cmds.push(CmdOrHole::Cmd(write.clone()));
                }
            }
        }
        // Children beyond the holes (none, for a parsed term) are kept.
        new_terms.extend(children.map(|c| go(c, stack)));
        match &**term {
            QTerm::Tuple { tag, .. } => tuple(tag, &new_terms, &new_cmds),
            QTerm::Quote {
                tag,
                index,
                lang,
                span,
                ..
            } => arc(crate::qterm::qquote_at(
                tag,
                *index,
                lang,
                new_terms.remove(0),
                &new_cmds,
                span.clone(),
            )),
            QTerm::Unquote {
                tag,
                index,
                lang,
                span,
                ..
            } => arc(crate::qterm::qunquote_at(
                tag,
                *index,
                lang,
                new_terms.remove(0),
                &new_cmds,
                span.clone(),
            )),
        }
    }
    go(term, &mut Vec::new())
}

/// The first node (pre-order) satisfying `pred`.
fn find_first(term: &Arc<QTerm>, pred: &dyn Fn(&QTerm) -> bool) -> Option<Arc<QTerm>> {
    if pred(term) {
        return Some(term.clone());
    }
    children(term).iter().find_map(|c| find_first(c, pred))
}

/// Rebuild the tree with the first node (pre-order) satisfying `pred`
/// replaced by `f(node)`; `None` if nothing matched.
fn map_first(
    term: &Arc<QTerm>,
    pred: &dyn Fn(&QTerm) -> bool,
    f: &dyn Fn(&Arc<QTerm>) -> Arc<QTerm>,
) -> Option<Arc<QTerm>> {
    if pred(term) {
        return Some(f(term));
    }
    let QTerm::Tuple { tag, terms, cmds } = &**term else {
        return None;
    };
    let (i, new) = terms
        .iter()
        .enumerate()
        .find_map(|(i, c)| map_first(c, pred, f).map(|n| (i, n)))?;
    let mut terms = terms.to_vec();
    terms[i] = new;
    Some(tuple(tag, &terms, cmds))
}

/// `container` with `child` appended: before the end tag of an element,
/// at the end of anything else. The new child lands on its own line, at the
/// indentation of its new siblings when the layout commands say what that
/// is.
fn append_child(container: &Arc<QTerm>, child: &Arc<QTerm>) -> Arc<QTerm> {
    let QTerm::Tuple { tag, terms, cmds } = &**container else {
        return container.clone();
    };
    let mut cmds: Vec<CmdOrHole> = cmds.to_vec();
    let mut terms = terms.to_vec();
    let closes = terms.last().is_some_and(|t| tag_is(t, "end_tag"));
    if closes {
        // Insert before the end tag's hole — and before the newline/pop
        // run that precedes it, so the child is indented like its siblings.
        let end_hole = cmds
            .iter()
            .rposition(|c| matches!(c, CmdOrHole::Hole))
            .expect("an element with an end tag has a hole for it");
        let mut at = end_hole;
        while at > 0 && matches!(cmds[at - 1], CmdOrHole::Cmd(StrCmd::NewLine | StrCmd::Pop)) {
            at -= 1;
        }
        let index = cmds[..at]
            .iter()
            .filter(|c| matches!(c, CmdOrHole::Hole))
            .count();
        cmds.splice(at..at, [CmdOrHole::Cmd(StrCmd::NewLine), CmdOrHole::Hole]);
        terms.insert(index, child.clone());
    } else {
        if !terms.is_empty() {
            cmds.push(CmdOrHole::Cmd(StrCmd::NewLine));
        }
        cmds.push(CmdOrHole::Hole);
        terms.push(child.clone());
    }
    tuple(tag, &terms, &cmds)
}

/**************************************************************/

#[cfg(all(test, feature = "html"))]
mod tests {
    use super::*;
    use crate::lang::{flat_nodes, Language as _};
    use crate::langs::html::lang::HtmlLanguage;

    fn html(src: &str) -> Arc<QTerm> {
        HtmlLanguage::default()
            .parse_as(None, &flat_nodes(src))
            .expect("fixture parses")
    }

    fn feed(m: &mut HtmlMachine, src: &str) -> Answer {
        m.feed(InnerKind::File, &html(src)).expect("feed")
    }

    fn ask(m: &mut HtmlMachine, src: &str) -> String {
        m.eval(&html(src)).expect("eval").value.unwrap().to_string()
    }

    #[test]
    fn definitions_are_ids_and_queries_are_selectors() {
        let mut m = HtmlMachine::default();
        feed(&mut m, "<p id=\"y\">40</p>");
        assert_eq!(ask(&mut m, "#y"), "<p id=\"y\">40</p>");
        assert_eq!(m.text("y").as_deref(), Some("40"));
    }

    #[test]
    fn redefining_an_id_replaces_in_place() {
        let mut m = HtmlMachine::new(&html(
            "<div id=\"a\">\n  <p id=\"y\">40</p>\n  <p id=\"z\">1</p>\n</div>",
        ));
        feed(&mut m, "<b id=\"y\">42</b>");
        let doc = m.document().coparse();
        assert_eq!(
            doc, "<div id=\"a\">\n  <b id=\"y\">42</b>\n  <p id=\"z\">1</p>\n</div>",
            "replaced where it stood, layout kept"
        );
        assert_eq!(m.ids(), ["a", "y", "z"]);
    }

    #[test]
    fn unknown_ids_and_idless_nodes_append_into_body() {
        let mut m = HtmlMachine::new(&html(
            "<html>\n<body>\n  <p id=\"a\">1</p>\n</body>\n</html>",
        ));
        feed(&mut m, "<p id=\"b\">2</p>");
        feed(&mut m, "plain text");
        let doc = m.document().coparse();
        assert_eq!(
            doc,
            "<html>\n<body>\n  <p id=\"a\">1</p>\n  <p id=\"b\">2</p>\n  plain text\n</body>\n</html>"
        );
    }

    #[test]
    fn fragments_without_a_body_append_at_the_end() {
        let mut m = HtmlMachine::default();
        feed(&mut m, "<p>a</p>");
        feed(&mut m, "<p>b</p>\n<p>c</p>");
        assert_eq!(m.document().coparse(), "<p>a</p>\n<p>b</p>\n<p>c</p>");
    }

    #[test]
    fn a_fragment_queried_answers_itself() {
        let mut m = HtmlMachine::default();
        assert_eq!(ask(&mut m, "<b>x</b>"), "<b>x</b>");
        assert!(m.ids().is_empty(), "a query defines nothing");
    }

    #[test]
    fn a_missing_id_is_an_error() {
        let mut m = HtmlMachine::default();
        let err = m.eval(&html("#nope")).unwrap_err();
        assert!(err.to_string().contains("\"nope\""), "{err}");
    }

    /// `<style>` and `<script>` carry ids like any element, under the
    /// grammar's own node kinds for them.
    #[test]
    fn style_and_script_elements_have_ids() {
        let mut m = HtmlMachine::default();
        feed(
            &mut m,
            "<style id=\"s\">.a{}</style><script id=\"j\">1</script>",
        );
        assert_eq!(m.ids(), ["s", "j"]);
        feed(&mut m, "<style id=\"s\">.b{}</style>");
        // Appended nodes land one per line; the replacement keeps the layout.
        assert_eq!(
            m.document().coparse(),
            "<style id=\"s\">.b{}</style>\n<script id=\"j\">1</script>"
        );
    }

    #[test]
    fn types_are_tag_names() {
        let mut m = HtmlMachine::default();
        feed(&mut m, "<section id=\"s\"><p>x</p></section>");
        let ty = |m: &mut HtmlMachine, s: &str| m.type_of(&html(s)).unwrap().value.unwrap();
        assert_eq!(&*ty(&mut m, "#s"), "section");
        assert_eq!(&*ty(&mut m, "<input id=\"q\"/>"), "input");
        assert_eq!(&*ty(&mut m, "hello"), "text");
    }

    #[test]
    fn text_content_decodes_entities_and_drops_markup() {
        let el = html("<p id=\"t\">a &amp; <b>b</b> &#x27;c&#39; &lt;d&gt;</p>");
        assert_eq!(text_content(&el), "a & b 'c' <d>");
    }

    #[test]
    fn snapshots_roll_the_document_back() {
        let mut m = HtmlMachine::default();
        feed(&mut m, "<p id=\"y\">1</p>");
        let snap = m.snapshot().unwrap();
        feed(&mut m, "<p id=\"y\">2</p>");
        assert_eq!(m.text("y").as_deref(), Some("2"));
        m.restore(&snap).unwrap();
        assert_eq!(m.text("y").as_deref(), Some("1"));
    }

    #[test]
    fn defs_are_the_ids() {
        let mut m = HtmlMachine::default();
        feed(&mut m, "<p id=\"a\">1</p><p id=\"b\">2</p>");
        let defs: Vec<_> = m.defs().unwrap().into_iter().map(|d| d.name).collect();
        assert_eq!(defs, ["a".into(), "b".into()]);
    }

    #[test]
    fn machines_are_isolated() {
        let mut a = HtmlMachine::default();
        let mut b = HtmlMachine::default();
        feed(&mut a, "<p id=\"y\">1</p>");
        assert!(b.eval(&html("#y")).is_err());
    }

    /// A multi-line fragment grafted into an indented position keeps its
    /// own lines exactly — inside a `<pre>` the enclosing indentation would
    /// be content.
    #[test]
    fn grafted_fragments_keep_their_layout() {
        let mut m = HtmlMachine::new(&html("<div>\n    <p id=\"slot\">x</p>\n</div>"));
        feed(&mut m, "<pre id=\"slot\">a\nb\n  c</pre>");
        let doc = m.document().coparse();
        assert!(
            doc.contains("<pre id=\"slot\">a\nb\n  c</pre>"),
            "lines must not be re-indented: {doc}"
        );
        assert_eq!(m.text("slot").as_deref(), Some("a\nb\n  c"));
    }
}
