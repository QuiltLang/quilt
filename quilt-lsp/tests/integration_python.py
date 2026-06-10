#!/usr/bin/env python3
"""Python ground-language tests: quilt-lsp driving a Python downstream server.

Three phases:

1. **Mock hover** (always runs, deterministic): opens a `.py.quilt` file with an
   inline quote and hovers over a symbol *after* the quote on the same line. The
   quote `↖X↗` (3 chars) projects to `()` (2 chars), so the symbol shifts by one
   column in the virtual document. The mock echoes the virtual position it
   receives as the hover range; quilt-lsp must map it back to the quilt column.
   This exercises the Python projection + source map through the whole server.

2. **Mock semantic tokens** (always runs, deterministic): the mock downstream
   advertises no `semanticTokensProvider` — exactly like pyright — so quilt-lsp
   must register the capability itself with its tree-sitter fallback legend and
   answer `semanticTokens/full` from the in-process Python highlighter. Asserts
   a `keyword` token covers `def` on the ground and a token lands inside the
   quote body (appended-fragment remap).

3. **Pyright** (skips if `pyright-langserver` is not on PATH): opens a
   `.py.quilt` defining and calling a function, polls hover over the call until
   pyright has indexed, then asserts go-to-definition lands back in the
   `.py.quilt` file on the `def` line.
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
MOCK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "mock_server.py")
sys.stdout.reconfigure(line_buffering=True)


def frame(obj):
    body = json.dumps(obj).encode("utf-8")
    return f"Content-Length: {len(body)}\r\n\r\n".encode("ascii") + body


class Session:
    """One quilt-lsp process with a `.py.quilt` document opened to it.

    Server→client requests (e.g. `client/registerCapability`,
    `workspace/semanticTokens/refresh`) are auto-acked with `null` so the
    server never stalls; registrations are recorded in `self.registrations`.
    """

    def __init__(self, quilt_text, env_overrides, capabilities=None):
        self.tmp = tempfile.mkdtemp(prefix="quilt-lsp-py-")
        self.path = os.path.join(self.tmp, "main.py.quilt")
        with open(self.path, "w") as f:
            f.write(quilt_text)
        self.uri = "file://" + self.path
        self.buf = b""
        self.registrations = []
        env = {**os.environ, "RUST_LOG": os.environ.get("RUST_LOG", "warn")}
        env.pop("QUILT_LSP_PYTHON_SERVER", None)
        env.update(env_overrides)
        self.proc = subprocess.Popen(
            [BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, env=env,
        )
        self.send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"processId": os.getpid(), "rootUri": "file://" + self.tmp,
                              "capabilities": capabilities or {}}})
        assert self.wait_for(lambda m: True if m.get("id") == 1 and "result" in m else None, 10), \
            "no initialize result"
        self.send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
        self.send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
                   "params": {"textDocument": {"uri": self.uri, "languageId": "quilt",
                                               "version": 1, "text": quilt_text}}})

    def send(self, obj):
        self.proc.stdin.write(frame(obj))
        self.proc.stdin.flush()

    def wait_for(self, pred, timeout):
        """Read frames until `pred(msg)` returns non-None, auto-acking any
        server→client request along the way."""
        fd = self.proc.stdout.fileno()
        deadline = time.time() + timeout
        while time.time() < deadline:
            while True:
                sep = self.buf.find(b"\r\n\r\n")
                if sep == -1:
                    break
                length = None
                for line in self.buf[:sep].decode("ascii", "replace").split("\r\n"):
                    if line.lower().startswith("content-length:"):
                        length = int(line.split(":", 1)[1].strip())
                if length is None or len(self.buf) < sep + 4 + length:
                    break
                msg = json.loads(self.buf[sep + 4: sep + 4 + length])
                self.buf = self.buf[sep + 4 + length:]
                if msg.get("id") is not None and msg.get("method") is not None:
                    if msg["method"] == "client/registerCapability":
                        self.registrations.extend(
                            msg.get("params", {}).get("registrations", []))
                    self.send({"jsonrpc": "2.0", "id": msg["id"], "result": None})
                    continue
                r = pred(msg)
                if r is not None:
                    return r
            if not select.select([fd], [], [], max(0.0, deadline - time.time()))[0]:
                continue
            chunk = self.proc.stdout.read1(65536)
            if not chunk:
                break
            self.buf += chunk
        return None

    def request(self, rid, method, params, timeout=10):
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        return self.wait_for(lambda m: m.get("result", "NULL") if m.get("id") == rid else None,
                             timeout)

    def position_request(self, rid, method, position, timeout=10):
        return self.request(rid, method,
                            {"textDocument": {"uri": self.uri}, "position": position}, timeout)

    def close(self):
        try:
            self.send({"jsonrpc": "2.0", "id": 999, "method": "shutdown"})
            self.send({"jsonrpc": "2.0", "method": "exit"})
        except BrokenPipeError:
            pass
        self.proc.terminate()
        shutil.rmtree(self.tmp, ignore_errors=True)


# --- phase 1: mock hover (deterministic position remap) ----------------------
# Column of `bb`'s use on line 2 (0-indexed line 1) of the *quilt* doc:
#   "a = ↖X↗ + bb"
#   `a = ` (0..4); `↖X↗` = 3 chars (4..7); ` + ` (7..10); `bb` starts at 10.
# The quote projects to `()` so `bb` sits at virtual column 9.
s = Session("bb = 1\na = ↖X↗ + bb\n",
            {"QUILT_LSP_PYTHON_SERVER": f"python3 {MOCK}"})
target = {"line": 1, "character": 10}
hover = s.position_request(2, "textDocument/hover", target)
s.close()
print("mock hover:", json.dumps(hover))
if not hover or hover == "NULL" or "range" not in hover:
    print("FAIL: no hover/range from mock via Python ground")
    sys.exit(1)
if hover["range"]["start"] != target:
    print(f"FAIL: expected {target}, got {hover['range']['start']}")
    sys.exit(1)
print("PASS: mock range start mapped back to quilt coords", target)

# --- phase 2: semantic tokens via the tree-sitter fallback --------------------
# The mock advertises no semanticTokensProvider (like pyright), so quilt-lsp
# registers its fallback legend and highlights the ground projection in-process.
SEMTOK_CAPS = {"textDocument": {"semanticTokens": {
    "dynamicRegistration": True,
    "requests": {"full": True},
    "tokenTypes": [], "tokenModifiers": [], "formats": ["relative"],
}}}
s = Session("def f(x):\n    return x\n\nq = ↖1 + 2↗\n",
            {"QUILT_LSP_PYTHON_SERVER": f"python3 {MOCK}"}, SEMTOK_CAPS)

data = None
rid = 100
deadline = time.time() + 20
while time.time() < deadline and not data:
    rid += 1
    res = s.request(rid, "textDocument/semanticTokens/full",
                    {"textDocument": {"uri": s.uri}}, timeout=3)
    if isinstance(res, dict) and res.get("data"):
        data = res["data"]
    else:
        time.sleep(0.5)

legend_types = []
for reg in s.registrations:
    if reg.get("method") == "textDocument/semanticTokens":
        legend_types = reg["registerOptions"]["legend"]["tokenTypes"]
s.close()

if not data:
    print("FAIL: no semantic tokens from the tree-sitter fallback")
    sys.exit(1)
if "keyword" not in legend_types:
    print(f"FAIL: fallback legend not registered (got {legend_types})")
    sys.exit(1)

# Decode the delta-encoded tokens to absolute (line, start, length, type).
toks, line, ch = [], 0, 0
for i in range(0, len(data), 5):
    dl, ds, ln, ty = data[i], data[i + 1], data[i + 2], data[i + 3]
    if dl:
        line, ch = line + dl, ds
    else:
        ch += ds
    toks.append((line, ch, ln, legend_types[ty]))
print("fallback tokens:", toks)

# `def` on line 0 must be a keyword token; the quote body `↖1 + 2↗` on line 3
# must carry a number token (appended-fragment remap; `1` is at column 5).
if (0, 0, 3, "keyword") not in toks:
    print("FAIL: no keyword token over `def`")
    sys.exit(1)
if not any(t[0] == 3 and t[3] == "number" for t in toks):
    print("FAIL: no number token inside the quote body")
    sys.exit(1)
print("PASS: tree-sitter fallback tokens for ground + quote body")

# --- phase 3: real pyright ----------------------------------------------------
if shutil.which("pyright-langserver") is None:
    print("SKIP: pyright-langserver not on PATH")
    sys.exit(0)

quilt_text = 'def greet(name):\n    return "hi " + name\n\nq = ↖X↗\nmsg = greet("world")\n'
s = Session(quilt_text, {})
call_pos = {"line": 4, "character": 7}  # inside `greet` on the call line

# Poll hover over the call until pyright has indexed.
hover = None
rid = 200
deadline = time.time() + 60
while time.time() < deadline and hover is None:
    rid += 1
    res = s.position_request(rid, "textDocument/hover", call_pos, timeout=4)
    if res and res != "NULL":
        hover = res
    else:
        time.sleep(1.0)

if hover is None:
    s.close()
    print("FAIL: no hover from pyright within timeout")
    sys.exit(1)
contents = json.dumps(hover)
print("pyright hover:", contents[:200])
if "greet" not in contents:
    s.close()
    print("FAIL: hover did not mention `greet`")
    sys.exit(1)

# Go-to-definition on the call must land on the `def greet` line of the
# *.py.quilt* file (URI and position both remapped back from the virtual doc).
defn = s.position_request(500, "textDocument/definition", call_pos)
s.close()
print("pyright definition:", json.dumps(defn)[:300])
loc = defn[0] if isinstance(defn, list) and defn else defn if isinstance(defn, dict) else None
ok = False
if loc:
    uri = loc.get("uri") or loc.get("targetUri") or ""
    rng = loc.get("range") or loc.get("targetSelectionRange") or {}
    ok = uri.endswith("main.py.quilt") and rng.get("start", {}).get("line") == 0
if not ok:
    print("FAIL: definition did not land on `def greet` in the .py.quilt file")
    sys.exit(1)
print("PASS: pyright hover + go-to-definition through the Python ground")
sys.exit(0)
