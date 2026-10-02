//! Filesystem watcher (inotify / FSEvents / ReadDirectoryChangesW through
//! the `notify` crate) feeding three consumers that used to poll or wait
//! for a focus event:
//! - the file tree, whose cached directory listings are re-read as soon as
//!   something is created, removed or renamed in them (the 1s `stat` poll
//!   in `Editor::poll_file_tree` stays as the fallback, and still covers
//!   expanded directories outside the watched project);
//! - autoread: an open buffer whose file changed on disk is reloaded when
//!   it has no unsaved edits, and flagged (`[changed on disk]` in the
//!   status line, plus a warning) when it does -- never clobbered;
//! - the project file index (`rg --files`) behind the picker and the
//!   tree's `/`, re-scanned after files appear or disappear (rate-limited,
//!   so a build writing thousands of files costs one scan every few
//!   seconds, not one per event).
//!
//! The OS watch is set up on a background thread -- a recursive inotify
//! watch walks the whole project, which must never delay the first frame
//! -- and lives there until the `Watcher` is dropped. Events are
//! coalesced into batches (`DEBOUNCE` after the last event, at most
//! `MAX_LATENCY` after the first) and applied from the idle loop through
//! `Editor::poll_watcher`. If the watch can't be set up (no backend, the
//! inotify watch limit, ...) the watcher reports `Failed` and everything
//! keeps its old polling/focus behavior (`:checkhealth` shows why).
use crate::editor::Editor;
use notify::{EventKind, RecursiveMode, Watcher as _};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

/// Quiet time after the last event before a batch is applied.
const DEBOUNCE: Duration = Duration::from_millis(75);
/// A steady stream of events still flushes this often.
const MAX_LATENCY: Duration = Duration::from_millis(500);
/// Minimum spacing of project file-index rescans driven by the watcher.
const INDEX_RESCAN: Duration = Duration::from_secs(2);
/// How often the set of out-of-project directories to watch is recomputed.
const EXTRA_SYNC: Duration = Duration::from_secs(1);

enum Msg {
    Ready,
    Failed(String),
    Event(notify::Result<notify::Event>),
}

enum Cmd {
    Watch(PathBuf),
    Unwatch(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Starting,
    Active,
    Failed(String),
}

/// One debounced set of changes. `structural` paths were created, removed
/// or renamed (their parent directory's listing changed); `content` paths
/// were only modified. `rescan` means the backend lost events (queue
/// overflow) and everything should be re-checked.
#[derive(Debug, Default)]
pub struct Batch {
    pub structural: HashSet<PathBuf>,
    pub content: HashSet<PathBuf>,
    pub rescan: bool,
}

impl Batch {
    fn is_empty(&self) -> bool {
        !self.rescan && self.structural.is_empty() && self.content.is_empty()
    }

    /// Whether `path` itself was touched in any way.
    pub fn touches(&self, path: &Path) -> bool {
        self.rescan || self.structural.contains(path) || self.content.contains(path)
    }
}

pub struct Watcher {
    /// The directory watched recursively (the project root at start).
    pub root: PathBuf,
    pub state: State,
    rx: Receiver<Msg>,
    cmd_tx: Sender<Cmd>,
    pending: Batch,
    first_event: Option<Instant>,
    last_event: Option<Instant>,
    /// Directories outside `root` watched non-recursively (open buffers'
    /// parents), and when that set was last recomputed.
    extra: HashSet<PathBuf>,
    extra_synced: Option<Instant>,
}

impl Watcher {
    /// Starts watching `root` recursively on a background thread; the
    /// result arrives as `State::Active` / `State::Failed` via `poll`.
    pub fn start(root: PathBuf) -> Watcher {
        let (tx, rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let watch_root = root.clone();
        std::thread::spawn(move || {
            if too_broad(&watch_root) {
                let _ = tx.send(Msg::Failed(format!(
                    "not watching {} recursively (home or filesystem root)",
                    watch_root.display()
                )));
                return;
            }
            let events = tx.clone();
            let mut w = match notify::recommended_watcher(move |res| {
                let _ = events.send(Msg::Event(res));
            }) {
                Ok(w) => w,
                Err(e) => {
                    let _ = tx.send(Msg::Failed(e.to_string()));
                    return;
                }
            };
            if let Err(e) = w.watch(&watch_root, RecursiveMode::Recursive) {
                let _ = tx.send(Msg::Failed(e.to_string()));
                return;
            }
            let _ = tx.send(Msg::Ready);
            // Keep the watch alive until the `Watcher` (and so `cmd_tx`)
            // is dropped; failures of the optional extra watches are
            // harmless (the focus/poll fallbacks still cover them).
            for cmd in cmd_rx {
                let _ = match cmd {
                    Cmd::Watch(p) => w.watch(&p, RecursiveMode::NonRecursive),
                    Cmd::Unwatch(p) => w.unwatch(&p),
                };
            }
        });
        Watcher {
            root,
            state: State::Starting,
            rx,
            cmd_tx,
            pending: Batch::default(),
            first_event: None,
            last_event: None,
            extra: HashSet::new(),
            extra_synced: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.state == State::Active
    }

    /// Drains the backend's messages and returns a batch once it has been
    /// quiet for `DEBOUNCE` (or pending for `MAX_LATENCY`).
    pub fn poll(&mut self) -> Option<Batch> {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> Option<Batch> {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Ready) => self.state = State::Active,
                Ok(Msg::Failed(e)) => self.state = State::Failed(e),
                Ok(Msg::Event(res)) => {
                    self.record(res);
                    self.first_event.get_or_insert(now);
                    self.last_event = Some(now);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.state != State::Active {
                        break;
                    }
                    // The backend thread died after starting: fall back.
                    self.state = State::Failed("watcher thread exited".into());
                    break;
                }
            }
        }
        let (first, last) = (self.first_event?, self.last_event?);
        if now.duration_since(last) < DEBOUNCE && now.duration_since(first) < MAX_LATENCY {
            return None;
        }
        self.first_event = None;
        self.last_event = None;
        let batch = std::mem::take(&mut self.pending);
        (!batch.is_empty()).then_some(batch)
    }

    fn record(&mut self, res: notify::Result<notify::Event>) {
        let ev = match res {
            Ok(ev) => ev,
            Err(_) => {
                self.pending.rescan = true;
                return;
            }
        };
        if ev.need_rescan() {
            self.pending.rescan = true;
        }
        let set = match classify(&ev.kind) {
            Some(true) => &mut self.pending.structural,
            Some(false) => &mut self.pending.content,
            None => return,
        };
        set.extend(ev.paths);
    }

    /// Watches exactly `dirs` (outside `root`) non-recursively, adding and
    /// removing OS watches for the difference from last time.
    fn sync_extra(&mut self, dirs: HashSet<PathBuf>) {
        for gone in self.extra.difference(&dirs) {
            let _ = self.cmd_tx.send(Cmd::Unwatch(gone.clone()));
        }
        for new in dirs.difference(&self.extra) {
            let _ = self.cmd_tx.send(Cmd::Watch(new.clone()));
        }
        self.extra = dirs;
    }

    /// Whether `dir` is covered by a live watch (recursive root or an
    /// extra directory), so it needn't be polled.
    pub fn covers(&self, dir: &Path) -> bool {
        self.is_active() && (dir.starts_with(&self.root) || self.extra.contains(dir))
    }
}

/// A project root too broad to watch recursively: walking (and holding an
/// inotify watch for) every directory under `$HOME` or `/` would cost far
/// more than the polling it replaces.
fn too_broad(root: &Path) -> bool {
    root.parent().is_none() || dirs::home_dir().is_some_and(|h| crate::files::identity(&h) == root)
}

/// `Some(true)` for an event that changes a directory listing, `Some(false)`
/// for a content/metadata change, `None` for noise (reads, opens).
fn classify(kind: &EventKind) -> Option<bool> {
    use notify::event::ModifyKind;
    match kind {
        EventKind::Create(_) | EventKind::Remove(_) => Some(true),
        EventKind::Modify(ModifyKind::Name(_)) => Some(true),
        EventKind::Modify(_) | EventKind::Any | EventKind::Other => Some(false),
        EventKind::Access(_) => None,
    }
}

/// Whether a change at `path` should refresh `git status`: anything in the
/// working tree, but inside `.git` only the index, HEAD and refs (not the
/// object store a `git gc` or fetch churns through).
fn affects_git(path: &Path) -> bool {
    let mut comps = path.components().map(|c| c.as_os_str());
    if !comps.any(|c| c == ".git") {
        return true;
    }
    matches!(
        comps.next().and_then(|c| c.to_str()),
        Some("index" | "HEAD" | "refs" | "packed-refs")
    )
}

fn inside_git_dir(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}

impl Editor {
    /// Idle-loop hook: (re)starts the watcher for the current project root,
    /// and applies a finished batch of changes to the file tree, open
    /// buffers and the project file index. Returns whether anything
    /// visible changed.
    pub fn poll_watcher(&mut self) -> bool {
        if !self.config.watch {
            self.watcher = None;
            return self.flush_index_rescan();
        }
        if self
            .watcher
            .as_ref()
            .is_none_or(|w| w.root != self.project_root)
        {
            self.watcher = Some(Watcher::start(self.project_root.clone()));
        }
        let extra = self.extra_watch_dirs_due();
        let w = self.watcher.as_mut().expect("started above");
        if let Some(dirs) = extra {
            w.sync_extra(dirs);
        }
        let batch = w.poll();
        let mut changed = self.flush_index_rescan();
        if let Some(batch) = batch {
            changed |= self.apply_fs_batch(&batch);
        }
        changed
    }

    /// The parents of open buffers' files outside the project root, when
    /// it's time to recompute them (about once a second).
    fn extra_watch_dirs_due(&mut self) -> Option<HashSet<PathBuf>> {
        let w = self.watcher.as_mut()?;
        if !w.is_active() || w.extra_synced.is_some_and(|t| t.elapsed() < EXTRA_SYNC) {
            return None;
        }
        w.extra_synced = Some(Instant::now());
        let root = w.root.clone();
        Some(
            self.buffers
                .iter()
                .filter_map(|b| b.path.as_deref()?.parent().map(Path::to_path_buf))
                .filter(|d| !d.starts_with(&root) && d.is_dir())
                .collect(),
        )
    }

    /// Applies one batch of filesystem changes. Split out of
    /// `poll_watcher` so tests can feed batches directly.
    pub fn apply_fs_batch(&mut self, batch: &Batch) -> bool {
        let mut changed = self.autoread_batch(batch);
        let root = self.project_root.clone();
        let mut regit = batch.rescan;
        if let Some(t) = &mut self.file_tree {
            if batch.rescan {
                changed |= t.refresh_stale();
            } else {
                let mut tree_changed = false;
                for p in &batch.structural {
                    tree_changed |= t.invalidate(p);
                    if let Some(parent) = p.parent() {
                        tree_changed |= t.invalidate(parent);
                    }
                }
                if tree_changed {
                    t.rebuild();
                    changed = true;
                }
            }
            regit |= batch
                .structural
                .iter()
                .chain(&batch.content)
                .any(|p| affects_git(p));
        }
        if regit {
            self.refresh_tree_git_status();
        }
        let index_stale = batch.rescan
            || batch
                .structural
                .iter()
                .any(|p| p.starts_with(&root) && !inside_git_dir(p));
        if index_stale {
            self.index_rescan_due.get_or_insert(Instant::now());
            changed |= self.flush_index_rescan();
        }
        changed
    }

    /// Re-scans the project file index if the watcher marked it stale and
    /// the last watcher-driven rescan was at least `INDEX_RESCAN` ago.
    fn flush_index_rescan(&mut self) -> bool {
        if self.index_rescan_due.is_none()
            || self
                .last_index_rescan
                .is_some_and(|t| t.elapsed() < INDEX_RESCAN)
            || self.search_job.files_rx.is_some()
        {
            return false;
        }
        self.index_rescan_due = None;
        self.last_index_rescan = Some(Instant::now());
        self.search_job.files_ready = false;
        self.start_file_scan();
        false
    }

    /// Autoread: reload every unmodified open buffer whose file the batch
    /// touched and whose disk content really differs; flag (and warn about)
    /// modified ones instead of clobbering them.
    fn autoread_batch(&mut self, batch: &Batch) -> bool {
        let mut reloaded: Vec<String> = Vec::new();
        // (name, has unsaved edits)
        let mut conflicted: Vec<(String, bool)> = Vec::new();
        let autoread = self.config.autoread;
        for b in &mut self.buffers {
            let Some(path) = b.path.clone() else {
                continue;
            };
            if !batch.touches(&path) || !b.changed_on_disk() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let modified = b.is_modified();
            if !modified && autoread && b.reload().is_ok() {
                reloaded.push(name);
            } else if !b.disk_changed {
                b.disk_changed = true;
                conflicted.push((name, modified));
            }
        }
        if let Some((name, modified)) = conflicted.first() {
            self.set_message(if *modified {
                format!(
                    "W: {name} changed on disk; buffer has unsaved changes (:e! to reload, :w! to overwrite)"
                )
            } else {
                format!("W: {name} changed on disk (:e! to reload)")
            });
        } else if let Some(name) = reloaded.first() {
            let more = match reloaded.len() {
                1 => String::new(),
                n => format!(" (+{} more)", n - 1),
            };
            self.set_message(format!("{name} reloaded (changed on disk){more}"));
        }
        !reloaded.is_empty() || !conflicted.is_empty()
    }

    /// One `:checkhealth` line describing the watcher.
    pub fn watcher_health(&self) -> String {
        match (&self.watcher, self.config.watch) {
            (_, false) => "- file watcher off (watch = false); polling".to_string(),
            (None, true) => "… file watcher not started yet".to_string(),
            (Some(w), true) => match &w.state {
                State::Active => format!("✓ file watcher on {}", w.root.display()),
                State::Starting => format!("… file watcher starting on {}", w.root.display()),
                State::Failed(e) => format!("✗ file watcher failed ({e}); polling"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, ModifyKind, RenameMode};

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "vaayu_watch_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        crate::files::identity(&d)
    }

    fn wait_active(w: &mut Watcher) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !w.is_active() && Instant::now() < deadline {
            assert!(w.poll().is_none() || w.is_active());
            assert!(!matches!(w.state, State::Failed(_)), "{:?}", w.state);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(w.is_active(), "watcher never became active");
    }

    fn wait_batch(w: &mut Watcher, want: impl Fn(&Batch) -> bool) -> Batch {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut all = Batch::default();
        while Instant::now() < deadline {
            if let Some(b) = w.poll() {
                all.rescan |= b.rescan;
                all.structural.extend(b.structural);
                all.content.extend(b.content);
                if want(&all) {
                    return all;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("expected change never arrived: {all:?}");
    }

    #[test]
    fn classify_separates_listing_changes_from_content_and_noise() {
        assert_eq!(classify(&EventKind::Create(CreateKind::File)), Some(true));
        assert_eq!(
            classify(&EventKind::Modify(ModifyKind::Name(RenameMode::Both))),
            Some(true)
        );
        assert_eq!(
            classify(&EventKind::Modify(ModifyKind::Data(DataChange::Content))),
            Some(false)
        );
        assert_eq!(
            classify(&EventKind::Access(notify::event::AccessKind::Read)),
            None
        );
    }

    #[test]
    fn git_relevance_ignores_the_object_store() {
        assert!(affects_git(Path::new("/p/src/main.rs")));
        assert!(affects_git(Path::new("/p/.git/index")));
        assert!(affects_git(Path::new("/p/.git/refs/heads/main")));
        assert!(!affects_git(Path::new("/p/.git/objects/ab/cdef")));
        assert!(!affects_git(Path::new("/p/.git/logs/HEAD")));
    }

    #[test]
    fn debounce_holds_a_batch_until_quiet_then_flushes_it_once() {
        let mut w = Watcher::start(tmpdir("debounce"));
        let t0 = Instant::now();
        w.record(Ok(
            notify::Event::new(EventKind::Create(CreateKind::File)).add_path(PathBuf::from("/x/a"))
        ));
        w.first_event = Some(t0);
        w.last_event = Some(t0);
        assert!(w.poll_at(t0 + DEBOUNCE / 2).is_none(), "still settling");
        let b = w.poll_at(t0 + DEBOUNCE).expect("quiet long enough");
        assert!(b.structural.contains(Path::new("/x/a")));
        assert!(w.poll_at(t0 + DEBOUNCE * 3).is_none(), "flushed only once");
    }

    #[test]
    fn a_steady_stream_still_flushes_after_max_latency() {
        let mut w = Watcher::start(tmpdir("latency"));
        let t0 = Instant::now();
        w.record(Ok(
            notify::Event::new(EventKind::Any).add_path(PathBuf::from("/x/a"))
        ));
        w.first_event = Some(t0);
        w.last_event = Some(t0 + MAX_LATENCY);
        assert!(w.poll_at(t0 + MAX_LATENCY).is_some());
    }

    #[test]
    fn real_watch_reports_created_and_modified_files() {
        let dir = tmpdir("real");
        let mut w = Watcher::start(dir.clone());
        wait_active(&mut w);
        std::fs::create_dir(dir.join("sub")).unwrap();
        let new = dir.join("sub").join("new.txt");
        // A directory created after the watch started is watched too.
        let b = wait_batch(&mut w, |b| b.structural.contains(&dir.join("sub")));
        assert!(!b.rescan);
        std::fs::write(&new, "x").unwrap();
        wait_batch(&mut w, |b| {
            b.structural.contains(&new) || b.content.contains(&new)
        });
        std::fs::write(&new, "xy").unwrap();
        wait_batch(&mut w, |b| b.content.contains(&new));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn editor_autoreads_a_clean_buffer_and_flags_a_dirty_one() {
        let dir = tmpdir("autoread");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "one\n").unwrap();
        std::fs::write(&b, "two\n").unwrap();
        let mut ed = Editor::new(crate::config::Config {
            clipboard_unnamedplus: false,
            ..Default::default()
        });
        ed.project_root = dir.clone();
        ed.open_file(a.clone()).unwrap();
        ed.open_file(b.clone()).unwrap();
        // `b` (current) gets an unsaved edit; `a` stays clean.
        ed.buf_mut().insert_str(0, 0, "dirty ");
        std::fs::write(&a, "ONE\n").unwrap();
        std::fs::write(&b, "TWO\n").unwrap();
        let mut batch = Batch::default();
        batch.content.insert(a.clone());
        batch.content.insert(b.clone());
        assert!(ed.apply_fs_batch(&batch));
        let find = |ed: &Editor, p: &Path| {
            ed.buffers
                .iter()
                .position(|x| x.path.as_deref() == Some(p))
                .unwrap()
        };
        let ia = find(&ed, &a);
        let ib = find(&ed, &b);
        assert_eq!(ed.buffers[ia].rope.to_string(), "ONE\n");
        assert!(!ed.buffers[ia].disk_changed);
        assert_eq!(
            ed.buffers[ib].rope.to_string(),
            "dirty two\n",
            "never clobbered"
        );
        assert!(ed.buffers[ib].disk_changed);
        assert!(
            ed.message.contains("b.txt changed on disk"),
            "{}",
            ed.message
        );
        // A second batch for the same conflict doesn't warn again, and an
        // untouched path is ignored entirely.
        ed.message.clear();
        assert!(!ed.apply_fs_batch(&batch));
        let mut other = Batch::default();
        other.content.insert(dir.join("unrelated"));
        assert!(!ed.apply_fs_batch(&other));
        // `:e!` clears the flag.
        ed.buffers[ib].reload().unwrap();
        assert!(!ed.buffers[ib].disk_changed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn noautoread_only_flags_clean_buffers() {
        let dir = tmpdir("noautoread");
        let a = dir.join("a.txt");
        std::fs::write(&a, "one\n").unwrap();
        let mut ed = Editor::new(crate::config::Config {
            clipboard_unnamedplus: false,
            autoread: false,
            ..Default::default()
        });
        ed.project_root = dir.clone();
        ed.open_file(a.clone()).unwrap();
        std::fs::write(&a, "ONE\n").unwrap();
        let mut batch = Batch::default();
        batch.content.insert(a.clone());
        ed.apply_fs_batch(&batch);
        assert_eq!(ed.buf().rope.to_string(), "one\n");
        assert!(ed.buf().disk_changed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn own_save_is_not_reported_as_an_external_change() {
        let dir = tmpdir("ownsave");
        let a = dir.join("a.txt");
        std::fs::write(&a, "one\n").unwrap();
        let mut ed = Editor::new(crate::config::Config {
            clipboard_unnamedplus: false,
            ..Default::default()
        });
        ed.project_root = dir.clone();
        ed.open_file(a.clone()).unwrap();
        ed.buf_mut().insert_str(0, 0, "x");
        ed.buf_mut().save().unwrap();
        let mut batch = Batch::default();
        batch.content.insert(a.clone());
        assert!(!ed.apply_fs_batch(&batch));
        assert!(!ed.buf().disk_changed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn home_and_filesystem_root_are_not_watched_recursively() {
        assert!(too_broad(Path::new("/")));
        if let Some(h) = dirs::home_dir() {
            assert!(too_broad(&crate::files::identity(&h)));
        }
        assert!(!too_broad(&tmpdir("broad")));
    }

    #[test]
    fn tree_picks_up_a_created_file_from_a_batch_without_polling() {
        let dir = tmpdir("tree");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut ed = Editor::new(crate::config::Config {
            clipboard_unnamedplus: false,
            ..Default::default()
        });
        ed.project_root = dir.clone();
        ed.toggle_file_tree();
        let names = |ed: &Editor| -> Vec<String> {
            let t = ed.file_tree.as_ref().unwrap();
            t.nodes.iter().map(|n| n.name.clone()).collect()
        };
        assert!(!names(&ed).contains(&"b.txt".to_string()));
        let b = dir.join("b.txt");
        std::fs::write(&b, "x").unwrap();
        let mut batch = Batch::default();
        batch.structural.insert(b.clone());
        assert!(ed.apply_fs_batch(&batch));
        assert!(
            names(&ed).contains(&"b.txt".to_string()),
            "{:?}",
            names(&ed)
        );
        // A removal shows up the same way; an uncached directory is ignored.
        std::fs::remove_file(&b).unwrap();
        assert!(ed.apply_fs_batch(&batch));
        assert!(!names(&ed).contains(&"b.txt".to_string()));
        let mut deep = Batch::default();
        deep.structural.insert(dir.join("sub").join("deep.txt"));
        let before = names(&ed);
        ed.apply_fs_batch(&deep);
        assert_eq!(names(&ed), before, "collapsed sub isn't re-read");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn structural_changes_mark_the_file_index_stale_rate_limited() {
        let dir = tmpdir("index");
        let mut ed = Editor::new(crate::config::Config {
            clipboard_unnamedplus: false,
            ..Default::default()
        });
        ed.project_root = dir.clone();
        ed.search_job.files_ready = true;
        let mut batch = Batch::default();
        batch.structural.insert(dir.join("new.rs"));
        ed.apply_fs_batch(&batch);
        assert!(ed.search_job.files_rx.is_some(), "rescan started");
        // Land it, then a second change within INDEX_RESCAN waits.
        let files = ed.search_job.files_rx.take().unwrap().recv().unwrap();
        ed.all_files = files;
        ed.search_job.files_ready = true;
        ed.apply_fs_batch(&batch);
        assert!(ed.search_job.files_rx.is_none(), "rate-limited");
        assert!(ed.index_rescan_due.is_some(), "but remembered");
        // `.git` internals never touch the index.
        ed.index_rescan_due = None;
        let mut git = Batch::default();
        git.structural
            .insert(dir.join(".git").join("objects").join("ab"));
        ed.apply_fs_batch(&git);
        assert!(ed.index_rescan_due.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
