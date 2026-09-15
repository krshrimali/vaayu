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
