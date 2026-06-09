#!/usr/bin/env python3
"""Deterministic proxy test using a mock downstream server (no rust-analyzer).

Opens a `.rs.quilt` file containing an inline quote, then hovers over a symbol
*after* the quote on the same line. The quote `↖X↗` (3 chars) projects to `()`
(2 chars), so the symbol shifts by one column in the virtual document. The mock
echoes the virtual position it receives as the hover range; quilt-lsp must map
that range back so the returned range start equals the symbol's *quilt* column.
This exercises the non-identity source map through the whole server.
"""
import json
import os
import select
import subprocess
import sys
import tempfile
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/debug/quilt-lsp"
MOCK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "mock_server.py")
sys.stdout.reconfigure(line_buffering=True)


def frame(obj):
    body = json.dumps(obj).encode("utf-8")
    return f"Content-Length: {len(body)}\r\n\r\n".encode("ascii") + body


def read_messages(stream, deadline):
    fd = stream.fileno()
    buf = b""
    while time.time() < deadline:
        if not select.select([fd], [], [], max(0.0, deadline - time.time()))[0]:
            break
        chunk = stream.read1(65536)
        if not chunk:
            break
        buf += chunk
        while True:
            sep = buf.find(b"\r\n\r\n")
            if sep == -1:
                break
            length = None
            for line in buf[:sep].decode("ascii", "replace").split("\r\n"):
                if line.lower().startswith("content-length:"):
                    length = int(line.split(":", 1)[1].strip())
            if length is None or len(buf) < sep + 4 + length:
                break
            body = buf[sep + 4 : sep + 4 + length]
            buf = buf[sep + 4 + length :]
            yield json.loads(body)


tmp = tempfile.mkdtemp(prefix="quilt-lsp-mock-")
quilt_text = "fn main() {\n    let bb = 1;\n    let a = ↖X↗ + bb;\n}\n"
quilt_path = os.path.join(tmp, "main.rs.quilt")
with open(quilt_path, "w") as f:
    f.write(quilt_text)
uri = "file://" + quilt_path

# Column of `bb`'s use on line 3 (0-indexed line 2) of the *quilt* doc:
#   "    let a = ↖X↗ + bb;"
#    0123456789...           4 spaces + "let a = " (8) = 12; "↖X↗" = 3 (12..15);
#   " + " (15..18); "bb" starts at 18.
target = {"line": 2, "character": 18}

proc = subprocess.Popen(
    [BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    env={**os.environ,
         "QUILT_LSP_RUST_ANALYZER": f"python3 {MOCK}",
         "RUST_LOG": os.environ.get("RUST_LOG", "warn")},
)


def send(obj):
    proc.stdin.write(frame(obj))
    proc.stdin.flush()


def wait_for(pred, timeout):
    for msg in read_messages(proc.stdout, time.time() + timeout):
        r = pred(msg)
        if r is not None:
            return r
    return None


send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"processId": os.getpid(), "rootUri": "file://" + tmp, "capabilities": {}}})
assert wait_for(lambda m: True if m.get("id") == 1 and "result" in m else None, 10), "no init"
print("initialize ok")

send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
      "params": {"textDocument": {"uri": uri, "languageId": "quilt", "version": 1, "text": quilt_text}}})

send({"jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
      "params": {"textDocument": {"uri": uri}, "position": target}})
hover = wait_for(lambda m: m.get("result") if m.get("id") == 2 else None, 10)

try:
    send({"jsonrpc": "2.0", "id": 3, "method": "shutdown"})
    send({"jsonrpc": "2.0", "method": "exit"})
except BrokenPipeError:
    pass
proc.terminate()
import shutil
shutil.rmtree(tmp, ignore_errors=True)

print("hover:", json.dumps(hover))
if not hover or "range" not in hover:
    print("FAIL: no hover/range")
    sys.exit(1)

start = hover["range"]["start"]
# Must map back to the quilt column (18), not the virtual column (17).
if start == target:
    print("PASS: range start mapped back to quilt coords", start)
    sys.exit(0)
print(f"FAIL: expected {target}, got {start}")
sys.exit(1)
