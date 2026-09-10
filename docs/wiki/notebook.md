# Notebooks

**Files:** `quilt/src/notebook.rs`, `quilt/src/machine/html.rs`, `quilt/src/langs/html/meta.rs`, `examples/notebook/tour.html.quilt`

A notebook is an `.html.quilt` file: ordinary HTML with *cells* spliced in as
quotes of other languages.

```html
<h1 id="headline">?</h1>
<div class="row">
  py↖
    total = 6 * 7
    print(html↖<h1 id="headline">Answer: ↙↑(total)↘</h1>↗.coparse())
  ↗
  sql↖SELECT ↙#headline↘;↗
</div>
```

```sh
quilt notebook notes.html.quilt          # writes notes.html, cells run and rendered in place
quilt notebook notes.html.quilt --open   # …and opens it
quilt check notes.html.quilt             # validates every cell's syntax; runs nothing
quilt repl html                          # the same thing a line at a time
```

HTML is the ground language, so the page is the program and the cells are its
quotes. Arranging cells is the page's business — a grid, a table row, a
`<details>` fold, a sidebar — the notebook only draws each cell's chrome (source,
output, errors) into a `<figure>` where the quote stood. The example
[`examples/notebook/tour.html.quilt`](../../examples/notebook/tour.html.quilt)
(rendered: [`tour.html`](../../examples/notebook/tour.html)) walks through
everything below.

## Machines, all the way down

Running a notebook is machines running programs (see
[machines](../design/machines.md)):

- **Each cell runs on its language's park machine** (`Multi::machine`): the
  persistent Python kernel, a live `sqlite3`, a long-lived `bash`/`zsh`, a Node
  context. Definitions persist from cell to cell — `x = 5` in one Python cell,
  `x * 2` in the next — exactly as they persist across lines of `quilt repl`.
  Every language with a machine can be a cell: `py`, `ts`, `sql`, `bash`, `zsh`
  (and `html`, below). A language quilt can parse but cannot run (`wgsl`,
  `rs`, …) fails its cell with a message saying so.
- **The page is the HTML machine** (`HtmlMachine`): a document held as a term,
  whose *definitions are its ids*. Each cell is planted as a placeholder
  element, and everything a cell produces is fed back to the page. An `html`
  cell (a bare `↖…↗`, or `html↖…↗`) feeds the page directly.
- **A cell is re-parsed as a program of its own language** and expanded by that
  language's meta before it is fed, so a Python cell quotes HTML with
  `html↖…↗` and lifts into it with `↑` exactly as a `.html.py.quilt` file would
  (the runtime, `bin/build-py`, is pre-imported by the kernel). The quote
  annotation is the cell's language; un-annotated quotes *inside* a cell default
  to the cell's language, so HTML is always written `html↖…↗`.

What a cell says is read back as **markup or text**: output containing a tag
(or a glyph) joins the page as elements; plain text is shown verbatim in a
`<pre>`. A query cell — a single expression — shows its answer as the value.

## Cells edit the page

Output that carries an `id` the page already has **redefines** that element,
wherever the author put it; ids are the page's namespace, and a cell that
prints `<p id="summary">…</p>` edits the summary at the top of the page. Output
without a known id lands in the cell's own output slot — and from then on its
ids are defined too, for later cells to redefine or read.

## The page is the bus

Machines share nothing with each other (the isolation law); what they share is
the page. **An unquote that reaches the page reads the page**: `↙#total↘` in a
cell is the text content of the element with that id, spliced into the cell as
a *string literal of the cell's language* — `"42.5"` in Python, `'42.5'` in
SQL, `"42.5"` in a shell — spelled with the same escaping rules the `LiftTo`
impls use. So SQL answers into `<b id="total">…</b>`, and Python, bash and
TypeScript read `↙#total↘` back. A reference reads the page *as it stands when
the cell runs*, so an earlier cell's edit is what a later cell sees; a missing
id fails the cell.

An unquote whose body is not a selector splices its own text (`↙40↘` is `40`),
which is the identity host's reading — and what lets a generated cell carry a
value its generator baked in.

## Cells create cells

Output is parsed as **Quilt-in-HTML**, not merely as HTML: a quote in a cell's
output is a new cell. It is planted where it appeared, run *next* — before the
cells that follow its creator in the page, so page order stays execution order
— and rendered in place with a dashed edge (`data-generation="1"`). A generated
cell's definitions are there for the cells after it, like any other.

Quilt's staging depth is what makes this natural. From a shell:

```html
bash↖for i in 1 2 3; do echo html↖<li>py↖print(↙↙${i}↘↘ * 10)↗</li>↗; done↗
```

The `html↖…↗` quote is the cell's own stage; the `py↖…↗` inside it belongs to
the *next* stage, so the shell meta re-emits it with its glyphs; and `${i}`,
unquoted twice, reaches the shell's ground and is expanded at run time.
(Braced, because bash reads the byte after `$i` as part of the variable name —
`$i↘` is `${i↘}`.)

From Python, build the new cell as a **term of its language first** — `↑` lifts
into it correctly there — and splice the term two quote levels up:

```python
py↖
  for name in ["latte", "cortado"]:
      cell = py↖print(↙↑(name)↘, "costs", menu[↙↑(name)↘])↗
      print(html↖<div>py↖↙↙cell↘↘↗</div>↗.coparse())
↗
```

(`↙↙↑(name)↘↘` directly inside the `html↖…↗` quote would lift into *HTML* — the
enclosing quote's language — and land in the generated Python as bare text.)

A cell without a runtime can still write a cell: a glyph escaped with `\` is
content, so `print('<b>py\↖6 * 7\↗</b>')` prints a real quote. And a cell whose
output recreates itself is stopped at `MAX_GENERATION` (8) with an error in the
cell that crossed the line.

## Failure is a cell's

A rejected feed (an exception, a non-zero shell status), a missing reference,
a language without a machine: each fails its own cell, renders the error in
it (`quilt-failed`), and the notebook goes on, as a REPL would. `quilt
notebook --strict` makes any failure the exit code; the page is written either
way.

Syntax is different: every cell is parsed with its own grammar when the page is
*opened*, so a cell that does not parse is an error of the notebook — the one
`quilt check` reports, with the caret in the cell — before anything runs.

## The CLI

| Command | What it does |
|---|---|
| `quilt notebook F.html.quilt` | Run and write `F.html` beside it (`-o` to choose, `--stdout` to print, `--open` to open, `--strict` to fail on a failed cell). The page starts with a `<!-- DO NOT EDIT … -->` header. |
| `quilt expand F.html.quilt` | The same: the identity expansion is nothing anyone wants written, so `expand` on an HTML-ground file renders the notebook (and never caches). |
| `quilt run F.html.quilt` | `--stdout`, so a notebook with a `#!/usr/bin/env quilt` shebang is a script whose output is its page. |
| `quilt check F.html.quilt` | Parse every cell with its grammar; run nothing. |
| `quilt repl html` | A notebook session a line at a time: markup defines (by id) or appends, `lang↖…↗` runs a cell, `#id` reads an element back, and the page is printed at the end. The polyglot REPL — every line names its language, every language's definitions persist. |

## The HTML machine on its own

The HTML machine is independent of notebooks. `html⟨M⟩` spawns one from a
Python or Rust program (it is the native provider, preferred over the
subprocess ones), `quilt repl html` talks to one, and the conformance battery
holds it to the same laws as every other machine:

```python
page = html⟨M⟩                            # spawn("html")
page.↓(↖<p id="y">40</p>↗)                # a definition: replace #y, or append
page.↓(↖#y↘)                              # '<p id="y">40</p>' — the value's literal
page.⟨T⟩(↖#y↗)                            # 'p' — the type of an HTML value is its tag
```

It implements `SnapshotMachine` (the snapshot is the document) and
`IntrospectMachine` (the definitions are the ids). Fed fragments have their
layout *baked* so they render exactly as fed wherever they land — inside a
`<pre>` the enclosing indentation would be content, not layout.

## Styling

The notebook adds one stylesheet, `<style id="quilt-cells">`, unless the page
defines an element with that id — define your own to take over the chrome. The
classes: `.quilt-cell` (with `.quilt-<lang>`, `.quilt-generated`,
`.quilt-failed`, `.quilt-pending`), `.quilt-lang`, `.quilt-src` (glyphs wrapped
in `.quilt-glyph`), `.quilt-out`, `.quilt-stdout`, `.quilt-value`,
`.quilt-stderr`, `.quilt-error`.
