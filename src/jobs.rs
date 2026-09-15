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
    pub generation: Arc<AtomicU64>,
    pub pending: Option<(Instant, String)>,
    pub rx: Option<Receiver<SearchReply>>,
}
impl Default for SearchJob {
    fn default() -> Self {
        Self {
            files_rx: None,
            generation: Arc::new(AtomicU64::new(0)),
            pending: None,
            rx: None,
        }
    }
}
impl Editor {
    pub fn open_grep(&mut self, query: &str) {
        let mut r = Results::new("Live grep", Vec::new());
        r.live = true;
        r.query = query.into();
        r.search_input = Some(true);
        self.show_results(r);
        self.schedule_grep();
    }
    pub fn schedule_grep(&mut self) {
        self.search_job.generation.fetch_add(1, Ordering::Relaxed);
        let query = self
            .results
            .as_ref()
            .map(|r| r.query.clone())
            .unwrap_or_default();
        self.search_job.pending = Some((Instant::now(), query));
        if let Some(r) = &mut self.results {
            r.busy = true;
        }
    }
    pub fn poll_jobs(&mut self) -> bool {
        let mut changed = self.poll_git();
        changed |= self.poll_git_task();
        if let Some(files) = self
            .search_job
            .files_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.search_job.files_rx = None;
            self.all_files = files;
            if let Some(p) = &mut self.file_picker {
                p.refilter(&self.all_files);
                changed = true;
            }
        }
        self.update_git_background();
        self.checkpoint_recovery();
        if self
            .search_job
            .pending
            .as_ref()
            .is_some_and(|(t, _)| t.elapsed() >= Duration::from_millis(120))
        {
            let (_, query) = self.search_job.pending.take().unwrap();
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
                command
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
                        &query,
                        ".",
                    ])
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
                    let text = d["lines"]["text"].as_str().unwrap_or("").trim_end();
                    let ln = d["line_number"].as_u64().unwrap_or(1) as usize - 1;
                    if let Some(matches) = d["submatches"].as_array() {
                        for m in matches {
                            let byte = m["start"].as_u64().unwrap_or(0) as usize;
                            let col = text.get(..byte).unwrap_or("").chars().count();
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
                            r.entries = entries;
                            r.selected.clear();
                            r.cursor = 0;
                            r.busy = false;
                            r.error = error;
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }
}
