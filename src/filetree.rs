//! Project file tree sidebar: `,ft` toggles a pane (the same special-pane
//! pattern as embedded terminals -- `Window::file_tree`, like
//! `Window::terminal`) showing a lazily-expanded directory listing.
//! Directories are only read when expanded, so opening the tree on a huge
//! project costs one `read_dir` of the root, not a full recursive walk.
//!
//! Scope for this slice: no `.gitignore`/dotfile filtering (only `.git`
//! itself is always skipped), no live filter, bookmarks, or Git/diagnostic
//! decorations -- see NEOVIM_PARITY_PLAN.md's progress log. Key handling
//! is entirely self-contained (its own `j`/`k`/`G`/Home/End, not routed
//! through `Awaiting::GPrefix`) since the tree's `cursor` indexes a node
//! list, not a buffer's lines -- reusing generic motion/operator dispatch
//! here would silently operate on the placeholder buffer instead.
use crate::editor::Editor;
use crate::key::Key;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
}

#[derive(Default)]
pub struct FileTree {
    pub root: PathBuf,
    pub expanded: BTreeSet<PathBuf>,
    pub cursor: usize,
    pub nodes: Vec<Node>,
}

fn list_dir(dir: &Path) -> Vec<(PathBuf, String, bool)> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != ".git")
        .map(|e| {
            let path = e.path();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = e.file_name().to_string_lossy().into_owned();
            (path, name, is_dir)
        })
        .collect();
    entries.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then(a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    entries
}

fn walk(dir: &Path, depth: usize, expanded: &BTreeSet<PathBuf>, out: &mut Vec<Node>) {
    for (path, name, is_dir) in list_dir(dir) {
        let expand_this = is_dir && expanded.contains(&path);
        out.push(Node {
            path: path.clone(),
            name,
            is_dir,
            depth,
        });
        if expand_this {
            walk(&path, depth + 1, expanded, out);
        }
    }
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut t = Self {
            root,
            expanded: BTreeSet::new(),
            cursor: 0,
            nodes: Vec::new(),
        };
        t.rebuild();
        t
    }

    pub fn rebuild(&mut self) {
        let mut nodes = Vec::new();
        walk(&self.root, 0, &self.expanded, &mut nodes);
        self.nodes = nodes;
        self.cursor = self.cursor.min(self.nodes.len().saturating_sub(1));
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.nodes.is_empty() {
            return;
        }
        let last = self.nodes.len() - 1;
        self.cursor = if delta < 0 {
            self.cursor.saturating_sub(delta.unsigned_abs())
        } else {
            (self.cursor + delta as usize).min(last)
        };
    }

    /// Enter/`o`/`l` on a directory toggles it; on a file, returns its
    /// path for the caller to open.
    fn activate(&mut self) -> Option<PathBuf> {
        let node = self.nodes.get(self.cursor)?.clone();
        if node.is_dir {
            if !self.expanded.remove(&node.path) {
                self.expanded.insert(node.path.clone());
            }
            self.rebuild();
            if let Some(i) = self.nodes.iter().position(|n| n.path == node.path) {
                self.cursor = i;
            }
            None
        } else {
            Some(node.path)
        }
    }

    /// `h`/Left: collapse the current directory, or if it's already
    /// collapsed (or this is a file), jump to its parent.
    fn collapse_or_parent(&mut self) {
        let Some(node) = self.nodes.get(self.cursor).cloned() else {
            return;
        };
        if node.is_dir && self.expanded.remove(&node.path) {
            self.rebuild();
            if let Some(i) = self.nodes.iter().position(|n| n.path == node.path) {
                self.cursor = i;
            }
            return;
        }
        let Some(parent) = node.path.parent() else {
            return;
        };
        if let Some(i) = self.nodes.iter().position(|n| n.path == parent) {
            self.cursor = i;
        }
    }

    /// Expands every ancestor of `file` and moves the cursor onto it.
    pub fn reveal(&mut self, file: &Path) {
        let Ok(rel) = file.strip_prefix(&self.root) else {
            return;
        };
        let mut cur = self.root.clone();
        for comp in rel.components() {
            cur.push(comp);
            if cur != file {
                self.expanded.insert(cur.clone());
            }
        }
        self.rebuild();
        if let Some(i) = self.nodes.iter().position(|n| n.path == file) {
            self.cursor = i;
        }
    }
}

impl Editor {
    pub fn active_file_tree(&self) -> bool {
        self.windows
            .get(self.active_window)
            .is_some_and(|w| w.file_tree)
    }

    /// `,ft`: opens the sidebar in a new vertical split, or closes it (and
    /// its pane) if already open.
    pub fn toggle_file_tree(&mut self) {
        if let Some(idx) = self.windows.iter().position(|w| w.file_tree) {
            self.active_window = idx;
            self.close_window();
            return;
        }
        let current_file = self.buf().path.clone();
        let tree = self
            .file_tree
            .get_or_insert_with(|| FileTree::new(self.project_root.clone()));
        if let Some(path) = current_file {
            tree.reveal(&path);
        }
        self.split_window(true, false);
        if let Some(w) = self.windows.get_mut(self.active_window) {
            w.file_tree = true;
        }
    }

    fn open_from_tree(&mut self, path: PathBuf) {
        let Some(other) = self.windows.iter().position(|w| !w.file_tree) else {
            return;
        };
        self.focus_window(other);
        if let Err(e) = self.open_file(path) {
            self.set_message(e.to_string());
        }
        self.store_window();
    }
}

pub fn handle_key(ed: &mut Editor, key: Key) {
    match key {
        Key::Char('j') | Key::Down => {
            if let Some(t) = &mut ed.file_tree {
                t.move_cursor(1);
            }
        }
        Key::Char('k') | Key::Up => {
            if let Some(t) = &mut ed.file_tree {
                t.move_cursor(-1);
            }
        }
        Key::Ctrl('d') | Key::PageDown => {
            if let Some(t) = &mut ed.file_tree {
                t.move_cursor(10);
            }
        }
        Key::Ctrl('u') | Key::PageUp => {
            if let Some(t) = &mut ed.file_tree {
                t.move_cursor(-10);
            }
        }
        Key::Home => {
            if let Some(t) = &mut ed.file_tree {
                t.cursor = 0;
            }
        }
        Key::End | Key::Char('G') => {
            if let Some(t) = &mut ed.file_tree {
                t.cursor = t.nodes.len().saturating_sub(1);
            }
        }
        Key::Char('h') | Key::Left => {
            if let Some(t) = &mut ed.file_tree {
                t.collapse_or_parent();
            }
        }
        Key::Enter | Key::Char('o') | Key::Char('l') | Key::Right => {
            let opened = ed.file_tree.as_mut().and_then(FileTree::activate);
            if let Some(path) = opened {
                ed.open_from_tree(path);
            }
        }
        Key::Char('R') => {
            if let Some(t) = &mut ed.file_tree {
                t.rebuild();
            }
        }
        Key::Char(':') => ed.enter_command(crate::mode::CommandKind::Ex),
        Key::Char('q') | Key::Esc => ed.toggle_file_tree(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, editor::Editor};

    fn project(files: &[&str], dirs: &[&str]) -> PathBuf {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "vaayu-filetree-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for d in dirs {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        for f in files {
            std::fs::write(root.join(f), "x").unwrap();
        }
        root
    }

    #[test]
    fn lists_root_and_skips_dot_git() {
        let root = project(&["a.txt", "b.txt"], &["src", ".git"]);
        let t = FileTree::new(root.clone());
        let names: Vec<_> = t.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"src"));
        assert!(!names.contains(&".git"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn directories_sort_before_files_then_alphabetically() {
        let root = project(&["z.txt", "a.txt"], &["dirb", "dira"]);
        let t = FileTree::new(root.clone());
        let names: Vec<_> = t.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["dira", "dirb", "a.txt", "z.txt"]);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn expand_collapse_toggles_children_lazily() {
        let root = project(&["outer.txt"], &["sub"]);
        std::fs::write(root.join("sub/inner.txt"), "x").unwrap();
        let mut t = FileTree::new(root.clone());
        assert_eq!(t.nodes.len(), 2); // "sub" dir + "outer.txt", not yet expanded
        t.cursor = t.nodes.iter().position(|n| n.name == "sub").unwrap();
        let opened = t.activate();
        assert!(opened.is_none(), "activating a directory must not open it");
        assert_eq!(t.nodes.len(), 3, "expanding should reveal inner.txt");
        assert!(t
            .nodes
            .iter()
            .any(|n| n.name == "inner.txt" && n.depth == 1));
        t.activate(); // collapse again
        assert_eq!(t.nodes.len(), 2);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn reveal_expands_ancestors_and_selects_the_file() {
        let root = project(&[], &["a/b"]);
        let target = root.join("a/b/c.txt");
        std::fs::write(&target, "x").unwrap();
        let mut t = FileTree::new(root.clone());
        assert_eq!(t.nodes.len(), 1, "nothing expanded yet");
        t.reveal(&target);
        assert_eq!(t.nodes[t.cursor].path, target);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn toggle_opens_and_closes_the_sidebar_pane() {
        let cfg = Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..Config::default()
        };
        let mut e = Editor::new(cfg);
        let root = project(&["f.txt"], &[]);
        e.project_root = root.clone();
        assert!(!e.active_file_tree());
        e.toggle_file_tree();
        assert!(e.file_tree.is_some());
        assert_eq!(e.windows.len(), 2);
        assert!(e.active_file_tree());
        e.toggle_file_tree();
        assert!(
            e.windows.is_empty() || !e.windows.iter().any(|w| w.file_tree),
            "toggling again must close the sidebar pane"
        );
        std::fs::remove_dir_all(root).ok();
    }
}
