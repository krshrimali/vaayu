use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    Added,
    Modified,
    Removed,
}

pub struct GitGutter {
    /// `None` means the file has no HEAD blob (new/untracked) -- diffed
    /// against an empty string, so the whole file shows as added.
    head_content: Option<String>,
    pub signs: HashMap<usize, Sign>,
    seq: Option<u64>,
}

impl GitGutter {
    /// Returns `None` if `path` isn't inside a git work tree at all (no
    /// point tracking it -- distinct from "tracked but no HEAD blob yet",
    /// which still returns `Some` with `head_content: None`).
    pub fn new(path: &Path) -> Option<GitGutter> {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let root_out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("rev-parse")
            .arg("--show-toplevel")
            .output()
            .ok()?;
        if !root_out.status.success() {
            return None;
        }
        let root = PathBuf::from(String::from_utf8_lossy(&root_out.stdout).trim());

        let abs_path = crate::files::identity(path);
        let rel = match abs_path.strip_prefix(&root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => return None,
        };

        let spec = format!("HEAD:{}", rel.to_string_lossy().replace('\\', "/"));
        let head_out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("show")
            .arg(&spec)
            .output();
        let head_content = match head_out {
            Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
            _ => None,
        };

        Some(GitGutter {
            head_content,
            signs: HashMap::new(),
            seq: None,
        })
    }

    /// Recomputes signs against the current buffer text if `seq` (the
    /// buffer's edit sequence) changed since the last call.
    pub fn refresh(&mut self, current: &str, seq: u64) {
        if self.seq == Some(seq) {
            return;
        }
        self.seq = Some(seq);
        self.signs.clear();
        let head = self.head_content.as_deref().unwrap_or("");
        let diff = similar::TextDiff::from_lines(head, current);
        for op in diff.ops() {
            match *op {
                similar::DiffOp::Insert {
                    new_index, new_len, ..
                } => {
                    for i in new_index..new_index + new_len {
                        self.signs.insert(i, Sign::Added);
                    }
                }
                similar::DiffOp::Replace {
                    new_index, new_len, ..
                } => {
                    for i in new_index..new_index + new_len {
                        self.signs.insert(i, Sign::Modified);
                    }
                }
                similar::DiffOp::Delete { new_index, .. } => {
                    self.signs
                        .entry(new_index.min(current.lines().count().saturating_sub(1)))
                        .or_insert(Sign::Removed);
                }
                similar::DiffOp::Equal { .. } => {}
            }
        }
    }
}

pub struct GitJob {
    pub rx: Option<std::sync::mpsc::Receiver<(std::path::PathBuf, u64, Option<GitGutter>)>>,
    pub last: Option<(std::path::PathBuf, u64)>,
    pub checked: std::time::Instant,
}
impl Default for GitJob {
    fn default() -> Self {
        Self {
            rx: None,
            last: None,
            checked: std::time::Instant::now() - std::time::Duration::from_secs(3),
        }
    }
}
impl crate::editor::Editor {
    pub fn poll_git(&mut self) -> bool {
        let received = self.git_job.rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some((path, seq, git)) = received {
            self.git_job.rx = None;
            if self.buf().path.as_ref() == Some(&path) && self.buf().edit_seq == seq {
                let changed = self.git.as_ref().map(|g| &g.signs) != git.as_ref().map(|g| &g.signs);
                self.git = git;
                return changed;
            }
        }
        false
    }
    pub fn update_git_background(&mut self) {
        let Some(path) = self.buf().path.clone() else {
            self.git = None;
            return;
        };
        let seq = self.buf().edit_seq;
        if self.git_job.last.as_ref().is_some_and(|(p, _)| p != &path) {
            self.git = None;
        }
        if self.git_job.rx.is_some() {
            return;
        }
        if self.git_job.last.as_ref() == Some(&(path.clone(), seq))
            && self.git_job.checked.elapsed() < std::time::Duration::from_secs(3)
        {
            return;
        }
        self.git_job.last = Some((path.clone(), seq));
        self.git_job.checked = std::time::Instant::now();
        let rope = self.buf().rope.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.git_job.rx = Some(rx);
        std::thread::spawn(move || {
            let mut git = GitGutter::new(&path);
            if let Some(g) = &mut git {
                g.refresh(&rope.to_string(), seq);
            }
            let _ = tx.send((path, seq, git));
        });
    }
}
