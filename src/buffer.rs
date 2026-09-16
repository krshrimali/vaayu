use ropey::Rope;
use std::path::PathBuf;

#[derive(Clone)]
struct UndoState {
    rope: Rope,
    cursor: (usize, usize),
}

#[derive(Clone)]
pub struct Buffer {
    pub id: u64,
    pub note_id: Option<u64>,
    disk_text: Option<String>,
    dirty_cache: std::cell::Cell<Option<(u64, bool)>>,
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub top_line: usize,
    pub top_wrap: usize,
    pub left_col: usize,
    pub desired_col: usize,
    /// Bumped on every content-changing operation, *including* undo/redo --
    /// this is a content revision, not just an "edited since load" flag, so
    /// syntax/git/LSP staleness checks (which key off it) see undo/redo too.
    pub edit_seq: u64,
    /// A snapshot of the rope as of the last load/save. `is_modified()`
    /// compares *content*, not a revision number, against this -- Rope
    /// clones are O(1) (structural sharing), so this costs nothing extra,
    /// and it means undoing back to exactly the saved text is correctly
    /// clean again, not just "fewer edits than before."
    saved_snapshot: Rope,
    undo_stack: Vec<UndoState>,
    redo_stack: Vec<UndoState>,
    pending_undo: Option<UndoState>,
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Buffer {
    pub fn resource_baseline(&mut self, text: String) {
        self.saved_snapshot = Rope::from_str(&text);
        self.disk_text = Some(text);
        self.dirty_cache.set(None);
    }

    pub fn empty() -> Buffer {
        let rope = Rope::from_str("\n");
        Buffer {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            note_id: None,
            disk_text: None,
            dirty_cache: std::cell::Cell::new(None),
            saved_snapshot: rope.clone(),
            rope,
            path: None,
            cursor_line: 0,
            cursor_col: 0,
            top_line: 0,
            top_wrap: 0,
            left_col: 0,
            desired_col: 0,
            edit_seq: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_undo: None,
        }
    }

    pub fn from_path(path: PathBuf) -> anyhow::Result<Buffer> {
        let path = crate::files::identity(&path);
        let content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            String::new()
        };
        let disk_text = if path.exists() {
            Some(content.clone())
        } else {
            None
        };
        let rope = Rope::from_str(&content);
        Ok(Buffer {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            note_id: None,
            disk_text,
            dirty_cache: std::cell::Cell::new(None),
            saved_snapshot: rope.clone(),
            rope,
            path: Some(path),
            cursor_line: 0,
            cursor_col: 0,
            top_line: 0,
            top_wrap: 0,
            left_col: 0,
            desired_col: 0,
            edit_seq: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_undo: None,
        })
    }

    pub fn is_modified(&self) -> bool {
        if let Some((seq, dirty)) = self.dirty_cache.get() {
            if seq == self.edit_seq {
                return dirty;
            }
        }
        let dirty = self.rope != self.saved_snapshot;
        self.dirty_cache.set(Some((self.edit_seq, dirty)));
        dirty
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no file name"))?;
        anyhow::ensure!(
            !std::fs::metadata(&path).is_ok_and(|m| m.permissions().readonly()),
            "file is read-only; use :w! to overwrite"
        );
        let actual = match std::fs::read_to_string(&path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(
            actual == self.disk_text,
            "file changed on disk; reload or use :w! to overwrite"
        );
        self.save_force()?;
        Ok(())
    }

    /// Only commits the new path once the write actually succeeds -- a
    /// failed save-as (bad directory, permissions) used to leave the
    /// buffer pointing at a path it never wrote, so a later plain `:w`
    /// would silently target the wrong file.
    pub fn save_as(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let path = crate::files::identity(&path);
        if self.path.as_ref() == Some(&path) {
            return self.save();
        }
        anyhow::ensure!(!path.exists(), "target already exists");
        crate::files::atomic_write(&path, self.rope.to_string().as_bytes(), false)?;
        self.path = Some(path);
        self.mark_saved();
        Ok(())
    }

    pub fn mark_saved(&mut self) {
        self.saved_snapshot = self.rope.clone();
        self.disk_text = Some(self.rope.to_string());
        self.dirty_cache.set(None);
    }
    pub fn save_force(&mut self) -> anyhow::Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no file name"))?;
        crate::files::atomic_write(path, self.rope.to_string().as_bytes(), false)?;
        self.mark_saved();
        Ok(())
    }

    pub fn name(&self) -> String {
        match &self.path {
            Some(p) => p.display().to_string(),
            None => self
                .note_id
                .map(|id| format!("[Private comment #{id}]"))
                .unwrap_or_else(|| "[No Name]".into()),
        }
    }

    pub fn line_count(&self) -> usize {
        // ropey counts a trailing empty line after the final '\n' as a line;
        // treat a fully-empty trailing line as not existing for cursor purposes.
        let lc = self.rope.len_lines();
        if lc > 1 && self.rope.line(lc - 1).len_chars() == 0 {
            lc - 1
        } else {
            lc.max(1)
        }
    }

    /// Length of a line in chars, excluding the trailing newline.
    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let l = self.rope.line(line);
        let mut n = l.len_chars();
        if n > 0 && l.char(n - 1) == '\n' {
            n -= 1;
            if n > 0 && l.char(n - 1) == '\r' {
                n -= 1;
            }
        }
        n
    }

    pub fn line_text(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let l = self.rope.line(line);
        let mut s = l.to_string();
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        s
    }

    pub fn char_idx(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let base = self.rope.line_to_char(line);
        let len = self.line_len(line);
        base + col.min(len)
    }

    /// Byte range of a line's content, excluding the trailing newline --
    /// matches `line_len`'s char-based exclusion, for tree-sitter spans
    /// (which are byte-indexed) to line up with rendering (char-indexed).
    pub fn line_byte_range(&self, line: usize) -> (usize, usize) {
        if line >= self.rope.len_lines() {
            let n = self.rope.len_bytes();
            return (n, n);
        }
        let start = self.rope.line_to_byte(line);
        let end_char = self.rope.line_to_char(line) + self.line_len(line);
        let end = self.rope.char_to_byte(end_char);
        (start, end)
    }

    pub fn pos_from_char_idx(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        let col = idx - self.rope.line_to_char(line);
        (line, col)
    }

    /// Clamp column for normal-mode display (cursor may not sit past the last char).
    pub fn clamp_col_normal(&self, line: usize, col: usize) -> usize {
        let len = self.line_len(line);
        if len == 0 {
            0
        } else {
            crate::grapheme::floor(&self.line_text(line), col.min(len - 1))
        }
    }

    pub fn clamp_col_insert(&self, line: usize, col: usize) -> usize {
        col.min(self.line_len(line))
    }

    pub fn first_non_blank(&self, line: usize) -> usize {
        let text = self.line_text(line);
        text.chars().position(|c| !c.is_whitespace()).unwrap_or(0)
    }

    // ---------------- editing (with undo tracking) ----------------

    pub fn begin_edit(&mut self) {
        if self.pending_undo.is_none() {
            self.pending_undo = Some(UndoState {
                rope: self.rope.clone(),
                cursor: (self.cursor_line, self.cursor_col),
            });
        }
    }

    /// Commits the edit started by `begin_edit`, unless the content is
    /// actually unchanged (e.g. `i<Esc>` with nothing typed, or a `:s` that
    /// matched but produced identical text) -- a no-op edit must not dirty
    /// the buffer, push a no-op undo entry, or discard redo history.
    pub fn commit_edit(&mut self) {
        if let Some(state) = self.pending_undo.take() {
            if state.rope == self.rope {
                return;
            }
            self.undo_stack.push(state);
            self.redo_stack.clear();
            // Every mutation method has already advanced edit_seq. Committing
            // only closes the undo transaction; bumping again here would
            // advertise a content change that never happened and make syntax
            // and LSP clients repeat their work when Insert mode ends.
        }
    }

    pub fn undo(&mut self) -> bool {
        if let Some(state) = self.undo_stack.pop() {
            let current = UndoState {
                rope: self.rope.clone(),
                cursor: (self.cursor_line, self.cursor_col),
            };
            self.redo_stack.push(current);
            self.rope = state.rope;
            self.cursor_line = state.cursor.0.min(self.line_count().saturating_sub(1));
            self.cursor_col = self.clamp_col_normal(self.cursor_line, state.cursor.1);
            self.edit_seq += 1;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(state) = self.redo_stack.pop() {
            let current = UndoState {
                rope: self.rope.clone(),
                cursor: (self.cursor_line, self.cursor_col),
            };
            self.undo_stack.push(current);
            self.rope = state.rope;
            self.cursor_line = state.cursor.0.min(self.line_count().saturating_sub(1));
            self.cursor_col = self.clamp_col_normal(self.cursor_line, state.cursor.1);
            self.edit_seq += 1;
            true
        } else {
            false
        }
    }

    // These bump `edit_seq` on every call, independent of begin_edit/
    // commit_edit's undo-transaction batching -- Insert mode intentionally
    // batches a whole typing session into one undo step, but syntax/git/LSP
    // staleness checks key off edit_seq and need to see *every* keystroke
    // live, not just the state once the user leaves Insert. Without this,
    // a mid-typing render can pair fresh line text with a stale parse tree
    // (byte offsets computed against the old text), which can slice a new
    // multibyte character at a non-boundary and panic.

    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) {
        let idx = self.char_idx(line, col);
        self.rope.insert_char(idx, ch);
        self.edit_seq += 1;
    }

    pub fn insert_str(&mut self, line: usize, col: usize, s: &str) {
        let idx = self.char_idx(line, col);
        self.rope.insert(idx, s);
        self.edit_seq += 1;
    }

    /// Char-index-addressed variant of `insert_char`, for call sites that
    /// already have a rope char index rather than (line, col).
    pub fn insert_char_at(&mut self, idx: usize, ch: char) {
        self.rope.insert_char(idx, ch);
        self.edit_seq += 1;
    }

    /// Char-index-addressed variant of `insert_str`.
    pub fn insert_str_at(&mut self, idx: usize, s: &str) {
        self.rope.insert(idx, s);
        self.edit_seq += 1;
    }

    pub fn delete_char_range(&mut self, start: usize, end: usize) -> String {
        if end <= start {
            return String::new();
        }
        let end = end.min(self.rope.len_chars());
        if start >= end {
            return String::new();
        }
        let text = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        self.edit_seq += 1;
        text
    }

    pub fn text_range(&self, start: usize, end: usize) -> String {
        let end = end.min(self.rope.len_chars());
        if end <= start {
            return String::new();
        }
        self.rope.slice(start..end).to_string()
    }
}
