//! Notebooks: an HTML page whose quoted cells run on machines.
//!
//! A notebook is an `.html.quilt` file — ordinary HTML with cells spliced in
//! as quotes of other languages:
//!
//! ```html
//! <div class="row">
//!   py↖
//!     total = 6 * 7
//!     print(html↖<b id="answer">↙↑(total)↘</b>↗.coparse())
//!   ↗
//!   sql↖SELECT ↙#answer↘ * 2;↗
//! </div>
//! ```
//!
//! HTML is the ground language, so the page is the program and the cells are
//! its quotes; the file's *expansion* (`langs::html::meta`) holds the cells
//! verbatim, which is what `quilt check` validates. *Running* it is this
//! module's job, and it is machines all the way down:
//!
//! * **The page is the HTML machine.** Each cell is planted as a placeholder
//!   element, the page becomes an [`HtmlMachine`], and every result is fed
//!   back to it as a definition. A cell whose output carries an `id` the page
//!   already has *redefines* that element wherever the author put it — a
//!   Python cell that prints `<p id="summary">…</p>` edits the summary at the
//!   top of the page. Ids are the page's definitions, and cells are how they
//!   are made.
//! * **Each cell runs on its language's park machine** (`Multi::machine`):
//!   the persistent python kernel, a live `sqlite3`, a long-lived shell —
//!   so definitions persist from cell to cell, the way they persist across
//!   lines of `quilt repl`. A cell is re-parsed as a *program* of its own
//!   language and expanded by that language's meta before it is fed, so a
//!   Python cell quotes HTML with `html↖…↗` and lifts into it with `↑`
//!   exactly as a `.html.py.quilt` file would.
//! * **An unquote that reaches the page reads the page.** `↙#total↘` inside a
//!   cell is a document reference: the text of the element with that id,
//!   spliced into the cell as a string literal of the cell's language. That
//!   is how machines that share nothing exchange values — SQL answers into
//!   `<b id="total">`, Python reads `↙#total↘` — through the document, the
//!   one thing every cell can see. An unquote whose body is not a selector
//!   splices its own text, which is what the identity host would have done.
//! * **Cells create cells.** Output is parsed as Quilt-in-HTML, not just as
//!   HTML, so a quote in a cell's output is a new cell: it is planted, run
//!   next (before the cells that follow in the page), and rendered in place.
//!   Quilt's own staging depth makes this natural — a Python cell writes
//!   `html↖<li>bash↖…↗</li>↗`, and the nested quote survives expansion with
//!   its glyphs because it belongs to the next stage. A value baked into a
//!   generated cell crosses two quote levels, so it is written `↙↙↑(n)↘↘`,
//!   and the generation counter stops a cell that keeps creating cells at
//!   [`MAX_GENERATION`].
//!
//! Output that contains a tag (or a glyph) is markup and joins the page as
//! elements; plain text is shown verbatim. A cell that fails — a rejected
//! feed, a parse error, a missing reference — renders its error and the
//! notebook goes on, as a REPL would. Nothing is retried and nothing is
//! cached: the notebook is a run, and its rendering is the record of it.

use std::collections::VecDeque;
use std::fmt::Write as _;

use miette::bail;

use crate::glyphs::GLYPHS;
use crate::lang::{flat_nodes, Language as _};
use crate::lift::escape_html;
use crate::machine::html::{element_id, top_level};
use crate::machine::{Answer, HtmlMachine, Machine as _};
use crate::multi::{Languages, MetaLanguages, Multi};
use crate::prelude::*;
use crate::qterm::QTerm;

/**************************************************************/

/// How many generations of cell-created cells a notebook will run before a
/// cell is refused: a cell whose output creates a cell whose output creates
/// a cell … is a loop with no other bound.
pub const MAX_GENERATION: usize = 8;

/// The id of the stylesheet the notebook adds when the page defines none —
/// define an element with this id yourself to take over the cell chrome.
pub const STYLE_ID: &str = "quilt-cells";

/// The literals that spell "no value" — what a query answers when it was
/// really an effect (`print(…)` in Python, a `console.log` in TypeScript).
/// Not shown as a value: a cell that printed has shown what it did.
pub const NO_VALUE: &[&str] = &["None", "undefined"];

/// The cell chrome: enough to read a notebook cold, and no page layout —
/// arranging cells is the page's business.
pub const DEFAULT_STYLE: &str = r"
.quilt-cell { --quilt-accent: #8250df; --quilt-line: #d0d7de; --quilt-ink: #1f2328; --quilt-paper: #fff; --quilt-mute: #57606a;
  border: 1px solid var(--quilt-line); border-left: 4px solid var(--quilt-accent); border-radius: 6px;
  margin: 1rem 0; padding: 0; background: var(--quilt-paper); color: var(--quilt-ink); font-size: .95rem; overflow: hidden; }
.quilt-cell > .quilt-lang { font: 600 .7rem/1 ui-monospace, SFMono-Regular, Menlo, monospace; text-transform: uppercase;
  letter-spacing: .08em; color: var(--quilt-accent); padding: .45rem .75rem 0; }
.quilt-cell > .quilt-src { margin: 0; padding: .45rem .75rem .6rem; overflow-x: auto; font: .85rem/1.45 ui-monospace, SFMono-Regular, Menlo, monospace; }
.quilt-cell > .quilt-out { display: block; border-top: 1px dashed var(--quilt-line); padding: .5rem .75rem; }
.quilt-cell .quilt-stdout, .quilt-cell .quilt-value { margin: 0; font: .85rem/1.45 ui-monospace, SFMono-Regular, Menlo, monospace; white-space: pre-wrap; }
.quilt-cell .quilt-value { color: #0550ae; }
.quilt-cell > .quilt-stderr, .quilt-cell > .quilt-error { margin: 0; border-top: 1px dashed var(--quilt-line); padding: .5rem .75rem;
  font: .8rem/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; white-space: pre-wrap; }
.quilt-cell > .quilt-stderr { color: #7d4e00; background: #fff8c5; }
.quilt-cell > .quilt-error { color: #a40e26; background: #ffebe9; }
.quilt-cell.quilt-failed { --quilt-accent: #cf222e; }
.quilt-cell.quilt-generated { border-left-style: dashed; }
.quilt-cell.quilt-pending { --quilt-accent: #8c959f; }
.quilt-cell .quilt-glyph { color: #bc4c00; font-weight: 700; }
.quilt-cell.quilt-py, .quilt-cell.quilt-python { --quilt-accent: #3572a5; }
.quilt-cell.quilt-sql, .quilt-cell.quilt-mysql { --quilt-accent: #e38c00; }
.quilt-cell.quilt-bash, .quilt-cell.quilt-zsh { --quilt-accent: #4e9a06; }
.quilt-cell.quilt-ts, .quilt-cell.quilt-typescript { --quilt-accent: #3178c6; }
.quilt-cell.quilt-html { --quilt-accent: #e34c26; }
@media (prefers-color-scheme: dark) {
  .quilt-cell { --quilt-line: #3d444d; --quilt-ink: #e6edf3; --quilt-paper: #161b22; --quilt-mute: #9198a1; }
  .quilt-cell .quilt-value { color: #79c0ff; }
  .quilt-cell > .quilt-stderr { color: #e3b341; background: #2b2411; }
  .quilt-cell > .quilt-error { color: #ff7b72; background: #2d1214; }
}
";

/**************************************************************/

/// One cell, as it ran.
#[derive(Debug, Clone)]
pub struct Cell {
    /// The cell's number: its element in the page is `quilt-cell-<id>`.
    pub id: usize,
    /// The language the cell was quoted in, as written (`py`, `sql`, …).
    pub lang: Box<str>,
    /// 0 for a cell the author wrote; `n + 1` for one created by the output
    /// of a generation-`n` cell.
    pub generation: usize,
    /// The cell's source as shown: dedented, references unresolved.
    pub src: String,
    /// The answered literal, for a cell that was a query.
    pub value: Option<Box<str>>,
    pub stdout: Box<str>,
    pub stderr: Box<str>,
    /// Why the cell did not run to an answer, when it did not.
    pub error: Option<String>,
    /// Ids of elements elsewhere in the page this cell's output redefined.
    pub edits: Vec<String>,
    /// Ids of the cells this cell's output created.
    pub spawned: Vec<usize>,
}

impl Cell {
    #[must_use]
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }

    /// The id of this cell's `<figure>` in the page.
    #[must_use]
    pub fn element_id(&self) -> String {
        cell_element_id(self.id)
    }
}

fn cell_element_id(id: usize) -> String {
    format!("quilt-cell-{id}")
}

/// A cell that has been given a place in the page and not yet run.
struct Planted {
    id: usize,
    lang: Box<str>,
    body: Arc<QTerm>,
    generation: usize,
}

/// A notebook run: the page with its cells rendered, and the cells.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub html: String,
    pub cells: Vec<Cell>,
}

impl Rendered {
    /// The cells that did not run to an answer.
    #[must_use]
    pub fn failures(&self) -> Vec<&Cell> {
        self.cells.iter().filter(|c| c.failed()).collect()
    }
}

/// Run a notebook: the page in `src`, its cells on `multi`'s park machines.
pub fn run<LS: Languages, MS: MetaLanguages>(
    multi: &mut Multi<LS, MS>,
    src: &str,
) -> Result<Rendered> {
    let mut notebook = Notebook::open(multi, src)?;
    notebook.run_pending();
    Ok(notebook.finish())
}

/**************************************************************/

/// A notebook session: a page held as an [`HtmlMachine`], the cells found in
/// it, and the `Multi` whose park machines run them. [`run`] is the
/// one-shot form; the session form is what `quilt repl html` drives a line
/// at a time.
pub struct Notebook<'m, LS: Languages, MS: MetaLanguages> {
    multi: &'m mut Multi<LS, MS>,
    page: HtmlMachine,
    cells: Vec<Cell>,
    queue: VecDeque<Planted>,
    /// Cells planted while a fragment was being read; moved to the queue
    /// once the fragment is placed.
    spawned: Vec<Planted>,
    next_id: usize,
}

impl<'m, LS: Languages, MS: MetaLanguages> Notebook<'m, LS, MS> {
    /// An empty page.
    pub fn new(multi: &'m mut Multi<LS, MS>) -> Self {
        Notebook {
            multi,
            page: HtmlMachine::default(),
            cells: Vec::new(),
            queue: VecDeque::new(),
            spawned: Vec::new(),
            next_id: 1,
        }
    }

    /// Open a notebook: parse the page, plant its cells, and hold it as the
    /// HTML machine. Nothing runs until [`run_pending`](Self::run_pending).
    pub fn open(multi: &'m mut Multi<LS, MS>, src: &str) -> Result<Self> {
        let mut nb = Self::new(multi);
        let doc = nb.multi.parse_chain(&["html"], src)?;
        let planted = nb.plant(&doc, 0);
        nb.page = HtmlMachine::new(&planted);
        nb.queue.extend(nb.spawned.drain(..));
        nb.ensure_style();
        Ok(nb)
    }

    /// Feed a fragment of Quilt-in-HTML to the page — the REPL's turn. Plain
    /// markup defines (by id) or appends; quotes are cells, run before this
    /// returns. Answers the ids of the cells that ran.
    pub fn feed_source(&mut self, src: &str) -> Result<Vec<usize>> {
        let frag = self.multi.parse_chain(&["html"], src)?;
        let planted = self.plant(&frag, 0);
        for node in top_level(&planted) {
            self.page.define(&node);
        }
        self.queue.extend(self.spawned.drain(..));
        let first = self.cells.len();
        self.run_pending();
        Ok(self.cells[first..].iter().map(|c| c.id).collect())
    }

    /// Run every planted cell, in page order — and every cell those create.
    pub fn run_pending(&mut self) {
        while let Some(planted) = self.queue.pop_front() {
            self.run_cell(planted);
        }
    }

    /// The page as it stands.
    #[must_use]
    pub fn page(&self) -> &HtmlMachine {
        &self.page
    }

    /// The cells run so far, in the order they ran.
    #[must_use]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The page, serialized.
    #[must_use]
    pub fn render(&self) -> String {
        self.page.document().coparse()
    }

    /// The run, as a page and its cells.
    #[must_use]
    pub fn finish(self) -> Rendered {
        Rendered {
            html: self.render(),
            cells: self.cells,
        }
    }

    /// Add the default cell chrome unless the page defines its own.
    fn ensure_style(&mut self) {
        if self.page.find(STYLE_ID).is_some() {
            return;
        }
        let style = format!("<style id=\"{STYLE_ID}\">{DEFAULT_STYLE}</style>");
        if let Ok(style) = self.parse_html(&style) {
            self.page.define(&style);
        }
    }

    /// Parse a fragment of finished HTML (no Quilt in it) with the HTML
    /// language.
    fn parse_html(&mut self, src: &str) -> Result<Arc<QTerm>> {
        self.multi
            .get_lang_mut("html")?
            .parse_as(None, &flat_nodes(src))
    }

    /// Rebuild a fragment with every quote replaced by a placeholder
    /// element, recording each as a cell to run at `generation`.
    fn plant(&mut self, term: &Arc<QTerm>, generation: usize) -> Arc<QTerm> {
        match &**term {
            QTerm::Quote {
                lang, term: body, ..
            } => {
                let id = self.next_id;
                self.next_id += 1;
                self.spawned.push(Planted {
                    id,
                    lang: lang.clone(),
                    body: body.clone(),
                    generation,
                });
                self.placeholder(id, lang)
            }
            QTerm::Tuple { tag, terms, cmds } => {
                let terms: Vec<_> = terms.iter().map(|t| self.plant(t, generation)).collect();
                tuple(tag, &terms, cmds)
            }
            // An unquote at page level is a parse error before it gets here.
            QTerm::Unquote { .. } => term.clone(),
        }
    }

    /// The element that stands where a cell will render.
    fn placeholder(&mut self, id: usize, lang: &str) -> Arc<QTerm> {
        let html = format!(
            "<figure class=\"quilt-cell quilt-pending\" id=\"{}\" data-lang=\"{lang}\"></figure>",
            cell_element_id(id)
        );
        self.parse_html(&html).unwrap_or_else(|_| leaf("text", ""))
    }

    /// Run one planted cell and render it into its place.
    fn run_cell(&mut self, planted: Planted) {
        let Planted {
            id,
            lang,
            body,
            generation,
        } = planted;
        let mut cell = Cell {
            id,
            lang: lang.clone(),
            generation,
            src: cell_source(&lang, &body),
            value: None,
            stdout: Box::default(),
            stderr: Box::default(),
            error: None,
            edits: Vec::new(),
            spawned: Vec::new(),
        };
        // The nodes that stay in the cell's own output slot.
        let mut slot: Vec<Arc<QTerm>> = Vec::new();
        match self.execute(&lang, &body, generation) {
            Ok(answer) => {
                // A query's answer is its last stdout line: it is the value,
                // and not output as well.
                cell.stdout = match &answer.value {
                    Some(value) => without_answer_line(&answer.stdout, value).into(),
                    None => answer.stdout,
                };
                cell.stderr = answer.stderr;
                cell.value = answer.value;
                let stdout = cell.stdout.to_string();
                let mut nodes = self.output_nodes(&stdout, "quilt-stdout", generation);
                if let Some(value) = cell.value.clone() {
                    if !NO_VALUE.contains(&value.trim()) {
                        nodes.extend(self.output_nodes(&value, "quilt-value", generation));
                    }
                }
                for node in nodes {
                    // An element with an id the page has already defined is a
                    // redefinition: it goes where that id is, not here.
                    match element_id(&node) {
                        Some(id) if self.page.find(&id).is_some() => {
                            self.page.define(&node);
                            cell.edits.push(id);
                        }
                        _ => slot.push(node),
                    }
                }
            }
            Err(e) => cell.error = Some(render_error(&e)),
        }
        // The cells this one created run next, before the rest of the page.
        cell.spawned = self.spawned.iter().map(|p| p.id).collect();
        let spawned: Vec<Planted> = self.spawned.drain(..).collect();
        for planted in spawned.into_iter().rev() {
            self.queue.push_front(planted);
        }
        self.place(&cell, &slot);
        self.cells.push(cell);
    }

    /// Resolve the cell's page references, re-parse it as a program of its
    /// own language, expand it with that language's meta, and feed it to
    /// the language's park machine.
    fn execute(&mut self, lang: &str, body: &Arc<QTerm>, generation: usize) -> Result<Answer> {
        if generation > MAX_GENERATION {
            bail!(
                "cell generation {generation} is past the limit of {MAX_GENERATION}: cells kept \
                 creating cells"
            );
        }
        let resolved = self.resolve_refs(lang, body, 1)?;
        let src = cell_source(lang, &resolved);
        let with_src = |e: miette::Report| e.with_source_code(src.clone());
        let term = self.multi.parse_chain(&[lang], &src).map_err(with_src)?;
        // A language with no meta (SQL, WGSL) has nothing to expand: the
        // parsed term is the program.
        let term = if self.multi.get_meta(lang).is_ok() {
            self.multi.expand_lang(lang, &term).map_err(with_src)?
        } else {
            term
        };
        let kind = self.multi.classify_for_feed(lang, &term)?;
        if lang == "html" {
            // An HTML cell's machine is the page itself.
            return self.page.feed(kind, &term);
        }
        self.multi.machine(lang)?.feed(kind, &term)
    }

    /// Replace every unquote that reaches the page (`depth` quote levels up
    /// from the cell body, which sits at depth 1) with what it reads: a
    /// selector's element text as a string literal of the cell's language,
    /// or any other body as its own text.
    fn resolve_refs(&self, lang: &str, term: &Arc<QTerm>, depth: usize) -> Result<Arc<QTerm>> {
        Ok(match &**term {
            QTerm::Quote {
                tag,
                index,
                lang: qlang,
                term: inner,
                cmds,
                span,
            } => arc(QTerm::Quote {
                tag: tag.clone(),
                index: *index,
                lang: qlang.clone(),
                term: self.resolve_refs(lang, inner, depth + usize::from(*index))?,
                cmds: cmds.clone(),
                span: span.clone(),
            }),
            QTerm::Unquote {
                tag,
                index,
                lang: ulang,
                term: inner,
                cmds,
                span,
            } => {
                if usize::from(*index) >= depth {
                    let text = inner.coparse();
                    let text = text.trim();
                    let spliced = match text.strip_prefix('#') {
                        Some(id) if !id.is_empty() && !id.contains(char::is_whitespace) => {
                            let value = self.page.text(id).ok_or_else(|| {
                                miette!(
                                    "the page has no element with id {id:?} for `↙{text}↘` to read"
                                )
                            })?;
                            string_literal(lang, &value)
                        }
                        _ => text.to_string(),
                    };
                    leaf("text", &spliced)
                } else {
                    arc(QTerm::Unquote {
                        tag: tag.clone(),
                        index: *index,
                        lang: ulang.clone(),
                        term: self.resolve_refs(lang, inner, depth - usize::from(*index))?,
                        cmds: cmds.clone(),
                        span: span.clone(),
                    })
                }
            }
            QTerm::Tuple { tag, terms, cmds } => {
                let terms = terms
                    .iter()
                    .map(|t| self.resolve_refs(lang, t, depth))
                    .collect::<Result<Vec<_>>>()?;
                tuple(tag, &terms, cmds)
            }
        })
    }

    /// What a cell said, as page nodes: markup (anything with a tag or a
    /// glyph) is parsed as Quilt-in-HTML, so its quotes become cells of the
    /// next generation; plain text is shown as it was printed.
    fn output_nodes(
        &mut self,
        text: &str,
        plain_class: &str,
        generation: usize,
    ) -> Vec<Arc<QTerm>> {
        let text = text.trim_end_matches(['\n', '\r']);
        if text.trim().is_empty() {
            return Vec::new();
        }
        let markup = text.contains('<') || text.chars().any(|c| GLYPHS.contains(&c));
        if markup {
            if let Ok(frag) = self.multi.parse_chain(&["html"], text) {
                let planted = self.plant(&frag, generation + 1);
                return top_level(&planted);
            }
        }
        vec![self.pre(plain_class, text)]
    }

    /// Text as a `<pre>` of the given class.
    fn pre(&mut self, class: &str, text: &str) -> Arc<QTerm> {
        let escaped = escape_html(text);
        self.parse_html(&format!("<pre class=\"{class}\">{escaped}</pre>"))
            .unwrap_or_else(|_| leaf("text", &escaped))
    }

    /// Render the cell's figure and define it into the page, where its
    /// placeholder stands.
    fn place(&mut self, cell: &Cell, slot: &[Arc<QTerm>]) {
        let slot_html = slot
            .iter()
            .map(|n| n.coparse())
            .collect::<Vec<_>>()
            .join("\n");
        let figure = self
            .parse_html(&figure_html(cell, &slot_html))
            .or_else(|_| {
                // The output did not survive as markup inside a figure (an
                // unbalanced tag, say): show it as the text it was.
                let escaped = format!(
                    "<pre class=\"quilt-stdout\">{}</pre>",
                    escape_html(&slot_html)
                );
                self.parse_html(&figure_html(cell, &escaped))
            });
        if let Ok(figure) = figure {
            self.page.define(&figure);
        }
    }
}

/**************************************************************/

/// The cell's `<figure>`: language badge, source, output slot, and what
/// went wrong if something did.
fn figure_html(cell: &Cell, slot_html: &str) -> String {
    let mut classes = format!("quilt-cell quilt-{}", cell.lang);
    if cell.generation > 0 {
        classes.push_str(" quilt-generated");
    }
    if cell.failed() {
        classes.push_str(" quilt-failed");
    }
    let mut h = String::new();
    let _ = writeln!(
        h,
        "<figure class=\"{classes}\" id=\"{}\" data-lang=\"{}\" data-generation=\"{}\">",
        cell.element_id(),
        cell.lang,
        cell.generation
    );
    let _ = writeln!(
        h,
        "<figcaption class=\"quilt-lang\">{}</figcaption>",
        cell.lang
    );
    let _ = writeln!(
        h,
        "<pre class=\"quilt-src\"><code>{}</code></pre>",
        highlight(&cell.src)
    );
    if !slot_html.is_empty() {
        let _ = writeln!(h, "<output class=\"quilt-out\">\n{slot_html}\n</output>");
    }
    let stderr = cell.stderr.trim_end();
    if !stderr.trim().is_empty() {
        let _ = writeln!(
            h,
            "<pre class=\"quilt-stderr\">{}</pre>",
            escape_html(stderr)
        );
    }
    if let Some(error) = &cell.error {
        let _ = writeln!(
            h,
            "<pre class=\"quilt-error\">{}</pre>",
            escape_html(error.trim_end())
        );
    }
    h.push_str("</figure>");
    h
}

/// Source, escaped, with the Quilt glyphs marked so a stylesheet can color
/// them.
fn highlight(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for c in escape_html(src).chars() {
        if GLYPHS.contains(&c) {
            let _ = write!(out, "<span class=\"quilt-glyph\">{c}</span>");
        } else {
            out.push(c);
        }
    }
    out
}

/// A string literal of `lang` denoting `s` — how a page reference crosses
/// into a cell. Spelled with the same escaping rules the `LiftTo` impls use
/// for each language; a language with no string syntax gets the text.
fn string_literal(lang: &str, s: &str) -> String {
    match lang {
        "python" | "py" | "typescript" | "ts" => {
            format!("\"{}\"", crate::lift::py_dquote_escape(s))
        }
        "bash" | "zsh" => format!("\"{}\"", crate::lift::sh_dquote_escape(s)),
        "sql" => format!("'{}'", crate::lift::sql_squote_escape(s)),
        "mysql" | "mariadb" => format!("'{}'", crate::lift::mysql_squote_escape(s)),
        "rust" | "rs" => format!("{s:?}"),
        _ => s.to_string(),
    }
}

/// A cell body as Quilt source again: nested quotes keep their glyphs, a
/// deferred operator keeps its glyph, and a glyph that was *content* (the
/// author wrote `\↖`) is escaped again — so the cell re-parses as the
/// author wrote it. `coparse_quilt` makes that distinction only inside a
/// bracket, which is where the body came from; a throwaway quote around it
/// puts it back there.
fn cell_source(lang: &str, body: &Arc<QTerm>) -> String {
    let quoted = quote(
        "cell",
        1,
        lang,
        body.clone(),
        &[
            crate::term::cmd(crate::strcmd::write("↖")),
            crate::term::HOLE,
            crate::term::cmd(crate::strcmd::write("↗")),
        ],
    );
    let text = quoted.coparse_quilt();
    text.strip_prefix('↖')
        .and_then(|t| t.strip_suffix('↗'))
        .map_or(text.clone(), str::to_string)
}

/// `stdout` without its answer line — the last non-empty line, when it is
/// `value` — so a query's answer is not shown as output and as value both.
fn without_answer_line(stdout: &str, value: &str) -> String {
    let trimmed = stdout.trim_end();
    match trimmed.rfind('\n') {
        Some(at) if trimmed[at + 1..].trim() == value => trimmed[..at].to_string(),
        None if trimmed.trim() == value => String::new(),
        _ => stdout.to_string(),
    }
}

/// An error as it reads in a cell. One that points into the cell's source
/// (a parse error) is rendered with its snippet and caret; any other is its
/// messages, outermost first — a rejected feed's message already carries
/// the machine's traceback.
fn render_error(e: &miette::Report) -> String {
    let diagnostic: &dyn miette::Diagnostic = e.as_ref();
    if diagnostic.labels().is_some() && diagnostic.source_code().is_some() {
        let handler =
            miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor())
                .with_width(96);
        let mut out = String::new();
        if handler.render_report(&mut out, diagnostic).is_ok() {
            return out;
        }
    }
    e.chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_literals_follow_each_language() {
        assert_eq!(string_literal("py", "a\"b"), "\"a\\\"b\"");
        assert_eq!(string_literal("bash", "$x"), "\"\\$x\"");
        assert_eq!(string_literal("sql", "it's"), "'it''s'");
        assert_eq!(string_literal("html", "<b>"), "<b>");
    }

    #[test]
    fn the_answer_line_is_shown_once() {
        assert_eq!(without_answer_line("hi\n42\n", "42"), "hi");
        assert_eq!(without_answer_line("42", "42"), "");
        assert_eq!(without_answer_line("hi\n", "42"), "hi\n");
    }

    #[test]
    fn highlight_marks_glyphs_and_escapes_markup() {
        assert_eq!(
            highlight("<b>↖x↗"),
            "&lt;b&gt;<span class=\"quilt-glyph\">↖</span>x<span class=\"quilt-glyph\">↗</span>"
        );
    }
}
