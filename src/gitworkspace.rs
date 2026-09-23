//! `:gitstatus`: a sectioned Git workspace (staged/unstaged/untracked/
//! conflicts) plus commit/amend, branch/checkout, log and push/pull/fetch
//! -- the Neogit-equivalent surface, built on the same `Results` list
//! every other picker/quickfix source already uses rather than a bespoke
//! hierarchical UI (Architecture item B's tree/sidebar generalization
//! isn't built yet; a flat list with header rows gets this working now
//! without waiting on it, the same choice `:everything` already made).
use crate::{
    editor::Editor,
    git_tools::run,
    results::{Entry, Results},
};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

/// `(staged, unstaged, untracked, conflicts)` -- `staged`/`unstaged`
/// pair each path with its index (X) / worktree (Y) status code.
type StatusReport = (
    Vec<(PathBuf, char)>,
    Vec<(PathBuf, char)>,
    Vec<PathBuf>,
    Vec<PathBuf>,
);

/// One `git status --porcelain=v1 -z` entry's index (X) and worktree (Y)
/// codes kept separate, unlike `git_tools::status`'s single collapsed
/// marker for the file tree's decoration -- a partially staged file
/// (`MM`) genuinely belongs in both the staged and unstaged sections at
/// once, which a single marker can't represent.
fn parse_status(root: &Path) -> Result<StatusReport, String> {
    let out = run(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();
    let mut conflicts = Vec::new();
    let mut parts = out.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = parts.next() {
        let Some(xy) = entry.get(0..2) else { continue };
        let Some(path_str) = entry.get(3..) else {
            continue;
        };
        let path = root.join(path_str);
        if xy == "??" {
            untracked.push(path);
        } else if matches!(xy, "UU" | "AA" | "DD" | "AU" | "UA" | "UD" | "DU") {
            conflicts.push(path);
        } else {
            let mut chars = xy.chars();
            let x = chars.next().unwrap_or(' ');
            let y = chars.next().unwrap_or(' ');
            if x != ' ' {
                staged.push((path.clone(), x));
            }
            if y != ' ' {
                unstaged.push((path, y));
            }
        }
        if xy.starts_with('R') || xy.starts_with('C') {
            parts.next(); // renames/copies carry a second NUL-terminated original path
        }
    }
    Ok((staged, unstaged, untracked, conflicts))
}

fn push_section(
    entries: &mut Vec<Entry>,
    root: &Path,
    title: &str,
    tag: &str,
    rows: Vec<(PathBuf, char)>,
) {
    if rows.is_empty() {
        return;
    }
    entries.push(Entry::text(format!("── {title} ({}) ──", rows.len())));
    for (path, code) in rows {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        // `.path` is set (unlike `:gitstash`'s own plain-text-plus-
        // action rows) so `p` (preview) and `,P` (permalink) work on
        // these rows too -- `no_path_prefix` keeps `Entry::display`
        // from prepending a redundant "path:1:1  " in front of the
        // already-self-describing "M src/foo.rs" text; Enter is still
        // handled via `.action` below, not a location jump.
        let mut e = Entry::text(format!("  {code} {rel}"));
        e.path = Some(path.clone());
        e.no_path_prefix = true;
        e.action =
            Some(serde_json::json!({"_vaayu_git_status_entry": {"path": path, "section": tag}}));
        entries.push(e);
    }
}

impl Editor {
    /// `:gitstatus`: opens (or refreshes) the Git workspace. Synchronous
    /// -- a plain `git status` is cheap with no diff content to compute,
    /// the same "fast enough to call straight from the key handler"
    /// choice `:gitstash`'s own list already makes; only per-file diff/
    /// log content below gets the background-thread treatment.
    pub fn open_git_status(&mut self) {
        let root = self.project_root.clone();
        let (staged, unstaged, untracked, conflicts) = match parse_status(&root) {
            Ok(v) => v,
            Err(e) => {
                self.set_message(e);
                return;
            }
        };
        let mut entries = Vec::new();
        push_section(&mut entries, &root, "Staged", "staged", staged);
        push_section(&mut entries, &root, "Unstaged", "unstaged", unstaged);
        push_section(
            &mut entries,
            &root,
            "Untracked",
            "untracked",
            untracked.into_iter().map(|p| (p, '?')).collect(),
        );
        push_section(
            &mut entries,
            &root,
            "Conflicts",
            "conflict",
            conflicts.into_iter().map(|p| (p, 'U')).collect(),
        );
        if entries.is_empty() {
            entries.push(Entry::text("Working tree clean"));
        }
        // Just the plain title -- the key hints live in the footer
        // (`draw_results`'s `git_status` branch), not crammed in here:
        // the title row already has to make room for the "· N results
        // · M selected" suffix `draw_results` always appends, and the
        // full hint text is too long to survive that on any but a very
        // wide terminal.
        let mut r = Results::new("Git status", entries);
        r.git_status = true;
        self.show_results(r);
    }

    /// The path(s) and section tag(s) to act on for a git-status key: the
    /// Tab-selected rows if there are any, otherwise just the one under
    /// the cursor -- the same "selection scopes it, otherwise the
    /// cursor's own item does" convention already used elsewhere (e.g.
    /// `Results::export`).
    fn selected_git_status_paths(&self) -> Vec<(PathBuf, String)> {
        let Some(r) = &self.results else {
            return Vec::new();
        };
        let indices: Vec<usize> = if r.selected.is_empty() {
            vec![r.cursor]
        } else {
            r.selected.iter().copied().collect()
        };
        indices
            .into_iter()
            .filter_map(|i| r.entries.get(i))
            .filter_map(|e| {
                let v = e.action.as_ref()?.get("_vaayu_git_status_entry")?;
                let path: PathBuf = serde_json::from_value(v["path"].clone()).ok()?;
                let section = v["section"].as_str()?.to_string();
                Some((path, section))
            })
            .collect()
    }

    /// `s` in the Git workspace: stages the file(s) under the cursor (or
    /// every Tab-selected one) that aren't already staged, then
    /// refreshes the view.
    pub fn git_status_stage(&mut self) {
        let root = self.project_root.clone();
        let mut staged = 0;
        for (path, section) in self.selected_git_status_paths() {
            if section == "staged" {
                continue;
            }
            let rel = path.to_string_lossy().into_owned();
            match run(&root, &["add", "--", &rel]) {
                Ok(_) => staged += 1,
                Err(e) => {
                    self.set_message(e);
                    return;
                }
            }
        }
        self.open_git_status();
        if staged > 0 {
            self.set_message(format!(
                "Staged {staged} file{}",
                if staged == 1 { "" } else { "s" }
            ));
        }
    }

    /// `u`: mirror of `git_status_stage`, unstaging instead.
    pub fn git_status_unstage(&mut self) {
        let root = self.project_root.clone();
        let mut unstaged = 0;
        for (path, section) in self.selected_git_status_paths() {
            if section != "staged" {
                continue;
            }
            let rel = path.to_string_lossy().into_owned();
            match run(&root, &["restore", "--staged", "--", &rel]) {
                Ok(_) => unstaged += 1,
                Err(e) => {
                    self.set_message(e);
                    return;
                }
            }
        }
        self.open_git_status();
        if unstaged > 0 {
            self.set_message(format!(
                "Unstaged {unstaged} file{}",
                if unstaged == 1 { "" } else { "s" }
            ));
        }
    }

    /// `D`: shows a confirmation prompt before discarding an unstaged
    /// file's working-tree changes back to what's in the index (`git
    /// checkout -- <path>`) -- tracked files only. An untracked file's
    /// "discard" would be a delete; the file tree's own trash already
    /// covers that more safely (reversible, via `.vaayu/trash/`) and
    /// isn't duplicated here.
    pub fn git_status_discard_prompt(&mut self) {
        let paths: Vec<PathBuf> = self
            .selected_git_status_paths()
            .into_iter()
            .filter(|(_, section)| section == "unstaged")
            .map(|(p, _)| p)
            .collect();
        if paths.is_empty() {
            self.set_message(
                "Select an unstaged file to discard (untracked files aren't discarded here -- use the file tree's trash)",
            );
            return;
        }
        if let Some(dirty) = paths.iter().find(|p| {
            self.buffers
                .iter()
                .any(|b| b.path.as_deref() == Some(p.as_path()) && b.is_modified())
        }) {
            self.set_message(format!("Save {} before discarding it", dirty.display()));
            return;
        }
        let root = self.project_root.clone();
        let rel: Vec<String> = paths
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string())
            .collect();
        let mut r = Results::new(
            format!(
                "Discard changes to {} file(s)? Enter confirms — q/Esc cancels",
                paths.len()
            ),
            rel.iter().map(Entry::text).collect(),
        );
        let action =
            serde_json::json!({"_vaayu_git_status_discard": {"root": root, "paths": paths}});
        for e in &mut r.entries {
            e.action = Some(action.clone());
        }
        self.show_results(r);
    }

    /// Applies the discard a `_vaayu_git_status_discard`-tagged entry
    /// describes, then reloads any open buffer for each discarded path
    /// so it can't silently disagree with the file underneath it.
    pub fn apply_git_status_discard(&mut self, value: &serde_json::Value) {
        let root: PathBuf = serde_json::from_value(value["root"].clone())
            .unwrap_or_else(|_| self.project_root.clone());
        let Ok(paths) = serde_json::from_value::<Vec<PathBuf>>(value["paths"].clone()) else {
            self.set_message("Invalid discard request");
            return;
        };
        let mut discarded = 0;
        let mut skipped = 0;
        for path in &paths {
            // Re-check the dirty guard `git_status_discard_prompt` checked
            // once up front: the buffer could have been edited between
            // showing the prompt and confirming it (mirrors the hunk-reset
            // path's own re-check). Never discard edits made after the
            // prompt -- skip that buffer instead.
            if self
                .buffers
                .iter()
                .any(|b| b.path.as_ref() == Some(path) && b.is_modified())
            {
                skipped += 1;
                continue;
            }
            let rel = path.to_string_lossy().into_owned();
            match run(&root, &["checkout", "--", &rel]) {
                Ok(_) => {
                    discarded += 1;
                    if let Some(b) = self
                        .buffers
                        .iter_mut()
                        .find(|b| b.path.as_ref() == Some(path))
                    {
                        if let Err(e) = b.reload() {
                            self.set_message(format!("Discarded on disk, but reload failed: {e}"));
                            return;
                        }
                    }
                }
                Err(e) => {
                    self.set_message(e);
                    return;
                }
            }
        }
        self.open_git_status();
        if skipped > 0 {
            self.set_message(format!(
                "Discarded {discarded} file{}; skipped {skipped} with unsaved edits",
                if discarded == 1 { "" } else { "s" }
            ));
        } else {
            self.set_message(format!(
                "Discarded {discarded} file{}",
                if discarded == 1 { "" } else { "s" }
            ));
        }
    }

    /// `c`/`C`: pre-fills `:gitcommit `/`:gitcommitamend ` on the command
    /// line, the same "prefill, let Enter run it" pattern `rename_prompt`
    /// already uses elsewhere. A single-line commit message via the
    /// command line, not a full multi-line compose buffer -- a
    /// deliberate scope choice for a terminal grid UI; `git commit
    /// --amend` without a message still opens git's own configured
    /// $EDITOR for anyone who wants that instead.
    pub fn git_status_commit_prompt(&mut self, amend: bool) {
        self.remember_results();
        self.enter_command(crate::mode::CommandKind::Ex);
        self.cmdline = if amend {
            "gitcommitamend ".into()
        } else {
            "gitcommit ".into()
        };
    }

    /// `:gitcommit`/`:gitcommitamend`: commits currently staged changes
    /// (or amends the last commit) with `message`. An empty message on
    /// a plain commit is refused up front (matching git's own refusal,
    /// just surfaced without shelling out first); an empty message on
    /// an amend keeps the previous one (`--no-edit`), since amending
    /// just to fix the index shouldn't force a rewrite of an already-fine
    /// message.
    pub fn git_commit(&mut self, message: &str, amend: bool) {
        if message.is_empty() && !amend {
            self.set_message("Commit message required");
            return;
        }
        let root = self.project_root.clone();
        let mut args = vec!["commit"];
        if amend {
            args.push("--amend");
        }
        if message.is_empty() {
            args.push("--no-edit");
        } else {
            args.push("-m");
            args.push(message);
        }
        match run(&root, &args) {
            Ok(out) => {
                self.set_message(out.lines().next().unwrap_or("Committed").to_string());
                self.open_git_status();
            }
            Err(e) => self.set_message(e),
        }
    }

    /// `:gitrevert [hash]` (default HEAD): `git revert --no-edit <hash>`,
    /// creating a new commit that undoes it. On conflict git exits non-zero
    /// and the error (with conflict info) is surfaced for manual resolution.
    pub fn git_revert(&mut self, target: &str) {
        let root = self.project_root.clone();
        let hash = if target.trim().is_empty() {
            "HEAD"
        } else {
            target.trim()
        };
        match run(&root, &["revert", "--no-edit", hash]) {
            Ok(out) => {
                self.set_message(out.lines().next().unwrap_or("Reverted").to_string());
                self.after_git_tree_change();
            }
            Err(e) => self.set_message(format!("git revert failed: {e}")),
        }
    }

    /// `:gitcherrypick <hash>`: `git cherry-pick <hash>`, applying that commit
    /// onto the current branch.
    pub fn git_cherry_pick(&mut self, target: &str) {
        let hash = target.trim();
        if hash.is_empty() {
            self.set_message("Usage: :gitcherrypick <hash>");
            return;
        }
        let root = self.project_root.clone();
        match run(&root, &["cherry-pick", hash]) {
            Ok(out) => {
                self.set_message(out.lines().next().unwrap_or("Cherry-picked").to_string());
                self.after_git_tree_change();
            }
            Err(e) => self.set_message(format!("git cherry-pick failed: {e}")),
        }
    }

    /// After a command that rewrites the working tree (revert/cherry-pick),
    /// reload any unmodified buffer whose file changed on disk and refresh the
    /// git decorations.
    fn after_git_tree_change(&mut self) {
        let ids: Vec<u64> = self
            .buffers
            .iter()
            .filter(|b| !b.is_modified() && b.changed_on_disk())
            .map(|b| b.id)
            .collect();
        for id in ids {
            if let Some(b) = self.buffers.iter_mut().find(|b| b.id == id) {
                let _ = b.reload();
            }
        }
        self.update_git_background();
    }

    /// `:gitstashpush`: stashes all current tracked changes (`git stash
    /// push`'s own default scope -- untracked files need `-u`,
    /// deliberately not the default here either, matching git's own).
    pub fn git_stash_push(&mut self) {
        let root = self.project_root.clone();
        match run(&root, &["stash", "push"]) {
            Ok(out) => {
                self.set_message(out.lines().next().unwrap_or("Stashed changes").to_string());
                self.open_git_status();
            }
            Err(e) => self.set_message(e),
        }
    }

    /// `:gitlog`: commit log as a Results list; Enter shows that
    /// commit's diff. Computed on a background thread -- a large
    /// repository's full history can be slow to format, same reasoning
    /// `git_results`'s own "diff"/"blame" kinds already follow.
    pub fn git_log(&mut self) {
        let root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.git_task = Some(rx);
        self.set_message("Reading Git log…");
        std::thread::spawn(move || {
            let result = run(
                &root,
                &["log", "--date=short", "--pretty=format:%h %ad %an: %s"],
            )
            .map(|text| {
                let entries = text
                    .lines()
                    .map(|line| {
                        let hash = line.split_whitespace().next().unwrap_or("").to_string();
                        let mut e = Entry::text(line);
                        e.action = Some(serde_json::json!({"_vaayu_git_show_commit": hash}));
                        e
                    })
                    .collect();
                Results::new("Git log — Enter shows a commit's diff", entries)
            });
            let _ = tx.send(result);
        });
    }

    /// `:gitfilehistory`: commits that touched the current file
    /// (`git log --follow -- <file>`) as a Results list; Enter shows that
    /// commit's diff. Background thread, like `:gitlog`.
    pub fn git_file_history(&mut self) {
        let Some(path) = self.buf().path.clone() else {
            self.set_message("No file for history");
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.git_task = Some(rx);
        self.set_message("Reading file history…");
        std::thread::spawn(move || {
            let path_str = path.to_string_lossy().into_owned();
            let result = run(
                &root,
                &[
                    "log",
                    "--follow",
                    "--date=short",
                    "--pretty=format:%h %ad %an: %s",
                    "--",
                    path_str.as_str(),
                ],
            )
            .map(|text| {
                let entries = text
                    .lines()
                    .map(|line| {
                        let hash = line.split_whitespace().next().unwrap_or("").to_string();
                        let mut e = Entry::text(line);
                        e.action = Some(serde_json::json!({"_vaayu_git_show_commit": hash}));
                        e
                    })
                    .collect();
                Results::new(format!("Git history — {name} (Enter shows a commit)"), entries)
            });
            let _ = tx.send(result);
        });
    }

    /// Shows one commit's diff (`git show`), the same background-thread
    /// treatment as `:gitlog` itself -- a single commit can still touch
    /// a lot of lines.
    pub fn show_commit_diff(&mut self, hash: &str) {
        let root = self.project_root.clone();
        let hash = hash.to_string();
        let (tx, rx) = mpsc::channel();
        self.git_task = Some(rx);
        self.set_message(format!("Reading commit {hash}…"));
        std::thread::spawn(move || {
            let result = run(&root, &["show", "--no-color", &hash]).map(|text| {
                Results::new(
                    format!("Git show {hash}"),
                    text.lines().map(Entry::text).collect(),
                )
            });
            let _ = tx.send(result);
        });
    }

    /// `:gitbranch`: local branches as a Results list (current one
    /// marked with `*`); Enter checks out the selected one. Synchronous
    /// -- listing local branches is a cheap, local, non-diff operation,
    /// same as `:gitstash`'s own list.
    pub fn git_branches(&mut self) {
        let root = self.project_root.clone();
        match run(&root, &["branch", "--list"]) {
            Ok(text) => {
                let entries = text
                    .lines()
                    .filter_map(|line| {
                        let name = line.trim_start_matches('*').trim();
                        if name.is_empty() {
                            return None;
                        }
                        let mut e = Entry::text(line);
                        e.action = Some(serde_json::json!({"_vaayu_git_checkout": name}));
                        Some(e)
                    })
                    .collect();
                self.show_results(Results::new("Git branches — Enter checks it out", entries));
            }
            Err(e) => self.set_message(e),
        }
    }

    /// Checks out `name`, refusing first if any buffer has unsaved
    /// edits (a checkout can silently change what's on disk under an
    /// open buffer). Reloads every open buffer afterward so none
    /// disagrees with the newly-checked-out branch's content.
    pub fn checkout_branch(&mut self, name: &str) {
        if self.buffers.iter().any(|b| b.is_modified()) {
            self.set_message("Save all buffers before checking out another branch");
            return;
        }
        let root = self.project_root.clone();
        match run(&root, &["checkout", name]) {
            Ok(_) => {
                // A file that doesn't exist on the newly checked-out branch
                // can't be reloaded; leaving the buffer's now-stale content
                // in place would let a later `:w!` resurrect the file at a
                // path this branch doesn't have. Surface those instead of
                // swallowing the reload error.
                let mut gone = Vec::new();
                for i in 0..self.buffers.len() {
                    if self.buffers[i].reload().is_err() {
                        if let Some(p) = self.buffers[i].path.clone() {
                            if !p.exists() {
                                gone.push(
                                    p.strip_prefix(&root).unwrap_or(&p).display().to_string(),
                                );
                            }
                        }
                    }
                }
                if gone.is_empty() {
                    self.set_message(format!("Checked out {name}"));
                } else {
                    self.set_message(format!(
                        "Checked out {name}; {} open buffer(s) don't exist on this branch (content is stale — don't :w! them): {}",
                        gone.len(),
                        gone.join(", ")
                    ));
                }
            }
            Err(e) => self.set_message(e),
        }
    }

    /// `:gitpush`/`:gitpull`/`:gitfetch`: the one place in this module
    /// where a background thread actually matters -- everything else
    /// here is a fast, local, one-shot command, but these hit the
    /// network. Captures stdout+stderr together regardless of exit
    /// status, since git prints its real progress/status text to
    /// stderr even on success (unlike this module's other commands,
    /// which only need stderr on failure -- see `git_tools::run`).
    fn git_remote_op(&mut self, op: &'static str) {
        let root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.git_task = Some(rx);
        self.set_message(format!("Running git {op}…"));
        std::thread::spawn(move || {
            let result = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .arg(op)
                .output();
            let sent = match result {
                Ok(out) => {
                    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                    text.push_str(&String::from_utf8_lossy(&out.stderr));
                    if out.status.success() {
                        Ok(Results::new(
                            format!("git {op}"),
                            text.lines().map(Entry::text).collect(),
                        ))
                    } else {
                        Err(text.trim().to_string())
                    }
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(sent);
        });
    }
    pub fn git_push(&mut self) {
        self.git_remote_op("push");
    }
    pub fn git_pull(&mut self) {
        self.git_remote_op("pull");
    }
    pub fn git_fetch(&mut self) {
        self.git_remote_op("fetch");
    }
}
