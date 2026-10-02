//! Fidget-style progress model: every long-running thing the editor is
//! waiting on -- LSP `$/progress` (workDoneProgress) tokens, live grep,
//! git commands, `:make`/`:task`, LSP formatting, the project file scan --
//! is one `Task` keyed by a stable string, shown as a compact stack in the
//! bottom-right corner (see `render::draw_progress`) with a spinner while
//! active, then a ✓/✗ that lingers briefly and fades before it's dropped.
//!
//! Background jobs aren't instrumented at every spawn site: `track` is
//! called once per poll with each job slot's busy state (an in-flight
//! receiver, a pending request) and begins/finishes the matching task on
//! the edges, so a job that's cancelled by clearing its slot still ends.
//! Spawn sites that know something more specific (the `:make` command, the
//! git op) call `begin` themselves first; `track` then leaves it alone.
use std::time::{Duration, Instant};

/// A task only appears once it has been running this long, so a grep per
/// keystroke or a millisecond file scan never flickers on screen.
pub const SHOW_AFTER: Duration = Duration::from_millis(250);
/// How long a finished (shown) task stays up before it's dropped.
pub const LINGER: Duration = Duration::from_millis(2000);
/// The last part of `LINGER`, during which a finished task is drawn dimmed.
pub const FADE: Duration = Duration::from_millis(1000);
/// Rows the stack uses at most (the last one becomes "+N more" if needed).
pub const MAX_ROWS: usize = 5;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_STEP: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub struct Task {
    pub key: String,
    /// Who's doing the work: a language server's name, "grep", "git", ...
    pub source: String,
    pub title: String,
    pub message: Option<String>,
    pub percentage: Option<u64>,
    pub started: Instant,
    /// When it finished, and whether it succeeded.
    pub done: Option<(Instant, bool)>,
}

/// How a visible task is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Still running; carries the spinner glyph for this frame.
    Active(char),
    Done {
        ok: bool,
    },
    /// Finished and in the last `FADE` of its linger.
    Fading {
        ok: bool,
    },
    /// The "+N more" row standing in for the oldest overflowing tasks.
    More,
}

/// One row of the stack, ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub phase: Phase,
    /// Title, percentage and message (or "+N more").
    pub text: String,
    pub source: String,
}

#[derive(Debug)]
pub struct Progress {
    pub tasks: Vec<Task>,
    epoch: Instant,
    /// What `rows` produced at the last `tick`, so the idle loop only
    /// redraws when the spinner advanced or a task appeared/faded/went.
    shown: Vec<Row>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            epoch: Instant::now(),
            shown: Vec::new(),
        }
    }
}

impl Progress {
    fn find(&mut self, key: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.key == key)
    }

    pub fn is_active(&self, key: &str) -> bool {
        self.tasks.iter().any(|t| t.key == key && t.done.is_none())
    }

    /// Starts (or restarts, if it's lingering as finished) the task `key`.
    pub fn begin(&mut self, key: &str, source: &str, title: &str, message: Option<String>) {
        let now = Instant::now();
        match self.find(key) {
            Some(t) => {
                if t.done.is_some() {
                    t.started = now;
                    t.percentage = None;
                }
                t.done = None;
                t.source = source.into();
                t.title = title.into();
                t.message = message;
            }
            None => self.tasks.push(Task {
                key: key.into(),
                source: source.into(),
                title: title.into(),
                message,
                percentage: None,
                started: now,
                done: None,
            }),
        }
    }

    /// Updates a running task's fields (each only when given). A report
    /// for an unknown key starts it, like a server that skipped "begin".
    pub fn report(
        &mut self,
        key: &str,
        source: &str,
        title: Option<String>,
        message: Option<String>,
        percentage: Option<u64>,
    ) {
        if !self.is_active(key) {
            let title = title.clone().unwrap_or_else(|| "Working…".into());
            self.begin(key, source, &title, None);
        }
        let t = self.find(key).expect("begun above");
        if let Some(title) = title {
            t.title = title;
        }
        if message.is_some() {
            t.message = message;
        }
        if percentage.is_some() {
            t.percentage = percentage.map(|p| p.min(100));
        }
    }

    /// Marks `key` finished. One that never became visible is dropped
    /// outright rather than flashing up just to say it's done.
    pub fn finish(&mut self, key: &str, ok: bool) {
        let now = Instant::now();
        let Some(i) = self
            .tasks
            .iter()
            .position(|t| t.key == key && t.done.is_none())
        else {
            return;
        };
        if now.duration_since(self.tasks[i].started) < SHOW_AFTER {
            self.tasks.remove(i);
        } else {
            self.tasks[i].done = Some((now, ok));
        }
    }

    /// Reconciles one job slot: begins `key` when it turns busy, finishes
    /// it (successfully) when it goes idle. Returns whether that changed
    /// anything.
    pub fn track(&mut self, key: &str, source: &str, title: &str, busy: bool) -> bool {
        match (busy, self.is_active(key)) {
            (true, false) => self.begin(key, source, title, None),
            (false, true) => self.finish(key, true),
            _ => return false,
        }
        true
    }

    /// Ends every running task whose key starts with `prefix` (e.g. all of
    /// one language server's tokens when it exits or is restarted).
    pub fn finish_prefix(&mut self, prefix: &str) {
        let keys: Vec<String> = self
            .tasks
            .iter()
            .filter(|t| t.key.starts_with(prefix) && t.done.is_none())
            .map(|t| t.key.clone())
            .collect();
        for key in keys {
            self.finish(&key, true);
        }
    }

    /// The stack to draw at `now`, oldest first (so the newest task sits
    /// nearest the corner), capped at `MAX_ROWS`.
    pub fn rows(&self, now: Instant) -> Vec<Row> {
        let frame = (now.saturating_duration_since(self.epoch).as_millis()
            / SPINNER_STEP.as_millis()) as usize;
        let mut rows: Vec<Row> = self
            .tasks
            .iter()
            .filter_map(|t| {
                let phase = match t.done {
                    None if now.saturating_duration_since(t.started) < SHOW_AFTER => return None,
                    None => Phase::Active(SPINNER[frame % SPINNER.len()]),
                    Some((at, ok)) => {
                        let age = now.saturating_duration_since(at);
                        if age >= LINGER {
                            return None;
                        } else if age >= LINGER - FADE {
                            Phase::Fading { ok }
                        } else {
                            Phase::Done { ok }
                        }
                    }
                };
                Some(Row {
                    phase,
                    text: task_text(t),
                    source: t.source.clone(),
                })
            })
            .collect();
        if rows.len() > MAX_ROWS {
            let hidden = rows.len() - (MAX_ROWS - 1);
            rows.drain(..hidden);
            rows.insert(
                0,
                Row {
                    phase: Phase::More,
                    text: format!("+{hidden} more"),
                    source: String::new(),
                },
            );
        }
        rows
    }

    /// Called from the idle loop: drops tasks whose linger ran out and
    /// reports whether the stack looks different from the last tick (the
    /// spinner advanced, a task crossed `SHOW_AFTER`, faded, or went), i.e.
    /// whether a redraw is due.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.tasks.retain(|t| {
            t.done
                .is_none_or(|(at, _)| now.saturating_duration_since(at) < LINGER)
        });
        let rows = self.rows(now);
        if rows == self.shown {
            return false;
        }
        self.shown = rows;
        true
    }
}

impl crate::editor::Editor {
    /// Reconciles the progress stack with every background job slot (see
    /// the module docs); called from `poll_jobs`. Returns whether a task
    /// began or finished.
    pub fn track_job_progress(&mut self) -> bool {
        let grep = self.results.as_ref().is_some_and(|r| r.live && r.busy);
        let query = grep
            .then(|| self.results.as_ref().map(|r| r.query.clone()))
            .flatten();
        let format = self
            .pending_language
            .values()
            .find(|ctx| ctx.kind == "format")
            .map(|ctx| ctx.client.split('@').next().unwrap_or("lsp").to_string());
        let p = &mut self.progress;
        let mut changed = false;
        if grep {
            // Follow the query as it's retyped mid-search.
            match p.find("grep").filter(|t| t.done.is_none()) {
                Some(t) => t.message = query,
                None => {
                    p.begin("grep", "grep", "Searching", query);
                    changed = true;
                }
            }
        }
        changed |= p.track("grep", "grep", "Searching", grep);
        changed |= p.track("git", "git", "Running git", self.git_task.is_some());
        changed |= p.track("blame", "git", "Reading blame", self.blame_task.is_some());
        changed |= p.track("make", "make", "Running task", self.make_task.is_some());
        changed |= p.track(
            "files",
            "files",
            "Scanning project files",
            self.search_job.files_rx.is_some(),
        );
        if let Some(source) = &format {
            if !p.is_active("format") {
                p.begin("format", source, "Formatting", None);
                changed = true;
            }
        }
        changed |= p.track("format", "lsp", "Formatting", format.is_some());
        changed
    }

    /// Idle-loop hook: whether the progress stack needs a redraw (see
    /// `Progress::tick`). Always false with `progress` turned off, though
    /// finished tasks are still pruned.
    pub fn tick_progress(&mut self) -> bool {
        let changed = self.progress.tick(Instant::now());
        self.config.progress && changed
    }
}

/// `title 42% — message`, the part of a row left of the source.
fn task_text(t: &Task) -> String {
    let mut s = t.title.clone();
    if let Some(pct) = t.percentage {
        s.push_str(&format!(" {pct}%"));
    }
    if let Some(msg) = t.message.as_deref().filter(|m| !m.is_empty()) {
        if s.is_empty() {
            s = msg.to_string();
        } else {
            s.push_str(&format!(" — {msg}"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aged(p: &mut Progress, key: &str, by: Duration) {
        let t = p.find(key).unwrap();
        t.started -= by;
        if let Some((at, ok)) = t.done {
            t.done = Some((at - by, ok));
        }
    }

    #[test]
    fn a_task_only_shows_after_the_show_delay() {
        let mut p = Progress::default();
        p.begin("grep", "grep", "Searching", Some("foo".into()));
        assert!(p.rows(Instant::now()).is_empty());
        aged(&mut p, "grep", SHOW_AFTER);
        let rows = p.rows(Instant::now());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "Searching — foo");
        assert_eq!(rows[0].source, "grep");
        assert!(matches!(rows[0].phase, Phase::Active(_)));
    }

    #[test]
    fn a_quick_task_that_never_showed_is_dropped_on_finish() {
        let mut p = Progress::default();
        p.begin("grep", "grep", "Searching", None);
        p.finish("grep", true);
        assert!(p.tasks.is_empty());
    }

    #[test]
    fn a_finished_task_lingers_then_fades_then_goes() {
        let mut p = Progress::default();
        p.begin("make", "make", "cargo build", None);
        aged(&mut p, "make", SHOW_AFTER);
        p.finish("make", false);
        let now = Instant::now();
        assert_eq!(p.rows(now)[0].phase, Phase::Done { ok: false });
        assert_eq!(
            p.rows(now + (LINGER - FADE))[0].phase,
            Phase::Fading { ok: false }
        );
        assert!(p.rows(now + LINGER).is_empty());
        p.tick(now + LINGER);
        assert!(p.tasks.is_empty(), "tick prunes an expired task");
    }

    #[test]
    fn report_updates_fields_and_starts_an_unknown_token() {
        let mut p = Progress::default();
        p.report("lsp:ra:1", "ra", None, Some("crates".into()), Some(140));
        let t = &p.tasks[0];
        assert_eq!(t.title, "Working…");
        assert_eq!(t.percentage, Some(100), "percentages clamp to 100");
        p.report("lsp:ra:1", "ra", Some("Indexing".into()), None, Some(50));
        let t = &p.tasks[0];
        assert_eq!(
            task_text(t),
            "Indexing 50% — crates",
            "a report without a message keeps the old one"
        );
    }

    #[test]
    fn track_begins_and_finishes_on_edges_only() {
        let mut p = Progress::default();
        assert!(!p.track("git", "git", "git", false));
        assert!(p.track("git", "git", "git", true));
        assert!(!p.track("git", "git", "git", true), "still busy: no change");
        aged(&mut p, "git", SHOW_AFTER);
        assert!(p.track("git", "git", "git", false));
        assert!(!p.is_active("git"));
        assert_eq!(p.tasks.len(), 1, "a shown task lingers after finishing");
    }

    #[test]
    fn begin_keeps_a_more_specific_title_that_track_would_overwrite() {
        let mut p = Progress::default();
        p.begin("make", "make", "cargo test", None);
        assert!(!p.track("make", "make", "make", true));
        assert_eq!(p.tasks[0].title, "cargo test");
    }

    #[test]
    fn restarting_a_lingering_task_makes_it_active_again() {
        let mut p = Progress::default();
        p.begin("git", "git", "git push", None);
        aged(&mut p, "git", SHOW_AFTER);
        p.finish("git", true);
        p.begin("git", "git", "git pull", None);
        assert_eq!(p.tasks.len(), 1);
        assert!(p.is_active("git"));
        assert_eq!(p.tasks[0].title, "git pull");
    }

    #[test]
    fn overflow_collapses_the_oldest_into_a_more_row() {
        let mut p = Progress::default();
        for i in 0..MAX_ROWS + 2 {
            let key = format!("t{i}");
            p.begin(&key, "lsp", &key, None);
            aged(&mut p, &key, SHOW_AFTER);
        }
        let rows = p.rows(Instant::now());
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(rows[0].text, "+3 more");
        assert_eq!(rows.last().unwrap().text, format!("t{}", MAX_ROWS + 1));
    }

    #[test]
    fn finish_prefix_ends_only_matching_tasks() {
        let mut p = Progress::default();
        p.begin("lsp:a:1", "a", "x", None);
        p.begin("lsp:b:1", "b", "y", None);
        p.finish_prefix("lsp:a:");
        assert!(!p.is_active("lsp:a:1"));
        assert!(p.is_active("lsp:b:1"));
    }

    #[test]
    fn tick_reports_a_change_only_when_the_stack_looks_different() {
        let mut p = Progress::default();
        assert!(!p.tick(Instant::now()), "nothing to show, nothing changed");
        p.begin("g", "git", "git log", None);
        aged(&mut p, "g", SHOW_AFTER);
        let now = Instant::now();
        assert!(p.tick(now), "a task became visible");
        assert!(!p.tick(now), "same frame, same stack");
        assert!(p.tick(now + SPINNER_STEP), "the spinner advanced");
    }
}
