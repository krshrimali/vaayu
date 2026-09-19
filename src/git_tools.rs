//! Git commands expose their output through the same result/quickfix adapter.
use crate::{
    editor::Editor,
    results::{Entry, Results},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
};
pub type GitTask = Receiver<Result<Results, String>>;
pub type BlameTask = Receiver<Result<Vec<String>, String>>;
pub(crate) fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    if output.stdout.len() > 8 * 1024 * 1024 {
        return Err("Git output exceeds 8 MiB; narrow the operation".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
/// One-shot `git status`, for the file tree's decoration -- a
/// `path -> status letter` map (M/A/D/R/C/U modified/added/deleted/
/// renamed/copied/unmerged, `?` untracked), the same idea as
/// `--porcelain`'s two-letter codes collapsed to whichever side has one.
/// Not run per-frame: callers cache this and refresh it explicitly (tree
/// open, `R`), the same way `hunks`/`blame` are already one-shot.
pub fn status(root: &Path) -> Result<std::collections::HashMap<PathBuf, char>, String> {
    let out = run(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut map = std::collections::HashMap::new();
    let mut parts = out.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = parts.next() {
        let Some(xy) = entry.get(0..2) else { continue };
        let Some(path_str) = entry.get(3..) else {
            continue;
        };
        let marker = ['?', 'M', 'A', 'D', 'U', 'C', 'R']
            .into_iter()
            .find(|c| xy.contains(*c));
        if let Some(marker) = marker {
            map.insert(root.join(path_str), marker);
        }
        if xy.starts_with('R') || xy.starts_with('C') {
            parts.next(); // renames/copies carry a second NUL-terminated original path
        }
    }
    Ok(map)
}
/// The file tree's `.gitignore` filter: paths `git status` reports as
/// ignored. Deliberately *without* `--untracked-files=all` -- with it,
/// git expands an entirely-ignored directory into every file inside it,
/// which would defeat the tree's laziness (an ignored `target/` should
/// collapse to one entry the tree never has to read_dir into at all).
pub fn ignored(root: &Path) -> Result<std::collections::BTreeSet<PathBuf>, String> {
    let out = run(root, &["status", "--porcelain=v1", "-z", "--ignored"])?;
    Ok(out
        .split('\0')
        .filter_map(|e| e.strip_prefix("!! "))
        .map(|p| root.join(p))
        .collect())
}
/// `:gitstash`: `git stash list` as a Results list -- a picker source
/// over the whole repo, not tied to the current buffer's path the way
/// `hunks`/blame/diff already are. Enter on an entry shows that stash's
/// diff (`stash_show`), tagged via `_vaayu_git_stash_show` the same way
/// hunks tag `_vaayu_git_patch`.
pub fn stash_list(root: &Path) -> Result<Results, String> {
    let out = run(root, &["stash", "list"])?;
    let entries = out
        .lines()
        .filter_map(|line| {
            let stash_ref = line.split(':').next()?.trim().to_string();
            let mut e = Entry::text(line);
            e.action = Some(serde_json::json!({"_vaayu_git_stash_show": stash_ref}));
            Some(e)
        })
        .collect();
    Ok(Results::new("Git stash", entries))
}
/// The diff for one stash entry (`git stash show -p <stash_ref>`), shown
/// the same one-entry-per-line way `git_results`'s plain "diff"/"blame"
/// kinds already display their output.
pub fn stash_show(root: &Path, stash_ref: &str) -> Result<Results, String> {
    let text = run(root, &["stash", "show", "-p", "--no-color", stash_ref])?;
    Ok(Results::new(
        format!("Git stash: {stash_ref}"),
        text.lines().map(Entry::text).collect(),
    ))
}
/// The current commit (`git rev-parse HEAD`), for pinning a permalink to
/// a specific SHA rather than a branch name that can move.
pub fn head_commit(root: &Path) -> Result<String, String> {
    Ok(run(root, &["rev-parse", "HEAD"])?.trim().to_string())
}
/// `origin`'s remote URL, for turning the current file into a GitHub
/// permalink.
pub fn remote_url(root: &Path) -> Result<String, String> {
    Ok(run(root, &["remote", "get-url", "origin"])?
        .trim()
        .to_string())
}
/// Parses a GitHub remote URL -- SSH (`git@github.com:owner/repo.git`),
/// HTTPS/HTTP (`https://github.com/owner/repo.git`) or the `ssh://`
/// long form, trailing `.git` optional -- into `(owner, repo)`. `None`
/// for anything that isn't a github.com remote.
pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches(".git").trim_end_matches('/');
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let (owner, repo) = rest.split_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}
pub fn hunks(root: &Path, path: &Path, staged: bool) -> Result<Results, String> {
    let file = path.to_str().ok_or("Git path is not UTF-8")?;
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-color",
        "--no-renames",
        "--unified=3",
    ];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", file]);
    let patch = run(root, &args)?;
    let mut header = String::new();
    let mut chunks = Vec::<String>::new();
    for line in patch.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            chunks.push(line.into());
        } else if let Some(h) = chunks.last_mut() {
            h.push_str(line);
        } else {
            header.push_str(line);
        }
    }
    let entries=chunks.into_iter().map(|h|{let title=h.lines().next().unwrap_or("");let line=title.split_whitespace().find(|s|s.starts_with('+')).and_then(|s|s[1..].split(',').next()?.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);let mut e=Entry::location(path.into(),line,0,title);e.detail=h.clone();e.action=Some(serde_json::json!({"_vaayu_git_patch":format!("{header}{h}"),"reverse":staged,"root":root}));e}).collect();
    Ok(Results::new(
        if staged {
            "Staged hunks — Enter unstages one hunk"
        } else {
            "Git hunks — Enter stages one saved hunk"
        },
        entries,
    ))
}
/// For a full `git diff`'s raw text lines (as `:gitdiff`/`git_results`
/// shows them, one `Entry` per line): the 0-indexed new-file line each
/// one corresponds to, so jumping to a diff line actually lands
/// somewhere meaningful instead of always line 1. A preamble line
/// (before the first `@@`) gets 0; a `@@` header itself and every
/// context/`+` line after it get the running new-file position (context
/// and `+` lines advance it, `-` lines don't -- the same "removed lines
/// anchor at the position of the next surviving line" convention
/// `hunk_subpatch` already establishes for the analogous stage/reset
/// split, so a removed line's neighbors land in the same place a
/// selected-range action would treat them as belonging to).
fn diff_line_numbers(text: &str) -> Vec<usize> {
    let mut new_line = 0usize;
    let mut in_hunk = false;
    let mut out = Vec::with_capacity(text.lines().count());
    for line in text.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            if let Some(start) = header
                .split_whitespace()
                .find(|s| s.starts_with('+'))
                .and_then(|s| s[1..].split(',').next())
                .and_then(|s| s.parse::<usize>().ok())
            {
                new_line = start.saturating_sub(1);
                in_hunk = true;
            }
            out.push(new_line);
            continue;
        }
        if !in_hunk {
            out.push(0);
            continue;
        }
        out.push(new_line);
        if !line.starts_with('-') {
            new_line += 1;
        }
    }
    out
}
/// The `+`-side line range a hunk's `@@ -a,b +c,d @@` header covers, as
/// an inclusive 0-indexed `(start, end)` -- `d` (and `b`) are omitted
/// from the header entirely when they'd be `1`, matching plain `git
/// diff`'s own header format. `hunks()` already parses this same header
/// inline for each entry's `.line` (the start only); this is the same
/// parse extended to also capture the hunk's length, for `Editor::
/// preview_current_hunk`'s "which hunk is the cursor inside" check.
fn hunk_range(detail: &str) -> Option<(usize, usize)> {
    let header = detail.lines().next()?;
    let plus = header.split_whitespace().find(|s| s.starts_with('+'))?;
    let mut parts = plus[1..].splitn(2, ',');
    let start: usize = parts.next()?.parse().ok()?;
    let count: usize = match parts.next() {
        Some(s) => s.parse().ok()?,
        None => 1,
    };
    let start0 = start.saturating_sub(1);
    Some((start0, start0 + count.saturating_sub(1)))
}
/// `git diff`'s own header convention: a count of 1 is omitted
/// entirely (`+5 @@`, not `+5,1 @@`); any other count (including 0,
/// for a pure deletion/pure addition side) is spelled out.
fn fmt_hunk_count(start: usize, count: usize) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}
/// Rebuilds a hunk's body keeping only the changes inside `[range_start,
/// range_end]` (inclusive, 0-indexed new-file line numbers). Context
/// lines are always kept as-is. A run of consecutive removed/added lines
/// (a "change block") is paired index-wise -- the 1st removed line with
/// the 1st added line, the 2nd with the 2nd, and so on, matching how a
/// straight N-line replacement is naturally read -- and each pair is
/// judged as a unit by the added line's new-file position: in range,
/// both the removal and the addition are kept (a partial replace). A
/// block with more removals than additions (net deletion) anchors its
/// leftover removals at the position right after the block, since they
/// have no new-file line of their own; a block with more additions than
/// removals (net insertion) judges its leftover additions by their own
/// sequential position, same as before. This is the same line-level
/// split `git add -p` and gitsigns.nvim's own visual-range hunk actions
/// perform internally, done directly on the unified-diff text `hunks()`
/// already hands out rather than pulling in a diff-parsing dependency.
///
/// `base_is_new` picks what an *excluded* item collapses to, since
/// which side of the change the sub-patch's base needs to match depends
/// on how it's applied: staging (`false`) applies forward against the
/// index, which -- until this line is actually staged -- still holds
/// the *old* content, so an excluded removal (or the removed half of an
/// excluded pair) becomes context at its old content, and an excluded
/// pure addition (nothing to fall back to on the old side) is dropped
/// entirely. Resetting (`true`) applies in reverse against the working
/// tree, which already holds the *new* content, so it's the mirror
/// image: an excluded addition (or the added half of a pair) becomes
/// context at its new content, and an excluded pure removal (nothing on
/// the new side at all -- the line's simply not there) is dropped.
/// Getting this backwards produces a sub-patch whose context lines
/// don't match either base and fails to apply at all.
///
/// `old_start`/`new_start` are unchanged (they're just this hunk's
/// starting position in each file, unaffected by which internal lines
/// get dropped/converted); `old_count`/`new_count` are recomputed from
/// what actually remains. Returns `None` if the hunk has no header, or
/// if nothing in `[range_start, range_end]` actually changed (the range
/// only touched context lines) -- there'd be nothing to apply.
fn hunk_subpatch(
    hunk: &str,
    range_start: usize,
    range_end: usize,
    base_is_new: bool,
) -> Option<String> {
    enum Line<'a> {
        Ctx(&'a str),
        Del(&'a str),
        Add(&'a str),
    }
    let mut lines = hunk.lines();
    let header = lines.next()?;
    let old_at = header.split_whitespace().find(|s| s.starts_with('-'))?;
    let new_at = header.split_whitespace().find(|s| s.starts_with('+'))?;
    let old_start: usize = old_at[1..].split(',').next()?.parse().ok()?;
    let new_start: usize = new_at[1..].split(',').next()?.parse().ok()?;
    let tokens: Vec<Line> = lines
        .map(|l| {
            if let Some(r) = l.strip_prefix('+') {
                Line::Add(r)
            } else if let Some(r) = l.strip_prefix('-') {
                Line::Del(r)
            } else {
                Line::Ctx(l.strip_prefix(' ').unwrap_or(l))
            }
        })
        .collect();

    let mut new_line = new_start;
    let mut body = String::new();
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    let mut any_change = false;
    let push = |body: &mut String, marker: char, content: &str| {
        body.push(marker);
        body.push_str(content);
        body.push('\n');
    };
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            Line::Ctx(c) => {
                push(&mut body, ' ', c);
                old_count += 1;
                new_count += 1;
                new_line += 1;
                i += 1;
            }
            Line::Del(_) | Line::Add(_) => {
                let mut dels = Vec::new();
                let mut adds = Vec::new();
                while let Some(tok) = tokens.get(i) {
                    match tok {
                        Line::Del(c) => {
                            dels.push(*c);
                            i += 1;
                        }
                        Line::Add(c) => {
                            adds.push(*c);
                            i += 1;
                        }
                        Line::Ctx(_) => break,
                    }
                }
                let base = new_line;
                let (ndels, nadds) = (dels.len(), adds.len());
                for k in 0..ndels.max(nadds) {
                    let has_del = k < ndels;
                    let has_add = k < nadds;
                    let idx0 = if has_add {
                        (base + k).saturating_sub(1)
                    } else {
                        (base + nadds).saturating_sub(1)
                    };
                    let in_range = idx0 >= range_start && idx0 <= range_end;
                    match (has_del, has_add) {
                        (true, true) => {
                            if in_range {
                                push(&mut body, '-', dels[k]);
                                push(&mut body, '+', adds[k]);
                                any_change = true;
                            } else {
                                push(&mut body, ' ', if base_is_new { adds[k] } else { dels[k] });
                            }
                            old_count += 1;
                            new_count += 1;
                        }
                        (true, false) => {
                            // Pure removal: no new-side content exists at
                            // all, so an excluded one can only be context
                            // when the base is the *old* side (staging).
                            if in_range {
                                push(&mut body, '-', dels[k]);
                                old_count += 1;
                                any_change = true;
                            } else if base_is_new {
                                // dropped: absent on the new side already
                            } else {
                                push(&mut body, ' ', dels[k]);
                                old_count += 1;
                                new_count += 1;
                            }
                        }
                        (false, true) => {
                            // Pure addition: mirror image of the above --
                            // an excluded one is context only when the
                            // base is the *new* side (resetting).
                            if in_range {
                                push(&mut body, '+', adds[k]);
                                new_count += 1;
                                any_change = true;
                            } else if base_is_new {
                                push(&mut body, ' ', adds[k]);
                                old_count += 1;
                                new_count += 1;
                            }
                        }
                        (false, false) => unreachable!(),
                    }
                }
                new_line = base + nadds;
            }
        }
    }
    if !any_change {
        return None;
    }
    Some(format!(
        "@@ -{} +{} @@\n{body}",
        fmt_hunk_count(old_start, old_count),
        fmt_hunk_count(new_start, new_count),
    ))
}
/// Builds one appliable sub-patch per hunk in `entries` restricted to
/// its overlap with `[start, end]`, via `hunk_subpatch` -- the full
/// header each `hunks()` entry already carries in `_vaayu_git_patch` is
/// recovered by stripping the entry's own whole-hunk `detail` off the
/// end (the same trick `hunks()`'s construction makes possible: the
/// full patch is always `header + detail` verbatim), then paired with
/// the range-restricted body instead of the whole-hunk one. `base_is_new`
/// is forwarded to `hunk_subpatch` -- `false` for staging (forward,
/// against the index's still-old content), `true` for resetting
/// (reverse, against the working tree's already-new content).
fn subpatches_for_range(
    entries: &[Entry],
    start: usize,
    end: usize,
    base_is_new: bool,
) -> Vec<String> {
    entries
        .iter()
        .filter_map(|e| {
            let full = e.action.as_ref()?.get("_vaayu_git_patch")?.as_str()?;
            let header = full.strip_suffix(e.detail.as_str())?;
            let (hstart, hend) = hunk_range(&e.detail)?;
            let sub = hunk_subpatch(&e.detail, start.max(hstart), end.min(hend), base_is_new)?;
            Some(format!("{header}{sub}"))
        })
        .collect()
}
/// One plain `git blame` output line (e.g. `^abc1234 (Author Name
/// 2024-01-15 10:23:45 +0000  5) content` -- a leading `^` marks a
/// boundary commit) reduced to `"<hash> <author/date, tz>"` for the
/// line-blame virtual text: the commit hash, then the parenthesized
/// metadata with its trailing line number stripped (the metadata is
/// free-form author name plus date/time/tz, not reliably splittable
/// into fields any further without knowing the author name's own word
/// count, so the line number -- always the last whitespace-delimited
/// token before the close paren -- is the only piece safe to drop).
fn blame_line_meta(raw: &str) -> String {
    let hash = raw
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('^');
    let open = raw.find('(');
    let meta = open
        .and_then(|o| raw[o..].find(')').map(|c| (o, o + c)))
        .map(|(o, c)| raw[o + 1..c].trim_end())
        .map(|inner| {
            inner
                .rfind(char::is_whitespace)
                .map(|i| inner[..i].trim_end())
                .unwrap_or(inner)
        })
        .unwrap_or("");
    if meta.is_empty() {
        hash.to_string()
    } else {
        format!("{hash} {meta}")
    }
}
/// `,gB`: spawns a background `git blame` for `path`, the same
/// background-thread pattern `git_results`'s own "blame" kind already
/// uses to avoid blocking the main loop on a large file/history.
pub fn spawn_blame(root: &Path, path: &Path) -> BlameTask {
    let (tx, rx) = mpsc::channel();
    let root = root.to_path_buf();
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let file = path.to_string_lossy().into_owned();
        // Unlike `git diff`, `git blame` has no `--no-color` flag at all
        // (it's ambiguous with `--no-color-lines`/`--no-color-by-age` and
        // git refuses to guess) -- plain output already has no color
        // when not attached to a tty, matching `git_results`'s own
        // "blame" kind, which omits any color flag for the same reason.
        let result = run(&root, &["blame", "--", &file])
            .map(|text| text.lines().map(blame_line_meta).collect());
        let _ = tx.send(result);
    });
    rx
}
/// Applies (or, `reverse`, un-applies) a hunk's patch text. `cached`
/// targets the index (staging/unstaging, `git apply --cached`, the
/// existing behavior every caller before hunk reset used); `!cached`
/// targets the working tree file directly instead, which is what
/// discarding a hunk back to HEAD's content actually needs -- staging
/// only ever touches the index, never a file an open buffer might
/// already have loaded, so hunk reset is the first caller that needs
/// this distinction at all.
pub fn apply_patch(root: &Path, patch: &str, reverse: bool, cached: bool) -> Result<(), String> {
    for check in [true, false] {
        let mut command = Command::new("git");
        command.arg("-C").arg(root).arg("apply");
        if cached {
            command.arg("--cached");
        }
        if reverse {
            command.arg("--reverse");
        }
        if check {
            command.arg("--check");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(patch.as_bytes())
            .map_err(|e| e.to_string())?;
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().into());
        }
    }
    Ok(())
}
impl Editor {
    /// `:gitstash`: lists `git stash list`, or a message if there's
    /// nothing stashed. Synchronous, like `git_tools::status`/`ignored`
    /// -- `git stash list` is a cheap, local, no-diff-computation call,
    /// not worth the background-thread machinery `hunks`/blame/diff use.
    pub fn show_git_stash(&mut self) {
        let root = self.project_root.clone();
        match stash_list(&root) {
            Ok(r) if r.entries.is_empty() => self.set_message("No stashes"),
            Ok(r) => self.show_results(r),
            Err(e) => self.set_message(e),
        }
    }
    pub(crate) fn show_git_stash_diff(&mut self, stash_ref: &str) {
        let root = self.project_root.clone();
        match stash_show(&root, stash_ref) {
            Ok(r) => self.show_results(r),
            Err(e) => self.set_message(e),
        }
    }
    /// Generates a GitHub permalink -- a blob URL pinned to a commit SHA
    /// (not a branch name, which can move) with a `#L<n>` or
    /// `#L<n>-L<m>` line-range fragment -- for `path` at 0-indexed lines
    /// `start..=end`, copies it to the clipboard/`+` register, and shows
    /// it in the message line. `commit` overrides HEAD: used by a
    /// `:gitblame` entry's own commit ("selected commit" in
    /// NEOVIM_PARITY_PLAN.md's Phase 4 item 6), so the link points at
    /// whichever commit actually introduced that line rather than the
    /// tip of the branch. Never opens a browser or touches the network --
    /// this only reads local git state and formats a string.
    pub fn generate_permalink(
        &mut self,
        path: &std::path::Path,
        start: usize,
        end: usize,
        commit: Option<String>,
    ) {
        let root = self.project_root.clone();
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let remote = match remote_url(&root) {
            Ok(u) => u,
            Err(e) => {
                self.set_message(format!("No git remote: {e}"));
                return;
            }
        };
        let Some((owner, repo)) = parse_github_remote(&remote) else {
            self.set_message("origin is not a github.com remote");
            return;
        };
        let commit = match commit {
            Some(c) => c,
            None => match head_commit(&root) {
                Ok(c) => c,
                Err(e) => {
                    self.set_message(e);
                    return;
                }
            },
        };
        let (lo, hi) = (start.min(end), start.max(end));
        let fragment = if lo == hi {
            format!("L{}", lo + 1)
        } else {
            format!("L{}-L{}", lo + 1, hi + 1)
        };
        let url = format!("https://github.com/{owner}/{repo}/blob/{commit}/{relative}#{fragment}");
        self.registers.set(Some('+'), url.clone(), false);
        self.set_message(format!("Copied permalink: {url}"));
    }
    /// `P` on a Results entry: for `:gitblame`, uses that line's own
    /// blamed commit (parsed as the first whitespace-delimited token,
    /// stripping a leading `^` for a boundary commit) instead of HEAD --
    /// the "selected commit" case. Any other Results list with a path
    /// falls back to HEAD, same as the cursor/selection binding.
    pub fn permalink_from_results_entry(&mut self) {
        let Some(r) = &self.results else { return };
        let is_blame = r.title == "Git blame";
        let Some(entry) = r.entries.get(r.cursor).cloned() else {
            return;
        };
        let Some(path) = entry.path.clone() else {
            self.set_message("This entry has no file to link to");
            return;
        };
        let commit = is_blame
            .then(|| entry.text.split_whitespace().next())
            .flatten()
            .map(|s| s.trim_start_matches('^').to_string());
        self.generate_permalink(&path, entry.line, entry.line, commit);
    }
    /// `,gh`: shows the diff for the saved hunk the cursor is inside (or,
    /// short of that, the nearest one starting before the cursor) as a
    /// read-only Results list, without staging/unstaging or navigating
    /// away first -- distinct from `:gitstage`'s full list and `]c`/`[c`'s
    /// pure navigation. Synchronous like `:gitstash`: computing hunks is
    /// a single `git diff` call, not worth the background-thread
    /// machinery `git_results` uses for its own "diff"/"blame" kinds.
    /// Shared by `preview_current_hunk`/`reset_current_hunk_prompt`: the
    /// unstaged hunk containing the cursor's line, or (cursor between
    /// hunks) the nearest one starting before it. Both callers need the
    /// same buffer-has-a-path/isn't-dirty checks and the same "which
    /// hunk" lookup; only what they *do* with the found hunk differs.
    /// Shared by `hunk_at_cursor`/`hunks_overlapping`: the open-file/
    /// isn't-dirty checks plus the saved hunk list itself.
    fn fetch_hunks(&mut self, refuse_reason: &str) -> Option<(PathBuf, PathBuf, Vec<Entry>)> {
        let Some(path) = self.buf().path.clone() else {
            self.set_message("Open a repository file first");
            return None;
        };
        if self.buf().is_modified() {
            self.set_message(refuse_reason);
            return None;
        }
        let root = self.project_root.clone();
        let r = match hunks(&root, &path, false) {
            Ok(r) => r,
            Err(e) => {
                self.set_message(e);
                return None;
            }
        };
        if r.entries.is_empty() {
            self.set_message("No changed hunks in this file");
            return None;
        }
        Some((root, path, r.entries))
    }
    fn hunk_at_cursor(&mut self, refuse_reason: &str) -> Option<(PathBuf, PathBuf, Entry)> {
        let line = self.cursor().0;
        let (root, path, entries) = self.fetch_hunks(refuse_reason)?;
        let ranged = entries
            .iter()
            .filter(|e| !e.detail.is_empty())
            .filter_map(|e| hunk_range(&e.detail).map(|rng| (rng, e)));
        let best = ranged
            .clone()
            .find(|((s, z), _)| line >= *s && line <= *z)
            .or_else(|| {
                ranged
                    .filter(|((s, _), _)| *s <= line)
                    .max_by_key(|((s, _), _)| *s)
            });
        match best {
            Some((_, e)) => Some((root, path, e.clone())),
            None => {
                self.set_message("No changed hunk at or before the cursor");
                None
            }
        }
    }
    /// All saved hunks overlapping `[start, end]` (inclusive, 0-indexed
    /// new-file line numbers) -- unlike `hunk_at_cursor`'s "nearest
    /// single hunk", a Visual selection can span multiple hunks, or only
    /// part of one; staging/resetting a range needs one sub-patch per
    /// overlapping hunk, not just the closest one.
    fn hunks_overlapping(
        &mut self,
        start: usize,
        end: usize,
        refuse_reason: &str,
    ) -> Option<(PathBuf, PathBuf, Vec<Entry>)> {
        let (root, path, entries) = self.fetch_hunks(refuse_reason)?;
        let overlapping: Vec<Entry> = entries
            .into_iter()
            .filter(|e| !e.detail.is_empty())
            .filter(|e| hunk_range(&e.detail).is_some_and(|(s, z)| s <= end && z >= start))
            .collect();
        if overlapping.is_empty() {
            self.set_message("No changed hunk in the selected range");
            return None;
        }
        Some((root, path, overlapping))
    }
    pub fn preview_current_hunk(&mut self) {
        if let Some((_, _, entry)) = self.hunk_at_cursor("Save this buffer before previewing hunks")
        {
            self.show_results(Results::new(
                "Hunk preview",
                entry.detail.lines().map(Entry::text).collect(),
            ));
        }
    }
    /// `,gx`: shows the hunk under the cursor (same lookup as `,gh`'s
    /// preview) as a confirmation prompt -- Enter on any of its lines
    /// discards it, restoring the working-tree file to HEAD's content
    /// for just that hunk; `q`/Esc cancels, same as dismissing any other
    /// Results list. Showing the exact hunk before doing anything
    /// destructive matches this plan's own Phase 4 exit criteria.
    pub fn reset_current_hunk_prompt(&mut self) {
        let Some((root, path, entry)) =
            self.hunk_at_cursor("Save this buffer before resetting a hunk")
        else {
            return;
        };
        let Some(patch) = entry
            .action
            .as_ref()
            .and_then(|a| a["_vaayu_git_patch"].as_str())
        else {
            self.set_message("Could not read this hunk's patch");
            return;
        };
        let mut r = Results::new(
            "Reset this hunk back to HEAD? Enter discards it — q/Esc cancels",
            entry.detail.lines().map(Entry::text).collect(),
        );
        let action = serde_json::json!({"_vaayu_git_hunk_reset": {
            "patch": patch, "path": path, "root": root,
        }});
        for e in &mut r.entries {
            e.action = Some(action.clone());
        }
        self.show_results(r);
    }
    /// Applies (in reverse, to the working tree, never the index) the
    /// hunk reset a `_vaayu_git_hunk_reset`-tagged entry describes, then
    /// reloads the affected buffer (if it's open) from disk so its
    /// in-memory content matches what the reset just wrote -- otherwise
    /// the buffer would silently disagree with the file underneath it.
    /// Re-checks the dirty guard `reset_current_hunk_prompt` already
    /// checked once: the buffer could in principle have been edited
    /// (via another window onto the same file) between showing the
    /// prompt and confirming it.
    pub fn apply_hunk_reset(&mut self, value: &serde_json::Value) {
        let root: PathBuf = match serde_json::from_value(value["root"].clone()) {
            Ok(p) => p,
            Err(_) => self.project_root.clone(),
        };
        let Ok(path) = serde_json::from_value::<PathBuf>(value["path"].clone()) else {
            self.set_message("Invalid hunk reset request");
            return;
        };
        let patch = value["patch"].as_str().unwrap_or("").to_string();
        if self
            .buffers
            .iter()
            .find(|b| b.path.as_ref() == Some(&path))
            .is_some_and(|b| b.is_modified())
        {
            self.set_message("Save this buffer before resetting a hunk");
            return;
        }
        match apply_patch(&root, &patch, true, false) {
            Ok(()) => {
                if let Some(b) = self
                    .buffers
                    .iter_mut()
                    .find(|b| b.path.as_ref() == Some(&path))
                {
                    if let Err(e) = b.reload() {
                        self.set_message(format!("Hunk reset on disk, but reload failed: {e}"));
                        return;
                    }
                }
                self.set_message("Hunk reset");
            }
            Err(e) => self.set_message(format!("Hunk reset failed: {e}")),
        }
    }
    /// `,gs`: stages the saved hunk under the cursor directly into the
    /// index -- no confirmation prompt, since staging is non-destructive
    /// and already reversible (`:gitunstage`), the same immediate-apply
    /// choice `:gitstage`'s own list makes on Enter.
    pub fn stage_current_hunk(&mut self) {
        let Some((root, _path, entry)) =
            self.hunk_at_cursor("Save this buffer before staging a hunk")
        else {
            return;
        };
        let Some(patch) = entry
            .action
            .as_ref()
            .and_then(|a| a["_vaayu_git_patch"].as_str())
        else {
            self.set_message("Could not read this hunk's patch");
            return;
        };
        match apply_patch(&root, patch, false, true) {
            Ok(()) => self.set_message("Hunk staged"),
            Err(e) => self.set_message(format!("Stage failed: {e}")),
        }
    }
    /// `,gs` in Visual mode: stages only the lines within `[start, end]`
    /// -- one sub-patch per overlapping hunk (see `hunk_subpatch`), so a
    /// selection covering part of a hunk stages exactly those lines,
    /// leaving the rest of the hunk as an unstaged change.
    pub fn stage_range(&mut self, start: usize, end: usize) {
        let Some((root, _path, entries)) =
            self.hunks_overlapping(start, end, "Save this buffer before staging")
        else {
            return;
        };
        let patches = subpatches_for_range(&entries, start, end, false);
        if patches.is_empty() {
            self.set_message("No changed lines in the selected range");
            return;
        }
        let mut staged = 0;
        for patch in &patches {
            match apply_patch(&root, patch, false, true) {
                Ok(()) => staged += 1,
                Err(e) => {
                    self.set_message(format!("Stage failed after {staged} hunk(s): {e}"));
                    return;
                }
            }
        }
        self.set_message(format!(
            "Staged {staged} hunk{}",
            if staged == 1 { "" } else { "s" }
        ));
    }
    /// `,gx` in Visual mode: shows the selected lines' combined
    /// sub-patches as a confirmation prompt, same idea as
    /// `reset_current_hunk_prompt` but scoped to `[start, end]` rather
    /// than a whole hunk -- Enter discards exactly those lines, `q`/Esc
    /// cancels.
    pub fn reset_range_prompt(&mut self, start: usize, end: usize) {
        let Some((root, path, entries)) =
            self.hunks_overlapping(start, end, "Save this buffer before resetting")
        else {
            return;
        };
        let patches = subpatches_for_range(&entries, start, end, true);
        if patches.is_empty() {
            self.set_message("No changed lines in the selected range");
            return;
        }
        let preview: Vec<String> = patches
            .iter()
            .flat_map(|p| {
                p.lines()
                    .skip_while(|l| !l.starts_with("@@"))
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut r = Results::new(
            "Reset the selected lines back to HEAD? Enter discards them — q/Esc cancels",
            preview.into_iter().map(Entry::text).collect(),
        );
        let action = serde_json::json!({"_vaayu_git_hunk_reset_range": {
            "patches": patches, "path": path, "root": root,
        }});
        for e in &mut r.entries {
            e.action = Some(action.clone());
        }
        self.show_results(r);
    }
    /// Applies each sub-patch a `_vaayu_git_hunk_reset_range`-tagged
    /// entry carries (working tree only, same as `apply_hunk_reset`),
    /// then reloads the affected buffer once at the end.
    pub fn apply_hunk_reset_range(&mut self, value: &serde_json::Value) {
        let root: PathBuf = match serde_json::from_value(value["root"].clone()) {
            Ok(p) => p,
            Err(_) => self.project_root.clone(),
        };
        let Ok(path) = serde_json::from_value::<PathBuf>(value["path"].clone()) else {
            self.set_message("Invalid hunk reset request");
            return;
        };
        let Some(patches) = value["patches"].as_array() else {
            self.set_message("Invalid hunk reset request");
            return;
        };
        if self
            .buffers
            .iter()
            .find(|b| b.path.as_ref() == Some(&path))
            .is_some_and(|b| b.is_modified())
        {
            self.set_message("Save this buffer before resetting");
            return;
        }
        let mut reset = 0;
        for patch in patches {
            let Some(patch) = patch.as_str() else {
                continue;
            };
            match apply_patch(&root, patch, true, false) {
                Ok(()) => reset += 1,
                Err(e) => {
                    self.set_message(format!("Reset failed after {reset} hunk(s): {e}"));
                    return;
                }
            }
        }
        if let Some(b) = self
            .buffers
            .iter_mut()
            .find(|b| b.path.as_ref() == Some(&path))
        {
            if let Err(e) = b.reload() {
                self.set_message(format!("Reset on disk, but reload failed: {e}"));
                return;
            }
        }
        self.set_message(format!(
            "Reset {reset} hunk{}",
            if reset == 1 { "" } else { "s" }
        ));
    }
    pub fn git_results(&mut self, kind: &str) {
        let Some(path) = self.buf().path.clone() else {
            self.set_message("Open a repository file first");
            return;
        };
        if matches!(kind, "stage" | "unstage") && self.buf().is_modified() {
            self.set_message("Save this buffer before staging or unstaging hunks");
            return;
        }
        let root = self.project_root.clone();
        let kind = kind.to_string();
        // Only the read-only "diff" view honors this -- deliberately
        // never threaded into `hunks()` (stage/unstage/reset all build
        // their patches from it): `--ignore-all-space` can shift a
        // hunk's reported context just enough that `git apply` no
        // longer matches it, which is fine for a display-only diff but
        // not worth the risk for anything that mutates the index or
        // working tree from the result.
        let ignore_ws = self.diff_ignore_whitespace && kind == "diff";
        let (tx, rx) = mpsc::channel();
        self.git_task = Some(rx);
        self.set_message("Reading Git results…");
        std::thread::spawn(move || {
            let result = if kind == "stage" || kind == "unstage" {
                hunks(&root, &path, kind == "unstage")
            } else {
                let file = path.to_string_lossy();
                let args = if kind == "blame" {
                    vec!["blame", "--", &file]
                } else if ignore_ws {
                    vec![
                        "diff",
                        "--no-ext-diff",
                        "--no-color",
                        "--ignore-all-space",
                        "HEAD",
                        "--",
                        &file,
                    ]
                } else {
                    vec!["diff", "--no-ext-diff", "--no-color", "HEAD", "--", &file]
                };
                run(&root, &args).map(|text| {
                    let diff_lines = (kind != "blame").then(|| diff_line_numbers(&text));
                    Results::new(
                        format!("Git {kind}"),
                        text.lines()
                            .enumerate()
                            .map(|(i, s)| {
                                let line = if kind == "blame" {
                                    i
                                } else {
                                    diff_lines.as_ref().map_or(0, |l| l[i])
                                };
                                Entry::location(path.clone(), line, 0, s)
                            })
                            .collect(),
                    )
                })
            };
            let _ = tx.send(result);
        });
    }
    /// `,gW`: toggles whitespace-ignoring for `:gitdiff`'s view, then
    /// re-runs it immediately if that's the list currently showing (so
    /// toggling has an instant, visible effect rather than only taking
    /// hold the next time `:gitdiff` happens to be opened).
    pub fn toggle_diff_ignore_whitespace(&mut self) {
        self.diff_ignore_whitespace = !self.diff_ignore_whitespace;
        self.set_message(if self.diff_ignore_whitespace {
            "Diff: ignoring whitespace"
        } else {
            "Diff: showing whitespace changes"
        });
        if self.results.as_ref().is_some_and(|r| r.title == "Git diff") {
            self.git_results("diff");
        }
    }
    pub fn stage_result(&mut self, value: serde_json::Value) {
        let root: PathBuf = serde_json::from_value(value["root"].clone())
            .unwrap_or_else(|_| self.project_root.clone());
        let patch = value["_vaayu_git_patch"].as_str().unwrap_or("").to_string();
        let reverse = value["reverse"].as_bool().unwrap_or(false);
        let (tx, rx) = mpsc::channel();
        self.git_task = Some(rx);
        self.set_message("Updating Git index…");
        std::thread::spawn(move || {
            let result = apply_patch(&root, &patch, reverse, true).map(|_| {
                Results::new(
                    "Git index updated",
                    vec![Entry::text(if reverse {
                        "Hunk unstaged"
                    } else {
                        "Hunk staged"
                    })],
                )
            });
            let _ = tx.send(result);
        });
    }
    pub fn poll_git_task(&mut self) -> bool {
        let result = self.git_task.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.git_task = None;
            match result {
                Ok(r) => {
                    if self.mode == crate::mode::Mode::Insert {
                        self.results = Some(r);
                        self.set_message("Git results ready — Ctrl-Q to view");
                    } else {
                        // Replaces whatever busy message ("Reading Git
                        // results…", "Running git push…") was showing
                        // while this was in flight -- otherwise it just
                        // sits there looking like the operation never
                        // finished, even once its results are on screen.
                        let count = r.entries.len();
                        let title = r.title.clone();
                        self.show_results(r);
                        self.set_message(format!(
                            "{title} — {count} result{}",
                            if count == 1 { "" } else { "s" }
                        ));
                    }
                }
                Err(e) => self.show_results(Results::new("Git error", vec![Entry::text(e)])),
            }
            return true;
        }
        false
    }
    /// `,gB`: toggles line-blame virtual text. Turning it on kicks off a
    /// background `git blame` for the current buffer; turning it off
    /// drops whatever's already loaded (and any in-flight request) --
    /// there's nothing left to show either way, so no need to keep it
    /// around for a possible re-enable.
    pub fn toggle_line_blame(&mut self) {
        if self.blame_toggle {
            self.blame_toggle = false;
            self.line_blame = None;
            self.line_blame_path = None;
            self.blame_task = None;
            return;
        }
        let Some(path) = self.buf().path.clone() else {
            self.set_message("Open a repository file first");
            return;
        };
        let root = self.project_root.clone();
        self.blame_toggle = true;
        self.line_blame_path = Some(path.clone());
        self.line_blame = None;
        self.blame_task = Some(spawn_blame(&root, &path));
        self.set_message("Reading blame…");
    }
    /// Called each frame (via `poll_jobs`, alongside the other
    /// potentially-slow git polls): picks up a finished background
    /// `git blame`, if one is in flight.
    pub fn poll_blame_task(&mut self) -> bool {
        let result = self.blame_task.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.blame_task = None;
            match result {
                Ok(lines) => {
                    self.line_blame = Some(lines);
                    self.set_message("Blame ready");
                }
                Err(e) => {
                    self.set_message(e);
                    self.blame_toggle = false;
                    self.line_blame_path = None;
                }
            }
            return true;
        }
        false
    }
}
#[cfg(test)]
mod tests {
    use super::{
        blame_line_meta, diff_line_numbers, hunk_range, hunk_subpatch, parse_github_remote,
    };

    #[test]
    fn parses_an_ssh_remote() {
        assert_eq!(
            parse_github_remote("git@github.com:owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parses_an_https_remote_without_the_git_suffix() {
        assert_eq!(
            parse_github_remote("https://github.com/owner/repo"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parses_the_long_ssh_url_form() {
        assert_eq!(
            parse_github_remote("ssh://git@github.com/owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn rejects_a_non_github_remote() {
        assert_eq!(parse_github_remote("git@gitlab.com:owner/repo.git"), None);
        assert_eq!(
            parse_github_remote("https://example.com/owner/repo.git"),
            None
        );
    }

    #[test]
    fn hunk_range_parses_a_normal_header() {
        assert_eq!(hunk_range("@@ -1,2 +1,3 @@\n"), Some((0, 2)));
    }

    #[test]
    fn hunk_range_defaults_the_count_to_one_when_omitted() {
        // git omits the count entirely when it's 1, e.g. "+5 @@" not "+5,1 @@".
        assert_eq!(hunk_range("@@ -5 +5 @@\n"), Some((4, 4)));
    }

    #[test]
    fn hunk_range_handles_a_pure_deletion() {
        // A "+c,0" (nothing added) still yields a valid single-point range
        // at the deletion location, not a panic from an underflowing count.
        assert_eq!(hunk_range("@@ -5,3 +4,0 @@\n"), Some((3, 3)));
    }

    #[test]
    fn diff_line_numbers_maps_preamble_lines_to_zero() {
        let text = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n";
        assert_eq!(diff_line_numbers(text), vec![0, 0, 0, 0]);
    }

    #[test]
    fn diff_line_numbers_tracks_context_and_added_lines() {
        let text = "@@ -1,3 +1,3 @@\n line0\n line1\n line2\n";
        assert_eq!(diff_line_numbers(text), vec![0, 0, 1, 2]);
    }

    #[test]
    fn diff_line_numbers_anchors_a_block_replacement_the_same_way_hunk_subpatch_does() {
        let text = "@@ -1,3 +1,3 @@\n line0\n-line1\n-line2\n+CHANGED1\n+CHANGED2\n line3\n";
        // header, ctx(line0)=0, then new_line=1 for both removals
        // (neither advances it), CHANGED1=1 (advances to 2),
        // CHANGED2=2 (advances to 3), trailing ctx=3.
        assert_eq!(diff_line_numbers(text), vec![0, 0, 1, 1, 1, 2, 3]);
    }

    #[test]
    fn diff_line_numbers_handles_multiple_hunks_independently() {
        let text = "@@ -1,1 +1,1 @@\n-a\n+b\n@@ -10,1 +10,1 @@\n-c\n+d\n";
        assert_eq!(diff_line_numbers(text), vec![0, 0, 0, 9, 9, 9]);
    }

    #[test]
    fn blame_line_meta_strips_the_line_number_keeps_the_rest() {
        assert_eq!(
            blame_line_meta("abc1234 (Kush Ravi 2024-01-15 10:23:45 +0000  5) some code"),
            "abc1234 Kush Ravi 2024-01-15 10:23:45 +0000"
        );
    }

    #[test]
    fn blame_line_meta_strips_a_boundary_commit_marker() {
        assert_eq!(
            blame_line_meta("^abc1234 (Kush Ravi 2024-01-15 10:23:45 +0000  1) first line"),
            "abc1234 Kush Ravi 2024-01-15 10:23:45 +0000"
        );
    }

    #[test]
    fn blame_line_meta_falls_back_to_just_the_hash_on_a_malformed_line() {
        assert_eq!(blame_line_meta("not a real blame line"), "not");
        assert_eq!(blame_line_meta(""), "");
    }

    #[test]
    fn hunk_subpatch_keeps_only_in_range_added_lines_dropping_the_rest_for_staging() {
        let hunk = "@@ -2,2 +2,5 @@\n ctx1\n+add1\n+add2\n+add3\n ctx2\n";
        // idx0 2..=3 is add1 (2) and add2 (3); add3 (4) is excluded and,
        // for staging (base_is_new=false, nothing on the old side to
        // fall back to), dropped rather than kept as context.
        let sub = hunk_subpatch(hunk, 2, 3, false).unwrap();
        assert_eq!(sub, "@@ -2,2 +2,4 @@\n ctx1\n+add1\n+add2\n ctx2\n");
        assert_eq!(hunk_range(&sub), Some((1, 4)));
    }

    #[test]
    fn hunk_subpatch_keeps_excluded_added_lines_as_context_when_resetting() {
        // Same hunk, but base_is_new=true (resetting the working tree,
        // which already has all three additions): an excluded addition
        // must survive as context, not be dropped, or the sub-patch's
        // line count/content would no longer match the working tree.
        let hunk = "@@ -2,2 +2,5 @@\n ctx1\n+add1\n+add2\n+add3\n ctx2\n";
        let sub = hunk_subpatch(hunk, 2, 3, true).unwrap();
        assert_eq!(sub, "@@ -2,3 +2,5 @@\n ctx1\n+add1\n+add2\n add3\n ctx2\n");
    }

    #[test]
    fn hunk_subpatch_drops_all_added_lines_outside_the_range_for_staging() {
        let hunk = "@@ -2,2 +2,5 @@\n ctx1\n+add1\n+add2\n+add3\n ctx2\n";
        let sub = hunk_subpatch(hunk, 4, 4, false).unwrap();
        assert_eq!(sub, "@@ -2,2 +2,3 @@\n ctx1\n+add3\n ctx2\n");
    }

    #[test]
    fn hunk_subpatch_converts_out_of_range_deletions_to_context_for_staging() {
        // A pure-deletion hunk (nothing added): selecting a range that
        // doesn't overlap any of it converts every deletion back to
        // context for staging (base_is_new=false, matching the index's
        // still-old content), leaving nothing changed -- None, not an
        // empty patch.
        let hunk = "@@ -3,4 +3,1 @@\n ctxA\n-del1\n-del2\n-del3\n";
        assert_eq!(hunk_subpatch(hunk, 50, 60, false), None);
    }

    #[test]
    fn hunk_subpatch_drops_out_of_range_deletions_entirely_when_resetting() {
        // Same hunk, base_is_new=true: the working tree never had these
        // lines to begin with (they're pure removals), so an excluded
        // one can't become context -- it has to be dropped, same as an
        // excluded pure addition is for staging.
        let hunk = "@@ -3,4 +3,1 @@\n ctxA\n-del1\n-del2\n-del3\n";
        assert_eq!(hunk_subpatch(hunk, 50, 60, true), None);
    }

    #[test]
    fn hunk_subpatch_pairs_removed_and_added_lines_index_wise() {
        // A change block's Nth removed line pairs with its Nth added
        // line (old1<->new1, old2<->new2); a pair outside the selected
        // range reverts to context for staging (its old content is what
        // the index still has -- new2 is dropped, old2 stays) rather
        // than either keeping both sides or misattributing the
        // selection to the wrong pair.
        let hunk =
            "@@ -1,6 +1,7 @@\n ctx1\n ctx2\n-old1\n-old2\n+new1\n+new2\n+new3\n ctx3\n ctx4\n";
        // idx0 2 is where +new1 (paired with -old1) lands.
        let sub = hunk_subpatch(hunk, 2, 2, false).unwrap();
        assert_eq!(
            sub,
            "@@ -1,6 +1,6 @@\n ctx1\n ctx2\n-old1\n+new1\n old2\n ctx3\n ctx4\n"
        );
    }

    #[test]
    fn hunk_subpatch_replaces_only_the_selected_lines_of_a_block_replacement_for_staging() {
        // The common "replace N lines with N lines" shape: git emits all
        // the removals then all the additions, but a Visual selection
        // over just two of the five new lines should stage only those
        // two pairs, leaving the other three lines' old content
        // untouched in the index -- not sweep in the whole run of
        // deletions.
        let hunk = "@@ -8,11 +8,11 @@\n line 7\n line 8\n line 9\n-line 10\n-line 11\n\
-line 12\n-line 13\n-line 14\n+changed 10\n+changed 11\n+changed 12\n+changed 13\n\
+changed 14\n line 15\n line 16\n line 17\n";
        let sub = hunk_subpatch(hunk, 11, 12, false).unwrap();
        assert_eq!(
            sub,
            "@@ -8,11 +8,11 @@\n line 7\n line 8\n line 9\n line 10\n-line 11\n\
+changed 11\n-line 12\n+changed 12\n line 13\n line 14\n line 15\n line 16\n line 17\n"
        );
    }

    #[test]
    fn hunk_subpatch_replaces_only_the_selected_lines_of_a_block_replacement_for_resetting() {
        // Same shape, base_is_new=true: the excluded pairs must keep
        // their *new* content as context (the working tree already has
        // "changed 10"/"changed 13"/"changed 14"), not their old
        // content, or the reverse-apply's context wouldn't match what's
        // actually on disk.
        let hunk = "@@ -8,11 +8,11 @@\n line 7\n line 8\n line 9\n-line 10\n-line 11\n\
-line 12\n-line 13\n-line 14\n+changed 10\n+changed 11\n+changed 12\n+changed 13\n\
+changed 14\n line 15\n line 16\n line 17\n";
        let sub = hunk_subpatch(hunk, 11, 12, true).unwrap();
        assert_eq!(
            sub,
            "@@ -8,11 +8,11 @@\n line 7\n line 8\n line 9\n changed 10\n-line 11\n\
+changed 11\n-line 12\n+changed 12\n changed 13\n changed 14\n line 15\n line 16\n line 17\n"
        );
    }

    #[test]
    fn hunk_subpatch_over_the_whole_hunk_reproduces_it_unchanged() {
        let hunk = "@@ -2,2 +2,5 @@\n ctx1\n+add1\n+add2\n+add3\n ctx2\n";
        let (start, end) = hunk_range(hunk).unwrap();
        // Nothing is excluded either way, so `base_is_new` can't matter.
        for base_is_new in [false, true] {
            let sub = hunk_subpatch(hunk, start, end, base_is_new).unwrap();
            assert_eq!(sub, hunk);
            assert_eq!(hunk_range(&sub), Some((start, end)));
        }
    }

    #[test]
    fn hunk_subpatch_returns_none_when_the_range_touches_no_change() {
        let hunk = "@@ -2,2 +2,5 @@\n ctx1\n+add1\n+add2\n+add3\n ctx2\n";
        assert_eq!(hunk_subpatch(hunk, 100, 200, false), None);
        assert_eq!(hunk_subpatch(hunk, 100, 200, true), None);
    }
}
