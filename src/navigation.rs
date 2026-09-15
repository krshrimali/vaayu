use crate::{
    editor::Editor,
    results::{Entry, Results},
};
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub buffer: u64,
    pub path: Option<PathBuf>,
    pub line: usize,
    pub col: usize,
}
impl Editor {
    pub fn location(&self) -> Location {
        Location {
            buffer: self.buf().id,
            path: self.buf().path.clone(),
            line: self.cursor().0,
            col: self.cursor().1,
        }
    }
    pub fn push_jump(&mut self) {
        let here = self.location();
        if self.jump_index < self.jumps.len() {
            self.jumps.truncate(self.jump_index + 1);
        }
        if self.jumps.last() != Some(&here) {
            self.jumps.push(here);
        }
        if self.jumps.len() > 200 {
            self.jumps.remove(0);
        }
        self.jump_index = self.jumps.len();
    }
    pub fn goto_location(&mut self, l: Location) {
        if let Some(i) = self.buffers.iter().position(|b| b.id == l.buffer) {
            self.cur = i;
        } else if let Some(p) = l.path {
            if let Err(e) = self.open_file(p) {
                self.set_message(e.to_string());
                return;
            }
        } else {
            self.set_message("Jump buffer no longer exists");
            return;
        }
        self.set_cursor(l.line, l.col);
    }
    pub fn jump_to(&mut self, path: PathBuf, line: usize, col: usize) {
        self.push_jump();
        if let Err(e) = self.open_file(path) {
            self.set_message(e.to_string());
            return;
        }
        self.set_cursor(line, col);
    }
    pub fn jump_history(&mut self, forward: bool) {
        if self.jumps.is_empty() {
            return;
        }
        if self.jump_index == self.jumps.len() {
            let here = self.location();
            self.jumps.push(here);
            self.jump_index = self.jumps.len() - 1;
        }
        let next = if forward {
            (self.jump_index + 1).min(self.jumps.len() - 1)
        } else {
            self.jump_index.saturating_sub(1)
        };
        self.jump_index = next;
        self.goto_location(self.jumps[next].clone());
    }
    pub fn set_mark(&mut self, c: char) {
        self.marks.insert(c, self.location());
        self.set_message(format!("Mark {c} set"));
    }
    pub fn jump_mark(&mut self, c: char, exact: bool) {
        if let Some(l) = self.marks.get(&c).cloned() {
            self.push_jump();
            self.goto_location(l);
            if !exact {
                let line = self.cursor().0;
                self.set_cursor(line, self.buf().first_non_blank(line));
            }
        } else {
            self.set_message(format!("Mark {c} is not set"));
        }
    }
    pub fn diagnostic_results(&self) -> Results {
        let mut entries = Vec::new();
        for (p, ds) in &self.diagnostics {
            for d in ds {
                let col = self
                    .buffers
                    .iter()
                    .find(|b| b.path.as_ref() == Some(p))
                    .map(|b| crate::language::utf16_to_col(&b.line_text(d.line), d.col))
                    .unwrap_or(d.col);
                entries.push(Entry::location(
                    p.clone(),
                    d.line,
                    col,
                    format!("{:?}: {}", d.severity, d.message),
                ));
            }
        }
        entries.sort_by(|a, b| (&a.path, a.line, a.col).cmp(&(&b.path, b.line, b.col)));
        Results::new("Diagnostics", entries)
    }
    pub fn next_diagnostic(&mut self, forward: bool) {
        let r = self.diagnostic_results();
        let Some(p) = &self.buf().path else { return };
        let here = self.cursor();
        let entries: Vec<_> = r
            .entries
            .into_iter()
            .filter(|e| e.path.as_ref() == Some(p))
            .collect();
        let e = if forward {
            entries
                .iter()
                .find(|e| (e.line, e.col) > here)
                .or(entries.first())
        } else {
            entries
                .iter()
                .rev()
                .find(|e| (e.line, e.col) < here)
                .or(entries.last())
        };
        if let Some(e) = e {
            self.push_jump();
            self.set_cursor(e.line, e.col);
            self.set_message(&e.text);
        } else {
            self.set_message("No diagnostics in this buffer");
        }
    }
}
