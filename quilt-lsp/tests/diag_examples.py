#!/usr/bin/env python3
"""Diagnostic: drive quilt-lsp against a real example file and report what the
downstream rust-analyzer returns for documentSymbol + hover, plus server logs."""
import json, os, select, subprocess, sys, time, tempfile

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/debug/quilt-lsp"
FILE = sys.argv[2] if len(sys.argv) > 2 else "/Users/avarga/Documents/Quilt2/examples/hello.rs.quilt"
sys.stdout.reconfigure(line_buffering=True)

text = open(FILE).read()
uri = "file://" + FILE
root = "file://" + os.path.dirname(FILE)

def frame(o):
    b = json.dumps(o).encode()
    return f"Content-Length: {len(b)}\r\n\r\n".encode() + b

errf = tempfile.NamedTemporaryFile(prefix="qlsp-stderr-", suffix=".log", delete=False)
proc = subprocess.Popen([BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=errf, env={**os.environ, "RUST_LOG": "info,quilt_lsp=debug"})
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
            for line in buf[:sep].decode("ascii","replace").split("\r\n"):
                if line.lower().startswith("content-length:"):
                    length = int(line.split(":",1)[1].strip())
            if length is None or len(buf) < sep+4+length: break
            msg = json.loads(buf[sep+4:sep+4+length]); buf = buf[sep+4+length:]
            if msg.get("id") is not None and msg.get("method") is not None:
                send({"jsonrpc":"2.0","id":msg["id"],"result":None}); continue
            r = want(msg)
            if r is not None: return r
        if not select.select([fd],[],[],max(0.0,deadline-time.time()))[0]: continue
        c = proc.stdout.read1(65536)
        if not c: break
        buf += c
    return None

send({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":os.getpid(),"rootUri":root,
     "capabilities":{"textDocument":{"semanticTokens":{"dynamicRegistration":True,"requests":{"full":True},
     "tokenTypes":[],"tokenModifiers":[],"formats":["relative"]}}}}})
pump(time.time()+10, lambda m: True if m.get("id")==1 and "result" in m else None)
send({"jsonrpc":"2.0","method":"initialized","params":{}})
send({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"quilt","version":1,"text":text}}})

def poll(method, params, timeout=60):
    rid=[200]
    end=time.time()+timeout
    while time.time()<end:
        rid[0]+=1
        send({"jsonrpc":"2.0","id":rid[0],"method":method,"params":params})
        res=pump(time.time()+4, lambda m,r=rid[0]: m.get("result","NONE") if m.get("id")==r else None)
        if res not in (None,"NONE","NULL") and res is not None and res!=[]:
            return res
        time.sleep(1.0)
    return res

ds = poll("textDocument/documentSymbol", {"textDocument":{"uri":uri}})
print("documentSymbol ->", json.dumps(ds)[:400])
# hover target: explicit argv[3]/argv[4], else first `fn <name>`.
lines = text.split("\n")
if len(sys.argv) > 4:
    hl, hc = int(sys.argv[3]), int(sys.argv[4])
else:
    import re
    hl = next((i for i, l in enumerate(lines) if re.search(r'\bfn\s+\w', l)), 2)
    m = re.search(r'\bfn\s+(\w+)', lines[hl]) if hl < len(lines) else None
    hc = lines[hl].index(m.group(1)) if m else 3
hv = poll("textDocument/hover", {"textDocument":{"uri":uri},"position":{"line":hl,"character":hc}}, 60)
print(f"hover @ {hl}:{hc} ->", json.dumps(hv)[:400])

st = poll("textDocument/semanticTokens/full", {"textDocument":{"uri":uri}}, 20)
ntok = (len(st["data"])//5) if isinstance(st, dict) and isinstance(st.get("data"), list) else 0
print("semanticTokens count ->", ntok)

try:
    send({"jsonrpc":"2.0","id":999,"method":"shutdown"}); send({"jsonrpc":"2.0","method":"exit"})
except BrokenPipeError: pass
time.sleep(0.5); proc.terminate()
errf.flush()
print("\n=== server log tail ===")
print("".join(open(errf.name).readlines()[-40:]))
