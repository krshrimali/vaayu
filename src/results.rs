//! One searchable, selectable result model for every producer and quickfix.
use crate::{editor::Editor, key::Key, mode::Mode};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: Option<PathBuf>,
    pub buffer_id: Option<u64>,
    pub line: usize,
    pub col: usize,
    pub text: String,
    pub detail: String,
    pub note_id: Option<u64>,
    pub action: Option<serde_json::Value>,
}
impl Entry {
    pub fn location(path: PathBuf, line: usize, col: usize, text: impl Into<String>) -> Self {
        Self {
            path: Some(path),
            buffer_id: None,
            line,
            col,
            text: text.into(),
            detail: String::new(),
            note_id: None,
            action: None,
        }
    }
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            path: None,
            buffer_id: None,
            line: 0,
            col: 0,
            text: text.into(),
            detail: String::new(),
            note_id: None,
            action: None,
        }
    }
    pub fn display(&self, root: &std::path::Path) -> String {
        match &self.path {
            Some(p) => format!(
                "{}:{}:{}  {}",
                p.strip_prefix(root).unwrap_or(p).display(),
                self.line + 1,
                self.col + 1,
                self.text
            ),
            None => self.text.clone(),
        }
    }
    pub fn export(&self, root: &std::path::Path) -> String {
        let mut text = self.display(root);
        if !self.detail.is_empty() && self.detail != self.text {
            text.push('\n');
            text.push_str(&self.detail);
        }
        text
    }
}
#[derive(Clone, Debug)]
pub struct Results {
    pub title: String,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub selected: BTreeSet<usize>,
    pub query: String,
    pub search_input: Option<bool>,
    pub search_forward: bool,
    pub quickfix: bool,
    pub live: bool,
    pub busy: bool,
    pub error: Option<String>,
}
impl Results {
    pub fn new(title: impl Into<String>, entries: Vec<Entry>) -> Self {
        Self {
            title: title.into(),
            entries,
            cursor: 0,
            selected: BTreeSet::new(),
            query: String::new(),
            search_input: None,
            search_forward: true,
            quickfix: false,
            live: false,
            busy: false,
            error: None,
        }
    }
    pub fn find(&mut self, forward: bool, ignorecase: bool, smartcase: bool) -> bool {
        if self.entries.is_empty() || self.query.is_empty() {
            return false;
        }
        let pattern = crate::vimregex::translate_pattern(&self.query);
        let re = match fancy_regex::RegexBuilder::new(&pattern)
            .backtrack_limit(100_000)
            .case_insensitive(
                ignorecase && !(smartcase && self.query.chars().any(char::is_uppercase)),
            )
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e.to_string());
                return false;
            }
        };
        for n in 1..=self.entries.len() {
            let idx = if forward {
                (self.cursor + n) % self.entries.len()
            } else {
                (self.cursor + self.entries.len() - n) % self.entries.len()
            };
            let e = &self.entries[idx];
            let matched = re.is_match(&format!(
                "{}\n{}",
                e.display(std::path::Path::new("")),
                e.detail
            ));
            let matched = match matched {
                Ok(v) => v,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return false;
                }
            };
            if matched {
                self.cursor = idx;
                self.error = None;
                return true;
            }
        }
        self.error = Some("Pattern not found".into());
        false
    }
    pub fn export(&self, all: bool, root: &std::path::Path) -> String {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                all || self.selected.contains(i) || (self.selected.is_empty() && *i == self.cursor)
            })
            .map(|(_, e)| e.export(root))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
pub fn handle(ed: &mut Editor, key: Key) {
    if ed.results.is_none() {
        ed.enter_normal();
        return;
    }
    let searching = ed.results.as_ref().unwrap().search_input;
    if let Some(forward) = searching {
        let r = ed.results.as_mut().unwrap();
        match key {
            Key::Esc => {
                r.search_input = None;
            }
            Key::Enter => {
                r.search_input = None;
                if !r.live {
                    r.search_forward = forward;
                    r.find(forward, ed.config.ignorecase, ed.config.smartcase);
                }
            }
            Key::Backspace => {
                r.query.pop();
                if r.live {
                    ed.schedule_grep();
                }
            }
            Key::Char(c) => {
                r.query.push(c);
                if r.live {
                    ed.schedule_grep();
                }
            }
            _ => {}
        }
        return;
    }
    match key {
        Key::Esc | Key::Char('q') => {
            ed.remember_results();
            ed.enter_normal();
        }
        Key::Enter => ed.open_result(),
        Key::Char('A') => ed.run_review(),
        Key::Char('R') => ed.resolve_review(),
        Key::Char('e') => {
            if let Some(id) = ed
                .results
                .as_ref()
                .and_then(|r| r.entries.get(r.cursor))
                .and_then(|e| e.note_id)
            {
                ed.edit_note(id);
            }
        }
        Key::Char('d') => ed.delete_selected_notes(),
        Key::Char('y') | Key::Char('Y') => {
            let text = ed
                .results
                .as_ref()
                .unwrap()
                .export(key == Key::Char('Y'), &ed.project_root);
            ed.registers.set(Some('+'), text, false);
            ed.set_message("Copied results to clipboard and + register");
        }
        Key::Char('/') | Key::Char('?') => {
            let r = ed.results.as_mut().unwrap();
            r.query.clear();
            r.search_input = Some(key == Key::Char('/'));
        }
        Key::Char('i') if ed.results.as_ref().unwrap().live => {
            ed.results.as_mut().unwrap().search_input = Some(true)
        }
        Key::Char('n') | Key::Char('N') => {
            let r = ed.results.as_mut().unwrap();
            let f = if key == Key::Char('n') {
                r.search_forward
            } else {
                !r.search_forward
            };
            r.find(f, ed.config.ignorecase, ed.config.smartcase);
        }
        Key::Tab | Key::Char(' ') => {
            let r = ed.results.as_mut().unwrap();
            if !r.entries.is_empty() {
                if !r.selected.insert(r.cursor) {
                    r.selected.remove(&r.cursor);
                }
                r.cursor = (r.cursor + 1).min(r.entries.len() - 1);
            }
        }
        Key::Char('a') => {
            let r = ed.results.as_mut().unwrap();
            if r.selected.len() == r.entries.len() {
                r.selected.clear();
            } else {
                r.selected = (0..r.entries.len()).collect();
            }
        }
        Key::Char('j') | Key::Down | Key::Ctrl('n') => {
            let r = ed.results.as_mut().unwrap();
            r.cursor = (r.cursor + 1).min(r.entries.len().saturating_sub(1));
        }
        Key::Char('k') | Key::Up | Key::Ctrl('p') => {
            let r = ed.results.as_mut().unwrap();
            r.cursor = r.cursor.saturating_sub(1);
        }
        Key::Ctrl('d') | Key::PageDown => {
            let r = ed.results.as_mut().unwrap();
            r.cursor = (r.cursor + ed.screen_rows / 2).min(r.entries.len().saturating_sub(1));
        }
        Key::Ctrl('u') | Key::PageUp => {
            let r = ed.results.as_mut().unwrap();
            r.cursor = r.cursor.saturating_sub(ed.screen_rows / 2);
        }
        Key::Char('g') => ed.results.as_mut().unwrap().cursor = 0,
        Key::Char('G') => {
            let r = ed.results.as_mut().unwrap();
            r.cursor = r.entries.len().saturating_sub(1);
        }
        Key::Char(':') => {
            ed.remember_results();
            ed.enter_command(crate::mode::CommandKind::Ex);
        }
        _ => {}
    }
}
impl Editor {
    pub fn show_results(&mut self, results: Results) {
        self.close_completion();
        self.pending.reset();
        self.results = Some(results);
        self.mode = Mode::Results;
    }
    pub fn remember_results(&mut self) {
        if let Some(r) = &self.results {
            if r.quickfix {
                self.quickfix = Some(r.clone());
            }
        }
    }
    pub fn export_quickfix(&mut self) {
        self.flush_pending_jk();
        let mut r = if let Some(c) = &self.completion {
            Results::new(
                "Completions",
                c.items
                    .iter()
                    .map(|i| {
                        let mut e = Entry::text(&i.label);
                        e.detail = i.detail.clone().unwrap_or_else(|| i.insert_text.clone());
                        e
                    })
                    .collect(),
            )
        } else if matches!(self.mode, Mode::Picker) {
            Results::new(
                "Files",
                self.file_picker
                    .as_ref()
                    .map(|p| {
                        p.matches
                            .iter()
                            .map(|(_, p)| {
                                Entry::location(self.project_root.join(p), 0, 0, p.clone())
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        } else if let Some(r) = &self.results {
            r.clone()
        } else if let Some(t) = &self.hover_text {
            Results::new("Hover", t.lines().map(Entry::text).collect())
        } else {
            self.diagnostic_results()
        };
        if !r.selected.is_empty() {
            r.entries = r
                .entries
                .into_iter()
                .enumerate()
                .filter(|(i, _)| r.selected.contains(i))
                .map(|(_, e)| e)
                .collect();
            r.cursor = 0;
            r.selected.clear();
        }
        if self.mode == Mode::Insert {
            crate::insert::leave_insert(self);
        }
        r.quickfix = true;
        r.live = false;
        r.busy = false;
        r.search_input = None;
        self.quickfix = Some(r.clone());
        self.show_results(r);
    }
    pub fn open_quickfix(&mut self) {
        if let Some(r) = self.quickfix.clone() {
            self.show_results(r);
        } else {
            self.set_message("Quickfix is empty — Ctrl-Q sends current results");
        }
    }
    pub fn quickfix_step(&mut self, forward: bool) {
        let Some(mut r) = self.quickfix.clone() else {
            return;
        };
        if r.entries.is_empty() {
            return;
        }
        r.cursor = if forward {
            (r.cursor + 1) % r.entries.len()
        } else {
            (r.cursor + r.entries.len() - 1) % r.entries.len()
        };
        self.results = Some(r.clone());
        self.quickfix = Some(r);
        self.open_result();
    }
    pub fn open_result(&mut self) {
        let Some(entry) = self
            .results
            .as_ref()
            .and_then(|r| r.entries.get(r.cursor))
            .cloned()
        else {
            return;
        };
        self.remember_results();
        if let Some(action) = entry.action {
            if let Some(id) = action.get("_vaayu_action_id").and_then(|v| v.as_str()) {
                self.enter_normal();
                if let Some(a) = crate::actions::find(id) {
                    (a.handler)(self);
                }
                return;
            }
            if action.get("_vaayu_git_patch").is_some() {
                self.stage_result(action);
                return;
            }
            if let Some(r) = action.get("_vaayu_spell_replace") {
                let line = r["line"].as_u64().unwrap_or(0) as usize;
                let start = r["start"].as_u64().unwrap_or(0) as usize;
                let end = r["end"].as_u64().unwrap_or(0) as usize;
                let replacement = r["replacement"].as_str().unwrap_or("").to_string();
                self.enter_normal();
                self.buf_mut().begin_edit();
                let s = self.buf().char_idx(line, start);
                let e = self.buf().char_idx(line, end);
                self.buf_mut().delete_char_range(s, e);
                self.buf_mut().insert_str_at(s, &replacement);
                self.buf_mut().commit_edit();
                self.set_cursor(line, start + replacement.chars().count());
                return;
            }
            if let Some(draft) = action.get("_vaayu_recovery") {
                self.restore_recovery(draft.clone());
            } else {
                self.apply_code_action(action);
            }
            return;
        }
        if let Some(id) = entry.buffer_id {
            if let Some(i) = self.buffers.iter().position(|b| b.id == id) {
                self.push_jump();
                self.cur = i;
                self.set_cursor(entry.line, entry.col);
                self.enter_normal();
                return;
            }
        }
        if let Some(path) = entry.path {
            self.jump_to(path, entry.line, entry.col);
            self.enter_normal();
        } else {
            self.set_message(entry.export(&self.project_root));
        }
    }
    pub fn show_buffers(&mut self) {
        let entries = self
            .buffers
            .iter()
            .map(|b| {
                let mut e = Entry::text(format!(
                    "{}{}",
                    b.name(),
                    if b.is_modified() { " [+]" } else { "" }
                ));
                e.path = b.path.clone();
                e.line = b.cursor_line;
                e.col = b.cursor_col;
                e.buffer_id = Some(b.id);
                e.note_id = b.note_id;
                e
            })
            .collect();
        self.show_results(Results::new("Buffers", entries));
    }
}
