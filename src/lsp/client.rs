use super::protocol::{read_message, write_message};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::BufReader,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
};
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
    pub raw: Value,
}
#[derive(Debug, Clone)]
pub struct CompletionResultItem {
    pub label: String,
    pub insert_text: String,
    pub detail: Option<String>,
    pub edit: Option<Value>,
    pub additional: Vec<Value>,
}
pub enum LspEvent {
    Diagnostics {
        uri: String,
        diags: Vec<Diagnostic>,
    },
    Response {
        request_id: u64,
        result: Value,
        error: Option<String>,
    },
    ApplyEdit {
        id: Value,
        edit: Value,
    },
    Error(String),
}
pub struct LspClient {
    child: Child,
    tx: SyncSender<Value>,
    rx: Receiver<Result<Value, String>>,
    next_id: i64,
    pending: HashMap<i64, u64>,
    ready: bool,
    pending_docs: HashMap<String, (String, String)>,
    doc_versions: HashMap<String, i64>,
    pub server_cmd: String,
    pub root: String,
    pub capabilities: Value,
    settings: Value,
}
pub fn candidates_for(lang: &str) -> Vec<Vec<String>> {
    let list: Vec<Vec<&str>> = match lang {
        "rust" => vec![vec!["rust-analyzer"]],
        "python" => vec![vec!["pylsp"], vec!["pyright-langserver", "--stdio"]],
        "javascript" | "typescript" | "javascriptreact" | "typescriptreact" => {
            vec![vec!["typescript-language-server", "--stdio"]]
        }
        "go" => vec![vec!["gopls"]],
        "c" | "cpp" => vec![vec!["clangd"]],
        "lua" => vec![vec!["lua-language-server"]],
        "bash" => vec![vec!["bash-language-server", "start"]],
        "json" => vec![vec!["vscode-json-language-server", "--stdio"]],
        "toml" => vec![vec!["taplo", "lsp", "stdio"]],
        "yaml" => vec![vec!["yaml-language-server", "--stdio"]],
        _ => vec![],
    };
    list.into_iter()
        .map(|v| v.into_iter().map(str::to_string).collect())
        .collect()
}
pub fn lang_id_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
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
    pub fn spawn(lang: &str, root: &str, cfg: &crate::config::LspServer) -> Option<Self> {
        let candidates = if cfg.cmd.is_empty() {
            candidates_for(lang)
        } else {
            vec![cfg.cmd.clone()]
        };
        for args in candidates {
            let bin = args.first()?;
            let mut command = Command::new(bin);
            command
                .args(&args[1..])
                .envs(&cfg.env)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            if let Some(path) = crate::files::from_uri(root) {
                command.current_dir(path);
            }
            let Ok(mut child) = command.spawn() else {
                continue;
            };
            let mut stdin = child.stdin.take()?;
            let stdout = child.stdout.take()?;
            let (tx, writer) = mpsc::sync_channel::<Value>(128);
            let (events, rx) = mpsc::channel();
            let errors = events.clone();
            std::thread::spawn(move || {
                while let Ok(v) = writer.recv() {
                    if let Err(e) = write_message(&mut stdin, &v) {
                        let _ = errors.send(Err(format!("LSP write failed: {e}")));
                        break;
                    }
                }
            });
            std::thread::spawn(move || {
                let mut r = BufReader::new(stdout);
                loop {
                    match read_message(&mut r) {
                        Ok(Some(v)) => {
                            if events.send(Ok(v)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = events.send(Err(format!("LSP read failed: {e}")));
                            break;
                        }
                    }
                }
            });
            let mut c = Self {
                child,
                tx,
                rx,
                next_id: 2,
                pending: HashMap::new(),
                ready: false,
                pending_docs: HashMap::new(),
                doc_versions: HashMap::new(),
                server_cmd: args.join(" "),
                root: root.into(),
                capabilities: Value::Null,
                settings: cfg.settings.clone(),
            };
            let mut caps = json!({"general":{"positionEncodings":["utf-16"]},"workspace":{"configuration":true,"applyEdit":true,"workspaceEdit":{"documentChanges":true},"workspaceFolders":true},"textDocument":{"synchronization":{"didSave":true},"hover":{"contentFormat":["plaintext"]},"completion":{"completionItem":{"snippetSupport":false}},"definition":{},"documentSymbol":{"hierarchicalDocumentSymbolSupport":true},"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["","quickfix","refactor","source"]}},"resolveSupport":{"properties":["edit"]}},"publishDiagnostics":{"relatedInformation":false}}});
            merge(&mut caps, &cfg.capabilities);
            c.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":std::process::id(),"rootUri":root,"workspaceFolders":[{"uri":root,"name":"workspace"}],"capabilities":caps,"initializationOptions":cfg.init_options}})).ok()?;
            return Some(c);
        }
        None
    }
    fn send(&mut self, v: Value) -> Result<(), String> {
        self.tx
            .try_send(v)
            .map_err(|e| format!("LSP outgoing queue unavailable: {e}"))
    }
    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    pub fn request(&mut self, method: &str, params: Value, id: u64) -> Result<(), String> {
        if !self.ready {
            return Err("Language server is initializing; retry shortly".into());
        }
        if self.pending.len() >= 256 {
            return Err("Too many pending LSP requests; restart the server".into());
        }
        let wire = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":wire,"method":method,"params":params}))?;
        self.pending.insert(wire, id);
        Ok(())
    }
    pub fn did_open(&mut self, uri: &str, lang: &str, text: &str) -> Result<(), String> {
        if !self.ready {
            self.pending_docs
                .insert(uri.into(), (lang.into(), text.into()));
            return Ok(());
        }
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":lang,"version":1,"text":text}}),
        )?;
        self.doc_versions.insert(uri.into(), 1);
        Ok(())
    }
    pub fn did_change(&mut self, uri: &str, text: &str) -> Result<(), String> {
        if !self.ready {
            if let Some((_, s)) = self.pending_docs.get_mut(uri) {
                *s = text.into();
            }
            return Ok(());
        }
        let version = self.doc_versions.get(uri).copied().unwrap_or(0) + 1;
        self.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":text}]}),
        )?;
        self.doc_versions.insert(uri.into(), version);
        Ok(())
    }
    pub fn version(&self, uri: &str) -> Option<i64> {
        self.doc_versions.get(uri).copied()
    }
    pub fn close(&mut self, uri: &str) {
        self.pending_docs.remove(uri);
        self.doc_versions.remove(uri);
        if self.ready {
            let _ = self.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
        }
    }
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    pub fn reply_edit(&mut self, id: Value, error: Option<String>) {
        let _=self.send(json!({"jsonrpc":"2.0","id":id,"result":{"applied":error.is_none(),"failureReason":error}}));
    }
    pub fn poll(&mut self) -> Vec<LspEvent> {
        let mut out = Vec::new();
        for _ in 0..128 {
            let Ok(v) = self.rx.try_recv() else { break };
            match v {
                Ok(v) => self.handle(v, &mut out),
                Err(e) => out.push(LspEvent::Error(e)),
            }
        }
        out
    }
    fn handle(&mut self, msg: Value, out: &mut Vec<LspEvent>) {
        if let Some(method) = msg["method"].as_str() {
            if let Some(id) = msg.get("id") {
                match method {
                    "workspace/configuration" => {
                        let values: Vec<Value> = msg["params"]["items"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|item| {
                                let mut v = &self.settings;
                                if let Some(section) = item["section"].as_str() {
                                    for key in section.split('.') {
                                        v = &v[key];
                                    }
                                }
                                v.clone()
                            })
                            .collect();
                        let _ = self.send(json!({"jsonrpc":"2.0","id":id,"result":values}));
                    }
                    "workspace/workspaceFolders" => {
                        let _=self.send(json!({"jsonrpc":"2.0","id":id,"result":[{"uri":self.root,"name":"workspace"}]}));
                    }
                    "workspace/applyEdit" => out.push(LspEvent::ApplyEdit {
                        id: id.clone(),
                        edit: msg["params"]["edit"].clone(),
                    }),
                    "window/workDoneProgress/create" => {
                        let _ = self.send(json!({"jsonrpc":"2.0","id":id,"result":null}));
                    }
                    _ => {
                        let _=self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not supported"}}));
                    }
                }
                return;
            }
            if method == "textDocument/publishDiagnostics" {
                if let Some(uri) = msg["params"]["uri"].as_str() {
                    let diags = msg["params"]["diagnostics"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(parse_diagnostic)
                        .collect();
                    out.push(LspEvent::Diagnostics {
                        uri: uri.into(),
                        diags,
                    });
                }
            }
            return;
        }
        let Some(id) = msg["id"].as_i64() else { return };
        if id == 1 {
            if let Some(e) = msg.get("error") {
                out.push(LspEvent::Error(format!("Initialization failed: {e}")));
                return;
            }
            self.capabilities = msg["result"]["capabilities"].clone();
            if self.capabilities["positionEncoding"]
                .as_str()
                .is_some_and(|s| s != "utf-16")
            {
                out.push(LspEvent::Error(
                    "Server chose unsupported position encoding".into(),
                ));
                return;
            }
            self.ready = true;
            let _ = self.notify("initialized", json!({}));
            let _ = self.notify(
                "workspace/didChangeConfiguration",
                json!({"settings":self.settings}),
            );
            for (uri, (lang, text)) in std::mem::take(&mut self.pending_docs) {
                if let Err(e) = self.did_open(&uri, &lang, &text) {
                    out.push(LspEvent::Error(e));
                }
            }
            return;
        }
        if let Some(request_id) = self.pending.remove(&id) {
            out.push(LspEvent::Response {
                request_id,
                result: msg["result"].clone(),
                error: msg.get("error").map(|e| {
                    e["message"]
                        .as_str()
                        .unwrap_or("LSP request failed")
                        .to_string()
                }),
            });
        }
    }
}
impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn merge(dst: &mut Value, src: &Value) {
    if let (Some(a), Some(b)) = (dst.as_object_mut(), src.as_object()) {
        for (k, v) in b {
            if let Some(old) = a.get_mut(k) {
                merge(old, v)
            } else {
                a.insert(k.clone(), v.clone());
            }
        }
    } else if !src.is_null() {
        *dst = src.clone();
    }
}
pub fn extract_completion_items(result: &Value) -> Vec<CompletionResultItem> {
    result
        .as_array()
        .or_else(|| result["items"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|it| {
            if it["insertTextFormat"].as_u64() == Some(2) {
                return None;
            }
            let label = it["label"].as_str()?.to_string();
            let edit = it.get("textEdit").cloned();
            let insert_text = edit
                .as_ref()
                .and_then(|v| v["newText"].as_str())
                .or_else(|| it["insertText"].as_str())
                .unwrap_or(&label)
                .to_string();
            Some(CompletionResultItem {
                label,
                insert_text,
                detail: it["detail"].as_str().map(str::to_string),
                edit,
                additional: it["additionalTextEdits"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .take(100)
        .collect()
}
fn parse_diagnostic(v: &Value) -> Option<Diagnostic> {
    let r = &v["range"];
    Some(Diagnostic {
        line: r["start"]["line"].as_u64()? as usize,
        col: r["start"]["character"].as_u64()? as usize,
        end_line: r["end"]["line"].as_u64()? as usize,
        end_col: r["end"]["character"].as_u64()? as usize,
        severity: match v["severity"].as_u64() {
            Some(1) => Severity::Error,
            Some(2) => Severity::Warning,
            Some(3) => Severity::Info,
            _ => Severity::Hint,
        },
        message: v["message"].as_str().unwrap_or("").into(),
        raw: v.clone(),
    })
}
