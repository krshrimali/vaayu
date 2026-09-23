use crate::{
    editor::Editor,
    windows::{Layout, Window},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
/// Per-file fold state for a session: `(path, [(start, end, closed)])`.
type FoldState = Vec<(PathBuf, Vec<(usize, usize, bool)>)>;
#[derive(Serialize, Deserialize)]
struct SessionTab {
    panes: Vec<(PathBuf, Window)>,
    layout: Option<Layout>,
    active: usize,
}
#[derive(Serialize, Deserialize)]
struct Session {
    version: u32,
    tabs: Vec<SessionTab>,
    active_tab: usize,
    /// Per-file fold state, restored onto each buffer on load.
    /// `#[serde(default)]` so older (foldless) session files still deserialize.
    #[serde(default)]
    folds: FoldState,
}
/// Rebuilds `layout` keeping only the leaves whose original pane index is
/// in `keep` (ascending), remapping each surviving leaf to its new
/// position in `keep` and collapsing any split left with a single child.
/// The result's leaf set is therefore exactly `0..keep.len()`, satisfying
/// the invariant `load_session` enforces. Returns `None` if nothing in
/// this subtree survives.
fn prune_layout(layout: &Layout, keep: &[usize]) -> Option<Layout> {
    match layout {
        Layout::Leaf(i) => keep.iter().position(|k| k == i).map(Layout::Leaf),
        Layout::Split {
            vertical,
            first,
            second,
            ratio,
        } => match (prune_layout(first, keep), prune_layout(second, keep)) {
            (Some(a), Some(b)) => Some(Layout::Split {
                vertical: *vertical,
                first: Box::new(a),
                second: Box::new(b),
                ratio: *ratio,
            }),
            (a, b) => a.or(b),
        },
    }
}
impl Editor {
    pub fn save_session(&mut self) -> anyhow::Result<()> {
        // Sync the live (current) tab back into `self.tabs` so every tab is
        // captured uniformly below.
        self.store_window();
        self.tabs[self.active_tab] = self.capture_tab();
        let mut out_tabs: Vec<SessionTab> = Vec::new();
        let mut active_tab = 0;
        for (ti, tab) in self.tabs.iter().enumerate() {
            // A single-pane tab keeps no `windows` entry (its state lives on the
            // buffer at `cur`); synthesize a window from it so non-active tabs
            // save the right buffer, not the live one.
            let windows = if tab.windows.is_empty() {
                match self.buffers.get(tab.cur) {
                    Some(b) => vec![Window {
                        buffer: b.id,
                        cursor: (b.cursor_line, b.cursor_col),
                        top: b.top_line,
                        wrap_row: b.top_wrap,
                        left: b.left_col,
                        preview: false,
                        preview_scroll: 0,
                        terminal: None,
                        file_tree: false,
                        outline: false,
                    }],
                    None => continue,
                }
            } else {
                tab.windows.clone()
            };
            // A path-less/scratch pane (or one whose buffer has gone missing)
            // can't be persisted; skip it and keep the rest. `kept` records each
            // saved pane's original index so the layout/active can be reindexed.
            let mut panes = Vec::new();
            let mut kept = Vec::new();
            for (idx, w) in windows.into_iter().enumerate() {
                let Some(b) = self.buffers.iter().find(|b| b.id == w.buffer) else {
                    continue;
                };
                let Some(path) = b.path.clone() else {
                    continue;
                };
                panes.push((path, w));
                kept.push(idx);
            }
            if panes.is_empty() {
                continue; // a whole tab with nothing saveable is dropped
            }
            let layout = if panes.len() == 1 {
                None
            } else {
                tab.window_layout
                    .as_ref()
                    .and_then(|l| prune_layout(l, &kept))
            };
            let active = kept.iter().position(|&i| i == tab.active_window).unwrap_or(0);
            if ti == self.active_tab {
                active_tab = out_tabs.len();
            }
            out_tabs.push(SessionTab {
                panes,
                layout,
                active,
            });
        }
        anyhow::ensure!(
            !out_tabs.is_empty(),
            "No saved-file panes to store in the session"
        );
        // Capture any buffer's fold state (keyed by path) so it survives a
        // session round-trip.
        let folds: FoldState = self
            .buffers
            .iter()
            .filter(|b| !b.folds.is_empty())
            .filter_map(|b| {
                b.path.clone().map(|p| {
                    (
                        p,
                        b.folds.iter().map(|f| (f.start, f.end, f.closed)).collect(),
                    )
                })
            })
            .collect();
        let dir = self.project_root.join(".vaayu");
        let _lock = crate::files::private_lock(&dir, "session.lock")?;
        crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true)?;
        crate::files::atomic_write(
            &dir.join("session.json"),
            &serde_json::to_vec(&Session {
                version: 2,
                tabs: out_tabs,
                active_tab,
                folds,
            })?,
            true,
        )
    }
    pub fn load_session(&mut self) -> anyhow::Result<()> {
        let s: Session = serde_json::from_slice(&std::fs::read(
            self.project_root.join(".vaayu/session.json"),
        )?)?;
        anyhow::ensure!(
            s.version == 2 && !s.tabs.is_empty() && s.tabs.len() <= 64 && s.active_tab < s.tabs.len(),
            "Invalid session"
        );
        fn visit(l: &Layout, depth: usize, out: &mut Vec<usize>) -> anyhow::Result<()> {
            anyhow::ensure!(depth <= 32, "Session layout too deep");
            match l {
                Layout::Leaf(i) => out.push(*i),
                Layout::Split { first, second, .. } => {
                    visit(first, depth + 1, out)?;
                    visit(second, depth + 1, out)?;
                }
            }
            Ok(())
        }
        self.store_window();
        // Buffers loaded here are deduped across every tab, so a file open in
        // two tabs shares one buffer; they're added to `self.buffers` up front.
        let mut loaded: Vec<crate::buffer::Buffer> = Vec::new();
        let mut new_tabs: Vec<crate::windows::Tab> = Vec::new();
        for st in s.tabs {
            anyhow::ensure!(
                !st.panes.is_empty() && st.panes.len() <= 32 && st.active < st.panes.len(),
                "Invalid session tab"
            );
            if let Some(l) = &st.layout {
                let mut leaves = Vec::new();
                visit(l, 0, &mut leaves)?;
                leaves.sort();
                anyhow::ensure!(
                    leaves == (0..st.panes.len()).collect::<Vec<_>>(),
                    "Invalid session pane references"
                );
            } else {
                anyhow::ensure!(st.panes.len() == 1, "Missing session layout");
            }
            let mut windows = Vec::new();
            for (path, mut w) in st.panes {
                let path = crate::files::identity(&path);
                let id = if let Some(b) = self
                    .buffers
                    .iter()
                    .chain(loaded.iter())
                    .find(|b| b.path.as_ref() == Some(&path))
                {
                    b.id
                } else {
                    let mut b = crate::buffer::Buffer::from_path(path)?;
                    b.apply_indent(&self.config);
                    let id = b.id;
                    loaded.push(b);
                    id
                };
                w.buffer = id;
                windows.push(w);
            }
            let active_window = st.active;
            new_tabs.push(crate::windows::Tab {
                windows,
                window_layout: st.layout,
                active_window,
                cur: 0,
            });
        }
        self.buffers.extend(loaded);
        // Restore saved fold state onto the (possibly freshly opened) buffers,
        // clamping to the buffer's current line count and dropping degenerate
        // ranges so a stale/edited-since session can't produce bad folds.
        for (path, folds) in s.folds {
            let path = crate::files::identity(&path);
            if let Some(b) = self.buffers.iter_mut().find(|b| b.path.as_ref() == Some(&path)) {
                let last = b.line_count().saturating_sub(1);
                b.folds = folds
                    .into_iter()
                    .take(10_000)
                    .filter(|&(start, end, _)| start < end && end <= last)
                    .map(|(start, end, closed)| crate::buffer::Fold { start, end, closed })
                    .collect();
            }
        }
        self.tabs = new_tabs;
        self.active_tab = s.active_tab;
        // Apply the active tab's live state, then place the cursor in its
        // active pane without capturing the previously active buffer over it.
        let tab = self.tabs[self.active_tab].clone();
        self.load_tab(tab);
        if let Some(w) = self.windows.get(self.active_window).cloned() {
            if let Some(i) = self.buffers.iter().position(|b| b.id == w.buffer) {
                self.cur = i;
            }
            self.set_cursor(w.cursor.0, w.cursor.1);
            self.buf_mut().top_line = w.top;
            self.buf_mut().top_wrap = w.wrap_row;
            self.buf_mut().left_col = w.left;
        }
        self.enter_normal();
        Ok(())
    }
}
