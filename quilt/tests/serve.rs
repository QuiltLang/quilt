//! Live notebooks end to end (`quilt::serve`): a session behind a socket,
//! driven the way the editor drives it.
//!
//! The tests are the client — including the *browser*: a TypeScript cell's
//! program comes back over the wire and its answer is posted back, which is
//! exactly the protocol the page's realm speaks. That is the point of the
//! split: where a cell ran is not something the notebook can tell.
//!
//! Needs `python3`, `bash` and `sqlite3` on `PATH`, like the notebook tests.
#![cfg(all(feature = "serve", feature = "python"))]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use quilt::langs::omni::Omni;
use quilt::serve::{serve_with, Config, Engine};
use serde_json::{json, Value};

/* ── a session, and a client for it ────────────────────────────────────── */

struct Session {
    addr: SocketAddr,
    token: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    /// Start a session on a free port, in this process, and wait for it to
    /// be listening.
    fn start(src: Option<&str>, config: &Config) -> Session {
        let (tx, rx) = std::sync::mpsc::channel();
        let src = src.map(str::to_string);
        let config = config.clone();
        let thread = std::thread::spawn(move || {
            let mut multi = Omni::default();
            let config = Config { port: 0, ..config };
            serve_with(&mut multi, src.as_deref(), &config, |ready| {
                tx.send((ready.addr, ready.token.clone())).expect("ready");
            })
            .expect("the session serves");
        });
        let (addr, token) = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the session binds");
        Session {
            addr,
            token,
            thread: Some(thread),
        }
    }

    fn empty() -> Session {
        Session::start(None, &Config::default())
    }

    /// One request, one response — `(status, body)`.
    fn send(&self, method: &str, path: &str, body: Option<Value>) -> (u16, String) {
        raw(self.addr, method, &self.with_token(path), body, None)
    }

    /// The JSON a request answers.
    fn json(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, text) = self.send(method, path, body);
        assert!(
            (200..300).contains(&status),
            "{method} {path} answered {status}: {text}"
        );
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{method} {path}: {e}\n{text}"))
    }

    fn with_token(&self, path: &str) -> String {
        let join = if path.contains('?') { '&' } else { '?' };
        format!("{path}{join}token={}", self.token)
    }

    /// Add a cell and run it, as the editor does.
    fn cell(&self, lang: &str, src: &str) -> Value {
        let added = self.json("POST", "/cells", Some(json!({ "lang": lang, "src": src })));
        let id = added["id"].as_u64().expect("a new cell has an id");
        self.json("POST", &format!("/cells/{id}/run"), Some(json!({})))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.send("POST", "/shutdown", Some(json!({})));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Write one request, read the whole response. `timeout` leaves the socket
/// open for a stream instead.
fn raw(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<Value>,
    timeout: Option<Duration>,
) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("the session is listening");
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    if let Some(timeout) = timeout {
        stream.set_read_timeout(Some(timeout)).expect("timeout");
    }
    let mut text = String::new();
    // A timed-out read on a stream that is still open is the point, there.
    let _ = stream.read_to_string(&mut text);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    (status, body.to_string())
}

fn cell_of(answer: &Value) -> &Value {
    &answer["cell"]
}

/* ── cells ─────────────────────────────────────────────────────────────── */

/// The loop the editor is: add a cell, run it, read what it said. The cell's
/// figure lands in the page, and the answer carries the page patch that puts
/// it there.
#[test]
fn a_cell_is_added_run_and_read_back() {
    let session = Session::empty();
    let answer = session.cell("py", "x = 6 * 7\nprint(x)");
    assert_eq!(cell_of(&answer)["stdout"], "42");
    assert_eq!(cell_of(&answer)["ran"], true);
    assert_eq!(answer["ran"], "server");
    let define = answer["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .find(|c| c["id"] == "quilt-cell-1")
        .expect("the cell's figure was defined");
    assert_eq!(define["kind"], "define");
    assert!(
        define["html"].as_str().unwrap().contains("42"),
        "the patch carries the rendered figure: {define}"
    );
}

/// Cells of one language share that language's machine — the sequencing law
/// again, now across requests rather than across a file.
#[test]
fn cells_share_a_machine_across_requests() {
    let session = Session::empty();
    session.cell("py", "greeting = 'hello'");
    let answer = session.cell("py", "greeting * 2");
    assert_eq!(cell_of(&answer)["value"], "'hellohello'");
}

/// Editing a cell puts it back to pending — its old output belonged to its
/// old source — and re-running it replaces its figure rather than adding one.
#[test]
fn a_cell_can_be_edited_and_re_run() {
    let session = Session::empty();
    session.cell("py", "print('first')");
    let edited = session.json("PUT", "/cells/1", Some(json!({ "src": "print('second')" })));
    assert_eq!(cell_of(&edited)["ran"], false);
    assert_eq!(cell_of(&edited)["stdout"], "");
    let answer = session.json("POST", "/cells/1/run", Some(json!({})));
    assert_eq!(cell_of(&answer)["stdout"], "second");
    let page = session.json("GET", "/page", None);
    let html = page["html"].as_str().unwrap();
    assert_eq!(html.matches("id=\"quilt-cell-1\"").count(), 1);
    assert!(!html.contains("first"), "the old run is gone: {html}");
}

/// A cell that cannot be read fails where it stands, and the session goes on
/// — the REPL's contract, which an editor needs more than a file does.
#[test]
fn a_broken_cell_does_not_stop_the_session() {
    let session = Session::empty();
    let answer = session.cell("py", "1 +");
    assert!(cell_of(&answer)["failed"].as_bool().unwrap());
    let next = session.cell("py", "print('still here')");
    assert_eq!(cell_of(&next)["stdout"], "still here");
}

/// A cell is added where the editor is looking: after the element it names.
#[test]
fn a_cell_is_added_after_the_element_it_names() {
    let session = Session::start(
        Some("<p id=\"top\">first</p>\n<p id=\"tail\">last</p>\n"),
        &Config::default(),
    );
    let added = session.json(
        "POST",
        "/cells",
        Some(json!({ "lang": "py", "src": "1", "after": "top" })),
    );
    let insert = added["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "insert")
        .expect("an anchored cell is an insert, not an append");
    assert_eq!(insert["after"], "top");
    let html = session.json("GET", "/page", None)["html"]
        .as_str()
        .unwrap()
        .to_string();
    let (cell, tail) = (
        html.find("quilt-cell-1").expect("the cell"),
        html.find("tail").expect("the tail"),
    );
    assert!(cell < tail, "the cell went between the two: {html}");
}

/// Removing a cell takes its figure out of the page.
#[test]
fn a_removed_cell_leaves_the_page() {
    let session = Session::empty();
    session.cell("py", "print('here')");
    let removed = session.json("DELETE", "/cells/1", None);
    assert_eq!(removed["removed"], true);
    assert!(!session.json("GET", "/page", None)["html"]
        .as_str()
        .unwrap()
        .contains("quilt-cell-1"));
    assert!(session.json("GET", "/cells", None)["cells"]
        .as_array()
        .unwrap()
        .is_empty());
}

/* ── the expander ──────────────────────────────────────────────────────── */

/// "Expand" answers both halves: the cell with its page references resolved,
/// and the program that resolution expands to. For a cell that quotes, the
/// two are worth seeing side by side — one is what the author wrote, the
/// other is what the machine is fed.
#[test]
fn expand_answers_the_resolved_source_and_the_program() {
    let session = Session::start(Some("<b id=\"n\">7</b>"), &Config::default());
    session.json(
        "POST",
        "/cells",
        Some(json!({ "lang": "py", "src": "int(↙#n↘) * 6" })),
    );
    let program = session.json("GET", "/cells/1/expand", None);
    assert_eq!(program["kind"], "Expr");
    assert_eq!(program["resolved"], "int(\"7\") * 6");
    assert_eq!(program["program"], "int(\"7\") * 6");

    // The same door, for source that is not a cell yet.
    let quoted = session.json(
        "POST",
        "/expand",
        Some(json!({ "lang": "py", "src": "html↖<b>hi</b>↗" })),
    );
    assert!(
        quoted["program"]
            .as_str()
            .unwrap()
            .contains("tb(\"element\")"),
        "a quote expands to builder calls: {quoted}"
    );
}

/* ── the browser as a machine ──────────────────────────────────────────── */

/// A TypeScript cell's machine is the viewer's browser: the run answers with
/// the *program* and waits. Posting the answer back lands it — the cell
/// cannot tell that its machine was a page.
#[test]
fn a_typescript_cell_runs_where_the_page_is() {
    let session = Session::empty();
    let added = session.json(
        "POST",
        "/cells",
        Some(json!({ "lang": "ts", "src": "const total = 6 * 7; total" })),
    );
    let id = added["id"].as_u64().unwrap();
    let handed = session.json("POST", &format!("/cells/{id}/run"), Some(json!({})));
    assert_eq!(handed["ran"], "browser");
    assert_eq!(handed["program"], "const total = 6 * 7; total");

    let landed = session.json(
        "POST",
        &format!("/cells/{id}/result"),
        Some(json!({ "value": "42", "stdout": "" })),
    );
    assert_eq!(landed["ran"], "browser");
    assert_eq!(cell_of(&landed)["value"], "42");
    assert!(session.json("GET", "/page", None)["html"]
        .as_str()
        .unwrap()
        .contains("<pre class=\"quilt-value\">42</pre>"));
}

/// What the browser answers is read back into the page like any other
/// machine's output: markup with an id the page has redefines that element.
#[test]
fn what_the_browser_answers_edits_the_page() {
    let session = Session::start(Some("<p id=\"total\">?</p>"), &Config::default());
    let added = session.json("POST", "/cells", Some(json!({ "lang": "ts", "src": "0" })));
    let id = added["id"].as_u64().unwrap();
    session.json("POST", &format!("/cells/{id}/run"), Some(json!({})));
    let landed = session.json(
        "POST",
        &format!("/cells/{id}/result"),
        Some(json!({ "stdout": "<p id=\"total\">42</p>" })),
    );
    assert_eq!(cell_of(&landed)["edits"], json!(["total"]));
    assert!(session.json("GET", "/page", None)["html"]
        .as_str()
        .unwrap()
        .contains("<p id=\"total\">42</p>"));
}

/// A browser that reports an exception fails the cell, with the page's own
/// error chrome — not the session.
#[test]
fn a_browser_error_fails_only_its_cell() {
    let session = Session::empty();
    let added = session.json(
        "POST",
        "/cells",
        Some(json!({ "lang": "ts", "src": "boom()" })),
    );
    let id = added["id"].as_u64().unwrap();
    session.json("POST", &format!("/cells/{id}/run"), Some(json!({})));
    let landed = session.json(
        "POST",
        &format!("/cells/{id}/result"),
        Some(json!({ "error": "ReferenceError: boom is not defined" })),
    );
    assert!(cell_of(&landed)["failed"].as_bool().unwrap());
    assert!(cell_of(&landed)["error"]
        .as_str()
        .unwrap()
        .contains("ReferenceError"));
}

/// `--ts server` puts TypeScript back in the park with everything else.
#[test]
fn typescript_can_run_on_the_server_instead() {
    let session = Session::start(
        None,
        &Config {
            typescript: Engine::Server,
            ..Config::default()
        },
    );
    let added = session.json(
        "POST",
        "/cells",
        Some(json!({ "lang": "ts", "src": "1 + 1" })),
    );
    let id = added["id"].as_u64().unwrap();
    let answer = session.json("POST", &format!("/cells/{id}/run"), Some(json!({})));
    assert_eq!(answer["ran"], "server", "{answer}");
}

/* ── the site the cells build ──────────────────────────────────────────── */

/// The SQL machine writes a file, and the Python kernel is told where it is:
/// one database, so a route can serve what the SQL cells built.
#[test]
fn sql_and_python_share_one_database() {
    let session = Session::empty();
    let made = session.cell(
        "sql",
        "CREATE TABLE menu(item TEXT, price INTEGER);\nINSERT INTO menu VALUES ('tea', 3);",
    );
    assert!(!made["cell"]["failed"].as_bool().unwrap(), "{made}");
    let read = session.cell("py", "db().execute('SELECT item FROM menu').fetchone()[0]");
    assert_eq!(cell_of(&read)["value"], "'tea'");
}

/// A Python cell that registers a route makes the notebook a server: the
/// request becomes a feed, and what the handler returns is the response.
#[test]
fn a_python_cell_serves_a_route() {
    let session = Session::empty();
    session.cell(
        "sql",
        "CREATE TABLE t(v INTEGER);\nINSERT INTO t VALUES (42);",
    );
    let registered = session.cell(
        "py",
        "@route(\"/answer\")\ndef answer():\n    return \"<b>%d</b>\" % db().execute(\"SELECT v FROM t\").fetchone()[0]",
    );
    assert!(
        !registered["cell"]["failed"].as_bool().unwrap(),
        "{registered}"
    );

    let (status, body) = raw(session.addr, "GET", "/app/answer", None, None);
    assert_eq!(status, 200);
    assert_eq!(body, "<b>42</b>");

    // A path no cell registered says so, and says what the notebook serves.
    let (status, body) = raw(session.addr, "GET", "/app/nothing", None, None);
    assert_eq!(status, 404);
    assert!(body.contains("GET /answer"), "{body}");
}

/// A route can take the request, and answer JSON by answering a dict.
#[test]
fn a_route_reads_its_request_and_answers_json() {
    let session = Session::empty();
    session.cell(
        "py",
        "@route(\"/echo\", methods=(\"GET\", \"POST\"))\ndef echo(req):\n    return {\"method\": req[\"method\"], \"query\": req[\"query\"], \"body\": req[\"body\"]}",
    );
    let (status, body) = raw(session.addr, "GET", "/app/echo?who=world", None, None);
    assert_eq!(status, 200);
    let answered: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(answered["method"], "GET");
    assert_eq!(answered["query"], "who=world");
}

/// A handler that throws answers 500 with its traceback, and the session
/// keeps serving.
#[test]
fn a_failing_route_is_a_500_not_a_dead_session() {
    let session = Session::empty();
    session.cell(
        "py",
        "@route(\"/bad\")\ndef bad():\n    raise ValueError(\"no\")",
    );
    let (status, body) = raw(session.addr, "GET", "/app/bad", None, None);
    assert_eq!(status, 500);
    assert!(body.contains("ValueError"), "{body}");
    let after = session.cell("py", "'alive'");
    assert_eq!(cell_of(&after)["value"], "'alive'");
}

/// `/site` is the page with the notebook's own scaffolding gone: what the
/// cells built, without the record of building it.
#[test]
fn the_site_is_the_page_without_the_cells() {
    let session = Session::start(
        Some("<div id=\"app\">nothing yet</div>"),
        &Config::default(),
    );
    session.cell("py", "print('<div id=\"app\">a tiny site</div>')");
    let (status, site) = raw(session.addr, "GET", "/site", None, None);
    assert_eq!(status, 200);
    assert!(site.contains("a tiny site"), "{site}");
    assert!(!site.contains("quilt-cell-"), "no cell chrome: {site}");
    assert!(!site.contains("quilt-cells"), "no cell stylesheet: {site}");
    // The notebook view still has both.
    assert!(session.json("GET", "/page", None)["html"]
        .as_str()
        .unwrap()
        .contains("quilt-cell-1"));
}

/// The way out of a session that keeps nothing: the page `quilt notebook`
/// would have written.
#[test]
fn export_is_the_page_a_file_run_would_have_written() {
    let session = Session::empty();
    session.cell("py", "print('<p id=\"done\">yes</p>')");
    let (status, page) = session.send("GET", "/export", None);
    assert_eq!(status, 200);
    assert!(page.starts_with("<!-- DO NOT EDIT"), "{page}");
    assert!(page.contains("<p id=\"done\">yes</p>"));
}

/* ── the wire ──────────────────────────────────────────────────────────── */

/// A viewer's first event is the whole state, so a page opened late is not
/// looking at changes it missed; a cell run after that arrives as a patch.
#[test]
fn the_event_stream_opens_with_the_state_and_then_patches() {
    let session = Session::empty();
    session.cell("py", "print('<p id=\"note\">watch</p>')");

    let mut stream = TcpStream::connect(session.addr).expect("connect");
    stream
        .write_all(
            format!(
                "GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n",
                session.with_token("/events")
            )
            .as_bytes(),
        )
        .expect("write");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let mut reader = BufReader::new(stream);
    let hello = read_event(&mut reader).expect("the hello event");
    assert_eq!(hello.0, "hello");
    let state: Value = serde_json::from_str(&hello.1).expect("json");
    assert!(state["html"].as_str().unwrap().contains("watch"));
    assert_eq!(state["cells"].as_array().unwrap().len(), 1);
    assert_eq!(state["typescript"], "browser");

    // A change made by someone else reaches this viewer.
    session.cell("py", "print('<p id=\"note\">changed</p>')");
    let mut sawpatch = false;
    for _ in 0..6 {
        let Some((name, data)) = read_event(&mut reader) else {
            break;
        };
        if name == "page" && data.contains("changed") {
            sawpatch = true;
            break;
        }
    }
    assert!(sawpatch, "the page change was pushed to the viewer");
}

/// Read one `text/event-stream` event: `event:` then its `data:` lines.
fn read_event(reader: &mut BufReader<TcpStream>) -> Option<(String, String)> {
    let (mut name, mut data) = (String::new(), String::new());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !name.is_empty() || !data.is_empty() {
                return Some((name, data));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("event: ") {
            name = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("data: ") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest);
        }
    }
}

/// The notebook wants its session token; the site it builds does not — a
/// website that asked for one would not be a website.
#[test]
fn the_token_guards_the_notebook_and_not_the_site() {
    let session = Session::empty();
    let (status, _) = raw(session.addr, "GET", "/cells", None, None);
    assert_eq!(status, 403, "no token, no notebook");
    let (status, _) = raw(session.addr, "GET", "/site", None, None);
    assert_eq!(status, 200, "the site is a site");
    let (status, _) = raw(session.addr, "GET", "/cells?token=nonsense", None, None);
    assert_eq!(status, 403);
    let (status, _) = session.send("GET", "/cells", None);
    assert_eq!(status, 200);
}

/// The editor is served from the session itself, so there is nothing to
/// install and nothing to keep in step with it.
#[test]
fn the_editor_is_served_with_its_own_assets() {
    let session = Session::empty();
    let (status, page) = session.send("GET", "/", None);
    assert_eq!(status, 200);
    assert!(page.contains("<script type=\"module\" src=\"/ui.js\">"));
    for asset in ["/ui.css", "/ui.js", "/runtime.js"] {
        let (status, body) = session.send("GET", asset, None);
        assert_eq!(status, 200, "{asset}");
        assert!(!body.is_empty(), "{asset}");
    }
    // The page the frame shows is the document itself, with nothing added.
    let (status, view) = session.send("GET", "/view", None);
    assert_eq!(status, 200);
    assert_eq!(view, session.json("GET", "/page", None)["html"]);
}

/// A viewer's own edit to the page comes home, so the next cell's `↙#id↘`
/// reads what the viewer is looking at.
#[test]
fn an_edit_from_the_page_reaches_the_cells() {
    let session = Session::start(Some("<b id=\"who\">nobody</b>"), &Config::default());
    session.json(
        "PUT",
        "/page/who",
        Some(json!({ "html": "<b id=\"who\">the browser</b>" })),
    );
    let answer = session.cell("py", "↙#who↘.upper()");
    assert_eq!(cell_of(&answer)["value"], "'THE BROWSER'");
}
