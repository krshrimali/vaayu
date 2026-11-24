//! The review screen: the files a change touched, listed beside the code
//! as a table of contents, and a focus mode that dims every line the change
//! left alone.
//!
//! ```text
//! :reviewstart [REV]           (Enter on a :worktrees row runs it too)
//!        |
//!        v
//! base = REV | the active review's base | the worktree's record
//!        | merge-base with the default branch | HEAD
//!        |
//!        v
//! git diff --name-status/--numstat base, plus untracked files
//!        |                         --> TocFile rows: status, path, +A −D
//!        v
//! ToC pane, pinned left/right/top/bottom (:reviewtoc moves it)
//!        |  Enter / click
//!        v
//! the file in the edit pane, diffed against base (FileDiff):
//!   changed lines as usual, unchanged lines dimmed, the cursor line and
//!   the line under the mouse pointer lit, removed lines as virtual text
//! ```
//!
//! Example: base `3f2a1c9`; `src/parser.rs` gained lines 40-45 and lost two
//! lines before line 80. The ToC shows `M src/parser.rs  +6 −2`. Opening it
//! puts the cursor on line 40: lines 1-39 and 46 onwards are dim, 40-45 are
//! not, and line 80 ends with `-2 lines: <first removed line>`. `]c` / `[c`
//! step between the two hunks, `]F` / `[F` between files, `,rc` comments and
//! `:reviewcopy` puts every comment on the clipboard for the agent.
//!
//! Comments themselves are the ordinary review comments (`crate::notes`);
//! the screen only joins or starts the review whose base it shows.
use crate::{
    editor::Editor,
    gitdiff::{GitGutter, Sign},
    key::Key,
    results::{Entry, Results},
    review_store::{AnchorState, Comment, Op, ReviewStatus},
    windows::Layout,
};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

/// How often an open table of contents re-reads `git diff`, so changes an
/// agent makes while you review show up without a key press.
const REFRESH_EVERY: Duration = Duration::from_secs(4);
const DEFAULT_WIDTH: usize = 40;
const DEFAULT_HEIGHT: usize = 10;
/// Untracked files above this size are listed without a line count.
const MAX_COUNTED_BYTES: u64 = 4 * 1024 * 1024;
/// Code lines quoted per comment by `:reviewcopy`.
const MAX_QUOTED_LINES: usize = 40;

/// Which screen edge the table of contents is pinned to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TocSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl TocSide {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" | "l" => Some(Self::Left),
            "right" | "r" => Some(Self::Right),
            "top" | "t" | "above" => Some(Self::Top),
            "bottom" | "b" | "below" => Some(Self::Bottom),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
    /// Left and right are sidebars beside the code (a vertical divider).
    pub fn vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
    /// `m` in the pane and a bare `:reviewtoc` go round the edges.
    fn next(self) -> Self {
        match self {
            Self::Left => Self::Bottom,
            Self::Bottom => Self::Right,
            Self::Right => Self::Top,
            Self::Top => Self::Left,
        }
    }
}

/// One changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocFile {
    /// Relative to the work tree root; the new path of a rename.
    pub path: PathBuf,
    /// The path at the base, for a rename or copy.
    pub old_path: Option<PathBuf>,
    /// Git's status letter: `A` (added, or untracked), `M`, `D`, `R`, `C`, `T`.
    pub status: char,
    /// Lines added and removed; `None` for a binary file.
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

/// The table of contents. Kept after its pane closes (`q`) so the focus
/// mode and `]F` still know the change; `:reviewstop` drops it.
pub struct ReviewToc {
    pub root: PathBuf,
    pub base: String,
    pub title: String,
    pub description: String,
    pub files: Vec<TocFile>,
    pub cursor: usize,
    /// First file row shown (the file rows scroll under the header).
    pub top: usize,
    pub side: TocSide,
    /// Pane size when pinned left/right (cells) and top/bottom (rows);
    /// remembered across moves, closes and mouse-drag resizes.
    pub width: usize,
    pub height: usize,
    /// The file last opened from here.
    pub current: Option<PathBuf>,
    /// Files marked viewed with `v` (this session only).
    pub viewed: BTreeSet<PathBuf>,
    /// The scratch buffer showing each deleted file at the base, so
    /// stepping back onto one reuses it.
    base_views: HashMap<PathBuf, u64>,
    pub error: Option<String>,
    pub pending_g: bool,
    pub last_edit_window: usize,
    refreshed: Instant,
    job: Option<Receiver<Result<Vec<TocFile>, String>>>,
}

impl ReviewToc {
    fn new(root: PathBuf, base: String, files: Vec<TocFile>, side: TocSide) -> Self {
        Self {
            root,
            base,
            title: String::new(),
            description: String::new(),
            files,
            cursor: 0,
            top: 0,
            side,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            current: None,
            viewed: BTreeSet::new(),
            base_views: HashMap::new(),
            error: None,
            pending_g: false,
            last_edit_window: 0,
            refreshed: Instant::now(),
            job: None,
        }
    }

    pub fn short_base(&self) -> &str {
        &self.base[..self.base.len().min(10)]
    }

    /// Replaces the file list after a refresh, keeping the cursor on the
    /// same file. Returns whether anything visible changed.
    fn apply(&mut self, result: Result<Vec<TocFile>, String>) -> bool {
        match result {
            Ok(files) => {
                let changed = files != self.files || self.error.is_some();
                if changed {
                    let at = self.files.get(self.cursor).map(|f| f.path.clone());
                    self.files = files;
                    self.error = None;
                    self.cursor = at
                        .and_then(|p| self.files.iter().position(|f| f.path == p))
                        .unwrap_or(self.cursor)
                        .min(self.files.len().saturating_sub(1));
                }
                changed
            }
            Err(e) => {
                let changed = self.error.as_deref() != Some(e.as_str());
                self.error = Some(e);
                changed
            }
        }
    }

    fn totals(&self) -> (usize, usize) {
        self.files.iter().fold((0, 0), |(a, d), f| {
            (a + f.added.unwrap_or(0), d + f.removed.unwrap_or(0))
        })
    }

    /// The pane's rows for a `width` x `height` pane: a fixed header, the
    /// number of file rows that fit, and an optional key-hint footer.
    pub fn layout(&self, width: usize, height: usize) -> TocLayout {
        let (added, removed) = self.totals();
        let n = self.files.len();
        let stats = format!(
            "base {} · {n} file{} · ",
            self.short_base(),
            if n == 1 { "" } else { "s" }
        );
        let title = if self.title.is_empty() {
            "Review".to_string()
        } else {
            self.title.clone()
        };
        let mut header: Vec<Vec<(String, Role)>> = Vec::new();
        if self.side.vertical() {
            header.push(vec![
                (" REVIEW ".into(), Role::Badge),
                (format!(" {title}"), Role::Title),
            ]);
            header.push(vec![
                (format!(" {stats}"), Role::Muted),
                (format!("+{added}"), Role::Added),
                (" ".into(), Role::Muted),
                (format!("−{removed}"), Role::Removed),
            ]);
            // A few lines of what the change is for, when a worktree record
            // says, wrapped to the pane.
            let room = height.saturating_sub(8).min(3);
            let wrapped = wrap(&self.description, width.saturating_sub(2));
            for line in wrapped.iter().take(room) {
                header.push(vec![(format!(" {line}"), Role::Muted)]);
            }
            if wrapped.len() > room && room > 0 {
                if let Some(last) = header.last_mut() {
                    last.push(("…".into(), Role::Muted));
                }
            }
            header.push(vec![("─".repeat(width), Role::Rule)]);
        } else {
            header.push(vec![
                (" REVIEW ".into(), Role::Badge),
                (format!(" {title}  "), Role::Title),
                (stats, Role::Muted),
                (format!("+{added}"), Role::Added),
                (" ".into(), Role::Muted),
                (format!("−{removed}"), Role::Removed),
                (
                    "  Enter open · v viewed · m move · ? keys".into(),
                    Role::Muted,
                ),
            ]);
        }
        if let Some(e) = &self.error {
            header.push(vec![(format!(" {e}"), Role::Removed)]);
        }
        let header = header.into_iter().take(height).collect::<Vec<_>>();
        let footer = (self.side.vertical() && height >= header.len() + 4).then(|| {
            vec![(
                " Enter open · v viewed · m move · ? keys".to_string(),
                Role::Muted,
            )]
        });
        let body = height
            .saturating_sub(header.len())
            .saturating_sub(usize::from(footer.is_some()));
        TocLayout {
            header,
            body,
            footer,
        }
    }

    /// One file row, `width` cells wide: marker, status, path (shortened
    /// from the left when it does not fit) and `+A −D` on the right, then
    /// the file's open comment count.
    pub fn file_row(&self, i: usize, width: usize, comments: usize) -> Vec<(String, Role)> {
        let Some(f) = self.files.get(i) else {
            return Vec::new();
        };
        let viewed = self.viewed.contains(&f.path);
        let (marker, marker_role) = if self.current.as_ref() == Some(&f.path) {
            ("▶", Role::Current)
        } else if viewed {
            ("✓", Role::Viewed)
        } else {
            (" ", Role::Text)
        };
        let mut right: Vec<(String, Role)> = Vec::new();
        match (f.added, f.removed) {
            (Some(a), Some(d)) => {
                if a > 0 || d == 0 {
                    right.push((format!("+{a}"), Role::Added));
                }
                if d > 0 {
                    if !right.is_empty() {
                        right.push((" ".into(), Role::Text));
                    }
                    right.push((format!("−{d}"), Role::Removed));
                }
            }
            _ => right.push(("bin".into(), Role::Muted)),
        }
        if comments > 0 {
            right.push((format!(" ●{comments}"), Role::Comment));
        }
        right.push((" ".into(), Role::Text));
        let right_w: usize = right.iter().map(|(s, _)| s.width()).sum();
        let room = width.saturating_sub(3 + right_w + 1);
        let path = f.path.display().to_string();
        let path = if path.width() > room {
            let tail: String = path
                .chars()
                .rev()
                .scan(0, |w, c| {
                    *w += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    (*w < room).then_some(c)
                })
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("…{tail}")
        } else {
            path
        };
        let gap = width.saturating_sub(3 + path.width() + right_w);
        let mut row = vec![
            (marker.to_string(), marker_role),
            (f.status.to_string(), Role::Status(f.status)),
            (" ".into(), Role::Text),
            (path, if viewed { Role::Muted } else { Role::Text }),
            (" ".repeat(gap), Role::Text),
        ];
        row.extend(right);
        row
    }

    pub fn ensure_visible(&mut self, body: usize) {
        let body = body.max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + body {
            self.top = self.cursor + 1 - body;
        }
        self.top = self.top.min(self.files.len().saturating_sub(body));
    }

    pub fn scroll(&mut self, delta: isize, body: usize) {
        let max = self.files.len().saturating_sub(body.max(1));
        self.top = self.top.saturating_add_signed(delta).min(max);
        self.cursor = self
            .cursor
            .clamp(self.top, (self.top + body.max(1) - 1).max(self.top));
        self.cursor = self.cursor.min(self.files.len().saturating_sub(1));
    }
}

/// What a segment of a table-of-contents row is, for the renderer's colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Badge,
    Title,
    Muted,
    Text,
    Rule,
    Added,
    Removed,
    Status(char),
    Comment,
    Current,
    Viewed,
}

pub struct TocLayout {
    pub header: Vec<Vec<(String, Role)>>,
    pub body: usize,
    pub footer: Option<Vec<(String, Role)>>,
}

/// Word-wraps `text` to `width` cells (paragraph breaks kept as breaks).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for para in text.trim().lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.width() + 1 + word.width() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

/// One file in the edit pane, diffed against the review's base.
pub struct FileDiff {
    pub rel: PathBuf,
    pub base: String,
    pub seq: u64,
    /// The file at the base; `None` when it did not exist there.
    pub base_text: Option<String>,
    pub gutter: GitGutter,
}

impl FileDiff {
    /// Whether `line` is the change's own (added or modified), which the
    /// focus mode leaves lit.
    pub fn changed(&self, line: usize) -> bool {
        matches!(
            self.gutter.signs.get(&line),
            Some(Sign::Added | Sign::Modified)
        )
    }
}

/// The focus mode: per visible buffer, its diff against the base, plus the
/// line under the mouse pointer (lit like the cursor line).
#[derive(Default)]
pub struct ReviewFocus {
    pub on: bool,
    pub diffs: HashMap<u64, FileDiff>,
    pub hover: Option<(u64, usize)>,
}

/// A full or abbreviated commit id, which `git diff` cannot mistake for an
/// option or a path.
pub fn looks_like_sha(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("failed")
        ));
    }
    Ok(out.stdout)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    git_bytes(root, args).map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

/// NUL-separated fields, as `-z` output gives them.
fn split_z(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect()
}

/// Parses `git diff --name-status -z`: `M\0path\0`, or `R100\0old\0new\0`
/// for a rename or copy.
pub fn parse_name_status(bytes: &[u8]) -> Vec<TocFile> {
    let fields = split_z(bytes);
    let mut out = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let status = fields[i].chars().next().unwrap_or('M');
        i += 1;
        if matches!(status, 'R' | 'C') {
            let (Some(old), Some(new)) = (fields.get(i), fields.get(i + 1)) else {
                break;
            };
            out.push(TocFile {
                path: PathBuf::from(new),
                old_path: Some(PathBuf::from(old)),
                status,
                added: Some(0),
                removed: Some(0),
            });
            i += 2;
        } else {
            let Some(path) = fields.get(i) else {
                break;
            };
            out.push(TocFile {
                path: PathBuf::from(path),
                old_path: None,
                status,
                added: Some(0),
                removed: Some(0),
            });
            i += 1;
        }
    }
    out
}

/// Fills in line counts from `git diff --numstat -z`: `A\tD\tpath\0`, or
/// `A\tD\t\0old\0new\0` for a rename; `-\t-` marks a binary file.
pub fn apply_numstat(files: &mut [TocFile], bytes: &[u8]) {
    let fields = split_z(bytes);
    let mut i = 0;
    while i < fields.len() {
        let mut cols = fields[i].splitn(3, '\t');
        i += 1;
        let added = cols.next().and_then(|n| n.parse::<usize>().ok());
        let removed = cols.next().and_then(|n| n.parse::<usize>().ok());
        let path = match cols.next() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => {
                let new = fields.get(i + 1).cloned().unwrap_or_default();
                i += 2;
                new
            }
        };
        if let Some(f) = files.iter_mut().find(|f| f.path == Path::new(&path)) {
            f.added = added;
            f.removed = removed;
        }
    }
}

/// Lines in an untracked file; `None` when it looks binary or is too big.
fn count_lines(path: &Path) -> Option<usize> {
    if std::fs::metadata(path).ok()?.len() > MAX_COUNTED_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return None;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    Some(newlines + usize::from(bytes.last().is_some_and(|b| *b != b'\n')))
}

/// The files changed between `base` and the working tree under `root`
/// (staged, unstaged and untracked-but-not-ignored), sorted by path.
pub fn changed_files(root: &Path, base: &str) -> Result<Vec<TocFile>, String> {
    if !looks_like_sha(base) {
        return Err(format!("Not a commit id: {base}"));
    }
    let status = git_bytes(root, &["diff", "--name-status", "-z", "-M", base, "--"])?;
    let numstat = git_bytes(root, &["diff", "--numstat", "-z", "-M", base, "--"])?;
    let untracked = git_bytes(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut files = parse_name_status(&status);
    apply_numstat(&mut files, &numstat);
    for path in split_z(&untracked) {
        let path = PathBuf::from(path);
        if files.iter().any(|f| f.path == path) {
            continue;
        }
        let lines = count_lines(&root.join(&path));
        files.push(TocFile {
            path,
            old_path: None,
            status: 'A',
            added: lines,
            removed: lines.map(|_| 0),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn rev(root: &Path, name: &str) -> Option<String> {
    git_text(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )
    .ok()
    .filter(|s| looks_like_sha(s))
}

/// The base when nothing names one: where this branch left the default
/// branch, or HEAD itself (so only uncommitted work shows) on the default
/// branch.
fn default_base(root: &Path) -> Result<String, String> {
    let head = rev(root, "HEAD").ok_or("This repository has no commits yet")?;
    for name in [
        "origin/HEAD",
        "main",
        "master",
        "origin/main",
        "origin/master",
    ] {
        let Some(other) = rev(root, name) else {
            continue;
        };
        if let Ok(fork) = git_text(root, &["merge-base", "HEAD", &other]) {
            if looks_like_sha(&fork) && fork != head {
                return Ok(fork);
            }
        }
    }
    Ok(head)
}

/// The file as it was at `base`; `None` when it did not exist there.
fn base_version(root: &Path, base: &str, rel: &Path) -> Option<String> {
    let spec = format!("{base}:{}", rel.to_string_lossy().replace('\\', "/"));
    git_bytes(root, &["show", &spec])
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn current_branch(root: &Path) -> Option<String> {
    crate::notes::git_branch(root)
        .or_else(|| git_text(root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok())
        .filter(|b| !b.is_empty() && b != "HEAD")
}

/// The Markdown fence language for a path (its extension, which most
/// renderers and agents read).
fn fence_lang(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

impl Editor {
    pub fn active_review_toc(&self) -> bool {
        self.windows
            .get(self.active_window)
            .is_some_and(|w| w.review_toc)
    }

    fn review_toc_window(&self) -> Option<usize> {
        self.windows.iter().position(|w| w.review_toc)
    }

    /// The diff the focus mode shows for buffer `id`, when that buffer is
    /// one of the review screen's files and the focus mode is on.
    pub fn review_diff(&self, id: u64) -> Option<&FileDiff> {
        if !self.review_focus.on || self.review_toc.is_none() {
            return None;
        }
        self.review_focus.diffs.get(&id)
    }

    /// `:reviewstart [REV]`: opens the review screen for the changes since
    /// REV (see the module docs for how the base is picked otherwise).
    pub fn review_start(&mut self, arg: &str) {
        let root = self.reviews.root.clone();
        // Validation check: the screen compares files with a Git commit.
        if !root.join(".git").exists() {
            self.set_message("Not in a Git work tree: :reviewstart compares files with a commit");
            return;
        }
        let record = crate::worktrees::record_for(&root);
        let arg = arg.trim();
        let active_base = self
            .reviews
            .view()
            .filter(|v| v.status == ReviewStatus::Open)
            .and_then(|v| v.base.clone());
        let record_base = record
            .as_ref()
            .map(|r| r.base.trim().to_string())
            .filter(|b| !b.is_empty())
            .and_then(|b| self.resolve_rev(&b).ok());
        let base = if !arg.is_empty() {
            match self.resolve_rev(arg) {
                Ok(b) => b,
                Err(e) => return self.set_message(e),
            }
        } else if let Some(b) = active_base.or(record_base) {
            b
        } else {
            match default_base(&root) {
                Ok(b) => b,
                Err(e) => return self.set_message(e),
            }
        };
        let title = record
            .as_ref()
            .map(|r| r.title.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| current_branch(&root).map(|b| format!("Review of {b}")))
            .unwrap_or_else(|| "Review".to_string());
        if let Err(e) = self.join_review(&title, &base) {
            self.set_message(format!("Could not open the review: {e}"));
            return;
        }
        let files = match changed_files(&root, &base) {
            Ok(f) => f,
            Err(e) => return self.set_message(e),
        };
        let side = TocSide::parse(&self.config.review_toc).unwrap_or(TocSide::Left);
        let mut toc = ReviewToc::new(root.clone(), base.clone(), files, side);
        if let Some(old) = self.review_toc.take().filter(|t| t.root == root) {
            toc.side = old.side;
            toc.width = old.width;
            toc.height = old.height;
            if old.base == base {
                toc.viewed = old.viewed;
            }
        }
        toc.title = title;
        toc.description = record
            .as_ref()
            .map(|r| r.description.trim().to_string())
            .unwrap_or_default();
        let n = toc.files.len();
        // Start on the file being edited when the change touches it, else
        // the first file the record asks to look at, else the first one.
        let here = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| p.strip_prefix(&root).ok())
            .map(Path::to_path_buf);
        let start = here
            .and_then(|h| toc.files.iter().position(|f| f.path == h))
            .or_else(|| {
                record.as_ref().and_then(|r| {
                    r.focus
                        .iter()
                        .find_map(|p| toc.files.iter().position(|f| f.path == Path::new(p)))
                })
            })
            .unwrap_or(0);
        let short = toc.short_base().to_string();
        self.review_toc = Some(toc);
        self.review_focus.on = true;
        self.review_focus.diffs.clear();
        self.open_review_toc_pane();
        if n == 0 {
            self.set_message(format!(
                "No changes since {short} yet — the list refreshes on its own"
            ));
            return;
        }
        self.review_toc_open(start, false);
        self.set_message(format!(
            "Reviewing {n} file{} changed since {short} — ]F/[F files · ]c/[c hunks · ,rc comment · :reviewcopy",
            if n == 1 { "" } else { "s" }
        ));
    }

    /// Makes the review for `base` active: the active one when it already
    /// compares with `base`, else an open one that does, else the active
    /// one when it is still empty and has no base, else a new one.
    fn join_review(&mut self, title: &str, base: &str) -> anyhow::Result<()> {
        let open_with_base =
            |status: ReviewStatus, b: Option<&str>| status == ReviewStatus::Open && b == Some(base);
        if self
            .reviews
            .view()
            .is_some_and(|v| open_with_base(v.status, v.base.as_deref()))
        {
            return Ok(());
        }
        let existing = self
            .reviews
            .list()
            .into_iter()
            .find(|s| s.error.is_none() && open_with_base(s.status, s.base.as_deref()));
        if let Some(s) = existing {
            self.review_switch_to(&s.id, false);
            return Ok(());
        }
        if self.reviews.view().is_some_and(|v| {
            v.status == ReviewStatus::Open && v.base.is_none() && v.comments.is_empty()
        }) {
            self.reviews
                .apply_batch(vec![Op::Base(Some(base.to_string()))])?;
            return Ok(());
        }
        self.reviews.create(title, Some(base.to_string()))?;
        self.review_ui.live.clear();
        self.review_ui.live_gen += 1;
        Ok(())
    }

    /// `:reviewstop`: closes the table of contents and turns the focus mode
    /// off. The review and its comments stay.
    pub fn review_stop(&mut self) {
        if self.review_toc.is_none() {
            self.set_message("No review screen open");
            return;
        }
        self.close_review_toc_pane();
        self.review_toc = None;
        self.review_focus = Default::default();
        self.set_message("Review screen closed — comments are kept (:comments lists them)");
    }

    /// `,rt`: shows the table of contents again (or starts the review screen),
    /// or closes it when it is open.
    pub fn review_toc_toggle(&mut self) {
        if self.review_toc_window().is_some() {
            self.close_review_toc_pane();
        } else if self.review_toc.is_some() {
            self.open_review_toc_pane();
            if let Some(idx) = self.review_toc_window() {
                self.focus_window(idx);
            }
        } else {
            self.review_start("");
        }
    }

    /// `:reviewfocus [on|off]`: dims what the change left alone (toggles
    /// without an argument).
    pub fn review_focus_set(&mut self, arg: &str) {
        if self.review_toc.is_none() {
            self.set_message("No review screen — :reviewstart opens one");
            return;
        }
        let on = match arg.trim() {
            "" | "toggle" => !self.review_focus.on,
            "on" => true,
            "off" => false,
            other => {
                return self.set_message(format!("Usage: :reviewfocus [on|off] (not {other})"))
            }
        };
        self.review_focus.on = on;
        self.set_message(if on {
            "Review focus on: unchanged lines are dimmed"
        } else {
            "Review focus off"
        });
    }

    /// `rest` with the table of contents pinned at its edge.
    fn review_toc_layout(&self, leaf: usize, rest: Layout) -> Layout {
        let (side, width, height) = self
            .review_toc
            .as_ref()
            .map_or((TocSide::Left, DEFAULT_WIDTH, DEFAULT_HEIGHT), |t| {
                (t.side, t.width, t.height)
            });
        let leaf = Box::new(Layout::Leaf(leaf));
        let rest = Box::new(rest);
        let (first, second, fixed) = match side {
            TocSide::Left | TocSide::Top => (leaf, rest, (false, 0)),
            TocSide::Right | TocSide::Bottom => (rest, leaf, (true, 0)),
        };
        let size = if side.vertical() { width } else { height };
        Layout::Split {
            vertical: side.vertical(),
            first,
            second,
            ratio: 0.5,
            fixed: Some((fixed.0, size)),
        }
    }

    /// Remembers a mouse-dragged pane size before the pane moves or closes.
    fn remember_toc_size(&mut self, idx: usize) {
        let size = self.window_layout.as_ref().and_then(|l| l.fixed_size(idx));
        if let (Some(size), Some(t)) = (size, self.review_toc.as_mut()) {
            if t.side.vertical() {
                t.width = size;
            } else {
                t.height = size;
            }
        }
    }

    fn open_review_toc_pane(&mut self) {
        if self.review_toc_window().is_some() {
            return;
        }
        if self.windows.len() >= 32 {
            self.set_message("At most 32 panes are supported");
            return;
        }
        self.store_window();
        if self.windows.is_empty() {
            self.windows.push(self.capture_window());
            self.window_layout = Some(Layout::Leaf(0));
        }
        let edit = if crate::filetree::is_edit_window(&self.windows[self.active_window]) {
            Some(self.active_window)
        } else {
            self.windows
                .iter()
                .position(crate::filetree::is_edit_window)
        };
        let idx = self.windows.len();
        let mut w = self.capture_window();
        w.review_toc = true;
        self.windows.push(w);
        let rest = self.window_layout.take().unwrap_or(Layout::Leaf(0));
        self.window_layout = Some(self.review_toc_layout(idx, rest));
        if let Some(t) = &mut self.review_toc {
            t.last_edit_window = edit.unwrap_or(idx);
        }
        match edit {
            Some(e) => self.focus_window(e),
            None => self.focus_window(idx),
        }
    }

    fn close_review_toc_pane(&mut self) {
        let Some(idx) = self.review_toc_window() else {
            return;
        };
        self.remember_toc_size(idx);
        let back = self
            .review_toc
            .as_ref()
            .map(|t| t.last_edit_window)
            .filter(|&i| {
                i != idx
                    && self
                        .windows
                        .get(i)
                        .is_some_and(crate::filetree::is_edit_window)
            });
        if self.windows.len() == 1 {
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

    /// `:reviewtoc [left|right|top|bottom]`: moves the table of contents to
    /// that edge (the next edge round without an argument).
    pub fn review_toc_move(&mut self, arg: &str) {
        let Some(toc) = self.review_toc.as_ref() else {
            self.set_message("No review screen — :reviewstart opens one");
            return;
        };
        let side = if arg.trim().is_empty() {
            toc.side.next()
        } else {
            match TocSide::parse(arg) {
                Some(s) => s,
                None => {
                    return self.set_message("Usage: :reviewtoc [left|right|top|bottom]");
                }
            }
        };
        let pane = self.review_toc_window();
        if let Some(idx) = pane {
            self.remember_toc_size(idx);
        }
        if let Some(t) = self.review_toc.as_mut() {
            t.side = side;
        }
        match pane {
            Some(idx) => {
                self.store_window();
                let rest = self
                    .window_layout
                    .take()
                    .and_then(|l| l.remove_keep_index(idx));
                self.window_layout = Some(match rest {
                    Some(rest) => self.review_toc_layout(idx, rest),
                    None => Layout::Leaf(idx),
                });
            }
            None => self.open_review_toc_pane(),
        }
        self.set_message(format!("Review contents on the {}", side.name()));
    }

    /// The pane files open into: the one last used, else any editing pane,
    /// else a new one beside the table of contents.
    fn review_edit_window(&mut self) -> Option<usize> {
        let last = self.review_toc.as_ref().map(|t| t.last_edit_window);
        if let Some(i) = last
            .filter(|&i| {
                self.windows
                    .get(i)
                    .is_some_and(crate::filetree::is_edit_window)
            })
            .or_else(|| {
                self.windows
                    .iter()
                    .position(crate::filetree::is_edit_window)
            })
        {
            return Some(i);
        }
        let toc_idx = self.review_toc_window()?;
        if self.windows.len() >= 32 {
            return None;
        }
        let idx = self.windows.len();
        let mut w = self.capture_window();
        w.review_toc = false;
        w.file_tree = false;
        w.outline = false;
        w.terminal = None;
        w.preview = false;
        self.windows.push(w);
        let rest = self
            .window_layout
            .take()
            .and_then(|l| l.remove_keep_index(toc_idx))
            .map(|l| Layout::Split {
                vertical: false,
                first: Box::new(Layout::Leaf(idx)),
                second: Box::new(l),
                ratio: 0.7,
                fixed: None,
            })
            .unwrap_or(Layout::Leaf(idx));
        self.window_layout = Some(self.review_toc_layout(toc_idx, rest));
        Some(idx)
    }

    /// Opens file `idx` of the table of contents in the edit pane. A file
    /// opened for the first time starts at its first change; one already
    /// open keeps its cursor. `keep_focus` leaves focus on the contents.
    pub fn review_toc_open(&mut self, idx: usize, keep_focus: bool) {
        let Some(toc) = self.review_toc.as_mut() else {
            return;
        };
        let Some(file) = toc.files.get(idx).cloned() else {
            return;
        };
        toc.cursor = idx;
        toc.current = Some(file.path.clone());
        let (root, base) = (toc.root.clone(), toc.base.clone());
        if !self.windows.is_empty() {
            let Some(target) = self.review_edit_window() else {
                self.set_message("No pane to open the file in");
                return;
            };
            self.focus_window(target);
        }
        if file.status == 'D' {
            // A deleted file has nothing in the working tree, so show what it
            // held at the base in a scratch view (reused on later visits).
            let known = self
                .review_toc
                .as_ref()
                .and_then(|t| t.base_views.get(&file.path).copied())
                .and_then(|id| self.buffers.iter().position(|b| b.id == id));
            let idx = match known {
                Some(i) => i,
                None => {
                    let text = base_version(&root, &base, &file.path).unwrap_or_default();
                    let mut b = crate::buffer::Buffer::empty();
                    b.rope = ropey::Rope::from_str(&text);
                    b.mark_saved();
                    b.view = true;
                    if let Some(t) = self.review_toc.as_mut() {
                        t.base_views.insert(file.path.clone(), b.id);
                    }
                    self.buffers.push(b);
                    self.invalidate_index_caches();
                    self.buffers.len() - 1
                }
            };
            self.cur = idx;
            let id = self.buffers[idx].id;
            if let Some(w) = self.windows.get_mut(self.active_window) {
                w.buffer = id;
            }
            if known.is_none() {
                self.set_cursor(0, 0);
            }
            self.set_message(format!(
                "{} was deleted by this change — this is its content at {}",
                file.path.display(),
                &base[..base.len().min(10)]
            ));
        } else {
            let path = crate::files::identity(&root.join(&file.path));
            let was_open = self.buffers.iter().any(|b| b.path.as_ref() == Some(&path));
            if let Err(e) = self.open_file(path) {
                self.set_message(e.to_string());
                return;
            }
            self.ensure_review_focus();
            let first = self.review_diff(self.buf().id).and_then(|d| {
                let mut lines: Vec<usize> = d.gutter.signs.keys().copied().collect();
                lines.sort_unstable();
                lines.first().copied()
            });
            if let (false, Some(line)) = (was_open, first) {
                let col = self.buf().first_non_blank(line);
                self.set_cursor(line, col);
                self.buf_mut().top_line = line.saturating_sub(4);
            }
        }
        self.store_window();
        let edit = self.active_window;
        if let Some(t) = self.review_toc.as_mut() {
            t.last_edit_window = edit;
        }
        if keep_focus {
            if let Some(idx) = self.review_toc_window() {
                self.focus_window(idx);
            }
        }
    }

    /// `]F` / `[F`: the next / previous file of the review screen.
    pub fn review_file_step(&mut self, forward: bool) {
        let Some(toc) = self.review_toc.as_ref() else {
            self.set_message("No review screen — :reviewstart opens one");
            return;
        };
        let n = toc.files.len();
        if n == 0 {
            self.set_message("No changed files");
            return;
        }
        let here = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| p.strip_prefix(&toc.root).ok())
            .and_then(|r| toc.files.iter().position(|f| f.path == r))
            .or_else(|| {
                toc.current
                    .as_ref()
                    .and_then(|c| toc.files.iter().position(|f| &f.path == c))
            });
        let next = match (here, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        let label = format!("File {}/{n}: {}", next + 1, toc.files[next].path.display());
        self.review_toc_open(next, false);
        self.set_message(label);
    }

    /// A click on the table of contents: a file row opens that file.
    pub fn review_toc_click(&mut self, pane: usize, row: usize, cols: usize, rows: usize) {
        let Some(rect) = self.pane_rects(cols, rows).get(pane).copied() else {
            return;
        };
        let Some(toc) = self.review_toc.as_mut() else {
            return;
        };
        let layout = toc.layout(rect.width, rect.height);
        let Some(i) = row
            .checked_sub(layout.header.len())
            .filter(|r| *r < layout.body)
            .map(|r| toc.top + r)
            .filter(|i| *i < toc.files.len())
        else {
            return;
        };
        toc.cursor = i;
        self.review_toc_open(i, false);
    }

    /// Per frame: polls a background refresh and (re)diffs every visible
    /// review file whose text changed since its last diff.
    pub fn ensure_review_focus(&mut self) {
        let Some(toc) = self.review_toc.as_ref() else {
            if !self.review_focus.diffs.is_empty() {
                self.review_focus.diffs.clear();
            }
            return;
        };
        if !self.review_focus.on {
            return;
        }
        let mut ids: Vec<u64> = self
            .windows
            .iter()
            .filter(|w| crate::filetree::is_edit_window(w))
            .map(|w| w.buffer)
            .collect();
        ids.push(self.buffers[self.cur].id);
        ids.sort_unstable();
        ids.dedup();
        let large = self.config.large_file_kb.saturating_mul(1024);
        for id in ids {
            let Some(b) = self.buffers.iter().find(|b| b.id == id) else {
                continue;
            };
            let file = b
                .path
                .as_ref()
                .and_then(|p| p.strip_prefix(&toc.root).ok())
                .and_then(|rel| toc.files.iter().find(|f| f.path == rel))
                .filter(|f| f.status != 'D')
                .filter(|_| large == 0 || b.rope.len_bytes() <= large);
            let Some(file) = file else {
                self.review_focus.diffs.remove(&id);
                continue;
            };
            let seq = b.edit_seq;
            match self.review_focus.diffs.get_mut(&id) {
                Some(d) if d.base == toc.base && d.rel == file.path => {
                    if d.seq != seq {
                        d.gutter.refresh(&b.rope.to_string(), seq);
                        d.seq = seq;
                    }
                }
                _ => {
                    let at_base = file.old_path.as_ref().unwrap_or(&file.path);
                    let base_text = base_version(&toc.root, &toc.base, at_base);
                    let mut gutter = GitGutter::against(base_text.clone());
                    gutter.refresh(&b.rope.to_string(), seq);
                    self.review_focus.diffs.insert(
                        id,
                        FileDiff {
                            rel: file.path.clone(),
                            base: toc.base.clone(),
                            seq,
                            base_text,
                            gutter,
                        },
                    );
                }
            }
        }
        let live: BTreeSet<u64> = self.buffers.iter().map(|b| b.id).collect();
        self.review_focus.diffs.retain(|id, _| live.contains(id));
    }

    /// Background refresh of the file list while its pane is on screen.
    /// Returns whether the list changed (so the idle loop redraws).
    pub fn poll_review_toc(&mut self) -> bool {
        let visible = self.review_toc_window().is_some();
        let Some(toc) = self.review_toc.as_mut() else {
            return false;
        };
        if let Some(rx) = &toc.job {
            match rx.try_recv() {
                Ok(result) => {
                    toc.job = None;
                    toc.refreshed = Instant::now();
                    return toc.apply(result);
                }
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => toc.job = None,
            }
        }
        if visible && toc.refreshed.elapsed() >= REFRESH_EVERY {
            toc.refreshed = Instant::now();
            let (root, base) = (toc.root.clone(), toc.base.clone());
            let (tx, rx) = mpsc::channel();
            toc.job = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(changed_files(&root, &base));
            });
        }
        false
    }

    /// `r` in the contents: re-reads the file list now.
    fn review_toc_refresh(&mut self) {
        let Some(toc) = self.review_toc.as_mut() else {
            return;
        };
        let result = changed_files(&toc.root, &toc.base);
        toc.job = None;
        toc.refreshed = Instant::now();
        toc.apply(result);
        let n = toc.files.len();
        self.set_message(format!("{n} changed file{}", if n == 1 { "" } else { "s" }));
    }

    /// Mouse motion over an editing pane: the line under the pointer is lit
    /// like the cursor line while the focus mode dims the rest.
    pub fn review_hover_at(&mut self, cols: usize, rows: usize, x: usize, y: usize) -> bool {
        if !self.review_focus.on || self.review_focus.diffs.is_empty() {
            return false;
        }
        let hit =
            crate::render::locate_click(self, cols, rows, x, y).and_then(|(pane, line, _)| {
                let w = if self.windows.is_empty() {
                    self.capture_window()
                } else {
                    self.windows.get(pane)?.clone()
                };
                if !crate::filetree::is_edit_window(&w) {
                    return None;
                }
                let id = if pane == self.active_window {
                    self.buf().id
                } else {
                    w.buffer
                };
                self.review_focus
                    .diffs
                    .contains_key(&id)
                    .then_some((id, line))
            });
        let changed = self.review_focus.hover != hit;
        self.review_focus.hover = hit;
        changed
    }

    /// `:reviewhunk`: the hunk under the cursor against the review's base,
    /// removed lines included, as a list (Enter jumps to its line).
    pub fn review_hunk(&mut self) {
        let id = self.buf().id;
        let Some(d) = self.review_diff(id) else {
            self.set_message("Not a file of the review screen (:reviewstart)");
            return;
        };
        let base_text = d.base_text.clone().unwrap_or_default();
        let short = d.base[..d.base.len().min(10)].to_string();
        let now = self.buf().rope.to_string();
        let path = self.buf().path.clone();
        let line = self.cursor().0;
        let diff = similar::TextDiff::from_lines(&base_text, &now);
        let groups = diff.grouped_ops(3);
        let group = groups.iter().find(|g| {
            let (Some(first), Some(last)) = (g.first(), g.last()) else {
                return false;
            };
            let (start, end) = (first.new_range().start, last.new_range().end);
            line >= start && line < end.max(start + 1)
        });
        let Some(group) = group else {
            self.set_message("No change at the cursor (]c jumps to the next one)");
            return;
        };
        let mut entries = Vec::new();
        for op in group {
            for change in diff.iter_changes(op) {
                let (sign, at) = match change.tag() {
                    similar::ChangeTag::Delete => ('-', op.new_range().start),
                    similar::ChangeTag::Insert => ('+', change.new_index().unwrap_or(0)),
                    similar::ChangeTag::Equal => (' ', change.new_index().unwrap_or(0)),
                };
                let text = format!("{sign} {}", change.value().trim_end_matches(['\n', '\r']));
                let mut e = match &path {
                    Some(p) => Entry::location(p.clone(), at, 0, text),
                    None => Entry::text(text),
                };
                e.no_path_prefix = true;
                entries.push(e);
            }
        }
        self.show_results(Results::new(
            format!("Change at the cursor vs {short}"),
            entries,
        ));
    }

    /// `:reviewcopy [all]`: puts the review's open comments (all of them
    /// with `all`) on the clipboard as a prompt an agent can act on.
    pub fn review_copy(&mut self, arg: &str) {
        let all = arg.trim() == "all";
        match self.review_copy_text(all) {
            Ok((text, n)) => {
                self.registers.set(Some('+'), text, false);
                self.set_message(format!(
                    "Copied {n} comment{} for the agent — paste it into its prompt",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Err(e) => self.set_message(e),
        }
    }

    /// The text `:reviewcopy` copies, and how many comments it holds.
    pub fn review_copy_text(&mut self, all: bool) -> Result<(String, usize), String> {
        self.review_refresh_all();
        let Some(v) = self.reviews.view().cloned() else {
            return Err("No active review (:reviewstart or ,rc starts one)".into());
        };
        let mut comments: Vec<Comment> = v
            .comments
            .iter()
            .filter(|c| all || c.is_open())
            .cloned()
            .collect();
        if comments.is_empty() {
            return Err(if all {
                "This review has no comments yet (,rc adds one)".into()
            } else {
                "No open comments to copy (:reviewcopy all includes resolved ones)".into()
            });
        }
        comments.sort_by_key(|c| (c.file.clone(), self.comment_pos(c).0, c.seq));
        let root = self.reviews.root.clone();
        let record = crate::worktrees::record_for(&root);
        let mut out = String::new();
        out.push_str(&format!("# Code review from vaayu: {}\n\n", v.title));
        out.push_str(&format!("Repository: {}\n", root.display()));
        if let Some(b) = current_branch(&root) {
            out.push_str(&format!("Branch: {b}\n"));
        }
        if let Some(base) = &v.base {
            out.push_str(&format!(
                "Compared with: {} (the working tree, uncommitted changes included)\n",
                &base[..base.len().min(10)]
            ));
        }
        if let Some(r) = &record {
            out.push_str(&format!("Worktree record: {}\n", r.file.display()));
        }
        out.push_str(&format!(
            "\n{} comment{}. Please address each one in this repository. Then reply per \
             comment number with what you changed, or why you left it as is. The quoted \
             code is what the reviewer was looking at; the comments are the reviewer's \
             requests.\n",
            comments.len(),
            if comments.len() == 1 { "" } else { "s" }
        ));
        for c in &comments {
            let (s, e, state) = self.comment_pos(c);
            let place = if c.whole_file {
                format!("{} (whole file)", c.file.display())
            } else if s == e {
                format!("{}:{}", c.file.display(), s + 1)
            } else {
                format!("{}:{}-{}", c.file.display(), s + 1, e + 1)
            };
            let resolved = if c.is_open() { "" } else { " (resolved)" };
            out.push_str(&format!("\n## {} {place}{resolved}\n\n", c.label()));
            if !c.whole_file {
                let lines = self.preview_source_lines(&root.join(&c.file));
                let end = (e + 1).min(lines.len());
                if state == AnchorState::Ok && s < end {
                    let shown = (end - s).min(MAX_QUOTED_LINES);
                    out.push_str(&format!("```{}\n", fence_lang(&c.file)));
                    for line in &lines[s..s + shown] {
                        out.push_str(line);
                        out.push('\n');
                    }
                    if end - s > shown {
                        out.push_str(&format!("… {} more lines\n", end - s - shown));
                    }
                    out.push_str("```\n\n");
                } else if !c.anchor.is_empty() {
                    // The code moved or changed since the comment; quote what
                    // it was written against.
                    out.push_str("The code has changed since this comment. It was written on:\n\n");
                    out.push_str(&format!(
                        "```{}\n{}\n```\n\n",
                        fence_lang(&c.file),
                        c.anchor.trim_end()
                    ));
                }
            }
            out.push_str(c.text.trim_end());
            out.push('\n');
            for r in &c.replies {
                let who = if r.author.is_empty() {
                    if r.agent { "agent" } else { "reviewer" }.to_string()
                } else {
                    r.author.clone()
                };
                out.push_str(&format!(
                    "\n> {who}: {}\n",
                    r.text.trim().replace('\n', "\n> ")
                ));
            }
        }
        Ok((out, comments.len()))
    }
}

/// Keys while the table of contents has focus.
pub fn handle_key(ed: &mut Editor, key: Key) {
    let pending_g = ed.review_toc.as_ref().is_some_and(|t| t.pending_g);
    if let Some(t) = ed.review_toc.as_mut() {
        t.pending_g = false;
        if pending_g && key == Key::Char('g') {
            t.cursor = 0;
            return;
        }
    }
    let n = ed.review_toc.as_ref().map_or(0, |t| t.files.len());
    let page = ed.screen_rows.saturating_sub(4).max(2) / 2;
    let move_to = |ed: &mut Editor, f: &dyn Fn(usize) -> usize| {
        if let Some(t) = ed.review_toc.as_mut() {
            if n > 0 {
                t.cursor = f(t.cursor).min(n - 1);
            }
        }
    };
    let cursor = ed.review_toc.as_ref().map_or(0, |t| t.cursor);
    match key {
        Key::Char('j') | Key::Down => move_to(ed, &|c| c + 1),
        Key::Char('k') | Key::Up => move_to(ed, &|c| c.saturating_sub(1)),
        Key::Char('g') => {
            if let Some(t) = ed.review_toc.as_mut() {
                t.pending_g = true;
            }
        }
        Key::Home => move_to(ed, &|_| 0),
        Key::Char('G') | Key::End => move_to(ed, &|_| usize::MAX),
        Key::Ctrl('d') => move_to(ed, &|c| c + page),
        Key::Ctrl('u') => move_to(ed, &|c| c.saturating_sub(page)),
        Key::Enter | Key::Char('o') | Key::Char('l') => ed.review_toc_open(cursor, false),
        Key::Char('p') | Key::Tab => ed.review_toc_open(cursor, true),
        Key::Char('v') => {
            if let Some(t) = ed.review_toc.as_mut() {
                if let Some(path) = t.files.get(cursor).map(|f| f.path.clone()) {
                    if !t.viewed.remove(&path) {
                        t.viewed.insert(path);
                        t.cursor = (cursor + 1).min(n.saturating_sub(1));
                    }
                }
            }
        }
        Key::Char('r') | Key::Char('R') => ed.review_toc_refresh(),
        Key::Char('f') => ed.review_focus_set(""),
        Key::Char('m') => ed.review_toc_move(""),
        Key::Char('c') | Key::Char('y') => ed.review_copy(""),
        Key::Char('C') => ed.comments_results(),
        Key::Char('?') => ed.set_message(
            "Review contents: j/k move · Enter/o open · p/Tab open, stay here · v viewed · \
             f dim on/off · m move pane · r refresh · c copy comments · C comment list · q close",
        ),
        Key::Char(':') => ed.enter_command(crate::mode::CommandKind::Ex),
        Key::Char('q') | Key::Esc => ed.close_review_toc_pane(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_status_with_renames() {
        let raw = b"M\0src/a.rs\0R087\0old/b.rs\0new/b.rs\0A\0c.txt\0D\0gone.rs\0";
        let files = parse_name_status(raw);
        assert_eq!(
            files
                .iter()
                .map(|f| (f.status, f.path.display().to_string()))
                .collect::<Vec<_>>(),
            [
                ('M', "src/a.rs".into()),
                ('R', "new/b.rs".into()),
                ('A', "c.txt".into()),
                ('D', "gone.rs".into())
            ]
        );
        assert_eq!(files[1].old_path, Some(PathBuf::from("old/b.rs")));
    }

    #[test]
    fn numstat_fills_counts_including_renames_and_binaries() {
        let mut files = parse_name_status(b"M\0a.rs\0R090\0x.rs\0y.rs\0M\0img.png\0");
        apply_numstat(
            &mut files,
            b"3\t1\ta.rs\x002\t0\t\0x.rs\0y.rs\0-\t-\timg.png\0",
        );
        assert_eq!((files[0].added, files[0].removed), (Some(3), Some(1)));
        assert_eq!((files[1].added, files[1].removed), (Some(2), Some(0)));
        assert_eq!((files[2].added, files[2].removed), (None, None));
    }

    #[test]
    fn sides_parse_and_cycle_round_the_edges() {
        assert_eq!(TocSide::parse("Bottom"), Some(TocSide::Bottom));
        assert_eq!(TocSide::parse("t"), Some(TocSide::Top));
        assert_eq!(TocSide::parse("middle"), None);
        let mut s = TocSide::Left;
        let mut seen = Vec::new();
        for _ in 0..4 {
            s = s.next();
            seen.push(s);
        }
        assert_eq!(
            seen,
            [TocSide::Bottom, TocSide::Right, TocSide::Top, TocSide::Left]
        );
    }

    #[test]
    fn file_rows_shorten_long_paths_from_the_left_and_keep_counts() {
        let mut toc = ReviewToc::new(
            PathBuf::from("/r"),
            "0123456789abcdef".into(),
            vec![TocFile {
                path: PathBuf::from("src/deeply/nested/module/with/a/long/name.rs"),
                old_path: None,
                status: 'M',
                added: Some(12),
                removed: Some(3),
            }],
            TocSide::Left,
        );
        toc.current = Some(toc.files[0].path.clone());
        let row = toc.file_row(0, 30, 2);
        let text: String = row.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(text.width(), 30, "{text:?}");
        assert!(text.starts_with("▶M …"), "{text:?}");
        assert!(text.contains("name.rs"), "{text:?}");
        assert!(text.trim_end().ends_with("+12 −3 ●2"), "{text:?}");
        let layout = toc.layout(30, 20);
        let header: String = layout.header[1].iter().map(|(s, _)| s.as_str()).collect();
        assert!(header.contains("base 0123456789 · 1 file"), "{header:?}");
        assert!(layout.footer.is_some());
    }

    #[test]
    fn wrap_breaks_on_words() {
        assert_eq!(
            wrap("one two three four five", 9),
            ["one two", "three", "four five"]
        );
    }
}
