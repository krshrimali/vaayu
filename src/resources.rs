//! Stage ordered LSP resource operations and buffer edits before touching disk.
use crate::{editor::Editor, language::RequestContext};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};
impl Editor {
    pub fn apply_resource_edit(
        &mut self,
        edit: &Value,
        context: Option<&RequestContext>,
    ) -> anyhow::Result<()> {
        let original = self.buffers.clone();
        let old_cur = self.cur;
        let old_results = self.results.clone();
        let mut before: BTreeMap<PathBuf, Option<Vec<u8>>> = BTreeMap::new();
        let mut after = BTreeMap::new();
        let mut permissions = BTreeMap::new();
        let mut staged_permissions = BTreeMap::new();
        let mut moved = Vec::new();
        let result = (|| -> anyhow::Result<()> {
            if let Some(ctx) = context {
                for b in &self.buffers {
                    if let Some(p) = &b.path {
                        if let Some(seq) = ctx.versions.get(p) {
                            anyhow::ensure!(*seq == b.edit_seq, "Workspace changed since request");
                        }
                    }
                }
            }
            anyhow::ensure!(
                edit.get("changes").is_none(),
                "Use either changes or documentChanges"
            );
            for change in edit["documentChanges"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing documentChanges"))?
            {
                let Some(kind) = change["kind"].as_str() else {
                    let mut change = change.clone();
                    if let Some(version) = change["textDocument"]["version"].as_i64() {
                        let uri = change["textDocument"]["uri"].as_str().unwrap_or("");
                        let actual = context
                            .and_then(|ctx| self.lsp_clients.get(&ctx.client))
                            .and_then(|c| c.version(uri));
                        anyhow::ensure!(actual == Some(version), "document version mismatch");
                    }
                    change["textDocument"]["version"] = Value::Null;
                    self.apply_workspace_edit(&json!({"documentChanges":[change]}), None)?;
                    continue;
                };
                let fields: &[&str] = if kind == "rename" {
                    &["oldUri", "newUri"]
                } else {
                    &["uri"]
                };
                let mut paths = Vec::new();
                for field in fields {
                    let uri = change[*field]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing resource URI"))?;
                    let raw = url::Url::parse(uri)?
                        .to_file_path()
                        .map_err(|_| anyhow::anyhow!("Unsupported resource URI"))?;
                    anyhow::ensure!(
                        !std::fs::symlink_metadata(&raw).is_ok_and(|m| m.file_type().is_symlink()),
                        "Symlink resource operations are unsupported"
                    );
                    let p = crate::files::from_uri(uri)
                        .ok_or_else(|| anyhow::anyhow!("Unsupported resource URI"))?;
                    anyhow::ensure!(
                        p.starts_with(&self.project_root)
                            && !p.starts_with(self.project_root.join(".vaayu")),
                        "Resource outside editable project files"
                    );
                    if !before.contains_key(&p) {
                        let bytes = match std::fs::metadata(&p) {
                            Ok(m) => {
                                anyhow::ensure!(
                                    m.is_file(),
                                    "Directory resource operations are unsupported"
                                );
                                permissions.insert(p.clone(), m.permissions());
                                staged_permissions.insert(p.clone(), m.permissions());
                                Some(std::fs::read(&p)?)
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                            Err(e) => return Err(e.into()),
                        };
                        before.insert(p.clone(), bytes.clone());
                        after.insert(p.clone(), bytes);
                    }
                    paths.push(p);
                }
                let p = &paths[0];
                let opts = &change["options"];
                match kind {
                    "create" => {
                        if after[p].is_some() && opts["overwrite"] != true {
                            if opts["ignoreIfExists"] == true {
                                continue;
                            }
                            anyhow::bail!("Create target exists");
                        }
                        anyhow::ensure!(
                            !self
                                .buffers
                                .iter()
                                .any(|b| b.path.as_ref() == Some(p) && b.is_modified()),
                            "Create would overwrite a dirty buffer"
                        );
                        after.insert(p.clone(), Some(Vec::new()));
                        self.buffers.retain(|b| b.path.as_ref() != Some(p));
                        let mut b = crate::buffer::Buffer::empty();
                        b.path = Some(p.clone());
                        b.rope = ropey::Rope::new();
                        b.mark_saved();
                        self.buffers.push(b);
                    }
                    "rename" => {
                        let dest = &paths[1];
                        if p == dest {
                            continue;
                        }
                        anyhow::ensure!(after[p].is_some(), "Rename source missing");
                        if after[dest].is_some() && opts["overwrite"] != true {
                            if opts["ignoreIfExists"] == true {
                                continue;
                            }
                            anyhow::bail!("Rename target exists");
                        }
                        anyhow::ensure!(
                            !self
                                .buffers
                                .iter()
                                .any(|b| b.path.as_ref() == Some(dest) && b.is_modified()),
                            "Rename target has unsaved changes"
                        );
                        if !self.buffers.iter().any(|b| b.path.as_ref() == Some(p)) {
                            let mut b = crate::buffer::Buffer::from_path(p.clone())?;
                            b.rope = ropey::Rope::from_str(std::str::from_utf8(
                                after[p].as_ref().unwrap(),
                            )?);
                            b.apply_indent(&self.config);
                            self.buffers.push(b);
                        }
                        moved.push((p.clone(), dest.clone()));
                        if let Some(mode) = staged_permissions.get(p).cloned() {
                            staged_permissions.insert(dest.clone(), mode);
                        }
                        let bytes = after[p].clone();
                        after.insert(dest.clone(), bytes);
                        after.insert(p.clone(), None);
                        self.buffers.retain(|b| b.path.as_ref() != Some(dest));
                        for b in &mut self.buffers {
                            if b.path.as_ref() == Some(p) {
                                b.path = Some(dest.clone());
                            }
                        }
                    }
                    "delete" => {
                        if after[p].is_none() {
                            if opts["ignoreIfNotExists"] == true {
                                continue;
                            }
                            anyhow::bail!("Delete target missing");
                        }
                        anyhow::ensure!(
                            !self
                                .buffers
                                .iter()
                                .any(|b| b.path.as_ref() == Some(p) && b.is_modified()),
                            "Delete target has unsaved changes"
                        );
                        after.insert(p.clone(), None);
                        self.buffers.retain(|b| b.path.as_ref() != Some(p));
                    }
                    _ => anyhow::bail!("Unsupported resource operation: {kind}"),
                }
                if self.buffers.is_empty() {
                    self.buffers.push(crate::buffer::Buffer::empty());
                }
                self.cur = self.cur.min(self.buffers.len() - 1);
            }
            // Recheck disk snapshots immediately before commit.
            for (p, bytes) in &before {
                let current = match std::fs::read(p) {
                    Ok(v) => Some(v),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.into()),
                };
                anyhow::ensure!(&current == bytes, "Resource changed on disk");
            }
            let mut applied: Vec<PathBuf> = Vec::new();
            for (p, bytes) in &after {
                if before.get(p) == Some(bytes) {
                    continue;
                }
                let write = match bytes {
                    Some(bytes) => {
                        let mode = staged_permissions.get(p).cloned();
                        crate::files::atomic_write_mode(p, bytes, false, mode)
                    }
                    None => {
                        if p.exists() {
                            std::fs::remove_file(p).map_err(Into::into)
                        } else {
                            Ok(())
                        }
                    }
                };
                if let Err(e) = write {
                    let mut rollback_errors = Vec::new();
                    for p in applied.iter().rev() {
                        let r = match &before[p] {
                            Some(bytes) => crate::files::atomic_write_mode(
                                p,
                                bytes,
                                false,
                                permissions.get(p).cloned(),
                            ),
                            None => std::fs::remove_file(p).map_err(Into::into),
                        };
                        if let Err(err) = r {
                            rollback_errors.push(err.to_string());
                        }
                    }
                    anyhow::bail!(
                        "Resource commit failed: {e}; rollback errors: {rollback_errors:?}"
                    );
                }
                applied.push(p.clone());
            }
            for b in &mut self.buffers {
                if let Some(p) = &b.path {
                    if let Some(Some(bytes)) = after.get(p) {
                        b.resource_baseline(String::from_utf8_lossy(bytes).into_owned());
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.buffers = original;
            self.cur = old_cur;
            self.results = old_results;
        } else {
            let active = original[old_cur].id;
            if let Some(i) = self.buffers.iter().position(|b| b.id == active) {
                self.cur = i;
            }
            let fallback = self.buf().id;
            for w in &mut self.windows {
                if !self.buffers.iter().any(|b| b.id == w.buffer) {
                    w.buffer = fallback;
                    w.cursor = (0, 0);
                    w.top = 0;
                    w.wrap_row = 0;
                    w.left = 0;
                }
            }
            for (old, new) in moved {
                for note in &mut self.notes.items {
                    if self.project_root.join(&note.file) == old {
                        if let Ok(p) = new.strip_prefix(&self.project_root) {
                            note.file = p.to_path_buf();
                            self.notes.dirty = true;
                        }
                    }
                }
            }
            self.lsp_stamp = None;
            self.invalidate_index_caches();
        }
        result
    }
}
