#!/usr/bin/env python3
"""End-to-end test for cross-stage go-to-definition against a live rust-analyzer.

Opens the cargo-demo `.rs.quilt`, requests `textDocument/definition` on the
`tag` identifier that is spliced into a quote via `↙tag()↘`, and asserts the
definition lands on the ground `fn tag` line. This proves the stage-aware
projection reabsorbs stage-0 splices as ground code that the host server
resolves. Skips (exit 0) if rust-analyzer can't be started.
"""
import json, os, select, shutil, subprocess, sys, time

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/debug/quilt-lsp"
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FILE = os.path.join(REPO, "examples", "cargo-demo", "src", "main.rs.quilt")
sys.stdout.reconfigure(line_buffering=True)
if shutil.which("rust-analyzer") is None:
    print("SKIP: rust-analyzer not on PATH"); sys.exit(0)

text = open(FILE).read()
uri = "file://" + FILE
root = "file://" + os.path.dirname(os.path.dirname(FILE))  # the crate dir


def frame(o):
    b = json.dumps(o).encode()
    return f"Content-Length: {len(b)}\r\n\r\n".encode() + b


# Locate the spliced `tag` (first "tag(") and the `fn tag` definition line.
def line_col(byte):
    pre = text[:byte]
    return pre.count("\n"), len(pre) - (pre.rfind("\n") + 1)

splice_byte = text.index("tag(")
sl, sc = line_col(splice_byte)
def_line = next(i for i, l in enumerate(text.split("\n")) if l.startswith("fn tag("))
print(f"splice tag @ {sl}:{sc}; expect def at line {def_line}")

proc = subprocess.Popen([BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=subprocess.DEVNULL,
                        env={**os.environ, "RUST_LOG": os.environ.get("RUST_LOG", "warn")})
buf = b""
def send(o): proc.stdin.write(frame(o)); proc.stdin.flush()
def pump(deadline, want):
    global buf
    fd = proc.stdout.fileno()
    while time.time() < deadline:
        while True:
            sep = buf.find(b"\r\n\r\n")
            if sep == -1: break
            length = None
            for ln in buf[:sep].decode("ascii", "replace").split("\r\n"):
                if ln.lower().startswith("content-length:"):
                    length = int(ln.split(":", 1)[1].strip())
            if length is None or len(buf) < sep + 4 + length: break
            msg = json.loads(buf[sep+4:sep+4+length]); buf = buf[sep+4+length:]
            if msg.get("id") is not None and msg.get("method") is not None:
                send({"jsonrpc": "2.0", "id": msg["id"], "result": None}); continue
            r = want(msg)
            if r is not None: return r
        if not select.select([fd], [], [], max(0.0, deadline - time.time()))[0]: continue
        c = proc.stdout.read1(65536)
        if not c: break
        buf += c
    return None


send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"processId": os.getpid(), "rootUri": root,
                 "capabilities": {"textDocument": {"definition": {"linkSupport": True}}}}})
assert pump(time.time()+10, lambda m: True if m.get("id") == 1 and "result" in m else None), "no init"
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
      "params": {"textDocument": {"uri": uri, "languageId": "quilt", "version": 1, "text": text}}})


def def_line_of(result):
    el = result[0] if isinstance(result, list) and result else result
    if not isinstance(el, dict):
        return None
    rng = el.get("targetSelectionRange") or el.get("targetRange") or el.get("range")
    return rng["start"]["line"] if rng else None


got = None
rid = 100
deadline = time.time() + 75
while time.time() < deadline and got is None:
    rid += 1
    send({"jsonrpc": "2.0", "id": rid, "method": "textDocument/definition",
          "params": {"textDocument": {"uri": uri}, "position": {"line": sl, "character": sc}}})
    res = pump(time.time()+4, lambda m, r=rid: m.get("result", "NONE") if m.get("id") == r else None)
    if res and res not in ("NONE", None):
        got = res
    else:
        time.sleep(1.5)

try:
    send({"jsonrpc": "2.0", "id": 999, "method": "shutdown"}); send({"jsonrpc": "2.0", "method": "exit"})
except BrokenPipeError:
    pass
proc.terminate()

print("definition ->", json.dumps(got)[:300])
ln = def_line_of(got) if got else None
if ln == def_line:
    print(f"PASS: F12 on spliced `tag` jumps to ground `fn tag` (line {ln})")
    sys.exit(0)
print(f"FAIL: expected def line {def_line}, got {ln}")
sys.exit(1)
