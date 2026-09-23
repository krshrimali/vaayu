use crate::{
    editor::Editor,
    windows::{Layout, Window},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Serialize, Deserialize)]
struct Session {
    version: u32,
    panes: Vec<(PathBuf, Window)>,
    layout: Option<Layout>,
    active: usize,
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
        self.store_window();
        let windows = if self.windows.is_empty() {
            vec![self.capture_window()]
        } else {
            self.windows.clone()
        };
        // A path-less/scratch pane (or one whose buffer has gone missing)
        // can't be persisted, but that's no reason to throw away the whole
        // session: skip those panes and save the rest. `kept` records each
        // saved pane's original window index so the layout and active pane
        // can be reindexed onto just the panes that survive.
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
        anyhow::ensure!(
            !panes.is_empty(),
            "No saved-file panes to store in the session"
        );
        // Reindex the layout onto only the kept panes (dropping the leaves
        // for skipped ones and collapsing any split left with a single
        // child). A lone surviving pane is stored layout-free, matching the
        // load side's invariant that a single pane has no layout.
        let layout = if panes.len() == 1 {
            None
        } else {
            self.window_layout
                .as_ref()
                .and_then(|l| prune_layout(l, &kept))
        };
        let active = kept
            .iter()
            .position(|&i| i == self.active_window)
            .unwrap_or(0);
        let dir = self.project_root.join(".vaayu");
        let _lock = crate::files::private_lock(&dir, "session.lock")?;
        crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true)?;
        crate::files::atomic_write(
            &dir.join("session.json"),
            &serde_json::to_vec(&Session {
                version: 1,
                panes,
                layout,
                active,
            })?,
            true,
        )
    }
    pub fn load_session(&mut self) -> anyhow::Result<()> {
        let s: Session = serde_json::from_slice(&std::fs::read(
            self.project_root.join(".vaayu/session.json"),
        )?)?;
        anyhow::ensure!(
            s.version == 1
                && !s.panes.is_empty()
                && s.panes.len() <= 32
                && s.active < s.panes.len(),
            "Invalid session"
        );
        let mut leaves = Vec::new();
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
        if let Some(l) = &s.layout {
            visit(l, 0, &mut leaves)?;
            leaves.sort();
            anyhow::ensure!(
                leaves == (0..s.panes.len()).collect::<Vec<_>>(),
                "Invalid session pane references"
            );
        } else {
            anyhow::ensure!(s.panes.len() == 1, "Missing session layout");
        }
        let mut loaded = Vec::new();
        let mut windows = Vec::new();
        for (path, mut w) in s.panes {
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
        self.store_window();
        self.buffers.extend(loaded);
        self.windows = windows;
        self.window_layout = s.layout;
        self.active_window = s.active;
        // Load without capturing the previously active buffer over the restored pane.
        let w = self.windows[s.active].clone();
        self.cur = self.buffers.iter().position(|b| b.id == w.buffer).unwrap();
        self.set_cursor(w.cursor.0, w.cursor.1);
        self.buf_mut().top_line = w.top;
        self.buf_mut().top_wrap = w.wrap_row;
        self.buf_mut().left_col = w.left;
        self.enter_normal();
        Ok(())
    }
}
