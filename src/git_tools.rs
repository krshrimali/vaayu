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
fn run(root: &Path, args: &[&str]) -> Result<String, String> {
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
pub fn apply_patch(root: &Path, patch: &str, reverse: bool) -> Result<(), String> {
    for check in [true, false] {
        let mut command = Command::new("git");
        command.arg("-C").arg(root).args(["apply", "--cached"]);
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
                } else {
                    vec!["diff", "--no-ext-diff", "--no-color", "HEAD", "--", &file]
                };
                run(&root, &args).map(|text| {
                    Results::new(
                        format!("Git {kind}"),
                        text.lines()
                            .enumerate()
                            .map(|(i, s)| {
                                Entry::location(
                                    path.clone(),
                                    if kind == "blame" { i } else { 0 },
                                    0,
                                    s,
                                )
                            })
                            .collect(),
                    )
                })
            };
            let _ = tx.send(result);
        });
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
            let result = apply_patch(&root, &patch, reverse).map(|_| {
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
                        self.show_results(r);
                    }
                }
                Err(e) => self.show_results(Results::new("Git error", vec![Entry::text(e)])),
            }
            return true;
        }
        false
    }
}
