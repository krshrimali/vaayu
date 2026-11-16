//! Review comments in the editor: composing, live anchors, gutter marks,
//! navigation and the comments list. Storage and merging live in
//! [`crate::review_store`]; agent runs and review management in
//! [`crate::review`]. Source files are never modified.
use crate::{
    buffer::Buffer,
    editor::Editor,
    mode::Mode,
    results::{Entry, Results},
    review_store::{self as store, AnchorState, Comment, LineIndex, Op, Reply, Status},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

/// What a comment composer buffer writes when saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteRef {
    pub review: String,
    pub kind: NoteKind,
    /// Buffer name shown in the status line.
    pub label: String,
    /// The buffer to show again when the composer closes.
    #[serde(default)]
    pub return_to: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NoteKind {
    /// Editing a saved comment's text.
    Comment(u64),
    /// A new comment, written to the review on first save.
    Draft(Box<Draft>),
    /// A reply to a comment, added on save.
    Reply(u64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
    pub whole_file: bool,
    pub anchor: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkKind {
    Resolved,
    Open,
    Attention,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineMark {
    pub glyph: char,
    pub kind: MarkKind,
    /// Text after the line (comment start lines only).
    pub text: Option<String>,
}

#[derive(Default, Debug)]
pub struct LineMarks {
    pub lines: HashMap<usize, LineMark>,
    pub open: usize,
    pub file_comments: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListFilter {
    /// Unresolved comments (the default).
    #[default]
    Open,
    /// Unresolved comments whose code changed or whose file is gone.
    Attention,
    Resolved,
    All,
}
impl ListFilter {
    pub fn next(self) -> Self {
        match self {
            Self::Open => Self::Attention,
            Self::Attention => Self::Resolved,
            Self::Resolved => Self::All,
            Self::All => Self::Open,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Attention => "needs attention",
            Self::Resolved => "resolved",
            Self::All => "all",
        }
    }
    fn keeps(self, c: &Comment, state: AnchorState) -> bool {
        match self {
            Self::Open => c.is_open(),
            Self::Attention => c.is_open() && state != AnchorState::Ok,
            Self::Resolved => !c.is_open(),
            Self::All => true,
        }
    }
}

/// Which review-specific list a Results panel shows.
#[derive(Clone, Debug, PartialEq)]
pub enum ReviewList {
    Comments {
        filter: ListFilter,
        /// Comments awaiting a `y`/`n` delete confirmation.
        confirm_delete: Option<Vec<u64>>,
    },
    Reviews {
        confirm_delete: Option<String>,
    },
    Runs,
    AgentOutput,
}

/// Cached marks for a buffer: (review generation, live generation, edit seq).
type MarksEntry = (u64, u64, u64, Rc<LineMarks>);
/// A comment's position: (start, end, state).
type Pos = (usize, usize, AnchorState);

/// Editor-side review state that is not stored.
#[derive(Default)]
pub struct ReviewUi {
    /// Positions of comments in open buffers with edits not yet saved.
    pub live: HashMap<u64, (usize, usize, AnchorState)>,
    pub live_gen: u64,
    checked: HashMap<u64, (u64, bool, u64)>,
    /// A comment waiting for `,rm` to re-anchor it.
    pub moving: Option<u64>,
    /// Lines highlighted while a comment is being written or viewed.
    pub focus: Option<(PathBuf, usize, usize)>,
    marks: RefCell<HashMap<u64, MarksEntry>>,
    /// Per-file fingerprint of the last full re-anchoring pass.
    refreshed: HashMap<PathBuf, u64>,
    author: Option<String>,
    pub filter: ListFilter,
}

fn lines_of(b: &Buffer) -> Vec<String> {
    (0..b.line_count()).map(|i| b.line_text(i)).collect()
}

fn text_lines(t: &str) -> Vec<String> {
    t.lines().map(str::to_string).collect()
}

fn first_line(s: &str) -> &str {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
}

/// The Git branch checked out under `root`, read without running Git.
pub fn git_branch(root: &Path) -> Option<String> {
    let head = std::fs::read_to_string(root.join(".git/HEAD")).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
}

impl Editor {
    pub fn review_author(&mut self) -> String {
        if !self.config.review_author.trim().is_empty() {
            return self.config.review_author.trim().to_string();
        }
        if let Some(a) = &self.review_ui.author {
            return a.clone();
        }
        let name = std::process::Command::new("git")
            .args(["config", "user.name"])
            .current_dir(&self.reviews.root)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_else(|| "me".into());
        self.review_ui.author = Some(name.clone());
        name
    }

    /// Where a comment is now: its live position in an edited buffer, or
    /// the stored one.
    pub fn comment_pos(&self, c: &Comment) -> (usize, usize, AnchorState) {
        self.review_ui
            .live
            .get(&c.id)
            .copied()
            .unwrap_or((c.start, c.end, c.state))
    }

    fn review_title_default(&self) -> String {
        git_branch(&self.reviews.root)
            .map(|b| format!("Review of {b}"))
            .unwrap_or_else(|| "Review".into())
    }

    /// Reports store notices (imports, unreadable files) once.
    pub fn review_notices(&mut self) {
        if !self.reviews.notices.is_empty() {
            let n = self.reviews.notices.join(" · ");
            self.reviews.notices.clear();
            self.set_message(n);
        }
    }

    /// Applies ops to the review named `review`, which need not be active.
    pub(crate) fn review_apply(
        &mut self,
        review: &str,
        ops: Vec<Op>,
    ) -> anyhow::Result<Vec<String>> {
        if self.reviews.view().is_some_and(|v| v.id == review) {
            let r = self.reviews.apply_batch(ops);
            self.review_ui.checked.clear();
            return r;
        }
        let path = self.reviews.dir.join(format!("{review}.json"));
        anyhow::ensure!(path.is_file(), "that review no longer exists");
        let mut doc = store::ReviewDoc::open_existing(path);
        if let Some(e) = &doc.error {
            anyhow::bail!("cannot open review: {e}");
        }
        doc.apply_batch(ops)
    }

    fn warn(&mut self, base: impl Into<String>, warnings: Vec<String>) {
        let base = base.into();
        if warnings.is_empty() {
            self.set_message(base);
        } else {
            self.set_message(format!("{base} · {}", warnings.join("; ")));
        }
    }

    // ----- composing ---------------------------------------------------

    pub fn new_note(&mut self, whole_file: bool) {
        let visual = self.visual_anchor.take();
        if self.buf().note.is_some() {
            self.set_message("This is a comment; switch to the source file to add another");
            return;
        }
        let Some(path) = self.buf().path.clone() else {
            self.set_message("Open a source file before adding a comment");
            return;
        };
        let Some(file) = self.reviews.rel(&path) else {
            self.set_message(format!(
                "Comments must refer to files inside {}",
                self.reviews.root.display()
            ));
            return;
        };
        if matches!(self.mode, Mode::Visual(_)) {
            self.enter_normal();
        }
        if let Some(a) = &self.reviews.active {
            if let Some(e) = &a.error {
                self.set_message(format!(
                    "Cannot use this review: {e} (:reviews picks another)"
                ));
                return;
            }
            if a.view.status == store::ReviewStatus::Closed {
                self.set_message(format!(
                    "Review \"{}\" is closed — :reviewreopen or :reviewnew",
                    a.view.title
                ));
                return;
            }
        }
        let title = self.review_title_default();
        let review = match self.reviews.ensure_active(&title) {
            Ok(r) => r.view.id.clone(),
            Err(e) => {
                self.set_message(format!("Cannot create a review: {e}"));
                return;
            }
        };
        let cursor = self.cursor().0;
        let (a, b) = visual
            .map(|v| (v.0.min(cursor), v.0.max(cursor)))
            .unwrap_or((cursor, cursor));
        let lines = lines_of(self.buf());
        let (anchor, before, after) = if whole_file {
            (String::new(), Vec::new(), Vec::new())
        } else {
            store::capture(&lines, a, b)
        };
        let (a, b) = if whole_file { (0, 0) } else { (a, b) };
        let scope = if whole_file {
            "file".to_string()
        } else if a == b {
            format!("{}", a + 1)
        } else {
            format!("{}–{}", a + 1, b + 1)
        };
        let label = format!("New comment · {}:{scope}", file.display());
        let draft = Draft {
            file,
            start: a,
            end: b,
            whole_file,
            anchor,
            before,
            after,
        };
        self.review_ui.focus = (!whole_file).then(|| (path.clone(), a, b));
        let source = self.buf().id;
        self.open_composer(
            NoteRef {
                review,
                kind: NoteKind::Draft(Box::new(draft)),
                label,
                return_to: Some(source),
            },
            "",
            true,
        );
        self.set_message(
            "Write the comment · Esc then :w saves · :wq saves and closes · :q! discards",
        );
    }

    /// Opens (or focuses) a composer below the current pane.
    pub fn open_composer(&mut self, note: NoteRef, text: &str, insert: bool) {
        if let Some(i) = self.buffers.iter().position(|b| {
            b.note.as_ref().is_some_and(|n| {
                n.review == note.review
                    && match (&n.kind, &note.kind) {
                        (NoteKind::Comment(x), NoteKind::Comment(y)) => x == y,
                        (NoteKind::Reply(x), NoteKind::Reply(y)) => x == y,
                        _ => false,
                    }
            })
        }) {
            let id = self.buffers[i].id;
            if let Some(w) = self.windows.iter().position(|w| w.buffer == id) {
                self.focus_window(w);
            } else {
                self.note_alternate_buffer();
                self.cur = i;
            }
            self.enter_normal();
            return;
        }
        let editing_pane = !self.active_file_tree()
            && !self.active_outline()
            && self
                .windows
                .get(self.active_window)
                .is_none_or(|w| w.terminal.is_none());
        if editing_pane && self.windows.len() < 32 {
            self.split_window(false, false);
        }
        let mut b = Buffer::empty();
        b.rope = ropey::Rope::from_str(text);
        b.note = Some(note);
        b.mark_saved();
        self.buffers.push(b);
        self.note_alternate_buffer();
        self.cur = self.buffers.len() - 1;
        let id = self.buffers[self.cur].id;
        self.touch_buffer_mru(id);
        self.invalidate_index_caches();
        self.set_cursor(0, 0);
        if insert {
            self.enter_insert();
        } else {
            self.enter_normal();
        }
    }

    /// `:w` / Ctrl-S in a composer.
    pub fn save_comment_buffer(&mut self) -> anyhow::Result<String> {
        let Some(note) = self.buf().note.clone() else {
            anyhow::bail!("not a comment");
        };
        let text = self.buf().rope.to_string().trim_end().to_string();
        anyhow::ensure!(
            !text.trim().is_empty(),
            "Empty comment not saved — type something, or :q! to discard"
        );
        let at = store::now();
        match note.kind.clone() {
            NoteKind::Draft(d) => {
                let id = store::new_id();
                let author = self.review_author();
                let c = Comment {
                    id,
                    seq: 0,
                    file: d.file.clone(),
                    start: d.start,
                    end: d.end,
                    whole_file: d.whole_file,
                    anchor: d.anchor.clone(),
                    before: d.before.clone(),
                    after: d.after.clone(),
                    text,
                    status: Status::Open,
                    state: AnchorState::Ok,
                    author,
                    created_at: at.clone(),
                    updated_at: at,
                    replies: Vec::new(),
                };
                let warnings = self.review_apply(&note.review, vec![Op::Create(Box::new(c))])?;
                let seq = self
                    .reviews
                    .view()
                    .and_then(|v| v.comment(id))
                    .map(|c| c.label())
                    .unwrap_or_else(|| "comment".into());
                let b = self.buf_mut();
                b.note = Some(NoteRef {
                    kind: NoteKind::Comment(id),
                    label: format!("Comment {seq} · {}", d.file.display()),
                    ..note
                });
                b.mark_saved();
                let mut msg = format!("Comment {seq} saved · :q closes · ,rl lists comments");
                if !warnings.is_empty() {
                    msg = format!("{msg} · {}", warnings.join("; "));
                }
                Ok(msg)
            }
            NoteKind::Comment(id) => {
                let warnings = self.review_apply(&note.review, vec![Op::Text { id, text, at }])?;
                anyhow::ensure!(
                    warnings.is_empty(),
                    "This comment was deleted in another session; copy your text before closing"
                );
                self.buf_mut().mark_saved();
                Ok("Comment saved".into())
            }
            NoteKind::Reply(id) => {
                let author = self.review_author();
                let reply = Reply {
                    id: store::new_id(),
                    author,
                    agent: false,
                    text,
                    created_at: at,
                    suggest_resolve: false,
                };
                let warnings = self.review_apply(&note.review, vec![Op::Reply { id, reply }])?;
                anyhow::ensure!(
                    warnings.is_empty(),
                    "The comment was deleted in another session; reply not saved"
                );
                self.buf_mut().mark_saved();
                self.close_composer(true);
                Ok("Reply saved".into())
            }
        }
    }

    /// Closes the current composer and its pane. Refuses to lose unsaved
    /// text unless `discard`.
    pub fn close_composer(&mut self, discard: bool) {
        let Some(note) = self.buf().note.clone() else {
            return;
        };
        if !discard && self.buf().is_modified() {
            self.set_message("Unsaved comment — :w saves, :q! discards");
            return;
        }
        crate::command::remove_current_buffer(self);
        if let Some(target) = note.return_to {
            if let Some(i) = self.buffers.iter().position(|b| b.id == target) {
                let shown_elsewhere = self.windows.iter().any(|w| w.buffer == target);
                if self.windows.is_empty() || !shown_elsewhere {
                    self.cur = i;
                } else if let Some(w) = self.windows.iter().position(|w| w.buffer == target) {
                    self.focus_window(w);
                }
            }
        }
        self.review_ui.focus = None;
        self.enter_normal();
    }

    pub fn edit_note(&mut self, id: u64) {
        let Some(c) = self.reviews.view().and_then(|v| v.comment(id)).cloned() else {
            self.set_message("That comment no longer exists");
            return;
        };
        let review = self.reviews.view().unwrap().id.clone();
        self.focus_comment_source(&c);
        let source = self.buf().id;
        self.open_composer(
            NoteRef {
                review,
                kind: NoteKind::Comment(id),
                label: format!("Comment {} · {}", c.label(), c.file.display()),
                return_to: Some(source),
            },
            &c.text,
            false,
        );
        self.set_message(format!(
            "Editing comment {} · i to edit · :w saves · :q closes",
            c.label()
        ));
    }

    pub fn reply_to(&mut self, id: u64) {
        let Some(c) = self.reviews.view().and_then(|v| v.comment(id)).cloned() else {
            self.set_message("That comment no longer exists");
            return;
        };
        let review = self.reviews.view().unwrap().id.clone();
        self.focus_comment_source(&c);
        let source = self.buf().id;
        self.open_composer(
            NoteRef {
                review,
                kind: NoteKind::Reply(id),
                label: format!("Reply to {} · {}", c.label(), first_line(&c.text)),
                return_to: Some(source),
            },
            "",
            true,
        );
        self.set_message(format!(
            "Replying to {}: {} · Esc then :w saves and closes · :q! discards",
            c.label(),
            first_line(&c.text)
        ));
    }

    /// Opens the comment's file at its lines so the composer sits under it.
    fn focus_comment_source(&mut self, c: &Comment) {
        let abs = self.reviews.abs(&c.file);
        let (s, e, _) = self.comment_pos(c);
        if self.buf().path.as_ref() != Some(&abs)
            && (abs.exists() || self.buffers.iter().any(|b| b.path.as_ref() == Some(&abs)))
        {
            if self.buf().note.is_some() {
                self.close_composer(false);
            }
            if self.open_file(abs.clone()).is_err() {
                return;
            }
        }
        if self.buf().path.as_ref() == Some(&abs) {
            let line = s.min(self.buf().line_count().saturating_sub(1));
            self.set_cursor(line, 0);
            self.review_ui.focus = (!c.whole_file).then_some((abs, s, e));
        }
    }

    // ----- comments at the cursor ----------------------------------------

    /// Comments of the active review touching the cursor line (or the file).
    pub fn comments_at_cursor(&self) -> Vec<Comment> {
        let Some(path) = self.buf().path.clone() else {
            return Vec::new();
        };
        let Some(rel) = self.reviews.rel(&path) else {
            return Vec::new();
        };
        let line = self.cursor().0;
        let Some(v) = self.reviews.view() else {
            return Vec::new();
        };
        let mut on_line: Vec<Comment> = v
            .comments
            .iter()
            .filter(|c| c.file == rel)
            .filter(|c| {
                let (s, e, _) = self.comment_pos(c);
                !c.whole_file && line >= s && line <= e
            })
            .cloned()
            .collect();
        if on_line.is_empty() {
            on_line = v
                .comments
                .iter()
                .filter(|c| c.file == rel && c.whole_file)
                .cloned()
                .collect();
        }
        on_line.sort_by_key(|c| (!c.is_open(), c.seq));
        on_line
    }

    fn comment_at_cursor(&mut self, verb: &str) -> Option<Comment> {
        if let Some(id) = self.buf().note_id() {
            return self.reviews.view().and_then(|v| v.comment(id)).cloned();
        }
        let found = self.comments_at_cursor().into_iter().next();
        if found.is_none() {
            self.set_message(format!(
                "No comment on this line to {verb} (]r jumps to the next one)"
            ));
        }
        found
    }

    pub fn reply_at_cursor(&mut self) {
        if let Some(c) = self.comment_at_cursor("reply to") {
            self.reply_to(c.id);
        }
    }

    pub fn resolve_at_cursor(&mut self) {
        if let Some(c) = self.comment_at_cursor("resolve") {
            self.set_comment_status(&[c.id], None);
        }
    }

    /// Toggles (or with `to`, sets) the status of `ids`; saved immediately.
    pub fn set_comment_status(&mut self, ids: &[u64], to: Option<Status>) {
        let Some(v) = self.reviews.view() else {
            return;
        };
        let at = store::now();
        let ops: Vec<Op> = ids
            .iter()
            .filter_map(|id| v.comment(*id))
            .map(|c| Op::Status {
                id: c.id,
                status: to.unwrap_or(if c.is_open() {
                    Status::Resolved
                } else {
                    Status::Open
                }),
                at: at.clone(),
            })
            .collect();
        let resolved = ops
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    Op::Status {
                        status: Status::Resolved,
                        ..
                    }
                )
            })
            .count();
        let n = ops.len();
        match self.reviews.apply_batch(ops) {
            Ok(w) => self.warn(
                if n == 1 {
                    if resolved == 1 {
                        "Comment resolved".to_string()
                    } else {
                        "Comment reopened".to_string()
                    }
                } else {
                    format!("{resolved} resolved, {} reopened", n - resolved)
                },
                w,
            ),
            Err(e) => self.set_message(format!("Could not save: {e}")),
        }
        self.review_ui.checked.clear();
    }

    /// `,rv`: the full thread(s) on the cursor line in a floating window.
    pub fn view_comments_at_cursor(&mut self) {
        let cs = self.comments_at_cursor();
        if cs.is_empty() {
            self.set_message("No comment on this line (]r jumps to the next one)");
            return;
        }
        let text = cs
            .iter()
            .map(|c| self.thread_text(c, false))
            .collect::<Vec<_>>()
            .join("\n\n────────\n\n");
        let title = if cs.len() == 1 {
            format!("Comment {}", cs[0].label())
        } else {
            format!("{} comments", cs.len())
        };
        self.open_text_float(&title, &text);
        self.set_message(",rr replies · ,rx resolves · ,rm moves · q closes");
    }

    /// The thread as text: header, body, replies and (optionally) code.
    pub fn thread_text(&self, c: &Comment, code: bool) -> String {
        let (s, e, state) = self.comment_pos(c);
        let mut out = format!(
            "{} · {} · {} · {}",
            c.label(),
            if c.is_open() { "open" } else { "resolved" },
            if c.author.is_empty() {
                "unknown"
            } else {
                &c.author
            },
            store::short_time(&c.created_at),
        );
        match state {
            AnchorState::Ok => {}
            AnchorState::Stale => {
                out.push_str("\n⚠ The code changed — check it, then ,rm re-anchors")
            }
            AnchorState::Missing => out.push_str("\n⚠ The file no longer exists"),
        }
        let scope = if c.whole_file {
            "whole file".to_string()
        } else if s == e {
            format!("line {}", s + 1)
        } else {
            format!("lines {}–{}", s + 1, e + 1)
        };
        out.push_str(&format!("\n{} · {scope}\n\n{}", c.file.display(), c.text));
        for r in &c.replies {
            out.push_str(&format!(
                "\n\n↳ {}{} · {}\n{}",
                r.author,
                if r.agent { " (agent)" } else { "" },
                store::short_time(&r.created_at),
                r.text
                    .lines()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
            if r.suggest_resolve && c.is_open() {
                out.push_str("\n  (suggests resolving — R / ,rx resolves)");
            }
        }
        if code && !c.whole_file {
            let numbered = |first: usize, text: &str| {
                text.lines()
                    .enumerate()
                    .map(|(i, l)| format!("{:>5} │ {l}", first + i + 1))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            out.push_str(&format!(
                "\n\nCode when commented:\n{}",
                numbered(c.start, &c.anchor)
            ));
            if state != AnchorState::Ok {
                let abs = self.reviews.abs(&c.file);
                let now = self.preview_source_lines(&abs);
                if !now.is_empty() {
                    let end = (e + 1).min(now.len());
                    let begin = s.min(end);
                    out.push_str(&format!(
                        "\n\nCode there now:\n{}",
                        numbered(begin, &now[begin..end].join("\n"))
                    ));
                }
            }
        }
        out
    }

    // ----- moving ------------------------------------------------------

    /// `,rm`: re-anchors the comment being moved (from the list's `m`) or the
    /// comment on the cursor line to the cursor line / Visual selection,
    /// capturing the current code.
    pub fn move_comment_here(&mut self) {
        let visual = self.visual_anchor.take();
        if matches!(self.mode, Mode::Visual(_)) {
            self.enter_normal();
        }
        let id = match self.review_ui.moving.take() {
            Some(id) => id,
            None => match self
                .comments_at_cursor()
                .into_iter()
                .find(|c| !c.whole_file)
            {
                Some(c) => c.id,
                None => {
                    self.set_message("No comment to move — press m on one in :comments first");
                    return;
                }
            },
        };
        let Some(path) = self.buf().path.clone() else {
            self.set_message("Open the source file to move the comment to");
            return;
        };
        let Some(file) = self.reviews.rel(&path) else {
            self.set_message("Comments must refer to files in this project");
            return;
        };
        let cursor = self.cursor().0;
        let (a, b) = visual
            .map(|v| (v.0.min(cursor), v.0.max(cursor)))
            .unwrap_or((cursor, cursor));
        let lines = lines_of(self.buf());
        let (anchor, before, after) = store::capture(&lines, a, b);
        let label = self
            .reviews
            .view()
            .and_then(|v| v.comment(id))
            .map(|c| c.label())
            .unwrap_or_default();
        let op = Op::Anchor {
            id,
            file,
            start: a,
            end: b,
            whole_file: false,
            anchor,
            before,
            after,
            at: store::now(),
        };
        self.review_ui.live.remove(&id);
        match self.reviews.apply_batch(vec![op]) {
            Ok(w) if w.is_empty() => self.set_message(format!(
                "Comment {label} now points at line{} {}",
                if a == b { "" } else { "s" },
                if a == b {
                    format!("{}", a + 1)
                } else {
                    format!("{}–{}", a + 1, b + 1)
                }
            )),
            Ok(w) => self.set_message(w.join("; ")),
            Err(e) => self.set_message(format!("Could not move the comment: {e}")),
        }
        self.review_ui.checked.clear();
        self.review_ui.live_gen += 1;
    }

    // ----- navigation ----------------------------------------------------

    /// `]r` / `[r`: next / previous open comment across the review.
    pub fn next_comment(&mut self, forward: bool) {
        let Some(v) = self.reviews.view() else {
            self.set_message("No review comments (,rc adds one)");
            return;
        };
        let mut all: Vec<(PathBuf, usize, String)> = v
            .comments
            .iter()
            .filter(|c| c.is_open())
            .map(|c| (c.file.clone(), self.comment_pos(c).0, c.label()))
            .collect();
        if all.is_empty() {
            self.set_message("No open comments in this review");
            return;
        }
        all.sort();
        let here = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| self.reviews.rel(p))
            .map(|r| (r, self.cursor().0));
        let pick = match &here {
            Some((f, l)) => {
                if forward {
                    all.iter().find(|(cf, cl, _)| (cf, *cl) > (f, *l))
                } else {
                    all.iter().rev().find(|(cf, cl, _)| (cf, *cl) < (f, *l))
                }
            }
            None => None,
        };
        let wrapped = pick.is_none();
        let (file, line, label) = pick
            .or(if forward { all.first() } else { all.last() })
            .cloned()
            .unwrap();
        let total = all.len();
        let index = all
            .iter()
            .position(|x| x.0 == file && x.1 == line)
            .unwrap_or(0)
            + 1;
        let abs = self.reviews.abs(&file);
        self.push_jump();
        if let Err(e) = self.open_file(abs) {
            self.set_message(e.to_string());
            return;
        }
        let line = line.min(self.buf().line_count().saturating_sub(1));
        self.set_cursor(line, 0);
        let preview = self
            .comments_at_cursor()
            .first()
            .map(|c| first_line(&c.text).to_string())
            .unwrap_or_default();
        self.set_message(format!(
            "{label} ({index}/{total}){} {preview}",
            if wrapped { " · wrapped ·" } else { " ·" }
        ));
    }

    // ----- live anchors --------------------------------------------------

    /// Shifts comments in buffer `bid` for a line-count change at `at`.
    pub fn review_shift(&mut self, bid: u64, at: usize, delta: i64) {
        let Some(rel) = self
            .buffers
            .iter()
            .find(|b| b.id == bid)
            .and_then(|b| b.path.as_ref())
            .and_then(|p| self.reviews.rel(p))
        else {
            return;
        };
        let Some(v) = self.reviews.view() else {
            return;
        };
        let shift = |x: usize| -> usize {
            if x > at {
                (x as i64 + delta).max(at as i64) as usize
            } else {
                x
            }
        };
        let mut changed = false;
        for c in v.comments.iter().filter(|c| c.file == rel && !c.whole_file) {
            let (s, e, st) = self
                .review_ui
                .live
                .get(&c.id)
                .copied()
                .unwrap_or((c.start, c.end, c.state));
            let (ns, ne) = (shift(s), shift(e));
            if (ns, ne) != (s, e) {
                self.review_ui.live.insert(c.id, (ns, ne.max(ns), st));
                changed = true;
            }
        }
        if changed {
            self.review_ui.live_gen += 1;
        }
    }

    /// Re-checks anchors in buffers whose text changed since the last key.
    /// Cheap when nothing changed; never writes to disk.
    pub fn review_verify(&mut self) {
        let Some(v) = self.reviews.view() else {
            return;
        };
        if v.comments.is_empty() {
            return;
        }
        let generation = self.reviews.generation;
        let insert = self.mode == Mode::Insert;
        let mut by_file: HashMap<&Path, Vec<&Comment>> = HashMap::new();
        for c in v.comments.iter().filter(|c| !c.whole_file) {
            by_file.entry(c.file.as_path()).or_default().push(c);
        }
        let mut updates: Vec<(u64, Option<Pos>)> = Vec::new();
        let mut seen = Vec::new();
        for b in &self.buffers {
            let Some(rel) = b.path.as_ref().and_then(|p| self.reviews.rel(p)) else {
                continue;
            };
            let Some(cs) = by_file.get(rel.as_path()) else {
                continue;
            };
            seen.push(b.id);
            let key = (b.edit_seq, insert, generation);
            if self.review_ui.checked.get(&b.id) == Some(&key) {
                continue;
            }
            let lines = lines_of(b);
            let idx = LineIndex::new(&lines);
            for c in cs {
                let (s, e, _) = self.comment_pos(c);
                let n = c.anchor.split('\n').count();
                let fast = s + n <= lines.len()
                    && c.anchor
                        .split('\n')
                        .enumerate()
                        .all(|(k, a)| lines[s + k] == a);
                let pos = if fast {
                    (s, s + n - 1, AnchorState::Ok)
                } else if insert {
                    (s, e, AnchorState::Stale)
                } else {
                    let mut probe = (*c).clone();
                    probe.start = s;
                    probe.end = e;
                    store::locate(&probe, &idx)
                };
                if pos == (c.start, c.end, c.state) {
                    updates.push((c.id, None));
                } else {
                    updates.push((c.id, Some(pos)));
                }
            }
        }
        let mut changed = false;
        for (id, pos) in updates {
            let old = match pos {
                Some(p) => self.review_ui.live.insert(id, p),
                None => self.review_ui.live.remove(&id),
            };
            changed |= old != pos;
        }
        for b in &self.buffers {
            if seen.contains(&b.id) {
                let key = (b.edit_seq, insert, generation);
                self.review_ui.checked.insert(b.id, key);
            }
        }
        if changed {
            self.review_ui.live_gen += 1;
        }
    }

    /// After a file is saved: its comments' live positions become the stored
    /// ones (written for other sessions and agents).
    pub fn review_after_save(&mut self, path: &Path) {
        let Some(rel) = self.reviews.rel(path) else {
            return;
        };
        let Some(v) = self.reviews.view() else {
            return;
        };
        let ops: Vec<Op> = v
            .comments
            .iter()
            .filter(|c| c.file == rel)
            .filter_map(|c| {
                let (s, e, st) = self.review_ui.live.get(&c.id).copied()?;
                ((s, e, st) != (c.start, c.end, c.state)).then_some(Op::Position {
                    id: c.id,
                    start: s,
                    end: e,
                    state: st,
                })
            })
            .collect();
        let ids: Vec<u64> = v
            .comments
            .iter()
            .filter(|c| c.file == rel)
            .map(|c| c.id)
            .collect();
        if !ops.is_empty() {
            if let Err(e) = self.reviews.apply_batch(ops) {
                self.set_message(format!("Comment positions not saved: {e}"));
                return;
            }
        }
        for id in ids {
            self.review_ui.live.remove(&id);
        }
        self.review_ui.checked.clear();
        self.review_ui.live_gen += 1;
    }

    /// Re-anchors every comment against current text: saved files and clean
    /// buffers update the store; edited buffers update the live positions.
    /// Files whose text and comments did not change since the last pass are
    /// skipped, so reopening the list on a large review stays cheap.
    pub fn review_refresh_all(&mut self) {
        let Some(v) = self.reviews.view() else {
            return;
        };
        let mut by_file: HashMap<PathBuf, Vec<Comment>> = HashMap::new();
        for c in &v.comments {
            by_file.entry(c.file.clone()).or_default().push(c.clone());
        }
        let mut ops = Vec::new();
        for (file, cs) in by_file {
            let abs = self.reviews.abs(&file);
            let buffer = self.buffers.iter().find(|b| b.path.as_ref() == Some(&abs));
            let dirty = buffer.is_some_and(|b| b.is_modified());
            let fingerprint = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for c in &cs {
                    (c.id, c.start, c.end, c.state, &c.anchor).hash(&mut h);
                }
                match buffer {
                    Some(b) => (b.id, b.edit_seq).hash(&mut h),
                    None => std::fs::metadata(&abs)
                        .ok()
                        .map(|m| (m.len(), m.modified().ok()))
                        .hash(&mut h),
                }
                h.finish()
            };
            if self.review_ui.refreshed.get(&file) == Some(&fingerprint) {
                continue;
            }
            self.review_ui.refreshed.insert(file.clone(), fingerprint);
            let lines = match buffer {
                Some(b) => Some(lines_of(b)),
                None => std::fs::read_to_string(&abs).ok().map(|t| text_lines(&t)),
            };
            let Some(lines) = lines else {
                for c in cs.iter().filter(|c| c.state != AnchorState::Missing) {
                    ops.push(Op::Position {
                        id: c.id,
                        start: c.start,
                        end: c.end,
                        state: AnchorState::Missing,
                    });
                }
                continue;
            };
            let idx = LineIndex::new(&lines);
            for c in &cs {
                let pos = if c.whole_file {
                    (c.start, c.end, AnchorState::Ok)
                } else {
                    let mut probe = c.clone();
                    let (s, e, _) = self.comment_pos(c);
                    probe.start = s;
                    probe.end = e;
                    store::locate(&probe, &idx)
                };
                if dirty {
                    if pos != (c.start, c.end, c.state) {
                        self.review_ui.live.insert(c.id, pos);
                    }
                } else {
                    self.review_ui.live.remove(&c.id);
                    if pos != (c.start, c.end, c.state) {
                        ops.push(Op::Position {
                            id: c.id,
                            start: pos.0,
                            end: pos.1,
                            state: pos.2,
                        });
                    }
                }
            }
        }
        if !ops.is_empty() {
            if let Err(e) = self.reviews.apply_batch(ops) {
                self.set_message(format!("Comment positions not saved: {e}"));
            }
        }
        self.review_ui.live_gen += 1;
    }

    /// A file was renamed in the tree: its comments follow.
    pub fn review_file_moved(&mut self, old: &Path, new: &Path) {
        let (Some(from), Some(to)) = (self.reviews.rel(old), self.reviews.rel(new)) else {
            return;
        };
        if self.reviews.view().is_some_and(|v| {
            v.comments
                .iter()
                .any(|c| c.file == from || c.file.starts_with(&from))
        }) {
            if let Err(e) = self.reviews.apply_batch(vec![Op::Rename { from, to }]) {
                self.set_message(format!("Comments not moved with the file: {e}"));
            }
        }
    }

    /// Picks up other sessions' changes; called from the job poller.
    pub fn poll_reviews(&mut self) -> bool {
        let changed = self.reviews.poll();
        if changed {
            self.review_ui.checked.clear();
            self.review_ui.refreshed.clear();
            if let Some(e) = self.reviews.active.as_ref().and_then(|a| a.error.clone()) {
                self.set_message(format!("Review: {e}"));
            }
            if self
                .results
                .as_ref()
                .is_some_and(|r| matches!(r.review, Some(ReviewList::Comments { .. })))
                && self.mode == Mode::Results
            {
                self.rebuild_comments_list();
            }
        }
        changed
    }

    // ----- gutter, virtual text and status ---------------------------------

    /// Per-line marks for `b`, cached until the review or the buffer changes.
    pub fn review_marks(&self, b: &Buffer) -> Option<Rc<LineMarks>> {
        let rel = b.path.as_ref().and_then(|p| self.reviews.rel(p))?;
        let v = self.reviews.view()?;
        let key = (self.reviews.generation, self.review_ui.live_gen, b.edit_seq);
        if let Some((g, l, e, m)) = self.review_ui.marks.borrow().get(&b.id) {
            if (*g, *l, *e) == key {
                return Some(m.clone());
            }
        }
        let mut marks = LineMarks::default();
        let mut starts: HashMap<usize, Vec<&Comment>> = HashMap::new();
        for c in v.comments.iter().filter(|c| c.file == rel) {
            let (s, e, state) = self.comment_pos(c);
            if c.is_open() {
                marks.open += 1;
            }
            let kind = if !c.is_open() {
                MarkKind::Resolved
            } else if state != AnchorState::Ok {
                MarkKind::Attention
            } else {
                MarkKind::Open
            };
            if c.whole_file {
                marks.file_comments += 1;
                if c.is_open() {
                    starts.entry(0).or_default().push(c);
                }
                continue;
            }
            for line in s..=e.max(s) {
                let glyph = match (line == s, kind) {
                    (true, MarkKind::Open) => '●',
                    (true, MarkKind::Attention) => '◌',
                    (true, MarkKind::Resolved) => '○',
                    (false, _) => '│',
                };
                let entry = marks.lines.entry(line).or_insert(LineMark {
                    glyph,
                    kind,
                    text: None,
                });
                if kind > entry.kind || (kind == entry.kind && line == s && entry.glyph == '│') {
                    entry.kind = kind;
                    entry.glyph = glyph;
                }
            }
            if c.is_open() {
                starts.entry(s).or_default().push(c);
            }
        }
        for (line, mut cs) in starts {
            cs.sort_by_key(|c| (c.whole_file, c.seq));
            let c = cs[0];
            let mut text = format!(
                "{}{} {}",
                if c.whole_file { "▤ " } else { "" },
                c.label(),
                first_line(&c.text)
            );
            if !c.replies.is_empty() {
                text.push_str(&format!("  ↩{}", c.replies.len()));
            }
            if c.agent_suggests_resolve() {
                text.push_str("  ✓?");
            }
            if cs.len() > 1 {
                text.push_str(&format!("  (+{} more)", cs.len() - 1));
            }
            let entry = marks.lines.entry(line).or_insert(LineMark {
                glyph: '●',
                kind: MarkKind::Open,
                text: None,
            });
            entry.text = Some(text);
        }
        let m = Rc::new(marks);
        self.review_ui
            .marks
            .borrow_mut()
            .insert(b.id, (key.0, key.1, key.2, m.clone()));
        Some(m)
    }

    /// Lines to tint while writing or viewing a comment.
    pub fn review_focus_covers(&self, b: &Buffer, line: usize) -> bool {
        self.review_ui
            .focus
            .as_ref()
            .is_some_and(|(p, s, e)| b.path.as_ref() == Some(p) && line >= *s && line <= *e)
    }

    /// Status line segment: open comments in this file and agent activity.
    pub fn review_status(&self, b: &Buffer) -> String {
        let mut out = String::new();
        if let Some(job) = &self.review_job {
            out.push_str(&format!("review agent {}s · ", job.elapsed_secs()));
        }
        if let Some(m) = self.review_marks(b) {
            if m.open > 0 {
                out.push_str(&format!("● {} · ", m.open));
            }
        }
        out
    }

    // ----- the comments list -------------------------------------------------

    /// `:comments` / `,rl`.
    pub fn comments_results(&mut self) {
        self.review_notices();
        self.review_refresh_all();
        self.rebuild_comments_list();
        if self.reviews.view().is_none() {
            self.set_message("No review yet — ,rc on a line starts one (:reviews lists reviews)");
        }
    }

    pub fn rebuild_comments_list(&mut self) {
        let filter = self.review_ui.filter;
        let (keep_cursor, keep_selected, confirm) = match &self.results {
            Some(r) if matches!(r.review, Some(ReviewList::Comments { .. })) => (
                r.entries.get(r.cursor).and_then(|e| e.note_id),
                r.selected
                    .iter()
                    .filter_map(|i| r.entries.get(*i).and_then(|e| e.note_id))
                    .collect::<Vec<_>>(),
                match &r.review {
                    Some(ReviewList::Comments { confirm_delete, .. }) => confirm_delete.clone(),
                    _ => None,
                },
            ),
            _ => (None, Vec::new(), None),
        };
        let mut entries = Vec::new();
        let (title, error) = match &self.reviews.active {
            Some(a) => {
                let v = &a.view;
                let mut cs: Vec<(&Comment, (usize, usize, AnchorState))> = v
                    .comments
                    .iter()
                    .map(|c| (c, self.comment_pos(c)))
                    .filter(|(c, p)| filter.keeps(c, p.2))
                    .collect();
                cs.sort_by(|a, b| (&a.0.file, a.1 .0, a.0.seq).cmp(&(&b.0.file, b.1 .0, b.0.seq)));
                for (c, (s, e, state)) in cs {
                    let badge = if !c.is_open() {
                        "✓"
                    } else {
                        match state {
                            AnchorState::Ok => "●",
                            AnchorState::Stale => "◌",
                            AnchorState::Missing => "✗",
                        }
                    };
                    let at = if c.whole_file {
                        format!("{}", c.file.display())
                    } else if s == e {
                        format!("{}:{}", c.file.display(), s + 1)
                    } else {
                        format!("{}:{}–{}", c.file.display(), s + 1, e + 1)
                    };
                    let mut text =
                        format!("{badge} {:<4} {at}  {}", c.label(), first_line(&c.text));
                    if !c.replies.is_empty() {
                        text.push_str(&format!("  ↩{}", c.replies.len()));
                    }
                    match state {
                        AnchorState::Stale if c.is_open() => text.push_str("  [code changed]"),
                        AnchorState::Missing => text.push_str("  [file missing]"),
                        _ => {}
                    }
                    if c.agent_suggests_resolve() {
                        text.push_str("  [agent: resolve?]");
                    }
                    let mut entry = Entry::location(self.reviews.abs(&c.file), s, 0, text);
                    entry.no_path_prefix = true;
                    entry.note_id = Some(c.id);
                    entry.detail = self.thread_text(c, true);
                    entries.push(entry);
                }
                let (open, resolved, attention) = v.counts();
                let mut t = format!("Review: {} — {open} open · {resolved} resolved", v.title);
                if attention > 0 {
                    t.push_str(&format!(" · {attention} need attention"));
                }
                if v.status == store::ReviewStatus::Closed {
                    t.push_str(" · closed");
                }
                t.push_str(&format!(" — showing {}", filter.label()));
                let err = a.error.clone().or_else(|| {
                    a.write_error
                        .as_ref()
                        .map(|e| format!("Not yet written (retrying): {e}"))
                });
                (t, err)
            }
            None => ("Review comments — no review yet".to_string(), None),
        };
        if entries.is_empty() {
            let hint = match (self.reviews.view().map(|v| v.comments.len()), filter) {
                (None, _) | (Some(0), _) => {
                    "No comments yet — ,rc on a line (or V-select lines) to add one".to_string()
                }
                (Some(n), f) => format!(
                    "No {} comments ({n} in total) — o shows {}",
                    f.label(),
                    f.next().label()
                ),
            };
            entries.push(Entry::text(hint));
        }
        let mut r = Results::new(title, entries);
        r.error = error;
        r.review = Some(ReviewList::Comments {
            filter,
            confirm_delete: confirm,
        });
        if let Some(id) = keep_cursor {
            if let Some(i) = r.entries.iter().position(|e| e.note_id == Some(id)) {
                r.cursor = i;
            }
        }
        for id in keep_selected {
            if let Some(i) = r.entries.iter().position(|e| e.note_id == Some(id)) {
                r.selected.insert(i);
            }
        }
        if let Some(old) = &self.results {
            if matches!(old.review, Some(ReviewList::Comments { .. })) && self.mode == Mode::Results
            {
                r.preview = old.preview;
                r.filter = old.filter.clone();
                r.apply_filter_keep_cursor();
            }
        }
        self.show_results(r);
    }

    /// Comment ids the list acts on: the selection, else the cursor entry.
    pub fn listed_comment_ids(&self) -> Vec<u64> {
        let Some(r) = &self.results else {
            return Vec::new();
        };
        r.entries
            .iter()
            .enumerate()
            .filter(|(i, _)| r.selected.contains(i) || (r.selected.is_empty() && *i == r.cursor))
            .filter_map(|(_, e)| e.note_id)
            .collect()
    }

    /// `d` in the list: asks before deleting.
    pub fn delete_selected_notes(&mut self) {
        let ids = self.listed_comment_ids();
        if ids.is_empty() {
            return;
        }
        if self
            .buffers
            .iter()
            .any(|b| b.note_id().is_some_and(|id| ids.contains(&id)) && b.is_modified())
        {
            self.set_message("Save or discard the open edit of that comment first");
            return;
        }
        if let Some(r) = &mut self.results {
            if let Some(ReviewList::Comments { confirm_delete, .. }) = &mut r.review {
                *confirm_delete = Some(ids);
            }
        }
    }

    pub fn confirm_delete_notes(&mut self, yes: bool) {
        let ids = match self.results.as_mut().and_then(|r| r.review.as_mut()) {
            Some(ReviewList::Comments { confirm_delete, .. }) => confirm_delete.take(),
            _ => None,
        };
        let Some(ids) = ids.filter(|_| yes) else {
            self.set_message("Nothing deleted");
            return;
        };
        let n = ids.len();
        let result = self
            .reviews
            .apply_batch(ids.iter().map(|id| Op::Delete { id: *id }).collect());
        self.buffers
            .retain(|b| !b.note_id().is_some_and(|id| ids.contains(&id)));
        if self.buffers.is_empty() {
            self.buffers.push(Buffer::empty());
        }
        self.cur = self.cur.min(self.buffers.len() - 1);
        let live: Vec<u64> = self.buffers.iter().map(|b| b.id).collect();
        self.windows.retain(|w| live.contains(&w.buffer));
        if self.windows.len() <= 1 {
            self.windows.clear();
            self.window_layout = None;
            self.active_window = 0;
        }
        self.invalidate_index_caches();
        if let Some(r) = &mut self.results {
            r.selected.clear();
        }
        self.rebuild_comments_list();
        match result {
            Ok(w) => self.warn(format!("Deleted {n} comment(s)"), w),
            Err(e) => self.set_message(format!("Could not delete: {e}")),
        }
    }

    /// `m` in the list: go to the comment's file and wait for `,rm`.
    pub fn begin_move_from_list(&mut self) {
        let Some(id) = self.listed_comment_ids().first().copied() else {
            return;
        };
        let Some(c) = self.reviews.view().and_then(|v| v.comment(id)).cloned() else {
            return;
        };
        if c.whole_file {
            self.set_message("File comments have no lines to move");
            return;
        }
        self.remember_results();
        self.enter_normal();
        self.focus_comment_source(&c);
        self.review_ui.moving = Some(id);
        self.set_message(format!(
            "Moving {} — put the cursor on (or V-select) the new lines, then ,rm",
            c.label()
        ));
    }

    /// `:commentswrite` / `,rw`: retries a failed write and picks up other
    /// sessions' changes now (both also happen automatically).
    pub fn review_sync(&mut self) {
        let Some(a) = self.reviews.active.as_mut() else {
            self.set_message("No active review");
            return;
        };
        let flushed = a.flush();
        a.reload_if_changed();
        self.reviews.generation += 1;
        self.review_ui.checked.clear();
        self.review_ui.refreshed.clear();
        match flushed {
            Ok(w) if w.is_empty() => self.set_message("Comments are saved and up to date"),
            Ok(w) => self.set_message(w.join("; ")),
            Err(e) => self.set_message(format!("Comments not written: {e}")),
        }
        if self
            .results
            .as_ref()
            .is_some_and(|r| matches!(r.review, Some(ReviewList::Comments { .. })))
            && self.mode == Mode::Results
        {
            self.rebuild_comments_list();
        }
    }

    /// Ctrl-S / `:w` on the current buffer.
    pub fn save_current(&mut self) -> anyhow::Result<()> {
        if self.buf().note.is_some() {
            let msg = self.save_comment_buffer()?;
            self.set_message(msg);
            return Ok(());
        }
        let task_path = self
            .buf()
            .path
            .clone()
            .filter(|p| crate::task_tracker::is_task_path(p));
        let _task_lock = if let Some(path) = &task_path {
            let dir = path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("invalid task path"))?;
            Some(crate::files::private_lock(dir, "tasks.lock")?)
        } else {
            None
        };
        if let Some(path) = task_path.as_deref() {
            self.validate_task_buffer(path, &self.buf().rope.to_string())?;
        }
        // BufWritePre runs the built-in on-save actions (trim / final
        // newline) and any matching user autocmds before the bytes hit
        // disk; BufWritePost fires once the write succeeds.
        self.fire_event(crate::events::Event::BufWritePre);
        if let Some(path) = task_path.as_deref() {
            self.validate_task_buffer(path, &self.buf().rope.to_string())?;
        }
        self.buf_mut().save()?;
        crate::undofile::save(&self.project_root, self.buf());
        self.notify_saved();
        if let Some(path) = self.buf().path.clone() {
            self.review_verify();
            self.review_after_save(&path);
        }
        self.fire_event(crate::events::Event::BufWritePost);
        Ok(())
    }
}
