use ropey::Rope;
use std::path::PathBuf;

#[derive(Clone)]
struct UndoState {
    rope: Rope,
    cursor: (usize, usize),
}

pub struct Buffer {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub top_line: usize,
    pub modified: bool,
    pub desired_col: usize,
    pub edit_seq: u64,
    undo_stack: Vec<UndoState>,
    redo_stack: Vec<UndoState>,
    pending_undo: Option<UndoState>,
}

impl Buffer {
    pub fn empty() -> Buffer {
        Buffer {
            rope: Rope::from_str("\n"),
            path: None,
            cursor_line: 0,
            cursor_col: 0,
            top_line: 0,
            modified: false,
            desired_col: 0,
            edit_seq: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_undo: None,
        }
    }

    pub fn from_path(path: PathBuf) -> anyhow::Result<Buffer> {
        let content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            String::new()
        };
        let content = if content.is_empty() { "\n".to_string() } else { content };
        Ok(Buffer {
            rope: Rope::from_str(&content),
            path: Some(path),
            cursor_line: 0,
            cursor_col: 0,
            top_line: 0,
            modified: false,
            desired_col: 0,
            edit_seq: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_undo: None,
        })
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no file name"))?;
        self.ensure_trailing_newline();
        let text = self.rope.to_string();
        std::fs::write(path, text)?;
        self.modified = false;
        Ok(())
    }

    pub fn save_as(&mut self, path: PathBuf) -> anyhow::Result<()> {
        self.path = Some(path);
        self.save()
    }

    pub fn name(&self) -> String {
        match &self.path {
            Some(p) => p.display().to_string(),
            None => "[No Name]".to_string(),
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
        }
        s
    }

    pub fn char_idx(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count().saturating_sub(1));
        let base = self.rope.line_to_char(line);
        let len = self.line_len(line);
        base + col.min(len)
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
            col.min(len - 1)
        }
    }

    pub fn clamp_col_insert(&self, line: usize, col: usize) -> usize {
        col.min(self.line_len(line))
    }

    pub fn first_non_blank(&self, line: usize) -> usize {
        let text = self.line_text(line);
        text.chars()
            .position(|c| !c.is_whitespace())
            .unwrap_or(0)
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

    pub fn commit_edit(&mut self) {
        if let Some(state) = self.pending_undo.take() {
            self.undo_stack.push(state);
            self.redo_stack.clear();
            self.modified = true;
            self.edit_seq += 1;
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
            true
        } else {
            false
        }
    }

    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) {
        let idx = self.char_idx(line, col);
        self.rope.insert_char(idx, ch);
    }

    pub fn insert_str(&mut self, line: usize, col: usize, s: &str) {
        let idx = self.char_idx(line, col);
        self.rope.insert(idx, s);
    }

    pub fn delete_char_range(&mut self, start: usize, end: usize) -> String {
        if end <= start {
            return String::new();
        }
        let end = end.min(self.rope.len_chars());
        let text = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        text
    }

    pub fn text_range(&self, start: usize, end: usize) -> String {
        let end = end.min(self.rope.len_chars());
        if end <= start {
            return String::new();
        }
        self.rope.slice(start..end).to_string()
    }

    pub fn ensure_trailing_newline(&mut self) {
        let len = self.rope.len_chars();
        if len == 0 || self.rope.char(len - 1) != '\n' {
            self.rope.insert_char(len, '\n');
        }
    }
}
