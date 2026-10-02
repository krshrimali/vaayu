//! Project file tree sidebar (`,e` / `,ft` / `:tree`): a neo-tree /
//! nvim-tree style explorer pinned to the left (or right, `tree_position`)
//! edge of the tab at a fixed width (`tree_width`), independent of the
//! split ratios of the panes beside it (see `Layout::Split::fixed`).
//!
//! Speed is the point of the design:
//! - Directories are only read when expanded, and each listing is cached
//!   (`FileTree::cache`). Expanding/collapsing, toggling hidden/ignored
//!   files or re-rooting re-flattens from the cache without touching the
//!   disk; a cached listing is re-read when the filesystem watcher reports
//!   a change in its directory (`src/watcher.rs`), or -- for directories
//!   the watcher doesn't cover, or without one -- when its mtime changes
//!   (one `stat` per *visible* directory about once a second while the
//!   tree is open, see `Editor::poll_file_tree`), so files created by
//!   `:w`, a shell or `git checkout` show up on their own.
//! - `git status` (decorations) and `git status --ignored` (the
//!   `.gitignore` filter) run on a background thread and never block a
//!   keystroke or a frame; per-directory roll-ups are precomputed once
//!   per refresh (`git_dirs`), not per rendered row.
//! - `/` fuzzy-filters the *whole project* (not just expanded
//!   directories) using the same background `rg --files` inventory and
//!   bounded scorer the file picker uses, showing the matches in tree
//!   form with their ancestor directories. Until that inventory is ready
//!   it falls back to the already-loaded nodes.
//!
//! Key handling is entirely self-contained (its own counts, `g`/`z`/`[`/`]`
//! prefixes, `j`/`k`/`G`, ...) and never routed through `Awaiting` since
//! the tree's `cursor` indexes a node list, not a buffer's lines --
//! reusing generic motion/operator dispatch here would silently operate
//! on the placeholder buffer instead. `?` in the tree lists every key.
//!
//! File operations keep the safety rules they always had: delete (`d d`)
//! and trash (`t t`, into `.vaayu/trash/`) need a second press; paste
//! refuses collisions and pasting a directory into itself; nothing is
//! renamed/moved/deleted while an open buffer under it has unsaved
//! changes. With `Space`-marked nodes, `d`/`t`/`y`/`x` act on all marks.
use crate::editor::Editor;
use crate::key::Key;
use crate::windows::{Layout, Window};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Rows above the node list (the root/status header).
pub const HEADER_ROWS: usize = 1;
/// How many filter hits a project-wide `/` keeps (best-scored first).
const FILTER_TAKE: usize = 300;
/// `E` (expand recursively) stops after loading this many entries, so a
/// stray `E` on a huge directory can't stall the editor.
const EXPAND_ALL_LIMIT: usize = 20_000;
/// How often visible directories are re-`stat`ed for external changes.
const STALE_CHECK: Duration = Duration::from_millis(1000);

/// True if `name` is a single, in-directory path component -- not
/// absolute, no path separator, and not `.`/`..`. Joining such a name onto
/// a target directory can't escape it; an absolute path would replace the
/// target entirely and a `..` (or embedded separator) would walk out of
/// it. A trailing `/` is fine: `Path::components` normalizes it away, so
/// `sub/` is still one `Normal` component.
fn is_safe_tree_name(name: &str) -> bool {
    let mut comps = Path::new(name).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(Component::Normal(_)), None)
    )
}

/// `:treenew` also accepts nested relative paths (`a/b/c.rs`, created with
/// any missing parents) -- every component must still be a plain name, so
/// it can't climb out of the target directory or be absolute.
fn is_safe_relative_path(name: &str) -> bool {
    let p = Path::new(name);
    p.components().next().is_some() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

#[derive(Clone, Debug)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    /// A directory whose children are shown right below it.
    pub open: bool,
    /// Last visible child of its parent: drawn with `└` rather than `├`.
    pub last: bool,
    /// Bit `d` set: the ancestor at depth `d` has more siblings below it,
    /// so an indent guide (`│`) runs through this row at column `d`.
    pub rails: u64,
    /// A symlink's target, shown dimmed after the name.
    pub link: Option<PathBuf>,
    /// Inside (or itself) a `.gitignore`d path; dimmed when shown.
    pub ignored: bool,
}

#[derive(Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    link: Option<PathBuf>,
}

struct Listing {
    entries: Arc<Vec<Entry>>,
    mtime: Option<SystemTime>,
}

/// One background `git status` result: (status letters, ignored paths).
/// Either half is `None` outside a repo or without `git`.
type GitSnapshot = (Option<HashMap<PathBuf, char>>, Option<BTreeSet<PathBuf>>);

/// How the tree opens a file (see `Editor::tree_open`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OpenIn {
    Edit,
    VSplit,
    HSplit,
    Tab,
}

pub struct FileTree {
    pub root: PathBuf,
    pub expanded: BTreeSet<PathBuf>,
    pub cursor: usize,
    /// First visible node (scroll offset). The sidebar has its own viewport,
    /// independent of the pane height, kept in sync with `cursor` at render
    /// time via `ensure_visible` and moved directly by the mouse wheel.
    pub top: usize,
    pub nodes: Vec<Node>,
    /// Armed by a first `d` for exactly these targets; a second `d` on the
    /// same targets deletes them, any other key cancels. A real delete has
    /// no undo, so it always takes a second explicit key.
    pub confirm_delete: Option<Vec<PathBuf>>,
    /// Same two-press shape as `confirm_delete`, for `t` (trash).
    pub confirm_trash: Option<Vec<PathBuf>>,
    /// Set by `y` (copy, `false`) or `x` (cut, `true`); `p` pastes into the
    /// cursor's target directory. A cut source is only removed once the
    /// paste actually succeeds.
    pub clipboard: Option<(Vec<PathBuf>, bool)>,
    /// Dotfiles (other than `.git`, which is always skipped) are hidden
    /// unless this is set; `.`/`H` toggles it.
    pub show_hidden: bool,
    /// Paths toggled with `m`; shown with a ★ and listed by `:treebookmarks`
    /// / `B`. In-memory only.
    pub bookmarks: BTreeSet<PathBuf>,
    /// The `/` query. Non-empty: the tree shows only matches (and their
    /// ancestors). `filter_input` is whether keys are going to the query.
    pub filter: String,
    pub filter_input: bool,
    /// `git status` letters per file, from the latest background refresh.
    pub git_status: HashMap<PathBuf, char>,
    /// Per-directory roll-up of `git_status` (the most significant letter
    /// of any descendant), precomputed when a refresh lands.
    pub git_dirs: HashMap<PathBuf, char>,
    /// Paths `git status --ignored` reports (an ignored directory is one
    /// entry, not each file inside it). Hidden unless `show_ignored`.
    pub gitignored: BTreeSet<PathBuf>,
    pub show_ignored: bool,
    /// `Space`-marked nodes; `d`/`t`/`y`/`x` act on all of them at once.
    pub marked: BTreeSet<PathBuf>,
    /// While filtering: the actual hits (vs. ancestor rows shown only for
    /// context), so they can be highlighted.
    pub matched: HashSet<PathBuf>,
    /// `?`: the in-pane key reference replaces the node list.
    pub show_help: bool,
    pub help_scroll: usize,
    /// Sidebar width in cells (resizable with `<`/`>`, `Ctrl-W <`/`>`, or
    /// by dragging its border).
    pub width: usize,
    /// The editing pane files open into (the last one focused).
    pub last_edit_window: usize,
    /// List rows in the pane at the last render (for paging).
    pub height: usize,
    cache: HashMap<PathBuf, Listing>,
    /// Project-wide filter hits, best first, from `Editor::refilter_tree`;
    /// `None` falls back to matching loaded nodes.
    filter_hits: Option<Vec<PathBuf>>,
    git_rx: Option<Receiver<GitSnapshot>>,
    git_again: bool,
    pending: Option<char>,
    count: Option<usize>,
    followed: Option<PathBuf>,
    last_stale_check: Option<Instant>,
    save_signature: u64,
    last_click: Option<(usize, Instant)>,
}

fn load_dir(dir: &Path) -> Listing {
    // mtime first: a change racing the read_dir below then shows up as a
    // newer mtime on the next staleness check instead of being missed.
    let mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != ".git")
        .map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let ft = e.file_type().ok();
            // `d_type` gives the kind for free; only symlinks need a
            // follow-up stat (to sort/expand a link to a directory).
            if ft.is_some_and(|t| t.is_symlink()) {
                let is_dir = std::fs::metadata(&path).is_ok_and(|m| m.is_dir());
                let link = std::fs::read_link(&path).ok();
                Entry {
                    name,
                    path,
                    is_dir,
                    link,
                }
            } else {
                Entry {
                    name,
                    path,
                    is_dir: ft.is_some_and(|t| t.is_dir()),
                    link: None,
                }
            }
        })
        .collect();
    entries.sort_by(|a, b| sort_entries(a.is_dir, &a.name, b.is_dir, &b.name));
    Listing {
        entries: Arc::new(entries),
        mtime,
    }
}

/// Directories first, then a case-insensitive natural order
/// (`file2` before `file10`), ties broken by raw name for stability.
fn sort_entries(a_dir: bool, a: &str, b_dir: bool, b: &str) -> Ordering {
    b_dir
        .cmp(&a_dir)
        .then_with(|| natural_cmp(a, b))
        .then_with(|| a.cmp(b))
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    x.next();
                }
                let mut nb = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(d);
                    y.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta
                    .len()
                    .cmp(&tb.len())
                    .then_with(|| ta.cmp(tb))
                    .then_with(|| na.len().cmp(&nb.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Importance of a git status letter when rolling it up onto directories.
fn git_rank(c: char) -> u8 {
    match c {
        'U' => 6,
        'M' => 5,
        'D' => 4,
        'R' | 'C' => 3,
        'A' => 2,
        '?' => 1,
        _ => 0,
    }
}

fn rails_with(rails: u64, depth: usize, last: bool) -> u64 {
    if depth < 64 && !last {
        rails | (1u64 << depth)
    } else {
        rails
    }
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut t = Self {
            root,
            expanded: BTreeSet::new(),
            cursor: 0,
            top: 0,
            nodes: Vec::new(),
            confirm_delete: None,
            confirm_trash: None,
            clipboard: None,
            show_hidden: false,
            bookmarks: BTreeSet::new(),
            filter: String::new(),
            filter_input: false,
            git_status: HashMap::new(),
            git_dirs: HashMap::new(),
            gitignored: BTreeSet::new(),
            show_ignored: false,
            marked: BTreeSet::new(),
            matched: HashSet::new(),
            show_help: false,
            help_scroll: 0,
            width: 32,
            last_edit_window: 0,
            height: 20,
            cache: HashMap::new(),
            filter_hits: None,
            git_rx: None,
            git_again: false,
            pending: None,
            count: None,
            followed: None,
            last_stale_check: None,
            save_signature: 0,
            last_click: None,
        };
        t.rebuild();
        t
    }

    /// The cached listing for `dir`, reading it on first use.
    fn listing(&mut self, dir: &Path) -> Arc<Vec<Entry>> {
        if let Some(l) = self.cache.get(dir) {
            return l.entries.clone();
        }
        let l = load_dir(dir);
        let entries = l.entries.clone();
        self.cache.insert(dir.to_path_buf(), l);
        entries
    }

    /// Re-reads `dir` if its mtime moved since it was cached. One `stat`.
    fn revalidate(&mut self, dir: &Path) -> bool {
        let Some(cached) = self.cache.get(dir) else {
            return false;
        };
        let now = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        if now == cached.mtime && now.is_some() {
            return false;
        }
        self.cache.insert(dir.to_path_buf(), load_dir(dir));
        true
    }

    /// Drops `dir`'s cached listing so the next flatten re-reads it --
    /// used after the tree's own file operations, which shouldn't depend
    /// on the filesystem's mtime granularity to show up.
    pub fn forget(&mut self, dir: &Path) {
        self.cache.remove(dir);
    }

    /// Re-reads `dir` if it's cached, regardless of its mtime -- for a
    /// directory the filesystem watcher saw change. Doesn't rebuild.
    pub fn invalidate(&mut self, dir: &Path) -> bool {
        if !self.cache.contains_key(dir) {
            return false;
        }
        self.cache.insert(dir.to_path_buf(), load_dir(dir));
        true
    }

    /// Re-`stat`s the root and every expanded directory, re-reading any
    /// that changed on disk. Returns whether the visible tree changed.
    pub fn refresh_stale(&mut self) -> bool {
        self.refresh_stale_unless(|_| false)
    }

    /// `refresh_stale`, skipping directories `covered` says are already
    /// watched (see `Watcher::covers`).
    pub fn refresh_stale_unless(&mut self, covered: impl Fn(&Path) -> bool) -> bool {
        let mut dirs: Vec<PathBuf> = vec![self.root.clone()];
        dirs.extend(
            self.expanded
                .iter()
                .filter(|p| p.starts_with(&self.root) && self.cache.contains_key(*p))
                .cloned(),
        );
        dirs.retain(|d| !covered(d));
        let mut changed = false;
        for d in dirs {
            changed |= self.revalidate(&d);
        }
        if changed {
            self.rebuild();
        }
        changed
    }

    pub fn rebuild(&mut self) {
        let anchor = self.nodes.get(self.cursor).map(|n| n.path.clone());
        let mut out = Vec::with_capacity(self.nodes.len().max(64));
        if self.filter.is_empty() {
            self.matched.clear();
            let root = self.root.clone();
            self.walk(&root, 0, 0, false, &mut out);
        } else {
            self.build_filtered(&mut out);
        }
        self.nodes = out;
        match anchor.and_then(|p| self.nodes.iter().position(|n| n.path == p)) {
            Some(i) => self.cursor = i,
            None => self.cursor = self.cursor.min(self.nodes.len().saturating_sub(1)),
        }
        self.top = self.top.min(self.nodes.len().saturating_sub(1));
    }

    fn visible(&self, e: &Entry) -> bool {
        (self.show_hidden || !e.name.starts_with('.'))
            && (self.show_ignored || !self.gitignored.contains(&e.path))
    }

    fn walk(&mut self, dir: &Path, depth: usize, rails: u64, ignored: bool, out: &mut Vec<Node>) {
        let listing = self.listing(dir);
        let shown: Vec<&Entry> = listing.iter().filter(|e| self.visible(e)).collect();
        let n = shown.len();
        for (i, e) in shown.into_iter().enumerate() {
            let last = i + 1 == n;
            let ignored = ignored || self.gitignored.contains(&e.path);
            let open = e.is_dir && self.expanded.contains(&e.path);
            out.push(Node {
                path: e.path.clone(),
                name: e.name.clone(),
                is_dir: e.is_dir,
                depth,
                open,
                last,
                rails,
                link: e.link.clone(),
                ignored,
            });
            if open && depth < 256 {
                self.walk(
                    &e.path,
                    depth + 1,
                    rails_with(rails, depth, last),
                    ignored,
                    out,
                );
            }
        }
    }

    /// Builds the filtered view: every hit plus its ancestor directories,
    /// in normal tree order (all shown ancestors open).
    fn build_filtered(&mut self, out: &mut Vec<Node>) {
        let root = self.root.clone();
        let hits: Vec<(PathBuf, bool)> = match &self.filter_hits {
            Some(h) => h
                .iter()
                .filter(|p| p.starts_with(&root) && **p != root)
                .map(|p| (p.clone(), false))
                .collect(),
            None => {
                let mut all = Vec::new();
                self.walk(&root, 0, 0, false, &mut all);
                let q = self.filter.clone();
                all.into_iter()
                    .filter(|n| crate::picker::fuzzy_score(&n.name, &q).is_some())
                    .map(|n| (n.path, n.is_dir))
                    .collect()
            }
        };
        self.matched = hits.iter().map(|(p, _)| p.clone()).collect();
        let mut children: HashMap<PathBuf, Vec<(PathBuf, String, bool)>> = HashMap::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        for (path, is_dir) in hits {
            let mut cur = path;
            let mut cur_dir = is_dir;
            loop {
                if !seen.insert(cur.clone()) {
                    break;
                }
                let Some(parent) = cur.parent().map(Path::to_path_buf) else {
                    break;
                };
                let name = cur
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                children
                    .entry(parent.clone())
                    .or_default()
                    .push((cur.clone(), name, cur_dir));
                if parent == root || !parent.starts_with(&root) {
                    break;
                }
                cur = parent;
                cur_dir = true;
            }
        }
        for v in children.values_mut() {
            v.sort_by(|a, b| sort_entries(a.2, &a.1, b.2, &b.1));
        }
        fn emit(
            t: &FileTree,
            dir: &Path,
            depth: usize,
            rails: u64,
            children: &HashMap<PathBuf, Vec<(PathBuf, String, bool)>>,
            out: &mut Vec<Node>,
        ) {
            let Some(kids) = children.get(dir) else {
                return;
            };
            let n = kids.len();
            for (i, (path, name, is_dir)) in kids.iter().enumerate() {
                let last = i + 1 == n;
                let open = *is_dir && children.contains_key(path);
                out.push(Node {
                    path: path.clone(),
                    name: name.clone(),
                    is_dir: *is_dir,
                    depth,
                    open,
                    last,
                    rails,
                    link: None,
                    ignored: t.gitignored.contains(path),
                });
                if open {
                    emit(
                        t,
                        path,
                        depth + 1,
                        rails_with(rails, depth, last),
                        children,
                        out,
                    );
                }
            }
        }
        emit(self, &root, 0, 0, &children, out);
    }

    /// Installs project-wide filter hits (best first) and puts the cursor
    /// on the best one.
    fn set_filter_hits(&mut self, hits: Option<Vec<PathBuf>>) {
        let best = hits.as_ref().and_then(|h| h.first().cloned());
        self.filter_hits = hits;
        self.rebuild();
        let best = best.or_else(|| {
            // Fallback mode: the first actual hit in tree order.
            self.nodes
                .iter()
                .find(|n| self.matched.contains(&n.path))
                .map(|n| n.path.clone())
        });
        if let Some(i) = best.and_then(|b| self.nodes.iter().position(|n| n.path == b)) {
            self.cursor = i;
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.nodes.is_empty() {
            return;
        }
        let last = self.nodes.len() - 1;
        self.cursor = if delta < 0 {
            self.cursor.saturating_sub(delta.unsigned_abs())
        } else {
            (self.cursor + delta as usize).min(last)
        };
    }

    /// Clamp the scroll offset so `cursor` is visible within a list of
    /// `height` rows. Called from the render pipeline, which knows the pane
    /// height; keyboard motion only moves `cursor`, and the view follows here.
    pub fn ensure_visible(&mut self, height: usize) {
        let height = height.max(1);
        self.height = height;
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + height {
            self.top = self.cursor + 1 - height;
        }
        let max_top = self.nodes.len().saturating_sub(height);
        self.top = self.top.min(max_top);
    }

    /// Mouse-wheel scroll by `delta` rows within a list of `height` rows,
    /// pulling `cursor` back into the visible window so it never strands
    /// off-screen (matching Ctrl-E/Ctrl-Y on a buffer).
    pub fn scroll(&mut self, delta: isize, height: usize) {
        if self.nodes.is_empty() {
            return;
        }
        let height = height.max(1);
        let max_top = self.nodes.len().saturating_sub(height);
        self.top = (self.top as isize + delta).clamp(0, max_top as isize) as usize;
        let last_visible = (self.top + height - 1).min(self.nodes.len() - 1);
        self.cursor = self.cursor.clamp(self.top, last_visible);
    }

    /// No tree-local key sequence or prompt is in flight, so a leader key
    /// may start an editor-wide leader sequence instead.
    pub fn leader_ok(&self) -> bool {
        !self.filter_input && self.pending.is_none() && self.count.is_none() && !self.show_help
    }

    fn selected(&self) -> Option<&Node> {
        self.nodes.get(self.cursor)
    }

    /// Enter/`o` on a directory toggles it; on a file, returns its path
    /// for the caller to open.
    fn activate(&mut self) -> Option<PathBuf> {
        let node = self.nodes.get(self.cursor)?.clone();
        if node.is_dir {
            if !self.filter.is_empty() {
                // In the filtered view a directory is just context; Enter
                // on it drops the filter and opens it in the real tree.
                self.clear_filter();
                self.expanded.insert(node.path.clone());
                self.reveal(&node.path);
                return None;
            }
            if !self.expanded.remove(&node.path) {
                self.revalidate(&node.path);
                self.expanded.insert(node.path.clone());
            }
            self.rebuild();
            if let Some(i) = self.nodes.iter().position(|n| n.path == node.path) {
                self.cursor = i;
            }
            None
        } else {
            Some(node.path)
        }
    }

    /// `l`/Right: expand a collapsed directory, step into an expanded one,
    /// or open a file.
    fn expand_or_enter(&mut self) -> Option<PathBuf> {
        let node = self.nodes.get(self.cursor)?.clone();
        if node.is_dir && node.open {
            if self
                .nodes
                .get(self.cursor + 1)
                .is_some_and(|n| n.depth > node.depth)
            {
                self.cursor += 1;
            }
            return None;
        }
        self.activate()
    }

    /// `h`/Left: collapse the current directory, or if it's already
    /// collapsed (or this is a file), jump to its parent.
    fn collapse_or_parent(&mut self) {
        let Some(node) = self.nodes.get(self.cursor).cloned() else {
            return;
        };
        if node.is_dir && self.filter.is_empty() && self.expanded.remove(&node.path) {
            self.rebuild();
            if let Some(i) = self.nodes.iter().position(|n| n.path == node.path) {
                self.cursor = i;
            }
            return;
        }
        self.goto_parent();
    }

    fn goto_parent(&mut self) {
        let Some(node) = self.nodes.get(self.cursor) else {
            return;
        };
        let Some(parent) = node.path.parent() else {
            return;
        };
        if let Some(i) = self.nodes.iter().position(|n| n.path == parent) {
            self.cursor = i;
        }
    }

    /// `K`/`J`: first/last sibling of the cursor's node.
    fn goto_sibling_edge(&mut self, last: bool) {
        let Some(depth) = self.selected().map(|n| n.depth) else {
            return;
        };
        let mut i = self.cursor;
        if last {
            let mut j = i + 1;
            while let Some(n) = self.nodes.get(j) {
                if n.depth < depth {
                    break;
                }
                if n.depth == depth {
                    i = j;
                }
                j += 1;
            }
        } else {
            while i > 0 && self.nodes[i - 1].depth >= depth {
                i -= 1;
            }
        }
        self.cursor = i;
    }

    /// Where a new file/dir created "at" the cursor should live: inside
    /// the selected directory, or alongside the selected file.
    fn target_dir(&self) -> PathBuf {
        match self.nodes.get(self.cursor) {
            Some(n) if n.is_dir => n.path.clone(),
            Some(n) => n.path.parent().map_or(self.root.clone(), Path::to_path_buf),
            None => self.root.clone(),
        }
    }

    /// The nodes a bulk-capable operation acts on: every mark, or else
    /// just the cursor's node.
    fn targets(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            self.selected()
                .map(|n| vec![n.path.clone()])
                .unwrap_or_default()
        } else {
            self.marked.iter().cloned().collect()
        }
    }

    /// Expands every ancestor of `file` and moves the cursor onto it.
    pub fn reveal(&mut self, file: &Path) {
        let Ok(rel) = file.strip_prefix(&self.root) else {
            return;
        };
        let mut cur = self.root.clone();
        for comp in rel.components() {
            cur.push(comp);
            if cur != file {
                self.expanded.insert(cur.clone());
            }
        }
        self.rebuild();
        if let Some(i) = self.nodes.iter().position(|n| n.path == file) {
            self.cursor = i;
        }
    }

    fn clear_filter(&mut self) {
        self.filter.clear();
        self.filter_input = false;
        self.filter_hits = None;
        self.matched.clear();
    }

    /// `W`: collapse everything, keeping the cursor on the top-level
    /// ancestor of where it was.
    fn collapse_all(&mut self) {
        let anchor = self.selected().map(|n| n.path.clone());
        self.expanded.retain(|p| !p.starts_with(&self.root));
        self.rebuild();
        if let Some(rel) = anchor
            .as_ref()
            .and_then(|a| a.strip_prefix(&self.root).ok())
        {
            if let Some(first) = rel.components().next() {
                let top = self.root.join(first);
                if let Some(i) = self.nodes.iter().position(|n| n.path == top) {
                    self.cursor = i;
                }
            }
        }
    }

    /// `E`: recursively expand `dir`, skipping hidden/ignored directories
    /// that the current view hides anyway, symlinks (cycles), and
    /// stopping at `EXPAND_ALL_LIMIT` loaded entries. Returns (dirs
    /// expanded, whether the limit cut it short).
    fn expand_all(&mut self, dir: &Path) -> (usize, bool) {
        let mut stack = vec![dir.to_path_buf()];
        let mut loaded = 0;
        let mut dirs = 0;
        while let Some(d) = stack.pop() {
            if loaded >= EXPAND_ALL_LIMIT {
                return (dirs, true);
            }
            self.revalidate(&d);
            self.expanded.insert(d.clone());
            dirs += 1;
            let listing = self.listing(&d);
            loaded += listing.len();
            for e in listing.iter().rev() {
                if e.is_dir && e.link.is_none() && self.visible(e) {
                    stack.push(e.path.clone());
                }
            }
        }
        (dirs, false)
    }

    fn set_root(&mut self, root: PathBuf) {
        let anchor = self.selected().map(|n| n.path.clone());
        self.root = root;
        self.clear_filter();
        self.cursor = 0;
        self.top = 0;
        self.followed = None;
        self.rebuild();
        if let Some(a) = anchor.filter(|a| a.starts_with(&self.root)) {
            self.reveal(&a);
        }
    }

    /// Applies a finished background `git status`.
    fn apply_git(&mut self, (status, ignored): GitSnapshot) {
        if let Some(status) = status {
            let mut dirs: HashMap<PathBuf, char> = HashMap::new();
            for (path, &c) in &status {
                let rank = git_rank(c);
                let mut cur = path.parent();
                while let Some(d) = cur {
                    match dirs.get(d) {
                        Some(&have) if git_rank(have) >= rank => break,
                        _ => {
                            dirs.insert(d.to_path_buf(), c);
                        }
                    }
                    cur = d.parent();
                }
            }
            self.git_status = status;
            self.git_dirs = dirs;
        }
        if let Some(ignored) = ignored {
            if ignored != self.gitignored {
                self.gitignored = ignored;
                self.rebuild();
            }
        }
    }

    /// The git letter shown for `node`: its own status, or for a directory
    /// the roll-up of its descendants.
    pub fn git_marker(&self, path: &Path, is_dir: bool) -> Option<char> {
        if is_dir {
            self.git_dirs.get(path).copied()
        } else {
            self.git_status.get(path).copied()
        }
    }
}

/// A pane that files can be opened into (not a sidebar, terminal or
/// Markdown preview).
fn is_edit_window(w: &Window) -> bool {
    !w.file_tree && !w.outline && w.terminal.is_none() && !w.preview
}

impl Editor {
    pub fn active_file_tree(&self) -> bool {
        self.windows
            .get(self.active_window)
            .is_some_and(|w| w.file_tree)
    }

    fn file_tree_window(&self) -> Option<usize> {
        self.windows.iter().position(|w| w.file_tree)
    }

    /// `,ft` / `:tree`: opens the sidebar (focused, revealing the current
    /// file), or closes it if it's already open.
    pub fn toggle_file_tree(&mut self) {
        if let Some(idx) = self.file_tree_window() {
            self.close_file_tree(idx);
        } else {
            self.open_file_tree();
        }
    }

    /// `,e`: open the tree, focus it if it's open but unfocused, or close it
    /// when it already has focus -- one key to get in and out.
    pub fn focus_or_toggle_file_tree(&mut self) {
        match self.file_tree_window() {
            Some(idx) if idx == self.active_window => self.close_file_tree(idx),
            Some(idx) => {
                let path = self.buf().path.clone();
                self.focus_window(idx);
                if let (Some(t), Some(p)) = (&mut self.file_tree, path) {
                    if t.filter.is_empty() {
                        t.reveal(&p);
                    }
                }
            }
            None => self.open_file_tree(),
        }
    }

    fn open_file_tree(&mut self) {
        if self.windows.len() >= 32 {
            self.set_message("At most 32 panes are supported");
            return;
        }
        let current_file = self.buf().path.clone();
        let width = self.config.tree_width.max(10);
        let project_root = self.project_root.clone();
        let tree = self.file_tree.get_or_insert_with(|| {
            let mut t = FileTree::new(project_root);
            t.width = width;
            t
        });
        if let Some(path) = current_file {
            if tree.filter.is_empty() {
                tree.reveal(&path);
            }
            tree.followed = Some(path);
        }
        self.store_window();
        if self.windows.is_empty() {
            self.windows.push(self.capture_window());
            self.window_layout = Some(Layout::Leaf(0));
        }
        let editor_window = self.active_window;
        let idx = self.windows.len();
        let mut w = self.capture_window();
        w.file_tree = true;
        self.windows.push(w);
        let rest = self.window_layout.take().unwrap_or(Layout::Leaf(0));
        self.window_layout = Some(self.sidebar_layout(idx, rest));
        if let Some(t) = &mut self.file_tree {
            t.last_edit_window = editor_window;
        }
        self.focus_window(idx);
        self.refresh_tree_git_status();
        self.start_file_scan();
    }

    /// `rest` with the tree's leaf pinned beside it at the sidebar width.
    fn sidebar_layout(&self, tree_leaf: usize, rest: Layout) -> Layout {
        let width = self.file_tree.as_ref().map_or(32, |t| t.width);
        let leaf = Box::new(Layout::Leaf(tree_leaf));
        if self.config.tree_position == "right" {
            Layout::Split {
                vertical: true,
                first: Box::new(rest),
                second: leaf,
                ratio: 0.5,
                fixed: Some((true, width)),
            }
        } else {
            Layout::Split {
                vertical: true,
                first: leaf,
                second: Box::new(rest),
                ratio: 0.5,
                fixed: Some((false, width)),
            }
        }
    }

    fn close_file_tree(&mut self, idx: usize) {
        // Remember a mouse-dragged width for the next open.
        if let Some(w) = self.window_layout.as_ref().and_then(|l| l.fixed_size(idx)) {
            if let Some(t) = &mut self.file_tree {
                t.width = w;
            }
        }
        let back = self
            .file_tree
            .as_ref()
            .map(|t| t.last_edit_window)
            .filter(|&i| i != idx && self.windows.get(i).is_some_and(is_edit_window));
        if self.windows.len() == 1 {
            // The tree is all that's left: fall back to the plain
            // single-pane view of the current buffer.
            self.windows.clear();
            self.window_layout = None;
            self.active_window = 0;
            self.enter_normal();
            return;
        }
        self.active_window = idx;
        self.close_window();
        if self.windows.len() > 1 {
            if let Some(b) = back {
                self.focus_window(if b > idx { b - 1 } else { b });
            }
        }
        self.enter_normal();
    }

    /// The editing pane last used beside the tree, if any is open.
    fn existing_edit_window(&self) -> Option<usize> {
        let preferred = self.file_tree.as_ref().map(|t| t.last_edit_window);
        preferred
            .filter(|&i| self.windows.get(i).is_some_and(is_edit_window))
            .or_else(|| self.windows.iter().position(is_edit_window))
    }

    /// The pane the tree opens files into, creating one beside the tree if
    /// the tree is the only pane left.
    fn tree_edit_window(&mut self) -> Option<usize> {
        if let Some(i) = self.existing_edit_window() {
            return Some(i);
        }
        let tree_idx = self.file_tree_window()?;
        if self.windows.len() >= 32 {
            return None;
        }
        // Rebuild the layout as [tree | new editing pane], keeping any
        // other (sidebar/terminal) panes in the rest of the space.
        let idx = self.windows.len();
        let mut w = self.capture_window();
        w.file_tree = false;
        w.outline = false;
        w.terminal = None;
        w.preview = false;
        self.windows.push(w);
        let layout = self.window_layout.take().unwrap_or(Layout::Leaf(tree_idx));
        let rest = layout
            .remove_keep_index(tree_idx)
            .map(|l| Layout::Split {
                vertical: false,
                first: Box::new(Layout::Leaf(idx)),
                second: Box::new(l),
                ratio: 0.7,
                fixed: None,
            })
            .unwrap_or(Layout::Leaf(idx));
        self.window_layout = Some(self.sidebar_layout(tree_idx, rest));
        Some(idx)
    }

    /// Moves focus from the tree to the pane files open into.
    pub(crate) fn focus_tree_edit_window(&mut self) {
        if let Some(w) = self.tree_edit_window() {
            self.focus_window(w);
        }
    }

    /// Opens `path` from the tree: in the last-used editing pane, a new
    /// split of it, or a new tab. `keep_focus` (`Tab`, preview) returns
    /// focus to the tree afterwards.
    pub fn tree_open(&mut self, path: PathBuf, how: OpenIn, keep_focus: bool) {
        let Some(target) = self.tree_edit_window() else {
            self.set_message("No pane to open the file in");
            return;
        };
        self.focus_window(target);
        match how {
            OpenIn::VSplit => self.split_window(true, false),
            OpenIn::HSplit => self.split_window(false, false),
            OpenIn::Tab => self.new_tab(),
            OpenIn::Edit => {}
        }
        if let Err(e) = self.open_file(path.clone()) {
            self.set_message(e.to_string());
        }
        self.store_window();
        let edit = self.active_window;
        if let Some(t) = &mut self.file_tree {
            t.followed = Some(path);
            if how != OpenIn::Tab {
                t.last_edit_window = edit;
            }
        }
        if keep_focus && how != OpenIn::Tab {
            if let Some(idx) = self.file_tree_window() {
                self.focus_window(idx);
            }
        }
    }

    pub(crate) fn open_from_tree(&mut self, path: PathBuf) {
        self.tree_open(path, OpenIn::Edit, false);
    }

    /// Re-runs `git status` (and `--ignored`) for the tree's decorations
    /// and `.gitignore` filter on a background thread; the result lands
    /// via `poll_file_tree`. Outside a Git repo, or without `git`, it
    /// silently leaves both empty.
    pub fn refresh_tree_git_status(&mut self) {
        let root = self.project_root.clone();
        let Some(t) = &mut self.file_tree else {
            return;
        };
        if t.git_rx.is_some() {
            t.git_again = true;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        t.git_rx = Some(rx);
        std::thread::spawn(move || {
            let status = crate::git_tools::status(&root).ok();
            let ignored = crate::git_tools::ignored(&root).ok();
            let _ = tx.send((status, ignored));
        });
    }

    /// Blocks until the in-flight git refresh (if any) has been applied.
    #[cfg(test)]
    pub fn wait_tree_git(&mut self) {
        loop {
            let Some(rx) = self.file_tree.as_mut().and_then(|t| t.git_rx.take()) else {
                return;
            };
            let snap = rx.recv_timeout(Duration::from_secs(20)).ok();
            let again = self.file_tree.as_mut().map(|t| {
                if let Some(s) = snap {
                    t.apply_git(s);
                }
                std::mem::take(&mut t.git_again)
            });
            if again == Some(true) {
                self.refresh_tree_git_status();
            }
        }
    }

    /// Per-tick housekeeping, from the idle loop: lands a finished git
    /// refresh, re-reads directories changed on disk (about once a second,
    /// only while the tree is visible, and only those the filesystem
    /// watcher doesn't cover -- see `src/watcher.rs`), and re-runs git
    /// status after a save. Returns whether anything visible changed.
    pub fn poll_file_tree(&mut self) -> bool {
        let mut changed = false;
        let mut again = false;
        if let Some(t) = &mut self.file_tree {
            let received = match t.git_rx.as_ref().map(Receiver::try_recv) {
                Some(Ok(snap)) => Some(Some(snap)),
                Some(Err(TryRecvError::Disconnected)) => Some(None),
                _ => None,
            };
            if let Some(snap) = received {
                t.git_rx = None;
                if let Some(snap) = snap {
                    t.apply_git(snap);
                    changed = true;
                }
                again = std::mem::take(&mut t.git_again);
            }
        }
        if again {
            self.refresh_tree_git_status();
        }
        if self.file_tree_window().is_none() {
            return changed;
        }
        // A save (a buffer going modified -> clean) changes `git status`.
        let signature = self
            .buffers
            .iter()
            .filter(|b| b.is_modified())
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b.id + 1));
        let mut regit = false;
        let watcher = self.watcher.as_ref();
        if let Some(t) = &mut self.file_tree {
            if t.save_signature != signature {
                t.save_signature = signature;
                regit = true;
            }
            if t.last_stale_check
                .is_none_or(|i| i.elapsed() >= STALE_CHECK)
            {
                t.last_stale_check = Some(Instant::now());
                if t.refresh_stale_unless(|d| watcher.is_some_and(|w| w.covers(d))) {
                    changed = true;
                    regit = true;
                }
            }
        }
        if regit {
            self.refresh_tree_git_status();
        }
        changed
    }

    /// Called every frame: remembers the editing pane last focused (files
    /// open there) and, with `tree_follow`, reveals the current buffer's
    /// file whenever it changes.
    pub fn sync_file_tree(&mut self) {
        if self.file_tree_window().is_none() {
            return;
        }
        let active = self.active_window;
        if !self.windows.get(active).is_some_and(is_edit_window) {
            return;
        }
        let path = self.buf().path.clone();
        let follow = self.config.tree_follow;
        let Some(t) = &mut self.file_tree else {
            return;
        };
        t.last_edit_window = active;
        if let Some(p) = path {
            if follow && t.filter.is_empty() && t.followed.as_ref() != Some(&p) {
                t.followed = Some(p.clone());
                if p.starts_with(&t.root) {
                    t.reveal(&p);
                }
            }
        }
    }

    /// Re-runs the `/` filter: project-wide over the background file
    /// inventory when it's ready, else over the loaded nodes.
    pub fn refilter_tree(&mut self) {
        let ready = self.search_job.files_ready;
        let project_root = self.project_root.clone();
        let Some(t) = &self.file_tree else {
            return;
        };
        let query = t.filter.clone();
        if query.is_empty() || !ready {
            if let Some(t) = &mut self.file_tree {
                t.set_filter_hits(None);
            }
            return;
        }
        let (tree_root, show_hidden) = (t.root.clone(), t.show_hidden);
        let mut scored: Vec<(i64, &String)> = self
            .all_files
            .iter()
            .filter(|f| show_hidden || !f.split('/').any(|c| c.starts_with('.')))
            .filter_map(|f| crate::picker::fuzzy_score(f, &query).map(|s| (s, f)))
            .collect();
        let take = FILTER_TAKE.min(scored.len());
        if take > 0 && scored.len() > take {
            scored.select_nth_unstable_by(take - 1, |a, b| b.0.cmp(&a.0));
            scored.truncate(take);
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        let hits: Vec<PathBuf> = scored
            .into_iter()
            .map(|(_, f)| project_root.join(f))
            .filter(|p| p.starts_with(&tree_root))
            .collect();
        if let Some(t) = &mut self.file_tree {
            t.set_filter_hits(Some(hits));
        }
    }

    /// Marks the project file inventory stale after the tree changed the
    /// filesystem, so the next `/` (and the file picker) sees it.
    pub(crate) fn invalidate_project_files(&mut self) {
        if self.search_job.files_rx.is_none() {
            self.search_job.files_ready = false;
        }
    }

    /// After a filesystem operation: re-read the touched directories,
    /// rebuild, and refresh git decorations and the file inventory.
    fn after_tree_fs_change(&mut self, touched: &[PathBuf]) {
        if let Some(t) = &mut self.file_tree {
            for p in touched {
                t.forget(p);
                if let Some(parent) = p.parent() {
                    t.forget(parent);
                }
            }
            t.rebuild();
        }
        self.invalidate_project_files();
        self.refresh_tree_git_status();
    }

    /// `:treenew name` (or `name/` for a directory; `a/b/c.rs` creates the
    /// missing parents): creates it inside the tree cursor's directory (or
    /// alongside its file), refusing to overwrite an existing path.
    pub fn tree_new(&mut self, name: &str) {
        let Some(tree) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        if name.is_empty() {
            self.set_message("Usage: treenew <name> (trailing / for a directory)");
            return;
        }
        if !is_safe_relative_path(name) {
            self.set_message("Name must stay inside this directory (no '..' or absolute path)");
            return;
        }
        let is_dir = name.ends_with('/') || name.ends_with(std::path::MAIN_SEPARATOR);
        let target = tree.target_dir().join(name.trim_end_matches('/'));
        if target.exists() {
            self.set_message(format!("{} already exists", target.display()));
            return;
        }
        let result = if is_dir {
            std::fs::create_dir_all(&target)
        } else {
            target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&target, b""))
        };
        match result {
            Ok(()) => {
                let mut touched = vec![target.clone()];
                touched.extend(target.ancestors().skip(1).map(Path::to_path_buf).take(16));
                self.after_tree_fs_change(&touched);
                if let Some(t) = &mut self.file_tree {
                    if !t.filter.is_empty() {
                        t.clear_filter();
                    }
                    t.reveal(&target);
                }
                self.set_message(format!("Created {}", target.display()));
            }
            Err(e) => self.set_message(format!("Create failed: {e}")),
        }
    }

    /// `:treerename name`: renames the tree cursor's node within its
    /// current directory. Refuses if the target exists, or if the file is
    /// open with unsaved changes.
    pub fn tree_rename(&mut self, name: &str) {
        let Some(tree) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        let Some(node) = tree.nodes.get(tree.cursor).cloned() else {
            self.set_message("No file tree selection");
            return;
        };
        if name.is_empty() {
            self.set_message("Usage: treerename <new-name>");
            return;
        }
        if !is_safe_tree_name(name) {
            self.set_message(
                "New name must stay in the same directory (no '/', '..', or absolute path)",
            );
            return;
        }
        let new_path = node.path.parent().unwrap_or(&tree.root).join(name);
        if new_path == node.path {
            return;
        }
        if new_path.exists() && !same_file_case_rename(&node.path, &new_path) {
            self.set_message(format!("{} already exists", new_path.display()));
            return;
        }
        if self.has_dirty_buffer_under(&node.path) {
            self.set_message("Cannot rename: open buffer has unsaved changes");
            return;
        }
        match std::fs::rename(&node.path, &new_path) {
            Ok(()) => {
                // Remap the renamed node itself and -- when it's a
                // directory -- every open buffer nested under it, so a
                // buffer for a file inside the moved directory keeps
                // pointing at its real new location instead of a stale
                // path a later `:w!` would recreate.
                self.remap_buffer_paths(&node.path, &new_path);
                if let Some(t) = &mut self.file_tree {
                    remap_set(&mut t.expanded, &node.path, &new_path);
                    remap_set(&mut t.bookmarks, &node.path, &new_path);
                    t.marked.clear();
                }
                self.after_tree_fs_change(&[node.path.clone(), new_path.clone()]);
                if let Some(t) = &mut self.file_tree {
                    t.reveal(&new_path);
                }
                self.set_message(format!("Renamed to {}", new_path.display()));
            }
            Err(e) => self.set_message(format!("Rename failed: {e}")),
        }
    }

    fn remap_buffer_paths(&mut self, from: &Path, to: &Path) {
        for b in &mut self.buffers {
            let Some(bp) = b.path.clone() else { continue };
            if bp == from {
                b.path = Some(to.to_path_buf());
            } else if let Ok(rel) = bp.strip_prefix(from) {
                b.path = Some(to.join(rel));
            }
        }
    }

    /// True if any open buffer under `target` (a directory delete/trash
    /// may contain several) has unsaved changes.
    fn has_dirty_buffer_under(&self, target: &Path) -> bool {
        self.buffers.iter().any(|b| {
            b.is_modified()
                && b.path
                    .as_ref()
                    .is_some_and(|p| p == target || p.starts_with(target))
        })
    }

    /// Permanently deletes `targets`. Refuses (deleting nothing) if any
    /// open buffer under one of them has unsaved changes.
    fn tree_delete_confirmed(&mut self, targets: &[PathBuf]) {
        if targets.iter().any(|t| self.has_dirty_buffer_under(t)) {
            self.set_message("Cannot delete: an open buffer under it has unsaved changes");
            return;
        }
        let mut done = Vec::new();
        let mut error = None;
        for target in targets {
            let result = if target.is_dir() && !target.is_symlink() {
                std::fs::remove_dir_all(target)
            } else {
                std::fs::remove_file(target)
            };
            match result {
                Ok(()) => done.push(target.clone()),
                Err(e) => {
                    error = Some(format!("Delete failed for {}: {e}", target.display()));
                    break;
                }
            }
        }
        self.finish_removal(&done);
        self.set_message(error.unwrap_or_else(|| match done.as_slice() {
            [one] => format!("Deleted {}", one.display()),
            many => format!("Deleted {} items", many.len()),
        }));
    }

    /// Moves `targets` into `.vaayu/trash/` instead of removing them -- a
    /// reversible alternative to delete for the "I didn't mean that" case.
    /// Refuses under the same dirty-buffer condition as a real delete.
    fn tree_trash_confirmed(&mut self, targets: &[PathBuf]) {
        if targets.iter().any(|t| self.has_dirty_buffer_under(t)) {
            self.set_message("Cannot trash: an open buffer under it has unsaved changes");
            return;
        }
        let trash_dir = self.project_root.join(".vaayu").join("trash");
        if let Err(e) = std::fs::create_dir_all(&trash_dir) {
            self.set_message(format!("Trash failed: {e}"));
            return;
        }
        let mut done = Vec::new();
        let mut error = None;
        for (i, target) in targets.iter().enumerate() {
            let name = target.file_name().unwrap_or_default().to_string_lossy();
            let stamp = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let suffix = if i == 0 {
                String::new()
            } else {
                format!("-{i}")
            };
            let dest = trash_dir.join(format!("{stamp}{suffix}-{name}"));
            match std::fs::rename(target, &dest) {
                Ok(()) => done.push(target.clone()),
                Err(e) => {
                    error = Some(format!("Trash failed for {}: {e}", target.display()));
                    break;
                }
            }
        }
        self.finish_removal(&done);
        self.set_message(error.unwrap_or_else(|| match done.as_slice() {
            [one] => format!("Trashed {} (in .vaayu/trash/)", one.display()),
            many => format!("Trashed {} items (in .vaayu/trash/)", many.len()),
        }));
    }

    fn finish_removal(&mut self, removed: &[PathBuf]) {
        if let Some(t) = &mut self.file_tree {
            for r in removed {
                t.expanded.retain(|p| !p.starts_with(r));
                t.marked.retain(|p| !p.starts_with(r));
            }
            if !t.filter.is_empty() {
                if let Some(h) = &mut t.filter_hits {
                    h.retain(|p| !removed.iter().any(|r| p.starts_with(r)));
                }
            }
        }
        self.after_tree_fs_change(removed);
    }

    /// `y`: copies the targets to the tree clipboard (`p` pastes them,
    /// leaving the originals in place).
    pub fn tree_yank(&mut self) {
        self.tree_clip(false);
    }

    /// `x`: marks the targets to be moved on the next `p`.
    pub fn tree_cut(&mut self) {
        self.tree_clip(true);
    }

    fn tree_clip(&mut self, cut: bool) {
        let Some(t) = &mut self.file_tree else {
            return;
        };
        let targets = t.targets();
        if targets.is_empty() {
            return;
        }
        t.marked.clear();
        let what = match targets.as_slice() {
            [one] => one.display().to_string(),
            many => format!("{} items", many.len()),
        };
        t.clipboard = Some((targets, cut));
        self.set_message(if cut {
            format!("Cut {what} (p moves it here)")
        } else {
            format!("Copied {what} (p to paste)")
        });
    }

    /// `p`: pastes the clipboard into the cursor's target directory.
    /// Every item is checked before anything is touched: a name
    /// collision, a vanished source, pasting a directory into itself or a
    /// descendant, or (for a cut) an unsaved buffer under a source refuses
    /// the whole paste. A move repoints open buffers nested inside it.
    pub fn tree_paste(&mut self) {
        let Some(tree) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        let Some((srcs, cut)) = tree.clipboard.clone() else {
            self.set_message("Nothing to paste -- y or x a node first");
            return;
        };
        let dir = tree.target_dir();
        let mut plan = Vec::new();
        for src in &srcs {
            if !src.exists() && !src.is_symlink() {
                self.set_message(format!("{} no longer exists", src.display()));
                return;
            }
            let dest = dir.join(src.file_name().unwrap_or_default());
            if dest == *src {
                self.set_message("Source and destination are the same");
                return;
            }
            // A directory can't be pasted into itself or one of its own
            // descendants: the copy would keep finding its own output.
            if dest.starts_with(src) {
                self.set_message("Cannot paste a directory into itself or a descendant");
                return;
            }
            if dest.exists() || plan.iter().any(|(_, d)| d == &dest) {
                self.set_message(format!("{} already exists", dest.display()));
                return;
            }
            if cut && self.has_dirty_buffer_under(src) {
                self.set_message("Cannot move: an open buffer under it has unsaved changes");
                return;
            }
            plan.push((src.clone(), dest));
        }
        let mut touched = vec![dir.clone()];
        let mut last = None;
        for (src, dest) in &plan {
            let result = if cut {
                std::fs::rename(src, dest)
            } else {
                copy_recursive(src, dest)
            };
            if let Err(e) = result {
                self.after_tree_fs_change(&touched);
                self.set_message(format!("Paste failed: {e}"));
                return;
            }
            if cut {
                self.remap_buffer_paths(src, dest);
                if let Some(t) = &mut self.file_tree {
                    remap_set(&mut t.expanded, src, dest);
                    remap_set(&mut t.bookmarks, src, dest);
                }
            }
            touched.push(src.clone());
            touched.push(dest.clone());
            last = Some(dest.clone());
        }
        self.after_tree_fs_change(&touched);
        self.set_message(format!(
            "{} {}",
            if cut { "Moved" } else { "Copied" },
            match plan.as_slice() {
                [(_, d)] => format!("to {}", d.display()),
                many => format!("{} items to {}", many.len(), dir.display()),
            }
        ));
        if let Some(t) = &mut self.file_tree {
            if cut {
                t.clipboard = None;
            }
            if let Some(dest) = last {
                if !t.filter.is_empty() {
                    t.clear_filter();
                }
                t.reveal(&dest);
            }
        }
    }

    /// `m`: toggles the tree cursor's node as a bookmark.
    pub fn tree_toggle_bookmark(&mut self) {
        let Some(t) = &mut self.file_tree else {
            return;
        };
        let Some(path) = t.selected().map(|n| n.path.clone()) else {
            return;
        };
        if t.bookmarks.remove(&path) {
            self.set_message(format!("Unbookmarked {}", path.display()));
        } else {
            t.bookmarks.insert(path.clone());
            self.set_message(format!("Bookmarked {}", path.display()));
        }
    }

    /// Shows the live filter's current query and match count.
    fn report_tree_filter(&mut self) {
        let Some(t) = &self.file_tree else { return };
        let n = t.matched.len();
        let scope = if t.filter_hits.is_some() {
            "project"
        } else {
            "loaded nodes"
        };
        self.set_message(format!(
            "Filter ({scope}): {}  ({n} match{})",
            t.filter,
            if n == 1 { "" } else { "es" }
        ));
    }

    /// `:treebookmarks`: lists the current tree's bookmarks as a Results
    /// list; Enter on a directory reveals it in the tree, on a file opens
    /// it into the adjacent pane like the tree's own Enter does.
    pub fn show_tree_bookmarks(&mut self) {
        let Some(t) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        if t.bookmarks.is_empty() {
            self.set_message("No bookmarks -- m in the tree to add one");
            return;
        }
        let root = self.project_root.clone();
        let entries = t
            .bookmarks
            .iter()
            .map(|p| {
                let rel = p.strip_prefix(&root).unwrap_or(p).display().to_string();
                let mut e = crate::results::Entry::text(rel);
                e.action = Some(serde_json::json!({
                    "_vaayu_tree_bookmark": p.display().to_string(),
                    "dir": p.is_dir(),
                }));
                e
            })
            .collect();
        self.show_results(crate::results::Results::new("Tree bookmarks", entries));
    }

    /// Opens (or reveals, for a directory) a `:treebookmarks` selection.
    /// Routed through `open_from_tree`/`tree_reveal` rather than the
    /// generic path-entry branch of `open_result` -- with the tree pane
    /// still `active_window`, the newly opened buffer would never actually
    /// become visible in either pane.
    pub(crate) fn open_tree_bookmark(&mut self, path: &Path, is_dir: bool) {
        if is_dir {
            self.tree_reveal(path);
        } else {
            self.open_from_tree(path.to_path_buf());
        }
    }

    /// Reveals `path` in the file tree, opening the sidebar first if it
    /// isn't already open.
    pub fn tree_reveal(&mut self, path: &Path) {
        match self.file_tree_window() {
            None => self.open_file_tree(),
            Some(idx) => self.focus_window(idx),
        }
        if let Some(t) = &mut self.file_tree {
            if !t.filter.is_empty() {
                t.clear_filter();
            }
            if !path.starts_with(&t.root) {
                if let Some(parent) = path.parent() {
                    t.set_root(parent.to_path_buf());
                }
            }
            t.reveal(path);
        }
    }

    /// `:treefind` / `F`: reveal the current buffer's file in the tree.
    pub fn tree_find_current(&mut self) {
        let path = self
            .windows
            .get(self.file_tree.as_ref().map_or(0, |t| t.last_edit_window))
            .filter(|w| is_edit_window(w))
            .and_then(|w| self.buffers.iter().find(|b| b.id == w.buffer))
            .and_then(|b| b.path.clone())
            .or_else(|| self.buf().path.clone());
        match path {
            Some(p) => self.tree_reveal(&p),
            None => self.set_message("Current buffer has no file"),
        }
    }

    /// `:treeroot [dir]`: re-roots the tree (no argument: the project root).
    pub fn tree_set_root(&mut self, arg: &str) {
        let dir = if arg.is_empty() {
            self.project_root.clone()
        } else {
            let p = PathBuf::from(shellexpand_home(arg));
            if p.is_absolute() {
                p
            } else {
                self.file_tree
                    .as_ref()
                    .map_or(self.project_root.clone(), |t| t.root.clone())
                    .join(p)
            }
        };
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        if !dir.is_dir() {
            self.set_message(format!("Not a directory: {}", dir.display()));
            return;
        }
        if self.file_tree_window().is_none() {
            self.open_file_tree();
        }
        if let Some(t) = &mut self.file_tree {
            t.set_root(dir.clone());
        }
        self.set_message(format!("Tree root: {}", dir.display()));
    }

    fn tree_set_width(&mut self, width: usize) {
        let width = width.clamp(12, 200);
        if let Some(t) = &mut self.file_tree {
            t.width = width;
        }
        if let (Some(idx), Some(l)) = (self.file_tree_window(), self.window_layout.as_mut()) {
            l.set_fixed_size(idx, width);
        }
    }

    fn tree_width(&self) -> usize {
        self.file_tree_window()
            .and_then(|i| self.window_layout.as_ref()?.fixed_size(i))
            .or(self.file_tree.as_ref().map(|t| t.width))
            .unwrap_or(32)
    }

    /// `i`: size / modification time / permissions of the cursor's node.
    fn tree_info(&mut self) {
        let Some(node) = self.file_tree.as_ref().and_then(|t| t.selected().cloned()) else {
            return;
        };
        let rel = node
            .path
            .strip_prefix(&self.project_root)
            .unwrap_or(&node.path)
            .display()
            .to_string();
        let Ok(meta) = std::fs::symlink_metadata(&node.path) else {
            self.set_message(format!("{rel}: cannot stat"));
            return;
        };
        let mut parts = vec![rel];
        if node.is_dir {
            let n = std::fs::read_dir(&node.path)
                .map(|d| d.count())
                .unwrap_or(0);
            parts.push(format!("{n} entr{}", if n == 1 { "y" } else { "ies" }));
        } else {
            parts.push(human_size(meta.len()));
        }
        if let Ok(m) = meta.modified() {
            parts.push(format!("modified {}", ago(m)));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            parts.push(format!("{:o}", meta.permissions().mode() & 0o7777));
        }
        if let Some(l) = &node.link {
            parts.push(format!("-> {}", l.display()));
        }
        if let Some(c) = self
            .file_tree
            .as_ref()
            .and_then(|t| t.git_marker(&node.path, node.is_dir))
        {
            parts.push(format!("git {c}"));
        }
        self.set_message(parts.join("  ·  "));
    }

    fn tree_copy_path(&mut self, absolute: bool) {
        let Some(path) = self
            .file_tree
            .as_ref()
            .and_then(|t| t.selected())
            .map(|n| n.path.clone())
        else {
            return;
        };
        let text = if absolute {
            path.display().to_string()
        } else {
            path.strip_prefix(&self.project_root)
                .unwrap_or(&path)
                .display()
                .to_string()
        };
        if !self.config.clipboard_unnamedplus {
            crate::clipboard::copy(&text);
        }
        self.registers.yank(None, text.clone(), false);
        self.set_message(format!("Copied path: {text}"));
    }

    fn tree_system_open(&mut self) {
        let Some(path) = self
            .file_tree
            .as_ref()
            .and_then(|t| t.selected())
            .map(|n| n.path.clone())
        else {
            return;
        };
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let spawned = std::process::Command::new(opener)
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
                self.set_message(format!("Opened {} with {opener}", path.display()));
            }
            Err(e) => self.set_message(format!("{opener}: {e}")),
        }
    }

    /// `]c`/`[c` (git) and `]d`/`[d` (diagnostics): the next/previous
    /// visible node carrying that marker, wrapping around.
    fn tree_jump_marked(&mut self, forward: bool, git: bool) {
        let Some(t) = &self.file_tree else {
            return;
        };
        let n = t.nodes.len();
        if n == 0 {
            return;
        }
        let has = |i: usize| {
            let node = &t.nodes[i];
            if git {
                t.git_marker(&node.path, node.is_dir).is_some()
            } else {
                crate::render::tree_diagnostic_marker(self, &node.path, node.is_dir).is_some()
            }
        };
        let found = (1..=n)
            .map(|k| {
                if forward {
                    (t.cursor + k) % n
                } else {
                    (t.cursor + n - k % n) % n
                }
            })
            .find(|&i| has(i));
        match found {
            Some(i) => {
                if let Some(t) = &mut self.file_tree {
                    t.cursor = i;
                }
            }
            None => self.set_message(if git {
                "No git changes in the visible tree"
            } else {
                "No diagnostics in the visible tree"
            }),
        }
    }

    /// Mouse click on row `row` (pane-relative) of the tree pane `pane`.
    /// A click selects; clicking the selected row again (or a
    /// double-click) opens it; a click on a directory's arrow toggles it.
    pub fn tree_click(&mut self, pane: usize, row: usize, col: usize) {
        let was_focused = pane == self.active_window;
        if !was_focused {
            self.focus_window(pane);
        }
        let Some(t) = &mut self.file_tree else {
            return;
        };
        let Some(list_row) = row.checked_sub(HEADER_ROWS) else {
            return;
        };
        let idx = t.top + list_row;
        let Some(node) = t.nodes.get(idx).cloned() else {
            return;
        };
        let again = t
            .last_click
            .is_some_and(|(i, at)| i == idx && at.elapsed() < Duration::from_millis(600))
            || (was_focused && t.cursor == idx);
        t.cursor = idx;
        t.last_click = Some((idx, Instant::now()));
        let arrow = 1 + node.depth * 2;
        let on_arrow = node.is_dir && (arrow..arrow + 4).contains(&col);
        if on_arrow || again {
            t.last_click = None;
            handle_key(self, Key::Enter);
        }
    }
}

/// Case-only rename on a case-insensitive filesystem: the "existing"
/// destination is the source itself.
fn same_file_case_rename(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

fn remap_set(set: &mut BTreeSet<PathBuf>, from: &Path, to: &Path) {
    let moved: Vec<PathBuf> = set
        .iter()
        .filter(|p| p.starts_with(from))
        .cloned()
        .collect();
    for p in moved {
        set.remove(&p);
        if let Ok(rel) = p.strip_prefix(from) {
            set.insert(if rel.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(rel)
            });
        }
    }
}

fn shellexpand_home(s: &str) -> String {
    match (s.strip_prefix('~'), dirs::home_dir()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{}", home.display(), rest)
        }
        _ => s.to_string(),
    }
}

fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn ago(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

/// Recursively copies `src` to `dest` (a plain file or a whole directory
/// tree) -- `std::fs` has no built-in directory copy. On failure partway
/// through (e.g. permission denied on one nested file, or disk full),
/// removes whatever was already created at `dest` before returning the
/// error, so a failed copy never leaves a half-copied destination behind
/// for a later `y`/`p` to trip over as a spurious "already exists".
fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    let result = copy_recursive_step(src, dest);
    if result.is_err() {
        let _ = if dest.is_dir() {
            std::fs::remove_dir_all(dest)
        } else {
            std::fs::remove_file(dest)
        };
    }
    result
}

fn copy_recursive_step(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let target = dest.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_recursive_step(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    } else {
        std::fs::copy(src, dest).map(|_| ())
    }
}

/// A Nerd Font glyph and color for a tree entry (`tree_icons = true`).
pub fn icon(
    name: &str,
    is_dir: bool,
    open: bool,
    link: bool,
) -> (&'static str, crossterm::style::Color) {
    use crossterm::style::Color;
    let rgb = |r, g, b| Color::Rgb { r, g, b };
    if is_dir {
        let glyph = match (link, open) {
            (true, _) => "\u{f482}",
            (false, true) => "\u{f07c}",
            (false, false) => "\u{f07b}",
        };
        return (glyph, rgb(0x7a, 0xa2, 0xf7));
    }
    if link {
        return ("\u{f481}", rgb(0x7d, 0xcf, 0xff));
    }
    let lower = name.to_ascii_lowercase();
    let by_name = match lower.as_str() {
        "cargo.toml" | "cargo.lock" => Some(("\u{e7a8}", rgb(0xde, 0xa5, 0x84))),
        "package.json" | "package-lock.json" => Some(("\u{e71e}", rgb(0xe8, 0x27, 0x4b))),
        "makefile" | "gnumakefile" | "cmakelists.txt" => Some(("\u{e779}", rgb(0x6d, 0x80, 0x86))),
        "dockerfile" | "containerfile" | "docker-compose.yml" | "docker-compose.yaml" => {
            Some(("\u{f308}", rgb(0x45, 0x8e, 0xe6)))
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" => {
            Some(("\u{e702}", rgb(0xf5, 0x4d, 0x27)))
        }
        "license" | "license.md" | "license.txt" | "copying" => {
            Some(("\u{e60a}", rgb(0xd0, 0xbf, 0x41)))
        }
        "readme" | "readme.md" | "readme.txt" => Some(("\u{f48a}", rgb(0x51, 0x9a, 0xba))),
        "go.mod" | "go.sum" => Some(("\u{e627}", rgb(0x00, 0xad, 0xd8))),
        ".env" | ".envrc" => Some(("\u{f462}", rgb(0xfa, 0xf7, 0x43))),
        _ => None,
    };
    if let Some(i) = by_name {
        return i;
    }
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "rs" => ("\u{e7a8}", rgb(0xde, 0xa5, 0x84)),
        "py" | "pyi" => ("\u{e606}", rgb(0xff, 0xbc, 0x03)),
        "js" | "mjs" | "cjs" => ("\u{e74e}", rgb(0xcb, 0xcb, 0x41)),
        "ts" | "mts" | "cts" => ("\u{e628}", rgb(0x51, 0x9a, 0xba)),
        "jsx" | "tsx" => ("\u{e7ba}", rgb(0x20, 0xc2, 0xe3)),
        "go" => ("\u{e627}", rgb(0x00, 0xad, 0xd8)),
        "c" => ("\u{e61e}", rgb(0x59, 0x9e, 0xff)),
        "h" | "hpp" | "hh" => ("\u{f0fd}", rgb(0xa0, 0x74, 0xc4)),
        "cpp" | "cc" | "cxx" => ("\u{e61d}", rgb(0x51, 0x9a, 0xba)),
        "java" => ("\u{e738}", rgb(0xcc, 0x3e, 0x44)),
        "kt" | "kts" => ("\u{e634}", rgb(0x7f, 0x52, 0xff)),
        "scala" => ("\u{e737}", rgb(0xcc, 0x3e, 0x44)),
        "cs" => ("\u{e648}", rgb(0x59, 0x67, 0x06)),
        "lua" => ("\u{e620}", rgb(0x51, 0xa0, 0xcf)),
        "vim" => ("\u{e62b}", rgb(0x01, 0x98, 0x33)),
        "sh" | "bash" | "zsh" | "fish" => ("\u{e795}", rgb(0x89, 0xe0, 0x51)),
        "json" | "jsonc" | "json5" => ("\u{e60b}", rgb(0xcb, 0xcb, 0x41)),
        "toml" | "ini" | "cfg" | "conf" => ("\u{e615}", rgb(0x6d, 0x80, 0x86)),
        "yaml" | "yml" => ("\u{e615}", rgb(0x6d, 0x80, 0x86)),
        "md" | "markdown" | "mdx" => ("\u{e609}", rgb(0x51, 0x9a, 0xba)),
        "html" | "htm" => ("\u{e736}", rgb(0xe4, 0x4d, 0x26)),
        "css" => ("\u{e749}", rgb(0x42, 0xa5, 0xf5)),
        "scss" | "sass" | "less" => ("\u{e603}", rgb(0xf5, 0x53, 0x85)),
        "vue" => ("\u{e6a0}", rgb(0x8d, 0xc1, 0x49)),
        "svelte" => ("\u{e697}", rgb(0xff, 0x3e, 0x00)),
        "sql" => ("\u{e706}", rgb(0xda, 0xd8, 0xd8)),
        "rb" => ("\u{e739}", rgb(0x70, 0x15, 0x16)),
        "php" => ("\u{e608}", rgb(0xa0, 0x74, 0xc4)),
        "swift" => ("\u{e755}", rgb(0xe3, 0x79, 0x33)),
        "dart" => ("\u{e798}", rgb(0x03, 0x58, 0x9c)),
        "ex" | "exs" => ("\u{e62d}", rgb(0xa0, 0x74, 0xc4)),
        "hs" => ("\u{e61f}", rgb(0xa0, 0x74, 0xc4)),
        "zig" => ("\u{e6a9}", rgb(0xf6, 0x9a, 0x1b)),
        "nix" => ("\u{f313}", rgb(0x7e, 0xba, 0xe4)),
        "r" => ("\u{e68a}", rgb(0x22, 0x66, 0xba)),
        "jl" => ("\u{e624}", rgb(0xa2, 0x70, 0xba)),
        "lock" => ("\u{f023}", rgb(0xbb, 0xbb, 0xbb)),
        "txt" | "log" => ("\u{f0f6}", rgb(0x89, 0xe0, 0x51)),
        "csv" | "tsv" => ("\u{f1c3}", rgb(0x89, 0xe0, 0x51)),
        "xml" | "sol" | "proto" | "graphql" | "gql" => ("\u{f121}", rgb(0xe3, 0x79, 0x33)),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "svg" => {
            ("\u{f1c5}", rgb(0xa0, 0x74, 0xc4))
        }
        "pdf" => ("\u{f1c1}", rgb(0xb3, 0x0b, 0x00)),
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "zst" | "rar" => {
            ("\u{f1c6}", rgb(0xec, 0xa5, 0x17))
        }
        _ => ("\u{f15b}", rgb(0x8a, 0x93, 0xa6)),
    }
}

/// The `?` reference, shown in the tree pane itself (`j`/`k` scroll it).
pub const HELP: &[(&str, &str)] = &[
    ("j k", "down / up (5j)"),
    ("gg G", "first / last"),
    ("C-d C-u", "half page"),
    ("C-f C-b", "full page"),
    ("C-e C-y", "scroll a line"),
    ("zz zt zb", "center/top/bottom"),
    ("P", "parent dir"),
    ("K J", "first/last sibling"),
    ("]c [c", "next/prev git change"),
    ("]d [d", "next/prev diagnostic"),
    ("", ""),
    ("l →", "expand / open"),
    ("h ←", "collapse / parent"),
    ("Enter o", "toggle / open"),
    ("Tab", "preview, stay here"),
    ("s S", "open vsplit / split"),
    ("C-v C-x", "open vsplit / split"),
    ("T C-t", "open in new tab"),
    ("gx", "open with system app"),
    ("E", "expand all below"),
    ("W zM", "collapse all"),
    ("F", "reveal current file"),
    ("", ""),
    ("/ f", "fuzzy find (project)"),
    ("Esc", "clear / back to editor"),
    ("- BS", "root: parent dir"),
    ("C", "root: this dir"),
    ("~", "root: project"),
    ("", ""),
    ("a", "new (dir/, a/b/c.rs)"),
    ("r", "rename"),
    ("d d", "delete"),
    ("t t", "trash (.vaayu/trash)"),
    ("y x p", "copy / cut / paste"),
    ("Space", "mark (multi-select)"),
    ("u", "clear marks"),
    ("Y gy", "copy rel / abs path"),
    ("i", "file info"),
    ("m B", "bookmark / list"),
    ("", ""),
    (". H", "toggle dotfiles"),
    ("! I", "toggle gitignored"),
    ("R", "refresh"),
    ("< >", "narrower / wider"),
    ("q", "close"),
    ("? g?", "this help"),
];

fn with_tree(ed: &mut Editor, f: impl FnOnce(&mut FileTree)) {
    if let Some(t) = &mut ed.file_tree {
        f(t);
    }
}

fn filter_key(ed: &mut Editor, key: Key) {
    match key {
        Key::Esc => {
            with_tree(ed, |t| {
                let anchor = t.selected().map(|n| n.path.clone());
                t.clear_filter();
                t.rebuild();
                if let Some(a) = anchor {
                    t.reveal(&a);
                }
            });
            ed.set_message("");
        }
        Key::Enter => with_tree(ed, |t| {
            t.filter_input = false;
            if t.filter.is_empty() {
                t.clear_filter();
                t.rebuild();
            }
        }),
        Key::Backspace => {
            with_tree(ed, |t| {
                t.filter.pop();
            });
            ed.refilter_tree();
            ed.report_tree_filter();
        }
        Key::Ctrl('u') => {
            with_tree(ed, |t| t.filter.clear());
            ed.refilter_tree();
        }
        Key::Down | Key::Ctrl('n') | Key::Ctrl('j') => with_tree(ed, |t| t.move_cursor(1)),
        Key::Up | Key::Ctrl('p') | Key::Ctrl('k') => with_tree(ed, |t| t.move_cursor(-1)),
        Key::Char(c) | Key::Literal(c) => {
            with_tree(ed, |t| t.filter.push(c));
            ed.refilter_tree();
            ed.report_tree_filter();
        }
        _ => {}
    }
}

fn arm_or_run(ed: &mut Editor, trash: bool) {
    let Some(t) = &ed.file_tree else { return };
    let targets = t.targets();
    if targets.is_empty() {
        return;
    }
    let armed = if trash {
        &t.confirm_trash
    } else {
        &t.confirm_delete
    };
    if armed.as_ref() == Some(&targets) {
        with_tree(ed, |t| {
            t.confirm_delete = None;
            t.confirm_trash = None;
        });
        if trash {
            ed.tree_trash_confirmed(&targets);
        } else {
            ed.tree_delete_confirmed(&targets);
        }
        return;
    }
    let what = match targets.as_slice() {
        [one] => one.display().to_string(),
        many => format!("{} marked items", many.len()),
    };
    let (key, verb) = if trash {
        ('t', "trash")
    } else {
        ('d', "delete")
    };
    ed.set_message(format!(
        "Press {key} again to {verb} {what} (any other key cancels)"
    ));
    with_tree(ed, |t| {
        if trash {
            t.confirm_trash = Some(targets);
        } else {
            t.confirm_delete = Some(targets);
        }
    });
}

fn open_selected(ed: &mut Editor, how: OpenIn, keep_focus: bool) {
    let Some(node) = ed.file_tree.as_ref().and_then(|t| t.selected().cloned()) else {
        return;
    };
    if node.is_dir {
        if how == OpenIn::Edit {
            with_tree(ed, |t| {
                t.activate();
            });
        } else {
            ed.set_message("Not a file");
        }
        return;
    }
    ed.tree_open(node.path, how, keep_focus);
}

/// Second key of a `g`/`z`/`[`/`]` sequence.
fn prefixed(ed: &mut Editor, prefix: char, key: Key, count: Option<usize>) {
    let half = ed.file_tree.as_ref().map_or(10, |t| t.height.max(2) / 2);
    match (prefix, key) {
        ('g', Key::Char('g')) => with_tree(ed, |t| {
            t.cursor = count
                .map_or(0, |n| n.saturating_sub(1))
                .min(t.nodes.len().saturating_sub(1));
        }),
        ('g', Key::Char('y')) => ed.tree_copy_path(true),
        ('g', Key::Char('x')) => ed.tree_system_open(),
        ('g', Key::Char('?')) => with_tree(ed, |t| {
            t.show_help = true;
            t.help_scroll = 0;
        }),
        ('z', Key::Char('z')) => with_tree(ed, |t| t.top = t.cursor.saturating_sub(half)),
        ('z', Key::Char('t')) => with_tree(ed, |t| t.top = t.cursor),
        ('z', Key::Char('b')) => with_tree(ed, |t| {
            t.top = (t.cursor + 1).saturating_sub(t.height.max(1))
        }),
        ('z', Key::Char('M')) | ('z', Key::Char('m')) => with_tree(ed, FileTree::collapse_all),
        ('z', Key::Char('c')) => with_tree(ed, |t| {
            if t.selected().is_some_and(|n| !(n.is_dir && n.open)) {
                t.goto_parent();
            }
            t.collapse_or_parent();
        }),
        ('z', Key::Char('o')) => with_tree(ed, |t| {
            if t.selected().is_some_and(|n| n.is_dir && !n.open) {
                t.activate();
            }
        }),
        (']', Key::Char('c')) => ed.tree_jump_marked(true, true),
        ('[', Key::Char('c')) => ed.tree_jump_marked(false, true),
        (']', Key::Char('d' | 'e')) => ed.tree_jump_marked(true, false),
        ('[', Key::Char('d' | 'e')) => ed.tree_jump_marked(false, false),
        _ => {}
    }
}

pub fn handle_key(ed: &mut Editor, key: Key) {
    let Some(t) = &mut ed.file_tree else {
        return;
    };
    if t.show_help {
        // j/k (and paging) scroll the reference; any other key closes it.
        let max = HELP.len().saturating_sub(t.height.max(1));
        let page = t.height.max(2) / 2;
        match key {
            Key::Char('j') | Key::Down => t.help_scroll = (t.help_scroll + 1).min(max),
            Key::Char('k') | Key::Up => t.help_scroll = t.help_scroll.saturating_sub(1),
            Key::Ctrl('d') | Key::PageDown => t.help_scroll = (t.help_scroll + page).min(max),
            Key::Ctrl('u') | Key::PageUp => t.help_scroll = t.help_scroll.saturating_sub(page),
            _ => t.show_help = false,
        }
        return;
    }
    if t.filter_input {
        filter_key(ed, key);
        return;
    }
    if let Some(p) = t.pending.take() {
        let count = t.count.take();
        prefixed(ed, p, key, count);
        return;
    }
    if let Key::Char(c) = key {
        if c.is_ascii_digit() && (c != '0' || t.count.is_some()) {
            let d = c.to_digit(10).unwrap_or(0) as usize;
            t.count = Some(
                t.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(d)
                    .min(99_999),
            );
            return;
        }
        if matches!(c, 'g' | 'z' | '[' | ']') {
            t.pending = Some(c);
            return;
        }
    }
    let count = t.count.take();
    let n = count.unwrap_or(1) as isize;
    if key != Key::Char('d') {
        t.confirm_delete = None;
    }
    if key != Key::Char('t') {
        t.confirm_trash = None;
    }
    let page = t.height.max(2) as isize;
    match key {
        Key::Char('j') | Key::Down => with_tree(ed, |t| t.move_cursor(n)),
        Key::Char('k') | Key::Up => with_tree(ed, |t| t.move_cursor(-n)),
        Key::Ctrl('d') => with_tree(ed, |t| t.move_cursor(page / 2)),
        Key::Ctrl('u') => with_tree(ed, |t| t.move_cursor(-page / 2)),
        Key::Ctrl('f') | Key::PageDown => with_tree(ed, |t| t.move_cursor(page)),
        Key::Ctrl('b') | Key::PageUp => with_tree(ed, |t| t.move_cursor(-page)),
        Key::Ctrl('e') => with_tree(ed, |t| t.scroll(n, page as usize)),
        Key::Ctrl('y') => with_tree(ed, |t| t.scroll(-n, page as usize)),
        Key::Home => with_tree(ed, |t| t.cursor = 0),
        Key::End | Key::Char('G') => with_tree(ed, |t| {
            let last = t.nodes.len().saturating_sub(1);
            t.cursor = count.map_or(last, |c| c.saturating_sub(1).min(last));
        }),
        Key::Char('P') => with_tree(ed, FileTree::goto_parent),
        Key::Char('K') => with_tree(ed, |t| t.goto_sibling_edge(false)),
        Key::Char('J') => with_tree(ed, |t| t.goto_sibling_edge(true)),
        Key::Char('h') | Key::Left => with_tree(ed, FileTree::collapse_or_parent),
        Key::Char('l') | Key::Right => {
            let opened = ed.file_tree.as_mut().and_then(FileTree::expand_or_enter);
            if let Some(path) = opened {
                ed.open_from_tree(path);
            }
        }
        Key::Enter | Key::Char('o') => {
            let opened = ed.file_tree.as_mut().and_then(FileTree::activate);
            if let Some(path) = opened {
                ed.open_from_tree(path);
            }
        }
        Key::Tab => open_selected(ed, OpenIn::Edit, true),
        Key::Char('s') | Key::Ctrl('v') => open_selected(ed, OpenIn::VSplit, false),
        Key::Char('S') | Key::Ctrl('x') => open_selected(ed, OpenIn::HSplit, false),
        Key::Char('T') | Key::Ctrl('t') => open_selected(ed, OpenIn::Tab, false),
        Key::Char('E') => {
            let Some(t) = &mut ed.file_tree else { return };
            let dir = t.target_dir();
            let (dirs, cut) = t.expand_all(&dir);
            t.rebuild();
            ed.set_message(if cut {
                format!("Expanded {dirs} directories (stopped at {EXPAND_ALL_LIMIT} entries)")
            } else {
                format!("Expanded {dirs} directories")
            });
        }
        Key::Char('W') => with_tree(ed, FileTree::collapse_all),
        Key::Char('F') => ed.tree_find_current(),
        Key::Char('-') | Key::Backspace => {
            let Some(t) = &mut ed.file_tree else { return };
            let old = t.root.clone();
            if let Some(parent) = old.parent().map(Path::to_path_buf) {
                t.expanded.insert(old.clone());
                t.set_root(parent.clone());
                if let Some(i) = t.nodes.iter().position(|n| n.path == old) {
                    t.cursor = i;
                }
                ed.set_message(format!("Tree root: {}", parent.display()));
            }
        }
        Key::Char('C') => {
            let Some(t) = &mut ed.file_tree else { return };
            let dir = t.target_dir();
            t.set_root(dir.clone());
            ed.set_message(format!("Tree root: {}", dir.display()));
        }
        Key::Char('~') => ed.tree_set_root(""),
        Key::Char('R') => {
            with_tree(ed, |t| {
                t.cache.clear();
                t.rebuild();
            });
            ed.invalidate_project_files();
            ed.refresh_tree_git_status();
            ed.set_message("Tree refreshed");
        }
        Key::Char('.') | Key::Char('H') => {
            let shown = ed.file_tree.as_mut().map(|t| {
                t.show_hidden = !t.show_hidden;
                t.rebuild();
                t.show_hidden
            });
            if ed.file_tree.as_ref().is_some_and(|t| !t.filter.is_empty()) {
                ed.refilter_tree();
            }
            if let Some(s) = shown {
                ed.set_message(if s {
                    "Showing dotfiles"
                } else {
                    "Hiding dotfiles"
                });
            }
        }
        Key::Char('!') | Key::Char('I') => {
            let shown = ed.file_tree.as_mut().map(|t| {
                t.show_ignored = !t.show_ignored;
                t.rebuild();
                t.show_ignored
            });
            if let Some(s) = shown {
                ed.set_message(if s {
                    "Showing gitignored files"
                } else {
                    "Hiding gitignored files"
                });
            }
        }
        Key::Char('a') => {
            ed.enter_command(crate::mode::CommandKind::Ex);
            ed.set_cmdline("treenew ");
        }
        Key::Char('r') => {
            let name = ed
                .file_tree
                .as_ref()
                .and_then(|t| t.selected())
                .map(|n| n.name.clone())
                .unwrap_or_default();
            ed.enter_command(crate::mode::CommandKind::Ex);
            ed.set_cmdline(format!("treerename {name}"));
        }
        Key::Char('d') => arm_or_run(ed, false),
        Key::Char('t') => arm_or_run(ed, true),
        Key::Char('y') | Key::Char('c') => ed.tree_yank(),
        Key::Char('x') => ed.tree_cut(),
        Key::Char('p') => ed.tree_paste(),
        Key::Char(' ') => {
            let Some(t) = &mut ed.file_tree else { return };
            for _ in 0..n {
                if let Some(p) = t.selected().map(|n| n.path.clone()) {
                    if !t.marked.remove(&p) {
                        t.marked.insert(p);
                    }
                }
                t.move_cursor(1);
            }
            let m = t.marked.len();
            ed.set_message(format!("{m} marked (d/t/y/x act on all, u clears)"));
        }
        Key::Char('u') => {
            with_tree(ed, |t| t.marked.clear());
            ed.set_message("Marks cleared");
        }
        Key::Char('Y') => ed.tree_copy_path(false),
        Key::Char('i') => ed.tree_info(),
        Key::Char('m') => ed.tree_toggle_bookmark(),
        Key::Char('B') => ed.show_tree_bookmarks(),
        Key::Char('/') | Key::Char('f') => {
            with_tree(ed, |t| t.filter_input = true);
            ed.start_file_scan();
            ed.report_tree_filter();
        }
        Key::Char('<') => {
            let w = ed.tree_width();
            ed.tree_set_width(w.saturating_sub(4 * n as usize));
        }
        Key::Char('>') => {
            let w = ed.tree_width();
            ed.tree_set_width(w + 4 * n as usize);
        }
        Key::Char('?') => with_tree(ed, |t| {
            t.show_help = true;
            t.help_scroll = 0;
        }),
        Key::Char(':') => ed.enter_command(crate::mode::CommandKind::Ex),
        Key::Char('q') => ed.toggle_file_tree(),
        Key::Esc => {
            let Some(t) = &mut ed.file_tree else { return };
            if !t.filter.is_empty() {
                filter_key(ed, Key::Esc);
            } else if !t.marked.is_empty() {
                t.marked.clear();
                ed.set_message("Marks cleared");
            } else if let Some(w) = ed.existing_edit_window() {
                ed.focus_window(w);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, editor::Editor};

    fn project(files: &[&str], dirs: &[&str]) -> PathBuf {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "vaayu-filetree-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for d in dirs {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        for f in files {
            std::fs::write(root.join(f), "x").unwrap();
        }
        root
    }

    fn names(t: &FileTree) -> Vec<&str> {
        t.nodes.iter().map(|n| n.name.as_str()).collect()
    }

    #[test]
    fn lists_root_and_skips_dot_git() {
        let root = project(&["a.txt", "b.txt"], &["src", ".git"]);
        let t = FileTree::new(root.clone());
        let names = names(&t);
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"src"));
        assert!(!names.contains(&".git"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn dotfiles_are_hidden_until_toggled() {
        let root = project(&["a.txt", ".hidden"], &[".hiddendir", "src", ".git"]);
        let mut t = FileTree::new(root.clone());
        let n = names(&t);
        assert!(n.contains(&"a.txt"));
        assert!(n.contains(&"src"));
        assert!(!n.contains(&".hidden"));
        assert!(!n.contains(&".hiddendir"));
        t.show_hidden = true;
        t.rebuild();
        let n = names(&t);
        assert!(n.contains(&".hidden"));
        assert!(n.contains(&".hiddendir"));
        assert!(
            !n.contains(&".git"),
            ".git stays hidden even with show_hidden"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn directories_sort_before_files_then_alphabetically() {
        let root = project(&["z.txt", "a.txt"], &["dirb", "dira"]);
        let t = FileTree::new(root.clone());
        assert_eq!(names(&t), vec!["dira", "dirb", "a.txt", "z.txt"]);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn natural_sort_orders_numbers_by_value_and_ignores_case() {
        let root = project(
            &["file10.txt", "file2.txt", "File1.txt", "b.txt", "A.txt"],
            &[],
        );
        let t = FileTree::new(root.clone());
        assert_eq!(
            names(&t),
            vec!["A.txt", "b.txt", "File1.txt", "file2.txt", "file10.txt"]
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn expand_collapse_toggles_children_lazily() {
        let root = project(&["outer.txt"], &["sub"]);
        std::fs::write(root.join("sub/inner.txt"), "x").unwrap();
        let mut t = FileTree::new(root.clone());
        assert_eq!(t.nodes.len(), 2); // "sub" dir + "outer.txt", not yet expanded
        assert!(
            !t.cache.contains_key(&root.join("sub")),
            "unexpanded dirs are never read"
        );
        t.cursor = t.nodes.iter().position(|n| n.name == "sub").unwrap();
        let opened = t.activate();
        assert!(opened.is_none(), "activating a directory must not open it");
        assert_eq!(t.nodes.len(), 3, "expanding should reveal inner.txt");
        assert!(t
            .nodes
            .iter()
            .any(|n| n.name == "inner.txt" && n.depth == 1));
        t.activate(); // collapse again
        assert_eq!(t.nodes.len(), 2);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn indent_guides_track_last_children() {
        let root = project(&["z.txt"], &["a", "b"]);
        std::fs::write(root.join("a/one.txt"), "x").unwrap();
        std::fs::write(root.join("a/two.txt"), "x").unwrap();
        let mut t = FileTree::new(root.clone());
        t.reveal(&root.join("a/two.txt"));
        let a = t.nodes.iter().find(|n| n.name == "a").unwrap();
        assert!(!a.last && a.open);
        let one = t.nodes.iter().find(|n| n.name == "one.txt").unwrap();
        let two = t.nodes.iter().find(|n| n.name == "two.txt").unwrap();
        assert!(!one.last && two.last);
        assert_eq!(
            two.rails & 1,
            1,
            "`a` has siblings below, so its subtree carries a rail"
        );
        let z = t.nodes.iter().find(|n| n.name == "z.txt").unwrap();
        assert!(z.last);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cached_listing_picks_up_external_changes_on_refresh() {
        let root = project(&["a.txt"], &[]);
        let mut t = FileTree::new(root.clone());
        assert_eq!(t.nodes.len(), 1);
        // Make sure the directory mtime visibly moves.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(root.join("b.txt"), "x").unwrap();
        t.rebuild();
        assert_eq!(
            t.nodes.len(),
            1,
            "a plain rebuild reuses the cached listing"
        );
        assert!(t.refresh_stale(), "the root's mtime changed");
        assert_eq!(names(&t), vec!["a.txt", "b.txt"]);
        assert!(!t.refresh_stale(), "nothing changed since");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn reveal_expands_ancestors_and_selects_the_file() {
        let root = project(&[], &["a/b"]);
        let target = root.join("a/b/c.txt");
        std::fs::write(&target, "x").unwrap();
        let mut t = FileTree::new(root.clone());
        assert_eq!(t.nodes.len(), 1, "nothing expanded yet");
        t.reveal(&target);
        assert_eq!(t.nodes[t.cursor].path, target);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn sibling_and_parent_navigation() {
        let root = project(&["z.txt"], &["a", "m"]);
        for f in ["a/1.txt", "a/2.txt", "a/3.txt"] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        let mut t = FileTree::new(root.clone());
        t.reveal(&root.join("a/2.txt"));
        t.goto_sibling_edge(true);
        assert_eq!(t.nodes[t.cursor].name, "3.txt");
        t.goto_sibling_edge(false);
        assert_eq!(t.nodes[t.cursor].name, "1.txt");
        t.goto_parent();
        assert_eq!(t.nodes[t.cursor].name, "a");
        t.goto_sibling_edge(true);
        assert_eq!(t.nodes[t.cursor].name, "z.txt");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn collapse_all_and_expand_all() {
        let root = project(&[], &["a/b/c", "d"]);
        std::fs::write(root.join("a/b/c/deep.txt"), "x").unwrap();
        let mut t = FileTree::new(root.clone());
        let (dirs, cut) = t.expand_all(&root);
        assert!(!cut);
        assert!(dirs >= 4);
        t.rebuild();
        assert!(names(&t).contains(&"deep.txt"));
        t.reveal(&root.join("a/b/c/deep.txt"));
        t.collapse_all();
        assert_eq!(names(&t), vec!["a", "d"]);
        assert_eq!(
            t.nodes[t.cursor].name, "a",
            "cursor lands on the top-level ancestor"
        );
        std::fs::remove_dir_all(root).ok();
    }

    fn editor_for(root: &Path) -> Editor {
        let cfg = Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..Config::default()
        };
        let mut e = Editor::new(cfg);
        e.project_root = root.to_path_buf();
        e
    }

    #[test]
    fn toggle_opens_and_closes_the_sidebar_pane() {
        let root = project(&["f.txt"], &[]);
        let mut e = editor_for(&root);
        assert!(!e.active_file_tree());
        e.toggle_file_tree();
        assert!(e.file_tree.is_some());
        assert_eq!(e.windows.len(), 2);
        assert!(e.active_file_tree());
        e.toggle_file_tree();
        assert!(
            e.windows.is_empty() || !e.windows.iter().any(|w| w.file_tree),
            "toggling again must close the sidebar pane"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn sidebar_is_pinned_left_at_a_fixed_width() {
        let root = project(&["f.txt"], &[]);
        let mut e = editor_for(&root);
        e.toggle_file_tree();
        let tree = e.windows.iter().position(|w| w.file_tree).unwrap();
        for cols in [100, 200] {
            let rects = e.pane_rects(cols, 40);
            assert_eq!(rects[tree].x, 0, "tree sits at the left edge");
            assert_eq!(
                rects[tree].width, 32,
                "width doesn't scale with the terminal"
            );
        }
        handle_key(&mut e, Key::Char('>'));
        assert_eq!(e.pane_rects(200, 40)[tree].width, 36);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn leader_e_focuses_then_closes() {
        let root = project(&["f.txt"], &[]);
        let mut e = editor_for(&root);
        e.open_file(root.join("f.txt")).unwrap();
        e.focus_or_toggle_file_tree();
        assert!(e.active_file_tree(), "opens focused");
        handle_key(&mut e, Key::Esc);
        assert!(!e.active_file_tree(), "Esc hands focus back to the editor");
        assert!(
            e.windows.iter().any(|w| w.file_tree),
            "...without closing it"
        );
        e.focus_or_toggle_file_tree();
        assert!(
            e.active_file_tree(),
            "an open-but-unfocused tree gets focus"
        );
        e.focus_or_toggle_file_tree();
        assert!(
            !e.windows.iter().any(|w| w.file_tree),
            "a focused tree closes"
        );
        assert_eq!(e.buf().path, Some(root.join("f.txt")));
        std::fs::remove_dir_all(root).ok();
    }

    /// Opens the tree through the real `,ft` path (a split, not just the
    /// `file_tree` field alone), since some operations (e.g. opening a
    /// file from `:treebookmarks`) need an actual other pane to focus.
    fn editor_with_tree(root: &Path) -> Editor {
        let mut e = editor_for(root);
        e.toggle_file_tree();
        e
    }

    #[test]
    fn open_variants_and_preview_keep_focus() {
        let root = project(&["a.txt", "b.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Tab); // preview a.txt
        assert!(e.active_file_tree(), "Tab keeps focus in the tree");
        let edit = e.file_tree.as_ref().unwrap().last_edit_window;
        let shown = e.windows[edit].buffer;
        assert_eq!(
            e.buffers.iter().find(|b| b.id == shown).unwrap().path,
            Some(root.join("a.txt"))
        );
        handle_key(&mut e, Key::Char('j'));
        let before = e.windows.len();
        handle_key(&mut e, Key::Char('s'));
        assert_eq!(e.windows.len(), before + 1, "s opens a vertical split");
        assert_eq!(e.buf().path, Some(root.join("b.txt")));
        assert!(!e.active_file_tree());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn opening_a_file_when_the_tree_is_the_only_pane_creates_one() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        // Close the editing pane, leaving just the tree.
        let edit = e.windows.iter().position(|w| !w.file_tree).unwrap();
        e.focus_window(edit);
        e.close_window();
        assert_eq!(e.windows.len(), 1);
        let tree = e.windows.iter().position(|w| w.file_tree).unwrap();
        e.focus_window(tree);
        handle_key(&mut e, Key::Enter);
        assert_eq!(e.windows.len(), 2);
        assert_eq!(e.buf().path, Some(root.join("a.txt")));
        assert_eq!(e.pane_rects(120, 40)[tree].width, 32);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn q_closes_a_tree_that_is_the_only_pane_left() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        let edit = e.windows.iter().position(|w| !w.file_tree).unwrap();
        e.focus_window(edit);
        e.close_window();
        assert!(e.active_file_tree());
        handle_key(&mut e, Key::Esc); // nothing to go back to: no new pane
        assert_eq!(e.windows.len(), 1);
        handle_key(&mut e, Key::Char('q'));
        assert!(e.windows.is_empty(), "back to the plain single-pane view");
        assert!(!e.active_file_tree());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_new_creates_a_file_at_root_and_a_dir_with_trailing_slash() {
        let root = project(&[], &[]);
        let mut e = editor_with_tree(&root);
        e.tree_new("hello.txt");
        assert!(root.join("hello.txt").is_file());
        e.file_tree.as_mut().unwrap().cursor = 0;
        e.tree_new("sub/");
        assert!(root.join("sub").is_dir());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_new_creates_nested_paths_but_refuses_escapes() {
        let root = project(&["anchor.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.tree_new("deep/er/file.rs");
        assert!(root.join("deep/er/file.rs").is_file());
        let t = e.file_tree.as_ref().unwrap();
        assert_eq!(
            t.nodes[t.cursor].path,
            root.join("deep/er/file.rs"),
            "new file is revealed"
        );
        e.tree_new("../escape.txt");
        assert!(!root.join("../escape.txt").exists());
        e.tree_new("/tmp/abs.txt");
        assert!(!Path::new("/tmp/abs.txt").exists() || root.starts_with("/tmp/abs.txt"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_new_refuses_to_overwrite_an_existing_path() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        std::fs::write(root.join("a.txt"), "original").unwrap();
        e.tree_new("a.txt");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "original"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_rename_moves_the_file_and_updates_open_buffer_path() {
        let root = project(&["old.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("old.txt")).unwrap();
        e.tree_rename("new.txt");
        assert!(!root.join("old.txt").exists());
        assert!(root.join("new.txt").exists());
        assert_eq!(e.buf().path, Some(root.join("new.txt")));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn r_prefills_the_current_name() {
        let root = project(&["old.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('r'));
        assert_eq!(e.cmdline, "treerename old.txt");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_rename_refuses_when_buffer_has_unsaved_changes() {
        let root = project(&["old.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("old.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        assert!(e.buf().is_modified());
        e.tree_rename("new.txt");
        assert!(
            root.join("old.txt").exists(),
            "must not rename a file with unsaved changes"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn delete_requires_two_presses_and_then_removes_the_file() {
        let root = project(&["victim.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('d'));
        assert!(root.join("victim.txt").exists(), "first d only arms delete");
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_some());
        handle_key(&mut e, Key::Char('d'));
        assert!(!root.join("victim.txt").exists(), "second d deletes it");
        assert!(
            e.file_tree.as_ref().unwrap().nodes.is_empty(),
            "and it leaves the tree"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn any_other_key_cancels_an_armed_delete() {
        let root = project(&["safe.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('d'));
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_some());
        handle_key(&mut e, Key::Char('j'));
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_none());
        handle_key(&mut e, Key::Char('d'));
        assert!(
            root.join("safe.txt").exists(),
            "arming again must not delete immediately"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn marked_nodes_are_deleted_together() {
        let root = project(&["a.txt", "b.txt", "c.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char(' ')); // mark a, move to b
        handle_key(&mut e, Key::Char('j')); // skip b
        handle_key(&mut e, Key::Char(' ')); // mark c
        assert_eq!(e.file_tree.as_ref().unwrap().marked.len(), 2);
        handle_key(&mut e, Key::Char('d'));
        handle_key(&mut e, Key::Char('d'));
        assert!(!root.join("a.txt").exists());
        assert!(root.join("b.txt").exists(), "unmarked node survives");
        assert!(!root.join("c.txt").exists());
        assert!(e.file_tree.as_ref().unwrap().marked.is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn delete_refuses_when_an_open_buffer_has_unsaved_changes() {
        let root = project(&["important.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("important.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        let tree = e.windows.iter().position(|w| w.file_tree).unwrap();
        e.focus_window(tree);
        handle_key(&mut e, Key::Char('d'));
        handle_key(&mut e, Key::Char('d'));
        assert!(
            root.join("important.txt").exists(),
            "must not delete a file with unsaved changes"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn trash_requires_two_presses_and_moves_the_file_into_vaayu_trash() {
        let root = project(&["victim.txt"], &[]);
        std::fs::write(root.join("victim.txt"), "keepsake content").unwrap();
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('t'));
        assert!(root.join("victim.txt").exists(), "first t only arms trash");
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_some());
        handle_key(&mut e, Key::Char('t'));
        assert!(
            !root.join("victim.txt").exists(),
            "second t moves it out of place"
        );
        let trash_dir = root.join(".vaayu/trash");
        let entries: Vec<_> = std::fs::read_dir(&trash_dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "exactly one file should land in trash");
        let trashed = entries.into_iter().next().unwrap().unwrap().path();
        assert!(trashed
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with("-victim.txt"));
        assert_eq!(
            std::fs::read_to_string(&trashed).unwrap(),
            "keepsake content",
            "trash must preserve file content, not just the name"
        );
        assert!(
            !e.file_tree
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|n| n.name == "victim.txt"),
            "trashed file must disappear from the tree"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn any_other_key_cancels_an_armed_trash() {
        let root = project(&["safe.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('t'));
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_some());
        handle_key(&mut e, Key::Char('j'));
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_none());
        handle_key(&mut e, Key::Char('t'));
        assert!(
            root.join("safe.txt").exists(),
            "arming again must not trash immediately"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn trash_refuses_when_an_open_buffer_has_unsaved_changes() {
        let root = project(&["important.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("important.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        let tree = e.windows.iter().position(|w| w.file_tree).unwrap();
        e.focus_window(tree);
        handle_key(&mut e, Key::Char('t'));
        handle_key(&mut e, Key::Char('t'));
        assert!(
            root.join("important.txt").exists(),
            "must not trash a file with unsaved changes"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn arming_delete_does_not_also_arm_trash_and_vice_versa() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('d'));
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_some());
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_none());
        handle_key(&mut e, Key::Char('t'));
        // 't' is "any other key" to the pending delete, canceling it, then
        // arms trash instead -- not both armed at once.
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_none());
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_some());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn yank_then_paste_copies_the_file_and_keeps_the_original() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let mut e = editor_with_tree(&root);
        // nodes: [0]=dest (dir, sorts first), [1]=a.txt
        handle_key(&mut e, Key::Char('j')); // onto a.txt
        handle_key(&mut e, Key::Char('y'));
        assert!(e.file_tree.as_ref().unwrap().clipboard.is_some());
        handle_key(&mut e, Key::Char('k')); // back onto dest/
        handle_key(&mut e, Key::Char('p'));
        assert!(root.join("a.txt").exists(), "copy must keep the original");
        assert_eq!(
            std::fs::read_to_string(root.join("dest/a.txt")).unwrap(),
            "hello"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cut_then_paste_moves_the_file_and_updates_the_open_buffer_path() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("a.txt")).unwrap();
        let tree = e.windows.iter().position(|w| w.file_tree).unwrap();
        e.focus_window(tree);
        e.file_tree.as_mut().unwrap().cursor = 0;
        handle_key(&mut e, Key::Char('j'));
        handle_key(&mut e, Key::Char('x'));
        assert_eq!(
            e.file_tree.as_ref().unwrap().clipboard,
            Some((vec![root.join("a.txt")], true))
        );
        handle_key(&mut e, Key::Char('k'));
        handle_key(&mut e, Key::Char('p'));
        assert!(!root.join("a.txt").exists(), "cut+paste must move it");
        assert!(root.join("dest/a.txt").exists());
        assert!(
            e.buffers
                .iter()
                .any(|b| b.path == Some(root.join("dest/a.txt"))),
            "the open buffer follows the move"
        );
        assert!(
            e.file_tree.as_ref().unwrap().clipboard.is_none(),
            "clipboard should clear once the move succeeds"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn paste_refuses_a_name_collision() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "source").unwrap();
        std::fs::write(root.join("dest/a.txt"), "already here").unwrap();
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('j'));
        handle_key(&mut e, Key::Char('y'));
        handle_key(&mut e, Key::Char('k'));
        handle_key(&mut e, Key::Char('p'));
        assert_eq!(
            std::fs::read_to_string(root.join("dest/a.txt")).unwrap(),
            "already here",
            "collision must refuse, not overwrite"
        );
        assert!(root.join("a.txt").exists(), "source must be untouched");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn paste_with_nothing_copied_or_cut_is_a_harmless_no_op() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('p'));
        assert!(root.join("a.txt").exists());
    }

    #[test]
    fn cut_paste_refuses_when_the_source_has_an_unsaved_buffer() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("a.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        let tree = e.windows.iter().position(|w| w.file_tree).unwrap();
        e.focus_window(tree);
        e.file_tree.as_mut().unwrap().cursor = 0;
        handle_key(&mut e, Key::Char('j'));
        handle_key(&mut e, Key::Char('x'));
        handle_key(&mut e, Key::Char('k'));
        handle_key(&mut e, Key::Char('p'));
        assert!(
            root.join("a.txt").exists(),
            "must not move a file with unsaved changes"
        );
        assert!(!root.join("dest/a.txt").exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn copy_recurses_into_a_directory() {
        let root = project(&[], &["src_dir", "dest"]);
        std::fs::write(root.join("src_dir/inner.txt"), "nested").unwrap();
        let mut e = editor_with_tree(&root);
        // nodes sorted alphabetically among dirs: dest, src_dir
        handle_key(&mut e, Key::Char('j')); // onto src_dir
        handle_key(&mut e, Key::Char('y'));
        handle_key(&mut e, Key::Char('k')); // onto dest
        handle_key(&mut e, Key::Char('p'));
        assert_eq!(
            std::fs::read_to_string(root.join("dest/src_dir/inner.txt")).unwrap(),
            "nested"
        );
        assert!(
            root.join("src_dir/inner.txt").exists(),
            "copy must keep the original directory"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn copy_recursive_rolls_back_a_partial_copy_on_failure() {
        let root = project(&[], &["src_dir"]);
        std::fs::write(root.join("src_dir/ok.txt"), "fine").unwrap();
        std::fs::write(root.join("src_dir/blocked.txt"), "denied").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join("src_dir/blocked.txt"),
                std::fs::Permissions::from_mode(0o000),
            )
            .unwrap();
        }
        let dest = root.join("dest_dir");
        let result = copy_recursive(&root.join("src_dir"), &dest);
        assert!(result.is_err(), "the unreadable file should fail the copy");
        assert!(
            !dest.exists(),
            "a failed copy should roll back, leaving no partial destination \
             (even just the empty directory) behind"
        );
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join("src_dir/blocked.txt"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn m_toggles_a_bookmark_on_and_off() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        let path = e.file_tree.as_ref().unwrap().nodes[0].path.clone();
        handle_key(&mut e, Key::Char('m'));
        assert!(e.file_tree.as_ref().unwrap().bookmarks.contains(&path));
        handle_key(&mut e, Key::Char('m'));
        assert!(!e.file_tree.as_ref().unwrap().bookmarks.contains(&path));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn treebookmarks_lists_bookmarks_and_enter_opens_a_file_one() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('m'));
        e.show_tree_bookmarks();
        let r = e
            .results
            .as_ref()
            .expect("treebookmarks should open a results list");
        assert_eq!(r.entries.len(), 1);
        e.open_result();
        assert_eq!(e.buf().path, Some(root.join("a.txt")));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn treebookmarks_enter_on_a_directory_reveals_it_instead_of_opening_it() {
        let root = project(&[], &["sub"]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('m'));
        e.show_tree_bookmarks();
        e.open_result();
        assert!(
            e.windows.iter().any(|w| w.file_tree),
            "reveal must (re)open the tree sidebar"
        );
        let t = e.file_tree.as_ref().unwrap();
        assert_eq!(t.nodes[t.cursor].path, root.join("sub"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn treebookmarks_with_none_set_shows_a_message_not_an_empty_list() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.show_tree_bookmarks();
        assert!(e.results.is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn slash_live_filters_the_currently_loaded_nodes() {
        let root = project(&["apple.txt", "banana.txt", "cherry.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        assert!(e.file_tree.as_ref().unwrap().filter_input);
        for c in "an".chars() {
            handle_key(&mut e, Key::Char(c));
        }
        assert_eq!(names(e.file_tree.as_ref().unwrap()), vec!["banana.txt"]);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn project_wide_filter_finds_files_in_unexpanded_dirs() {
        let root = project(&["top.txt"], &["src/deep"]);
        std::fs::write(root.join("src/deep/needle.rs"), "x").unwrap();
        let mut e = editor_with_tree(&root);
        assert!(!names(e.file_tree.as_ref().unwrap()).contains(&"needle.rs"));
        e.all_files = vec!["src/deep/needle.rs".into(), "top.txt".into()];
        e.search_job.files_ready = true;
        handle_key(&mut e, Key::Char('/'));
        for c in "needle".chars() {
            handle_key(&mut e, Key::Char(c));
        }
        let t = e.file_tree.as_ref().unwrap();
        assert_eq!(
            names(t),
            vec!["src", "deep", "needle.rs"],
            "hit shown with its ancestors"
        );
        assert_eq!(
            t.nodes[t.cursor].name, "needle.rs",
            "cursor on the best hit"
        );
        handle_key(&mut e, Key::Enter); // stop typing, keep filter
        handle_key(&mut e, Key::Esc); // clear filter; keep the selection revealed
        let t = e.file_tree.as_ref().unwrap();
        assert!(t.filter.is_empty());
        assert_eq!(t.nodes[t.cursor].path, root.join("src/deep/needle.rs"));
        assert!(names(t).contains(&"top.txt"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn backspace_narrows_the_filter_back_toward_showing_more() {
        let root = project(&["apple.txt", "banana.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('b'));
        assert_eq!(e.file_tree.as_ref().unwrap().nodes.len(), 1);
        handle_key(&mut e, Key::Backspace);
        assert_eq!(
            e.file_tree.as_ref().unwrap().nodes.len(),
            2,
            "clearing the query back to empty should show everything again"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn esc_clears_the_filter_and_enter_keeps_it() {
        let root = project(&["apple.txt", "banana.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('b'));
        handle_key(&mut e, Key::Esc);
        assert!(e.file_tree.as_ref().unwrap().filter.is_empty());
        assert!(!e.file_tree.as_ref().unwrap().filter_input);
        assert_eq!(
            e.file_tree.as_ref().unwrap().nodes.len(),
            2,
            "Esc must clear the filter, not just stop typing"
        );

        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('b'));
        handle_key(&mut e, Key::Enter);
        assert!(
            !e.file_tree.as_ref().unwrap().filter_input,
            "Enter should stop typing"
        );
        assert_eq!(
            e.file_tree.as_ref().unwrap().nodes.len(),
            1,
            "Enter must keep the filter applied, unlike Esc"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn filter_input_intercepts_letters_that_are_normally_tree_commands() {
        // While typing a filter, 'd' must become part of the query, not
        // arm a delete -- otherwise every filename containing a command
        // letter would be untypeable.
        let root = project(&["deleteme.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('d'));
        assert_eq!(e.file_tree.as_ref().unwrap().filter, "d");
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_none());
        assert!(root.join("deleteme.txt").exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn counts_and_g_prefix_motions() {
        let names_: Vec<String> = (0..10).map(|i| format!("f{i}.txt")).collect();
        let refs: Vec<&str> = names_.iter().map(String::as_str).collect();
        let root = project(&refs, &[]);
        let mut e = editor_with_tree(&root);
        e.file_tree.as_mut().unwrap().cursor = 0;
        handle_key(&mut e, Key::Char('3'));
        handle_key(&mut e, Key::Char('j'));
        assert_eq!(e.file_tree.as_ref().unwrap().cursor, 3);
        handle_key(&mut e, Key::Char('G'));
        assert_eq!(e.file_tree.as_ref().unwrap().cursor, 9);
        handle_key(&mut e, Key::Char('g'));
        handle_key(&mut e, Key::Char('g'));
        assert_eq!(e.file_tree.as_ref().unwrap().cursor, 0);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn root_up_and_into_a_directory() {
        let root = project(&["top.txt"], &["sub"]);
        std::fs::write(root.join("sub/inner.txt"), "x").unwrap();
        let mut e = editor_with_tree(&root);
        e.file_tree.as_mut().unwrap().cursor = 0; // sub
        handle_key(&mut e, Key::Char('C'));
        let t = e.file_tree.as_ref().unwrap();
        assert_eq!(t.root, root.join("sub"));
        assert_eq!(names(t), vec!["inner.txt"]);
        handle_key(&mut e, Key::Char('-'));
        let t = e.file_tree.as_ref().unwrap();
        assert_eq!(t.root, root);
        assert_eq!(
            t.nodes[t.cursor].path,
            root.join("sub"),
            "cursor lands on the old root"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn help_overlay_is_dismissed_by_any_key() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('?'));
        assert!(e.file_tree.as_ref().unwrap().show_help);
        handle_key(&mut e, Key::Char('d'));
        assert!(!e.file_tree.as_ref().unwrap().show_help);
        assert!(
            e.file_tree.as_ref().unwrap().confirm_delete.is_none(),
            "the dismissing key is swallowed"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// `cargo test --release filetree::tests::benchmark -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn benchmark_large_tree_operations() {
        let root = project(&[], &[]);
        for d in 0..100 {
            let dir = root.join(format!("dir{d:03}"));
            std::fs::create_dir_all(&dir).unwrap();
            for f in 0..500 {
                std::fs::write(dir.join(format!("file{f}.rs")), "").unwrap();
            }
        }
        let time = |label: &str, f: &mut dyn FnMut()| {
            let start = Instant::now();
            f();
            eprintln!(
                "{label:<44} {:>9.3} ms",
                start.elapsed().as_secs_f64() * 1e3
            );
        };
        let mut t = FileTree::new(root.clone());
        let (_, capped) = t.expand_all(&root);
        assert!(capped, "E stops at EXPAND_ALL_LIMIT entries");
        t.cache.clear();
        time("expand 100 dirs (cold: read_dir 50k entries)", &mut || {
            for d in 0..100 {
                t.expanded.insert(root.join(format!("dir{d:03}")));
            }
            t.rebuild();
        });
        assert_eq!(t.nodes.len(), 100 + 50_000);
        time("rebuild 50k visible nodes (warm cache)", &mut || {
            t.rebuild()
        });
        t.cursor = t.nodes.iter().position(|n| n.name == "dir050").unwrap();
        time("collapse one dir + rebuild", &mut || {
            t.activate();
        });
        assert_eq!(t.nodes.len(), 100 + 49_500);
        time("expand it again (one stat + cached listing)", &mut || {
            t.activate();
        });
        time("toggle dotfiles (no disk access)", &mut || {
            t.show_hidden = !t.show_hidden;
            t.rebuild();
        });
        time("staleness check (101 stats, nothing changed)", &mut || {
            assert!(!t.refresh_stale());
        });
        let files: Vec<PathBuf> = (0..100)
            .flat_map(|d| (0..500).map(move |f| PathBuf::from(format!("dir{d:03}/file{f}.rs"))))
            .collect();
        time("filter build from 300 project-wide hits", &mut || {
            t.filter = "file42".into();
            t.set_filter_hits(Some(files.iter().take(300).map(|f| root.join(f)).collect()));
        });
        t.clear_filter();
        time("collapse all", &mut || t.collapse_all());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn git_rollup_prefers_the_most_significant_letter() {
        let root = project(&[], &["a/b"]);
        let mut t = FileTree::new(root.clone());
        let mut status = HashMap::new();
        status.insert(root.join("a/new.txt"), '?');
        status.insert(root.join("a/b/changed.txt"), 'M');
        t.apply_git((Some(status), None));
        assert_eq!(t.git_marker(&root.join("a"), true), Some('M'));
        assert_eq!(t.git_marker(&root.join("a/b"), true), Some('M'));
        assert_eq!(t.git_marker(&root.join("a/new.txt"), false), Some('?'));
        std::fs::remove_dir_all(root).ok();
    }
}
