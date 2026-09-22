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
            // Same dedup guard as `push_jump`: don't record the current
            // spot if it's already the last jump, otherwise the first
            // Ctrl-O would just land back where we already are.
            if self.jumps.last() != Some(&here) {
                self.jumps.push(here);
            }
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
        // Sort diagnostics themselves first, then build entries in that
        // order with no entries-level sort afterward -- a related-info
        // entry has to stay immediately after its own parent diagnostic,
        // which a later sort by (path, line, col) alone could separate
        // if another diagnostic's location happened to fall in between.
        let mut all: Vec<(&std::path::PathBuf, &crate::lsp::Diagnostic)> = self
            .diagnostics
            .iter()
            .flat_map(|(p, ds)| ds.iter().map(move |d| (p, d)))
            .collect();
        all.sort_by(|(ap, ad), (bp, bd)| (*ap, ad.line, ad.col).cmp(&(*bp, bd.line, bd.col)));
        for (p, d) in all {
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
                format!(
                    "{:?}{}: {}",
                    d.severity,
                    crate::lsp::code_source_label(d),
                    d.message
                ),
            ));
            // relatedInformation points at (possibly another) file's
            // location explaining *why* -- shown as its own,
            // separately-jumpable entry right after its parent
            // rather than folded into one row's text, since a
            // location worth mentioning is a location worth being
            // able to jump to.
            for r in d.raw["relatedInformation"].as_array().into_iter().flatten() {
                let Some(msg) = r["message"].as_str() else {
                    continue;
                };
                let rpath = r["location"]["uri"]
                    .as_str()
                    .and_then(crate::files::from_uri)
                    .unwrap_or_else(|| p.clone());
                let rline = r["location"]["range"]["start"]["line"]
                    .as_u64()
                    .unwrap_or(0) as usize;
                let raw_col = r["location"]["range"]["start"]["character"]
                    .as_u64()
                    .unwrap_or(0) as usize;
                let rtext = self
                    .buffers
                    .iter()
                    .find(|b| b.path.as_ref() == Some(&rpath))
                    .map(|b| b.line_text(rline))
                    .unwrap_or_default();
                let rcol = crate::language::utf16_to_col(&rtext, raw_col);
                entries.push(Entry::location(rpath, rline, rcol, format!("    ↳ {msg}")));
            }
        }
        Results::new("Diagnostics", entries)
    }
    pub fn next_diagnostic(&mut self, forward: bool) {
        let Some(p) = self.buf().path.clone() else {
            return;
        };
        let here = self.cursor();
        // Only *real* diagnostics for this file participate in `]d`/`[d`.
        // `diagnostic_results` interleaves relatedInformation rows that
        // point at arbitrary (and possibly out-of-order) lines, so build
        // the target list straight from `self.diagnostics` instead, and
        // sort it by (line, col) so "nearest following/preceding" is
        // well-defined regardless of the map's iteration order.
        let mut diags: Vec<(usize, usize, String)> = self
            .diagnostics
            .get(&p)
            .into_iter()
            .flatten()
            .map(|d| {
                let col = self
                    .buffers
                    .iter()
                    .find(|b| b.path.as_ref() == Some(&p))
                    .map(|b| crate::language::utf16_to_col(&b.line_text(d.line), d.col))
                    .unwrap_or(d.col);
                (
                    d.line,
                    col,
                    format!(
                        "{:?}{}: {}",
                        d.severity,
                        crate::lsp::code_source_label(d),
                        d.message
                    ),
                )
            })
            .collect();
        diags.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        let target = if forward {
            diags.iter().find(|d| (d.0, d.1) > here).or(diags.first())
        } else {
            diags.iter().rev().find(|d| (d.0, d.1) < here).or(diags.last())
        };
        if let Some((line, col, text)) = target.cloned() {
            self.push_jump();
            self.set_cursor(line, col);
            self.set_message(text);
        } else {
            self.set_message("No diagnostics in this buffer");
        }
    }
}
