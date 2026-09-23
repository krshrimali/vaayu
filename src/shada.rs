use crate::editor::Editor;
use crate::registers::RegisterEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Largest register text persisted; anything bigger is dropped rather than
/// bloating the shada file.
const MAX_REGISTER_BYTES: usize = 100_000;
/// How many command/search history entries to keep on disk.
const MAX_HISTORY: usize = 100;

/// On-disk shape of the per-project shada file (`.vaayu/shada.json`). New
/// fields are `#[serde(default)]` so an older file (positions only) still
/// loads.
#[derive(Serialize, Deserialize, Default)]
struct Shada {
    version: u32,
    positions: HashMap<String, (usize, usize)>,
    #[serde(default)]
    registers: HashMap<String, RegisterEntry>,
    #[serde(default)]
    command_history: Vec<String>,
    #[serde(default)]
    search_history: Vec<String>,
    #[serde(default)]
    marks: HashMap<String, SavedLoc>,
    #[serde(default)]
    jumps: Vec<SavedLoc>,
}

/// A persisted location: only the path (not the session's buffer id) survives,
/// so on restore it navigates by path.
#[derive(Serialize, Deserialize, Clone)]
struct SavedLoc {
    path: String,
    line: usize,
    col: usize,
}

impl SavedLoc {
    fn from_location(l: &crate::navigation::Location) -> Option<SavedLoc> {
        l.path.as_ref().map(|p| SavedLoc {
            path: p.display().to_string(),
            line: l.line,
            col: l.col,
        })
    }
    fn into_location(self) -> crate::navigation::Location {
        crate::navigation::Location {
            buffer: 0, // never a real buffer id — forces navigation by path
            path: Some(PathBuf::from(self.path)),
            line: self.line,
            col: self.col,
        }
    }
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
    /// Loads `.vaayu/shada.json` once: file positions, named registers, and
    /// command/search history. A missing or unreadable file is a no-op. Safe
    /// to call eagerly at startup (before any file opens).
    pub fn load_shada(&mut self) {
        if self.file_positions_loaded {
            return;
        }
        self.file_positions_loaded = true;
        let path = self.project_root.join(".vaayu/shada.json");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let Ok(s) = serde_json::from_slice::<Shada>(&bytes) else {
            return;
        };
        if s.version != 1 {
            return;
        }
        self.file_positions = s
            .positions
            .into_iter()
            .map(|(k, v)| (PathBuf::from(k), v))
            .collect();
        for (name, entry) in s.registers {
            if let Some(ch) = name.chars().next() {
                self.registers.restore(ch, entry);
            }
        }
        // Only seed histories we don't already have (startup: both empty).
        if self.command_history.is_empty() {
            self.command_history = s.command_history;
        }
        if self.search_history.is_empty() {
            self.search_history = s.search_history;
        }
        for (name, loc) in s.marks {
            if let Some(ch) = name.chars().next() {
                self.marks.insert(ch, loc.into_location());
            }
        }
        if self.jumps.is_empty() {
            self.jumps = s.jumps.into_iter().map(SavedLoc::into_location).collect();
            self.jump_index = self.jumps.len();
        }
    }

    /// Restores the current buffer's last-known cursor position from shada,
    /// clamped to the buffer. No-op when disabled, for a nameless buffer, or
    /// for a VCS message file.
    pub fn restore_file_position(&mut self) {
        if !self.config.restore_cursor {
            return;
        }
        self.load_shada();
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
        self.load_shada();
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
        // Persist named registers (skip clipboard/blackhole and oversized ones).
        let registers: HashMap<String, RegisterEntry> = self
            .registers
            .list()
            .into_iter()
            .filter(|(name, e)| {
                (name.is_ascii_alphanumeric() || *name == '"') && e.text.len() <= MAX_REGISTER_BYTES
            })
            .map(|(name, e)| (name.to_string(), e))
            .collect();
        let tail = |v: &[String]| -> Vec<String> {
            v.iter().rev().take(MAX_HISTORY).rev().cloned().collect()
        };
        // Named marks (a-z/A-Z/0-9) that point at a real file.
        let marks: HashMap<String, SavedLoc> = self
            .marks
            .iter()
            .filter(|(c, _)| c.is_ascii_alphanumeric())
            .filter_map(|(c, l)| SavedLoc::from_location(l).map(|s| (c.to_string(), s)))
            .collect();
        let mut jumps: Vec<SavedLoc> =
            self.jumps.iter().filter_map(SavedLoc::from_location).collect();
        if jumps.len() > MAX_HISTORY {
            jumps.drain(0..jumps.len() - MAX_HISTORY);
        }
        let shada = Shada {
            version: 1,
            positions,
            registers,
            command_history: tail(&self.command_history),
            search_history: tail(&self.search_history),
            marks,
            jumps,
        };
        let dir = self.project_root.join(".vaayu");
        let Ok(_lock) = crate::files::private_lock(&dir, "shada.lock") else {
            return;
        };
        let _ = crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true);
        if let Ok(bytes) = serde_json::to_vec(&shada) {
            let _ = crate::files::atomic_write(&dir.join("shada.json"), &bytes, true);
        }
    }
}
