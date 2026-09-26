//! The least HTTP a local session needs: request in, response out, and one
//! long-lived stream for events.
//!
//! Hand-written, on `std::net`, because of what this server *is*. It binds
//! `127.0.0.1`, it serves one notebook to one person, its handlers hold a
//! mutex over a `Notebook` and run subprocesses — nothing about it is
//! throughput, TLS, HTTP/2, or a thousand concurrent sockets. Against that,
//! an async runtime and a framework are a dependency tree the published
//! `quiltlang` would carry for one subcommand; the parts actually used here
//! are a request line, a `Content-Length`, and `text/event-stream`, which is
//! this file.
//!
//! What it deliberately does *not* do: chunked request bodies (browsers do
//! not send them for `fetch` with a string body), `100-continue`, ranges,
//! compression, or trailers. A request it cannot read is answered `400` and
//! the connection is closed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::Receiver;

/// The most a request body may carry: a cell's source, a page fragment, a
/// posted result. Anything larger is a mistake or an attack, and is refused
/// before it is read.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// The most a request line or header may carry.
const MAX_LINE: usize = 64 * 1024;

/**************************************************************/

/// One request, as the handlers want it.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    /// The path, percent-decoded, without the query.
    pub path: String,
    /// The query string, as written, without the `?`.
    pub query: String,
    /// Header names lowercased — the wire says they are case-insensitive.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// The body as text; invalid UTF-8 is replaced rather than refused.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The path split on `/`, empty segments dropped: `/cells/3/run` is
    /// `["cells", "3", "run"]`.
    #[must_use]
    pub fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }

    /// One query parameter, percent-decoded.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<String> {
        self.query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (percent_decode(k) == name).then(|| percent_decode(v))
        })
    }

    /// One cookie value.
    #[must_use]
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.header("cookie")?.split(';').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k.trim() == name).then(|| v.trim().to_string())
        })
    }
}

/// One response.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    /// Extra headers, in the order they are written.
    pub headers: Vec<(String, String)>,
}

impl Response {
    #[must_use]
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Response {
        Response {
            status,
            content_type: content_type.to_string(),
            body: body.into(),
            headers: Vec::new(),
        }
    }

    #[must_use]
    pub fn html(body: impl Into<Vec<u8>>) -> Response {
        Response::new(200, "text/html; charset=utf-8", body)
    }

    #[must_use]
    pub fn text(body: impl Into<Vec<u8>>) -> Response {
        Response::new(200, "text/plain; charset=utf-8", body)
    }

    #[must_use]
    pub fn json(body: impl Into<Vec<u8>>) -> Response {
        Response::new(200, "application/json; charset=utf-8", body)
    }

    /// A status with a plain-text reason — every error this server answers.
    #[must_use]
    pub fn status(status: u16, message: &str) -> Response {
        Response::new(status, "text/plain; charset=utf-8", message.to_string())
    }

    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// One server-sent event.
#[derive(Debug, Clone)]
pub struct Event {
    pub name: String,
    pub data: String,
}

/// What a handler answers: a response, or the event stream.
pub enum Reply {
    Done(Response),
    /// Hold the connection open and write every event that arrives, until
    /// the viewer goes away.
    Events(Receiver<Event>),
}

/**************************************************************/

/// Read one request. `Ok(None)` is a connection the peer closed between
/// requests — the ordinary end of a keep-alive connection, not an error.
pub fn read_request<R: BufRead>(r: &mut R) -> std::io::Result<Option<Request>> {
    let Some(line) = read_line(r)? else {
        return Ok(None);
    };
    if line.trim().is_empty() {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(bad("a request line is `METHOD TARGET VERSION`"));
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut req = Request {
        method: method.to_ascii_uppercase(),
        path: percent_decode(path),
        query: query.to_string(),
        ..Request::default()
    };
    while let Some(line) = read_line(r)? {
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            req.headers
                .insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length: usize = req
        .header("content-length")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(bad("request body too large"));
    }
    req.body = vec![0; length];
    r.read_exact(&mut req.body)?;
    Ok(Some(req))
}

/// Read one CRLF-terminated line, without its terminator.
fn read_line<R: BufRead>(r: &mut R) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let read = r.take(MAX_LINE as u64).read_until(b'\n', &mut buf)?;
    if read == 0 {
        return Ok(None);
    }
    while buf.last().is_some_and(|c| *c == b'\n' || *c == b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn bad(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

/// Percent-decoding, plus `+` for a space — enough for the paths and query
/// values a browser sends here.
#[must_use]
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/**************************************************************/

/// Serve one connection until the peer closes it or a handler takes it over
/// as an event stream.
pub fn serve_connection<H>(stream: TcpStream, handle: &H)
where
    H: Fn(&Request) -> Reply,
{
    let _ = stream.set_nodelay(true);
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut writer = write_half;
    loop {
        let request = match read_request(&mut reader) {
            Ok(Some(request)) => request,
            // A request that cannot be read is answered and the connection
            // ends; a closed connection just ends.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                let _ = write_response(&mut writer, &Response::status(400, &e.to_string()), false);
                return;
            }
            Ok(None) | Err(_) => return,
        };
        // A client that said `Connection: close` is waiting for the end of
        // the stream, not for another response.
        let keep_alive = !request
            .header("connection")
            .is_some_and(|v| v.eq_ignore_ascii_case("close"));
        match handle(&request) {
            Reply::Done(response) => {
                if write_response(&mut writer, &response, keep_alive).is_err() || !keep_alive {
                    return;
                }
            }
            Reply::Events(events) => {
                stream_events(&mut writer, &events);
                return;
            }
        }
    }
}

/// Write one response. `keep_alive` is a promise about this connection, not
/// about the request: a body is always length-delimited here, so the peer
/// can always find the end.
pub fn write_response<W: Write>(
    w: &mut W,
    response: &Response,
    keep_alive: bool,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len(),
        if keep_alive { "keep-alive" } else { "close" },
    );
    for (name, value) in &response.headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    w.write_all(&response.body)?;
    w.flush()
}

/// Hold the connection open, writing events as they arrive. Returns when the
/// viewer goes away (the write fails) or the sender is dropped.
fn stream_events<W: Write>(w: &mut W, events: &Receiver<Event>) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
                Connection: keep-alive\r\n\r\n";
    if w.write_all(head.as_bytes()).is_err() || w.flush().is_err() {
        return;
    }
    while let Ok(event) = events.recv() {
        if write_event(w, &event).is_err() {
            return;
        }
    }
}

/// One event, in the `text/event-stream` framing: a `data:` line per line of
/// the payload, then a blank line.
pub fn write_event<W: Write>(w: &mut W, event: &Event) -> std::io::Result<()> {
    let mut out = String::new();
    if !event.name.is_empty() {
        let _ = writeln!(out, "event: {}", event.name);
    }
    for line in event.data.split('\n') {
        let _ = writeln!(out, "data: {line}");
    }
    out.push('\n');
    w.write_all(out.as_bytes())?;
    w.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Request {
        let mut reader = BufReader::new(raw.as_bytes());
        read_request(&mut reader)
            .expect("the request reads")
            .expect("there is a request")
    }

    #[test]
    fn a_request_is_its_line_headers_and_body() {
        let req = parse(
            "POST /cells/3/run?engine=browser HTTP/1.1\r\nHost: localhost\r\n\
             Content-Length: 7\r\n\r\n{\"a\":1}",
        );
        assert_eq!(req.method, "POST");
        assert_eq!(req.segments(), ["cells", "3", "run"]);
        assert_eq!(req.param("engine").as_deref(), Some("browser"));
        assert_eq!(req.header("host"), Some("localhost"));
        assert_eq!(req.text(), "{\"a\":1}");
    }

    #[test]
    fn paths_and_params_are_decoded() {
        let req = parse("GET /page/a%20b?q=x%2By&t=%E2%86%96 HTTP/1.1\r\n\r\n");
        assert_eq!(req.path, "/page/a b");
        assert_eq!(req.param("q").as_deref(), Some("x+y"));
        assert_eq!(req.param("t").as_deref(), Some("↖"));
    }

    #[test]
    fn cookies_are_read_by_name() {
        let req = parse("GET / HTTP/1.1\r\nCookie: other=1; quilt_token=abc\r\n\r\n");
        assert_eq!(req.cookie("quilt_token").as_deref(), Some("abc"));
    }

    #[test]
    fn an_event_is_framed_line_by_line() {
        let mut out = Vec::new();
        write_event(
            &mut out,
            &Event {
                name: "page".into(),
                data: "one\ntwo".into(),
            },
        )
        .expect("write");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "event: page\ndata: one\ndata: two\n\n"
        );
    }
}
