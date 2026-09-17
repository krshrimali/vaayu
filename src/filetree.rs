//! Project file tree sidebar: `,ft` toggles a pane (the same special-pane
//! pattern as embedded terminals -- `Window::file_tree`, like
//! `Window::terminal`) showing a lazily-expanded directory listing.
//! Directories are only read when expanded, so opening the tree on a huge
//! project costs one `read_dir` of the root, not a full recursive walk.
//!
//! Scope for this slice: dotfiles are hidden by default and toggled with
//! `.` (`.git` itself is always skipped regardless); a file or directory
//! with LSP diagnostics shows an E/W/I marker (see `render.rs`'s
//! `tree_diagnostic_marker`, keyed straight off `Editor::diagnostics` so
//! an unexpanded directory's marker doesn't need its children loaded).
//! `t`/`t` moves a node into `.vaayu/trash/` (a reversible alternative to
//! `d`/`d`'s real delete), timestamped so repeated trashings of the same
//! name never collide. `y`/`x`/`p` copy/cut/paste a node (recursively for
//! a directory); `p` refuses a name collision or, for a cut, an unsaved
//! buffer under the source. `m` toggles a bookmark (shown with a ★),
//! listed by `:treebookmarks` as a Results list. `/` live-filters by
//! substring -- deliberately only over already-loaded nodes (expanded
//! directories), never a full recursive project search, since that
//! would defeat the laziness this whole module exists for; Enter keeps
//! the filter while returning to normal navigation, Esc clears it. A
//! file's `git status` letter (or `*` for a directory with any changed
//! descendant) is shown too, refreshed on open and `R` -- never re-run
//! per frame (see `Editor::refresh_tree_git_status`). `.gitignore`d
//! paths are hidden by default (`!` toggles `show_ignored`) at the
//! `list_dir` level, same granularity `git status --ignored` itself
//! uses -- an entirely-ignored directory is one hidden entry, never
//! `read_dir`'d into, not a hidden entry per file inside it.
//! NEOVIM_PARITY_PLAN.md has the remaining gaps. Key handling
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
    /// Same two-press-confirm shape as `confirm_delete`, but for `t`
    /// (trash: moves into `.vaayu/trash/` instead of removing outright).
    pub confirm_trash: Option<PathBuf>,
    /// Set by `y` (copy, `false`) or `x` (cut, `true`); `p` pastes it into
    /// the cursor's target directory. A cut is only removed from its
    /// original location once `p` actually succeeds.
    pub clipboard: Option<(PathBuf, bool)>,
    /// Dotfiles (other than `.git`, which is always skipped) are hidden
    /// unless this is set; `.` in the tree toggles it.
    pub show_hidden: bool,
    /// Paths toggled on with `m`; shown with a ★ marker and listable via
    /// `:treebookmarks`. In-memory only, like every other sidebar
    /// preference in this slice -- not saved across restarts.
    pub bookmarks: BTreeSet<PathBuf>,
    /// A live substring filter over the *currently loaded* nodes (already
    /// expanded directories) -- not a full recursive search of the whole
    /// project, which would defeat this tree's whole reason for being
    /// lazy (see the module doc comment). `/` starts typing it (empty
    /// string = active but not yet narrowing anything); `filter_input`
    /// is whether keys are currently going to the query.
    pub filter: String,
    pub filter_input: bool,
    /// `git status`, refreshed on tree open and `R` (see `Editor::
    /// refresh_tree_git_status`) -- never re-run per frame.
    pub git_status: std::collections::HashMap<PathBuf, char>,
    /// Paths `git status --ignored` reports as ignored (an ignored
    /// directory is one entry, not each file inside -- see
    /// `git_tools::ignored`'s doc comment). Refreshed alongside
    /// `git_status`. Hidden by default; `!` toggles `show_ignored`.
    pub gitignored: BTreeSet<PathBuf>,
    pub show_ignored: bool,
}

fn list_dir(
    dir: &Path,
    show_hidden: bool,
    gitignored: &BTreeSet<PathBuf>,
    show_ignored: bool,
) -> Vec<(PathBuf, String, bool)> {
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
        .filter(|(path, ..)| show_ignored || !gitignored.contains(path))
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
    gitignored: &BTreeSet<PathBuf>,
    show_ignored: bool,
    out: &mut Vec<Node>,
) {
    for (path, name, is_dir) in list_dir(dir, show_hidden, gitignored, show_ignored) {
        let expand_this = is_dir && expanded.contains(&path);
        out.push(Node {
            path: path.clone(),
            name,
            is_dir,
            depth,
        });
        if expand_this {
            walk(
                &path,
                depth + 1,
                expanded,
                show_hidden,
                gitignored,
                show_ignored,
                out,
            );
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
            confirm_trash: None,
            clipboard: None,
            show_hidden: false,
            bookmarks: BTreeSet::new(),
            filter: String::new(),
            filter_input: false,
            git_status: std::collections::HashMap::new(),
            gitignored: BTreeSet::new(),
            show_ignored: false,
        };
        t.rebuild();
        t
    }

    pub fn rebuild(&mut self) {
        let mut nodes = Vec::new();
        walk(
            &self.root,
            0,
            &self.expanded,
            self.show_hidden,
            &self.gitignored,
            self.show_ignored,
            &mut nodes,
        );
        if !self.filter.is_empty() {
            let q = self.filter.to_lowercase();
            nodes.retain(|n| n.name.to_lowercase().contains(&q));
        }
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
        self.refresh_tree_git_status();
    }

    /// Re-runs `git status` for the tree's decoration. Called on open and
    /// `R`; silently leaves it empty (no crash, no message) outside a Git
    /// repo or if `git` isn't on `PATH` -- this is a nice-to-have, not a
    /// required feature.
    pub fn refresh_tree_git_status(&mut self) {
        if let Ok(status) = crate::git_tools::status(&self.project_root) {
            if let Some(t) = &mut self.file_tree {
                t.git_status = status;
            }
        }
        if let Ok(ignored) = crate::git_tools::ignored(&self.project_root) {
            if let Some(t) = &mut self.file_tree {
                let changed = t.gitignored != ignored;
                t.gitignored = ignored;
                if changed {
                    t.rebuild();
                }
            }
        }
    }

    pub(crate) fn open_from_tree(&mut self, path: PathBuf) {
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

    /// True if any open buffer under `target` (a directory delete/trash
    /// may contain several) has unsaved changes.
    fn has_dirty_buffer_under(&self, target: &Path) -> bool {
        self.buffers.iter().any(|b| {
            b.is_modified()
                && b.path
                    .as_ref()
                    .is_some_and(|p| p == target || p.starts_with(target))
        })
    }

    /// Permanently deletes the tree cursor's node. Refuses if any open
    /// buffer under that path has unsaved changes.
    fn tree_delete_confirmed(&mut self, target: &Path) {
        if self.has_dirty_buffer_under(target) {
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

    /// Moves the tree cursor's node into `.vaayu/trash/` instead of
    /// removing it outright -- a reversible alternative to `tree_delete_confirmed`
    /// for the common "I didn't mean that" case. Refuses under the same
    /// dirty-buffer condition as a real delete.
    fn tree_trash_confirmed(&mut self, target: &Path) {
        if self.has_dirty_buffer_under(target) {
            self.set_message("Cannot trash: an open buffer under it has unsaved changes");
            return;
        }
        let trash_dir = self.project_root.join(".vaayu").join("trash");
        if let Err(e) = std::fs::create_dir_all(&trash_dir) {
            self.set_message(format!("Trash failed: {e}"));
            return;
        }
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let dest = trash_dir.join(format!("{stamp}-{name}"));
        match std::fs::rename(target, &dest) {
            Ok(()) => {
                self.set_message(format!("Trashed {} (in .vaayu/trash/)", target.display()));
                if let Some(t) = &mut self.file_tree {
                    t.expanded.remove(target);
                    t.rebuild();
                }
            }
            Err(e) => self.set_message(format!("Trash failed: {e}")),
        }
    }

    /// `y`: copies the tree cursor's node to the clipboard (`p` pastes it,
    /// leaving the original in place).
    pub fn tree_yank(&mut self) {
        let Some(path) = self
            .file_tree
            .as_ref()
            .and_then(|t| t.nodes.get(t.cursor))
            .map(|n| n.path.clone())
        else {
            return;
        };
        self.set_message(format!("Copied {} (p to paste)", path.display()));
        if let Some(t) = &mut self.file_tree {
            t.clipboard = Some((path, false));
        }
    }

    /// `x`: marks the tree cursor's node to be moved on the next `p`.
    pub fn tree_cut(&mut self) {
        let Some(path) = self
            .file_tree
            .as_ref()
            .and_then(|t| t.nodes.get(t.cursor))
            .map(|n| n.path.clone())
        else {
            return;
        };
        self.set_message(format!("Cut {} (p moves it here)", path.display()));
        if let Some(t) = &mut self.file_tree {
            t.clipboard = Some((path, true));
        }
    }

    /// `p`: pastes the clipboard entry into the cursor's target directory.
    /// Refuses a name collision, a vanished source, or (for a cut) an
    /// unsaved buffer under the source -- the same dirty-buffer condition
    /// delete/trash already use. A directory move doesn't repoint any
    /// open buffer nested inside it to the new location (matching
    /// `tree_rename`'s existing behavior for directory renames); only an
    /// exact source-path match is remapped.
    pub fn tree_paste(&mut self) {
        let Some(tree) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        let Some((src, cut)) = tree.clipboard.clone() else {
            self.set_message("Nothing to paste -- y or x a node first");
            return;
        };
        if !src.exists() {
            self.set_message(format!("{} no longer exists", src.display()));
            return;
        }
        let dest = tree.target_dir().join(src.file_name().unwrap_or_default());
        if dest == src {
            self.set_message("Source and destination are the same");
            return;
        }
        if dest.exists() {
            self.set_message(format!("{} already exists", dest.display()));
            return;
        }
        if cut && self.has_dirty_buffer_under(&src) {
            self.set_message("Cannot move: an open buffer under it has unsaved changes");
            return;
        }
        let result = if cut {
            std::fs::rename(&src, &dest)
        } else {
            copy_recursive(&src, &dest)
        };
        match result {
            Ok(()) => {
                if cut {
                    for b in &mut self.buffers {
                        if b.path.as_ref() == Some(&src) {
                            b.path = Some(dest.clone());
                        }
                    }
                }
                self.set_message(format!(
                    "{} to {}",
                    if cut { "Moved" } else { "Copied" },
                    dest.display()
                ));
                if let Some(t) = &mut self.file_tree {
                    if cut {
                        t.clipboard = None;
                    }
                    t.reveal(&dest);
                }
            }
            Err(e) => self.set_message(format!("Paste failed: {e}")),
        }
    }

    /// `m`: toggles the tree cursor's node as a bookmark.
    pub fn tree_toggle_bookmark(&mut self) {
        let Some(path) = self
            .file_tree
            .as_ref()
            .and_then(|t| t.nodes.get(t.cursor))
            .map(|n| n.path.clone())
        else {
            return;
        };
        let Some(t) = &mut self.file_tree else {
            return;
        };
        if t.bookmarks.remove(&path) {
            self.set_message(format!("Unbookmarked {}", path.display()));
        } else {
            t.bookmarks.insert(path.clone());
            self.set_message(format!("Bookmarked {}", path.display()));
        }
    }

    /// Shows the live filter's current query and match count. Only
    /// already-loaded nodes (expanded directories) are ever searched --
    /// see `FileTree::filter`'s doc comment -- so the message says so
    /// rather than implying a full-project search.
    fn report_tree_filter(&mut self) {
        let Some(t) = &self.file_tree else { return };
        self.set_message(format!(
            "Filter (loaded nodes only): {}  ({} match{})",
            t.filter,
            t.nodes.len(),
            if t.nodes.len() == 1 { "" } else { "es" }
        ));
    }

    /// `:treebookmarks`: lists the current tree's bookmarks as a Results
    /// list; Enter on a directory reveals it in the tree, on a file opens
    /// it into the adjacent pane like the tree's own Enter does.
    pub fn show_tree_bookmarks(&mut self) {
        let Some(t) = &self.file_tree else {
            self.set_message("No file tree open");
            return;
        };
        if t.bookmarks.is_empty() {
            self.set_message("No bookmarks -- m in the tree to add one");
            return;
        }
        let root = self.project_root.clone();
        let entries = t
            .bookmarks
            .iter()
            .map(|p| {
                let rel = p.strip_prefix(&root).unwrap_or(p).display().to_string();
                let mut e = crate::results::Entry::text(rel);
                e.action = Some(serde_json::json!({
                    "_vaayu_tree_bookmark": p.display().to_string(),
                    "dir": p.is_dir(),
                }));
                e
            })
            .collect();
        self.show_results(crate::results::Results::new("Tree bookmarks", entries));
    }

    /// Opens (or reveals, for a directory) a `:treebookmarks` selection.
    /// Routed through `open_from_tree`/`tree_reveal` rather than the
    /// generic path-entry branch of `open_result` -- that branch just
    /// changes `self.cur`, but with the tree pane still `active_window`
    /// (as it usually is right after `:treebookmarks`), the newly opened
    /// buffer would never actually become visible in either pane.
    pub(crate) fn open_tree_bookmark(&mut self, path: &Path, is_dir: bool) {
        if is_dir {
            self.tree_reveal(path);
        } else {
            self.open_from_tree(path.to_path_buf());
        }
    }

    /// Reveals `path` in the file tree, opening the sidebar first if it
    /// isn't already open. Used by `:treebookmarks`' directory entries,
    /// which have nothing to "open" as a buffer.
    fn tree_reveal(&mut self, path: &Path) {
        if !self.windows.iter().any(|w| w.file_tree) {
            self.toggle_file_tree();
        } else if let Some(idx) = self.windows.iter().position(|w| w.file_tree) {
            self.focus_window(idx);
        }
        if let Some(t) = &mut self.file_tree {
            t.reveal(path);
        }
    }
}

/// Recursively copies `src` to `dest` (a plain file or a whole directory
/// tree) -- `std::fs` has no built-in directory copy. On failure partway
/// through (e.g. permission denied on one nested file, or disk full),
/// removes whatever was already created at `dest` before returning the
/// error, so a failed copy never leaves a half-copied destination behind
/// for a later `y`/`p` to trip over as a spurious "already exists".
fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    let result = copy_recursive_step(src, dest);
    if result.is_err() {
        let _ = if dest.is_dir() {
            std::fs::remove_dir_all(dest)
        } else {
            std::fs::remove_file(dest)
        };
    }
    result
}

fn copy_recursive_step(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let target = dest.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_recursive_step(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    } else {
        std::fs::copy(src, dest).map(|_| ())
    }
}

pub fn handle_key(ed: &mut Editor, key: Key) {
    if ed.file_tree.as_ref().is_some_and(|t| t.filter_input) {
        match key {
            Key::Esc => {
                if let Some(t) = &mut ed.file_tree {
                    t.filter.clear();
                    t.filter_input = false;
                    t.rebuild();
                }
            }
            Key::Enter => {
                if let Some(t) = &mut ed.file_tree {
                    t.filter_input = false;
                }
            }
            Key::Backspace => {
                if let Some(t) = &mut ed.file_tree {
                    t.filter.pop();
                    t.rebuild();
                }
                ed.report_tree_filter();
            }
            Key::Down => {
                if let Some(t) = &mut ed.file_tree {
                    t.move_cursor(1);
                }
            }
            Key::Up => {
                if let Some(t) = &mut ed.file_tree {
                    t.move_cursor(-1);
                }
            }
            Key::Char(c) => {
                if let Some(t) = &mut ed.file_tree {
                    t.filter.push(c);
                    t.rebuild();
                }
                ed.report_tree_filter();
            }
            _ => {}
        }
        return;
    }
    if !matches!(key, Key::Char('d')) {
        if let Some(t) = &mut ed.file_tree {
            t.confirm_delete = None;
        }
    }
    if !matches!(key, Key::Char('t')) {
        if let Some(t) = &mut ed.file_tree {
            t.confirm_trash = None;
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
            ed.refresh_tree_git_status();
        }
        Key::Char('.') => {
            if let Some(t) = &mut ed.file_tree {
                t.show_hidden = !t.show_hidden;
                t.rebuild();
            }
        }
        Key::Char('!') => {
            if let Some(t) = &mut ed.file_tree {
                t.show_ignored = !t.show_ignored;
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
        Key::Char('t') => {
            let Some(target) = ed
                .file_tree
                .as_ref()
                .and_then(|t| t.nodes.get(t.cursor))
                .map(|n| n.path.clone())
            else {
                return;
            };
            let armed =
                ed.file_tree.as_ref().and_then(|t| t.confirm_trash.as_ref()) == Some(&target);
            if armed {
                ed.tree_trash_confirmed(&target);
                if let Some(t) = &mut ed.file_tree {
                    t.confirm_trash = None;
                }
            } else {
                ed.set_message(format!(
                    "Press t again to trash {} (any other key cancels)",
                    target.display()
                ));
                if let Some(t) = &mut ed.file_tree {
                    t.confirm_trash = Some(target);
                }
            }
        }
        Key::Char('y') => ed.tree_yank(),
        Key::Char('x') => ed.tree_cut(),
        Key::Char('p') => ed.tree_paste(),
        Key::Char('m') => ed.tree_toggle_bookmark(),
        Key::Char('/') => {
            if let Some(t) = &mut ed.file_tree {
                t.filter_input = true;
            }
            ed.report_tree_filter();
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

    /// Opens the tree through the real `,ft` path (a split, not just the
    /// `file_tree` field alone), since some operations (e.g. opening a
    /// file from `:treebookmarks`) need an actual other pane to focus.
    fn editor_with_tree(root: &Path) -> Editor {
        let cfg = Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..Config::default()
        };
        let mut e = Editor::new(cfg);
        e.project_root = root.to_path_buf();
        e.toggle_file_tree();
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

    #[test]
    fn trash_requires_two_presses_and_moves_the_file_into_vaayu_trash() {
        let root = project(&["victim.txt"], &[]);
        std::fs::write(root.join("victim.txt"), "keepsake content").unwrap();
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('t'));
        assert!(root.join("victim.txt").exists(), "first t only arms trash");
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_some());
        handle_key(&mut e, Key::Char('t'));
        assert!(
            !root.join("victim.txt").exists(),
            "second t moves it out of place"
        );
        let trash_dir = root.join(".vaayu/trash");
        let entries: Vec<_> = std::fs::read_dir(&trash_dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "exactly one file should land in trash");
        let trashed = entries.into_iter().next().unwrap().unwrap().path();
        assert!(trashed
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with("-victim.txt"));
        assert_eq!(
            std::fs::read_to_string(&trashed).unwrap(),
            "keepsake content",
            "trash must preserve file content, not just the name"
        );
        assert!(
            !e.file_tree
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|n| n.name == "victim.txt"),
            "trashed file must disappear from the tree"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn any_other_key_cancels_an_armed_trash() {
        let root = project(&["safe.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('t'));
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_some());
        handle_key(&mut e, Key::Char('j'));
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_none());
        handle_key(&mut e, Key::Char('t'));
        assert!(
            safe_exists(&root),
            "arming again must not trash immediately"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn trash_refuses_when_an_open_buffer_has_unsaved_changes() {
        let root = project(&["important.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("important.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        handle_key(&mut e, Key::Char('t'));
        handle_key(&mut e, Key::Char('t'));
        assert!(
            root.join("important.txt").exists(),
            "must not trash a file with unsaved changes"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn arming_delete_does_not_also_arm_trash_and_vice_versa() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('d'));
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_some());
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_none());
        handle_key(&mut e, Key::Char('t'));
        // 't' is "any other key" to the pending delete, canceling it, then
        // arms trash instead -- not both armed at once.
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_none());
        assert!(e.file_tree.as_ref().unwrap().confirm_trash.is_some());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn yank_then_paste_copies_the_file_and_keeps_the_original() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let mut e = editor_with_tree(&root);
        // nodes: [0]=dest (dir, sorts first), [1]=a.txt
        handle_key(&mut e, Key::Char('j')); // onto a.txt
        handle_key(&mut e, Key::Char('y'));
        assert!(e.file_tree.as_ref().unwrap().clipboard.is_some());
        handle_key(&mut e, Key::Char('k')); // back onto dest/
        handle_key(&mut e, Key::Char('p'));
        assert!(root.join("a.txt").exists(), "copy must keep the original");
        assert_eq!(
            std::fs::read_to_string(root.join("dest/a.txt")).unwrap(),
            "hello"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cut_then_paste_moves_the_file_and_updates_the_open_buffer_path() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("a.txt")).unwrap();
        handle_key(&mut e, Key::Char('j'));
        handle_key(&mut e, Key::Char('x'));
        assert_eq!(
            e.file_tree.as_ref().unwrap().clipboard,
            Some((root.join("a.txt"), true))
        );
        handle_key(&mut e, Key::Char('k'));
        handle_key(&mut e, Key::Char('p'));
        assert!(!root.join("a.txt").exists(), "cut+paste must move it");
        assert!(root.join("dest/a.txt").exists());
        assert_eq!(e.buf().path, Some(root.join("dest/a.txt")));
        assert!(
            e.file_tree.as_ref().unwrap().clipboard.is_none(),
            "clipboard should clear once the move succeeds"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn paste_refuses_a_name_collision() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "source").unwrap();
        std::fs::write(root.join("dest/a.txt"), "already here").unwrap();
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('j'));
        handle_key(&mut e, Key::Char('y'));
        handle_key(&mut e, Key::Char('k'));
        handle_key(&mut e, Key::Char('p'));
        assert_eq!(
            std::fs::read_to_string(root.join("dest/a.txt")).unwrap(),
            "already here",
            "collision must refuse, not overwrite"
        );
        assert!(root.join("a.txt").exists(), "source must be untouched");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn paste_with_nothing_copied_or_cut_is_a_harmless_no_op() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('p'));
        assert!(root.join("a.txt").exists());
    }

    #[test]
    fn cut_paste_refuses_when_the_source_has_an_unsaved_buffer() {
        let root = project(&["a.txt"], &["dest"]);
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let mut e = editor_with_tree(&root);
        e.open_file(root.join("a.txt")).unwrap();
        e.buf_mut().begin_edit();
        e.buf_mut().insert_char(0, 0, 'x');
        e.buf_mut().commit_edit();
        handle_key(&mut e, Key::Char('j'));
        handle_key(&mut e, Key::Char('x'));
        handle_key(&mut e, Key::Char('k'));
        handle_key(&mut e, Key::Char('p'));
        assert!(
            root.join("a.txt").exists(),
            "must not move a file with unsaved changes"
        );
        assert!(!root.join("dest/a.txt").exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn copy_recurses_into_a_directory() {
        let root = project(&[], &["src_dir", "dest"]);
        std::fs::write(root.join("src_dir/inner.txt"), "nested").unwrap();
        let mut e = editor_with_tree(&root);
        // nodes sorted alphabetically among dirs: dest, src_dir
        handle_key(&mut e, Key::Char('j')); // onto src_dir
        handle_key(&mut e, Key::Char('y'));
        handle_key(&mut e, Key::Char('k')); // onto dest
        handle_key(&mut e, Key::Char('p'));
        assert_eq!(
            std::fs::read_to_string(root.join("dest/src_dir/inner.txt")).unwrap(),
            "nested"
        );
        assert!(
            root.join("src_dir/inner.txt").exists(),
            "copy must keep the original directory"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn copy_recursive_rolls_back_a_partial_copy_on_failure() {
        let root = project(&[], &["src_dir"]);
        std::fs::write(root.join("src_dir/ok.txt"), "fine").unwrap();
        std::fs::write(root.join("src_dir/blocked.txt"), "denied").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            // Unreadable, so copying it fails partway through the
            // directory -- regardless of read_dir's (unspecified) entry
            // order, ok.txt and blocked.txt can't both succeed.
            std::fs::set_permissions(
                root.join("src_dir/blocked.txt"),
                std::fs::Permissions::from_mode(0o000),
            )
            .unwrap();
        }
        let dest = root.join("dest_dir");
        let result = copy_recursive(&root.join("src_dir"), &dest);
        assert!(result.is_err(), "the unreadable file should fail the copy");
        assert!(
            !dest.exists(),
            "a failed copy should roll back, leaving no partial destination \
             (even just the empty directory) behind"
        );
        // Restore permissions so the temp dir can be cleaned up.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join("src_dir/blocked.txt"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn m_toggles_a_bookmark_on_and_off() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        let path = e.file_tree.as_ref().unwrap().nodes[0].path.clone();
        handle_key(&mut e, Key::Char('m'));
        assert!(e.file_tree.as_ref().unwrap().bookmarks.contains(&path));
        handle_key(&mut e, Key::Char('m'));
        assert!(!e.file_tree.as_ref().unwrap().bookmarks.contains(&path));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn treebookmarks_lists_bookmarks_and_enter_opens_a_file_one() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('m')); // bookmark a.txt
        e.show_tree_bookmarks();
        let r = e
            .results
            .as_ref()
            .expect("treebookmarks should open a results list");
        assert_eq!(r.entries.len(), 1);
        e.open_result();
        assert_eq!(e.buf().path, Some(root.join("a.txt")));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn treebookmarks_enter_on_a_directory_reveals_it_instead_of_opening_it() {
        let root = project(&[], &["sub"]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('m')); // bookmark sub/
        e.show_tree_bookmarks();
        e.open_result();
        assert!(
            e.windows.iter().any(|w| w.file_tree),
            "reveal must (re)open the tree sidebar"
        );
        assert_eq!(
            e.file_tree.as_ref().unwrap().nodes[e.file_tree.as_ref().unwrap().cursor].path,
            root.join("sub"),
            "cursor should land on the revealed directory"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn treebookmarks_with_none_set_shows_a_message_not_an_empty_list() {
        let root = project(&["a.txt"], &[]);
        let mut e = editor_with_tree(&root);
        e.show_tree_bookmarks();
        assert!(e.results.is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn slash_live_filters_the_currently_loaded_nodes() {
        let root = project(&["apple.txt", "banana.txt", "cherry.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        assert!(e.file_tree.as_ref().unwrap().filter_input);
        for c in "an".chars() {
            handle_key(&mut e, Key::Char(c));
        }
        let names: Vec<_> = e
            .file_tree
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(names, vec!["banana.txt"], "only banana.txt contains \"an\"");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn backspace_narrows_the_filter_back_toward_showing_more() {
        let root = project(&["apple.txt", "banana.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('b'));
        assert_eq!(e.file_tree.as_ref().unwrap().nodes.len(), 1);
        handle_key(&mut e, Key::Backspace);
        assert_eq!(
            e.file_tree.as_ref().unwrap().nodes.len(),
            2,
            "clearing the query back to empty should show everything again"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn esc_clears_the_filter_and_enter_keeps_it() {
        let root = project(&["apple.txt", "banana.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('b'));
        handle_key(&mut e, Key::Esc);
        assert!(e.file_tree.as_ref().unwrap().filter.is_empty());
        assert!(!e.file_tree.as_ref().unwrap().filter_input);
        assert_eq!(
            e.file_tree.as_ref().unwrap().nodes.len(),
            2,
            "Esc must clear the filter, not just stop typing"
        );

        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('b'));
        handle_key(&mut e, Key::Enter);
        assert!(
            !e.file_tree.as_ref().unwrap().filter_input,
            "Enter should stop typing"
        );
        assert_eq!(
            e.file_tree.as_ref().unwrap().nodes.len(),
            1,
            "Enter must keep the filter applied, unlike Esc"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn filter_input_intercepts_letters_that_are_normally_tree_commands() {
        // While typing a filter, 'd' must become part of the query, not
        // arm a delete -- otherwise every filename containing a command
        // letter would be untypeable.
        let root = project(&["deleteme.txt"], &[]);
        let mut e = editor_with_tree(&root);
        handle_key(&mut e, Key::Char('/'));
        handle_key(&mut e, Key::Char('d'));
        assert_eq!(e.file_tree.as_ref().unwrap().filter, "d");
        assert!(e.file_tree.as_ref().unwrap().confirm_delete.is_none());
        assert!(root.join("deleteme.txt").exists());
        std::fs::remove_dir_all(root).ok();
    }
}
