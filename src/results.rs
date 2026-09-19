//! One searchable, selectable result model for every producer and quickfix.
use crate::{
    editor::{Editor, ResumeTarget},
    key::Key,
    mode::Mode,
};
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
    /// `p` toggles a file-content preview pane (for entries with a path)
    /// in place of the plain `detail`/`text` area.
    pub preview: bool,
    /// `w`, only meaningful while `preview` is on: long source lines wrap
    /// into extra preview rows instead of being clipped to one row each.
    pub preview_wrap: bool,
    /// Ctrl-e/Ctrl-y adjust this while `preview` is on, scrolling the
    /// preview window down/up without moving the list cursor. Reset to 0
    /// on every cursor move so it never carries over to an unrelated entry.
    pub preview_scroll: usize,
    /// The full, unfiltered list from the last producer call. `entries`
    /// (what's actually rendered/navigated) is re-derived from this by
    /// `apply_filter` -- the same `all_nodes`/`nodes` split
    /// `outline::Outline` already established, so every existing reader
    /// of `entries` throughout the codebase keeps working unchanged: it
    /// just sees a possibly-narrower list, with no call site needing to
    /// know filtering exists.
    pub all_entries: Vec<Entry>,
    /// `f` toggles editing this (case-insensitive substring against an
    /// entry's text/detail) -- including for a `live` list: a fresh
    /// batch of grep-as-you-type results is written into `all_entries`
    /// (see `Editor::poll_jobs`), then re-derived through `apply_filter`
    /// the same as any other producer's results, so this narrower
    /// substring filter keeps working across every new ripgrep query
    /// instead of only filtering whatever the last one happened to be.
    pub filter: String,
    pub filter_input: bool,
    /// Set only by `Editor::open_git_status`, the same "which specific
    /// producer is this" flag `quickfix`/`live` already establish --
    /// gates the `s`/`u`/`D`/`c`/`C`/`r` git-workspace keys in
    /// `results::handle` so they don't activate for and don't collide
    /// with any other Results list's own key meanings for those letters.
    pub git_status: bool,
}
impl Results {
    pub fn new(title: impl Into<String>, entries: Vec<Entry>) -> Self {
        Self {
            title: title.into(),
            all_entries: entries.clone(),
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
            preview: false,
            preview_wrap: false,
            preview_scroll: 0,
            filter: String::new(),
            filter_input: false,
            git_status: false,
        }
    }
    /// Re-derives the displayed `entries` from `all_entries` by
    /// case-insensitive substring match against `filter` (empty shows
    /// everything), clamping `cursor` and dropping `selected` outright --
    /// indices into the old, differently-sized `entries` can't be
    /// trusted to still mean the same thing after the list is narrowed
    /// or widened, the same invariant `Outline::apply_filter` already
    /// preserves for its own kind-filter.
    pub fn apply_filter(&mut self) {
        self.entries = if self.filter.is_empty() {
            self.all_entries.clone()
        } else {
            let needle = self.filter.to_lowercase();
            self.all_entries
                .iter()
                .filter(|e| {
                    e.text.to_lowercase().contains(&needle)
                        || e.detail.to_lowercase().contains(&needle)
                })
                .cloned()
                .collect()
        };
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        self.selected.clear();
        self.preview_scroll = 0;
    }
    /// Moves the list cursor, resetting any preview scroll -- a scroll
    /// offset from one entry's preview should never leak into another's.
    pub fn move_cursor(&mut self, new: usize) {
        self.cursor = new;
        self.preview_scroll = 0;
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
    /// Builds the rows for the file-content preview pane: up to `rows`
    /// terminal rows, starting `context_before` lines above the current
    /// entry's line (further adjusted by `preview_scroll`), each optionally
    /// wrapped to `width` columns. `source` is the target file's lines,
    /// already resolved by the caller (open buffer or disk). Pure and
    /// terminal-independent so it's unit-testable without a real screen;
    /// `render.rs` only turns the result into painted rows. Returns `None`
    /// when preview is off or the current entry has no path (nothing to
    /// preview -- the caller falls back to the plain `detail`/`text` area).
    pub fn preview_rows(
        &self,
        source: &[String],
        rows: usize,
        width: usize,
        context_before: usize,
    ) -> Option<Vec<PreviewRow>> {
        if !self.preview {
            return None;
        }
        let e = self.entries.get(self.cursor)?;
        e.path.as_ref()?;
        let start = e.line.saturating_sub(context_before) + self.preview_scroll;
        let mut out = Vec::new();
        let mut line_no = start;
        while out.len() < rows && line_no < source.len() {
            let is_match = line_no == e.line;
            let chunks = if self.preview_wrap {
                wrap_chunks(&source[line_no], width)
            } else {
                vec![source[line_no].clone()]
            };
            for c in chunks {
                if out.len() >= rows {
                    break;
                }
                out.push(PreviewRow { text: c, is_match });
            }
            line_no += 1;
        }
        Some(out)
    }
}
/// One ready-to-paint row of `Results::preview_rows` output: its text
/// (already wrapped if requested) and whether it belongs to the entry's
/// own matched source line, for highlighting.
pub struct PreviewRow {
    pub text: String,
    pub is_match: bool,
}
fn wrap_chunks(s: &str, width: usize) -> Vec<String> {
    if width == 0 || s.is_empty() {
        return vec![s.to_string()];
    }
    s.chars()
        .collect::<Vec<_>>()
        .chunks(width)
        .map(|c| c.iter().collect())
        .collect()
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
    if ed.results.as_ref().unwrap().filter_input {
        let r = ed.results.as_mut().unwrap();
        match key {
            Key::Esc | Key::Enter => r.filter_input = false,
            Key::Backspace => {
                r.filter.pop();
                r.apply_filter();
            }
            Key::Char(c) => {
                r.filter.push(c);
                r.apply_filter();
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
        Key::Ctrl('v') => ed.open_result_split(true),
        Key::Ctrl('x') => ed.open_result_split(false),
        Key::Ctrl('t') => ed.open_result_tab(),
        Key::Char('A') => ed.run_review(),
        Key::Char('R') => ed.resolve_review(),
        Key::Char('s') if ed.results.as_ref().unwrap().git_status => ed.git_status_stage(),
        Key::Char('u') if ed.results.as_ref().unwrap().git_status => ed.git_status_unstage(),
        Key::Char('D') if ed.results.as_ref().unwrap().git_status => ed.git_status_discard_prompt(),
        Key::Char('c') if ed.results.as_ref().unwrap().git_status => {
            ed.git_status_commit_prompt(false)
        }
        Key::Char('C') if ed.results.as_ref().unwrap().git_status => {
            ed.git_status_commit_prompt(true)
        }
        Key::Char('r') if ed.results.as_ref().unwrap().git_status => ed.open_git_status(),
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
        Key::Char('P') => ed.permalink_from_results_entry(),
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
        Key::Char('p') => {
            let r = ed.results.as_mut().unwrap();
            r.preview = !r.preview;
            r.preview_scroll = 0;
        }
        Key::Char('w') if ed.results.as_ref().unwrap().preview => {
            ed.results.as_mut().unwrap().preview_wrap ^= true;
        }
        Key::Ctrl('e') if ed.results.as_ref().unwrap().preview => {
            ed.results.as_mut().unwrap().preview_scroll += 1;
        }
        Key::Ctrl('y') if ed.results.as_ref().unwrap().preview => {
            let r = ed.results.as_mut().unwrap();
            r.preview_scroll = r.preview_scroll.saturating_sub(1);
        }
        Key::Char('f') => {
            let r = ed.results.as_mut().unwrap();
            r.filter.clear();
            r.apply_filter();
            r.filter_input = true;
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
                let next = (r.cursor + 1).min(r.entries.len() - 1);
                r.move_cursor(next);
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
            let next = (r.cursor + 1).min(r.entries.len().saturating_sub(1));
            r.move_cursor(next);
        }
        Key::Char('k') | Key::Up | Key::Ctrl('p') => {
            let r = ed.results.as_mut().unwrap();
            let next = r.cursor.saturating_sub(1);
            r.move_cursor(next);
        }
        Key::Ctrl('d') | Key::PageDown => {
            let r = ed.results.as_mut().unwrap();
            let next = (r.cursor + ed.screen_rows / 2).min(r.entries.len().saturating_sub(1));
            r.move_cursor(next);
        }
        Key::Ctrl('u') | Key::PageUp => {
            let r = ed.results.as_mut().unwrap();
            let next = r.cursor.saturating_sub(ed.screen_rows / 2);
            r.move_cursor(next);
        }
        Key::Char('g') => {
            let r = ed.results.as_mut().unwrap();
            r.move_cursor(0);
        }
        Key::Char('G') => {
            let r = ed.results.as_mut().unwrap();
            let last = r.entries.len().saturating_sub(1);
            r.move_cursor(last);
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
    /// Source lines for the Results/file-picker preview pane: from the
    /// matching open buffer if there is one (so unsaved edits show up in
    /// the preview, same reasoning as `language.rs`'s outline
    /// column-correction fallback), else read fresh from disk -- refused
    /// past a generous size bound (an open buffer, already fully in
    /// memory regardless, is exempt) so a preview pane can't be made to
    /// read an arbitrarily large file into memory just by fuzzy-matching
    /// or grep-matching it; the caller (`Results::preview_rows`) already
    /// degrades gracefully when `source` doesn't reach as far as the
    /// entry's own line.
    pub(crate) fn preview_source_lines(&self, path: &std::path::Path) -> Vec<String> {
        if let Some(b) = self
            .buffers
            .iter()
            .find(|b| b.path.as_deref() == Some(path))
        {
            return (0..b.rope.len_lines()).map(|i| b.line_text(i)).collect();
        }
        const MAX_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;
        if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_PREVIEW_BYTES) {
            return vec!["(file too large to preview)".to_string()];
        }
        std::fs::read_to_string(path)
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }
    /// Called at every point a Results list is dismissed or acted on
    /// (Esc/q, opening a location, entering `:`). Preserves it to
    /// `quickfix` if it's flagged as one, and marks it as the most
    /// recently active resumable session (`self.results` itself is never
    /// cleared, so ":resume" just needs to know it should switch back).
    pub fn remember_results(&mut self) {
        if let Some(r) = &self.results {
            if r.quickfix {
                self.quickfix = Some(r.clone());
                // Revisiting/dismissing the current list updates its
                // history slot in place (so cursor/selection changes
                // stick) rather than growing history -- only a genuinely
                // new list (export_quickfix) does that.
                if let Some(slot) = self.quickfix_history.get_mut(self.quickfix_history_pos) {
                    *slot = r.clone();
                }
            }
            self.last_resume = Some(ResumeTarget::Results);
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
        if !self.quickfix_history.is_empty() {
            self.quickfix_history
                .truncate(self.quickfix_history_pos + 1);
        }
        self.quickfix_history.push(r.clone());
        self.quickfix_history_pos = self.quickfix_history.len() - 1;
        self.show_results(r);
    }
    /// `:colder`: switches to the previous (older) quickfix list.
    pub fn quickfix_older(&mut self) {
        if self.quickfix_history_pos == 0 || self.quickfix_history.is_empty() {
            self.set_message("Already at the oldest quickfix list");
            return;
        }
        self.quickfix_history_pos -= 1;
        let r = self.quickfix_history[self.quickfix_history_pos].clone();
        self.quickfix = Some(r.clone());
        self.show_results(r);
    }
    /// `:cnewer`: switches to the next (newer) quickfix list.
    pub fn quickfix_newer(&mut self) {
        if self.quickfix_history.is_empty()
            || self.quickfix_history_pos + 1 >= self.quickfix_history.len()
        {
            self.set_message("Already at the newest quickfix list");
            return;
        }
        self.quickfix_history_pos += 1;
        let r = self.quickfix_history[self.quickfix_history_pos].clone();
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
        self.quickfix = Some(r.clone());
        if let Some(slot) = self.quickfix_history.get_mut(self.quickfix_history_pos) {
            *slot = r;
        }
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
            if let Some(cmd) = action.get("_vaayu_rerun_ex").and_then(|v| v.as_str()) {
                self.enter_normal();
                crate::command::run_ex(self, cmd);
                return;
            }
            // `:commands`: pre-fills the command line rather than
            // executing immediately like `_vaayu_rerun_ex` -- most
            // commands need arguments the browsing list can't supply
            // (a bare `:rename` or `:grep` would just be a confusing
            // no-op), so this lets the user add them before Enter.
            if let Some(cmd) = action.get("_vaayu_prefill_ex").and_then(|v| v.as_str()) {
                self.enter_normal();
                self.enter_command(crate::mode::CommandKind::Ex);
                self.cmdline = cmd.to_string();
                return;
            }
            if let Some(pat) = action.get("_vaayu_rerun_search").and_then(|v| v.as_str()) {
                self.enter_normal();
                crate::command::run_search(self, pat, true);
                return;
            }
            // `:documentlinks`: a `file://` target opens directly (same
            // as jumping to any other location); anything else (http(s),
            // an unresolvable scheme) is copied to the clipboard/+
            // register instead of opened -- same "generate/copy, never
            // open a browser" choice already made for `,gp` permalinks.
            if let Some(target) = action.get("_vaayu_open_link").and_then(|v| v.as_str()) {
                self.enter_normal();
                match crate::files::from_uri(target) {
                    Some(path) => {
                        if let Err(e) = self.open_file(path) {
                            self.set_message(format!("Could not open link: {e}"));
                        }
                    }
                    None => {
                        self.registers.set(Some('+'), target.to_string(), false);
                        self.set_message(format!("Copied link: {target}"));
                    }
                }
                return;
            }
            if let Some(root) = action.get("_vaayu_switch_project").and_then(|v| v.as_str()) {
                self.enter_normal();
                self.switch_project(std::path::PathBuf::from(root));
                return;
            }
            if let Some(p) = action.get("_vaayu_tree_bookmark").and_then(|v| v.as_str()) {
                self.enter_normal();
                let is_dir = action.get("dir").and_then(|v| v.as_bool()).unwrap_or(false);
                self.open_tree_bookmark(&PathBuf::from(p), is_dir);
                return;
            }
            if let Some(stash_ref) = action.get("_vaayu_git_stash_show").and_then(|v| v.as_str()) {
                self.enter_normal();
                self.show_git_stash_diff(stash_ref);
                return;
            }
            if let Some(v) = action.get("_vaayu_git_hunk_reset") {
                self.enter_normal();
                self.apply_hunk_reset(v);
                return;
            }
            if let Some(v) = action.get("_vaayu_git_hunk_reset_range") {
                self.enter_normal();
                self.apply_hunk_reset_range(v);
                return;
            }
            if let Some(v) = action.get("_vaayu_git_status_entry") {
                self.enter_normal();
                if let Ok(path) = serde_json::from_value::<PathBuf>(v["path"].clone()) {
                    self.jump_to(path, 0, 0);
                }
                return;
            }
            if let Some(v) = action.get("_vaayu_git_status_discard") {
                self.enter_normal();
                self.apply_git_status_discard(v);
                return;
            }
            if let Some(hash) = action
                .get("_vaayu_git_show_commit")
                .and_then(|v| v.as_str())
            {
                self.enter_normal();
                self.show_commit_diff(hash);
                return;
            }
            if let Some(name) = action.get("_vaayu_git_checkout").and_then(|v| v.as_str()) {
                self.enter_normal();
                self.checkout_branch(name);
                return;
            }
            if let Some(v) = action.get("_vaayu_tool_install") {
                self.enter_normal();
                if v["installed"] == true {
                    self.set_message(format!(
                        "{} is already installed",
                        v["name"].as_str().unwrap_or("tool")
                    ));
                } else if let Some(cmd) = v["install"].as_str() {
                    self.run_tool_install(cmd);
                }
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
                if i != self.cur {
                    self.note_alternate_buffer();
                }
                self.cur = i;
                self.touch_buffer_mru(id);
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

    /// Like `open_result`, but opens a location entry into a new split
    /// instead of the current pane. Action/text entries (nothing to
    /// "split" into) fall back to plain `open_result`.
    pub fn open_result_split(&mut self, vertical: bool) {
        let Some(entry) = self
            .results
            .as_ref()
            .and_then(|r| r.entries.get(r.cursor))
            .cloned()
        else {
            return;
        };
        if entry.action.is_some() {
            self.open_result();
            return;
        }
        self.remember_results();
        if let Some(id) = entry.buffer_id {
            if let Some(i) = self.buffers.iter().position(|b| b.id == id) {
                self.push_jump();
                self.split_window(vertical, false);
                if i != self.cur {
                    self.note_alternate_buffer();
                }
                self.cur = i;
                self.touch_buffer_mru(id);
                self.set_cursor(entry.line, entry.col);
                self.enter_normal();
                return;
            }
        }
        if let Some(path) = entry.path {
            self.split_window(vertical, false);
            self.jump_to(path, entry.line, entry.col);
            self.enter_normal();
        } else {
            self.set_message(entry.export(&self.project_root));
        }
    }
    /// Like `open_result`, but opens a location entry into a brand-new tab
    /// instead of the current pane, mirroring `open_result_split`'s
    /// buffer_id vs. path handling.
    pub fn open_result_tab(&mut self) {
        let Some(entry) = self
            .results
            .as_ref()
            .and_then(|r| r.entries.get(r.cursor))
            .cloned()
        else {
            return;
        };
        if entry.action.is_some() {
            self.open_result();
            return;
        }
        self.remember_results();
        if let Some(id) = entry.buffer_id {
            if let Some(i) = self.buffers.iter().position(|b| b.id == id) {
                self.push_jump();
                self.new_tab();
                if i != self.cur {
                    self.note_alternate_buffer();
                }
                self.cur = i;
                self.touch_buffer_mru(id);
                self.set_cursor(entry.line, entry.col);
                self.enter_normal();
                return;
            }
        }
        if let Some(path) = entry.path {
            self.new_tab();
            self.jump_to(path, entry.line, entry.col);
            self.enter_normal();
        } else {
            self.set_message(entry.export(&self.project_root));
        }
    }
    /// `,b`/`:buffer`'s ordering: most-recently-activated first (see
    /// `Editor::buffer_mru`), then any buffer `buffer_mru` never recorded
    /// (e.g. one only ever reached via session-restore/pane-focus
    /// bookkeeping, not a genuine user switch) in its natural order --
    /// every open buffer appears exactly once either way.
    fn buffers_in_mru_order(&self) -> Vec<&crate::buffer::Buffer> {
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<&crate::buffer::Buffer> = self
            .buffer_mru
            .iter()
            .filter_map(|id| self.buffers.iter().find(|b| b.id == *id))
            .filter(|b| seen.insert(b.id))
            .collect();
        out.extend(self.buffers.iter().filter(|b| !seen.contains(&b.id)));
        out
    }
    pub fn show_buffers(&mut self) {
        let entries = self
            .buffers_in_mru_order()
            .into_iter()
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
    /// `:blines`: every line in the current buffer as a jump-to-line
    /// picker source, matching this plan's "current-buffer lines" item.
    /// Blank lines are skipped (nothing to search or usefully jump to
    /// that isn't already one `j`/`k` away), matching fzf.vim/Telescope's
    /// own current-buffer-lines behavior.
    pub fn show_buffer_lines(&mut self) {
        let id = self.buf().id;
        let entries = (0..self.buf().line_count())
            .filter_map(|line| {
                let text = self.buf().line_text(line);
                if text.trim().is_empty() {
                    return None;
                }
                let mut e = Entry::text(format!("{:>5} {}", line + 1, text));
                e.buffer_id = Some(id);
                e.line = line;
                Some(e)
            })
            .collect();
        self.show_results(Results::new("Buffer lines", entries));
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn results_with_entry(line: usize) -> Results {
        let mut r = Results::new("t", vec![Entry::location("f.rs".into(), line, 0, "hit")]);
        r.preview = true;
        r
    }

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line{i}")).collect()
    }

    #[test]
    fn preview_rows_is_none_when_preview_is_off() {
        let mut r = results_with_entry(5);
        r.preview = false;
        assert!(r.preview_rows(&lines(10), 4, 80, 1).is_none());
    }

    #[test]
    fn preview_rows_is_none_without_a_path() {
        let mut r = Results::new("t", vec![Entry::text("no path")]);
        r.preview = true;
        assert!(r.preview_rows(&lines(10), 4, 80, 1).is_none());
    }

    #[test]
    fn preview_rows_centers_on_the_entry_line_and_marks_the_match() {
        let r = results_with_entry(5);
        let rows = r.preview_rows(&lines(10), 4, 80, 1).unwrap();
        // context_before=1: starts at line 4, four rows -> lines 4..8
        let texts: Vec<_> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, vec!["line4", "line5", "line6", "line7"]);
        assert_eq!(
            rows.iter().filter(|row| row.is_match).count(),
            1,
            "exactly the entry's own line should be marked as the match"
        );
        assert_eq!(rows[1].text, "line5");
        assert!(rows[1].is_match);
    }

    #[test]
    fn preview_scroll_shifts_the_window_down() {
        let mut r = results_with_entry(5);
        r.preview_scroll = 2;
        let rows = r.preview_rows(&lines(10), 3, 80, 1).unwrap();
        let texts: Vec<_> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, vec!["line6", "line7", "line8"]);
    }

    #[test]
    fn preview_rows_stops_at_the_end_of_the_source_without_padding() {
        let r = results_with_entry(9);
        let rows = r.preview_rows(&lines(10), 6, 80, 1).unwrap();
        // context_before=1 starts at line 8; only lines 8 and 9 exist.
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn preview_wrap_splits_a_long_line_into_multiple_rows_sharing_is_match() {
        let mut r = results_with_entry(0);
        r.preview_wrap = true;
        let source = vec!["x".repeat(25)];
        let rows = r.preview_rows(&source, 10, 10, 0).unwrap();
        assert_eq!(rows.len(), 3, "25 chars at width 10 wraps into 3 rows");
        assert!(rows.iter().all(|row| row.is_match));
        assert_eq!(rows[0].text.len(), 10);
        assert_eq!(rows[2].text.len(), 5);
    }

    #[test]
    fn move_cursor_resets_preview_scroll() {
        let mut r = results_with_entry(5);
        r.preview_scroll = 3;
        r.move_cursor(0);
        assert_eq!(r.cursor, 0);
        assert_eq!(r.preview_scroll, 0);
    }

    #[test]
    fn apply_filter_narrows_entries_by_a_case_insensitive_substring() {
        let mut r = Results::new(
            "t",
            vec![
                Entry::text("alpha item"),
                Entry::text("beta item"),
                Entry::text("gamma other"),
            ],
        );
        r.filter = "ALPHA".into();
        r.apply_filter();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].text, "alpha item");
        // The full backing set is untouched.
        assert_eq!(r.all_entries.len(), 3);
    }

    #[test]
    fn apply_filter_also_matches_detail_text() {
        let mut e = Entry::text("summary");
        e.detail = "needle in the haystack".into();
        let mut r = Results::new("t", vec![e, Entry::text("unrelated")]);
        r.filter = "needle".into();
        r.apply_filter();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].text, "summary");
    }

    #[test]
    fn apply_filter_empty_shows_everything_again() {
        let mut r = Results::new("t", vec![Entry::text("a"), Entry::text("b")]);
        r.filter = "a".into();
        r.apply_filter();
        assert_eq!(r.entries.len(), 1);
        r.filter.clear();
        r.apply_filter();
        assert_eq!(
            r.entries.len(),
            2,
            "clearing the filter restores everything"
        );
    }

    #[test]
    fn apply_filter_clamps_cursor_and_drops_stale_selection() {
        let mut r = Results::new(
            "t",
            vec![Entry::text("a"), Entry::text("bb"), Entry::text("ccc")],
        );
        r.cursor = 2;
        r.selected.insert(0);
        r.selected.insert(2);
        // Narrows to exactly one entry, at an index lower than the stale
        // cursor, to prove clamping actually moves it.
        r.filter = "bb".into();
        r.apply_filter();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.cursor, 0, "cursor should clamp into the narrowed list");
        assert!(
            r.selected.is_empty(),
            "stale selection indices must not survive a filter change"
        );
    }
}
