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
impl Editor {
    pub fn save_session(&mut self) -> anyhow::Result<()> {
        self.store_window();
        let windows = if self.windows.is_empty() {
            vec![self.capture_window()]
        } else {
            self.windows.clone()
        };
        let panes = windows
            .into_iter()
            .map(|w| {
                let b = self
                    .buffers
                    .iter()
                    .find(|b| b.id == w.buffer)
                    .ok_or_else(|| anyhow::anyhow!("Missing pane buffer"))?;
                let path = b.path.clone().ok_or_else(|| {
                    anyhow::anyhow!("Session panes must refer to saved file paths")
                })?;
                Ok((path, w))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let dir = self.project_root.join(".vaayu");
        let _lock = crate::files::private_lock(&dir, "session.lock")?;
        crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true)?;
        crate::files::atomic_write(
            &dir.join("session.json"),
            &serde_json::to_vec(&Session {
                version: 1,
                panes,
                layout: self.window_layout.clone(),
                active: self.active_window,
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
                let b = crate::buffer::Buffer::from_path(path)?;
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
