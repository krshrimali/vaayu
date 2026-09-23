//! Autocommand bus. A small set of editor lifecycle events that user
//! `[[autocmd]]` entries (config.rs `Autocmd`) can hook with an Ex command,
//! plus the built-in on-save actions (trim trailing whitespace, ensure a
//! final newline) which are modelled as our own `BufWritePre` handlers.
//!
//! This is deliberately declarative -- events + glob pattern + an Ex command
//! -- not a scripting host. It is enough to cover the common Neovim autocmd
//! uses (format/trim on save, per-filetype tweaks) without a plugin runtime.
use crate::editor::Editor;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    BufWritePre,
    BufWritePost,
    BufEnter,
    InsertLeave,
    FocusGained,
    CursorHold,
    FileType,
}

impl Event {
    fn name(self) -> &'static str {
        match self {
            Event::BufWritePre => "BufWritePre",
            Event::BufWritePost => "BufWritePost",
            Event::BufEnter => "BufEnter",
            Event::InsertLeave => "InsertLeave",
            Event::FocusGained => "FocusGained",
            Event::CursorHold => "CursorHold",
            Event::FileType => "FileType",
        }
    }

    /// Parse an event name from config (case-insensitive).
    pub fn from_name(s: &str) -> Option<Event> {
        [
            Event::BufWritePre,
            Event::BufWritePost,
            Event::BufEnter,
            Event::InsertLeave,
            Event::FocusGained,
            Event::CursorHold,
            Event::FileType,
        ]
        .into_iter()
        .find(|e| e.name().eq_ignore_ascii_case(s))
    }
}

/// Minimal shell-style glob match supporting `*` (any run) and `?` (one char).
/// Matches the whole string (anchored). `*` alone is the common "match all".
fn glob_match(glob: &str, name: &str) -> bool {
    if glob == "*" {
        return true;
    }
    let mut re = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    regex::Regex::new(&re).is_ok_and(|r| r.is_match(name))
}

impl Editor {
    /// Fire `event`: run the built-in handlers, then every matching user
    /// `[[autocmd]]`'s Ex command. Re-entrancy (an autocmd that re-fires the
    /// same or another event) is bounded so a pathological config can't hang.
    pub fn fire_event(&mut self, event: Event) {
        if self.event_depth >= 8 {
            return;
        }
        self.event_depth += 1;

        // Built-in on-save actions, modelled as our own BufWritePre handlers.
        if event == Event::BufWritePre {
            if self.config.trim_trailing_whitespace {
                self.trim_trailing_whitespace();
            }
            if self.config.insert_final_newline {
                self.ensure_final_newline();
            }
        }

        // The buffer name the glob patterns match against: both the basename
        // and the full path, so `*.rs` and `*/tests/*` both work. A nameless
        // (scratch/note) buffer only matches `*`.
        let (basename, fullpath) = match &self.buf().path {
            Some(p) => (
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                p.display().to_string(),
            ),
            None => (String::new(), String::new()),
        };
        let commands: Vec<String> = self
            .config
            .autocmd
            .iter()
            .filter(|a| Event::from_name(&a.event) == Some(event))
            .filter(|a| !a.command.is_empty())
            .filter(|a| glob_match(&a.pattern, &basename) || glob_match(&a.pattern, &fullpath))
            .map(|a| a.command.clone())
            .collect();
        for cmd in commands {
            crate::command::run_ex(self, &cmd);
        }

        self.event_depth -= 1;
    }

    /// Terminal regained focus: fire the `FocusGained` autocmd event and
    /// auto-reload the current buffer if its file changed on disk and has no
    /// unsaved edits (a dirty buffer is only warned about, never clobbered).
    pub fn on_focus_gained(&mut self) {
        self.fire_event(Event::FocusGained);
        if self.buf().changed_on_disk() {
            if self.buf().is_modified() {
                self.set_message("W: file changed on disk (:e! to reload)");
            } else if self.buf_mut().reload().is_ok() {
                self.set_message("File reloaded (changed on disk)");
            }
        }
    }

    /// Remove trailing spaces/tabs from every line, as one undo step. A `\r`
    /// (CRLF line) is intentionally left alone.
    fn trim_trailing_whitespace(&mut self) {
        let has_trailing = {
            let b = self.buf();
            (0..b.line_count()).any(|l| {
                let t = b.line_text(l);
                t.trim_end_matches([' ', '\t']).len() != t.len()
            })
        };
        if !has_trailing {
            return;
        }
        {
            let b = self.buf_mut();
            b.begin_edit();
            for line in 0..b.line_count() {
                let text = b.line_text(line);
                let full = text.chars().count();
                let trimmed = text.trim_end_matches([' ', '\t']).chars().count();
                if trimmed < full {
                    let start = b.char_idx(line, trimmed);
                    let end = b.char_idx(line, full);
                    b.delete_char_range(start, end);
                }
            }
            b.commit_edit();
        }
        // The cursor may have sat on trimmed text; re-clamp it.
        let (l, c) = self.cursor();
        self.set_cursor(l, c);
    }

    /// Ensure a non-empty buffer ends with exactly one `\n`. An empty buffer
    /// is left empty (matching Vim's `fixendofline`).
    fn ensure_final_newline(&mut self) {
        let b = self.buf_mut();
        let len = b.rope.len_chars();
        if len > 0 && b.rope.char(len - 1) != '\n' {
            b.begin_edit();
            b.insert_char_at(len, '\n');
            b.commit_edit();
        }
    }
}
