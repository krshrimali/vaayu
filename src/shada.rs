use crate::editor::Editor;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// On-disk shape of the per-project shada file (`.vaayu/shada.json`): a map of
/// file path → last cursor `(line, col)`.
#[derive(Serialize, Deserialize, Default)]
struct Shada {
    version: u32,
    positions: HashMap<String, (usize, usize)>,
}

/// VCS message files are intentionally left at the top on open (matching Vim),
/// so a stale `'"` never drops the cursor into the middle of a commit message.
fn is_vcs_message(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("COMMIT_EDITMSG" | "MERGE_MSG" | "git-rebase-todo" | "TAG_EDITMSG")
    )
}

impl Editor {
    /// Loads `.vaayu/shada.json` into `file_positions` once. A missing or
    /// unreadable file just leaves the map empty.
    fn ensure_shada_loaded(&mut self) {
        if self.file_positions_loaded {
            return;
        }
        self.file_positions_loaded = true;
        let path = self.project_root.join(".vaayu/shada.json");
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(s) = serde_json::from_slice::<Shada>(&bytes) {
                if s.version == 1 {
                    self.file_positions = s
                        .positions
                        .into_iter()
                        .map(|(k, v)| (PathBuf::from(k), v))
                        .collect();
                }
            }
        }
    }

    /// Restores the current buffer's last-known cursor position from shada,
    /// clamped to the buffer. No-op when disabled, for a nameless buffer, or
    /// for a VCS message file.
    pub fn restore_file_position(&mut self) {
        if !self.config.restore_cursor {
            return;
        }
        self.ensure_shada_loaded();
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        if is_vcs_message(&path) {
            return;
        }
        if let Some(&(line, col)) = self.file_positions.get(&path) {
            self.set_cursor(line, col);
        }
    }

    /// Records every open file's current cursor and persists the shada file.
    /// Called on quit. Prunes entries whose file no longer exists so the store
    /// stays bounded.
    pub fn save_shada(&mut self) {
        if !self.config.restore_cursor {
            return;
        }
        self.ensure_shada_loaded();
        for b in &self.buffers {
            if let Some(p) = &b.path {
                if !is_vcs_message(p) {
                    self.file_positions
                        .insert(p.clone(), (b.cursor_line, b.cursor_col));
                }
            }
        }
        self.file_positions.retain(|p, _| p.exists());
        let positions: HashMap<String, (usize, usize)> = self
            .file_positions
            .iter()
            .map(|(k, v)| (k.display().to_string(), *v))
            .collect();
        let dir = self.project_root.join(".vaayu");
        let Ok(_lock) = crate::files::private_lock(&dir, "shada.lock") else {
            return;
        };
        let _ = crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true);
        if let Ok(bytes) = serde_json::to_vec(&Shada {
            version: 1,
            positions,
        }) {
            let _ = crate::files::atomic_write(&dir.join("shada.json"), &bytes, true);
        }
    }
}
