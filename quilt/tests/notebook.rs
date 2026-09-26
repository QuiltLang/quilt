//! Notebooks end to end (`quilt::notebook`): an HTML page whose quoted cells
//! run on the machines of their languages, with the page as the HTML machine.
//!
//! Needs `python3`, `bash` and `sqlite3` on `PATH` — the same interpreters
//! the conformance battery's machine probes need. Cells that quote HTML from
//! Python need the `quilt_python` runtime (`bin/build-py`) and skip without
//! it, as the `quilt run` tests in `cli.rs` do; everything else runs on plain
//! interpreters.
#![cfg(all(feature = "parse", feature = "html", feature = "python"))]

use quilt::langs::omni::Omni;
use quilt::notebook::{run, Cell, Notebook, Rendered, MAX_GENERATION, STYLE_ID};

fn render(src: &str) -> Rendered {
    let mut multi = Omni::default();
    run(&mut multi, src).expect("the notebook opens")
}

fn cell(rendered: &Rendered, id: usize) -> &Cell {
    rendered
        .cells
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no cell {id}"))
}

fn python_runtime_built() -> bool {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quilt-python/quilt/_quilt.abi3.so")
        .exists()
}

/* ── machines persist across cells ─────────────────────────────────────── */

/// Cells of one language share that language's park machine, so a
/// definition in one cell is visible to the next — the sequencing law,
/// across cells. Three languages, three machines, in one page.
#[test]
fn cells_share_their_languages_machine() {
    let r = render(
        "<div>\n\
         py↖x = 5↗\n\
         py↖x * 2↗\n\
         bash↖n=7↗\n\
         bash↖echo $((n + 1))↗\n\
         sql↖CREATE TEMP TABLE t(v INTEGER); INSERT INTO t VALUES (40);↗\n\
         sql↖SELECT v + 2 FROM t;↗\n\
         </div>\n",
    );
    assert_eq!(r.cells.len(), 6);
    assert!(r.failures().is_empty(), "{:#?}", r.failures());
    assert_eq!(cell(&r, 2).value.as_deref(), Some("10"));
    assert_eq!(cell(&r, 4).stdout.trim(), "8");
    assert_eq!(cell(&r, 6).stdout.trim(), "42");
    assert!(
        r.html.contains("<pre class=\"quilt-value\">10</pre>"),
        "{}",
        r.html
    );
}

/// The cell's source is shown dedented, glyphs marked, and a query's
/// answer is shown once, as the value — not again as stdout.
#[test]
fn cells_render_their_source_and_answer() {
    let r = render("<section>\n  py↖\n    y = 6\n    y * 7\n  ↗\n</section>\n");
    let c = cell(&r, 1);
    assert_eq!(c.src, "y = 6\ny * 7");
    assert!(r.html.contains("<code>y = 6\ny * 7</code>"), "{}", r.html);
    // A file-shaped cell is fed whole; its trailing expression is not a
    // query, so it prints nothing. Compare with an expression cell:
    let r = render("<section>py↖6 * 7↗</section>");
    assert!(
        r.html.contains("<pre class=\"quilt-value\">42</pre>"),
        "{}",
        r.html
    );
    assert!(
        !r.html.contains("<pre class=\"quilt-stdout\">"),
        "the answer must not also appear as stdout: {}",
        r.html
    );
    assert_eq!(
        cell(&r, 1).stdout.trim(),
        "",
        "the answer line is not stdout"
    );
}

/* ── the page is the HTML machine ──────────────────────────────────────── */

/// Output that carries an `id` the page already has *redefines* that
/// element where it stands: cells edit the page. Output without one lands
/// in the cell's own slot.
#[test]
fn output_markup_redefines_ids_elsewhere_in_the_page() {
    let r = render(
        "<h1 id=\"title\">before</h1>\n\
         <p>intro</p>\n\
         py↖print('<h1 id=\"title\">after</h1><b id=\"new\">mine</b>')↗\n",
    );
    let c = cell(&r, 1);
    assert_eq!(c.edits, ["title"]);
    let title = r
        .html
        .find("<h1 id=\"title\">after</h1>")
        .expect("redefined");
    let intro = r.html.find("<p>intro</p>").expect("intro");
    let figure = r.html.find("<figure").expect("the cell");
    assert!(
        title < intro && intro < figure,
        "redefined in place: {}",
        r.html
    );
    assert!(!r.html.contains("before"), "{}", r.html);
    assert!(
        r.html
            .contains("<output class=\"quilt-out\">\n<b id=\"new\">mine</b>\n</output>"),
        "a new id stays in the slot: {}",
        r.html
    );
}

/// Plain text output — anything without a tag or a glyph — is shown as it
/// was printed, escaped.
#[test]
fn plain_text_output_is_shown_verbatim() {
    let r = render("<p>py↖print('1 < 2 &', 'done')↗</p>");
    assert!(
        r.html
            .contains("<pre class=\"quilt-stdout\">1 &lt; 2 &amp; done</pre>"),
        "{}",
        r.html
    );
}

/// An HTML cell's machine is the page: its markup is a definition.
#[test]
fn html_cells_feed_the_page() {
    let r = render("<div>html↖<p id=\"k\">kv</p>↗ py↖↙#k↘↗</div>");
    assert_eq!(cell(&r, 2).value.as_deref(), Some("'kv'"));
}

/// The default cell chrome is a definition the page can shadow.
#[test]
fn the_stylesheet_is_a_definition() {
    let r = render("<p>py↖1↗</p>");
    assert!(
        r.html.contains(&format!("<style id=\"{STYLE_ID}\">")),
        "{}",
        r.html
    );
    let r = render(&format!(
        "<style id=\"{STYLE_ID}\">.mine{{}}</style><p>py↖1↗</p>"
    ));
    assert_eq!(r.html.matches(STYLE_ID).count(), 1, "{}", r.html);
    assert!(r.html.contains(".mine{}"));
}

/* ── an unquote that reaches the page reads the page ───────────────────── */

/// `↙#id↘` in a cell is the text of that element, spliced as a string
/// literal of the cell's language — how machines that share nothing pass
/// values: through the page.
#[test]
fn an_unquote_reads_the_page_in_every_language() {
    let r = render(
        "<b id=\"n\">21</b>\n\
         py↖int(↙#n↘) * 2↗\n\
         sql↖SELECT ↙#n↘ + 1;↗\n\
         bash↖echo \"n is ↙#n↘\"↗\n",
    );
    assert!(r.failures().is_empty(), "{:#?}", r.failures());
    assert_eq!(cell(&r, 1).value.as_deref(), Some("42"));
    assert_eq!(cell(&r, 2).stdout.trim(), "22");
    assert_eq!(cell(&r, 3).stdout.trim(), "n is 21");
}

/// A reference reads the page *as it stands when the cell runs*: an earlier
/// cell's redefinition is what a later cell sees.
#[test]
fn references_see_earlier_edits() {
    let r = render(
        "<b id=\"n\">1</b>\n\
         py↖print('<b id=\"n\">2</b>')↗\n\
         py↖int(↙#n↘)↗\n",
    );
    assert_eq!(cell(&r, 2).value.as_deref(), Some("2"));
}

/// The literal is spelled by the cell's language, so text that would break
/// a bare splice cannot.
#[test]
fn references_are_string_literals() {
    let r = render("<b id=\"s\">it's \"quoted\"</b>\npy↖↙#s↘.upper()↗\nsql↖SELECT ↙#s↘;↗\n");
    assert_eq!(cell(&r, 1).value.as_deref(), Some("'IT\\'S \"QUOTED\"'"));
    assert_eq!(cell(&r, 2).stdout.trim(), "it's \"quoted\"");
}

/// An unquote whose body is not a selector splices its own text — the
/// identity host's reading, which is what lets a generated cell carry a
/// value baked in by its generator.
#[test]
fn a_non_selector_unquote_splices_its_text() {
    let r = render("<p>py↖↙40↘ + 2↗</p>");
    assert_eq!(cell(&r, 1).value.as_deref(), Some("42"));
}

/* ── failure is a cell's, not the notebook's ───────────────────────────── */

/// A missing reference, a rejected feed, a raised exception: each fails its
/// cell, renders in it, and the notebook goes on.
#[test]
fn a_failed_cell_does_not_stop_the_notebook() {
    let r = render(
        "<div>\n\
         py↖↙#nope↘↗\n\
         py↖undefined_name↗\n\
         py↖x = 1 / 0↗\n\
         bash↖false↗\n\
         py↖1 + 1↗\n\
         </div>\n",
    );
    assert_eq!(r.failures().len(), 4, "{:#?}", r.failures());
    assert!(cell(&r, 1).error.as_deref().unwrap().contains("\"nope\""));
    assert!(cell(&r, 2).error.as_deref().unwrap().contains("NameError"));
    assert!(cell(&r, 3)
        .error
        .as_deref()
        .unwrap()
        .contains("ZeroDivisionError"));
    assert!(cell(&r, 4).error.as_deref().unwrap().contains("rejected"));
    assert_eq!(cell(&r, 5).value.as_deref(), Some("2"));
    assert!(r.html.contains("quilt-failed"));
    assert!(r.html.contains("<pre class=\"quilt-error\">"));
}

/// Syntax is the notebook's to get right, not a cell's: every cell is
/// parsed with its own grammar when the page is opened, and a cell that does
/// not parse is an error of the page — the one `quilt check` reports — before
/// anything runs.
#[test]
fn a_syntax_error_fails_the_notebook_before_it_runs() {
    let mut multi = Omni::default();
    let err = run(&mut multi, "<p>py↖def oops(:↗</p>").expect_err("does not open");
    assert!(err.to_string().contains("Parsed with errors"), "{err}");
}

/// A language quilt can parse but has no machine for fails its cell with a
/// message that says so.
#[test]
fn a_language_without_a_machine_fails_its_cell() {
    let r = render("<p>wgsl↖fn f() {}↗</p>");
    let err = cell(&r, 1).error.as_deref().expect("no wgsl machine");
    assert!(err.contains("no machine"), "{err}");
}

/* ── cells create cells ────────────────────────────────────────────────── */

/// Output is read as Quilt-in-HTML: a quote in it is a new cell, run right
/// after its creator and rendered in its creator's output. A shell cell
/// generating Python cells, with the loop variable baked in — `${i}`
/// braced, because bash reads the byte after `$i` as part of the name.
#[test]
fn a_bash_cell_creates_python_cells() {
    let r = render(
        "<ul>bash↖for i in 1 2; do echo html↖<li>py↖print(↙↙${i}↘↘ * 10)↗</li>↗; done↗</ul>",
    );
    assert!(r.failures().is_empty(), "{:#?}", r.failures());
    let ids: Vec<_> = r.cells.iter().map(|c| (c.id, c.generation)).collect();
    assert_eq!(ids, [(1, 0), (2, 1), (3, 1)]);
    assert_eq!(cell(&r, 1).spawned, [2, 3]);
    assert_eq!(cell(&r, 2).stdout.trim(), "10");
    assert_eq!(cell(&r, 3).stdout.trim(), "20");
    assert!(r
        .html
        .contains("<li><figure class=\"quilt-cell quilt-py quilt-generated\""));
}

/// Generated cells run *before* the cells that follow their creator in the
/// page, so page order is execution order — and a generated cell's
/// definitions are there for the cells after it.
#[test]
fn generated_cells_run_in_page_order() {
    let r = render(
        "<ol>\n\
         bash↖echo html↖<li>py↖g = 'from gen'↗</li>↗↗\n\
         py↖g↗\n\
         </ol>\n",
    );
    let order: Vec<_> = r.cells.iter().map(|c| c.id).collect();
    assert_eq!(
        order,
        [1, 3, 2],
        "creator, its child, then the next page cell"
    );
    assert_eq!(cell(&r, 2).value.as_deref(), Some("'from gen'"));
}

/// A glyph can be written as text by escaping it, which is how a cell
/// without a runtime writes a cell: the printed `↖…↗` is a real quote in
/// the output.
#[test]
fn escaped_glyphs_are_how_plain_text_writes_a_cell() {
    let r = render("<p>py↖print('<b>py\\↖6 * 7\\↗</b>')↗</p>");
    assert_eq!(cell(&r, 2).value.as_deref(), Some("42"));
    assert_eq!(cell(&r, 2).generation, 1);
}

/// A cell whose output recreates itself is stopped at the generation cap.
/// The page holds the template — read back through `↙#tpl↘`, evaluated by
/// each generation — which is a quine with the page as its memory.
#[test]
fn generations_are_capped() {
    let r = render(
        "<pre id=\"tpl\">echo \"&lt;p&gt;bash\\↖eval \\↙#tpl\\↘\\↗&lt;/p&gt;\"</pre>\n\
         bash↖eval ↙#tpl↘↗\n",
    );
    let generations: Vec<_> = r.cells.iter().map(|c| c.generation).collect();
    assert_eq!(
        generations,
        (0..=MAX_GENERATION + 1).collect::<Vec<_>>(),
        "one cell per generation, then the refusal"
    );
    let last = r.cells.last().unwrap();
    assert!(last.failed(), "the cell past the cap must fail");
    assert!(last.error.as_deref().unwrap().contains("generation"));
    assert_eq!(r.failures().len(), 1);
}

/* ── quoting HTML from a cell (needs the runtime) ──────────────────────── */

/// A Python cell is expanded by the Python meta before it runs, so it
/// quotes HTML with `html↖…↗` and lifts into it with `↑`, as a
/// `.html.py.quilt` file would — and what it prints edits the page.
#[test]
fn python_cells_quote_html_and_lift_into_it() {
    if !python_runtime_built() {
        eprintln!("skipping python_cells_quote_html_and_lift_into_it: run `bin/build-py`");
        return;
    }
    let r = render(
        "<h1 id=\"title\">?</h1>\n\
         py↖\n\
           total = 6 * 7\n\
           print(html↖<h1 id=\"title\">Answer: ↙↑(total)↘ &amp; more</h1>↗.coparse())\n\
           print(html↖<b>↙↑('<escaped>')↘</b>↗.coparse())\n\
         ↗\n",
    );
    assert!(r.failures().is_empty(), "{:#?}", r.failures());
    assert!(
        r.html
            .contains("<h1 id=\"title\">Answer: 42 &amp; more</h1>"),
        "{}",
        r.html
    );
    assert!(
        r.html.contains("<b>&lt;escaped&gt;</b>"),
        "lifts escape: {}",
        r.html
    );
}

/// A Python cell creating Python cells with a value baked in: the value
/// crosses two quote levels, so it is unquoted twice.
#[test]
fn python_cells_create_python_cells() {
    if !python_runtime_built() {
        eprintln!("skipping python_cells_create_python_cells: run `bin/build-py`");
        return;
    }
    // (Indentation written after the `\n`, not via line continuation, which
    // strips it.)
    let r = render(concat!(
        "<div>py↖\nfor n in [2, 3]:",
        "\n    print(html↖<p>py↖print(↙↙↑(n)↘↘ ** 2)↗</p>↗.coparse())",
        "\n↗</div>\n",
    ));
    assert!(r.failures().is_empty(), "{:#?}", r.failures());
    assert_eq!(cell(&r, 1).spawned, [2, 3]);
    assert_eq!(cell(&r, 2).src, "print(↙2↘ ** 2)");
    assert_eq!(cell(&r, 2).stdout.trim(), "4");
    assert_eq!(cell(&r, 3).stdout.trim(), "9");
}

/* ── the session form ──────────────────────────────────────────────────── */

/// `Notebook::feed_source` is a notebook a fragment at a time — what
/// `quilt repl html` drives: markup defines, quotes run, the page grows.
#[test]
fn a_session_grows_the_page_a_fragment_at_a_time() {
    let mut multi = Omni::default();
    let mut nb = Notebook::new(&mut multi);
    assert!(nb.feed_source("<p id=\"x\">5</p>").unwrap().is_empty());
    assert_eq!(nb.feed_source("py↖int(↙#x↘) * 2↗").unwrap(), [1]);
    assert_eq!(nb.cells()[0].value.as_deref(), Some("10"));
    nb.feed_source("<p id=\"x\">6</p>").unwrap();
    assert_eq!(nb.page().text("x").as_deref(), Some("6"));
    assert_eq!(nb.feed_source("py↖int(↙#x↘) * 2↗").unwrap(), [2]);
    assert_eq!(nb.cells()[1].value.as_deref(), Some("12"));
    let html = nb.render();
    assert_eq!(
        html.matches("<p id=\"x\">").count(),
        1,
        "redefined, not appended: {html}"
    );
    assert_eq!(html.matches("<figure").count(), 2);
}
