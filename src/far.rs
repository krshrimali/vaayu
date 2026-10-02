//! `:far` / `,sr`: a reviewed project-wide search & replace screen -- the
//! grug-far workflow. One screen holds a search pattern, a replacement and
//! a file-glob filter; every edit re-scans the project in the background
//! and lists each match, grouped by file, as the line it would become.
//! Matches (or whole files) toggle on and off individually, and `R`
//! applies only the selected ones: each touched file's buffer gets a
//! single undo step (`u` in that buffer reverts it, `U` here reverts the
//! whole last apply) and is saved, unless it already had unsaved edits.
//!
//! Unlike `:cfar`, which rewrites every file in a prior `:grep` list, this
//! scans the project file index itself, so there is no separate grep step
//! and the regex dialect is the same one `:s` uses (Vim syntax translated
//! by `vimregex`, `\1`/`&` in the replacement) for both finding and
//! replacing. Matching is per line, like `:s` without a range.
use crate::{editor::Editor, key::Key, mode::Mode, queryline::QueryCursor};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

/// Matches listed before the scan stops and asks for a narrower query --
/// generous for a reviewed replace, small enough to stay responsive.
pub const MAX_MATCHES: usize = 10_000;
/// Files above this size are skipped by the scan (an open buffer's text is
/// used whatever its size, the same exemption `preview_source_lines` makes).
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Quiet period after the last keystroke in a field before re-scanning.
const DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Search,
    Replace,
    Files,
}

impl Field {
    pub const ALL: [Field; 3] = [Field::Search, Field::Replace, Field::Files];
    pub fn label(self) -> &'static str {
        match self {
            Field::Search => "Search: ",
            Field::Replace => "Replace:",
            Field::Files => "Files:  ",
        }
    }
    fn next(self) -> Field {
        match self {
            Field::Search => Field::Replace,
            Field::Replace => Field::Files,
            Field::Files => Field::Search,
        }
    }
    fn prev(self) -> Field {
        self.next().next()
    }
}

/// `c` cycles this: `Auto` follows `ignorecase`/`smartcase` like `:s`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
    #[default]
    Auto,
    Ignore,
    Match,
}

impl Case {
    pub fn label(self) -> &'static str {
        match self {
            Case::Auto => "case: auto",
            Case::Ignore => "case: ignore",
            Case::Match => "case: match",
        }
    }
    fn next(self) -> Case {
        match self {
            Case::Auto => Case::Ignore,
            Case::Ignore => Case::Match,
            Case::Match => Case::Auto,
        }
    }
}

/// One occurrence: byte range `start..end` within `old` (the whole line as
/// scanned) and its already-expanded replacement text.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    pub old: String,
}

impl Match {
    /// 0-based char column of the match, for jumping to it.
    pub fn col(&self) -> usize {
        self.old[..self.start].chars().count()
    }
    /// The line with only this occurrence replaced -- what each list row
    /// previews.
    pub fn new_line(&self) -> String {
        format!(
            "{}{}{}",
            &self.old[..self.start],
            self.replacement,
            &self.old[self.end..]
        )
    }
}

#[derive(Clone, Debug)]
pub struct FileMatches {
    pub path: PathBuf,
    pub matches: Vec<Match>,
}

/// One row of the flattened list: a file header or one of its matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    File(usize),
    Match(usize, usize),
}

/// A finished background scan.
struct Scan {
    files: Vec<FileMatches>,
    truncated: bool,
    error: Option<String>,
}

/// One buffer an apply changed, so `U` can revert exactly that step --
/// only while the buffer is still at `edit_seq` (nothing edited it since).
struct Applied {
    buffer_id: u64,
    edit_seq: u64,
    saved: bool,
}

pub struct Far {
    pub search: String,
    pub replace: String,
    pub glob: String,
    /// `F`: the search is a literal string and the replacement is inserted
    /// verbatim (no `\1`/`&` expansion).
    pub fixed: bool,
    pub case: Case,
    /// The field being typed into, or `None` while the match list has focus.
    pub editing: Option<Field>,
    /// The field `i` returns to.
    pub field: Field,
    pub qcursor: QueryCursor,
    pub files: Vec<FileMatches>,
    /// Matches toggled off, keyed by `(path, line, start byte)` rather than
    /// stored on each `Match` so a toggle survives the re-scan every edit
    /// of the replacement text triggers.
    pub disabled: HashSet<(PathBuf, usize, usize)>,
    /// Index into `rows()`.
    pub cursor: usize,
    pub busy: bool,
    pub truncated: bool,
    pub error: Option<String>,
    last_apply: Vec<Applied>,
    due: Option<Instant>,
    generation: Arc<AtomicU64>,
    rx: Option<mpsc::Receiver<(u64, Scan)>>,
}

impl Far {
    pub fn new() -> Far {
        Far {
            search: String::new(),
            replace: String::new(),
            glob: String::new(),
            fixed: false,
            case: Case::Auto,
            editing: Some(Field::Search),
            field: Field::Search,
            qcursor: QueryCursor::default(),
            files: Vec::new(),
            disabled: HashSet::new(),
            cursor: 0,
            busy: false,
            truncated: false,
            error: None,
            last_apply: Vec::new(),
            due: None,
            generation: Arc::new(AtomicU64::new(0)),
            rx: None,
        }
    }

    pub fn text(&self, field: Field) -> &str {
        match field {
            Field::Search => &self.search,
            Field::Replace => &self.replace,
            Field::Files => &self.glob,
        }
    }

    fn text_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Search => &mut self.search,
            Field::Replace => &mut self.replace,
            Field::Files => &mut self.glob,
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (fi, f) in self.files.iter().enumerate() {
            rows.push(Row::File(fi));
            rows.extend((0..f.matches.len()).map(|mi| Row::Match(fi, mi)));
        }
        rows
    }

    pub fn match_count(&self) -> usize {
        self.files.iter().map(|f| f.matches.len()).sum()
    }

    pub fn is_enabled(&self, path: &Path, m: &Match) -> bool {
        !self
            .disabled
            .contains(&(path.to_path_buf(), m.line, m.start))
    }

    /// How many of file `fi`'s matches are selected.
    pub fn enabled_in(&self, fi: usize) -> usize {
        let f = &self.files[fi];
        f.matches
            .iter()
            .filter(|m| self.is_enabled(&f.path, m))
            .count()
    }

    pub fn enabled_count(&self) -> usize {
        (0..self.files.len()).map(|fi| self.enabled_in(fi)).sum()
    }

    fn set_enabled(&mut self, fi: usize, mi: usize, on: bool) {
        let f = &self.files[fi];
        let key = (f.path.clone(), f.matches[mi].line, f.matches[mi].start);
        if on {
            self.disabled.remove(&key);
        } else {
            self.disabled.insert(key);
        }
    }

    /// Toggles the row under the cursor: one match, or -- on a file row --
    /// all of that file's matches (on unless every one already is).
    pub fn toggle(&mut self) {
        match self.rows().get(self.cursor).copied() {
            Some(Row::Match(fi, mi)) => {
                let on = !self.is_enabled(&self.files[fi].path, &self.files[fi].matches[mi]);
                self.set_enabled(fi, mi, on);
            }
            Some(Row::File(fi)) => {
                let on = self.enabled_in(fi) < self.files[fi].matches.len();
                for mi in 0..self.files[fi].matches.len() {
                    self.set_enabled(fi, mi, on);
                }
            }
            None => {}
        }
    }

    /// `a`: selects everything, or nothing when everything already is.
    pub fn toggle_all(&mut self) {
        let on = self.enabled_count() < self.match_count();
        for fi in 0..self.files.len() {
            for mi in 0..self.files[fi].matches.len() {
                self.set_enabled(fi, mi, on);
            }
        }
    }

    /// The selected matches, per file and per line, ready to apply.
    fn plan(&self) -> Vec<(PathBuf, BTreeMap<usize, Vec<Match>>)> {
        let mut plan = Vec::new();
        for f in &self.files {
            let mut lines: BTreeMap<usize, Vec<Match>> = BTreeMap::new();
            for m in f.matches.iter().filter(|m| self.is_enabled(&f.path, m)) {
                lines.entry(m.line).or_default().push(m.clone());
            }
            if !lines.is_empty() {
                plan.push((f.path.clone(), lines));
            }
        }
        plan
    }

    /// Moves to the next (`forward`) or previous file header row.
    fn jump_file(&mut self, forward: bool) {
        let rows = self.rows();
        let headers = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Row::File(_)))
            .map(|(i, _)| i);
        let target = if forward {
            headers.filter(|&i| i > self.cursor).min()
        } else {
            headers.filter(|&i| i < self.cursor).max()
        };
        if let Some(i) = target {
            self.cursor = i;
        }
    }

    fn schedule(&mut self, delay: bool) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.due = Some(if delay {
            Instant::now() + DEBOUNCE
        } else {
            Instant::now()
        });
        self.busy = true;
    }
}

impl Default for Far {
    fn default() -> Self {
        Far::new()
    }
}

/// Compiles the search field into a regex and the replacement field into
/// `Captures::expand` syntax, mirroring `:s`'s translation and case rules.
pub fn compile(
    search: &str,
    replace: &str,
    fixed: bool,
    case: Case,
    ignorecase: bool,
    smartcase: bool,
) -> Result<(fancy_regex::Regex, String), String> {
    let (pattern, replacement) = if fixed {
        (
            fancy_regex::escape(search).into_owned(),
            replace.replace('$', "$$"),
        )
    } else {
        (
            crate::vimregex::translate_pattern(search),
            crate::vimregex::translate_replacement(replace),
        )
    };
    let insensitive = match case {
        Case::Ignore => true,
        Case::Match => false,
        Case::Auto => ignorecase && !(smartcase && search.chars().any(char::is_uppercase)),
    };
    let re = fancy_regex::RegexBuilder::new(&pattern)
        .backtrack_limit(100_000)
        .case_insensitive(insensitive)
        .build()
        .map_err(|e| format!("bad pattern: {e}"))?;
    Ok((re, replacement))
}

/// Appends every match in `text` to `out`, line by line, stopping once
/// `out` holds `limit` matches. Returns whether it stopped early.
pub fn scan_text(
    text: &str,
    re: &fancy_regex::Regex,
    replacement: &str,
    limit: usize,
    out: &mut Vec<Match>,
) -> bool {
    for (line, old) in text.lines().enumerate() {
        for caps in re.captures_iter(old) {
            let Ok(caps) = caps else { break };
            let Some(m) = caps.get(0) else { continue };
            if out.len() >= limit {
                return true;
            }
            let mut expanded = String::new();
            caps.expand(replacement, &mut expanded);
            out.push(Match {
                line,
                start: m.start(),
                end: m.end(),
                replacement: expanded,
                old: old.to_string(),
            });
        }
    }
    false
}

/// `line` with each of `matches` (sorted, non-overlapping byte ranges into
/// it) replaced.
pub fn replace_line(line: &str, matches: &[Match]) -> String {
    let mut out = String::new();
    let mut at = 0;
    for m in matches {
        out.push_str(&line[at..m.start]);
        out.push_str(&m.replacement);
        at = m.end;
    }
    out.push_str(&line[at..]);
    out
}

/// The `Files:` field: whitespace- or comma-separated globs, `!` to
/// exclude. A glob without `/` matches a file name at any depth (`*.rs`);
/// one with `/` matches from the project root (`src/**/*.rs`). Every glob
/// also matches everything under a directory it names (`src`, `docs/`).
/// `*` and `?` stay within one path component, `**` crosses them and
/// `{a,b}` is an alternation. No include globs means every file.
#[derive(Debug, Default)]
pub struct Globs {
    include: Vec<regex::Regex>,
    exclude: Vec<regex::Regex>,
}

impl Globs {
    pub fn parse(spec: &str) -> Result<Globs, String> {
        let mut globs = Globs::default();
        for word in split_globs(spec) {
            let (negate, glob) = match word.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, word.as_str()),
            };
            if glob.is_empty() {
                continue;
            }
            let re = regex::Regex::new(&glob_regex(glob))
                .map_err(|_| format!("bad file glob: {word}"))?;
            if negate {
                globs.exclude.push(re);
            } else {
                globs.include.push(re);
            }
        }
        Ok(globs)
    }

    /// `rel` is a `/`-separated path relative to the project root.
    pub fn matches(&self, rel: &str) -> bool {
        (self.include.is_empty() || self.include.iter().any(|r| r.is_match(rel)))
            && !self.exclude.iter().any(|r| r.is_match(rel))
    }
}

/// Splits on whitespace, and on commas outside `{...}`.
fn split_globs(spec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in spec.chars() {
        match c {
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            c if c.is_whitespace() => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.retain(|s| !s.is_empty());
    out
}

fn glob_regex(glob: &str) -> String {
    let glob = glob.trim_start_matches("./");
    let anchored = glob.trim_end_matches('/').contains('/');
    let glob = glob.trim_start_matches('/').trim_end_matches('/');
    let mut re = String::from(if anchored { "^" } else { "(?:^|/)" });
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                if chars.get(i + 2) == Some(&'/') {
                    re.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    re.push_str(".*");
                    i += 2;
                }
                continue;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '{' => re.push_str("(?:"),
            '}' => re.push(')'),
            ',' => re.push('|'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push_str("(?:/.*)?$");
    re
}

/// The background half of a scan: every indexed file the globs admit,
/// read from its open buffer if there is one, else from disk (skipping
/// large, binary and non-UTF-8 files). Gives up early once `generation`
/// moves past `id` (a newer keystroke superseded this scan).
#[allow(clippy::too_many_arguments)]
fn run_scan(
    root: &Path,
    files: Vec<String>,
    open: HashMap<PathBuf, String>,
    re: &fancy_regex::Regex,
    replacement: &str,
    globs: &Globs,
    generation: &AtomicU64,
    id: u64,
) -> Scan {
    let files = if files.is_empty() {
        crate::picker::scan_files(root)
    } else {
        files
    };
    let mut seen = HashSet::new();
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    for rel in files {
        let path = root.join(&rel);
        seen.insert(path.clone());
        candidates.push((rel, path));
    }
    // Open buffers the index doesn't know yet (a new, never-saved file).
    let mut extra: Vec<&PathBuf> = open.keys().filter(|p| !seen.contains(*p)).collect();
    extra.sort();
    for path in extra {
        if let Ok(rel) = path.strip_prefix(root) {
            candidates.push((rel.to_string_lossy().replace('\\', "/"), path.clone()));
        }
    }
    let mut out = Vec::new();
    let mut total = 0usize;
    for (rel, path) in candidates {
        if generation.load(Ordering::Relaxed) != id {
            break;
        }
        if !globs.matches(&rel) {
            continue;
        }
        let owned;
        let text = match open.get(&path) {
            Some(t) => t.as_str(),
            None => {
                if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_FILE_BYTES) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                if bytes[..bytes.len().min(8192)].contains(&0) {
                    continue;
                }
                let Ok(s) = String::from_utf8(bytes) else {
                    continue;
                };
                owned = s;
                owned.as_str()
            }
        };
        let mut matches = Vec::new();
        let truncated = scan_text(text, re, replacement, MAX_MATCHES - total, &mut matches);
        total += matches.len();
        if !matches.is_empty() {
            out.push(FileMatches { path, matches });
        }
        if truncated {
            return Scan {
                files: out,
                truncated: true,
                error: None,
            };
        }
    }
    Scan {
        files: out,
        truncated: false,
        error: None,
    }
}

pub fn handle(ed: &mut Editor, key: Key) {
    let Some(far) = ed.far.as_mut() else {
        ed.enter_normal();
        return;
    };
    if let Some(field) = far.editing {
        let text = match field {
            Field::Search => &mut far.search,
            Field::Replace => &mut far.replace,
            Field::Files => &mut far.glob,
        };
        if let Some(changed) = crate::queryline::handle(text, &mut far.qcursor, key) {
            if changed {
                far.schedule(true);
            }
            return;
        }
        match key {
            Key::Tab | Key::BackTab => {
                let next = if key == Key::Tab {
                    field.next()
                } else {
                    field.prev()
                };
                far.qcursor = QueryCursor::at_end(far.text(next));
                far.editing = Some(next);
                far.field = next;
            }
            // Enter (or Esc from the field's Normal sub-mode) hands focus to
            // the match list, re-scanning at once if a scan is still due.
            Key::Enter | Key::Esc => {
                far.editing = None;
                if far.due.is_some() {
                    far.due = Some(Instant::now());
                }
            }
            Key::Down | Key::Up => far.editing = None,
            _ => {}
        }
        return;
    }
    let len = far.rows().len();
    let half = ed.screen_rows / 2;
    match key {
        Key::Esc | Key::Char('q') => ed.close_far(),
        Key::Char('j') | Key::Down | Key::Ctrl('n') => {
            far.cursor = (far.cursor + 1).min(len.saturating_sub(1))
        }
        Key::Char('k') | Key::Up | Key::Ctrl('p') => far.cursor = far.cursor.saturating_sub(1),
        Key::Ctrl('d') | Key::PageDown => {
            far.cursor = (far.cursor + half).min(len.saturating_sub(1))
        }
        Key::Ctrl('u') | Key::PageUp => far.cursor = far.cursor.saturating_sub(half),
        Key::Char('g') => far.cursor = 0,
        Key::Char('G') => far.cursor = len.saturating_sub(1),
        Key::Char('n') => far.jump_file(true),
        Key::Char('N') => far.jump_file(false),
        Key::Char(' ') | Key::Tab | Key::Char('x') => {
            far.toggle();
            far.cursor = (far.cursor + 1).min(len.saturating_sub(1));
        }
        Key::Char('a') => far.toggle_all(),
        Key::Char('s') | Key::Char('/') => ed.far_edit(Field::Search),
        Key::Char('r') => ed.far_edit(Field::Replace),
        Key::Char('f') => ed.far_edit(Field::Files),
        Key::Char('i') => {
            let field = far.field;
            ed.far_edit(field);
        }
        Key::Char('c') => {
            far.case = far.case.next();
            far.schedule(false);
        }
        Key::Char('F') => {
            far.fixed = !far.fixed;
            far.schedule(false);
        }
        Key::Enter => ed.far_jump(),
        Key::Char('R') => ed.far_apply(),
        Key::Char('U') => ed.far_undo(),
        _ => {}
    }
}

impl Editor {
    /// `:far` / `,sr`: opens the replace screen -- resuming the previous
    /// one (fields, toggles) if there is one. `prefill` puts a literal
    /// search string in (`,sw`'s word or selection) and focuses the
    /// replacement instead.
    pub fn open_far(&mut self, prefill: Option<String>) {
        self.close_completion();
        self.pending.reset();
        let far = self.far.get_or_insert_with(Far::new);
        match prefill {
            Some(text) => {
                far.search = text;
                far.fixed = true;
                far.disabled.clear();
                far.cursor = 0;
                far.editing = Some(Field::Replace);
                far.field = Field::Replace;
            }
            None => {
                let field = far.field;
                far.editing = Some(field);
            }
        }
        let field = far.editing.unwrap_or(far.field);
        far.qcursor = QueryCursor::at_end(far.text(field));
        far.schedule(false);
        self.mode = Mode::Far;
    }

    /// Leaves the screen but keeps its state for the next `:far`.
    pub fn close_far(&mut self) {
        if let Some(far) = &mut self.far {
            far.generation.fetch_add(1, Ordering::Relaxed);
            far.rx = None;
            far.due = None;
            far.busy = false;
            far.editing = None;
        }
        self.enter_normal();
    }

    fn far_edit(&mut self, field: Field) {
        if let Some(far) = &mut self.far {
            far.qcursor = QueryCursor::at_end(far.text(field));
            far.editing = Some(field);
            far.field = field;
        }
    }

    /// Bracketed paste into the focused field.
    pub fn far_paste(&mut self, pasted: &str) {
        let Some(far) = &mut self.far else { return };
        let Some(field) = far.editing else { return };
        let mut qc = far.qcursor.clone();
        let changed = crate::queryline::paste_at(far.text_mut(field), &mut qc, pasted);
        far.qcursor = qc;
        if changed {
            far.schedule(true);
        }
    }

    /// Starts a due scan and collects a finished one. Returns whether the
    /// screen changed.
    pub fn poll_far(&mut self) -> bool {
        let Some(far) = &mut self.far else {
            return false;
        };
        let mut changed = false;
        if far.due.is_some_and(|t| Instant::now() >= t) {
            far.due = None;
            let id = far.generation.load(Ordering::Relaxed);
            if far.search.is_empty() {
                far.files.clear();
                far.busy = false;
                far.truncated = false;
                far.error = None;
                far.rx = None;
                return true;
            }
            let compiled = compile(
                &far.search,
                &far.replace,
                far.fixed,
                far.case,
                self.config.ignorecase,
                self.config.smartcase,
            )
            .and_then(|c| Globs::parse(&far.glob).map(|g| (c, g)));
            let ((re, replacement), globs) = match compiled {
                Ok(v) => v,
                Err(e) => {
                    far.error = Some(e);
                    far.busy = false;
                    far.rx = None;
                    return true;
                }
            };
            let open: HashMap<PathBuf, String> = self
                .buffers
                .iter()
                .filter(|b| b.note_id.is_none())
                .filter_map(|b| Some((b.path.clone()?, b.rope.to_string())))
                .collect();
            let files = self.all_files.clone();
            let root = self.project_root.clone();
            let generation = far.generation.clone();
            let (tx, rx) = mpsc::channel();
            far.rx = Some(rx);
            std::thread::spawn(move || {
                let scan = run_scan(
                    &root,
                    files,
                    open,
                    &re,
                    &replacement,
                    &globs,
                    &generation,
                    id,
                );
                let _ = tx.send((id, scan));
            });
            changed = true;
        }
        if let Some(Ok((id, scan))) = far.rx.as_ref().map(|rx| rx.try_recv()) {
            far.rx = None;
            if id == far.generation.load(Ordering::Relaxed) {
                far.files = scan.files;
                far.truncated = scan.truncated;
                far.error = scan.error;
                far.busy = false;
                far.cursor = far.cursor.min(far.rows().len().saturating_sub(1));
                changed = true;
            }
        }
        changed
    }

    /// Enter: opens the match (or file) under the cursor. The screen's
    /// state is kept, so `,sr` comes straight back to it.
    fn far_jump(&mut self) {
        let Some(far) = &self.far else { return };
        let target = match far.rows().get(far.cursor).copied() {
            Some(Row::Match(fi, mi)) => {
                let m = &far.files[fi].matches[mi];
                Some((far.files[fi].path.clone(), m.line, m.col()))
            }
            Some(Row::File(fi)) => far.files[fi]
                .matches
                .first()
                .map(|m| (far.files[fi].path.clone(), m.line, m.col())),
            None => None,
        };
        if let Some((path, line, col)) = target {
            self.close_far();
            self.jump_to(path, line, col);
        }
    }

    /// The buffer index for `path`, loading it as a hidden buffer (with its
    /// persistent undo history) if it isn't open -- without switching the
    /// window to it the way `open_file` would.
    fn far_buffer(&mut self, path: &Path) -> anyhow::Result<usize> {
        let path = crate::files::identity(path);
        if let Some(i) = self
            .buffers
            .iter()
            .position(|b| b.path.as_ref() == Some(&path))
        {
            return Ok(i);
        }
        let mut buf = crate::buffer::Buffer::from_path(path)?;
        buf.apply_indent(&self.config);
        crate::undofile::restore(&self.project_root, &mut buf);
        self.buffers.push(buf);
        Ok(self.buffers.len() - 1)
    }

    /// `R`: replaces the selected matches. Each file is one undo step in
    /// its buffer; a buffer that was clean is saved, one with unsaved edits
    /// of its own is left modified rather than writing those edits out
    /// behind the user's back. A line that no longer reads as it did when
    /// scanned (edited since) is skipped, never patched blindly.
    pub fn far_apply(&mut self) {
        let Some(far) = &self.far else { return };
        if far.busy {
            self.set_message("Still searching -- try again in a moment");
            return;
        }
        let plan = far.plan();
        if plan.is_empty() {
            self.set_message("Nothing selected to replace");
            return;
        }
        let origin = self.buf().id;
        let (mut replaced, mut files, mut stale, mut unsaved, mut failed) = (0, 0, 0, 0, 0);
        let mut applied = Vec::new();
        for (path, lines) in plan {
            let idx = match self.far_buffer(&path) {
                Ok(i) => i,
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };
            let buf = &mut self.buffers[idx];
            let was_clean = !buf.is_modified();
            let mut count = 0;
            buf.begin_edit();
            // Bottom-up, so a replacement containing a newline can't shift
            // the lines still to be patched.
            for (line, matches) in lines.iter().rev() {
                let current = buf.line_text(*line);
                if *line >= buf.line_count() || current != matches[0].old {
                    stale += matches.len();
                    continue;
                }
                let new = replace_line(&current, matches);
                let start = buf.char_idx(*line, 0);
                let end = buf.char_idx(*line, buf.line_len(*line));
                buf.delete_char_range(start, end);
                buf.insert_str(*line, 0, &new);
                count += matches.len();
            }
            buf.commit_edit();
            if count == 0 {
                continue;
            }
            replaced += count;
            files += 1;
            let mut saved = false;
            if was_clean {
                let prev = self.cur;
                self.cur = idx;
                saved = self.save_current().is_ok();
                self.cur = prev;
            }
            if !saved {
                unsaved += 1;
            }
            let buf = &self.buffers[idx];
            applied.push(Applied {
                buffer_id: buf.id,
                edit_seq: buf.edit_seq,
                saved,
            });
        }
        if let Some(i) = self.buffers.iter().position(|b| b.id == origin) {
            self.cur = i;
        }
        let mut msg = format!("Replaced {replaced} match(es) in {files} file(s)");
        if unsaved > 0 {
            msg.push_str(&format!(" · {unsaved} left unsaved (had unsaved edits)"));
        }
        if stale > 0 {
            msg.push_str(&format!(" · {stale} skipped (changed since the scan)"));
        }
        if failed > 0 {
            msg.push_str(&format!(" · {failed} file(s) could not be opened"));
        }
        if files > 0 {
            msg.push_str(" · U undoes");
        }
        self.set_message(msg);
        if let Some(far) = &mut self.far {
            if !applied.is_empty() {
                far.last_apply = applied;
            }
            far.disabled.clear();
            far.schedule(false);
        }
    }

    /// `U`: reverts the last apply in every buffer it touched that hasn't
    /// been edited since, re-saving the ones the apply saved.
    pub fn far_undo(&mut self) {
        let Some(far) = &mut self.far else { return };
        let applied = std::mem::take(&mut far.last_apply);
        if applied.is_empty() {
            self.set_message("No replace to undo");
            return;
        }
        let origin = self.buf().id;
        let (mut undone, mut skipped) = (0, 0);
        for a in applied {
            let Some(idx) = self.buffers.iter().position(|b| b.id == a.buffer_id) else {
                skipped += 1;
                continue;
            };
            if self.buffers[idx].edit_seq != a.edit_seq || !self.buffers[idx].undo() {
                skipped += 1;
                continue;
            }
            undone += 1;
            if a.saved {
                let prev = self.cur;
                self.cur = idx;
                let _ = self.save_current();
                self.cur = prev;
            }
        }
        if let Some(i) = self.buffers.iter().position(|b| b.id == origin) {
            self.cur = i;
        }
        self.set_message(if skipped > 0 {
            format!("Undid the replace in {undone} file(s) · {skipped} edited since, left alone")
        } else {
            format!("Undid the replace in {undone} file(s)")
        });
        if let Some(far) = &mut self.far {
            far.schedule(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(search: &str, replace: &str) -> (fancy_regex::Regex, String) {
        compile(search, replace, false, Case::Match, false, false).unwrap()
    }

    #[test]
    fn scan_text_lists_every_occurrence_with_its_expanded_replacement() {
        let (r, rep) = re(r"\(\w\+\)_old", r"new_\1");
        let mut out = Vec::new();
        let truncated = scan_text("a_old b_old\nnone\nc_old\n", &r, &rep, 100, &mut out);
        assert!(!truncated);
        let got: Vec<_> = out
            .iter()
            .map(|m| (m.line, m.col(), m.new_line()))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, 0, "new_a b_old".to_string()),
                (0, 6, "a_old new_b".to_string()),
                (2, 0, "new_c".to_string()),
            ]
        );
    }

    #[test]
    fn scan_text_stops_at_the_limit() {
        let (r, rep) = re("x", "y");
        let mut out = Vec::new();
        assert!(scan_text("xxx\nxx\n", &r, &rep, 4, &mut out));
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn fixed_mode_is_literal_both_ways() {
        let (r, rep) = compile("a.b(", "$1 & \\1", true, Case::Match, false, false).unwrap();
        let mut out = Vec::new();
        scan_text("axb( a.b(\n", &r, &rep, 100, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].new_line(), "axb( $1 & \\1");
    }

    #[test]
    fn case_modes_follow_the_toggle_and_smartcase() {
        let hits = |case, search: &str| {
            let (r, rep) = compile(search, "", false, case, true, true).unwrap();
            let mut out = Vec::new();
            scan_text("Foo foo\n", &r, &rep, 100, &mut out);
            out.len()
        };
        assert_eq!(hits(Case::Auto, "foo"), 2);
        assert_eq!(
            hits(Case::Auto, "Foo"),
            1,
            "smartcase: uppercase means exact"
        );
        assert_eq!(hits(Case::Ignore, "Foo"), 2);
        assert_eq!(hits(Case::Match, "foo"), 1);
    }

    #[test]
    fn bad_pattern_is_an_error_not_a_panic() {
        // Vim syntax: a bare `(` is literal, an unclosed `\(` group is not.
        assert!(compile("(", "", false, Case::Auto, false, false).is_ok());
        assert!(compile("\\(", "", false, Case::Auto, false, false).is_err());
    }

    #[test]
    fn replace_line_applies_only_the_given_occurrences() {
        let (r, rep) = re("o", "0");
        let mut out = Vec::new();
        scan_text("foo boo", &r, &rep, 100, &mut out);
        assert_eq!(replace_line("foo boo", &out), "f00 b00");
        assert_eq!(
            replace_line("foo boo", &[out[1].clone(), out[3].clone()]),
            "fo0 bo0"
        );
    }

    #[test]
    fn globs_match_names_paths_dirs_and_exclusions() {
        let g = Globs::parse("*.rs, !tests/**").unwrap();
        assert!(g.matches("src/main.rs"));
        assert!(g.matches("main.rs"));
        assert!(!g.matches("tests/a.rs"));
        assert!(!g.matches("README.md"));
        let g = Globs::parse("src/**/*.{rs,toml} docs").unwrap();
        assert!(g.matches("src/a.rs"));
        assert!(g.matches("src/x/y/Cargo.toml"));
        assert!(!g.matches("lib/a.rs"));
        assert!(
            g.matches("docs/guide/intro.md"),
            "a bare dir name includes its contents"
        );
        let g = Globs::parse("").unwrap();
        assert!(g.matches("anything/at/all"));
        let g = Globs::parse("!*.lock").unwrap();
        assert!(g.matches("src/a.rs"));
        assert!(!g.matches("Cargo.lock"));
        let g = Globs::parse("a?c").unwrap();
        assert!(g.matches("x/abc") && !g.matches("a/c"));
    }

    fn far_with(files: Vec<FileMatches>) -> Far {
        let mut f = Far::new();
        f.files = files;
        f
    }

    fn sample() -> Far {
        let (r, rep) = re("old", "new");
        let mut a = Vec::new();
        scan_text("old old\nold\n", &r, &rep, 100, &mut a);
        let mut b = Vec::new();
        scan_text("x old\n", &r, &rep, 100, &mut b);
        far_with(vec![
            FileMatches {
                path: PathBuf::from("/p/a"),
                matches: a,
            },
            FileMatches {
                path: PathBuf::from("/p/b"),
                matches: b,
            },
        ])
    }

    #[test]
    fn toggling_matches_files_and_everything() {
        let mut f = sample();
        assert_eq!(f.rows().len(), 6);
        assert_eq!(f.enabled_count(), 4);
        f.cursor = 1; // first match of a
        f.toggle();
        assert_eq!(f.enabled_count(), 3);
        f.cursor = 0; // a's header: partially on -> all on
        f.toggle();
        assert_eq!(f.enabled_in(0), 3);
        f.toggle(); // all on -> all off
        assert_eq!(f.enabled_in(0), 0);
        assert_eq!(f.enabled_count(), 1);
        f.toggle_all();
        assert_eq!(f.enabled_count(), 4);
        f.toggle_all();
        assert_eq!(f.enabled_count(), 0);
    }

    #[test]
    fn plan_groups_selected_matches_by_file_and_line() {
        let mut f = sample();
        f.cursor = 2; // a's second match on line 0
        f.toggle();
        let plan = f.plan();
        assert_eq!(plan.len(), 2);
        let lines = &plan[0].1;
        assert_eq!(lines[&0].len(), 1);
        assert_eq!(lines[&1].len(), 1);
        assert_eq!(replace_line("old old", &lines[&0]), "new old");
    }

    #[test]
    fn toggles_survive_a_rescan_of_the_same_matches() {
        let mut f = sample();
        f.cursor = 5; // b's only match
        f.toggle();
        let rescanned = sample().files;
        f.files = rescanned;
        assert_eq!(f.enabled_in(1), 0);
    }

    #[test]
    fn file_jumps_move_between_headers() {
        let mut f = sample();
        f.cursor = 1;
        f.jump_file(true);
        assert_eq!(f.cursor, 4);
        f.jump_file(true);
        assert_eq!(f.cursor, 4, "no header past the last one");
        f.jump_file(false);
        assert_eq!(f.cursor, 0);
    }
}
