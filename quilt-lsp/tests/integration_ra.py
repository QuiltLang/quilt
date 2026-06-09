#!/usr/bin/env python3
"""End-to-end test: quilt-lsp proxying a live rust-analyzer.

Creates a tiny cargo project containing `src/main.rs.quilt`, opens it through
quilt-lsp, and polls `textDocument/hover` over a local `value` binding until
rust-analyzer has indexed. Asserts the hover comes back (proving project ->
projection -> overlay -> forward -> remap all work) and that its type is `i32`.

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


# --- temp cargo project -----------------------------------------------------
proj = tempfile.mkdtemp(prefix="quilt-lsp-it-")
os.makedirs(os.path.join(proj, "src"))
with open(os.path.join(proj, "Cargo.toml"), "w") as f:
    f.write('[package]\nname = "it"\nversion = "0.0.0"\nedition = "2021"\n\n[[bin]]\nname = "it"\npath = "src/main.rs"\n')
# The de-quilted overlay target must exist on disk so rust-analyzer includes it
# in the crate graph; quilt-lsp overlays our projected content on top.
with open(os.path.join(proj, "src", "main.rs"), "w") as f:
    f.write("fn main() {}\n")

quilt_text = "fn main() {\n    let value = 42;\n    let _ = value;\n}\n"
quilt_path = os.path.join(proj, "src", "main.rs.quilt")
with open(quilt_path, "w") as f:
    f.write(quilt_text)
uri = "file://" + quilt_path

proc = subprocess.Popen(
    [BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    env={**os.environ, "RUST_LOG": os.environ.get("RUST_LOG", "info")},
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
      "params": {"processId": os.getpid(), "rootUri": "file://" + proj, "capabilities": {}}})
init = wait_for(lambda m: True if m.get("id") == 1 and "result" in m else None, 10)
assert init, "no initialize result"
print("initialize ok")

send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
      "params": {"textDocument": {"uri": uri, "languageId": "quilt", "version": 1, "text": quilt_text}}})

# Poll hover over `value` (line 2, char 13) until rust-analyzer answers.
hover = None
hid = 100
deadline = time.time() + 75
while time.time() < deadline and hover is None:
    hid += 1
    send({"jsonrpc": "2.0", "id": hid, "method": "textDocument/hover",
          "params": {"textDocument": {"uri": uri},
                     "position": {"line": 2, "character": 13}}})

    def grab(m, hid=hid):
        if m.get("id") == hid:
            return m.get("result", "NULL")
        return None

    res = wait_for(grab, 4)
    if res and res != "NULL":
        hover = res
    else:
        time.sleep(1.5)

try:
    send({"jsonrpc": "2.0", "id": 999, "method": "shutdown"})
    send({"jsonrpc": "2.0", "method": "exit"})
except BrokenPipeError:
    pass
proc.terminate()
shutil.rmtree(proj, ignore_errors=True)

if hover is None:
    print("FAIL: no hover from rust-analyzer within timeout")
    sys.exit(1)

contents = json.dumps(hover)
print("hover:", contents[:300])
ok = "i32" in contents
# If the hover carried a range, it must be in the quilt doc (line 2 here).
rng = hover.get("range") if isinstance(hover, dict) else None
if rng is not None:
    print("range:", rng)
    ok = ok and rng["start"]["line"] == 2

print("PASS" if ok else "FAIL: hover did not contain expected i32/range")
sys.exit(0 if ok else 1)
