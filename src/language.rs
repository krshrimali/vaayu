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
    /// Recomputes the *visible* `self.diagnostics` entry for `path` from
    /// `self.server_diagnostics` (every server's own last-known set for
    /// it, merged) -- the only place that actually changes what's
    /// rendered. Diagnostics update immediately outside of Insert mode;
    /// see `flush_deferred_diagnostics` for the Insert-mode catch-up.
    fn refresh_visible_diagnostics(&mut self, path: &std::path::Path) {
        let merged = self
            .server_diagnostics
            .iter()
            .filter(|((_, p), _)| p == path)
            .flat_map(|(_, ds)| ds.clone())
            .collect();
        self.diagnostics.insert(path.to_path_buf(), merged);
    }
    /// `diagnostics_update_in_insert=false` (the default) keeps
    /// `self.diagnostics` frozen while typing so it can't flicker
    /// mid-keystroke, even though `self.server_diagnostics` (the raw,
    /// per-server data) keeps recording every update as it arrives.
    /// Called when leaving Insert mode to catch the visible set up to
    /// whatever actually arrived while it was frozen.
    pub fn flush_deferred_diagnostics(&mut self) {
        let paths: std::collections::HashSet<_> = self
            .server_diagnostics
            .keys()
            .map(|(_, p)| p.clone())
            .collect();
        for path in paths {
            self.refresh_visible_diagnostics(&path);
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
            "typeDefinition" => (
                "textDocument/typeDefinition",
                json!({"textDocument":doc,"position":pos}),
            ),
            "implementation" => (
                "textDocument/implementation",
                json!({"textDocument":doc,"position":pos}),
            ),
            "declaration" => (
                "textDocument/declaration",
                json!({"textDocument":doc,"position":pos}),
            ),
            "workspaceSymbols" => ("workspace/symbol", json!({"query": argument.unwrap_or("")})),
            "references" => (
                "textDocument/references",
                json!({"textDocument":doc,"position":pos,"context":{"includeDeclaration":true}}),
            ),
            "outline" => ("textDocument/documentSymbol", json!({"textDocument":doc})),
            "documentLinks" => ("textDocument/documentLink", json!({"textDocument":doc})),
            "codeLens" => ("textDocument/codeLens", json!({"textDocument":doc})),
            "inlayHints" => {
                // The spec requires a range; whole-document, like the
                // codeLens/documentLinks requests above, rather than just
                // the visible viewport -- simpler, and this is a manual
                // request (`,li`), not a scroll-triggered one, so it
                // doesn't need to be re-issued on every scroll.
                let last_line = self.buf().rope.len_lines().saturating_sub(1);
                let end_char = utf16_col(
                    &self.buf().line_text(last_line),
                    self.buf().line_len(last_line),
                );
                (
                    "textDocument/inlayHint",
                    json!({"textDocument":doc,"range":{"start":{"line":0,"character":0},"end":{"line":last_line,"character":end_char}}}),
                )
            }
            "documentHighlight" => (
                "textDocument/documentHighlight",
                json!({"textDocument":doc,"position":pos}),
            ),
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
            "organizeImports" => {
                // Whole-document range, like the codeLens/inlayHints
                // requests above -- organizing imports isn't a
                // cursor-position operation, unlike the plain "actions"
                // request right above it.
                let last_line = self.buf().rope.len_lines().saturating_sub(1);
                let end_char = utf16_col(
                    &self.buf().line_text(last_line),
                    self.buf().line_len(last_line),
                );
                (
                    "textDocument/codeAction",
                    json!({"textDocument":doc,"range":{"start":{"line":0,"character":0},"end":{"line":last_line,"character":end_char}},"context":{"diagnostics":[],"only":["source.organizeImports"]}}),
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
            "typeDefinition" => "typeDefinitionProvider",
            "implementation" => "implementationProvider",
            "declaration" => "declarationProvider",
            "workspaceSymbols" => "workspaceSymbolProvider",
            "outline" => "documentSymbolProvider",
            "documentLinks" => "documentLinkProvider",
            "codeLens" => "codeLensProvider",
            "inlayHints" => "inlayHintProvider",
            "documentHighlight" => "documentHighlightProvider",
            "references" => "referencesProvider",
            "actions" => "codeActionProvider",
            "organizeImports" => "codeActionProvider",
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
    pub fn request_type_definition(&mut self) {
        self.request_language("typeDefinition", None);
    }
    pub fn request_implementation(&mut self) {
        self.request_language("implementation", None);
    }
    pub fn request_declaration(&mut self) {
        self.request_language("declaration", None);
    }
    pub fn request_workspace_symbols(&mut self, query: &str) {
        self.request_language("workspaceSymbols", Some(query));
    }
    /// `textDocument/rangeFormatting` for `[start_line, end_line]`
    /// (inclusive, whole lines). A separate method from `request_language`
    /// rather than a new `kind` there, since range formatting needs a
    /// range instead of the single cursor position every other kind uses
    /// -- but it reuses the "format" response kind/handling as-is, since
    /// a range-formatting reply is the same TextEdit[] shape a
    /// whole-buffer one is.
    pub fn request_range_format(&mut self, start_line: usize, end_line: usize) {
        self.sync_lsp();
        let Some(path) = self.buf().path.clone() else {
            self.set_message("No language server for this buffer");
            return;
        };
        let end_text = self.buf().line_text(end_line);
        let end_col = utf16_col(&end_text, end_text.chars().count());
        let params = json!({
            "textDocument": {"uri": crate::files::uri(&path)},
            "range": {
                "start": {"line": start_line, "character": 0},
                "end": {"line": end_line, "character": end_col},
            },
            "options": {"tabSize": self.buf().tabstop, "insertSpaces": self.buf().expandtab},
        });
        let keys = self.clients_for_current();
        if keys.is_empty() {
            self.set_message("No language server available; configure [lsp] or install a server");
            return;
        }
        let key = keys
            .iter()
            .find(|key| {
                self.lsp_clients.get(*key).is_some_and(|c| {
                    !c.capabilities["documentRangeFormattingProvider"].is_null()
                        && c.capabilities["documentRangeFormattingProvider"] != false
                })
            })
            .cloned()
            .unwrap_or_else(|| keys[0].clone());
        self.send_language(&key, "format", "textDocument/rangeFormatting", params, None);
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
        self.lsp_progress.clear();
        self.document_highlights.clear();
        self.sync_lsp();
        self.set_message("Language servers restarted");
    }
    /// Joins every active `$/progress` token into one line (title,
    /// percentage, message), or `None` once none are active. `None`
    /// deliberately leaves the message line alone rather than clearing
    /// it -- there's no way to tell whether it still shows the last
    /// progress update or something unrelated that happened since.
    /// Also used by `render.rs` for the persistent status-line progress
    /// indicator, which -- unlike the message line -- never gets
    /// silently clobbered by an unrelated action's own message.
    pub(crate) fn format_lsp_progress(&self) -> Option<String> {
        if self.lsp_progress.is_empty() {
            return None;
        }
        Some(
            self.lsp_progress
                .values()
                .map(|p| {
                    let mut s = p.title.clone().unwrap_or_else(|| "Working…".into());
                    if let Some(pct) = p.percentage {
                        s.push_str(&format!(" {pct}%"));
                    }
                    if let Some(msg) = &p.message {
                        s.push_str(&format!(" — {msg}"));
                    }
                    s
                })
                .collect::<Vec<_>>()
                .join(" · "),
        )
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
                            // Always record the raw per-server data --
                            // only whether it's reflected in the
                            // *visible* `self.diagnostics` map depends on
                            // `diagnostics_update_in_insert` below.
                            self.server_diagnostics
                                .insert((key.clone(), p.clone()), diags);
                            if self.config.diagnostics_update_in_insert
                                || !matches!(self.mode, crate::mode::Mode::Insert)
                            {
                                self.refresh_visible_diagnostics(&p);
                            }
                        }
                    }
                    LspEvent::Error(e) => self.set_message(e),
                    LspEvent::Progress {
                        token,
                        kind,
                        title,
                        message,
                        percentage,
                    } => {
                        let map_key = (key.clone(), token);
                        match kind.as_str() {
                            "begin" => {
                                self.lsp_progress.insert(
                                    map_key,
                                    crate::lsp::LspProgress {
                                        title,
                                        message,
                                        percentage,
                                    },
                                );
                            }
                            "report" => {
                                let entry = self.lsp_progress.entry(map_key).or_default();
                                if title.is_some() {
                                    entry.title = title;
                                }
                                if message.is_some() {
                                    entry.message = message;
                                }
                                if percentage.is_some() {
                                    entry.percentage = percentage;
                                }
                            }
                            "end" => {
                                self.lsp_progress.remove(&map_key);
                            }
                            _ => {}
                        }
                        if let Some(text) = self.format_lsp_progress() {
                            self.set_message(text);
                        }
                    }
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
                // If the user has left insert mode there is no pending
                // Tab/Enter to honor, so drop the stale resolve.
                if self.mode != crate::mode::Mode::Insert {
                    return;
                }
                let matched = expected.as_deref() == Some(kind);
                if let Some(comp) = &mut self.completion {
                    if let Some(old) = comp.items.get_mut(comp.selected) {
                        // Clear `raw` so the re-entrant accept_completion below
                        // won't fire another resolve request (which would loop
                        // or re-race).
                        old.raw = None;
                        // Only fold the resolved payload in when it still
                        // matches the current selection. On a race (a late
                        // completion batch reselected/refiltered the popup) we
                        // fall through and accept the original, un-resolved
                        // item instead of silently dropping the keystroke.
                        if matched {
                            if let Some(item) =
                                crate::lsp::client::extract_completion_items(&json!([v]))
                                    .into_iter()
                                    .next()
                            {
                                old.insert_text = item.insert_text;
                                old.edit = item.edit;
                                old.additional = item.additional;
                                old.snippet = item.snippet;
                            }
                        }
                    }
                }
                crate::insert::accept_completion(self);
            }
            "completion" => {
                let prefix = {
                    let (line, col) = self.cursor();
                    crate::completion::word_prefix(self.buf(), line, col).1
                };
                if let Some(c) = &mut self.completion {
                    if c.request_id == id {
                        c.items
                            .retain(|i| i.source != crate::completion::Source::Lsp);
                        let mut items: Vec<_> = crate::lsp::client::extract_completion_items(&v)
                            .into_iter()
                            .filter(|i| crate::completion::matches(&i.filter_text, &prefix))
                            // Cap the displayed count only after prefix
                            // filtering, so relevant matches past the first
                            // few hundred server items aren't dropped before
                            // they can be filtered.
                            .take(200)
                            .map(|i| crate::completion::Item {
                                label: i.label,
                                insert_text: i.insert_text,
                                detail: i.detail,
                                source: crate::completion::Source::Lsp,
                                edit: i.edit,
                                additional: i.additional,
                                snippet: i.snippet,
                                kind: i.kind,
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
            "documentLinks" => {
                // DocumentLink's `target` is optional -- a server can
                // defer it to `documentLink/resolve`, the same lazy
                // pattern completion items use for `documentation`/edits.
                // Skipping those (rather than adding another resolve
                // round trip) keeps this a single request/response pair,
                // like every other Phase 3.1 feature so far; a link
                // without an inline target just doesn't show up.
                let entries: Vec<Entry> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|link| {
                        let target = link["target"].as_str()?.to_string();
                        let line = link["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                        let raw_col =
                            link["range"]["start"]["character"].as_u64().unwrap_or(0) as usize;
                        let text = self
                            .buffers
                            .iter()
                            .find(|b| b.path.as_ref() == Some(&ctx.path))
                            .map(|b| b.line_text(line))
                            .unwrap_or_default();
                        let col = utf16_to_col(&text, raw_col);
                        let label = link["tooltip"].as_str().unwrap_or(&target).to_string();
                        let mut e = Entry::location(ctx.path.clone(), line, col, label);
                        e.action = Some(json!({"_vaayu_open_link": target}));
                        Some(e)
                    })
                    .collect();
                if entries.is_empty() {
                    self.set_message("No document links found");
                } else {
                    self.show_results(Results::new("Document links", entries));
                }
            }
            "codeLens" => {
                // A lens with no `command` defers it to `codeLens/resolve`
                // -- skipped, the same "don't add another resolve round
                // trip" choice already made for a target-less document
                // link.
                let lenses: Vec<(usize, String, Value)> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|lens| {
                        let command = lens.get("command")?.clone();
                        let title = command["title"].as_str().unwrap_or("Run").to_string();
                        let line =
                            lens["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                        let action = json!({
                            "command": command,
                            "_vaayu_client": ctx.client,
                            "_vaayu_path": ctx.path.to_string_lossy().to_string(),
                            "_vaayu_revision": ctx.revision,
                            "_vaayu_versions": serde_json::to_value(&ctx.versions).unwrap_or(Value::Null),
                        });
                        Some((line, title, action))
                    })
                    .collect();
                let count = lenses.len();
                let entries: Vec<Entry> = lenses
                    .iter()
                    .map(|(line, title, action)| {
                        let mut e = Entry::location(ctx.path.clone(), *line, 0, title.clone());
                        e.action = Some(action.clone());
                        e
                    })
                    .collect();
                if let Some(b) = self
                    .buffers
                    .iter()
                    .find(|b| b.path.as_ref() == Some(&ctx.path))
                {
                    self.code_lenses_buffer = Some(b.id);
                    self.code_lenses_edit_seq = b.edit_seq;
                }
                self.code_lenses = lenses;
                if count == 0 {
                    self.set_message("No code lenses");
                } else {
                    self.show_results(Results::new("Code lenses", entries));
                }
            }
            "inlayHints" => {
                let hints: Vec<(usize, usize, String)> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|hint| {
                        let line = hint["position"]["line"].as_u64().unwrap_or(0) as usize;
                        let raw_col = hint["position"]["character"].as_u64().unwrap_or(0) as usize;
                        let line_text = self
                            .buffers
                            .iter()
                            .find(|b| b.path.as_ref() == Some(&ctx.path))
                            .map(|b| b.line_text(line))
                            .unwrap_or_default();
                        let col = utf16_to_col(&line_text, raw_col);
                        // `label` is either a plain string or a list of
                        // InlayHintLabelPart objects (each with its own
                        // `value`, plus optional tooltip/location we don't
                        // need here) -- concatenated the same way either
                        // shape reads as one line of text.
                        let mut label = match &hint["label"] {
                            Value::String(s) => s.clone(),
                            Value::Array(parts) => parts
                                .iter()
                                .filter_map(|p| p["value"].as_str())
                                .collect::<Vec<_>>()
                                .join(""),
                            _ => String::new(),
                        };
                        if hint["paddingLeft"] == true {
                            label = format!(" {label}");
                        }
                        if hint["paddingRight"] == true {
                            label.push(' ');
                        }
                        (line, col, label)
                    })
                    .collect();
                let count = hints.len();
                if let Some(b) = self
                    .buffers
                    .iter()
                    .find(|b| b.path.as_ref() == Some(&ctx.path))
                {
                    self.inlay_hints_buffer = Some(b.id);
                    self.inlay_hints_edit_seq = b.edit_seq;
                }
                self.inlay_hints = hints;
                self.set_message(if count == 0 {
                    "No inlay hints".to_string()
                } else {
                    format!("{count} inlay hint(s) — Esc to clear")
                });
            }
            "documentHighlight" => {
                // Unlike definition/references/outline (locations() -- a
                // single jump point per entry, possibly cross-file), a
                // DocumentHighlight is a same-file *span* (start..end) to
                // paint as an in-buffer overlay, so it needs its own
                // parsing rather than reusing locations()'s single-point
                // Entry model.
                let mut ranges = Vec::new();
                for item in v.as_array().into_iter().flatten() {
                    let r = &item["range"];
                    let l1 = r["start"]["line"].as_u64().unwrap_or(0) as usize;
                    let l2 = r["end"]["line"].as_u64().unwrap_or(0) as usize;
                    let line1_text = self
                        .buffers
                        .iter()
                        .find(|b| b.path.as_ref() == Some(&ctx.path))
                        .map(|b| b.line_text(l1))
                        .unwrap_or_default();
                    let line2_text = if l2 == l1 {
                        line1_text.clone()
                    } else {
                        self.buffers
                            .iter()
                            .find(|b| b.path.as_ref() == Some(&ctx.path))
                            .map(|b| b.line_text(l2))
                            .unwrap_or_default()
                    };
                    let c1 = utf16_to_col(
                        &line1_text,
                        r["start"]["character"].as_u64().unwrap_or(0) as usize,
                    );
                    let c2 = utf16_to_col(
                        &line2_text,
                        r["end"]["character"].as_u64().unwrap_or(0) as usize,
                    );
                    ranges.push((l1, c1, l2, c2));
                }
                let count = ranges.len();
                self.document_highlights = ranges;
                if let Some(b) = self
                    .buffers
                    .iter()
                    .find(|b| b.path.as_ref() == Some(&ctx.path))
                {
                    self.document_highlights_buffer = Some(b.id);
                    self.document_highlights_edit_seq = b.edit_seq;
                }
                self.set_message(if count == 0 {
                    "No other occurrences found".to_string()
                } else {
                    format!("{count} occurrence(s) highlighted — Esc to clear")
                });
            }
            "definition" | "typeDefinition" | "implementation" | "declaration" | "references"
            | "outline" | "workspaceSymbols" => {
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
                let auto_jump = matches!(
                    ctx.kind.as_str(),
                    "definition" | "typeDefinition" | "implementation" | "declaration"
                );
                if auto_jump && self.results.as_ref().unwrap().entries.len() == 1 {
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
                // A disabled action is shown (with its reason) rather
                // than silently dropped -- `apply_code_action` is what
                // actually refuses to run one, so the list stays an
                // honest reflection of what the server offered instead
                // of quietly hiding some of it. `isPreferred` actions
                // sort first (a stable sort, so ties keep the server's
                // own relative order) and get a "* " marker, matching
                // how editors typically surface the server's own
                // preferred quick-fix first.
                let mut actions: Vec<Value> = v.as_array().cloned().unwrap_or_default();
                actions.sort_by_key(|a| !a["isPreferred"].as_bool().unwrap_or(false));
                let entries = actions
                    .into_iter()
                    .map(|a| {
                        let mut title = a["title"].as_str().unwrap_or("Code action").to_string();
                        if a["isPreferred"].as_bool().unwrap_or(false) {
                            title = format!("* {title}");
                        }
                        if let Some(reason) = a["disabled"]["reason"].as_str() {
                            title = format!("{title} (disabled: {reason})");
                        }
                        let mut e = Entry::text(title);
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
            "organizeImports" => {
                // Unlike the general `,la` list, organize-imports has
                // exactly one meaningful outcome per file -- servers
                // return at most one `source.organizeImports` action --
                // so this applies it directly instead of opening a
                // one-item picker, the same "just do it" choice already
                // made for `,lf`/:format and :rename.
                match v.as_array().and_then(|a| a.first()) {
                    Some(a) => {
                        let mut action = a.clone();
                        action["_vaayu_client"] = ctx.client.into();
                        action["_vaayu_path"] = ctx.path.to_string_lossy().to_string().into();
                        action["_vaayu_revision"] = ctx.revision.into();
                        action["_vaayu_versions"] =
                            serde_json::to_value(&ctx.versions).unwrap_or(Value::Null);
                        self.apply_code_action(action);
                    }
                    None => self.set_message("No organize-imports action available"),
                }
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
        // A `disabled` action is still shown (with its reason, see the
        // "actions" response arm below) rather than hidden -- so it
        // still needs a check here refusing to actually run it, the
        // same way a resolved-but-unusable action would be refused.
        if let Some(reason) = action["disabled"]["reason"].as_str() {
            self.set_message(format!("This action is disabled: {reason}"));
            return;
        }
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
        // A one-past-last-line end position (e.g. a whole-document formatter
        // edit whose end is {line: lineCount, character: 0}, common on files
        // with no trailing newline) maps to the end of the document rather
        // than being rejected. Anything further out of range is still rejected
        // so a stale/bogus edit can't silently land text at EOF.
        if line == rope.len_lines() {
            return Ok(rope.len_chars());
        }
        anyhow::ensure!(line < rope.len_lines(), "edit line out of range");
        let s = rope.line(line).to_string();
        let s = s.trim_end_matches(['\r', '\n']);
        // Per the LSP spec the client clamps a character past the line's
        // UTF-16 length (or one landing inside a surrogate pair) to the line
        // end rather than erroring -- `utf16_to_col` already clamps.
        let col = utf16_to_col(s, units);
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
