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
| `quilt notebook --serve [F.html.quilt]` | The same session held open behind a socket: an editor, the live page, and the site the cells build. See [Live notebooks](#live-notebooks). |

## Live notebooks

**Files:** `quilt/src/serve.rs`, `quilt/src/serve/` (the editor),
[`examples/notebook/cafe.html.quilt`](../../examples/notebook/cafe.html.quilt)

```sh
quilt notebook --serve                     # an empty page, a scratchpad
quilt notebook --serve notes.html.quilt    # a page, its cells pending
quilt notebook --serve notes.html.quilt --run --open   # …run them, and open it
```

A session prints a URL with a token in it and listens on `127.0.0.1`. The
window is the editor on the left and the **live page** on the right: write a
cell, expand it to see the program it becomes, run it, and watch the page
change. Nothing new happens to a cell — `Notebook::run_cell` is what a file run
calls too — what is new is who asks, and when.

Four things are worth knowing.

**The page is the truth; the DOM is a view.** The server's `HtmlMachine` holds
the document. Every edit a cell makes is journalled as a `Change` and pushed to
viewers over `GET /events`, which patch by id. (The other way round — DOM as
truth — would make the browser cheaper and every Python `↙#id↘` a race against
a page the server has not seen.) An edit made *in* the page goes home with
`PUT /page/{id}`, so the next cell's `↙#id↘` reads what the viewer is looking
at.

**The browser is a machine.** A TypeScript cell is expanded here and
*evaluated there*, in the realm of the page frame — the "browser as a
JavaScript machine" of [machines](../design/machines.md), reached from inside
the page rather than over CDP. Its `document` is the page, its event handlers
outlive the cell, its `const`s are there for the next TS cell, and it can
`fetch` the backend. The answer comes back as an ordinary `Answer` and is
landed by `Notebook::land_cell`, which cannot tell where it was produced — a
value that is a quilt term answers with its markup, so a TS cell that builds
HTML lands in the page like any other cell's output. A cell that *quotes* needs
the runtime in the page: `wasm-pack build quilt-wasm --target web --out-dir
pkg-web` (the editor says so if it is missing). `--ts server` runs them on the
node kernel with everything else instead.

**Python cells can serve.** The session feeds the Python machine a prelude with
two names in it:

```python
@route("/menu")                 # serves GET /app/menu
def menu():
    rows = db().execute("SELECT item, price FROM menu").fetchall()
    return "<ul>" + "".join(f"<li>{r['item']}: {r['price']}" for r in rows) + "</ul>"
```

`route(path, methods=("GET",))` registers a handler *in the kernel*; a handler
taking one argument is given the request as a dict (`method`, `path`, `query`,
`body`). What it returns is the response: a `str` is the body (HTML if it
starts with `<`), a `dict` or `list` is JSON, `(status, value)` sets the
status, `None` is 204, and an exception is a 500 with its traceback. A request
to `/app/…` becomes a **feed** — `__quilt_dispatch(…)` on the kernel's stdin,
the response read back off its stdout — so there is no second port, no thread
inside the kernel, and every request is serialized with cell execution. A
handler that needs real concurrency can still open its own listener.

`db()` is a connection to the **session database**: the server keeps a
`sqlite3` file in a temp dir, parks the SQL machine on it, and hands its path
to the Python kernel as `QUILT_DB` — so a route sees the tables the SQL cells
built.

**The page is the site.** `GET /site` is the page with the cell chrome gone.
SQL cells fill the database, Python cells serve `/app/…` from it, TypeScript
cells render into the page's own elements — and what is left when the
scaffolding is removed is a small website.

Nothing persists: the machines die with the process and the temp dir goes with
them. `GET /export` (the **Export** button) is the way out — the same page
`quilt notebook` would have written.

### The protocol

Everything the editor does, a `curl` can do. The notebook's own endpoints want
the session token (`?token=…`, the `quilt_token` cookie, or `X-Quilt-Token`);
`/site` and `/app/…` are open, because they are the website the session exists
to build.

| Route | What it does |
|---|---|
| `GET /` | The editor (`/ui.css`, `/ui.js`, `/runtime.js` beside it) |
| `GET /events` | Server-sent events: `hello` (the whole state), `page` (patches), `cells`, `app` |
| `GET /cells`, `POST /cells` | List cells; create one (`{lang, src, after?}`) or edit by `id` |
| `PUT`/`DELETE /cells/{id}` | Edit, remove |
| `POST /cells/{id}/run` | Run it — or, for a browser cell, answer `{ran: "browser", program}` |
| `POST /cells/{id}/result` | Land an answer produced elsewhere (`{value, stdout, stderr, error}`) |
| `GET /cells/{id}/expand`, `POST /expand` | What a cell becomes: `{kind, resolved, program}` |
| `GET /page`, `GET /view`, `GET /page/{id}`, `PUT /page/{id}` | The document, the frame's copy of it, one element, define one element |
| `GET /site`, `GET /export` | The page without the chrome; the page as a file |
| `ANY /app/…` | The routes the Python cells registered |
| `POST /shutdown` | End the session |

### What a session does not do

- **One notebook, one mutex.** A cell run, a route dispatch and a page read are
  the same lock: a slow cell delays the next request. That is what machines
  that are single interpreters give you, and it is the order a one-shot run has
  anyway.
- **A cell created by a cell's output runs on the park**, even a TypeScript one
  — it runs as part of its creator's run, before any browser could be asked.
- **No auth, one session, one user.** Cells run with the user's privileges,
  exactly as `↓` and `quilt run` already do. The token and the `Host` check
  keep another page in another tab from driving the notebook; they are not a
  sandbox.

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
