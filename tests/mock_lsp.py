"""Deterministic stdio LSP fixture: no network, external packages or real workspace writes."""
import json, sys
log = sys.argv[1]
last_opened_uri = None
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
             "typeDefinitionProvider": True, "implementationProvider": True,
             "declarationProvider": True, "workspaceSymbolProvider": True,
             "documentSymbolProvider": True, "documentFormattingProvider": True,
             "documentRangeFormattingProvider": True,
             "renameProvider": True, "codeActionProvider": True,
             "documentHighlightProvider": True, "documentLinkProvider": {},
             "codeLensProvider": {}, "inlayHintProvider": True}})
    elif method == "initialized":
        send({"jsonrpc": "2.0", "id": "config-request", "method": "workspace/configuration",
              "params": {"items": [{"section": "test"}]}})
        if "--progress" in sys.argv:
            send({"jsonrpc": "2.0", "method": "$/progress", "params": {
                 "token": "fixture-progress",
                 "value": {"kind": "begin", "title": "Indexing", "percentage": 0}}})
    elif method == "textDocument/didOpen":
        uri = params["textDocument"]["uri"]
        last_opened_uri = uri
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
             "uri": uri, "diagnostics": [{"range": {"start": position(), "end": position(character=3)},
             "severity": 2, "message": "fixture warning"}]}})
        if "--progress" in sys.argv:
            send({"jsonrpc": "2.0", "method": "$/progress", "params": {
                 "token": "fixture-progress",
                 "value": {"kind": "report", "percentage": 50, "message": "halfway"}}})
    elif method == "textDocument/hover":
        if "--progress" in sys.argv:
            send({"jsonrpc": "2.0", "method": "$/progress", "params": {
                 "token": "fixture-progress", "value": {"kind": "end"}}})
        reply(id, {"contents": {"kind": "plaintext", "value": "fixture hover"}})
    elif method == "textDocument/completion": reply(id, [{"label": "display", "filterText": "FIX", "textEdit": edit("completed"), "kind": 3,
         "documentation": {"kind": "markdown", "value": "fixture docs for FIX"}}])
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
    elif method == "textDocument/rangeFormatting":
        # A distinct replacement text (and echoing the requested range
        # back via its start line) so a test can tell this apart from a
        # whole-buffer format and confirm the right range was sent.
        start_line = params["range"]["start"]["line"]
        reply(id, [{"range": {"start": position(line=start_line), "end": position(line=start_line, character=3)},
             "newText": "RANGEFMT"}])
    elif method == "textDocument/rename": reply(id, {"changes": {params["textDocument"]["uri"]: [edit(params["newName"])]}})
    elif method == "textDocument/codeAction": reply(id, [{"title": "Fix fixture", "edit": {"changes": {params["textDocument"]["uri"]: [edit("FIX")]}}}])
    elif method in ("textDocument/typeDefinition", "textDocument/implementation", "textDocument/declaration"):
        reply(id, {"uri": params["textDocument"]["uri"],
             "range": {"start": position(line=4, character=2), "end": position(line=4, character=6)}})
    elif method == "textDocument/documentHighlight":
        # Two fixed occurrences, deterministic regardless of the requested
        # position -- line 0 chars 4-10 and line 2 chars 0-6.
        reply(id, [
            {"range": {"start": position(line=0, character=4), "end": position(line=0, character=10)}, "kind": 2},
            {"range": {"start": position(line=2, character=0), "end": position(line=2, character=6)}, "kind": 3},
        ])
    elif method == "textDocument/documentLink":
        # One resolvable file:// link (a sibling of the opened document,
        # so a test can create it and confirm opening actually works) and
        # one http(s) link (never opened, just copied -- the editor's own
        # "no browser" policy applies to this regardless of server intent).
        sibling = last_opened_uri.rsplit("/", 1)[0] + "/other.txt" if last_opened_uri else "file:///tmp/other.txt"
        reply(id, [
            {"range": {"start": position(line=0, character=0), "end": position(line=0, character=4)},
             "target": sibling, "tooltip": "Open other.txt"},
            {"range": {"start": position(line=1, character=0), "end": position(line=1, character=4)},
             "target": "https://example.com/docs"},
        ])
    elif method == "textDocument/codeLens":
        # One runnable lens (has a `command`) and one resolve-only lens
        # (no `command`, deferred to codeLens/resolve) -- the editor
        # should show only the runnable one and skip the other, the same
        # "no extra resolve round trip" choice already made for a
        # target-less document link.
        reply(id, [
            {"range": {"start": position(line=0), "end": position(line=0, character=3)},
             "command": {"title": "▶ Run fixture", "command": "fixture.run", "arguments": ["x"]}},
            {"range": {"start": position(line=1), "end": position(line=1, character=3)},
             "data": {"deferred": True}},
        ])
    elif method == "textDocument/inlayHint":
        # One plain-string label with paddingLeft (a type hint, sitting
        # right after "one" on line 0) and one label given as parts
        # (a parameter-name hint on line 1) -- so a test can confirm
        # both label shapes round-trip.
        reply(id, [
            {"position": position(line=0, character=3), "label": ": Type", "paddingLeft": True},
            {"position": position(line=1, character=0), "label": [{"value": "param"}, {"value": ": "}]},
        ])
    elif method == "workspace/symbol":
        # Echoes the query into the symbol name so a test can confirm it
        # actually round-tripped, not just that *some* list came back.
        reply(id, [{"name": f"match_for_{params.get('query','')}", "kind": 12,
             "location": {"uri": last_opened_uri,
             "range": {"start": position(line=1, character=0), "end": position(line=1, character=3)}}}])
    elif id is not None and method: reply(id, None)
