//! Private crash-recovery snapshots. Explicit save remains the only disk-file write.
use crate::{
    editor::Editor,
    results::{Entry, Results},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    thread::JoinHandle,
    time::{Duration, Instant},
};
#[derive(Serialize, Deserialize)]
struct Draft {
    #[serde(default)]
    note: Option<crate::notes::Note>,
    path: PathBuf,
    text: String,
    line: usize,
    col: usize,
}
pub struct Recovery {
    path: Option<PathBuf>,
    last: Instant,
    versions: Vec<(u64, u64)>,
    worker: Option<JoinHandle<anyhow::Result<()>>>,
}
impl Default for Recovery {
    fn default() -> Self {
        Self {
            path: None,
            last: Instant::now(),
            versions: Vec::new(),
            worker: None,
        }
    }
}
impl Recovery {
    pub fn cleanup(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl Editor {
    pub fn checkpoint_recovery(&mut self) {
        if self
            .recovery
            .worker
            .as_ref()
            .is_some_and(|w| w.is_finished())
        {
            if let Some(w) = self.recovery.worker.take() {
                if let Ok(Err(e)) = w.join() {
                    self.set_message(format!("Recovery snapshot failed: {e}"));
                }
            }
        }
        if self.recovery.worker.is_some() || self.recovery.last.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.recovery.last = Instant::now();
        let dirty: Vec<_> = self
            .buffers
            .iter()
            .filter(|b| {
                (b.note_id.is_some()
                    || b.path
                        .as_ref()
                        .is_some_and(|p| p.starts_with(&self.project_root)))
                    && b.is_modified()
            })
            .collect();
        let versions: Vec<_> = dirty.iter().map(|b| (b.id, b.edit_seq)).collect();
        if versions == self.recovery.versions {
            return;
        }
        self.recovery.versions = versions;
        let snapshots: Vec<_> = dirty
            .iter()
            .map(|b| {
                (
                    b.path.clone().unwrap_or_else(|| self.project_root.clone()),
                    b.note_id
                        .and_then(|id| self.notes.items.iter().find(|n| n.id == id).cloned()),
                    b.rope.clone(),
                    b.cursor_line,
                    b.cursor_col,
                )
            })
            .collect();
        let root = self.project_root.clone();
        let path = self
            .recovery
            .path
            .get_or_insert_with(|| {
                root.join(format!(".vaayu/recovery-{}.json", std::process::id()))
            })
            .clone();
        self.recovery.worker = Some(std::thread::spawn(move || {
            if snapshots.is_empty() {
                let _ = std::fs::remove_file(path);
                return Ok(());
            }
            let dir = root.join(".vaayu");
            if !dir.exists() {
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder.create(&dir)?;
            }
            anyhow::ensure!(
                !std::fs::symlink_metadata(&dir)?.file_type().is_symlink(),
                "recovery directory must not be a symlink"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            }
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                anyhow::ensure!(
                    !meta.file_type().is_symlink(),
                    "recovery file must not be a symlink"
                );
            }
            if !dir.join(".gitignore").exists() {
                crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true)?;
            }
            let drafts: Vec<_> = snapshots
                .into_iter()
                .map(|(path, note, rope, line, col)| Draft {
                    note,
                    path,
                    text: rope.to_string(),
                    line,
                    col,
                })
                .collect();
            crate::files::atomic_write(&path, &serde_json::to_vec(&drafts)?, true)
        }));
    }
    pub fn show_recovery(&mut self) {
        let mut entries = Vec::new();
        if let Ok(dir) = std::fs::read_dir(self.project_root.join(".vaayu")) {
            for file in dir.flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                if !name.starts_with("recovery-") || !name.ends_with(".json") {
                    continue;
                }
                if let Ok(pid) = name
                    .trim_start_matches("recovery-")
                    .trim_end_matches(".json")
                    .parse::<u32>()
                {
                    if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                        continue;
                    }
                }
                if let Ok(text) = std::fs::read_to_string(file.path()) {
                    if let Ok(drafts) = serde_json::from_str::<Vec<Draft>>(&text) {
                        for d in drafts {
                            if !d.path.starts_with(&self.project_root) {
                                continue;
                            }
                            let mut e = Entry::location(
                                d.path.clone(),
                                d.line,
                                d.col,
                                format!("Recover draft · {}", d.path.display()),
                            );
                            e.detail = d.text.lines().take(4).collect::<Vec<_>>().join("\n");
                            e.action = Some(serde_json::json!({"_vaayu_recovery":d}));
                            entries.push(e);
                        }
                    }
                }
            }
        }
        self.show_results(Results::new(
            "Recovery drafts — Enter restores into an unsaved buffer",
            entries,
        ));
    }
    pub fn restore_recovery(&mut self, value: serde_json::Value) {
        let Ok(d) = serde_json::from_value::<Draft>(value) else {
            self.set_message("Invalid recovery draft");
            return;
        };
        if let Some(mut note) = d.note {
            // Restore as a new note so a newer saved comment is never replaced.
            note.id = self.notes.items.iter().map(|n| n.id).max().unwrap_or(0) + 1;
            note.text = d.text;
            let id = note.id;
            self.notes.items.push(note);
            self.notes.dirty = true;
            self.edit_note(id);
            self.set_message("Comment draft restored — Ctrl-S saves it");
            return;
        }
        if self
            .buffers
            .iter()
            .any(|b| b.path.as_ref() == Some(&d.path) && b.is_modified())
        {
            self.set_message("Save or discard the current edits before restoring this file");
            return;
        }
        if let Err(e) = self.open_file(d.path) {
            self.set_message(e.to_string());
            return;
        }
        self.buf_mut().begin_edit();
        let len = self.buf().rope.len_chars();
        self.buf_mut().delete_char_range(0, len);
        self.buf_mut().insert_str_at(0, &d.text);
        self.buf_mut().commit_edit();
        self.set_cursor(d.line, d.col);
        self.enter_normal();
        self.set_message("Draft restored in memory — inspect and :w to save; u to undo");
    }
}
