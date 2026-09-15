use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::{json, Value};

use super::protocol::{read_message, write_message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct CompletionResultItem {
    pub label: String,
    pub insert_text: String,
    pub detail: Option<String>,
}

pub enum LspEvent {
    Diagnostics { uri: String, diags: Vec<Diagnostic> },
    Hover { request_id: u64, text: Option<String> },
    Definition { request_id: u64, uri: String, line: usize, col: usize },
    Completion { request_id: u64, items: Vec<CompletionResultItem> },
}

enum PendingKind {
    Initialize,
    Hover(u64),
    Definition(u64),
    Completion(u64),
}

struct PendingOpen {
    uri: String,
    lang_id: String,
    version: i64,
    text: String,
}

pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: i64,
    pending: HashMap<i64, PendingKind>,
    ready: bool,
    pending_opens: Vec<PendingOpen>,
    doc_versions: HashMap<String, i64>,
    pub server_cmd: String,
}

/// Best-effort candidate commands per language id, tried in order until one
/// spawns successfully. Not exhaustive -- covers common installs; anything
/// else is a config knob for later.
pub fn candidates_for(lang_id: &str) -> &'static [&'static [&'static str]] {
    match lang_id {
        "rust" => &[&["rust-analyzer"]],
        "python" => &[&["pylsp"], &["pyright-langserver", "--stdio"], &["basedpyright-langserver", "--stdio"]],
        "javascript" | "typescript" | "javascriptreact" | "typescriptreact" => {
            &[&["typescript-language-server", "--stdio"]]
        }
        "go" => &[&["gopls"]],
        "c" | "cpp" => &[&["clangd"]],
        "lua" => &[&["lua-language-server"]],
        "bash" => &[&["bash-language-server", "start"]],
        "json" => &[&["vscode-json-language-server", "--stdio"]],
        "toml" => &[&["taplo", "lsp", "stdio"]],
        "yaml" => &[&["yaml-language-server", "--stdio"]],
        _ => &[],
    }
}

pub fn lang_id_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "lua" => "lua",
        "sh" | "bash" => "bash",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => return None,
    })
}

impl LspClient {
    pub fn spawn(lang_id: &str, root_uri: &str) -> Option<LspClient> {
        let candidates = candidates_for(lang_id);
        let mut last_cmd = String::new();
        for cmd_parts in candidates {
            let (bin, args) = cmd_parts.split_first()?;
            last_cmd = cmd_parts.join(" ");
            let spawned = Command::new(bin)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let mut child = match spawned {
                Ok(c) => c,
                Err(_) => continue,
            };

            let stdin = child.stdin.take()?;
            let stdout = child.stdout.take()?;
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_message(&mut reader) {
                        Ok(Some(v)) => {
                            if tx.send(v).is_err() {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
            });

            let mut client = LspClient {
                child,
                stdin,
                rx,
                next_id: 1,
                pending: HashMap::new(),
                ready: false,
                pending_opens: Vec::new(),
                doc_versions: HashMap::new(),
                server_cmd: last_cmd.clone(),
            };
            client.initialize(root_uri);
            return Some(client);
        }
        let _ = last_cmd;
        None
    }

    fn send_raw(&mut self, value: &Value) {
        let _ = write_message(&mut self.stdin, value);
    }

    fn request(&mut self, method: &str, params: Value, kind: PendingKind) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, kind);
        self.send_raw(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send_raw(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn initialize(&mut self, root_uri: &str) {
        let params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "completion": { "completionItem": { "snippetSupport": false } },
                    "definition": {},
                    "publishDiagnostics": { "relatedInformation": false }
                }
            }
        });
        self.request("initialize", params, PendingKind::Initialize);
    }

    pub fn did_open(&mut self, uri: &str, lang_id: &str, text: &str) {
        let version = 1;
        if !self.ready {
            self.pending_opens.push(PendingOpen {
                uri: uri.to_string(),
                lang_id: lang_id.to_string(),
                version,
                text: text.to_string(),
            });
            return;
        }
        self.doc_versions.insert(uri.to_string(), version);
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": lang_id, "version": version, "text": text } }),
        );
    }

    pub fn did_change(&mut self, uri: &str, text: &str) {
        if !self.ready {
            return;
        }
        let version = self.doc_versions.entry(uri.to_string()).or_insert(1);
        *version += 1;
        let v = *version;
        self.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri, "version": v }, "contentChanges": [{ "text": text }] }),
        );
    }

    pub fn request_hover(&mut self, uri: &str, line: usize, col: usize, request_id: u64) {
        if !self.ready {
            return;
        }
        let params = json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col } });
        self.request("textDocument/hover", params, PendingKind::Hover(request_id));
    }

    pub fn request_definition(&mut self, uri: &str, line: usize, col: usize, request_id: u64) {
        if !self.ready {
            return;
        }
        let params = json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col } });
        self.request("textDocument/definition", params, PendingKind::Definition(request_id));
    }

    pub fn request_completion(&mut self, uri: &str, line: usize, col: usize, request_id: u64) {
        if !self.ready {
            return;
        }
        let params = json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col } });
        self.request("textDocument/completion", params, PendingKind::Completion(request_id));
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Drains every message currently buffered from the server without
    /// blocking, turning responses/notifications into `LspEvent`s.
    pub fn poll(&mut self) -> Vec<LspEvent> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            self.handle_message(msg, &mut out);
        }
        out
    }

    fn handle_message(&mut self, msg: Value, out: &mut Vec<LspEvent>) {
        let obj = match msg.as_object() {
            Some(o) => o,
            None => return,
        };

        if let Some(id) = obj.get("id").and_then(|v| v.as_i64()) {
            if obj.contains_key("method") {
                // A server-to-client request we don't specifically support;
                // acknowledge it so the server doesn't stall waiting.
                self.send_raw(&json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null }));
                return;
            }
            let Some(kind) = self.pending.remove(&id) else { return };
            let result = obj.get("result").cloned();
            match kind {
                PendingKind::Initialize => {
                    self.ready = true;
                    self.notify("initialized", json!({}));
                    let opens = std::mem::take(&mut self.pending_opens);
                    for o in opens {
                        self.doc_versions.insert(o.uri.clone(), o.version);
                        self.notify(
                            "textDocument/didOpen",
                            json!({ "textDocument": { "uri": o.uri, "languageId": o.lang_id, "version": o.version, "text": o.text } }),
                        );
                    }
                }
                PendingKind::Hover(request_id) => {
                    let text = result.as_ref().and_then(extract_hover_text);
                    out.push(LspEvent::Hover { request_id, text });
                }
                PendingKind::Definition(request_id) => {
                    if let Some((uri, line, col)) = result.as_ref().and_then(extract_first_location) {
                        out.push(LspEvent::Definition { request_id, uri, line, col });
                    }
                }
                PendingKind::Completion(request_id) => {
                    let items = result.as_ref().map(extract_completion_items).unwrap_or_default();
                    out.push(LspEvent::Completion { request_id, items });
                }
            }
            return;
        }

        if let Some(method) = obj.get("method").and_then(|v| v.as_str()) {
            if method == "textDocument/publishDiagnostics" {
                if let Some(params) = obj.get("params") {
                    if let (Some(uri), Some(diags)) =
                        (params.get("uri").and_then(|v| v.as_str()), params.get("diagnostics").and_then(|v| v.as_array()))
                    {
                        let parsed = diags.iter().filter_map(parse_diagnostic).collect();
                        out.push(LspEvent::Diagnostics { uri: uri.to_string(), diags: parsed });
                    }
                }
            }
            // Other notifications (log messages, progress) are ignored.
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn extract_hover_text(result: &Value) -> Option<String> {
    if result.is_null() {
        return None;
    }
    let contents = result.get("contents")?;
    if let Some(s) = contents.as_str() {
        return Some(s.to_string());
    }
    if let Some(obj) = contents.as_object() {
        if let Some(v) = obj.get("value").and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    if let Some(arr) = contents.as_array() {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|v| {
                v.as_str().map(String::from).or_else(|| v.get("value").and_then(|v| v.as_str()).map(String::from))
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    None
}

fn extract_first_location(result: &Value) -> Option<(String, usize, usize)> {
    let loc = if let Some(arr) = result.as_array() {
        arr.first()?
    } else if result.is_object() {
        result
    } else {
        return None;
    };
    let uri = loc.get("uri").and_then(|v| v.as_str())?.to_string();
    let range = loc.get("range")?;
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()? as usize;
    let col = start.get("character")?.as_u64()? as usize;
    Some((uri, line, col))
}

fn extract_completion_items(result: &Value) -> Vec<CompletionResultItem> {
    let items = if let Some(arr) = result.as_array() {
        arr.clone()
    } else if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
        items.clone()
    } else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|it| {
            let label = it.get("label")?.as_str()?.to_string();
            let insert_text = it
                .get("insertText")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| label.clone());
            let detail = it.get("detail").and_then(|v| v.as_str()).map(String::from);
            Some(CompletionResultItem { label, insert_text, detail })
        })
        .take(50)
        .collect()
}

fn parse_diagnostic(v: &Value) -> Option<Diagnostic> {
    let range = v.get("range")?;
    let start = range.get("start")?;
    let end = range.get("end")?;
    let severity = match v.get("severity").and_then(|s| s.as_u64()) {
        Some(1) => Severity::Error,
        Some(2) => Severity::Warning,
        Some(3) => Severity::Info,
        _ => Severity::Hint,
    };
    Some(Diagnostic {
        line: start.get("line")?.as_u64()? as usize,
        col: start.get("character")?.as_u64()? as usize,
        end_line: end.get("line")?.as_u64()? as usize,
        end_col: end.get("character")?.as_u64()? as usize,
        severity,
        message: v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string(),
    })
}
