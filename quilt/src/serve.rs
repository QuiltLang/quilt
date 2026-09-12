//! Live notebooks: `quilt notebook --serve`.
//!
//! `quilt notebook FILE` runs a page once and writes it out. This is the same
//! [`Notebook`] session held open behind a socket, so the cells can be
//! written, expanded, run and re-run while someone watches the page change.
//! Nothing new happens to a cell here — [`Notebook::run_cell`] is what the
//! file path calls too — what is new is *who* asks, and when.
//!
//! # The shape
//!
//! * **The page is the truth, the DOM is a view.** The server's
//!   [`HtmlMachine`](crate::machine::HtmlMachine) holds the document; every
//!   edit a cell makes is journalled ([`Change`]) and pushed to viewers over
//!   `GET /events`, which patch by id. The other way round — DOM as truth,
//!   server mirroring — would make the browser cheaper and every Python
//!   `↙#id↘` a race against a page the server has not seen yet. Ids are the
//!   page's definitions; definitions live with the machine.
//! * **The browser is a machine.** A TypeScript cell's program is expanded
//!   here and *evaluated there*, in the realm of the page itself, so its
//!   `document` is the page, its event handlers outlive the cell, and its
//!   `const`s are visible to the next TS cell — the "browser as a JavaScript
//!   machine" of `docs/design/machines.md`, reached from inside the page
//!   rather than over CDP. The answer comes back as an [`Answer`] and is
//!   landed by [`Notebook::land_cell`], which cannot tell where it was
//!   produced. `--ts server` runs them on the node kernel instead.
//! * **Python cells can serve.** The session feeds the Python machine a
//!   prelude with a `route` decorator; `/app/…` requests are dispatched *as a
//!   feed* — `__quilt_dispatch(…)` on the kernel's stdin, the response read
//!   back off stdout. No second port, no thread inside the kernel, and every
//!   request serialized with cell execution, which is what a session wants.
//!   A handler that needs real concurrency can still open its own listener.
//! * **One database.** The session keeps a `sqlite3` file in a temp dir,
//!   parks the SQL machine on it, and hands its path to the Python kernel as
//!   `QUILT_DB`, so `db()` in a route sees the tables the SQL cells built.
//! * **The page is the site.** `GET /site` is the page with the cell chrome
//!   gone: SQL cells fill the database, Python cells serve `/app/…` from it,
//!   TypeScript cells render into the page's own elements — and what is left
//!   when the scaffolding is removed is a small website.
//!
//! Nothing persists: the machines die with the process and the temp dir goes
//! with them. `GET /export` is the way out — the same page `quilt notebook`
//! would have written.
//!
//! # What holds it together
//!
//! One [`Notebook`] behind one mutex. A cell run, a route dispatch and a page
//! read are all the same lock, so a slow cell delays the next request — the
//! honest consequence of machines that are single interpreters, and the same
//! order a one-shot run has. Viewers are notified from a second, uncontended
//! lock, so a stream cannot be blocked by a cell.
//!
//! # Reaching it
//!
//! The listener binds `127.0.0.1` only, and the notebook's own endpoints want
//! a session token — printed in the URL at startup, kept in a `SameSite`
//! cookie afterwards — so a page in another tab cannot drive the notebook.
//! `GET /site` and `/app/…` are open, because they are the website the
//! session exists to build. Cells run with the user's privileges, exactly as
//! `↓` and `quilt run` already do.

pub mod http;

use std::fmt::Write as _;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

use miette::IntoDiagnostic;
use serde_json::{json, Value};

use crate::lang::InnerKind;
use crate::machine::{Answer, ReplMachine};
use crate::multi::{Languages, MetaLanguages, Multi};
use crate::notebook::{cell_element_id, Cell, CellDef, Change, Notebook};
use crate::prelude::*;

use http::{Event, Reply, Request, Response};

/**************************************************************/

/// The default port. Arbitrary, and above anything a system service wants.
pub const DEFAULT_PORT: u16 = 8788;

/// The cookie a viewer keeps its session token in.
const TOKEN_COOKIE: &str = "quilt_token";

/// The line the Python dispatcher prints its response on.
const RESPONSE_MARK: &str = "__QUILT_RESP__";

/// Where the browser-side runtime is looked for, relative to this crate —
/// `wasm-pack build quilt-wasm --target web --out-dir pkg-web` (what
/// `examples/web/build.mjs` runs) puts it there. Absent, TypeScript cells
/// still run; only the ones that *quote* need the builders.
const WASM_PKG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../quilt-wasm/pkg-web");

/// Where a TypeScript cell runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    /// In the page's own realm, in the viewer's browser.
    #[default]
    Browser,
    /// On the node kernel in the park, like every other language.
    Server,
}

/// How to run the session.
#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    /// Open the notebook in the default browser once it is listening.
    pub open: bool,
    /// Run the cells the page came with at startup, rather than leaving them
    /// pending.
    pub run: bool,
    pub typescript: Engine,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            port: DEFAULT_PORT,
            open: false,
            run: false,
            typescript: Engine::default(),
        }
    }
}

/// The session, once it is listening.
#[derive(Debug, Clone)]
pub struct Ready {
    pub addr: SocketAddr,
    pub token: String,
    /// The URL to open: the address with the session token in it.
    pub url: String,
}

/**************************************************************/

/// Serve a notebook session until `POST /shutdown`, printing where it is.
pub fn serve<LS, MS>(multi: &mut Multi<LS, MS>, src: Option<&str>, cfg: &Config) -> Result<()>
where
    LS: Languages + Send,
    MS: MetaLanguages + Send,
{
    serve_with(multi, src, cfg, |ready| {
        eprintln!("quilt notebook — the page is live at {}", ready.url);
        eprintln!("  the site it builds: http://{}/site", ready.addr);
        eprintln!("  ctrl-C, or POST /shutdown, ends the session; nothing persists");
        if cfg.open {
            open_in_browser(&ready.url);
        }
    })
}

/// [`serve`], telling `ready` where it landed — which is how a caller that
/// asked for port 0 finds out, and how a test drives it.
pub fn serve_with<LS, MS>(
    multi: &mut Multi<LS, MS>,
    src: Option<&str>,
    cfg: &Config,
    ready: impl FnOnce(&Ready),
) -> Result<()>
where
    LS: Languages + Send,
    MS: MetaLanguages + Send,
{
    let notebook = match src {
        Some(src) => Notebook::open(multi, src)?,
        None => Notebook::new(multi),
    };
    let dir = tempfile::Builder::new()
        .prefix("quilt-notebook-")
        .tempdir()
        .into_diagnostic()?;
    let db = dir.path().join("session.db");
    let mut session = Session {
        notebook,
        db,
        typescript: cfg.typescript,
        python_ready: false,
    };
    if cfg.run {
        session.run_all()?;
    }
    let listener = TcpListener::bind(("127.0.0.1", cfg.port)).into_diagnostic()?;
    let addr = listener.local_addr().into_diagnostic()?;
    let token = session_token();
    let live = Live {
        session: Mutex::new(session),
        viewers: Mutex::new(Vec::new()),
        token: token.clone(),
        addr,
        stop: AtomicBool::new(false),
    };
    ready(&Ready {
        addr,
        token: token.clone(),
        url: format!("http://{addr}/?token={token}"),
    });

    std::thread::scope(|scope| {
        for stream in listener.incoming() {
            if live.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            scope.spawn(|| http::serve_connection(stream, &|req| handle(&live, req)));
        }
        // Dropping every sender ends the streams, so the scope can join.
        live.viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    });
    // `dir` drops here: the session's database goes with the session.
    drop(dir);
    Ok(())
}

/**************************************************************/

/// The session: one notebook, and what the session added to it.
struct Session<'m, LS: Languages, MS: MetaLanguages> {
    notebook: Notebook<'m, LS, MS>,
    /// The `sqlite3` file the SQL cells and the Python routes share.
    db: PathBuf,
    typescript: Engine,
    /// Whether the Python machine has been given the session prelude.
    python_ready: bool,
}

/// Everything a connection thread can reach.
struct Live<'m, LS: Languages, MS: MetaLanguages> {
    session: Mutex<Session<'m, LS, MS>>,
    /// The event streams to push page changes to. A second lock, so a cell
    /// that takes a minute does not hold up a viewer's stream.
    viewers: Mutex<Vec<Sender<Event>>>,
    token: String,
    addr: SocketAddr,
    stop: AtomicBool,
}

impl<LS: Languages, MS: MetaLanguages> Session<'_, LS, MS> {
    /// Park the session's own machines before *any* cell runs.
    ///
    /// Not per-cell, because a cell's output creates cells: a bash cell that
    /// writes a Python cell would otherwise have the park spawn a plain
    /// Python machine — no `route`, no `db` — halfway through a run. The
    /// session's machines have to be in place before the first feed, so this
    /// is called before every run rather than for the language being run.
    ///
    /// A machine that cannot start (no `sqlite3`, no `python3`) is left
    /// unparked: the cell that wants it then fails with the park's own
    /// message, which says what is missing, instead of the session refusing
    /// to run anything.
    fn ensure_session(&mut self) {
        for lang in ["sql", "py"] {
            if let Err(e) = self.ensure(lang) {
                tracing::debug!("no session machine for {lang}: {e}");
            }
        }
    }

    /// Run every cell the page came with, in page order — `--run`. Cells
    /// whose machine is a browser are left pending: there is no viewer yet,
    /// and the page is the one thing a session can wait for.
    fn run_all(&mut self) -> Result<()> {
        self.ensure_session();
        let cells: Vec<(usize, Box<str>)> = self
            .notebook
            .cell_defs()
            .iter()
            .map(|def| (def.id, def.lang.clone()))
            .collect();
        for (id, lang) in cells {
            if self.delegates(&lang) {
                continue;
            }
            self.notebook.run_cell(id)?;
        }
        Ok(())
    }

    /// Make sure `lang`'s machine is the *session's* machine before anything
    /// spawns the language's default one. SQL gets a file to write in; Python
    /// gets that file's path and the session prelude.
    fn ensure(&mut self, lang: &str) -> Result<()> {
        match lang {
            "sql" | "mysql" | "mariadb" if !self.notebook.has_machine("sql") => {
                let Some(mut spec) = crate::machine::repl_spec("sql") else {
                    return Ok(());
                };
                // `sqlite3 <file>` rather than the default in-memory database,
                // so a Python route can open the same tables.
                let mut args: Vec<Box<str>> = spec.args.to_vec();
                args.push(self.db.display().to_string().into());
                spec.args = args.into();
                let machine = ReplMachine::spawn("sql", spec)?;
                self.notebook.park_machine("sql", Box::new(machine));
            }
            "py" | "python" if !self.python_ready => {
                if !self.notebook.has_machine("py") {
                    let Some(mut spec) = crate::machine::repl_spec("py") else {
                        return Ok(());
                    };
                    let mut env: Vec<(Box<str>, Box<str>)> = spec.env.to_vec();
                    env.push(("QUILT_DB".into(), self.db.display().to_string().into()));
                    spec.env = env.into();
                    let machine = ReplMachine::spawn("py", spec)?;
                    self.notebook.park_machine("py", Box::new(machine));
                }
                self.notebook
                    .feed_lang("py", InnerKind::File, PYTHON_PRELUDE)?;
                self.python_ready = true;
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether this cell's machine is the viewer's browser.
    fn delegates(&self, lang: &str) -> bool {
        self.typescript == Engine::Browser && matches!(lang, "ts" | "typescript")
    }
}

/**************************************************************/

/// Route one request. Errors become a JSON `error`, so the editor can show
/// what went wrong where it asked.
fn handle<LS, MS>(live: &Live<'_, LS, MS>, req: &Request) -> Reply
where
    LS: Languages,
    MS: MetaLanguages,
{
    if !local_host(req) {
        return Reply::Done(Response::status(
            403,
            "a quilt notebook serves 127.0.0.1 under its own name only",
        ));
    }
    if !authorized(live, req) {
        return Reply::Done(Response::status(
            403,
            "this notebook wants its session token: open the URL quilt printed",
        ));
    }
    match route(live, req) {
        Ok(reply) => reply,
        Err(e) => Reply::Done(Response::new(
            500,
            "application/json; charset=utf-8",
            json!({ "error": format!("{e:?}") }).to_string(),
        )),
    }
}

#[allow(clippy::too_many_lines)]
fn route<LS, MS>(live: &Live<'_, LS, MS>, req: &Request) -> Result<Reply>
where
    LS: Languages,
    MS: MetaLanguages,
{
    let segments = req.segments();
    let done = |response: Response| Ok(Reply::Done(response));
    match (req.method.as_str(), segments.as_slice()) {
        /* ── the editor ──────────────────────────────────────────────── */
        ("GET", []) => {
            let mut page = Response::html(assets::UI_HTML);
            if let Some(token) = req.param("token") {
                page = page.with_header(
                    "Set-Cookie",
                    &format!("{TOKEN_COOKIE}={token}; Path=/; SameSite=Strict"),
                );
            }
            done(page)
        }
        ("GET", ["ui.css"]) => done(Response::new(
            200,
            "text/css; charset=utf-8",
            assets::UI_CSS,
        )),
        ("GET", ["ui.js"]) => done(Response::new(
            200,
            "text/javascript; charset=utf-8",
            assets::UI_JS,
        )),
        ("GET", ["runtime.js"]) => done(Response::new(
            200,
            "text/javascript; charset=utf-8",
            runtime_js(),
        )),
        ("GET", ["quilt-wasm", file]) => done(wasm_asset(file)),

        /* ── events ──────────────────────────────────────────────────── */
        ("GET", ["events"]) => {
            let (tx, rx) = channel();
            // The first event is the whole state, so a viewer that connects
            // late is not looking at a page it missed the changes to.
            let hello = {
                let mut session = lock(live);
                Event {
                    name: "hello".into(),
                    data: state_json(&mut session, live).to_string(),
                }
            };
            let _ = tx.send(hello);
            live.viewers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(tx);
            Ok(Reply::Events(rx))
        }

        /* ── the page ────────────────────────────────────────────────── */
        ("GET", ["page"]) => {
            let session = lock(live);
            done(Response::json(
                json!({ "html": session.notebook.render() }).to_string(),
            ))
        }
        // The document itself, for the viewer's frame: exactly what the
        // machine holds, with nothing injected — the frame's runtime is
        // installed from outside, so the page stays the page.
        // Not under `/page/`, where it would shadow an element whose id is
        // `view` — the page's ids are the page's, not the protocol's.
        ("GET", ["view"]) => {
            let session = lock(live);
            done(Response::html(session.notebook.render()))
        }
        ("GET", ["page", id]) => {
            let session = lock(live);
            match session.notebook.page().find(id) {
                Some(el) => done(Response::json(
                    json!({
                        "id": id,
                        "html": el.coparse(),
                        "text": session.notebook.page().text(id),
                    })
                    .to_string(),
                )),
                None => done(Response::status(404, &format!("no element with id {id:?}"))),
            }
        }
        // A viewer's edit coming home: the page the cells read is this one.
        ("PUT", ["page", id]) => {
            let html = string_field(&body(req)?, "html")?;
            let mut session = lock(live);
            let element = session.notebook.define_html(&html)?;
            let changes = drain(&mut session, live);
            done(Response::json(
                json!({ "id": id, "defined": element, "changes": changes }).to_string(),
            ))
        }
        ("GET", ["site"]) => {
            let session = lock(live);
            done(Response::html(session.notebook.site_html()))
        }
        ("GET", ["export"]) => {
            let session = lock(live);
            let html = format!(
                "<!-- DO NOT EDIT. GENERATED BY `quilt notebook --serve`. -->\n{}\n",
                session.notebook.render()
            );
            done(Response::html(html).with_header(
                "Content-Disposition",
                "attachment; filename=\"notebook.html\"",
            ))
        }

        /* ── cells ───────────────────────────────────────────────────── */
        ("GET", ["cells"]) => {
            let session = lock(live);
            done(Response::json(cells_json(&session).to_string()))
        }
        ("POST", ["cells"]) => {
            let body = body(req)?;
            let lang = string_field(&body, "lang")?;
            let src = body
                .get("src")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let after = body.get("after").and_then(Value::as_str);
            let mut session = lock(live);
            let id = match body.get("id").and_then(Value::as_u64) {
                Some(id) => {
                    let id = usize::try_from(id).into_diagnostic()?;
                    session.notebook.set_cell(id, Some(&lang), &src)?;
                    id
                }
                None => session.notebook.add_cell(&lang, &src, after)?,
            };
            let answer = cell_reply(&mut session, live, id);
            done(Response::json(answer.to_string()))
        }
        ("PUT", ["cells", id]) => {
            let id = cell_number(id)?;
            let body = body(req)?;
            let lang = body.get("lang").and_then(Value::as_str);
            let src = string_field(&body, "src")?;
            let mut session = lock(live);
            session.notebook.set_cell(id, lang, &src)?;
            let answer = cell_reply(&mut session, live, id);
            done(Response::json(answer.to_string()))
        }
        ("DELETE", ["cells", id]) => {
            let id = cell_number(id)?;
            let mut session = lock(live);
            session.notebook.remove_cell(id)?;
            let changes = drain(&mut session, live);
            done(Response::json(
                json!({ "id": id, "removed": true, "changes": changes }).to_string(),
            ))
        }
        ("GET", ["cells", id, "expand"]) => {
            let id = cell_number(id)?;
            let mut session = lock(live);
            let program = session.notebook.expand_cell(id)?;
            done(Response::json(program_json(&program).to_string()))
        }
        ("POST", ["cells", id, "run"]) => {
            let id = cell_number(id)?;
            let mut session = lock(live);
            let Some(def) = session.notebook.cell_def(id).cloned() else {
                return done(Response::status(404, &format!("no cell {id}")));
            };
            if session.delegates(&def.lang) {
                // The machine is the viewer's browser: hand over the program
                // and wait for `…/result`.
                let program = session.notebook.expand_cell(id)?;
                let mut answer = program_json(&program);
                answer["id"] = json!(id);
                answer["ran"] = json!("browser");
                return done(Response::json(answer.to_string()));
            }
            session.ensure_session();
            session.notebook.run_cell(id)?;
            let mut answer = cell_reply(&mut session, live, id);
            answer["ran"] = json!("server");
            done(Response::json(answer.to_string()))
        }
        // A machine that is not in this process, answering.
        ("POST", ["cells", id, "result"]) => {
            let id = cell_number(id)?;
            let body = body(req)?;
            let mut session = lock(live);
            // A cell the browser ran can still create cells, and those run
            // here — on the session's machines.
            session.ensure_session();
            session.notebook.land_cell(id, answer_from(&body))?;
            let mut answer = cell_reply(&mut session, live, id);
            answer["ran"] = json!("browser");
            done(Response::json(answer.to_string()))
        }

        /* ── the expander, for anyone ────────────────────────────────── */
        ("POST", ["expand"]) => {
            let body = body(req)?;
            let lang = string_field(&body, "lang")?;
            let src = string_field(&body, "src")?;
            let mut session = lock(live);
            let program = session.notebook.expand_source(&lang, &src)?;
            done(Response::json(program_json(&program).to_string()))
        }

        /* ── the site the cells are building ─────────────────────────── */
        (_, ["app", rest @ ..]) => {
            let path = format!("/{}", rest.join("/"));
            let mut session = lock(live);
            let (response, stdout) = dispatch_app(&mut session, &req.method, &path, req)?;
            if !stdout.trim().is_empty() {
                notify(live, "app", &json!({ "path": path, "stdout": stdout }));
            }
            done(response)
        }

        /* ── the end ─────────────────────────────────────────────────── */
        ("POST", ["shutdown"]) => {
            live.stop.store(true, Ordering::SeqCst);
            // Wake the accept loop, which is blocked on a socket that will
            // not get another connection on its own.
            let _ = TcpStream::connect(live.addr);
            done(Response::text("the session is over\n"))
        }
        _ => done(Response::status(
            404,
            &format!("no route {} {}", req.method, req.path),
        )),
    }
}

/**************************************************************/

fn lock<'a, 'm, LS: Languages, MS: MetaLanguages>(
    live: &'a Live<'m, LS, MS>,
) -> std::sync::MutexGuard<'a, Session<'m, LS, MS>> {
    // A panicking handler leaves the notebook as it was mid-edit; the next
    // request is still better served than refused.
    live.session.lock().unwrap_or_else(|e| e.into_inner())
}

/// Take what the page did, tell every viewer, and answer it to the caller
/// too — the caller is a viewer that asked a question.
fn drain<LS: Languages, MS: MetaLanguages>(
    session: &mut Session<'_, LS, MS>,
    live: &Live<'_, LS, MS>,
) -> Value {
    let changes: Vec<Value> = session
        .notebook
        .take_changes()
        .iter()
        .map(change_json)
        .collect();
    if !changes.is_empty() {
        notify(live, "page", &json!({ "changes": changes }));
    }
    Value::Array(changes)
}

/// Send an event to every viewer, dropping the ones that have gone.
fn notify<LS: Languages, MS: MetaLanguages>(live: &Live<'_, LS, MS>, name: &str, data: &Value) {
    let mut viewers = live.viewers.lock().unwrap_or_else(|e| e.into_inner());
    let data = data.to_string();
    viewers.retain(|tx| {
        tx.send(Event {
            name: name.to_string(),
            data: data.clone(),
        })
        .is_ok()
    });
}

/// One cell's state, its page changes, and the cell list — what every editing
/// request answers, because every one of them can move all three.
fn cell_reply<LS: Languages, MS: MetaLanguages>(
    session: &mut Session<'_, LS, MS>,
    live: &Live<'_, LS, MS>,
    id: usize,
) -> Value {
    let changes = drain(session, live);
    let cells = cells_json(session);
    notify(live, "cells", &cells);
    json!({
        "id": id,
        "cell": session
            .notebook
            .cell_def(id)
            .map(|def| cell_json(def, session.notebook.cell(id))),
        "figure": session.notebook.cell_figure(id),
        "changes": changes,
        "cells": cells["cells"],
    })
}

fn state_json<LS: Languages, MS: MetaLanguages>(
    session: &mut Session<'_, LS, MS>,
    live: &Live<'_, LS, MS>,
) -> Value {
    let _ = live;
    json!({
        "html": session.notebook.render(),
        "cells": cells_json(session)["cells"],
        "typescript": match session.typescript {
            Engine::Browser => "browser",
            Engine::Server => "server",
        },
        "runtime": Path::new(WASM_PKG).join("quilt_wasm.js").exists(),
    })
}

fn cells_json<LS: Languages, MS: MetaLanguages>(session: &Session<'_, LS, MS>) -> Value {
    let cells: Vec<Value> = session
        .notebook
        .cell_defs()
        .iter()
        .map(|def| cell_json(def, session.notebook.cell(def.id)))
        .collect();
    json!({ "cells": cells })
}

fn cell_json(def: &CellDef, cell: Option<&Cell>) -> Value {
    json!({
        "id": def.id,
        "lang": def.lang,
        "src": def.src,
        "generation": def.generation,
        "element": cell_element_id(def.id),
        "ran": cell.is_some_and(|c| c.ran),
        "failed": cell.is_some_and(Cell::failed),
        "value": cell.and_then(|c| c.value.as_deref()),
        "stdout": cell.map_or("", |c| &c.stdout),
        "stderr": cell.map_or("", |c| &c.stderr),
        "error": cell.and_then(|c| c.error.as_deref()),
        "edits": cell.map(|c| c.edits.clone()).unwrap_or_default(),
        "spawned": cell.map(|c| c.spawned.clone()).unwrap_or_default(),
    })
}

fn program_json(program: &crate::notebook::Program) -> Value {
    json!({
        "kind": format!("{:?}", program.kind),
        "resolved": program.resolved,
        "program": program.program,
    })
}

fn change_json(change: &Change) -> Value {
    match change {
        Change::Define { id, html } => json!({ "kind": "define", "id": id, "html": html }),
        Change::Insert { after, html } => {
            json!({ "kind": "insert", "after": after, "html": html })
        }
        Change::Append { html } => json!({ "kind": "append", "html": html }),
        Change::Remove { id } => json!({ "kind": "remove", "id": id }),
    }
}

/// An [`Answer`] a browser produced, as this server's JSON spells it. An
/// `error` is a cell that threw — the same outcome a rejected feed has.
fn answer_from(body: &Value) -> Result<Answer> {
    if let Some(error) = body.get("error").and_then(Value::as_str) {
        if !error.trim().is_empty() {
            return Err(miette!("{error}"));
        }
    }
    Ok(Answer {
        value: body
            .get("value")
            .and_then(Value::as_str)
            .map(|v| v.to_string().into_boxed_str()),
        name: None,
        stdout: body
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        stderr: body
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
    })
}

fn body(req: &Request) -> Result<Value> {
    if req.body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&req.body)
        .into_diagnostic()
        .map_err(|e| e.context("the request body is not JSON"))
}

fn string_field(body: &Value, name: &str) -> Result<String> {
    body.get(name)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| miette!("this request wants a string {name:?}"))
}

fn cell_number(segment: &str) -> Result<usize> {
    segment
        .parse()
        .map_err(|_| miette!("{segment:?} is not a cell number"))
}

/**************************************************************/

/// Dispatch an `/app/…` request to the Python machine as a feed: the
/// kernel's own `__quilt_dispatch` runs the registered handler and prints
/// one framed line, which is the response.
fn dispatch_app<LS: Languages, MS: MetaLanguages>(
    session: &mut Session<'_, LS, MS>,
    method: &str,
    path: &str,
    req: &Request,
) -> Result<(Response, String)> {
    session.ensure_session();
    // Fed as a statement, not a query: the dispatcher prints its response,
    // so the machine's answer is on stdout rather than in a value.
    let call = format!(
        "__quilt_dispatch({}, {}, {}, {})",
        py_str(method),
        py_str(path),
        py_str(&req.query),
        py_str(&req.text()),
    );
    let answer = match session.notebook.feed_lang("py", InnerKind::Stmt, &call) {
        Ok(answer) => answer,
        Err(e) => {
            return Ok((
                Response::status(
                    500,
                    &format!("the python machine refused the request: {e:?}"),
                ),
                String::new(),
            ))
        }
    };
    let Some(line) = answer
        .stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(RESPONSE_MARK))
    else {
        return Ok((
            Response::status(
                500,
                &format!(
                    "the python machine answered no response.\nstdout:\n{}\nstderr:\n{}",
                    answer.stdout, answer.stderr
                ),
            ),
            String::new(),
        ));
    };
    let value: Value = serde_json::from_str(line).into_diagnostic()?;
    let status =
        u16::try_from(value.get("status").and_then(Value::as_u64).unwrap_or(200)).unwrap_or(500);
    let content_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("text/plain; charset=utf-8");
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let stdout = value
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok((
        Response::new(status, content_type, body.to_string()),
        stdout,
    ))
}

/// A Python string literal denoting `s` — the same escaping the `LiftTo`
/// impls use, which is the only spelling a machine's wire ever needs.
fn py_str(s: &str) -> String {
    format!("\"{}\"", crate::lift::py_dquote_escape(s))
}

/**************************************************************/

/// Whether the request is for this server under a name that resolves to this
/// machine. A browser sends `Host`; a page on the internet that guesses the
/// port cannot make it say `localhost` unless it already resolves here.
fn local_host(req: &Request) -> bool {
    let Some(host) = req.header("host") else {
        return true; // HTTP/1.0, or a hand-written client: nothing to check.
    };
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    matches!(
        host.trim_matches(['[', ']']),
        "localhost" | "127.0.0.1" | "::1"
    )
}

/// Whether this request may drive the notebook. The site the notebook builds
/// (`/site`, `/app/…`) is open — it is a website, and asking a website for a
/// token is asking it not to be one.
fn authorized<LS: Languages, MS: MetaLanguages>(live: &Live<'_, LS, MS>, req: &Request) -> bool {
    if req.path == "/site" || req.path == "/app" || req.path.starts_with("/app/") {
        return true;
    }
    let matches = |given: Option<String>| given.is_some_and(|g| g == live.token);
    matches(req.param("token"))
        || matches(req.cookie(TOKEN_COOKIE))
        || matches(req.header("x-quilt-token").map(ToString::to_string))
}

/// A session token: 128 bits from the system, or — where that cannot be read
/// — from the clock and this process's own addresses, which is weaker and
/// still unguessable by a page that cannot see this machine.
fn session_token() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        use std::hash::{BuildHasher as _, Hasher as _};
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        hasher.write_usize(std::process::id() as usize);
        hasher.write_usize(std::ptr::addr_of!(bytes) as usize);
        let seed = hasher.finish().to_le_bytes();
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = seed[i % seed.len()] ^ u8::try_from(i).unwrap_or(0).wrapping_mul(31);
        }
    }
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn open_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

/// One file of the browser runtime, when it has been built.
fn wasm_asset(file: &str) -> Response {
    if file.contains("..") || file.contains('/') {
        return Response::status(403, "no");
    }
    let path = Path::new(WASM_PKG).join(file);
    let content_type = match path.extension().and_then(|e| e.to_str()) {
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json; charset=utf-8",
        Some("ts") => "text/plain; charset=utf-8",
        _ => return Response::status(404, "not a runtime file"),
    };
    match std::fs::read(&path) {
        Ok(bytes) => Response::new(200, content_type, bytes),
        Err(_) => Response::status(
            404,
            "the browser runtime is not built: `wasm-pack build quilt-wasm --target web \
             --out-dir pkg-web`",
        ),
    }
}

/// The module a viewer installs in the page's realm: the quilt runtime a
/// quoting TypeScript cell needs, and `↓`.
fn runtime_js() -> String {
    let built = Path::new(WASM_PKG).join("quilt_wasm.js").exists();
    format!(
        "export const RUNTIME_BUILT = {built};\n{}",
        assets::RUNTIME_JS
    )
}

/**************************************************************/

/// The session prelude the Python machine is given: what a *notebook*
/// Python kernel knows that a plain one does not — how to register a route,
/// how to reach the session database, and how to answer one request.
///
/// It is fed like any other fragment, so it is ordinary Python and the cells
/// can read it (`route`, `db`, `_quilt_routes` are just names in the
/// namespace). The framing is one printed line, which is the same trick the
/// kernel's own sentinel uses: a machine's wire is its stdout.
const PYTHON_PRELUDE: &str = r#"
import contextlib as _q_ctx, inspect as _q_inspect, io as _q_io, json as _q_json, os as _q_os, traceback as _q_tb
_quilt_routes = {}
def route(path, methods=("GET",)):
    """Serve `path` from this notebook: the server proxies /app<path> here."""
    if isinstance(methods, str):
        methods = (methods,)
    def register(fn):
        for method in methods:
            _quilt_routes[(method.upper(), path)] = fn
        return fn
    return register
def db():
    """A connection to the session database — the one the SQL cells write."""
    import sqlite3
    path = _q_os.environ.get("QUILT_DB")
    if not path:
        raise RuntimeError("no session database: QUILT_DB is unset")
    con = sqlite3.connect(path)
    con.row_factory = sqlite3.Row
    return con
def __quilt_dispatch(method, path, query, body):
    handler = _quilt_routes.get((method, path))
    out = _q_io.StringIO()
    status, kind, text = 200, "text/plain; charset=utf-8", ""
    if handler is None:
        known = sorted("%s %s" % (m, p) for m, p in _quilt_routes)
        status, text = 404, "no route for %s %s\nthis notebook serves: %s" % (
            method, path, ", ".join(known) or "nothing yet")
    else:
        try:
            with _q_ctx.redirect_stdout(out):
                if len(_q_inspect.signature(handler).parameters):
                    value = handler({"method": method, "path": path, "query": query, "body": body})
                else:
                    value = handler()
            if isinstance(value, tuple) and len(value) == 2 and isinstance(value[0], int):
                status, value = value
            if value is None:
                status, text = (204 if status == 200 else status), ""
            elif isinstance(value, (dict, list)):
                kind, text = "application/json; charset=utf-8", _q_json.dumps(value)
            else:
                text = value if isinstance(value, str) else str(value)
                if text.lstrip().startswith("<"):
                    kind = "text/html; charset=utf-8"
        except Exception:
            status, kind, text = 500, "text/plain; charset=utf-8", _q_tb.format_exc()
    print("__QUILT_RESP__" + _q_json.dumps(
        {"status": status, "type": kind, "body": text, "stdout": out.getvalue()}))
"#;

/**************************************************************/

/// The editor, as files. Held here rather than written to disk at startup:
/// a session server that unpacked assets would be a session server with a
/// cache to invalidate.
mod assets {
    pub const UI_HTML: &str = include_str!("serve/ui.html");
    pub const UI_CSS: &str = include_str!("serve/ui.css");
    pub const UI_JS: &str = include_str!("serve/ui.js");
    pub const RUNTIME_JS: &str = include_str!("serve/runtime.js");
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_unguessable_and_stable_for_the_session() {
        let a = session_token();
        assert_eq!(a.len(), 32, "128 bits, hex");
        assert_ne!(a, session_token());
    }

    #[test]
    fn only_this_machines_names_are_served() {
        let host = |value: &str| {
            let mut req = Request::default();
            req.headers.insert("host".into(), value.into());
            local_host(&req)
        };
        assert!(host("localhost:8788"));
        assert!(host("127.0.0.1:8788"));
        assert!(host("[::1]:8788"));
        assert!(!host("notebook.example.com"));
    }

    #[test]
    fn python_literals_survive_the_wire() {
        assert_eq!(py_str("a\"b\nc"), "\"a\\\"b\\nc\"");
    }
}
