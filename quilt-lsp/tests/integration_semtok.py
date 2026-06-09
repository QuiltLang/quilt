#!/usr/bin/env python3
"""End-to-end semantic-tokens test against a live rust-analyzer.

Opens a `.rs.quilt` file with a quote `↖v + 1↗` on a known line, then requests
`textDocument/semanticTokens/full`. Asserts tokens come back and that at least
one lands on the quote's line (proving the appended-fragment tokens are remapped
back onto the quote). Also exercises the dynamic `client/registerCapability`
handshake (the driver must answer it or the server would stall).

Skips (exit 0) if rust-analyzer can't be started.
"""
import json
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/debug/quilt-lsp"
sys.stdout.reconfigure(line_buffering=True)

if shutil.which("rust-analyzer") is None:
    print("SKIP: rust-analyzer not on PATH")
    sys.exit(0)


def frame(obj):
    body = json.dumps(obj).encode("utf-8")
    return f"Content-Length: {len(body)}\r\n\r\n".encode("ascii") + body


proj = tempfile.mkdtemp(prefix="quilt-lsp-st-")
os.makedirs(os.path.join(proj, "src"))
with open(os.path.join(proj, "Cargo.toml"), "w") as f:
    f.write('[package]\nname="st"\nversion="0.0.0"\nedition="2021"\n\n[[bin]]\nname="st"\npath="src/main.rs"\n')
with open(os.path.join(proj, "src", "main.rs"), "w") as f:
    f.write("fn main() {}\n")

# Quote is on line 2 (0-indexed).
quilt_text = "fn main() {\n    let v = 1;\n    let _ = ↖v + 1↗;\n}\n"
quilt_path = os.path.join(proj, "src", "main.rs.quilt")
with open(quilt_path, "w") as f:
    f.write(quilt_text)
uri = "file://" + quilt_path
QUOTE_LINE = 2

proc = subprocess.Popen(
    [BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    env={**os.environ, "RUST_LOG": os.environ.get("RUST_LOG", "warn")},
)


def send(obj):
    proc.stdin.write(frame(obj))
    proc.stdin.flush()


buf = b""


def pump(deadline, want):
    """Read frames until `want(msg)` returns non-None. Auto-reply null to any
    server->client request so the server never stalls."""
    global buf
    fd = proc.stdout.fileno()
    while time.time() < deadline:
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
            msg = json.loads(buf[sep + 4 : sep + 4 + length])
            buf = buf[sep + 4 + length :]
            if msg.get("id") is not None and msg.get("method") is not None:
                send({"jsonrpc": "2.0", "id": msg["id"], "result": None})  # ack registration etc.
                continue
            r = want(msg)
            if r is not None:
                return r
        if not select.select([fd], [], [], max(0.0, deadline - time.time()))[0]:
            continue
        chunk = proc.stdout.read1(65536)
        if not chunk:
            break
        buf += chunk
    return None


caps = {"textDocument": {"semanticTokens": {
    "dynamicRegistration": True,
    "requests": {"full": True},
    "tokenTypes": [], "tokenModifiers": [], "formats": ["relative"],
}}}
send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"processId": os.getpid(), "rootUri": "file://" + proj, "capabilities": caps}})
assert pump(time.time() + 10, lambda m: True if m.get("id") == 1 and "result" in m else None), "no init"
print("initialize ok")

send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
      "params": {"textDocument": {"uri": uri, "languageId": "quilt", "version": 1, "text": quilt_text}}})

data = None
rid = 100
deadline = time.time() + 75
while time.time() < deadline and not data:
    rid += 1
    send({"jsonrpc": "2.0", "id": rid, "method": "textDocument/semanticTokens/full",
          "params": {"textDocument": {"uri": uri}}})
    res = pump(time.time() + 4, lambda m, rid=rid: m.get("result", "NONE") if m.get("id") == rid else None)
    if isinstance(res, dict) and res.get("data"):
        data = res["data"]
    else:
        time.sleep(1.5)

try:
    send({"jsonrpc": "2.0", "id": 999, "method": "shutdown"})
    send({"jsonrpc": "2.0", "method": "exit"})
except BrokenPipeError:
    pass
proc.terminate()
shutil.rmtree(proj, ignore_errors=True)

if not data:
    print("FAIL: no semantic tokens returned")
    sys.exit(1)

# Decode delta-encoded tokens to absolute lines.
lines = []
line = 0
for i in range(0, len(data), 5):
    dl = data[i]
    line = line + dl
    lines.append(line)
print("token lines:", sorted(set(lines)))
if QUOTE_LINE in lines:
    print(f"PASS: a token maps onto the quote line {QUOTE_LINE}")
    sys.exit(0)
print(f"FAIL: no token on quote line {QUOTE_LINE}")
sys.exit(1)
