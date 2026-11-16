//! Private, project-local review storage.
//!
//! A folder can hold any number of named reviews. Each review is one JSON
//! document under `<repo>/.vaayu/reviews/<id>.json`, so sessions working on
//! different reviews never contend, and a large review only rewrites its own
//! file. `<repo>` is the enclosing Git work tree (or the project folder when
//! there is none), which keeps comment paths stable across `:cd` and matches
//! the paths GitHub uses.
//!
//! Every change is recorded as an [`Op`] and written through immediately.
//! A write takes the review's OS lock, re-reads the file, replays this
//! session's pending operations onto whatever is there now, and replaces it
//! atomically -- so two editors (or an editor and an agent run) adding,
//! editing and resolving comments in the same review merge instead of
//! refusing or overwriting each other. Comment identity is a random 64-bit
//! id; the short `#N` shown to people is a per-review sequence number that is
//! assigned under the lock and never reused.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

pub const FORMAT_VERSION: u32 = 2;
/// Lines of surrounding code captured on each side of an anchor, used to
/// tell identical lines apart when a file changes.
pub const CONTEXT_LINES: usize = 2;

mod hexid {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{v:016x}"))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(u64),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Num(n) => Ok(n),
            Raw::Str(s) => {
                u64::from_str_radix(s.trim_start_matches('c'), 16).map_err(serde::de::Error::custom)
            }
        }
    }
}

/// A random non-zero id. Collisions across sessions are what the random
/// space avoids; display numbers come from the review's sequence instead.
pub fn new_id() -> u64 {
    let mut b = [0u8; 8];
    if getrandom::fill(&mut b).is_err() {
        let t = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        b = (t ^ ((std::process::id() as u64) << 32)).to_le_bytes();
    }
    (u64::from_le_bytes(b) >> 1).max(1)
}

pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// `2026-11-16T10:04:05+05:30` → `2026-11-16 10:04`.
pub fn short_time(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| ts.to_string())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Open,
    Resolved,
}

/// Whether a comment's anchor still matches the code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorState {
    /// The commented lines are where the comment says.
    #[default]
    Ok,
    /// The commented code changed or became ambiguous: a person should check.
    Stale,
    /// The file no longer exists.
    Missing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    #[serde(with = "hexid")]
    pub id: u64,
    #[serde(default)]
    pub author: String,
    /// Written by an agent review run rather than a person.
    #[serde(default)]
    pub agent: bool,
    pub text: String,
    #[serde(default)]
    pub created_at: String,
    /// The agent believes the comment is addressed. Never applied on its own.
    #[serde(default)]
    pub suggest_resolve: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    #[serde(with = "hexid")]
    pub id: u64,
    /// The `#N` people see; unique within a review and never reused.
    #[serde(default)]
    pub seq: u64,
    /// Relative to the store root.
    pub file: PathBuf,
    /// 0-based, inclusive.
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub whole_file: bool,
    /// The commented lines as they were when the comment was placed.
    #[serde(default)]
    pub anchor: String,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    pub text: String,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub state: AnchorState,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
}

impl Comment {
    pub fn is_open(&self) -> bool {
        self.status == Status::Open
    }
    pub fn needs_attention(&self) -> bool {
        self.is_open() && self.state != AnchorState::Ok
    }
    pub fn agent_suggests_resolve(&self) -> bool {
        self.is_open()
            && self
                .replies
                .iter()
                .rev()
                .find(|r| r.agent)
                .is_some_and(|r| r.suggest_resolve)
    }
    pub fn label(&self) -> String {
        if self.seq > 0 {
            format!("#{}", self.seq)
        } else {
            "#new".into()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewStatus {
    #[default]
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub version: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub status: ReviewStatus,
    /// The Git revision the review compares against, when it reviews a range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default = "one")]
    pub next_seq: u64,
    #[serde(default)]
    pub comments: Vec<Comment>,
}
fn one() -> u64 {
    1
}

impl Review {
    pub fn new(title: &str, base: Option<String>) -> Self {
        let t = now();
        Self {
            version: FORMAT_VERSION,
            id: format!("r{:012x}", new_id() >> 16),
            title: title.to_string(),
            created_at: t.clone(),
            updated_at: t,
            status: ReviewStatus::Open,
            base,
            next_seq: 1,
            comments: Vec::new(),
        }
    }
    pub fn comment(&self, id: u64) -> Option<&Comment> {
        self.comments.iter().find(|c| c.id == id)
    }
    fn comment_mut(&mut self, id: u64) -> Option<&mut Comment> {
        self.comments.iter_mut().find(|c| c.id == id)
    }
    pub fn counts(&self) -> (usize, usize, usize) {
        let open = self.comments.iter().filter(|c| c.is_open()).count();
        let attention = self.comments.iter().filter(|c| c.needs_attention()).count();
        (open, self.comments.len() - open, attention)
    }
}

/// One recorded change. Pending operations are replayed onto the newest
/// copy on disk at every write, which is what makes concurrent sessions
/// merge: each operation names exactly the fields it changes.
#[derive(Clone, Debug)]
pub enum Op {
    Create(Box<Comment>),
    Text {
        id: u64,
        text: String,
        at: String,
    },
    Status {
        id: u64,
        status: Status,
        at: String,
    },
    /// Re-anchor (`m`/`,rm`): new lines and a fresh code snapshot.
    Anchor {
        id: u64,
        file: PathBuf,
        start: usize,
        end: usize,
        whole_file: bool,
        anchor: String,
        before: Vec<String>,
        after: Vec<String>,
        at: String,
    },
    /// Where the anchor was found after the file changed.
    Position {
        id: u64,
        start: usize,
        end: usize,
        state: AnchorState,
    },
    /// A file was renamed or moved.
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Delete {
        id: u64,
    },
    Reply {
        id: u64,
        reply: Reply,
    },
    Title(String),
    ReviewStatus(ReviewStatus),
    Base(Option<String>),
}

impl Op {
    /// Applies the operation. `Err` means it no longer makes sense (its
    /// comment was deleted elsewhere) and is dropped with a warning.
    pub fn apply(&self, r: &mut Review) -> Result<(), String> {
        let missing = |id: &u64| format!("comment {id:016x} was deleted in another session");
        match self {
            Op::Create(c) => {
                if r.comment(c.id).is_none() {
                    let mut c = (**c).clone();
                    c.seq = r.next_seq;
                    r.next_seq += 1;
                    r.comments.push(c);
                }
            }
            Op::Text { id, text, at } => {
                let c = r.comment_mut(*id).ok_or_else(|| missing(id))?;
                c.text = text.clone();
                c.updated_at = at.clone();
            }
            Op::Status { id, status, at } => {
                let c = r.comment_mut(*id).ok_or_else(|| missing(id))?;
                c.status = *status;
                c.updated_at = at.clone();
            }
            Op::Anchor {
                id,
                file,
                start,
                end,
                whole_file,
                anchor,
                before,
                after,
                at,
            } => {
                let c = r.comment_mut(*id).ok_or_else(|| missing(id))?;
                c.file = file.clone();
                c.start = *start;
                c.end = *end;
                c.whole_file = *whole_file;
                c.anchor = anchor.clone();
                c.before = before.clone();
                c.after = after.clone();
                c.state = AnchorState::Ok;
                c.updated_at = at.clone();
            }
            Op::Position {
                id,
                start,
                end,
                state,
            } => {
                // Positions are derived data: a comment deleted elsewhere
                // just has nothing to move.
                if let Some(c) = r.comment_mut(*id) {
                    c.start = *start;
                    c.end = *end;
                    c.state = *state;
                }
            }
            Op::Rename { from, to } => {
                for c in &mut r.comments {
                    if c.file == *from {
                        c.file = to.clone();
                    } else if let Ok(rest) = c.file.strip_prefix(from) {
                        c.file = to.join(rest);
                    }
                }
            }
            Op::Delete { id } => r.comments.retain(|c| c.id != *id),
            Op::Reply { id, reply } => {
                let c = r.comment_mut(*id).ok_or_else(|| missing(id))?;
                if !c.replies.iter().any(|x| x.id == reply.id) {
                    c.replies.push(reply.clone());
                    c.updated_at = reply.created_at.clone();
                }
            }

            Op::Title(t) => r.title = t.clone(),
            Op::ReviewStatus(s) => r.status = *s,
            Op::Base(b) => r.base = b.clone(),
        }
        r.updated_at = now();
        Ok(())
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
}

fn read_review(path: &Path) -> Result<Option<Review>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let r: Review = serde_json::from_str(&s).map_err(|e| e.to_string())?;
            if r.version > FORMAT_VERSION {
                return Err(format!(
                    "review format {} is newer than this Vaayu supports",
                    r.version
                ));
            }
            Ok(Some(r))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// One open review: what is on disk, what this session has changed but not
/// yet written, and the merged view the UI reads.
pub struct ReviewDoc {
    pub path: PathBuf,
    pub view: Review,
    pending: Vec<Op>,
    /// The review existed on disk at some point (so a missing file means it
    /// was deleted, not that it is new).
    persisted: bool,
    disk_stamp: Option<(SystemTime, u64)>,
    /// Set when the file could not be read; the review is then read-only so
    /// a damaged file is never overwritten.
    pub error: Option<String>,
    /// The last write failure, kept until a later write succeeds.
    pub write_error: Option<String>,
}

impl ReviewDoc {
    /// Opens a review file that should exist (errors are kept in `error`).
    pub fn open_existing(path: PathBuf) -> Self {
        Self::open(path)
    }
    fn open(path: PathBuf) -> Self {
        let (view, error, persisted) = match read_review(&path) {
            Ok(Some(r)) => (r, None, true),
            Ok(None) => (
                Review::new("Review", None),
                Some("review file is missing".into()),
                false,
            ),
            Err(e) => (Review::new("Review", None), Some(e), false),
        };
        let disk_stamp = stamp(&path);
        Self {
            path,
            view,
            pending: Vec::new(),
            persisted,
            disk_stamp,
            error,
            write_error: None,
        }
    }
    fn fresh(dir: &Path, review: Review) -> Self {
        Self {
            path: dir.join(format!("{}.json", review.id)),
            view: review,
            pending: Vec::new(),
            persisted: false,
            disk_stamp: None,
            error: None,
            write_error: None,
        }
    }
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    /// Records several ops and writes them in one transaction. Ops that no
    /// longer apply locally are skipped with a warning.
    pub fn apply_batch(&mut self, ops: Vec<Op>) -> anyhow::Result<Vec<String>> {
        if let Some(e) = &self.error {
            anyhow::bail!("review is read-only: {e}");
        }
        let mut warnings = Vec::new();
        for op in ops {
            match op.apply(&mut self.view) {
                Ok(()) => self.pending.push(op),
                Err(w) => warnings.push(w),
            }
        }
        warnings.extend(self.flush()?);
        Ok(warnings)
    }
    /// Replays pending operations onto the newest file under its lock.
    /// Returns warnings for operations that could no longer apply.
    pub fn flush(&mut self) -> anyhow::Result<Vec<String>> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let result = self.flush_inner();
        match &result {
            Ok(_) => self.write_error = None,
            Err(e) => self.write_error = Some(e.to_string()),
        }
        result
    }
    fn flush_inner(&mut self) -> anyhow::Result<Vec<String>> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid review path"))?
            .to_path_buf();
        let _lock = crate::files::private_lock(&dir, &format!("{}.lock", self.view.id))?;
        ensure_private_dir(&dir)?;
        if let Ok(meta) = std::fs::symlink_metadata(&self.path) {
            anyhow::ensure!(
                !meta.file_type().is_symlink(),
                "review files must not be symlinks"
            );
        }
        let mut base = match read_review(&self.path).map_err(|e| anyhow::anyhow!(e))? {
            Some(r) => r,
            None if self.persisted => {
                anyhow::bail!(
                    "this review was deleted in another session (:reviews to pick another)"
                )
            }
            None => {
                let mut r = self.view.clone();
                r.comments.clear();
                r.next_seq = 1;
                r
            }
        };
        let mut warnings = Vec::new();
        for op in &self.pending {
            if let Err(w) = op.apply(&mut base) {
                warnings.push(w);
            }
        }
        base.version = FORMAT_VERSION;
        let text = serde_json::to_string_pretty(&base)?;
        crate::files::atomic_write(&self.path, text.as_bytes(), true)?;
        self.pending.clear();
        self.persisted = true;
        self.disk_stamp = stamp(&self.path);
        self.view = base;
        Ok(warnings)
    }
    /// Picks up writes from other sessions. Returns whether the view changed.
    pub fn reload_if_changed(&mut self) -> bool {
        let now = stamp(&self.path);
        if now == self.disk_stamp {
            return false;
        }
        self.disk_stamp = now;
        match read_review(&self.path) {
            Ok(Some(mut r)) => {
                for op in &self.pending {
                    let _ = op.apply(&mut r);
                }
                let changed = r != self.view;
                self.view = r;
                self.persisted = true;
                if self.error.take().is_some() {
                    return true;
                }
                changed
            }
            Ok(None) => {
                if self.persisted {
                    self.error = Some("this review was deleted in another session".into());
                    return true;
                }
                false
            }
            Err(e) => {
                self.error = Some(e);
                true
            }
        }
    }
}

pub fn ensure_private_dir(dir: &Path) -> anyhow::Result<()> {
    if !dir.exists() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(dir)?;
    }
    anyhow::ensure!(
        !std::fs::symlink_metadata(dir)?.file_type().is_symlink(),
        "private directory must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The enclosing Git work tree, or `project` itself.
pub fn store_root(project: &Path) -> PathBuf {
    let mut p = Some(project);
    while let Some(dir) = p {
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        p = dir.parent();
    }
    project.to_path_buf()
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub status: ReviewStatus,
    pub base: Option<String>,
    pub open: usize,
    pub resolved: usize,
    pub attention: usize,
    pub updated_at: String,
    pub error: Option<String>,
}

/// Every review in one store, and which one this session works in.
pub struct Reviews {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub active: Option<ReviewDoc>,
    /// Bumped on every visible change, so caches keyed on it refresh.
    pub generation: u64,
    last_poll: Instant,
    /// Messages from loading (migration, unreadable files) for the UI.
    pub notices: Vec<String>,
}

impl Reviews {
    pub fn load(project: &Path) -> Self {
        let root = store_root(project);
        let dir = root.join(".vaayu/reviews");
        let mut out = Self {
            root,
            dir,
            active: None,
            generation: 1,
            last_poll: Instant::now(),
            notices: Vec::new(),
        };
        for legacy in [
            project.join(".vaayu/comments.json"),
            out.root.join(".vaayu/comments.json"),
        ] {
            match out.migrate_legacy(&legacy, project) {
                Ok(Some(n)) => out.notices.push(format!(
                    "Moved {n} private comment(s) into the review \"Imported comments\""
                )),
                Ok(None) => {}
                Err(e) => out
                    .notices
                    .push(format!("Could not import {}: {e}", legacy.display())),
            }
        }
        out.adopt_existing();
        out
    }

    /// Makes the remembered review (or the most recently updated open one)
    /// active. Returns whether one was found.
    fn adopt_existing(&mut self) -> bool {
        let preferred = std::fs::read_to_string(self.dir.join("active"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|id| self.dir.join(format!("{id}.json")).exists());
        let id = preferred.or_else(|| {
            self.list()
                .into_iter()
                .filter(|s| s.error.is_none())
                .max_by_key(|s| (s.status == ReviewStatus::Open, s.updated_at.clone()))
                .map(|s| s.id)
        });
        match id {
            Some(id) => {
                self.active = Some(ReviewDoc::open(self.dir.join(format!("{id}.json"))));
                self.generation += 1;
                true
            }
            None => false,
        }
    }

    /// Imports the version-1 `comments.json` once, as its own review, and
    /// renames the old file so it is never imported twice.
    fn migrate_legacy(&mut self, legacy: &Path, project: &Path) -> anyhow::Result<Option<usize>> {
        if !legacy.is_file() {
            return Ok(None);
        }
        let legacy_dir = legacy.parent().unwrap();
        let _lock = crate::files::private_lock(legacy_dir, "comments.lock")?;
        if !legacy.is_file() {
            return Ok(None);
        }
        #[derive(Deserialize)]
        struct V1 {
            version: u32,
            notes: Vec<V1Note>,
        }
        #[derive(Deserialize)]
        struct V1Note {
            file: PathBuf,
            start: usize,
            end: usize,
            #[serde(default)]
            whole_file: bool,
            #[serde(default)]
            anchor: String,
            text: String,
            #[serde(default)]
            stale: bool,
            #[serde(default)]
            resolved: bool,
        }
        let doc: V1 = serde_json::from_str(&std::fs::read_to_string(legacy)?)?;
        anyhow::ensure!(doc.version == 1, "unsupported comments version");
        let base = legacy_dir.parent().unwrap_or(project);
        let mut review = Review::new("Imported comments", None);
        let t = now();
        for n in doc.notes.into_iter().filter(|n| !n.text.trim().is_empty()) {
            let abs = base.join(&n.file);
            let file = abs
                .strip_prefix(&self.root)
                .map(Path::to_path_buf)
                .unwrap_or(n.file);
            review.comments.push(Comment {
                id: new_id(),
                seq: review.next_seq,
                file,
                start: n.start,
                end: n.end,
                whole_file: n.whole_file,
                anchor: n.anchor,
                before: Vec::new(),
                after: Vec::new(),
                text: n.text,
                status: if n.resolved {
                    Status::Resolved
                } else {
                    Status::Open
                },
                state: if n.stale {
                    AnchorState::Stale
                } else {
                    AnchorState::Ok
                },
                author: String::new(),
                created_at: t.clone(),
                updated_at: t.clone(),
                replies: Vec::new(),
            });
            review.next_seq += 1;
        }
        let n = review.comments.len();
        if n > 0 {
            ensure_private_dir(&self.dir)?;
            crate::files::atomic_write(
                &self.dir.join(format!("{}.json", review.id)),
                serde_json::to_string_pretty(&review)?.as_bytes(),
                true,
            )?;
        }
        std::fs::rename(legacy, legacy.with_extension("json.v1-imported"))?;
        Ok((n > 0).then_some(n))
    }

    pub fn list(&self) -> Vec<Summary> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for f in dir.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            // The active review's view includes edits not yet on disk.
            let r = match self.active.as_ref().filter(|a| a.view.id == id) {
                Some(a) => Ok(Some(a.view.clone())),
                None => read_review(&f.path()),
            };
            match r {
                Ok(Some(r)) => {
                    let (open, resolved, attention) = r.counts();
                    out.push(Summary {
                        id: r.id,
                        title: r.title,
                        status: r.status,
                        base: r.base,
                        open,
                        resolved,
                        attention,
                        updated_at: r.updated_at,
                        error: None,
                    });
                }
                Ok(None) => {}
                Err(e) => out.push(Summary {
                    id: id.to_string(),
                    title: name.clone(),
                    status: ReviewStatus::Open,
                    base: None,
                    open: 0,
                    resolved: 0,
                    attention: 0,
                    updated_at: String::new(),
                    error: Some(e),
                }),
            }
        }
        out.sort_by(|a, b| {
            (a.status != ReviewStatus::Open)
                .cmp(&(b.status != ReviewStatus::Open))
                .then(b.updated_at.cmp(&a.updated_at))
        });
        out
    }

    fn remember_active(&self) {
        if let Some(a) = &self.active {
            if ensure_private_dir(&self.dir).is_ok() {
                let _ = crate::files::atomic_write(
                    &self.dir.join("active"),
                    a.view.id.as_bytes(),
                    true,
                );
            }
        }
    }

    pub fn create(&mut self, title: &str, base: Option<String>) -> anyhow::Result<String> {
        ensure_private_dir(&self.dir)?;
        crate::files::atomic_write(&self.dir.join(".gitignore"), b"*\n", true)?;
        let parent = self.dir.parent().unwrap();
        if !parent.join(".gitignore").exists() {
            crate::files::atomic_write(&parent.join(".gitignore"), b"*\n", true)?;
        }
        let review = Review::new(title, base);
        let id = review.id.clone();
        let mut doc = ReviewDoc::fresh(&self.dir, review);
        doc.pending.push(Op::Title(title.to_string()));
        doc.flush()?;
        self.active = Some(doc);
        self.remember_active();
        self.generation += 1;
        Ok(id)
    }

    pub fn switch(&mut self, id: &str) -> anyhow::Result<()> {
        let path = self.dir.join(format!("{id}.json"));
        anyhow::ensure!(path.is_file(), "no review {id}");
        if let Some(a) = &mut self.active {
            if a.view.id == id {
                return Ok(());
            }
            a.flush()?;
        }
        let doc = ReviewDoc::open(path);
        if let Some(e) = &doc.error {
            anyhow::bail!("cannot open review: {e}");
        }
        self.active = Some(doc);
        self.remember_active();
        self.generation += 1;
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> anyhow::Result<()> {
        let path = self.dir.join(format!("{id}.json"));
        {
            let _lock = crate::files::private_lock(&self.dir, &format!("{id}.lock"))?;
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.lock")));
        let _ = std::fs::remove_dir_all(self.dir.join("runs").join(id));
        if self.active.as_ref().is_some_and(|a| a.view.id == id) {
            self.active = None;
            let _ = std::fs::remove_file(self.dir.join("active"));
        }
        self.generation += 1;
        Ok(())
    }

    /// The active review, creating one titled `title` when there is none.
    pub fn ensure_active(&mut self, title: &str) -> anyhow::Result<&mut ReviewDoc> {
        // Another session may have started one since this one loaded:
        // join it rather than starting a parallel review.
        if self.active.is_none() && !self.adopt_existing() {
            self.create(title, None)?;
        }
        Ok(self.active.as_mut().unwrap())
    }

    pub fn apply_batch(&mut self, ops: Vec<Op>) -> anyhow::Result<Vec<String>> {
        let a = self
            .active
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no active review (:reviewnew creates one)"))?;
        let r = a.apply_batch(ops);
        self.generation += 1;
        r
    }

    /// Cheap periodic check for other sessions' writes (a `stat`, at most
    /// twice a second). Also retries a write that failed earlier.
    pub fn poll(&mut self) -> bool {
        if self.last_poll.elapsed() < Duration::from_millis(500) {
            return false;
        }
        self.last_poll = Instant::now();
        let Some(a) = &mut self.active else {
            // No review yet: pick up one another session starts.
            return self.dir.join("active").exists() && self.adopt_existing();
        };
        let mut changed = false;
        if a.has_pending() && a.error.is_none() {
            changed |= a.flush().is_ok();
        }
        changed |= a.reload_if_changed();
        if changed {
            self.generation += 1;
        }
        changed
    }

    pub fn view(&self) -> Option<&Review> {
        self.active.as_ref().map(|a| &a.view)
    }

    pub fn abs(&self, file: &Path) -> PathBuf {
        self.root.join(file)
    }
    pub fn rel(&self, path: &Path) -> Option<PathBuf> {
        path.strip_prefix(&self.root).ok().map(Path::to_path_buf)
    }
}

/// Snapshot of the lines `start..=end` plus surrounding context.
pub fn capture(lines: &[String], start: usize, end: usize) -> (String, Vec<String>, Vec<String>) {
    let end = end.min(lines.len().saturating_sub(1));
    let start = start.min(end);
    let anchor = lines
        .get(start..=end)
        .map(|s| s.join("\n"))
        .unwrap_or_default();
    let before = lines[start.saturating_sub(CONTEXT_LINES)..start].to_vec();
    let after = lines
        .get(end + 1..(end + 1 + CONTEXT_LINES).min(lines.len()))
        .map(<[String]>::to_vec)
        .unwrap_or_default();
    (anchor, before, after)
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Line text index for finding anchors; built once per file and pass.
pub struct LineIndex<'a> {
    pub lines: &'a [String],
    exact: HashMap<&'a str, Vec<usize>>,
    normalized: std::cell::OnceCell<HashMap<String, Vec<usize>>>,
}

impl<'a> LineIndex<'a> {
    pub fn new(lines: &'a [String]) -> Self {
        let mut exact: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            exact.entry(l.as_str()).or_default().push(i);
        }
        Self {
            lines,
            exact,
            normalized: std::cell::OnceCell::new(),
        }
    }
    fn normalized(&self) -> &HashMap<String, Vec<usize>> {
        self.normalized.get_or_init(|| {
            let mut m: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, l) in self.lines.iter().enumerate() {
                m.entry(norm(l)).or_default().push(i);
            }
            m
        })
    }
}

fn block_matches(lines: &[String], at: usize, anchor: &[&str], fuzzy: bool) -> bool {
    at + anchor.len() <= lines.len()
        && anchor.iter().enumerate().all(|(k, a)| {
            if fuzzy {
                norm(&lines[at + k]) == norm(a)
            } else {
                lines[at + k] == *a
            }
        })
}

fn context_score(lines: &[String], start: usize, n: usize, c: &Comment) -> usize {
    let mut score = 0;
    for (k, b) in c.before.iter().rev().enumerate() {
        if start > k && norm(&lines[start - 1 - k]) == norm(b) {
            score += 1;
        } else {
            break;
        }
    }
    for (k, a) in c.after.iter().enumerate() {
        if lines.get(start + n + k).is_some_and(|l| norm(l) == norm(a)) {
            score += 1;
        } else {
            break;
        }
    }
    score
}

/// Finds where `c`'s anchor is now: `(start, end, state)`.
///
/// The fast path keeps the comment where it is when the anchor and its
/// context still match there (live edits already shifted it). Otherwise
/// every occurrence is scored by how much surrounding context still
/// matches; a unique best match moves the comment, an unresolvable tie or a
/// whitespace-only match without context keeps the nearest candidate but
/// marks it stale for a person to check.
pub fn locate(c: &Comment, idx: &LineIndex) -> (usize, usize, AnchorState) {
    let lines = idx.lines;
    if c.whole_file || c.anchor.is_empty() {
        return (c.start, c.end, AnchorState::Ok);
    }
    let anchor: Vec<&str> = c.anchor.split('\n').collect();
    let n = anchor.len();
    let full_ctx = c.before.len() + c.after.len();
    let clamp = |s: usize| {
        let s = s.min(lines.len().saturating_sub(1));
        (s, (s + n - 1).min(lines.len().saturating_sub(1)))
    };
    if block_matches(lines, c.start, &anchor, false)
        && context_score(lines, c.start, n, c) == full_ctx
    {
        return (c.start, c.start + n - 1, AnchorState::Ok);
    }
    for fuzzy in [false, true] {
        let first = if fuzzy {
            idx.normalized().get(&norm(anchor[0]))
        } else {
            idx.exact.get(anchor[0])
        };
        let candidates: Vec<usize> = first
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| block_matches(lines, i, &anchor, fuzzy))
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let scored: Vec<(usize, usize)> = candidates
            .iter()
            .map(|&i| (i, context_score(lines, i, n, c)))
            .collect();
        let best = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);
        let top: Vec<usize> = scored
            .iter()
            .filter(|(_, s)| *s == best)
            .map(|(i, _)| *i)
            .collect();
        let pick = *top.iter().min_by_key(|&&i| i.abs_diff(c.start)).unwrap();
        // Certain: one best candidate whose surroundings agree (or there
        // is no context to compare). A unique match whose context all
        // differs is more likely an unrelated identical line, so the
        // comment stays put and is flagged instead of jumping there.
        let certain = top.len() == 1
            && if fuzzy {
                full_ctx > 0 && best == full_ctx
            } else {
                full_ctx == 0 || best > 0
            };
        if certain {
            return (pick, pick + n - 1, AnchorState::Ok);
        }
        if best == 0 && full_ctx > 0 {
            let (s, e) = clamp(c.start);
            return (s, e, AnchorState::Stale);
        }
        return (pick, pick + n - 1, AnchorState::Stale);
    }
    let (s, e) = clamp(c.start);
    (s, e, AnchorState::Stale)
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Reviews {
        fn apply(&mut self, op: Op) -> anyhow::Result<Vec<String>> {
            self.apply_batch(vec![op])
        }
    }

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }
    fn comment_at(text: &[String], start: usize, end: usize) -> Comment {
        let (anchor, before, after) = capture(text, start, end);
        Comment {
            id: new_id(),
            seq: 1,
            file: "a".into(),
            start,
            end,
            whole_file: false,
            anchor,
            before,
            after,
            text: "x".into(),
            status: Status::Open,
            state: AnchorState::Ok,
            author: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            replies: Vec::new(),
        }
    }
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("vaayu-rs-{}-{}", std::process::id(), new_id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn locate_follows_moves_and_uses_context_for_duplicates() {
        let v1 = lines("fn a() {\n    1\n}\nfn b() {\n    2\n}\n");
        let c = comment_at(&v1, 5, 5); // the second `}`
        let v2 = lines("// header\nfn a() {\n    1\n}\nfn b() {\n    2\n}\n");
        // Stored position (5) is now the *first* `}`... no: line 5 is `    2`.
        let idx = LineIndex::new(&v2);
        assert_eq!(locate(&c, &idx), (6, 6, AnchorState::Ok));
        // Identical lines with identical context are ambiguous: stale.
        let v3 = lines("x\n}\ny\nx\n}\ny\n");
        let c = comment_at(&v3, 4, 4);
        let v4 = lines("top\nx\n}\ny\nx\n}\ny\n");
        let mut moved = c.clone();
        moved.before.clear();
        moved.after.clear();
        assert_eq!(locate(&moved, &LineIndex::new(&v4)).2, AnchorState::Stale);
        // With context the right one wins even if the stored line is wrong.
        let v5 = lines("a\n}\nb\nc\n}\nd\n");
        let c = comment_at(&v5, 4, 4);
        let v6 = lines("new\na\n}\nb\nc\n}\nd\n");
        assert_eq!(locate(&c, &LineIndex::new(&v6)), (5, 5, AnchorState::Ok));
    }

    #[test]
    fn locate_marks_changed_code_stale_and_whitespace_changes_with_context_ok() {
        let v1 = lines("one\ntwo\nthree\nfour\n");
        let c = comment_at(&v1, 1, 2);
        let gone = lines("one\nTWO\nthree\nfour\n");
        assert_eq!(locate(&c, &LineIndex::new(&gone)).2, AnchorState::Stale);
        let reindented = lines("one\n  two\n  three\nfour\n");
        assert_eq!(
            locate(&c, &LineIndex::new(&reindented)),
            (1, 2, AnchorState::Ok)
        );
    }

    #[test]
    fn concurrent_sessions_merge_and_never_reuse_numbers() {
        let dir = temp();
        let mut a = Reviews::load(&dir);
        a.create("R", None).unwrap();
        let id = a.active.as_ref().unwrap().view.id.clone();
        let mut b = Reviews::load(&dir);
        assert_eq!(b.active.as_ref().unwrap().view.id, id);
        let v = lines("x\ny\n");
        let mk = |t: &str| {
            let mut c = comment_at(&v, 0, 0);
            c.text = t.into();
            c
        };
        a.apply(Op::Create(Box::new(mk("from a")))).unwrap();
        b.apply(Op::Create(Box::new(mk("from b")))).unwrap();
        let doc = read_review(&b.active.as_ref().unwrap().path)
            .unwrap()
            .unwrap();
        assert_eq!(doc.comments.len(), 2);
        let seqs: Vec<u64> = doc.comments.iter().map(|c| c.seq).collect();
        assert_eq!(seqs, vec![1, 2]);
        // A deletes #2, then a new comment never reuses 2.
        let second = doc.comments[1].id;
        a.active.as_mut().unwrap().reload_if_changed();
        a.apply(Op::Delete { id: second }).unwrap();
        a.apply(Op::Create(Box::new(mk("third")))).unwrap();
        assert_eq!(a.view().unwrap().comments.last().unwrap().seq, 3);
        // B edits the deleted comment: dropped with a warning, nothing lost.
        let w = b
            .apply(Op::Text {
                id: second,
                text: "edit".into(),
                at: now(),
            })
            .unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(b.view().unwrap().comments.len(), 2);
        // B resolving A's comment while A replies keeps both changes.
        let first = doc.comments[0].id;
        b.apply(Op::Status {
            id: first,
            status: Status::Resolved,
            at: now(),
        })
        .unwrap();
        a.apply(Op::Reply {
            id: first,
            reply: Reply {
                id: new_id(),
                author: "a".into(),
                agent: false,
                text: "done?".into(),
                created_at: now(),
                suggest_resolve: false,
            },
        })
        .unwrap();
        let c = a.view().unwrap().comment(first).unwrap().clone();
        assert_eq!(c.status, Status::Resolved);
        assert_eq!(c.replies.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn several_reviews_per_folder_and_legacy_import() {
        let dir = temp();
        std::fs::create_dir_all(dir.join(".vaayu")).unwrap();
        std::fs::write(
            dir.join(".vaayu/comments.json"),
            r#"{"version":1,"notes":[{"id":1,"file":"a.rs","start":0,"end":0,"whole_file":false,"anchor":"x","text":"old","stale":false,"resolved":true},{"id":2,"file":"a.rs","start":0,"end":0,"whole_file":false,"anchor":"x","text":"","stale":false,"resolved":false}]}"#,
        )
        .unwrap();
        let mut r = Reviews::load(&dir);
        assert!(r.notices[0].contains("1 private comment"));
        assert!(dir.join(".vaayu/comments.json.v1-imported").exists());
        let imported = r.active.as_ref().unwrap().view.clone();
        assert_eq!(imported.comments[0].status, Status::Resolved);
        r.create("Second", Some("HEAD~1".into())).unwrap();
        r.create("Third", None).unwrap();
        let list = r.list();
        assert_eq!(list.len(), 3);
        // A new session starts in the last review someone switched to.
        r.switch(&imported.id).unwrap();
        let again = Reviews::load(&dir);
        assert_eq!(again.active.as_ref().unwrap().view.id, imported.id);
        assert!(again.notices.is_empty(), "imports only once");
        r.delete(&imported.id).unwrap();
        assert_eq!(r.list().len(), 2);
        assert!(r.active.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn damaged_review_is_read_only() {
        let dir = temp();
        let mut r = Reviews::load(&dir);
        let id = r.create("R", None).unwrap();
        std::fs::write(r.dir.join(format!("{id}.json")), "broken").unwrap();
        let mut again = Reviews::load(&dir);
        assert!(again.active.as_ref().unwrap().error.is_some());
        assert!(again.apply(Op::Title("x".into())).is_err());
        assert_eq!(
            std::fs::read_to_string(again.dir.join(format!("{id}.json"))).unwrap(),
            "broken"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
