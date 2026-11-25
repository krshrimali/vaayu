//! `:worktrees`: every Git worktree of this repository, with what each one
//! was made for, and Enter to review it.
//!
//! An agent that writes code in its own worktree (the `review-worktree`
//! Claude skill) records why next to it, at the main worktree's root:
//!
//! ```text
//! <main>/code-reviews/
//!     fix-parser/          <- `git worktree add`: the whole repository
//!     fix-parser.json      <- the record: title, description, base, ...
//! ```
//!
//! This file joins both sides:
//!
//! ```text
//! git worktree list --porcelain ---> Worktree (path, branch, HEAD)
//! code-reviews/*.json           ---> Record   (title, description, base)
//!                 \                    /
//!          joined on path, then branch, then slug
//!                          |
//!                          v
//!   Results list. Enter: `:cd` into the worktree, then `:reviewstart`
//!   (which reads the record's base, see `crate::review_toc`).
//! ```
//!
//! Example: Claude fixes a parser crash in `code-reviews/fix-parser` on
//! branch `fix-parser`, forked from `master` at `3f2a1c9`, and writes
//! `code-reviews/fix-parser.json` with `"base": "3f2a1c9..."`. `:worktrees`
//! lists it as `fix-parser  Fix the parser crash on empty input  +42 −3`;
//! Enter makes `code-reviews/fix-parser` the project and opens its review
//! against `3f2a1c9`, so the diff is exactly what Claude wrote, committed
//! or not.
//!
//! A worktree without a record still lists (branch and HEAD only); one with
//! `git branch --edit-description` text shows that as its description.
use crate::{
    editor::Editor,
    results::{Entry, Results},
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The folder at the main worktree's root that holds agent worktrees and
/// their records.
pub const RECORDS_DIR: &str = "code-reviews";

/// Records larger than this are skipped rather than parsed (a record is a
/// few hundred bytes; anything this big is not one).
const MAX_RECORD_BYTES: u64 = 1024 * 1024;

/// One entry of `git worktree list --porcelain`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// The short branch name (`refs/heads/` stripped); `None` when detached.
    pub branch: Option<String>,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// One revision note an agent appends after addressing a review.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Revision {
    pub at: String,
    pub summary: String,
}

/// `code-reviews/<slug>.json`, written by the agent that made the worktree.
/// Every field is optional so a hand-written or older record still loads;
/// see the skill's reference for what each one means.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Record {
    pub version: u32,
    pub slug: String,
    pub title: String,
    pub description: String,
    /// The user's request, close to verbatim.
    pub request: String,
    /// The worktree's path: relative to the main worktree's root, or
    /// absolute.
    pub worktree: String,
    pub branch: String,
    /// The commit the worktree branched from; the review compares with it.
    pub base: String,
    pub base_ref: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub agent: String,
    pub notes: String,
    pub test_command: String,
    /// Files to look at first, relative to the worktree.
    pub focus: Vec<String>,
    pub revisions: Vec<Revision>,
    /// Where this record was read from (not part of the JSON).
    #[serde(skip)]
    pub file: PathBuf,
}

impl Record {
    /// The worktree this record describes, as an absolute path.
    pub fn worktree_path(&self, main_root: &Path) -> Option<PathBuf> {
        if self.worktree.trim().is_empty() {
            return (!self.slug.is_empty()).then(|| main_root.join(RECORDS_DIR).join(&self.slug));
        }
        let p = Path::new(self.worktree.trim());
        Some(if p.is_absolute() {
            p.to_path_buf()
        } else {
            main_root.join(p)
        })
    }

    /// The name shown in lists: the slug, else the record file's stem.
    pub fn name(&self) -> String {
        if !self.slug.is_empty() {
            return self.slug.clone();
        }
        self.file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Parses `git worktree list --porcelain`: blocks separated by blank lines,
/// each starting with `worktree <path>`.
pub fn parse_worktree_list(text: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            out.push(Worktree {
                path: PathBuf::from(path),
                ..Default::default()
            });
            continue;
        }
        let Some(w) = out.last_mut() else {
            continue;
        };
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "HEAD" => w.head = Some(value.to_string()),
            "branch" => {
                w.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_string(),
                )
            }
            "bare" => w.bare = true,
            "locked" => w.locked = true,
            "prunable" => w.prunable = true,
            _ => {}
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .next()
            .unwrap_or("git failed")
            .to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Every worktree of the repository containing `dir`, main one first (Git
/// always lists it first).
pub fn list_worktrees(dir: &Path) -> Result<Vec<Worktree>, String> {
    git(dir, &["worktree", "list", "--porcelain"]).map(|t| parse_worktree_list(&t))
}

/// Reads every `*.json` record under `<main_root>/code-reviews/`. Files that
/// do not parse are reported by name in the second list instead of failing
/// the whole load.
pub fn load_records(main_root: &Path) -> (Vec<Record>, Vec<String>) {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let Ok(dir) = std::fs::read_dir(main_root.join(RECORDS_DIR)) else {
        return (records, errors);
    };
    for f in dir.flatten() {
        let path = f.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if f.metadata().is_ok_and(|m| m.len() > MAX_RECORD_BYTES) {
            errors.push(format!("{name}: larger than 1 MiB"));
            continue;
        }
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Record>(&t).map_err(|e| e.to_string()))
        {
            Ok(mut r) => {
                r.file = path;
                records.push(r);
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    records.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then(a.name().cmp(&b.name()))
    });
    (records, errors)
}

/// Whether `record` describes worktree `w`: by path, then branch, then a
/// slug that names the worktree's folder under `code-reviews/`.
fn describes(record: &Record, w: &Worktree, main_root: &Path) -> bool {
    let here = crate::files::identity(&w.path);
    if record
        .worktree_path(main_root)
        .is_some_and(|p| crate::files::identity(&p) == here)
    {
        return true;
    }
    if !record.branch.is_empty() && w.branch.as_deref() == Some(record.branch.as_str()) {
        return true;
    }
    !record.slug.is_empty()
        && here == crate::files::identity(&main_root.join(RECORDS_DIR).join(&record.slug))
}

/// The record describing the worktree whose root is `root`, if an agent
/// left one. Used by `:reviewstart` to pick the review's base.
pub fn record_for(root: &Path) -> Option<Record> {
    let worktrees = list_worktrees(root).ok()?;
    let main_root = worktrees.first()?.path.clone();
    let here = crate::files::identity(root);
    let w = worktrees
        .iter()
        .find(|w| crate::files::identity(&w.path) == here)?;
    let (records, _) = load_records(&main_root);
    records.into_iter().find(|r| describes(r, w, &main_root))
}

/// `git branch --edit-description` text, the fallback description of a
/// worktree that has no record.
fn branch_description(root: &Path, branch: &str) -> Option<String> {
    git(
        root,
        &["config", "--get", &format!("branch.{branch}.description")],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// `+A −D · N files` for the worktree against `base`. `git diff --shortstat`
/// only sees tracked changes, so a worktree holding nothing but new files
/// would otherwise read "no changes yet" here while `:reviewstart` shows
/// them as added -- the two disagreeing is worse than the extra `git`
/// call, so untracked files are folded in the same way `review_toc` counts
/// them. `None` when Git cannot say.
fn diff_summary(path: &Path, base: &str) -> Option<String> {
    if !crate::review_toc::looks_like_sha(base) {
        return None;
    }
    let text = git(path, &["diff", "--shortstat", base, "--"]).ok()?;
    let (mut files, mut added, mut removed) = (0usize, 0usize, 0usize);
    for part in text.trim().split(',') {
        let mut words = part.split_whitespace();
        let n: usize = words.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        match words.next().unwrap_or("") {
            w if w.starts_with("file") => files = n,
            w if w.starts_with("insertion") => added = n,
            w if w.starts_with("deletion") => removed = n,
            _ => {}
        }
    }
    if let Ok(untracked) = git(path, &["ls-files", "--others", "--exclude-standard", "-z"]) {
        for rel in untracked.split('\0').filter(|s| !s.is_empty()) {
            files += 1;
            if let Some(n) = crate::review_toc::count_lines(&path.join(rel)) {
                added += n;
            }
        }
    }
    Some(if files == 0 {
        "no changes yet".to_string()
    } else {
        format!(
            "+{added} −{removed} · {files} file{}",
            if files == 1 { "" } else { "s" }
        )
    })
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// The label a worktree gets in the list: its path under the main root
/// (`code-reviews/fix-parser`), `(main)` for the main worktree, otherwise
/// the full path.
fn worktree_name(w: &Worktree, main_root: &Path, is_main: bool) -> String {
    if is_main {
        return "(main)".to_string();
    }
    w.path
        .strip_prefix(main_root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| w.path.display().to_string())
}

/// The detail pane text for one worktree.
fn detail_text(w: &Worktree, record: Option<&Record>, fallback: Option<&str>) -> String {
    let mut out = Vec::new();
    match record {
        Some(r) => {
            if !r.title.is_empty() {
                out.push(r.title.clone());
            }
            if !r.description.trim().is_empty() {
                out.extend(r.description.trim().lines().map(str::to_string));
            }
            if !r.request.trim().is_empty() {
                out.push(format!("Asked: {}", r.request.trim().replace('\n', " ")));
            }
        }
        None => out.push(
            fallback
                .unwrap_or("No description recorded for this worktree")
                .to_string(),
        ),
    }
    out.push(String::new());
    out.push(format!("Path:    {}", w.path.display()));
    let branch = w.branch.as_deref().unwrap_or("(detached)");
    let head = w.head.as_deref().map(short).unwrap_or("?");
    out.push(format!("Branch:  {branch} at {head}"));
    if let Some(r) = record {
        if !r.base.is_empty() {
            let from = if r.base_ref.is_empty() {
                String::new()
            } else {
                format!(" ({})", r.base_ref)
            };
            out.push(format!("Base:    {}{from}", short(&r.base)));
        }
        let mut status = Vec::new();
        if !r.status.is_empty() {
            status.push(r.status.clone());
        }
        if !r.agent.is_empty() {
            status.push(format!("by {}", r.agent));
        }
        if !r.updated_at.is_empty() {
            status.push(format!("updated {}", r.updated_at));
        }
        if !status.is_empty() {
            out.push(format!("Status:  {}", status.join(" · ")));
        }
        if !r.notes.trim().is_empty() {
            out.push(format!("Notes:   {}", r.notes.trim().replace('\n', " ")));
        }
        if !r.test_command.is_empty() {
            out.push(format!("Test:    {}", r.test_command));
        }
        if let Some(last) = r.revisions.last() {
            out.push(format!(
                "Revised: {} time(s), last: {}",
                r.revisions.len(),
                last.summary
            ));
        }
    }
    if w.prunable {
        out.push("Git reports this worktree as missing (git worktree prune removes it)".into());
    }
    out.push("Enter: switch to this worktree and review it".into());
    out.join("\n")
}

impl Editor {
    /// `:worktrees`: the repository's worktrees, those with an agent record
    /// first (newest first), then the others, then the main worktree.
    pub fn show_worktrees(&mut self) {
        let worktrees = match list_worktrees(&self.project_root) {
            Ok(w) if !w.is_empty() => w,
            Ok(_) => return self.set_message("No Git worktrees here"),
            Err(e) => return self.set_message(format!("Not a Git repository: {e}")),
        };
        let main_root = worktrees[0].path.clone();
        let (records, errors) = load_records(&main_root);
        let here = crate::files::identity(&self.reviews.root);
        // (has a record, record's updated_at, entry) so sorting puts recorded
        // worktrees first, newest first.
        let mut rows: Vec<(u8, String, Entry)> = Vec::new();
        let mut used = vec![false; records.len()];
        for (i, w) in worktrees.iter().enumerate() {
            if w.bare {
                continue;
            }
            let is_main = i == 0;
            let found = records
                .iter()
                .enumerate()
                .find(|(j, r)| !used[*j] && describes(r, w, &main_root));
            if let Some((j, _)) = found {
                used[j] = true;
            }
            let record = found.map(|(_, r)| r);
            let fallback = w
                .branch
                .as_deref()
                .and_then(|b| branch_description(&w.path, b));
            let current = crate::files::identity(&w.path) == here;
            let title = record
                .map(|r| r.title.clone())
                .filter(|t| !t.is_empty())
                .or_else(|| {
                    fallback
                        .as_ref()
                        .and_then(|f| f.lines().next())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "(no description)".to_string());
            let stats = record
                .filter(|_| !w.prunable)
                .and_then(|r| diff_summary(&w.path, &r.base))
                .map(|s| format!("  {s}"))
                .unwrap_or_default();
            let branch = w
                .branch
                .as_deref()
                .map(|b| format!("  [{b}]"))
                .unwrap_or_else(|| "  [detached]".to_string());
            let missing = if w.prunable { "  (missing)" } else { "" };
            let text = format!(
                "{} {}  {title}{stats}{branch}{missing}",
                if current { "●" } else { " " },
                worktree_name(w, &main_root, is_main),
            );
            let mut e = Entry::text(text);
            e.detail = detail_text(w, record, fallback.as_deref());
            e.action = Some(serde_json::json!({"_vaayu_worktree_review": w.path}));
            let rank = if is_main {
                2
            } else if record.is_some() {
                0
            } else {
                1
            };
            let updated = record.map(|r| r.updated_at.clone()).unwrap_or_default();
            rows.push((rank, updated, e));
        }
        // Records whose worktree is gone (removed without its record) still
        // show, so their description is not silently lost.
        for r in records
            .iter()
            .zip(&used)
            .filter(|(_, u)| !**u)
            .map(|(r, _)| r)
        {
            let mut e = Entry::text(format!(
                "  {}  {}  (worktree removed)",
                r.name(),
                if r.title.is_empty() {
                    "(no title)"
                } else {
                    &r.title
                }
            ));
            e.detail = format!(
                "{}\n\nRecord: {}\nThe worktree it describes is not in git worktree list.",
                r.description.trim(),
                r.file.display()
            );
            rows.push((3, r.updated_at.clone(), e));
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let recorded = rows.iter().filter(|r| r.0 == 0).count();
        let mut title = format!(
            "Worktrees — {} ({recorded} with a description in {RECORDS_DIR}/) · Enter reviews",
            rows.len()
        );
        if !errors.is_empty() {
            title.push_str(&format!(" · {} unreadable record(s)", errors.len()));
            self.set_message(format!("Unreadable records: {}", errors.join("; ")));
        }
        let mut results = Results::new(title, rows.into_iter().map(|r| r.2).collect());
        results.tall_detail = true;
        self.show_results(results);
    }

    /// Enter on a `:worktrees` row: make the worktree the project (like
    /// `:cd`) and open its review screen.
    pub fn open_worktree_review(&mut self, path: &Path) {
        self.enter_normal();
        if !path.is_dir() {
            self.set_message(format!(
                "{} is missing — git worktree prune forgets it",
                path.display()
            ));
            return;
        }
        let target = crate::files::identity(path);
        if crate::files::identity(&self.project_root) != target {
            self.change_directory(&target.display().to_string());
        }
        self.review_start("");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_worktree_list() {
        let text = "worktree /repo\nHEAD 1111\nbranch refs/heads/master\n\n\
                    worktree /repo/code-reviews/fix\nHEAD 2222\nbranch refs/heads/fix\nlocked\n\n\
                    worktree /tmp/old\nHEAD 3333\ndetached\nprunable gitdir file points to non-existent location\n";
        let w = parse_worktree_list(text);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].path, PathBuf::from("/repo"));
        assert_eq!(w[0].branch.as_deref(), Some("master"));
        assert_eq!(w[1].branch.as_deref(), Some("fix"));
        assert!(w[1].locked);
        assert_eq!(w[2].branch, None);
        assert_eq!(w[2].head.as_deref(), Some("3333"));
        assert!(w[2].prunable);
    }

    #[test]
    fn record_matches_by_path_then_branch_then_slug() {
        let main = std::env::temp_dir().join(format!("vaayu-wt-match-{}", std::process::id()));
        let wt = Worktree {
            path: main.join("code-reviews/fix"),
            branch: Some("fix-branch".into()),
            ..Default::default()
        };
        let by_path = Record {
            worktree: "code-reviews/fix".into(),
            ..Default::default()
        };
        let by_branch = Record {
            branch: "fix-branch".into(),
            ..Default::default()
        };
        let by_slug = Record {
            slug: "fix".into(),
            ..Default::default()
        };
        let other = Record {
            slug: "other".into(),
            branch: "other".into(),
            ..Default::default()
        };
        assert!(describes(&by_path, &wt, &main));
        assert!(describes(&by_branch, &wt, &main));
        assert!(describes(&by_slug, &wt, &main));
        assert!(!describes(&other, &wt, &main));
    }

    #[test]
    fn records_load_newest_first_and_report_bad_files() {
        let main = std::env::temp_dir().join(format!("vaayu-wt-load-{}", std::process::id()));
        let dir = main.join(RECORDS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.json"),
            r#"{"slug": "a", "title": "Old", "updated_at": "2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("b.json"),
            r#"{"slug": "b", "title": "New", "updated_at": "2026-02-01T00:00:00Z", "extra": 1}"#,
        )
        .unwrap();
        std::fs::write(dir.join("bad.json"), "{not json").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let (records, errors) = load_records(&main);
        assert_eq!(
            records.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["New", "Old"]
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("bad.json"));
        std::fs::remove_dir_all(main).ok();
    }
}
