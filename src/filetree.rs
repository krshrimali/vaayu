//! Project file tree sidebar: `,ft` toggles a pane (the same special-pane
//! pattern as embedded terminals -- `Window::file_tree`, like
//! `Window::terminal`) showing a lazily-expanded directory listing.
//! Directories are only read when expanded, so opening the tree on a huge
//! project costs one `read_dir` of the root, not a full recursive walk.
//!
//! Scope for this slice: dotfiles are hidden by default and toggled with
//! `.` (`.git` itself is always skipped regardless); no `.gitignore`
//! filtering, live filter, bookmarks, or Git/diagnostic decorations --
//! see NEOVIM_PARITY_PLAN.md's progress log. Key handling
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
    /// Set by a first `d` press, armed only for that exact path; a second
    /// `d` on the same node deletes it, any other key cancels. Deleting a
    /// file requires a second explicit key on purpose -- there is no undo
    /// for a real filesystem delete.
    pub confirm_delete: Option<PathBuf>,
    /// Dotfiles (other than `.git`, which is always skipped) are hidden
    /// unless this is set; `.` in the tree toggles it.
    pub show_hidden: bool,
}

fn list_dir(dir: &Path, show_hidden: bool) -> Vec<(PathBuf, String, bool)> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != ".git")
        .filter(|e| show_hidden || !e.file_name().to_string_lossy().starts_with('.'))
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

fn walk(
    dir: &Path,
    depth: usize,
    expanded: &BTreeSet<PathBuf>,
    show_hidden: bool,
    out: &mut Vec<Node>,
) {
    for (path, name, is_dir) in list_dir(dir, show_hidden) {
        let expand_this = is_dir && expanded.contains(&path);
        out.push(Node {
            path: path.clone(),
            name,
            is_dir,
            depth,
        });
        if expand_this {
            walk(&path, depth + 1, expanded, show_hidden, out);
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
            confirm_delete: None,
            show_hidden: false,
        };
        t.rebuild();
        t
    }

    pub fn rebuild(&mut self) {
        let mut nodes = Vec::new();
        walk(&self.root, 0, &self.expanded, self.show_hidden, &mut nodes);
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

    /// Where a new file/dir created "at" the cursor should live: inside
    /// the selected directory, or alongside the selected file.
    fn target_dir(&self) -> PathBuf {
        match self.nodes.get(self.cursor) {
            Some(n) if n.is_dir => n.path.clone(),
            Some(n) => n.path.parent().map_or(self.root.clone(), Path::to_path_buf),
            None => self.root.clone(),
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

    /// `:treenew name` (or `name/` for a directory): creates it inside the
    /// tree cursor's directory (or alongside its file), refusing to
    /// overwrite an existing path.
    pub fn tree_new(&mut self, name: &str) {
        let Some(tree) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        if name.is_empty() {
            self.set_message("Usage: treenew <name> (trailing / for a directory)");
            return;
        }
        let is_dir = name.ends_with('/') || name.ends_with(std::path::MAIN_SEPARATOR);
        let target = tree.target_dir().join(name.trim_end_matches('/'));
        if target.exists() {
            self.set_message(format!("{} already exists", target.display()));
            return;
        }
        let result = if is_dir {
            std::fs::create_dir_all(&target)
        } else {
            target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&target, b""))
        };
        match result {
            Ok(()) => {
                if let Some(t) = &mut self.file_tree {
                    t.reveal(&target);
                }
                self.set_message(format!("Created {}", target.display()));
            }
            Err(e) => self.set_message(format!("Create failed: {e}")),
        }
    }

    /// `:treerename name`: renames the tree cursor's node within its
    /// current directory. Refuses if the target exists, or if the file is
    /// open with unsaved changes.
    pub fn tree_rename(&mut self, name: &str) {
        let Some(tree) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        let Some(node) = tree.nodes.get(tree.cursor).cloned() else {
            self.set_message("No file tree selection");
            return;
        };
        if name.is_empty() {
            self.set_message("Usage: treerename <new-name>");
            return;
        }
        let new_path = node.path.parent().unwrap_or(&tree.root).join(name);
        if new_path.exists() {
            self.set_message(format!("{} already exists", new_path.display()));
            return;
        }
        if self
            .buffers
            .iter()
            .any(|b| b.path.as_ref() == Some(&node.path) && b.is_modified())
        {
            self.set_message("Cannot rename: open buffer has unsaved changes");
            return;
        }
        match std::fs::rename(&node.path, &new_path) {
            Ok(()) => {
                for b in &mut self.buffers {
                    if b.path.as_ref() == Some(&node.path) {
                        b.path = Some(new_path.clone());
                    }
                }
                if let Some(t) = &mut self.file_tree {
                    t.reveal(&new_path);
                }
                self.set_message(format!("Renamed to {}", new_path.display()));
            }
            Err(e) => self.set_message(format!("Rename failed: {e}")),
        }
    }

    /// Deletes the tree cursor's node. Refuses if any open buffer under
    /// that path (a directory delete may contain several) has unsaved
    /// changes.
    fn tree_delete_confirmed(&mut self, target: &Path) {
        let dirty = self.buffers.iter().any(|b| {
            b.is_modified()
                && b.path
                    .as_ref()
                    .is_some_and(|p| p == target || p.starts_with(target))
        });
        if dirty {
            self.set_message("Cannot delete: an open buffer under it has unsaved changes");
            return;
        }
        let result = if target.is_dir() {
            std::fs::remove_dir_all(target)
        } else {
            std::fs::remove_file(target)
        };
        match result {
            Ok(()) => {
                self.set_message(format!("Deleted {}", target.display()));
                if let Some(t) = &mut self.file_tree {
                    t.expanded.remove(target);
                    t.rebuild();
                }
            }
            Err(e) => self.set_message(format!("Delete failed: {e}")),
        }
    }
}

pub fn handle_key(ed: &mut Editor, key: Key) {
    if !matches!(key, Key::Char('d')) {
        if let Some(t) = &mut ed.file_tree {
            t.confirm_delete = None;
        }
    }
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
        Key::Char('.') => {
            if let Some(t) = &mut ed.file_tree {
                t.show_hidden = !t.show_hidden;
                t.rebuild();
            }
        }
        Key::Char('a') => {
            ed.enter_command(crate::mode::CommandKind::Ex);
            ed.cmdline = "treenew ".into();
        }
        Key::Char('r') => {
            ed.enter_command(crate::mode::CommandKind::Ex);
            ed.cmdline = "treerename ".into();
        }
        Key::Char('d') => {
            let Some(target) = ed
                .file_tree
                .as_ref()
                .and_then(|t| t.nodes.get(t.cursor))
                .map(|n| n.path.clone())
            else {
                return;
            };
            let armed = ed
                .file_tree
                .as_ref()
                .and_then(|t| t.confirm_delete.as_ref())
                == Some(&target);
            if armed {
                ed.tree_delete_confirmed(&target);
                if let Some(t) = &mut ed.file_tree {
                    t.confirm_delete = None;
                }
            } else {
                ed.set_message(format!(
                    "Press d again to delete {} (any other key cancels)",
                    target.display()
                ));
                if let Some(t) = &mut ed.file_tree {
                    t.confirm_delete = Some(target);
                }
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
    fn dotfiles_are_hidden_until_toggled() {
        let root = project(&["a.txt", ".hidden"], &[".hiddendir", "src", ".git"]);
        let mut t = FileTree::new(root.clone());
        let names: Vec<_> = t.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"src"));
        assert!(!names.contains(&".hidden"));
        assert!(!names.contains(&".hiddendir"));
        t.show_hidden = true;
        t.rebuild();
        let names: Vec<_> = t.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&".hidden"));
        assert!(names.contains(&".hiddendir"));
        assert!(
            !names.contains(&".git"),
            ".git stays hidden even with show_hidden"
        );
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

    fn editor_with_tree(root: &Path) -> Editor {
        let cfg = Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..Config::default()
        };
        let mut e = Editor::new(cfg);
        e.project_root = root.to_path_buf();
        e.file_tree = Some(FileTree::new(root.to_path_buf()));
        e
    }

    #[test]
    fn tree_new_creates_a_file_at_root_and_a_dir_with_trailing_slash() {
        let root = project(&[], &[]);
        let mut e = editor_with_tree(&root);
        e.tree_new("hello.txt");
        assert!(root.join("hello.txt").is_file());
        e.file_tree.as_mut().unwrap().cursor = 0; // now on the new file/dir
        e.tree_new("sub/");
        assert!(root.join("sub").is_dir());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_new_refuses_to_overwrite_an_existing_path() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        std::fs::write(root.join("a.txt"), "original").unwrap();
        e.tree_new("a.txt");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "original"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_rename_moves_the_file_and_updates_open_buffer_path() {
        let root = project(&["old.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("old.txt")).unwrap();
        e.tree_rename("new.txt");
        assert!(!root.join("old.txt").exists());
        assert!(root.join("new.txt").exists());
        assert_eq!(e.buf().path, Some(root.join("new.txt")));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tree_rename_refuses_when_buffer_has_unsaved_changes() {
        let root = project(&["old.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("old.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        assert!(e.buf().is_modified());
        e.tree_rename("new.txt");
        assert!(
            root.join("old.txt").exists(),
            "must not rename a file with unsaved changes"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn delete_requires_two_presses_and_then_removes_the_file() {
        let root = project(&["victim.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('d'));
        assert!(root.join("victim.txt").exists(), "first d only arms delete");
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_some());
        handle_key(&mut e, Key::Char('d'));
        assert!(!root.join("victim.txt").exists(), "second d deletes it");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn any_other_key_cancels_an_armed_delete() {
        let root = project(&["safe.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('d'));
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_some());
        handle_key(&mut e, Key::Char('j'));
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_none());
        handle_key(&mut e, Key::Char('d'));
        assert!(
            safe_exists(&root),
            "arming again must not delete immediately"
        );
        std::fs::remove_dir_all(root).ok();
    }
    fn safe_exists(root: &Path) -> bool {
        root.join("safe.txt").exists()
    }

    #[test]
    fn delete_refuses_when_an_open_buffer_has_unsaved_changes() {
        let root = project(&["important.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("important.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        handle_key(&mut e, Key::Char('d'));
        handle_key(&mut e, Key::Char('d'));
        assert!(
            root.join("important.txt").exists(),
            "must not delete a file with unsaved changes"
        );
        std::fs::remove_dir_all(root).ok();
    }
}
