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
    /// For `,gd`'s diff overlay: HEAD content of lines removed at this
    /// (0-indexed, current-buffer) line position -- i.e. lines that used
    /// to sit immediately before this line and no longer exist. Several
    /// entries in one `Vec` when more than one line was removed at the
    /// same position (a net deletion, or the leftover tail of a replace
    /// whose old side was longer than its new side).
    pub deleted_before: HashMap<usize, Vec<String>>,
    /// For `,gd`'s diff overlay: char-column ranges on this (0-indexed,
    /// current-buffer) line that differ from its HEAD counterpart --
    /// e.g. a single-word rename inside an otherwise-unchanged line
    /// highlights just that word, not the whole line the way the gutter
    /// sign already does.
    pub word_diff: HashMap<usize, Vec<(usize, usize)>>,
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
            deleted_before: HashMap::new(),
            word_diff: HashMap::new(),
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
        self.deleted_before.clear();
        self.word_diff.clear();
        let head = self.head_content.as_deref().unwrap_or("");
        let head_lines: Vec<&str> = head.lines().collect();
        let current_lines: Vec<&str> = current.lines().collect();
        let last_line = current_lines.len().saturating_sub(1);
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
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    for i in new_index..new_index + new_len {
                        self.signs.insert(i, Sign::Modified);
                    }
                    // Pair the block's Nth removed line with its Nth
                    // added line (old[0]<->new[0], old[1]<->new[1], ...),
                    // the same convention `git_tools::hunk_subpatch` uses
                    // for the analogous stage/reset split: a leftover
                    // removed line (old longer than new) has no new-file
                    // position of its own, so it anchors right after the
                    // block instead of being paired for a word diff.
                    for k in 0..old_len.max(new_len) {
                        let Some(old_line) = head_lines.get(old_index + k) else {
                            continue;
                        };
                        if k < new_len {
                            if let Some(new_line) = current_lines.get(new_index + k) {
                                let spans = word_diff_spans(old_line, new_line);
                                if !spans.is_empty() {
                                    self.word_diff.insert(new_index + k, spans);
                                }
                            }
                        } else {
                            let anchor = (new_index + new_len).min(last_line);
                            self.deleted_before
                                .entry(anchor)
                                .or_default()
                                .push((*old_line).to_string());
                        }
                    }
                }
                similar::DiffOp::Delete {
                    old_index,
                    old_len,
                    new_index,
                } => {
                    let anchor = new_index.min(last_line);
                    for old_line in head_lines.iter().skip(old_index).take(old_len) {
                        self.deleted_before
                            .entry(anchor)
                            .or_default()
                            .push((*old_line).to_string());
                    }
                    self.signs.entry(anchor).or_insert(Sign::Removed);
                }
                similar::DiffOp::Equal { .. } => {}
            }
        }
    }
}
/// The char-column ranges in `new` that differ from `old`, at word
/// granularity -- merges a run of adjacent insert/delete word-changes
/// (e.g. replacing one word with a different one) into a single span
/// rather than reporting them as separate adjacent ranges, since a
/// deleted word contributes no columns of its own on the `new` side to
/// separate two insertions. Used for `,gd`'s diff overlay to highlight
/// just the changed words within an otherwise-unchanged line, not the
/// whole line.
fn word_diff_spans(old: &str, new: &str) -> Vec<(usize, usize)> {
    let diff = similar::TextDiff::from_words(old, new);
    let mut spans = Vec::new();
    let mut pos = 0usize;
    let mut current: Option<(usize, usize)> = None;
    for change in diff.iter_all_changes() {
        let chars = change.value().chars().count();
        match change.tag() {
            similar::ChangeTag::Equal => {
                if let Some(span) = current.take() {
                    spans.push(span);
                }
                pos += chars;
            }
            similar::ChangeTag::Insert => {
                let end = pos + chars;
                current = Some(match current {
                    Some((start, _)) => (start, end),
                    None => (pos, end),
                });
                pos = end;
            }
            similar::ChangeTag::Delete => {}
        }
    }
    if let Some(span) = current {
        spans.push(span);
    }
    spans
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
    /// `]c`/`[c`: jumps to the start of the next/previous changed hunk
    /// (a contiguous run of `signs` lines collapses to one stop, matching
    /// vim-gitgutter/fugitive's `]c`/`[c`, not one stop per changed line),
    /// wrapping around like `next_diagnostic` already does.
    pub fn next_hunk(&mut self, forward: bool) {
        let Some(git) = &self.git else {
            self.set_message("No git diff for this buffer");
            return;
        };
        let mut lines: Vec<usize> = git.signs.keys().copied().collect();
        if lines.is_empty() {
            self.set_message("No changes in this buffer");
            return;
        }
        lines.sort_unstable();
        let mut starts = Vec::new();
        let mut prev = None;
        for l in lines {
            if prev != Some(l.wrapping_sub(1)) {
                starts.push(l);
            }
            prev = Some(l);
        }
        let here = self.cursor().0;
        let target = if forward {
            starts.iter().find(|&&l| l > here).or(starts.first())
        } else {
            starts.iter().rev().find(|&&l| l < here).or(starts.last())
        };
        if let Some(&line) = target {
            self.push_jump();
            let col = self.buf().first_non_blank(line);
            self.set_cursor(line, col);
        }
    }
    /// `,gd`: toggles the diff overlay (deleted-line content shown as
    /// virtual text, changed-word highlighting on modified lines). Just
    /// a flag flip -- unlike `,gB`'s line blame, there's no separate
    /// fetch to kick off, since `GitGutter::refresh` already computes
    /// `deleted_before`/`word_diff` alongside the gutter signs
    /// regardless of whether this is on; turning it on only changes
    /// what the renderer reads from data that's already there.
    pub fn toggle_diff_overlay(&mut self) {
        self.diff_overlay = !self.diff_overlay;
        self.set_message(if self.diff_overlay {
            "Diff overlay on"
        } else {
            "Diff overlay off"
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn gutter(head: &str) -> GitGutter {
        GitGutter {
            head_content: Some(head.to_string()),
            signs: HashMap::new(),
            deleted_before: HashMap::new(),
            word_diff: HashMap::new(),
            seq: None,
        }
    }

    #[test]
    fn word_diff_spans_highlights_only_the_changed_word() {
        let spans = word_diff_spans("let x = old_value;", "let x = new_value;");
        // "let x = " is a shared prefix; "old_value;"/"new_value;" (the
        // similar crate's word tokenizer keeps trailing punctuation
        // attached) differ and form the one changed span.
        assert_eq!(spans, vec![(8, 18)]);
        assert_eq!(&"let x = new_value;"[8..18], "new_value;");
    }

    #[test]
    fn word_diff_spans_is_empty_for_identical_lines() {
        assert_eq!(word_diff_spans("same line", "same line"), Vec::new());
    }

    #[test]
    fn refresh_records_a_pure_deletion_anchored_at_the_next_surviving_line() {
        let mut g = gutter("a\nb\nc\nd\n");
        g.refresh("a\nd\n", 1);
        assert_eq!(
            g.deleted_before.get(&1),
            Some(&vec!["b".to_string(), "c".to_string()])
        );
        assert_eq!(g.signs.get(&1), Some(&Sign::Removed));
    }

    #[test]
    fn refresh_pairs_replaced_lines_for_a_word_diff() {
        let mut g = gutter("one\ntwo\n");
        g.refresh("one\nTWO\n", 1);
        assert_eq!(g.word_diff.get(&1), Some(&vec![(0, 3)]));
        assert!(g.deleted_before.is_empty());
    }

    #[test]
    fn refresh_anchors_a_replace_blocks_leftover_removed_lines_after_it() {
        // Three old lines collapse into one new line: the first pairs up
        // for a word diff, the other two have no new-side counterpart at
        // all and anchor right after the block instead.
        let mut g = gutter("a\nb\nc\n");
        g.refresh("A\n", 1);
        assert_eq!(
            g.deleted_before.get(&0),
            Some(&vec!["b".to_string(), "c".to_string()])
        );
        assert!(g.word_diff.contains_key(&0));
    }

    #[test]
    fn refresh_is_a_no_op_when_the_seq_is_unchanged() {
        let mut g = gutter("a\n");
        g.refresh("a\nb\n", 5);
        assert!(g.signs.contains_key(&1));
        // Same seq again, with content that would otherwise clear
        // everything -- refresh should skip recomputing entirely.
        g.refresh("a\n", 5);
        assert!(g.signs.contains_key(&1));
    }
}
