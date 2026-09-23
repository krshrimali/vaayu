use ropey::Rope;
use std::path::PathBuf;

#[derive(Clone)]
struct UndoState {
    rope: Rope,
    cursor: (usize, usize),
}

/// The line-ending convention of a file on disk. The in-memory rope always
/// holds `\n`-only text; the format is recorded on load and re-applied on
/// save so a DOS/old-Mac file round-trips without its endings being flipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FileFormat {
    #[default]
    Unix, // \n
    Dos,  // \r\n
    Mac,  // \r
}

impl FileFormat {
    pub fn name(self) -> &'static str {
        match self {
            FileFormat::Unix => "unix",
            FileFormat::Dos => "dos",
            FileFormat::Mac => "mac",
        }
    }
    pub fn parse(s: &str) -> Option<FileFormat> {
        match s {
            "unix" => Some(FileFormat::Unix),
            "dos" => Some(FileFormat::Dos),
            "mac" => Some(FileFormat::Mac),
            _ => None,
        }
    }
    fn eol(self) -> &'static str {
        match self {
            FileFormat::Unix => "\n",
            FileFormat::Dos => "\r\n",
            FileFormat::Mac => "\r",
        }
    }
}

/// Strips a leading UTF-8 BOM and detects the dominant line ending, returning
/// the `\n`-normalized text alongside the detected format and whether a BOM
/// was present. DOS wins if any `\r\n` occurs; a lone `\r` implies old-Mac.
fn normalize_content(raw: &str) -> (String, FileFormat, bool) {
    let (bom, s) = match raw.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let ff = if s.contains("\r\n") {
        FileFormat::Dos
    } else if s.contains('\r') {
        FileFormat::Mac
    } else {
        FileFormat::Unix
    };
    let content = match ff {
        FileFormat::Unix => s.to_string(),
        FileFormat::Dos => s.replace("\r\n", "\n"),
        FileFormat::Mac => s.replace('\r', "\n"),
    };
    (content, ff, bom)
}

/// The byte encoding of a file on disk. The rope always holds Rust `String`
/// (UTF-8 internally); this records how to decode on load and re-encode on
/// save so a latin1/UTF-16 file round-trips without corruption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    #[default]
    Utf8,
    Latin1,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf-8",
            Encoding::Latin1 => "latin1",
            Encoding::Utf16Le => "utf-16le",
            Encoding::Utf16Be => "utf-16be",
        }
    }
}

/// Decodes file bytes to a `String`, detecting the encoding: a UTF-16 BOM
/// (`FF FE`/`FE FF`) → UTF-16; otherwise valid UTF-8 → UTF-8; otherwise
/// latin1 (every byte is a code point). The returned string keeps its original
/// line endings and any leading BOM char (both handled by `normalize_content`).
fn decode_bytes(bytes: &[u8]) -> (String, Encoding) {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return (String::from_utf16_lossy(&u16s), Encoding::Utf16Le);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return (String::from_utf16_lossy(&u16s), Encoding::Utf16Be);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), Encoding::Utf8),
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), Encoding::Latin1),
    }
}

/// Encodes UTF-8 `text` to bytes in `encoding` (inverse of `decode_bytes`).
/// A UTF-16 BOM is prepended; latin1 replaces non-representable chars with `?`.
fn encode_bytes(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|c| if (c as u32) <= 0xFF { c as u8 } else { b'?' })
            .collect(),
        Encoding::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            for u in text.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
        Encoding::Utf16Be => {
            let mut out = vec![0xFE, 0xFF];
            for u in text.encode_utf16() {
                out.extend_from_slice(&u.to_be_bytes());
            }
            out
        }
    }
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
    /// Resolved once when the buffer is opened (see `crate::indent`), not
    /// read from the global `Config` on every use -- a project can freely
    /// mix a tab-indented file with a space-indented one open at once.
    /// `Buffer::empty`/`from_path` set neutral placeholders; callers with
    /// a `Config` in scope refine them with `apply_indent`.
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub expandtab: bool,
    pub indent_source: crate::indent::IndentSource,
    /// Line-ending convention detected on load, re-applied on save.
    pub fileformat: FileFormat,
    /// Whether the file began with a UTF-8 BOM (preserved on save).
    pub bom: bool,
    /// Byte encoding detected on load, re-applied on save.
    pub encoding: Encoding,
    /// Manual folds over inclusive line ranges. A closed fold hides its inner
    /// lines (all but its first) in the display. Not yet adjusted on edits --
    /// ranges are clamped to the buffer at use time (see `render`/motions).
    pub folds: Vec<Fold>,
    /// Per-file `.editorconfig` overrides (resolved on load), each falling back
    /// to the global config when `None`.
    pub ec_trim_trailing: Option<bool>,
    pub ec_final_newline: Option<bool>,
    pub ec_max_line_length: Option<usize>,
}

/// A manual fold over an inclusive line range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fold {
    pub start: usize,
    pub end: usize,
    pub closed: bool,
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
            tabstop: 4,
            shiftwidth: 4,
            expandtab: true,
            indent_source: crate::indent::IndentSource::Default,
            fileformat: FileFormat::default(),
            bom: false,
            encoding: Encoding::default(),
            folds: Vec::new(),
            ec_trim_trailing: None,
            ec_final_newline: None,
            ec_max_line_length: None,
        }
    }

    pub fn from_path(path: PathBuf) -> anyhow::Result<Buffer> {
        let path = crate::files::identity(&path);
        let exists = path.exists();
        let (raw, encoding) = if exists {
            decode_bytes(&std::fs::read(&path)?)
        } else {
            (String::new(), Encoding::default())
        };
        let (content, fileformat, bom) = normalize_content(&raw);
        // `disk_text` mirrors the in-memory (\n-normalized) content, so the
        // external-change checks compare like-for-like against a re-read that
        // is normalized the same way (see `changed_on_disk`/`save`).
        let disk_text = if exists { Some(content.clone()) } else { None };
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
            tabstop: 4,
            shiftwidth: 4,
            expandtab: true,
            indent_source: crate::indent::IndentSource::Default,
            fileformat,
            bom,
            encoding,
            folds: Vec::new(),
            ec_trim_trailing: None,
            ec_final_newline: None,
            ec_max_line_length: None,
        })
    }

    /// The rope's `\n`-only text re-encoded with this buffer's line ending and
    /// BOM — i.e. the exact bytes written to disk on save.
    pub fn encoded(&self) -> String {
        let lf = self.rope.to_string();
        let body = match self.fileformat {
            FileFormat::Unix => lf,
            FileFormat::Dos | FileFormat::Mac => lf.replace('\n', self.fileformat.eol()),
        };
        if self.bom {
            format!("\u{feff}{body}")
        } else {
            body
        }
    }

    /// The exact bytes written to disk on save: `encoded()` re-encoded in the
    /// buffer's byte encoding.
    pub fn encoded_bytes(&self) -> Vec<u8> {
        encode_bytes(&self.encoded(), self.encoding)
    }

    /// Refines the placeholder indent settings set by `empty`/`from_path`
    /// using `crate::indent::resolve` (modeline, then .editorconfig, then
    /// detected, then the given config's default). Callers that have a
    /// `Config` in scope should call this right after construction; it is
    /// a separate step (not baked into the constructors) so `Buffer`
    /// doesn't need to depend on `Config` at every call site that just
    /// wants an empty or loaded buffer (tests, notes, recovery drafts, ...).
    pub fn apply_indent(&mut self, cfg: &crate::config::Config) {
        let text = self.rope.to_string();
        let settings = crate::indent::resolve(self.path.as_deref(), &text, cfg);
        self.tabstop = settings.tabstop;
        self.shiftwidth = settings.shiftwidth;
        self.expandtab = settings.expandtab;
        self.indent_source = settings.source;
        // `.editorconfig` `end_of_line` overrides the ending detected from the
        // file's own content, so saving normalizes to the configured style.
        if let Some(eol) = self.path.as_deref().and_then(crate::indent::editorconfig_eol) {
            self.fileformat = eol;
        }
        if let Some(path) = self.path.as_deref() {
            let extras = crate::indent::editorconfig_extras(path);
            self.ec_trim_trailing = extras.trim_trailing;
            self.ec_final_newline = extras.final_newline;
            self.ec_max_line_length = extras.max_line_length;
        }
    }

    /// Whether this buffer's file has changed on disk since we last read/wrote
    /// it (for autoread-on-focus). False for a nameless or unreadable file.
    pub fn changed_on_disk(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        match std::fs::read(path) {
            // Compare the decoded, \n-normalized disk content against our
            // baseline, so a pure line-ending/encoding difference isn't a
            // false change.
            Ok(disk) => {
                let (raw, _) = decode_bytes(&disk);
                self.disk_text.as_deref() != Some(normalize_content(&raw).0.as_str())
            }
            Err(_) => false,
        }
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
        let actual = match std::fs::read(&path) {
            Ok(bytes) => Some(normalize_content(&decode_bytes(&bytes).0).0),
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
        crate::files::atomic_write(&path, &self.encoded_bytes(), false)?;
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
        crate::files::atomic_write(path, &self.encoded_bytes(), false)?;
        self.mark_saved();
        Ok(())
    }

    /// Re-reads this buffer's content from disk, discarding any in-memory
    /// changes and clearing undo/redo history -- matching real Vim's
    /// `:e!` (a reload isn't itself undoable, unlike a normal edit; there
    /// would be nothing coherent for `u` to reconstruct once the
    /// in-memory rope this undo stack was built against is gone). Used by
    /// `:e!`/`:edit!` directly, and by git hunk reset (discarding a hunk
    /// back to HEAD's content after `git apply --reverse` has already
    /// rewritten the file on disk out from under this buffer).
    pub fn reload(&mut self) -> anyhow::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no file name"))?;
        let (raw, encoding) = decode_bytes(&std::fs::read(&path)?);
        let (content, fileformat, bom) = normalize_content(&raw);
        self.fileformat = fileformat;
        self.bom = bom;
        self.encoding = encoding;
        self.rope = Rope::from_str(&content);
        self.saved_snapshot = self.rope.clone();
        self.disk_text = Some(content);
        self.dirty_cache.set(None);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.pending_undo = None;
        self.edit_seq += 1;
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = self.cursor_line.min(max_line);
        self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
        self.desired_col = self.cursor_col;
        self.top_line = self.top_line.min(max_line);
        self.top_wrap = 0;
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

    /// The closed fold that hides `line` -- i.e. `line` sits past the fold's
    /// first (still-visible) row. Ranges are clamped to the current buffer.
    pub fn hidden_by_fold(&self, line: usize) -> Option<Fold> {
        let last = self.line_count().saturating_sub(1);
        self.folds
            .iter()
            .copied()
            .find(|f| f.closed && f.start <= last && line > f.start && line <= f.end.min(last))
    }

    /// True when `line` is hidden inside a closed fold (all rows but the first).
    pub fn line_hidden(&self, line: usize) -> bool {
        self.hidden_by_fold(line).is_some()
    }

    /// A closed fold whose first line is exactly `line`, if any.
    pub fn closed_fold_starting_at(&self, line: usize) -> Option<Fold> {
        self.folds
            .iter()
            .copied()
            .find(|f| f.closed && f.start == line)
    }

    /// Step one visible line down, jumping over a closed fold that starts at
    /// `line` and never landing inside one.
    pub fn visible_line_below(&self, line: usize) -> usize {
        let last = self.line_count().saturating_sub(1);
        let mut next = match self.closed_fold_starting_at(line) {
            Some(f) => f.end.min(last).saturating_add(1),
            None => line + 1,
        };
        if let Some(f) = self.hidden_by_fold(next) {
            next = f.start;
        }
        next.min(last)
    }

    /// Step one visible line up, landing on a fold's start rather than inside.
    pub fn visible_line_above(&self, line: usize) -> usize {
        let prev = line.saturating_sub(1);
        match self.hidden_by_fold(prev) {
            Some(f) => f.start,
            None => prev,
        }
    }

    /// A cheap hash of the closed folds, for the layout cache key so toggling a
    /// fold invalidates cached viewports.
    pub fn folds_stamp(&self) -> u64 {
        let mut h: u64 = 0;
        for f in self.folds.iter().filter(|f| f.closed) {
            h = h.wrapping_mul(1_000_003).wrapping_add(f.start as u64 + 1);
            h = h.wrapping_mul(1_000_003).wrapping_add(f.end as u64 + 1);
        }
        h
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

    /// Exports the undo stack (oldest first) as plain text snapshots, for
    /// `crate::undofile`. Full-text, not diffs -- Rope clones are already
    /// structural-sharing, so this only materializes strings at the point
    /// they actually leave the process to be serialized.
    pub fn undo_snapshots(&self) -> Vec<(String, (usize, usize))> {
        self.undo_stack
            .iter()
            .map(|s| (s.rope.to_string(), s.cursor))
            .collect()
    }

    /// Replaces the undo stack with the given snapshots (oldest first) and
    /// clears redo -- used only right after loading a file, before any
    /// real edit has happened.
    pub fn restore_undo_snapshots(&mut self, snapshots: Vec<(String, (usize, usize))>) {
        self.undo_stack = snapshots
            .into_iter()
            .map(|(text, cursor)| UndoState {
                rope: Rope::from_str(&text),
                cursor,
            })
            .collect();
        self.redo_stack.clear();
    }

    // These bump `edit_seq` on every call, independent of begin_edit/
    // commit_edit's undo-transaction batching -- Insert mode intentionally
    // batches a whole typing session into one undo step, but syntax/git/LSP
    // staleness checks key off edit_seq and need to see *every* keystroke
    // live, not just the state once the user leaves Insert. Without this,
    // a mid-typing render can pair fresh line text with a stale parse tree
    // (byte offsets computed against the old text), which can slice a new
    // multibyte character at a non-boundary and panic.

    /// Shift/grow fold line ranges through an edit that changed the line count
    /// by `delta` at `edit_line`, so manual/auto folds keep covering the same
    /// logical region after inserts and deletes. Only mutates `self.folds`
    /// (never the rope) and is a no-op with no folds, so the hot edit path is
    /// untouched. Approximate for edits straddling a fold boundary; folds that
    /// collapse to <2 lines are dropped.
    fn adjust_folds_for_edit(&mut self, edit_line: usize, delta: i64) {
        if self.folds.is_empty() || delta == 0 {
            return;
        }
        let last = self.line_count().saturating_sub(1);
        let shift = |v: usize| -> usize { (v as i64 + delta).max(0) as usize };
        let mut kept = Vec::with_capacity(self.folds.len());
        for mut f in std::mem::take(&mut self.folds) {
            if f.start > edit_line {
                f.start = shift(f.start).min(last);
            }
            if f.end >= edit_line {
                f.end = shift(f.end).min(last);
            } else {
                f.end = f.end.min(last);
            }
            if f.end > f.start {
                kept.push(f);
            }
        }
        self.folds = kept;
    }

    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) {
        let idx = self.char_idx(line, col);
        let edit_line = (ch == '\n' && !self.folds.is_empty()).then(|| self.rope.char_to_line(idx));
        self.rope.insert_char(idx, ch);
        self.edit_seq += 1;
        if let Some(l) = edit_line {
            self.adjust_folds_for_edit(l, 1);
        }
    }

    pub fn insert_str(&mut self, line: usize, col: usize, s: &str) {
        let idx = self.char_idx(line, col);
        let nl = if self.folds.is_empty() {
            0
        } else {
            s.matches('\n').count()
        };
        let edit_line = (nl > 0).then(|| self.rope.char_to_line(idx));
        self.rope.insert(idx, s);
        self.edit_seq += 1;
        if let Some(l) = edit_line {
            self.adjust_folds_for_edit(l, nl as i64);
        }
    }

    /// Char-index-addressed variant of `insert_char`, for call sites that
    /// already have a rope char index rather than (line, col).
    pub fn insert_char_at(&mut self, idx: usize, ch: char) {
        let edit_line = (ch == '\n' && !self.folds.is_empty()).then(|| self.rope.char_to_line(idx));
        self.rope.insert_char(idx, ch);
        self.edit_seq += 1;
        if let Some(l) = edit_line {
            self.adjust_folds_for_edit(l, 1);
        }
    }

    /// Char-index-addressed variant of `insert_str`.
    pub fn insert_str_at(&mut self, idx: usize, s: &str) {
        let nl = if self.folds.is_empty() {
            0
        } else {
            s.matches('\n').count()
        };
        let edit_line = (nl > 0).then(|| self.rope.char_to_line(idx));
        self.rope.insert(idx, s);
        self.edit_seq += 1;
        if let Some(l) = edit_line {
            self.adjust_folds_for_edit(l, nl as i64);
        }
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
        let nl = if self.folds.is_empty() {
            0
        } else {
            text.matches('\n').count()
        };
        let edit_line = (nl > 0).then(|| self.rope.char_to_line(start));
        self.rope.remove(start..end);
        self.edit_seq += 1;
        if let Some(l) = edit_line {
            self.adjust_folds_for_edit(l, -(nl as i64));
        }
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
