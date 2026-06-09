#!/usr/bin/env python3
"""Minimal LSP stdio driver: initialize -> didOpen(broken quilt) -> read.

Confirms the server publishes a `quilt` syntax diagnostic for an unclosed `↖`.
Exits non-zero if no such diagnostic arrives.
"""
import json
import select
import subprocess
import sys
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/debug/quilt-lsp"
sys.stdout.reconfigure(line_buffering=True)


def frame(obj):
    body = json.dumps(obj).encode("utf-8")
    return f"Content-Length: {len(body)}\r\n\r\n".encode("ascii") + body


def read_messages(stream, deadline):
    """Yield JSON-RPC messages until deadline (seconds since epoch).

    Uses select() so a quiet server can never block us past the deadline.
    """
    fd = stream.fileno()
    buf = b""
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], max(0.0, deadline - time.time()))
        if not ready:
            break
        chunk = stream.read1(4096)
        if not chunk:
            break
        buf += chunk
        while True:
            sep = buf.find(b"\r\n\r\n")
            if sep == -1:
                break
            header = buf[:sep].decode("ascii", "replace")
            length = None
            for line in header.split("\r\n"):
                if line.lower().startswith("content-length:"):
                    length = int(line.split(":", 1)[1].strip())
            if length is None or len(buf) < sep + 4 + length:
                break
            body = buf[sep + 4 : sep + 4 + length]
            buf = buf[sep + 4 + length :]
            yield json.loads(body)


import os

proc = subprocess.Popen(
    [BIN],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=sys.stderr,
    env={**os.environ, "RUST_LOG": os.environ.get("RUST_LOG", "debug")},
)

uri = "file:///tmp/broken.rs.quilt"
broken = "fn main() {\n    let x = ↖ 1 + 2;\n}\n"  # unclosed quote

def send(obj):
    proc.stdin.write(frame(obj))
    proc.stdin.flush()


# 1) initialize, and wait for its result before proceeding.
send({
    "jsonrpc": "2.0", "id": 1, "method": "initialize",
    "params": {"processId": None, "rootUri": None, "capabilities": {}},
})
init_ok = False
for msg in read_messages(proc.stdout, time.time() + 5):
    print("<-", msg.get("method") or f"id={msg.get('id')}")
    if msg.get("id") == 1 and "result" in msg:
        init_ok = True
        caps = msg["result"].get("capabilities", {})
        print("initialize ok; positionEncoding =", caps.get("positionEncoding"))
        break

# 2) initialized + didOpen, then wait for diagnostics.
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({
    "jsonrpc": "2.0", "method": "textDocument/didOpen",
    "params": {"textDocument": {"uri": uri, "languageId": "quilt", "version": 1, "text": broken}},
})

found = None
for msg in read_messages(proc.stdout, time.time() + 5):
    print("<-", msg.get("method") or f"id={msg.get('id')}")
    if msg.get("method") == "textDocument/publishDiagnostics":
        diags = msg["params"]["diagnostics"]
        print("diagnostics:", json.dumps(diags))
        if any(d.get("source") == "quilt" for d in diags):
            found = diags
            break

try:
    proc.stdin.write(frame({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}))
    proc.stdin.write(frame({"jsonrpc": "2.0", "method": "exit"}))
    proc.stdin.flush()
except BrokenPipeError:
    pass
proc.terminate()

if not init_ok:
    print("FAIL: no initialize result")
    sys.exit(1)
if not found:
    print("FAIL: no quilt diagnostic for unclosed quote")
    sys.exit(1)
print("PASS")
