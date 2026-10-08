//! Debounced project search off the input thread, with stale generation rejection.
use crate::{
    editor::Editor,
    results::{Entry, Results},
};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};
type SearchReply = (u64, Vec<Entry>, Option<String>);
pub struct SearchJob {
    pub files_rx: Option<Receiver<Vec<String>>>,
    pub files_ready: bool,
    pub generation: Arc<AtomicU64>,
    /// (query, fixed-strings, search paths) -- the last two mirror
    /// `Results::grep_fixed`/`grep_paths` at the moment the query changed,
    /// since the spawned thread below can't borrow `self`/`Results`.
    pub pending: Option<(Instant, String, bool, Vec<String>)>,
    pub rx: Option<Receiver<SearchReply>>,
}
impl Default for SearchJob {
    fn default() -> Self {
        Self {
            files_rx: None,
            files_ready: false,
            generation: Arc::new(AtomicU64::new(0)),
            pending: None,
            rx: None,
        }
    }
}
impl Editor {
    /// `:lgrep <pattern>`: a synchronous project grep whose results populate the
    /// (buffer-independent) location list rather than the live results view --
    /// a static snapshot, matching how a loclist works. Reuses ripgrep's
    /// `--json` output with the same globs as live grep.
    pub fn lgrep(&mut self, pattern: &str) {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            self.set_message("Usage: :lgrep <pattern>");
            return;
        }
        let root = self.project_root.clone();
        let output = Command::new("rg")
            .current_dir(&root)
            .args([
                "--json",
                "--line-number",
                "--hidden",
                "--max-columns",
                "2000",
                "--glob",
                "!.git/**",
                "--glob",
                "!.vaayu/**",
                "--glob",
                "!target/**",
                "--",
                pattern,
                ".",
            ])
            .output();
        let output = match output {
            Ok(o) => o,
            Err(e) => {
                self.set_message(format!("lgrep requires ripgrep: {e}"));
                return;
            }
        };
        let mut entries = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v["type"] != "match" {
                continue;
            }
            let d = &v["data"];
            let Some(path) = d["path"]["text"].as_str() else {
                continue;
            };
            let raw_line = d["lines"]["text"].as_str().unwrap_or("");
            let text = raw_line.trim_end();
            let ln = (d["line_number"].as_u64().unwrap_or(1) as usize).saturating_sub(1);
            let col = d["submatches"]
                .as_array()
                .and_then(|m| m.first())
                .and_then(|m| m["start"].as_u64())
                .map(|byte| {
                    raw_line
                        .char_indices()
                        .take_while(|(i, _)| *i < byte as usize)
                        .count()
                })
                .unwrap_or(0);
            entries.push(crate::results::Entry::location(
                crate::files::identity(&root.join(path)),
                ln,
                col,
                text,
            ));
            if entries.len() >= 5000 {
                break;
            }
        }
        if entries.is_empty() {
            self.set_message(format!("lgrep: no matches for {pattern}"));
            return;
        }
        let count = entries.len();
        let mut r =
            crate::results::Results::new(format!("Location list — lgrep {pattern}"), entries);
        r.live = false;
        r.quickfix = false;
        self.set_loclist(r.clone());
        self.show_results(r);
        self.set_message(format!("lgrep: {count} match(es) — :lnext/:lprev to step"));
    }

    pub fn open_grep(&mut self, query: &str) {
        self.open_grep_scoped(query, vec![".".to_string()]);
    }
    /// Live grep restricted to `paths` (absolute file paths, or `["."]`
    /// for the whole project) -- shared by `open_grep` and the
    /// buffer-scoped `,fb`/`,fB` actions.
    pub fn open_grep_scoped(&mut self, query: &str, paths: Vec<String>) {
        let mut r = Results::new("Live grep", Vec::new());
        r.live = true;
        r.query = query.into();
        r.qcursor = crate::queryline::QueryCursor::at_end(&r.query);
        r.search_input = Some(true);
        r.grep_paths = paths;
        self.show_results(r);
        self.schedule_grep();
    }
    /// `,fb`: live grep scoped to just the current buffer's file.
    pub fn open_grep_current_buffer(&mut self, query: &str) {
        let Some(path) = self.buf().path.clone() else {
            self.set_message("This buffer has no file on disk");
            return;
        };
        self.open_grep_scoped(query, vec![path.display().to_string()]);
    }
    /// `,fB`: live grep scoped to every currently open buffer's file.
    pub fn open_grep_open_buffers(&mut self, query: &str) {
        let paths: Vec<String> = self
            .buffers
            .iter()
            .filter_map(|b| b.path.as_ref())
            .map(|p| p.display().to_string())
            .collect();
        if paths.is_empty() {
            self.set_message("No open buffers have a file on disk");
            return;
        }
        self.open_grep_scoped(query, paths);
    }
    /// Records the current live-grep query in `grep_history` (most recent
    /// last, capped, no consecutive duplicate) -- called when the query
    /// bar closes with a non-empty query, mirroring `command.rs`'s
    /// `push_history` for `:`/`/`/`?`.
    pub fn push_grep_history(&mut self) {
        let Some(r) = &self.results else { return };
        if !r.live {
            return;
        }
        let q = r.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        if self.grep_history.last().map(String::as_str) != Some(q.as_str()) {
            self.grep_history.push(q);
            if self.grep_history.len() > 200 {
                self.grep_history.remove(0);
            }
        }
        self.grep_history_browse = None;
        self.grep_history_draft.clear();
    }
    /// `Up`/`Down` while editing a live-grep query: cycles `grep_history`,
    /// same Up=older/Down=newer-then-draft convention as `command.rs`'s
    /// own `history_step`.
    pub fn grep_history_step(&mut self, older: bool) {
        let len = self.grep_history.len();
        let next = if older {
            if len == 0 {
                return;
            }
            match self.grep_history_browse {
                None => {
                    self.grep_history_draft = self
                        .results
                        .as_ref()
                        .map(|r| r.query.clone())
                        .unwrap_or_default();
                    Some(len - 1)
                }
                Some(0) => Some(0),
                Some(i) => Some(i - 1),
            }
        } else {
            match self.grep_history_browse {
                None => return,
                Some(i) if i + 1 < len => Some(i + 1),
                Some(_) => None,
            }
        };
        self.grep_history_browse = next;
        let query = match next {
            Some(i) => self.grep_history[i].clone(),
            None => std::mem::take(&mut self.grep_history_draft),
        };
        if let Some(r) = &mut self.results {
            r.query = query;
            r.qcursor = crate::queryline::QueryCursor::at_end(&r.query);
        }
        self.schedule_grep();
    }
    pub fn schedule_grep(&mut self) {
        self.search_job.generation.fetch_add(1, Ordering::Relaxed);
        let (query, fixed, paths) = self
            .results
            .as_ref()
            .map(|r| (r.query.clone(), r.grep_fixed, r.grep_paths.clone()))
            .unwrap_or_default();
        self.search_job.pending = Some((Instant::now(), query, fixed, paths));
        if let Some(r) = &mut self.results {
            r.busy = true;
        }
    }
    pub fn poll_jobs(&mut self) -> bool {
        let mut changed = self.poll_git();
        changed |= self.mermaid.borrow_mut().poll();
        changed |= self.poll_review();
        changed |= self.poll_git_task();
        changed |= self.poll_gh_task();
        changed |= self.poll_blame_task();
        changed |= self.poll_make_task();
        changed |= self.poll_tests();
        changed |= self.poll_file_tree();
        changed |= self.poll_watcher();
        changed |= self.poll_far();
        if let Some(files) = self
            .search_job
            .files_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.search_job.files_rx = None;
            self.search_job.files_ready = true;
            self.all_files = files;
            if self
                .file_tree
                .as_ref()
                .is_some_and(|t| !t.filter.is_empty())
            {
                self.refilter_tree();
                changed = true;
            }
            let recent = self.recent_relative_files();
            if let Some(p) = &mut self.file_picker {
                p.refilter(&self.all_files, &recent);
                changed = true;
            }
        }
        self.update_git_background();
        self.checkpoint_recovery();
        if self
            .search_job
            .pending
            .as_ref()
            .is_some_and(|(t, ..)| t.elapsed() >= Duration::from_millis(60))
        {
            let (_, query, fixed, paths) = self.search_job.pending.take().unwrap();
            let root = self.project_root.clone();
            let generation = self.search_job.generation.clone();
            let id = generation.load(Ordering::Relaxed);
            let (tx, rx) = mpsc::channel();
            self.search_job.rx = Some(rx);
            std::thread::spawn(move || {
                if query.is_empty() {
                    let _ = tx.send((id, Vec::new(), None));
                    return;
                }
                let mut command = Command::new("rg");
                command.current_dir(&root).args([
                    "--json",
                    "--line-number",
                    "--hidden",
                    "--max-columns",
                    "2000",
                    "--glob",
                    "!.git/**",
                    "--glob",
                    "!.vaayu/**",
                    "--glob",
                    "!target/**",
                ]);
                if fixed {
                    command.arg("-F");
                }
                command
                    .arg("--")
                    .arg(&query)
                    .args(if paths.is_empty() {
                        vec![".".to_string()]
                    } else {
                        paths
                    })
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let mut child = match command.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send((
                            id,
                            Vec::new(),
                            Some(format!("Live grep requires ripgrep: {e}")),
                        ));
                        return;
                    }
                };
                // Drain stderr concurrently so a verbose failure cannot fill its pipe.
                let err = child.stderr.take().unwrap();
                let err_thread = std::thread::spawn(move || {
                    use std::io::Read;
                    let mut s = String::new();
                    let _ = err.take(16384).read_to_string(&mut s);
                    s
                });
                let mut entries = Vec::new();
                let reader = BufReader::new(child.stdout.take().unwrap());
                for line in reader.lines().map_while(Result::ok) {
                    if generation.load(Ordering::Relaxed) != id || entries.len() >= 5000 {
                        let _ = child.kill();
                        break;
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    if v["type"] != "match" {
                        continue;
                    }
                    let d = &v["data"];
                    let Some(path) = d["path"]["text"].as_str() else {
                        continue;
                    };
                    let raw_line = d["lines"]["text"].as_str().unwrap_or("");
                    let text = raw_line.trim_end();
                    // saturating_sub guards a line_number of 0 (ripgrep uses
                    // 1-based lines, but don't underflow if it ever isn't).
                    let ln = (d["line_number"].as_u64().unwrap_or(1) as usize).saturating_sub(1);
                    if let Some(matches) = d["submatches"].as_array() {
                        for m in matches {
                            let byte = m["start"].as_u64().unwrap_or(0) as usize;
                            // ripgrep's byte offset indexes the original
                            // (untrimmed) line, so count columns against that;
                            // the trimmed `text` would yield None (-> column 0)
                            // for a match in trailing whitespace. Counting by
                            // char-start offsets is boundary-safe.
                            let col = raw_line
                                .char_indices()
                                .take_while(|(i, _)| *i < byte)
                                .count();
                            entries.push(Entry::location(
                                crate::files::identity(&root.join(path)),
                                ln,
                                col,
                                text,
                            ));
                            if entries.len() >= 5000 {
                                break;
                            }
                        }
                    }
                }
                let status = child.wait();
                let stderr = err_thread.join().unwrap_or_default();
                let error = if entries.len() >= 5000 {
                    Some("Showing first 5,000 matches; narrow the query".into())
                } else if status.ok().and_then(|s| s.code()).is_some_and(|c| c > 1) {
                    Some(stderr.trim().into())
                } else {
                    None
                };
                let _ = tx.send((id, entries, error));
            });
        }
        if let Some(rx) = &self.search_job.rx {
            if let Ok((id, entries, error)) = rx.try_recv() {
                if id == self.search_job.generation.load(Ordering::Relaxed) {
                    if let Some(r) = &mut self.results {
                        if r.live {
                            // Written into `all_entries` and re-derived
                            // through `apply_filter` -- not straight into
                            // `entries` -- so an active `f` filter (see
                            // `results.rs`'s key handler) keeps narrowing
                            // every new batch of grep-as-you-type matches
                            // instead of being silently dropped the next
                            // time the query changes.
                            r.all_entries = entries;
                            r.cursor = 0;
                            r.apply_filter();
                            r.busy = false;
                            r.error = error;
                            changed = true;
                        }
                    }
                }
            }
        }
        changed |= self.track_job_progress();
        changed
    }
}
