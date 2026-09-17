"""Deterministic stdio LSP fixture: no network, external packages or real workspace writes."""
import json, sys
log = sys.argv[1]
def send(value):
    data = json.dumps(value).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    sys.stdout.buffer.flush()
def reply(id, result):
    send({"jsonrpc": "2.0", "id": id, "result": result})
def position(line=0, character=0):
    return {"line": line, "character": character}
def edit(text):
    return {"range": {"start": position(), "end": position(character=3)}, "newText": text}
while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line == b"\r\n": break
        k, v = line.decode().split(":", 1)
        headers[k.lower()] = v.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    with open(log, "a") as f: f.write(json.dumps(message) + "\n")
    method = message.get("method")
    params = message.get("params", {})
    id = message.get("id")
    if method == "vaayu/hang": continue
    if method == "initialize" and "--hang-init" in sys.argv: continue
    if method == "completionItem/resolve":
        params["detail"]="resolved detail"
        reply(id, params)
        continue
    if method == "initialize":
        reply(id, {"capabilities": {"textDocumentSync": 1, "hoverProvider": True,
             "completionProvider": {"resolveProvider": True}, "definitionProvider": True,
             "documentSymbolProvider": True, "documentFormattingProvider": True,
             "renameProvider": True, "codeActionProvider": True}})
    elif method == "initialized":
        send({"jsonrpc": "2.0", "id": "config-request", "method": "workspace/configuration",
              "params": {"items": [{"section": "test"}]}})
    elif method == "textDocument/didOpen":
        uri = params["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
             "uri": uri, "diagnostics": [{"range": {"start": position(), "end": position(character=3)},
             "severity": 2, "message": "fixture warning"}]}})
    elif method == "textDocument/hover": reply(id, {"contents": {"kind": "plaintext", "value": "fixture hover"}})
    elif method == "textDocument/completion": reply(id, [{"label": "display", "filterText": "FIX", "textEdit": edit("completed")}])
    elif method == "textDocument/documentSymbol":
        if "--nested-symbol" in sys.argv:
            reply(id, [
                {"name": "Parent", "kind": 5,
                 "selectionRange": {"start": position(line=0), "end": position(line=0, character=6)},
                 "children": [
                     {"name": "Child", "kind": 6,
                      "selectionRange": {"start": position(line=1), "end": position(line=1, character=5)}},
                 ]},
                {"name": "Sibling", "kind": 12,
                 "selectionRange": {"start": position(line=2), "end": position(line=2, character=7)}},
            ])
        elif "--multi-symbol" in sys.argv:
            reply(id, [
                {"name": "a_fn", "kind": 12,
                 "selectionRange": {"start": position(line=0), "end": position(line=0, character=3)}},
                {"name": "b_fn", "kind": 12,
                 "selectionRange": {"start": position(line=1), "end": position(line=1, character=3)}},
                {"name": "c_var", "kind": 13,
                 "selectionRange": {"start": position(line=2), "end": position(line=2, character=3)}},
            ])
        else:
            reply(id, [{"name": "symbol", "kind": 12,
                 "range": {"start": position(), "end": position(character=3)},
                 "selectionRange": {"start": position(character=3), "end": position(character=3)}}])
    elif method == "textDocument/formatting": reply(id, [edit("FMT")])
    elif method == "textDocument/rename": reply(id, {"changes": {params["textDocument"]["uri"]: [edit(params["newName"])]}})
    elif method == "textDocument/codeAction": reply(id, [{"title": "Fix fixture", "edit": {"changes": {params["textDocument"]["uri"]: [edit("FIX")]}}}])
    elif id is not None and method: reply(id, None)
