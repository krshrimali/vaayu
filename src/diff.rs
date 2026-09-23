//! Minimal diff mode: `:diffthis` marks buffers to compare; the differing
//! lines of each are highlighted (a line-level diff via the `similar` crate).
//! Scroll is kept in sync between the diffed panes (scrollbind, below).
//! Unchanged-region folding and per-side colors are follow-ups.

use crate::editor::Editor;
use std::collections::HashSet;

impl Editor {
    /// `:diffthis`: mark the current buffer for diffing (with the previously
    /// marked one). Recomputes on the next `update_diff`.
    pub fn diff_this(&mut self) {
        let id = self.buf().id;
        if !self.diff_buffers.contains(&id) {
            self.diff_buffers.push(id);
        }
        self.diff_stamp = None; // force a recompute
        if self.diff_buffers.len() < 2 {
            self.set_message("diffthis: mark a second buffer (open it, :diffthis)");
        } else {
            self.set_message("Diff mode on — :diffoff to clear");
        }
    }

    /// `:diffoff`: clear diff mode.
    pub fn diff_off(&mut self) {
        self.diff_buffers.clear();
        self.diff_lines.clear();
        self.diff_stamp = None;
        self.set_message("Diff mode off");
    }

    /// Recomputes the differing lines of the first two marked buffers when
    /// their content has changed. Cheap no-op otherwise; call once per frame.
    pub fn update_diff(&mut self) {
        // Drop marks for buffers that no longer exist.
        self.diff_buffers
            .retain(|id| self.buffers.iter().any(|b| b.id == *id));
        if self.diff_buffers.len() < 2 {
            if !self.diff_lines.is_empty() {
                self.diff_lines.clear();
                self.diff_stamp = None;
            }
            return;
        }
        let (a, b) = (self.diff_buffers[0], self.diff_buffers[1]);
        let seq = |id: u64| self.buffers.iter().find(|x| x.id == id).map(|x| x.edit_seq);
        let (Some(aseq), Some(bseq)) = (seq(a), seq(b)) else {
            return;
        };
        let stamp = (a, aseq, b, bseq);
        if self.diff_stamp == Some(stamp) {
            return;
        }
        let text = |id: u64| {
            self.buffers
                .iter()
                .find(|x| x.id == id)
                .map(|x| x.rope.to_string())
                .unwrap_or_default()
        };
        let (ta, tb) = (text(a), text(b));
        let (mut set_a, mut set_b) = (HashSet::new(), HashSet::new());
        let diff = similar::TextDiff::from_lines(&ta, &tb);
        for ch in diff.iter_all_changes() {
            match ch.tag() {
                similar::ChangeTag::Delete => {
                    if let Some(i) = ch.old_index() {
                        set_a.insert(i);
                    }
                }
                similar::ChangeTag::Insert => {
                    if let Some(i) = ch.new_index() {
                        set_b.insert(i);
                    }
                }
                similar::ChangeTag::Equal => {}
            }
        }
        self.diff_lines.clear();
        self.diff_lines.insert(a, set_a);
        self.diff_lines.insert(b, set_b);
        self.diff_stamp = Some(stamp);
    }

    /// Scrollbind for diff mode: mirror the active diff pane's top line into
    /// every other pane showing a diffed buffer, so the two sides scroll
    /// together. A line-for-line mirror; hunk-aware alignment across inserted
    /// or deleted regions is a follow-up. No-op unless diff mode is on and the
    /// active pane is one of the diffed buffers (so a third, unrelated pane
    /// keeps its own scroll).
    pub fn sync_diff_scroll(&mut self) {
        if self.diff_buffers.len() < 2 || self.windows.len() < 2 {
            return;
        }
        let active = self.active_window.min(self.windows.len() - 1);
        let src_buf = self.windows[active].buffer;
        if !self.diff_buffers.contains(&src_buf) {
            return;
        }
        let top = self.windows[active].top;
        for i in 0..self.windows.len() {
            if i == active {
                continue;
            }
            let (buf_id, is_sidebar) = {
                let w = &self.windows[i];
                (
                    w.buffer,
                    w.terminal.is_some() || w.file_tree || w.outline || w.preview,
                )
            };
            if is_sidebar || !self.diff_buffers.contains(&buf_id) {
                continue;
            }
            let max_top = self
                .buffers
                .iter()
                .find(|b| b.id == buf_id)
                .map(|b| b.rope.len_lines().saturating_sub(1))
                .unwrap_or(0);
            self.windows[i].top = top.min(max_top);
        }
    }
}
