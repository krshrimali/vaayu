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
              "params": {"items": [{"section": "test"}, {"section": "json.schemas"},
                                    {"section": "yaml.schemas"}]}})
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
    elif method == "textDocument/didChange" and "--diag-on-change" in sys.argv:
        # Opt-in (existing tests that edit-then-sync without this flag
        # must keep seeing only didOpen's diagnostics) -- a distinct
        # diagnostic set (error, with source/code and one
        # relatedInformation entry) from didOpen's plain warning, so a
        # test can tell the two apart: confirming an Insert-mode update
        # actually got deferred, or checking code/source/related
        # information round-trip at all.
        uri = params["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
             "uri": uri, "diagnostics": [{"range": {"start": position(), "end": position(character=3)},
             "severity": 1, "message": "fixture error", "source": "eslint", "code": "no-unused-vars",
             "relatedInformation": [{"location": {"uri": uri, "range": {"start": position(line=1),
             "end": position(line=1, character=3)}}, "message": "declared here"}]}]}})
    elif method == "textDocument/hover":
        if "--progress" in sys.argv:
            send({"jsonrpc": "2.0", "method": "$/progress", "params": {
                 "token": "fixture-progress", "value": {"kind": "end"}}})
        reply(id, {"contents": {"kind": "plaintext", "value": "fixture hover"}})
    elif method == "textDocument/completion": reply(id, [{"label": "display", "filterText": "FIX", "textEdit": edit("completed"), "kind": 3,
         "documentation": {"kind": "markdown", "value": "fixture docs for FIX"}},
         # A snippet item mixing a choice, a placeholder nested inside
         # another one's default, and a plain final tab stop -- so a test
         # can drive the choices UI and nested-placeholder tabbing
         # through a real completion round trip.
         {"label": "snip", "filterText": "SNIP", "insertTextFormat": 2,
          "textEdit": edit("fn ${1|foo,bar,baz|}(${2:arg ${3:nested}}) {$0}")}])
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
    elif method == "textDocument/codeAction":
        only = (params.get("context") or {}).get("only")
        if only == ["source.organizeImports"]:
            # Exactly one action, like a real organize-imports server
            # reply usually is -- the editor applies it directly, no
            # picker.
            reply(id, [{"title": "Organize Imports", "kind": "source.organizeImports",
                 "edit": {"changes": {params["textDocument"]["uri"]: [edit("ORGANIZED")]}}}])
        else:
            # A preferred, runnable action (kept first/isPreferred so
            # existing tests that just apply cursor 0 still get "Fix
            # fixture") plus a disabled one -- the editor must show
            # both (not silently drop the disabled one) and refuse to
            # actually run it.
            reply(id, [
                {"title": "Fix fixture", "isPreferred": True,
                 "edit": {"changes": {params["textDocument"]["uri"]: [edit("FIX")]}}},
                {"title": "Disabled fixture", "disabled": {"reason": "not applicable here"}},
            ])
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
