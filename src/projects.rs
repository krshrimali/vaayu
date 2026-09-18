//! Recent-projects list for the `:projects` picker source: a small,
//! global (one file under the user's config dir, unlike `session.rs`'s
//! own per-project `.vaayu/session.json`) persisted history of
//! directories vaayu has been launched from -- there's nothing
//! project-specific to record here, just "where have I worked recently."
use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 20;

fn recent_projects_path() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("vaayu")
            .join("recent_projects.json"),
    )
}

pub fn load_recent_projects() -> Vec<PathBuf> {
    match recent_projects_path() {
        Some(path) => load_recent_projects_from(&path),
        None => Vec::new(),
    }
}

/// Moves `root` to the front of the recent list (inserting it if new),
/// capped at `MAX_RECENT`. Best-effort: a write failure (no config dir,
/// read-only filesystem, ...) is silently ignored -- this is a
/// convenience feature, not something worth surfacing an error for on
/// every single launch.
pub fn record_recent_project(root: &Path) {
    if let Some(path) = recent_projects_path() {
        record_recent_project_at(&path, root);
    }
}

/// The real logic, parameterized by the state file's path -- kept
/// separate from `record_recent_project` so tests can exercise real
/// file I/O against a real temp path instead of the user's actual
/// config directory (which `dirs::config_dir()` resolves from `$HOME`/
/// `$XDG_CONFIG_HOME`, both awkward and unsafe to mutate from a test
/// running in parallel with every other `cargo test` in the process).
fn record_recent_project_at(path: &Path, root: &Path) {
    let mut list = load_recent_projects_from(path);
    list.retain(|p| p != root);
    list.insert(0, root.to_path_buf());
    list.truncate(MAX_RECENT);
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&list) {
        let _ = crate::files::atomic_write(path, &bytes, false);
    }
}

fn load_recent_projects_from(path: &Path) -> Vec<PathBuf> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
        .unwrap_or_default()
}

impl crate::editor::Editor {
    /// `:projects`: a Results list of recently-launched-from directories
    /// (excluding the current one -- switching to where you already are
    /// is a no-op), most recent first. Enter switches `project_root` via
    /// `switch_project`.
    pub fn show_recent_projects(&mut self) {
        let entries: Vec<_> = load_recent_projects()
            .into_iter()
            .filter(|p| p != &self.project_root)
            .map(|p| {
                let mut e = crate::results::Entry::text(p.display().to_string());
                e.action = Some(serde_json::json!({"_vaayu_switch_project": p}));
                e
            })
            .collect();
        if entries.is_empty() {
            self.set_message("No other recent projects");
        } else {
            self.show_results(crate::results::Results::new("Recent projects", entries));
        }
    }
    /// Switches the working project root. Only updates in-memory/session
    /// state (`project_root`, the file tree) -- it deliberately doesn't
    /// close or reopen any buffers, unlike a real "switch workspace"
    /// feature would; that's a much bigger scope than a quick "look at a
    /// different project's file tree" jump. Any already-open file tree is
    /// dropped rather than rebuilt in place, since `FileTree` derives its
    /// whole state (root, expanded set, git status) from the root it was
    /// created with; the next `,ft` lazily creates a fresh one at the new
    /// root, the same `get_or_insert_with` path a first-ever tree open
    /// already uses. Deliberately does *not* re-record `root` into the
    /// recent-projects list: it's already there (that's how it showed up
    /// in this picker in the first place), and re-recording on every
    /// switch would touch the real, global `~/.config` state file from
    /// this method -- recording only happens once, at real process
    /// launch (`main.rs`), keeping this method's effect purely in-memory
    /// and safe to exercise freely from a test.
    pub fn switch_project(&mut self, root: std::path::PathBuf) {
        self.project_root = root.clone();
        if let Some(idx) = self.windows.iter().position(|w| w.file_tree) {
            self.active_window = idx;
            self.close_window();
        }
        self.file_tree = None;
        self.set_message(format!("Switched project root to {}", root.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_path() -> PathBuf {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "vaayu-recent-projects-test-{}-{}.json",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[test]
    fn record_creates_the_file_and_lists_the_project() {
        let path = temp_state_path();
        record_recent_project_at(&path, Path::new("/tmp/proj-a"));
        assert_eq!(
            load_recent_projects_from(&path),
            vec![PathBuf::from("/tmp/proj-a")]
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn record_moves_an_existing_entry_to_the_front_without_duplicating() {
        let path = temp_state_path();
        record_recent_project_at(&path, Path::new("/tmp/a"));
        record_recent_project_at(&path, Path::new("/tmp/b"));
        record_recent_project_at(&path, Path::new("/tmp/a")); // re-visit a
        assert_eq!(
            load_recent_projects_from(&path),
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn record_caps_the_list_at_max_recent() {
        let path = temp_state_path();
        for i in 0..MAX_RECENT + 5 {
            record_recent_project_at(&path, &PathBuf::from(format!("/tmp/p{i}")));
        }
        let list = load_recent_projects_from(&path);
        assert_eq!(list.len(), MAX_RECENT);
        // Most recently recorded stays first.
        assert_eq!(list[0], PathBuf::from(format!("/tmp/p{}", MAX_RECENT + 4)));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn load_from_a_missing_file_is_an_empty_list_not_an_error() {
        let path = temp_state_path(); // never created
        assert_eq!(load_recent_projects_from(&path), Vec::<PathBuf>::new());
    }
}
