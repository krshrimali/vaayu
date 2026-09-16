//! Private project-local review annotations. Source files are never annotated.
use crate::{
    buffer::Buffer,
    editor::Editor,
    results::{Entry, Results},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
    pub whole_file: bool,
    pub anchor: String,
    pub text: String,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub resolved: bool,
}
#[derive(Default, Serialize, Deserialize)]
struct Document {
    version: u32,
    notes: Vec<Note>,
}
pub struct Notes {
    pub items: Vec<Note>,
    pub dirty: bool,
    path: PathBuf,
    disk: Option<String>,
    pub load_error: Option<String>,
}
impl Notes {
    pub fn load(root: &Path) -> Self {
        let path = root.join(".vaayu/comments.json");
        let mut out = Self {
            items: Vec::new(),
            dirty: false,
            path,
            disk: None,
            load_error: None,
        };
        match std::fs::read_to_string(&out.path) {
            Ok(s) => {
                out.disk = Some(s.clone());
                match serde_json::from_str::<Document>(&s) {
                    Ok(d) if d.version == 1 => out.items = d.notes,
                    Ok(_) => out.load_error = Some("unsupported comments version".into()),
                    Err(e) => out.load_error = Some(e.to_string()),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => out.load_error = Some(e.to_string()),
        }
        out
    }
    pub fn save(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.load_error.is_none(),
            "comments could not be loaded; refusing to overwrite: {}",
            self.load_error.as_deref().unwrap_or("")
        );
        let dir = self.path.parent().unwrap();
        let _lock = crate::files::private_lock(dir, "comments.lock")?;
        if !dir.exists() {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(dir)?;
        }
        anyhow::ensure!(
            !std::fs::symlink_metadata(dir)?.file_type().is_symlink(),
            "comments directory must not be a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        for p in [&self.path, &dir.join(".gitignore")] {
            if let Ok(meta) = std::fs::symlink_metadata(p) {
                anyhow::ensure!(
                    !meta.file_type().is_symlink(),
                    "private store files must not be symlinks"
                );
            }
        }
        let current = match std::fs::read_to_string(&self.path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(
            current == self.disk,
            "comments changed in another session; reopen before saving"
        );
        crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true)?;
        let text = serde_json::to_string_pretty(&Document {
            version: 1,
            notes: self.items.clone(),
        })?;
        crate::files::atomic_write(&self.path, text.as_bytes(), true)?;
        self.disk = Some(text);
        self.dirty = false;
        Ok(())
    }
    pub fn relocate(note: &mut Note, text: &str) {
        if note.whole_file || note.anchor.is_empty() {
            return;
        }
        let lines: Vec<&str> = text.lines().collect();
        let n = note.anchor.lines().count();
        let mut matches: Vec<usize> = (0..lines.len())
            .filter(|i| i + n <= lines.len() && lines[*i..*i + n].join("\n") == note.anchor)
            .collect();
        if matches.is_empty() {
            let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
            let anchor = norm(&note.anchor);
            matches = (0..lines.len())
                .filter(|i| i + n <= lines.len() && norm(&lines[*i..*i + n].join("\n")) == anchor)
                .collect();
        }
        if matches.len() == 1 {
            note.start = matches[0];
            note.end = note.start + n.saturating_sub(1);
            note.stale = false;
        } else {
            note.stale = !matches.contains(&note.start);
        }
    }
}
impl Editor {
    pub fn new_note(&mut self, whole_file: bool) {
        let Some(path) = self.buf().path.clone() else {
            self.set_message("Open a source file before adding a comment");
            return;
        };
        let Ok(file) = path.strip_prefix(&self.project_root) else {
            self.set_message("Comments must refer to files inside the current project folder");
            return;
        };
        if let Some(err) = &self.notes.load_error {
            self.set_message(format!("Cannot load comments: {err}"));
            return;
        }
        let (a, b) = if let Some(a) = self.visual_anchor.take() {
            (a.0.min(self.cursor().0), a.0.max(self.cursor().0))
        } else {
            (self.cursor().0, self.cursor().0)
        };
        let anchor = if whole_file {
            String::new()
        } else {
            (a..=b)
                .map(|l| self.buf().line_text(l))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let id = self.notes.items.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        self.notes.items.push(Note {
            id,
            file: file.to_path_buf(),
            start: a,
            end: b,
            whole_file,
            anchor,
            text: String::new(),
            stale: false,
            resolved: false,
        });
        self.notes.dirty = true;
        self.edit_note(id);
    }
    pub fn edit_note(&mut self, id: u64) {
        let Some(note) = self.notes.items.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        if let Some(i) = self.buffers.iter().position(|b| b.note_id == Some(id)) {
            self.cur = i;
            self.enter_normal();
            return;
        }
        let mut b = Buffer::empty();
        b.rope = ropey::Rope::from_str(&note.text);
        b.note_id = Some(id);
        b.mark_saved();
        self.buffers.push(b);
        self.cur = self.buffers.len() - 1;
        self.invalidate_index_caches();
        self.enter_normal();
        self.set_message(format!(
            "Private comment #{} · {} · i to edit, Ctrl-S/:w to save, :comments to return",
            id,
            note.file.display()
        ));
    }
    pub fn save_notes(&mut self) -> anyhow::Result<()> {
        let edits: Vec<_> = self
            .buffers
            .iter()
            .filter_map(|b| b.note_id.map(|id| (id, b.rope.to_string())))
            .collect();
        for (id, text) in edits {
            if let Some(n) = self.notes.items.iter_mut().find(|n| n.id == id) {
                if n.text != text {
                    n.text = text;
                    self.notes.dirty = true;
                }
            }
        }
        self.notes.save()?;
        for b in &mut self.buffers {
            if b.note_id.is_some() {
                b.mark_saved();
            }
        }
        Ok(())
    }
    pub fn save_current(&mut self) -> anyhow::Result<()> {
        if let Some(id) = self.buf().note_id {
            let text = self.buf().rope.to_string();
            let note = self
                .notes
                .items
                .iter_mut()
                .find(|n| n.id == id)
                .ok_or_else(|| anyhow::anyhow!("comment deleted"))?;
            note.text = text;
            self.notes.dirty = true;
            self.notes.save()?;
            self.buf_mut().mark_saved();
        } else {
            self.buf_mut().save()?;
            crate::undofile::save(&self.project_root, self.buf());
            self.notify_saved();
        }
        Ok(())
    }
    pub fn comments_results(&mut self) {
        for note in &mut self.notes.items {
            let p = self.project_root.join(&note.file);
            let text = self
                .buffers
                .iter()
                .find(|b| b.path.as_ref() == Some(&p))
                .map(|b| b.rope.to_string())
                .or_else(|| std::fs::read_to_string(p).ok());
            if let Some(text) = text {
                let old = (note.start, note.end, note.stale);
                Notes::relocate(note, &text);
                if old != (note.start, note.end, note.stale) {
                    self.notes.dirty = true;
                }
            }
        }
        let entries = self
            .notes
            .items
            .iter()
            .map(|n| {
                let scope = if n.whole_file {
                    "file".into()
                } else {
                    format!("lines {}–{}", n.start + 1, n.end + 1)
                };
                let mut e = Entry::location(
                    self.project_root.join(&n.file),
                    n.start,
                    0,
                    format!(
                        "#{} [{}{}] {}",
                        n.id,
                        scope,
                        if n.resolved {
                            " · resolved"
                        } else if n.stale {
                            " · anchor changed"
                        } else {
                            ""
                        },
                        n.text.lines().next().unwrap_or("(empty comment)")
                    ),
                );
                e.note_id = Some(n.id);
                e.detail = format!(
                    "{}\n\n{}",
                    n.text,
                    if n.anchor.is_empty() {
                        String::new()
                    } else {
                        format!("Source at review:\n{}", n.anchor)
                    }
                );
                e
            })
            .collect();
        let mut r = Results::new("Private comments", entries);
        r.error = self.notes.load_error.clone();
        self.show_results(r);
    }
    pub fn delete_selected_notes(&mut self) {
        let Some(r) = &self.results else { return };
        let ids: Vec<u64> = r
            .entries
            .iter()
            .enumerate()
            .filter(|(i, _)| r.selected.contains(i) || (r.selected.is_empty() && *i == r.cursor))
            .filter_map(|(_, e)| e.note_id)
            .collect();
        if ids.is_empty() {
            return;
        }
        if self
            .buffers
            .iter()
            .any(|b| b.note_id.is_some_and(|id| ids.contains(&id)) && b.is_modified())
        {
            self.set_message("Save or discard the open comment edit before deleting");
            return;
        }
        self.notes.items.retain(|n| !ids.contains(&n.id));
        self.notes.dirty = true;
        self.buffers
            .retain(|b| !b.note_id.is_some_and(|id| ids.contains(&id)));
        if self.buffers.is_empty() {
            self.buffers.push(Buffer::empty());
        }
        self.cur = self.cur.min(self.buffers.len() - 1);
        self.invalidate_index_caches();
        self.comments_results();
        self.set_message("Comments removed in memory — Ctrl-S or :commentswrite to save");
    }
}
