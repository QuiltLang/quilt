#!/usr/bin/env python3
"""A tiny mock downstream LSP server for testing quilt-lsp's proxy layer.

Responds to `initialize`, replies `null` to most requests, and for
`textDocument/hover` echoes the requested position back as the hover range
(start = position, end = position + 2 chars). That lets the test assert that
quilt-lsp maps positions virtual->quilt correctly without needing a real
rust-analyzer.
"""
import json
import sys

stdin = sys.stdin.buffer
stdout = sys.stdout.buffer


def write(obj):
    body = json.dumps(obj).encode("utf-8")
    stdout.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
    stdout.write(body)
    stdout.flush()


def read():
    length = None
    while True:
        line = stdin.readline()
        if not line:
            return None
        line = line.strip()
        if line == b"":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1].strip())
    if length is None:
        return None
    return json.loads(stdin.read(length))


while True:
    msg = read()
    if msg is None:
        break
    method = msg.get("method")
    mid = msg.get("id")

    if method == "initialize":
        write({"jsonrpc": "2.0", "id": mid, "result": {
            "capabilities": {"hoverProvider": True, "textDocumentSync": 1},
            "serverInfo": {"name": "mock"},
        }})
    elif method == "shutdown":
        write({"jsonrpc": "2.0", "id": mid, "result": None})
    elif method == "exit":
        break
    elif method == "textDocument/hover":
        pos = msg["params"]["position"]
        write({"jsonrpc": "2.0", "id": mid, "result": {
            "contents": {"kind": "markdown", "value": "mock hover"},
            "range": {
                "start": {"line": pos["line"], "character": pos["character"]},
                "end": {"line": pos["line"], "character": pos["character"] + 2},
            },
        }})
    elif mid is not None:
        # Any other request: minimal null reply.
        write({"jsonrpc": "2.0", "id": mid, "result": None})
    # notifications (didOpen/didChange/initialized/...) are ignored
