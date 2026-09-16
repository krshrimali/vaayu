use crate::{
    editor::Editor,
    lsp::{LspClient, LspEvent},
    results::{Entry, Results},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct RequestContext {
    pub kind: String,
    pub path: PathBuf,
    pub revision: u64,
    pub client: String,
    pub versions: HashMap<PathBuf, u64>,
}
pub fn utf16_col(s: &str, col: usize) -> usize {
    s.chars().take(col).map(char::len_utf16).sum()
}
pub fn utf16_to_col(s: &str, units: usize) -> usize {
    let mut n = 0;
    for (i, c) in s.chars().enumerate() {
        if n + c.len_utf16() > units {
            return i;
        }
        n += c.len_utf16();
    }
    s.chars().count()
}
fn root(path: &Path, markers: &[String]) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("/"));
    for dir in parent.ancestors() {
        if markers.iter().any(|m| dir.join(m).exists()) {
            return dir.into();
        }
    }
    parent.into()
}
fn language(path: &Path) -> Option<&'static str> {
    crate::lsp::lang_id_for_extension(&path.extension()?.to_str()?.to_lowercase())
}
impl Editor {
    pub fn clients_for_current(&self) -> Vec<String> {
        let Some(path) = &self.buf().path else {
            return vec![];
        };
        let mut keys: Vec<_> = self
            .lsp_opened_docs
            .iter()
            .filter(|(_, p)| p == path)
            .map(|(k, _)| k.clone())
            .filter(|k| self.lsp_clients.contains_key(k))
            .collect();
        keys.sort();
        keys
    }
    pub fn sync_lsp(&mut self) {
        let stamp = (self.buf().id, self.buf().edit_seq, self.buf().path.clone());
        if self.lsp_stamp.as_ref() == Some(&stamp) {
            return;
        }

        // Close documents whose buffers were deleted or renamed.
        let closed: Vec<_> = self
            .lsp_opened_docs
            .iter()
            .filter(|(_, p)| !self.buffers.iter().any(|b| b.path.as_ref() == Some(p)))
            .cloned()
            .collect();
        for (key, p) in closed {
            if let Some(c) = self.lsp_clients.get_mut(&key) {
                c.close(&crate::files::uri(&p));
            }
            self.lsp_opened_docs.remove(&(key.clone(), p.clone()));
            self.lsp_synced_seq.remove(&(key, p.clone()));
            self.diagnostics.remove(&p);
        }
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let Some(lang) = language(&path) else { return };
        let configured: Vec<_> = self
            .config
            .lsp
            .iter()
            .filter(|(name, cfg)| name.as_str() == lang || cfg.filetypes.iter().any(|f| f == lang))
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();
        let configs = if configured.is_empty() {
            vec![(lang.to_string(), crate::config::LspServer::default())]
        } else {
            configured
        };
        let seq = self.buf().edit_seq;
        let mut all_synced = true;
        for (name, cfg) in configs {
            if !cfg.enabled {
                continue;
            }
            let root = root(&path, &cfg.root_markers);
            let uri = crate::files::uri(&root);
            let key = format!("{name}@{}", root.display());
            if !self.lsp_clients.contains_key(&key) && !self.lsp_unavailable.contains(&key) {
                match LspClient::spawn(lang, &uri, &cfg) {
                    Some(c) => {
                        self.lsp_clients.insert(key.clone(), c);
                    }
                    None => {
                        self.lsp_unavailable.insert(key.clone());
                    }
                }
            }
            let doc = (key.clone(), path.clone());
            if !self.lsp_clients.contains_key(&key) || self.lsp_synced_seq.get(&doc) == Some(&seq) {
                continue;
            }
            let text = self.buffer_text();
            let opened = self.lsp_opened_docs.contains(&doc);
            let c = self.lsp_clients.get_mut(&key).unwrap();
            let result = if opened {
                c.did_change(&crate::files::uri(&path), &text)
            } else {
                c.did_open(&crate::files::uri(&path), lang, &text)
            };
            match result {
                Ok(()) => {
                    self.lsp_opened_docs.insert(doc.clone());
                    self.lsp_synced_seq.insert(doc, seq);
                }
                Err(e) => {
                    all_synced = false;
                    self.set_message(e);
                }
            }
        }
        if all_synced {
            self.lsp_stamp = Some(stamp);
        }
    }
    pub fn notify_saved(&mut self) {
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        self.sync_lsp();
        for key in self.clients_for_current() {
            if let Some(c) = self.lsp_clients.get_mut(&key) {
                let _ = c.notify(
                    "textDocument/didSave",
                    json!({"textDocument":{"uri":crate::files::uri(&path)}}),
                );
            }
        }
    }
    pub fn request_language(&mut self, kind: &str, argument: Option<&str>) {
        self.sync_lsp();
        let Some(path) = self.buf().path.clone() else {
            self.set_message("No language server for this buffer");
            return;
        };
        let (line, col) = self.cursor();
        let pos = json!({"line":line,"character":utf16_col(&self.buf().line_text(line),col)});
        let doc = json!({"uri":crate::files::uri(&path)});
        let (method, params) = match kind {
            "hover" => (
                "textDocument/hover",
                json!({"textDocument":doc,"position":pos}),
            ),
            "definition" => (
                "textDocument/definition",
                json!({"textDocument":doc,"position":pos}),
            ),
            "references" => (
                "textDocument/references",
                json!({"textDocument":doc,"position":pos,"context":{"includeDeclaration":true}}),
            ),
            "outline" => ("textDocument/documentSymbol", json!({"textDocument":doc})),
            "format" => (
                "textDocument/formatting",
                json!({"textDocument":doc,"options":{"tabSize":self.buf().tabstop,"insertSpaces":self.buf().expandtab}}),
            ),
            "rename" => {
                let Some(name) = argument.filter(|s| !s.is_empty()) else {
                    self.set_message("Usage: :rename new_name");
                    return;
                };
                (
                    "textDocument/rename",
                    json!({"textDocument":doc,"position":pos,"newName":name}),
                )
            }
            "actions" => {
                let diagnostics: Vec<_> = self
                    .diagnostics
                    .get(&path)
                    .into_iter()
                    .flatten()
                    .filter(|d| {
                        (d.line, d.col) <= (line, utf16_col(&self.buf().line_text(line), col))
                            && (d.end_line, d.end_col)
                                >= (line, utf16_col(&self.buf().line_text(line), col))
                    })
                    .map(|d| d.raw.clone())
                    .collect();
                (
                    "textDocument/codeAction",
                    json!({"textDocument":doc,"range":{"start":pos,"end":pos},"context":{"diagnostics":diagnostics}}),
                )
            }
            "signature" => (
                "textDocument/signatureHelp",
                json!({"textDocument":doc,"position":pos}),
            ),
            _ => return,
        };
        let keys = self.clients_for_current();
        if keys.is_empty() {
            self.set_message("No language server available; configure [lsp] or install a server");
            return;
        }
        // First capable server handles an edit to avoid conflicting multi-server changes.
        let capability = match kind {
            "format" => "documentFormattingProvider",
            "rename" => "renameProvider",
            "hover" => "hoverProvider",
            "definition" => "definitionProvider",
            "outline" => "documentSymbolProvider",
            "references" => "referencesProvider",
            "actions" => "codeActionProvider",
            "signature" => "signatureHelpProvider",
            _ => "",
        };
        let key = keys
            .iter()
            .find(|key| {
                self.lsp_clients.get(*key).is_some_and(|c| {
                    !c.capabilities[capability].is_null() && c.capabilities[capability] != false
                })
            })
            .cloned()
            .unwrap_or_else(|| keys[0].clone());
        self.send_language(&key, kind, method, params, None);
    }
    fn send_language(
        &mut self,
        key: &str,
        kind: &str,
        method: &str,
        params: Value,
        id: Option<u64>,
    ) {
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let revision = self.buf().edit_seq;
        let id = id.unwrap_or_else(|| {
            self.next_request_id += 1;
            self.next_request_id
        });
        let versions = self
            .buffers
            .iter()
            .filter_map(|b| b.path.clone().map(|p| (p, b.edit_seq)))
            .collect();
        let Some(client) = self.lsp_clients.get_mut(key) else {
            return;
        };
        match client.request(method, params, id) {
            Ok(()) => {
                self.pending_language.insert(
                    id,
                    RequestContext {
                        kind: kind.into(),
                        path,
                        revision,
                        client: key.into(),
                        versions,
                    },
                );
            }
            Err(e) => self.set_message(e),
        }
    }
    pub fn resolve_completion(&mut self, mut item: Value) -> bool {
        let Some(key) = item["_vaayu_client"].as_str().map(str::to_string) else {
            return false;
        };
        item.as_object_mut().unwrap().remove("_vaayu_client");
        if !self
            .lsp_clients
            .get(&key)
            .is_some_and(|c| c.capabilities["completionProvider"]["resolveProvider"] == true)
        {
            return false;
        }
        let Some(comp) = &self.completion else {
            return false;
        };
        let kind = format!("completion_resolve:{}:{}", comp.request_id, comp.selected);
        self.send_language(&key, &kind, "completionItem/resolve", item, None);
        true
    }
    pub fn cancel_language_requests(&mut self) {
        for (id, ctx) in std::mem::take(&mut self.pending_language) {
            if let Some(c) = self.lsp_clients.get_mut(&ctx.client) {
                c.cancel(id);
            }
        }
        self.set_message("Language requests cancelled");
    }
    pub fn request_hover(&mut self) {
        self.request_language("hover", None);
    }
    pub fn request_definition(&mut self) {
        self.request_language("definition", None);
    }
    pub(crate) fn request_lsp_completion(&mut self, line: usize, col: usize, id: u64) {
        self.sync_lsp();
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let params = json!({"textDocument":{"uri":crate::files::uri(&path)},"position":{"line":line,"character":utf16_col(&self.buf().line_text(line),col)}});
        if let Some(key) = self.clients_for_current().first().cloned() {
            self.send_language(
                &key,
                "completion",
                "textDocument/completion",
                params,
                Some(id),
            );
        }
    }
    pub fn restart_lsp(&mut self) {
        self.lsp_stamp = None;
        self.lsp_clients.clear();
        self.lsp_unavailable.clear();
        self.lsp_opened_docs.clear();
        self.lsp_synced_seq.clear();
        self.pending_language.clear();
        self.diagnostics.clear();
        self.server_diagnostics.clear();
        self.sync_lsp();
        self.set_message("Language servers restarted");
    }
    pub fn poll_lsp_events(&mut self) -> bool {
        let stale: Vec<_> = self
            .pending_language
            .iter()
            .filter(|(_, ctx)| {
                self.buf().path.as_ref() != Some(&ctx.path) || self.buf().edit_seq != ctx.revision
            })
            .map(|(id, ctx)| (*id, ctx.client.clone()))
            .collect();
        for (id, client) in stale {
            self.pending_language.remove(&id);
            if let Some(c) = self.lsp_clients.get_mut(&client) {
                c.cancel(id);
            }
        }

        let keys: Vec<_> = self.lsp_clients.keys().cloned().collect();
        let mut changed = false;
        for key in keys {
            let c = self.lsp_clients.get_mut(&key).unwrap();
            let alive = c.is_alive();
            let events = c.poll();
            for ev in events {
                changed = true;
                match ev {
                    LspEvent::Diagnostics { uri, diags } => {
                        if let Some(p) = crate::files::from_uri(&uri) {
                            self.server_diagnostics
                                .insert((key.clone(), p.clone()), diags);
                            let merged = self
                                .server_diagnostics
                                .iter()
                                .filter(|((_, path), _)| path == &p)
                                .flat_map(|(_, ds)| ds.clone())
                                .collect();
                            self.diagnostics.insert(p, merged);
                        }
                    }
                    LspEvent::Error(e) => self.set_message(e),
                    LspEvent::ApplyEdit { id, edit } => {
                        let ctx = RequestContext {
                            kind: "server edit".into(),
                            path: self.buf().path.clone().unwrap_or_default(),
                            revision: self.buf().edit_seq,
                            client: key.clone(),
                            versions: self
                                .buffers
                                .iter()
                                .filter_map(|b| b.path.clone().map(|p| (p, b.edit_seq)))
                                .collect(),
                        };
                        let result = self.apply_workspace_edit(&edit, Some(&ctx));
                        if let Some(c) = self.lsp_clients.get_mut(&key) {
                            c.reply_edit(id, result.err().map(|e| e.to_string()));
                        }
                    }
                    LspEvent::Response {
                        request_id,
                        result,
                        error,
                    } => {
                        let Some(ctx) = self.pending_language.remove(&request_id) else {
                            continue;
                        };
                        if let Some(error) = error {
                            self.set_message(error);
                            continue;
                        }
                        if self.buf().path.as_ref() != Some(&ctx.path)
                            || self.buf().edit_seq != ctx.revision
                        {
                            self.set_message("Ignored stale language-server response");
                            continue;
                        }
                        self.language_result(request_id, result, ctx);
                    }
                }
            }
            if !alive {
                self.lsp_stamp = None;
                self.server_diagnostics.retain(|(k, _), _| k != &key);
                self.diagnostics.clear();
                for ((_, p), ds) in &self.server_diagnostics {
                    self.diagnostics
                        .entry(p.clone())
                        .or_default()
                        .extend(ds.clone());
                }
                self.lsp_clients.remove(&key);
                self.pending_language.retain(|_, ctx| ctx.client != key);
                self.lsp_unavailable.insert(key.clone());
                self.set_message(format!(
                    "Language server exited: {key}; :lsprestart to retry"
                ));
                changed = true;
            }
        }
        changed
    }
    fn language_result(&mut self, id: u64, v: Value, ctx: RequestContext) {
        match ctx.kind.as_str() {
            kind if kind.starts_with("completion_resolve:") => {
                let expected = self
                    .completion
                    .as_ref()
                    .map(|c| format!("completion_resolve:{}:{}", c.request_id, c.selected));
                if expected.as_deref() != Some(kind) || self.mode != crate::mode::Mode::Insert {
                    return;
                }
                if let Some(comp) = &mut self.completion {
                    if let Some(old) = comp.items.get_mut(comp.selected) {
                        old.raw = None;
                        if let Some(item) =
                            crate::lsp::client::extract_completion_items(&json!([v]))
                                .into_iter()
                                .next()
                        {
                            old.insert_text = item.insert_text;
                            old.edit = item.edit;
                            old.additional = item.additional;
                            old.snippet = item.snippet;
                            old.raw = None;
                        }
                    }
                }
                crate::insert::accept_completion(self);
            }
            "completion" => {
                if let Some(c) = &mut self.completion {
                    if c.request_id == id {
                        c.items
                            .retain(|i| i.source != crate::completion::Source::Lsp);
                        let mut items: Vec<_> = crate::lsp::client::extract_completion_items(&v)
                            .into_iter()
                            .map(|i| crate::completion::Item {
                                label: i.label,
                                insert_text: i.insert_text,
                                detail: i.detail,
                                source: crate::completion::Source::Lsp,
                                edit: i.edit,
                                additional: i.additional,
                                snippet: i.snippet,
                                raw: i.raw.map(|mut v| {
                                    v["_vaayu_client"] = json!(ctx.client);
                                    v
                                }),
                            })
                            .collect();
                        items.append(&mut c.items);
                        c.items = items;
                        c.selected = c.selected.min(c.items.len().saturating_sub(1));
                    }
                }
            }
            "hover" | "signature" => {
                let text = if ctx.kind == "signature" {
                    v["signatures"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v["label"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    hover_text(&v["contents"])
                };
                self.hover_text = Some(text.clone());
                let r = Results::new(
                    if ctx.kind == "hover" {
                        "Hover"
                    } else {
                        "Signature help"
                    },
                    text.lines().map(Entry::text).collect(),
                );
                self.show_results(r);
            }
            "outline" if self.windows.iter().any(|w| w.outline) => {
                let mut nodes = Vec::new();
                crate::outline::flatten(&v, 0, &mut nodes);
                for n in &mut nodes {
                    let text = self
                        .buffers
                        .iter()
                        .find(|b| b.path.as_ref() == Some(&ctx.path))
                        .map(|b| b.line_text(n.line))
                        .or_else(|| {
                            std::fs::read_to_string(&ctx.path)
                                .ok()
                                .and_then(|t| t.lines().nth(n.line).map(str::to_string))
                        })
                        .unwrap_or_default();
                    n.col = utf16_to_col(&text, n.col);
                }
                if let Some(o) = &mut self.outline {
                    o.set_nodes(nodes);
                    o.buffer_path = Some(ctx.path.clone());
                }
            }
            "definition" | "references" | "outline" => {
                let mut entries = Vec::new();
                locations(&v, &ctx.path, &mut entries, 0);
                for e in &mut entries {
                    if let Some(path) = &e.path {
                        let text = self
                            .buffers
                            .iter()
                            .find(|b| b.path.as_ref() == Some(path))
                            .map(|b| b.line_text(e.line))
                            .or_else(|| {
                                std::fs::read_to_string(path)
                                    .ok()
                                    .and_then(|t| t.lines().nth(e.line).map(str::to_string))
                            })
                            .unwrap_or_default();
                        e.col = utf16_to_col(&text, e.col);
                        if e.text.is_empty() {
                            e.text = text;
                        }
                    }
                }
                self.results = Some(Results::new(&ctx.kind, entries));
                if ctx.kind == "definition" && self.results.as_ref().unwrap().entries.len() == 1 {
                    self.open_result();
                } else {
                    self.mode = crate::mode::Mode::Results;
                }
            }
            "format" => {
                if v.is_null() {
                    self.set_message("No formatting edits");
                    return;
                }
                let edit = json!({"changes":{crate::files::uri(&ctx.path):v}});
                let result = self.apply_workspace_edit(&edit, Some(&ctx));
                self.set_message(match result {
                    Ok(()) => "Formatted — :w to save".into(),
                    Err(e) => e.to_string(),
                });
            }
            "rename" => {
                let result = self.apply_workspace_edit(&v, Some(&ctx));
                self.set_message(match result {
                    Ok(()) => "Renamed across buffers — :wqa to save all".into(),
                    Err(e) => e.to_string(),
                });
            }
            "actions" => {
                let entries = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|a| a.get("disabled").is_none())
                    .map(|a| {
                        let mut e = Entry::text(a["title"].as_str().unwrap_or("Code action"));
                        let mut action = a.clone();
                        action["_vaayu_client"] = ctx.client.clone().into();
                        action["_vaayu_path"] = ctx.path.to_string_lossy().to_string().into();
                        action["_vaayu_revision"] = ctx.revision.into();
                        action["_vaayu_versions"] =
                            serde_json::to_value(&ctx.versions).unwrap_or(Value::Null);
                        e.action = Some(action);
                        e
                    })
                    .collect();
                self.show_results(Results::new("Code actions", entries));
            }
            "resolve" => {
                let mut action = v;
                action["_vaayu_resolved"] = true.into();
                action["_vaayu_client"] = ctx.client.into();
                action["_vaayu_path"] = ctx.path.to_string_lossy().to_string().into();
                action["_vaayu_revision"] = ctx.revision.into();
                action["_vaayu_versions"] =
                    serde_json::to_value(ctx.versions).unwrap_or(Value::Null);
                self.apply_code_action(action);
            }
            "execute" => {
                self.set_message("Code action completed");
            }
            _ => {}
        }
    }
    pub fn apply_code_action(&mut self, mut action: Value) {
        let key = action["_vaayu_client"]
            .as_str()
            .map(str::to_string)
            .or_else(|| self.clients_for_current().first().cloned());
        let Some(key) = key else { return };
        if let Some(rev) = action["_vaayu_revision"].as_u64() {
            if rev != self.buf().edit_seq
                || action["_vaayu_path"].as_str()
                    != self.buf().path.as_ref().and_then(|p| p.to_str())
            {
                self.set_message("Code action is stale; request actions again");
                return;
            }
        }
        let ctx = RequestContext {
            kind: "action".into(),
            path: self.buf().path.clone().unwrap_or_default(),
            revision: self.buf().edit_seq,
            client: key.clone(),
            versions: serde_json::from_value(action["_vaayu_versions"].clone()).unwrap_or_default(),
        };
        if let Some(obj) = action.as_object_mut() {
            obj.remove("_vaayu_client");
            obj.remove("_vaayu_path");
            obj.remove("_vaayu_revision");
            obj.remove("_vaayu_versions");
        }
        if let Some(edit) = action.get("edit") {
            if let Err(e) = self.apply_workspace_edit(edit, Some(&ctx)) {
                self.set_message(e.to_string());
                return;
            }
        }
        if let Some(command) = action.get("command") {
            let cmd = if command.is_string() {
                json!({"command":command,"arguments":action["arguments"]})
            } else {
                command.clone()
            };
            self.send_language(&key, "execute", "workspace/executeCommand", cmd, None);
        } else if action.get("edit").is_none()
            && action.get("data").is_some()
            && action["_vaayu_resolved"] != true
        {
            self.send_language(&key, "resolve", "codeAction/resolve", action, None);
            return;
        }
        self.enter_normal();
        self.set_message("Code action applied — save edited buffers with :wqa");
    }
    pub fn apply_workspace_edit(
        &mut self,
        edit: &Value,
        context: Option<&RequestContext>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(edit.is_object(), "No workspace edits returned");
        if edit["documentChanges"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| c.get("kind").is_some()))
        {
            return self.apply_resource_edit(edit, context);
        }
        let mut docs: Vec<(PathBuf, Vec<Value>, Option<i64>)> = Vec::new();
        if let Some(changes) = edit["changes"].as_object() {
            for (uri, edits) in changes {
                docs.push((
                    crate::files::from_uri(uri)
                        .ok_or_else(|| anyhow::anyhow!("unsupported URI"))?,
                    edits
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("invalid edits"))?
                        .clone(),
                    None,
                ));
            }
        }
        if let Some(changes) = edit["documentChanges"].as_array() {
            for change in changes {
                anyhow::ensure!(
                    change.get("kind").is_none(),
                    "File create/rename/delete operations are not supported by this edit"
                );
                let uri = change["textDocument"]["uri"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing document URI"))?;
                docs.push((
                    crate::files::from_uri(uri)
                        .ok_or_else(|| anyhow::anyhow!("unsupported URI"))?,
                    change["edits"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("invalid edits"))?
                        .clone(),
                    change["textDocument"]["version"].as_i64(),
                ));
            }
        }
        let mut plan = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (path, edits, version) in docs {
            anyhow::ensure!(seen.insert(path.clone()), "duplicate document edits");
            let existing = self
                .buffers
                .iter()
                .position(|b| b.path.as_ref() == Some(&path));
            if let (Some(ctx), Some(idx)) = (context, existing) {
                if let Some(rev) = ctx.versions.get(&path) {
                    anyhow::ensure!(
                        *rev == self.buffers[idx].edit_seq,
                        "document changed while awaiting language server: {}",
                        path.display()
                    );
                }
            }
            if let Some(version) = version {
                let actual = context
                    .and_then(|ctx| self.lsp_clients.get(&ctx.client))
                    .and_then(|c| c.version(&crate::files::uri(&path)));
                anyhow::ensure!(actual == Some(version), "document version mismatch");
            }
            let new_buffer = if existing.is_none() {
                let mut b = crate::buffer::Buffer::from_path(path.clone())?;
                b.apply_indent(&self.config);
                Some(b)
            } else {
                None
            };
            let b = if let Some(idx) = existing {
                &self.buffers[idx].rope
            } else {
                &new_buffer.as_ref().unwrap().rope
            };
            let changes = validate_edits(b, &edits)?;
            plan.push((new_buffer, existing, changes));
        }
        let mut entries = Vec::new();
        for (new_buffer, existing, changes) in plan {
            let idx = match existing {
                Some(i) => i,
                None => {
                    self.buffers.push(new_buffer.unwrap());
                    self.buffers.len() - 1
                }
            };
            let b = &mut self.buffers[idx];
            if let Some((start, _, _)) = changes.last() {
                let (l, c) = b.pos_from_char_idx(*start);
                if let Some(path) = &b.path {
                    entries.push(Entry::location(path.clone(), l, c, "Language-server edit"));
                }
            }
            b.begin_edit();
            for (start, end, text) in changes {
                b.delete_char_range(start, end);
                b.insert_str_at(start, &text);
            }
            b.commit_edit();
            b.cursor_line = b.cursor_line.min(b.line_count().saturating_sub(1));
            b.cursor_col = b.clamp_col_normal(b.cursor_line, b.cursor_col);
        }
        self.results = Some(Results::new("Edited locations", entries));
        self.invalidate_index_caches();
        Ok(())
    }
}
pub type TextEdits = Vec<(usize, usize, String)>;
pub fn validate_edits(rope: &ropey::Rope, edits: &[Value]) -> anyhow::Result<TextEdits> {
    fn offset(rope: &ropey::Rope, p: &Value) -> anyhow::Result<usize> {
        let line = p["line"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing edit line"))? as usize;
        let units = p["character"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing edit column"))? as usize;
        anyhow::ensure!(line < rope.len_lines(), "edit line out of range");
        let s = rope.line(line).to_string();
        let s = s.trim_end_matches(['\r', '\n']);
        let col = utf16_to_col(s, units);
        anyhow::ensure!(
            utf16_col(s, col) == units,
            "edit column is invalid UTF-16 boundary"
        );
        Ok(rope.line_to_char(line) + col)
    }
    let mut out = Vec::new();
    for e in edits {
        let r = e
            .get("range")
            .or_else(|| e.get("replace"))
            .ok_or_else(|| anyhow::anyhow!("missing edit range"))?;
        let a = offset(rope, &r["start"])?;
        let b = offset(rope, &r["end"])?;
        anyhow::ensure!(a <= b, "reversed edit range");
        let text = e["newText"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing edit text"))?;
        out.push((a, b, text.into()));
    }
    out.sort_by_key(|item| std::cmp::Reverse((item.0, item.1)));
    for pair in out.windows(2) {
        anyhow::ensure!(
            pair[1].1 <= pair[0].0 && !(pair[1].0 == pair[0].0 && pair[1].1 == pair[0].1),
            "overlapping edits"
        );
    }
    Ok(out)
}
fn hover_text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.into();
    }
    if let Some(s) = v["value"].as_str() {
        return s.into();
    }
    v.as_array()
        .into_iter()
        .flatten()
        .map(hover_text)
        .collect::<Vec<_>>()
        .join("\n")
}
fn locations(v: &Value, default: &Path, out: &mut Vec<Entry>, depth: usize) {
    if depth > 64 || out.len() >= 5000 {
        return;
    }
    if let Some(a) = v.as_array() {
        for e in a {
            locations(e, default, out, depth + 1);
        }
        return;
    }
    let loc = v.get("location").unwrap_or(v);
    let uri = loc["uri"].as_str().or_else(|| loc["targetUri"].as_str());
    let path = uri
        .and_then(crate::files::from_uri)
        .unwrap_or_else(|| default.into());
    let range = loc
        .get("selectionRange")
        .or_else(|| loc.get("targetSelectionRange"))
        .or_else(|| loc.get("range"));
    if let Some(r) = range {
        let line = r["start"]["line"].as_u64().unwrap_or(0) as usize;
        let col = r["start"]["character"].as_u64().unwrap_or(0) as usize;
        out.push(Entry::location(
            path,
            line,
            col,
            v["name"].as_str().unwrap_or(""),
        ));
    }
    if let Some(children) = v.get("children") {
        locations(children, default, out, depth + 1);
    }
}
