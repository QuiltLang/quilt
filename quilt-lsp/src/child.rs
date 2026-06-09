//! A client connection to a downstream language server (e.g. rust-analyzer).
//!
//! The quilt server is itself a *client* of one process per language. This
//! module owns the transport: spawning the process, framing JSON-RPC over its
//! stdio, matching responses to requests, auto-answering the handful of
//! server→client requests rust-analyzer makes during startup, and forwarding
//! its notifications (chiefly `textDocument/publishDiagnostics`) to a channel
//! the router drains.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

/// A notification pushed by the downstream server (no reply expected).
#[derive(Debug)]
pub struct ChildNotification {
    pub method: String,
    pub params: Value,
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<std::result::Result<Value, Value>>>>>;

/// Upper bound on how long we wait for any downstream response. Generous enough
/// for slow first responses during indexing, but bounded so a wedged or dead
/// child can never hang a request indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ChildServer {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: AtomicI64,
    // Keep the process handle alive for the lifetime of the connection.
    _child: Child,
}

impl ChildServer {
    /// Spawn `program args…` and start its read loop. Returns the connection
    /// and a receiver of the server's notifications.
    pub fn spawn(
        program: &str,
        args: &[String],
        cwd: Option<&std::path::Path>,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<ChildNotification>)> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Run in the project root so toolchain wrappers (e.g. the rustup
        // `rust-analyzer` shim) resolve the project's toolchain from its
        // `rust-toolchain.toml` rather than the inherited cwd's default.
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("spawning downstream server `{program}`"))?;

        let stdin = child.stdin.take().context("child stdin")?;
        let stdout = child.stdout.take().context("child stdout")?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let stdin = Arc::new(Mutex::new(stdin));
        let (notif_tx, notif_rx) = mpsc::unbounded_channel();

        let server = Arc::new(Self {
            stdin: stdin.clone(),
            pending: pending.clone(),
            next_id: AtomicI64::new(1),
            _child: child,
        });

        tokio::spawn(read_loop(BufReader::new(stdout), stdin, pending, notif_tx));

        Ok((server, notif_rx))
    }

    /// Send a request and await its result.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(e) = write_message(&mut *self.stdin.lock().await, &msg).await {
            self.pending.lock().await.remove(&id);
            return Err(e).context("writing request");
        }

        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => bail!("downstream error: {e}"),
            // Sender dropped: the read loop ended (child exited / EOF).
            Ok(Err(_)) => bail!("downstream connection closed"),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                bail!("downstream request `{method}` timed out");
            }
        }
    }

    /// Send a notification (no reply).
    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        write_message(&mut *self.stdin.lock().await, &msg).await
    }

    /// Run the LSP startup handshake: `initialize` then `initialized`.
    pub async fn initialize(
        &self,
        root_uri: Option<&str>,
        capabilities: Value,
        initialization_options: Value,
    ) -> Result<Value> {
        let params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": capabilities,
            "initializationOptions": initialization_options,
            "clientInfo": {"name": "quilt-lsp"},
        });
        let result = self.request("initialize", params).await?;
        self.notify("initialized", json!({})).await?;
        Ok(result)
    }
}

async fn read_loop(
    mut reader: BufReader<tokio::process::ChildStdout>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    notif_tx: mpsc::UnboundedSender<ChildNotification>,
) {
    loop {
        let msg = match read_message(&mut reader).await {
            Ok(Some(m)) => m,
            Ok(None) => break, // EOF: downstream exited
            Err(e) => {
                tracing::warn!("downstream read error: {e}");
                break;
            }
        };

        let id = msg.get("id").cloned();
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);

        match (id, method) {
            // Server→client request: auto-answer so startup doesn't stall, then
            // surface it to the router too (e.g. to forward refresh requests).
            (Some(id), Some(method)) => {
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": canned_result(&method, msg.get("params")),
                });
                if let Err(e) = write_message(&mut *stdin.lock().await, &reply).await {
                    tracing::warn!("failed to answer downstream request {method}: {e}");
                }
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                let _ = notif_tx.send(ChildNotification { method, params });
            }
            // Response to one of our requests.
            (Some(id), None) => {
                if let Some(id) = id.as_i64() {
                    let result = msg.get("error").map_or_else(
                        || Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                        |e| Err(e.clone()),
                    );
                    if let Some(tx) = pending.lock().await.remove(&id) {
                        let _ = tx.send(result);
                    }
                }
            }
            // Notification.
            (None, Some(method)) => {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                let _ = notif_tx.send(ChildNotification { method, params });
            }
            (None, None) => {}
        }
    }

    // The connection ended: fail every in-flight request rather than letting it
    // wait for the timeout.
    for (_, tx) in pending.lock().await.drain() {
        let _ = tx.send(Err(json!("downstream connection closed")));
    }
    tracing::info!("downstream read loop ended");
}

/// Minimal, safe replies to the server→client requests rust-analyzer issues
/// during startup. Defaults everywhere; we never apply edits.
fn canned_result(method: &str, params: Option<&Value>) -> Value {
    match method {
        "workspace/configuration" => {
            let n = params
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; n])
        }
        "workspace/applyEdit" => json!({"applied": false}),
        // registerCapability, unregisterCapability, workDoneProgress/create, …
        _ => Value::Null,
    }
}

/// Read one LSP message (headers + JSON body) from `reader`.
async fn read_message<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break; // end of headers
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_length = Some(v.trim().parse().context("parsing Content-Length")?);
        }
    }
    let len = content_length.ok_or_else(|| anyhow!("message without Content-Length"))?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(Some(
        serde_json::from_slice(&buf).context("parsing message body")?,
    ))
}

/// Write one LSP message (Content-Length header + JSON body).
async fn write_message(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    stdin
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    stdin.write_all(&body).await?;
    stdin.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A downstream process that exits immediately (as the broken rust-analyzer
    /// shim effectively did) must make in-flight requests fail fast, never hang.
    #[tokio::test]
    async fn request_to_dead_child_fails_fast() {
        let Ok((server, _rx)) = ChildServer::spawn("true", &[], None) else {
            return; // `true` not available; skip
        };
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            server.request("initialize", json!({})),
        )
        .await;
        assert!(res.is_ok(), "request hung instead of failing fast");
        assert!(res.unwrap().is_err(), "expected an error from a dead child");
    }
}
