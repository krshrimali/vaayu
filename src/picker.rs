use std::path::{Path, PathBuf};

use crate::editor::Editor;
use crate::key::Key;

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".cache", "__pycache__", ".venv"];
const MAX_FILES: usize = 40_000;

pub struct FilePicker {
    pub query: String,
    pub matches: Vec<(i64, String)>,
    pub selected: usize,
}

impl FilePicker {
    pub fn new(all_files: &[String]) -> FilePicker {
        let mut p = FilePicker { query: String::new(), matches: Vec::new(), selected: 0 };
        p.refilter(all_files);
        p
    }

    fn refilter(&mut self, all_files: &[String]) {
        if self.query.is_empty() {
            self.matches = all_files.iter().take(500).map(|f| (0, f.clone())).collect();
        } else {
            let mut scored: Vec<(i64, String)> = all_files
                .iter()
                .filter_map(|f| fuzzy_score(f, &self.query).map(|s| (s, f.clone())))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
            scored.truncate(500);
            self.matches = scored;
        }
        self.selected = 0;
    }
}

/// Case-insensitive subsequence fuzzy match. Returns None if `query` isn't a
/// subsequence of `candidate`; otherwise a score that rewards contiguous runs
/// and matches near the start (and especially near the last path segment).
fn fuzzy_score(candidate: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.chars().collect();
    let cand_lower: Vec<char> = candidate.to_lowercase().chars().collect();
    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let basename_start = candidate.rfind('/').map(|i| candidate[..i].chars().count() + 1).unwrap_or(0);

    let mut ci = 0usize;
    let mut score: i64 = 0;
    let mut last_match: Option<usize> = None;
    for &qc in &query_lower {
        let mut found = None;
        while ci < cand_lower.len() {
            if cand_lower[ci] == qc {
                found = Some(ci);
                break;
            }
            ci += 1;
        }
        let idx = found?;
        score += 10;
        if idx >= basename_start {
            score += 15;
        }
        if let Some(last) = last_match {
            if idx == last + 1 {
                score += 20;
            }
        } else {
            score += (20i64 - idx.min(20) as i64) / 2;
        }
        if cand[idx].is_uppercase() {
            score += 3;
        }
        last_match = Some(idx);
        ci = idx + 1;
    }
    score -= cand.len() as i64 / 10;
    Some(score)
}

pub fn scan_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Ok(ft) = entry.file_type() {
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    if SKIP_DIRS.contains(&name.as_ref()) {
                        continue;
                    }
                    stack.push(path);
                } else if ft.is_file() {
                    if let Ok(rel) = path.strip_prefix(root) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                    if out.len() >= MAX_FILES {
                        break;
                    }
                }
            }
        }
    }
    out.sort();
    out
}

pub fn handle(ed: &mut Editor, key: Key) {
    match key {
        Key::Esc => {
            ed.file_picker = None;
            ed.enter_normal();
            return;
        }
        Key::Enter => {
            let chosen = ed
                .file_picker
                .as_ref()
                .and_then(|p| p.matches.get(p.selected))
                .map(|(_, f)| f.clone());
            ed.file_picker = None;
            ed.enter_normal();
            if let Some(rel) = chosen {
                if let Err(e) = ed.open_file(PathBuf::from(rel)) {
                    ed.set_message(format!("could not open: {}", e));
                }
            }
            return;
        }
        Key::Backspace => {
            if let Some(p) = &mut ed.file_picker {
                p.query.pop();
                let all = ed.all_files.clone();
                if let Some(p) = &mut ed.file_picker {
                    p.refilter(&all);
                }
            }
            return;
        }
        Key::Char(c) => {
            if let Some(p) = &mut ed.file_picker {
                p.query.push(c);
            }
            let all = ed.all_files.clone();
            if let Some(p) = &mut ed.file_picker {
                p.refilter(&all);
            }
            return;
        }
        Key::Down | Key::Ctrl('n') => {
            if let Some(p) = &mut ed.file_picker {
                if p.selected + 1 < p.matches.len() {
                    p.selected += 1;
                }
            }
            return;
        }
        Key::Up | Key::Ctrl('p') => {
            if let Some(p) = &mut ed.file_picker {
                p.selected = p.selected.saturating_sub(1);
            }
            return;
        }
        _ => {}
    }
}
