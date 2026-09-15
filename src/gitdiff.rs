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
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let root_out = Command::new("git").arg("-C").arg(dir).arg("rev-parse").arg("--show-toplevel").output().ok()?;
        if !root_out.status.success() {
            return None;
        }
        let root = PathBuf::from(String::from_utf8_lossy(&root_out.stdout).trim());

        let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let rel = match abs_path.strip_prefix(&root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => return None,
        };

        let spec = format!("HEAD:{}", rel.to_string_lossy().replace('\\', "/"));
        let head_out = Command::new("git").arg("-C").arg(&root).arg("show").arg(&spec).output();
        let head_content = match head_out {
            Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
            _ => None,
        };

        Some(GitGutter { head_content, signs: HashMap::new(), seq: None })
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
                similar::DiffOp::Insert { new_index, new_len, .. } => {
                    for i in new_index..new_index + new_len {
                        self.signs.insert(i, Sign::Added);
                    }
                }
                similar::DiffOp::Replace { new_index, new_len, .. } => {
                    for i in new_index..new_index + new_len {
                        self.signs.insert(i, Sign::Modified);
                    }
                }
                similar::DiffOp::Delete { new_index, .. } => {
                    self.signs.entry(new_index).or_insert(Sign::Removed);
                }
                similar::DiffOp::Equal { .. } => {}
            }
        }
    }
}
