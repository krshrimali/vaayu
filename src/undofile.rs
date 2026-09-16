//! Persistent undo across restarts. On a successful `:w`, the current
//! buffer's undo history (capped) is written to a private per-project
//! store keyed by the file's absolute path. On open, it is restored only
//! if the just-loaded content's hash matches what was recorded at save
//! time -- undo history is never replayed against content that changed on
//! disk (externally, or via a different Vaayu process) since the save
//! that wrote it. This is a convenience, not a source of truth: any
//! failure to read, parse or write it is silently ignored.
use crate::buffer::Buffer;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Most recent undo steps kept; older ones are dropped rather than growing
/// the file without bound on a long editing session.
const MAX_ENTRIES: usize = 50;
/// Total serialized size cap, trimming oldest-first -- a handful of
/// snapshots of a huge file could otherwise dwarf the file itself.
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Entry {
    text: String,
    cursor: (usize, usize),
}

#[derive(Serialize, Deserialize)]
struct Store {
    content_hash: u64,
    entries: Vec<Entry>,
}

fn hash_text(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn key_for(path: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    format!("{:016x}.json", h.finish())
}

fn ensure_dir(root: &Path) -> anyhow::Result<PathBuf> {
    let vaayu = root.join(".vaayu");
    let undo = vaayu.join("undo");
    for dir in [&vaayu, &undo] {
        if !dir.exists() {
            let mut b = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                b.mode(0o700);
            }
            if let Err(e) = b.create(dir) {
                if !dir.is_dir() {
                    return Err(e.into());
                }
            }
        }
        anyhow::ensure!(
            !std::fs::symlink_metadata(dir)?.file_type().is_symlink(),
            "private directory must not be a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let gitignore = vaayu.join(".gitignore");
    if !gitignore.exists() {
        crate::files::atomic_write(&gitignore, b"*\n", true)?;
    }
    Ok(undo)
}

/// Writes `buf`'s undo history (if it has one) under `root`. Best effort:
/// any I/O or permission failure is dropped, since this is a convenience
/// feature, never a save the user is relying on to not lose work.
pub fn save(root: &Path, buf: &Buffer) {
    let Some(path) = buf.path.clone() else {
        return;
    };
    let Ok(dir) = ensure_dir(root) else {
        return;
    };
    let file_path = dir.join(key_for(&path));
    let snapshots = buf.undo_snapshots();
    if snapshots.is_empty() {
        let _ = std::fs::remove_file(&file_path);
        return;
    }
    let mut entries: Vec<Entry> = snapshots
        .into_iter()
        .rev()
        .take(MAX_ENTRIES)
        .map(|(text, cursor)| Entry { text, cursor })
        .collect();
    entries.reverse();
    let mut total: usize = entries.iter().map(|e| e.text.len()).sum();
    while total > MAX_BYTES && entries.len() > 1 {
        total -= entries.remove(0).text.len();
    }
    let store = Store {
        content_hash: hash_text(&buf.rope.to_string()),
        entries,
    };
    if let Ok(json) = serde_json::to_vec(&store) {
        let _ = crate::files::atomic_write(&file_path, &json, true);
    }
}

/// Restores `buf`'s undo history from `root`'s store, only if the file's
/// content hasn't changed since the history was recorded.
pub fn restore(root: &Path, buf: &mut Buffer) {
    let Some(path) = buf.path.clone() else {
        return;
    };
    let file_path = root.join(".vaayu").join("undo").join(key_for(&path));
    let Ok(bytes) = std::fs::read(&file_path) else {
        return;
    };
    let Ok(store) = serde_json::from_slice::<Store>(&bytes) else {
        return;
    };
    if store.content_hash != hash_text(&buf.rope.to_string()) {
        return;
    }
    buf.restore_undo_snapshots(
        store
            .entries
            .into_iter()
            .map(|e| (e.text, e.cursor))
            .collect(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, editor::Editor, key::Key};
    use std::path::PathBuf;

    fn temp_project() -> PathBuf {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let p = std::env::temp_dir().join(format!(
            "vaayu-undofile-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn editor(text: &str) -> Editor {
        let cfg = Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..Config::default()
        };
        let mut e = Editor::new(cfg);
        e.buf_mut().rope = ropey::Rope::from_str(text);
        e.buf_mut().mark_saved();
        e
    }
    fn keys(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.feed_key(match c {
                '\x1b' => Key::Esc,
                _ => Key::Char(c),
            });
        }
    }

    #[test]
    fn roundtrips_and_restores_undo_history() {
        let root = temp_project();
        let file = root.join("f.txt");
        std::fs::write(&file, "one\n").unwrap();
        let mut e = editor("one\n");
        e.buf_mut().path = Some(file.clone());
        e.buf_mut().mark_saved();
        keys(&mut e, "Atwo\x1b"); // "one" -> "onetwo"
        assert_eq!(e.buf().rope.to_string(), "onetwo\n");
        save(&root, e.buf());

        let mut e2 = editor("onetwo\n");
        e2.buf_mut().path = Some(file.clone());
        restore(&root, e2.buf_mut());
        assert!(e2.buf_mut().undo());
        assert_eq!(e2.buf().rope.to_string(), "one\n");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn refuses_to_restore_against_changed_content() {
        let root = temp_project();
        let file = root.join("f.txt");
        std::fs::write(&file, "one\n").unwrap();
        let mut e = editor("one\n");
        e.buf_mut().path = Some(file.clone());
        e.buf_mut().mark_saved();
        keys(&mut e, "Atwo\x1b");
        save(&root, e.buf());

        // A different on-disk content than what was recorded at save time.
        let mut e2 = editor("something else entirely\n");
        e2.buf_mut().path = Some(file.clone());
        restore(&root, e2.buf_mut());
        assert!(
            !e2.buf_mut().undo(),
            "must not restore against stale content"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn no_undo_history_removes_any_stale_file() {
        let root = temp_project();
        let file = root.join("f.txt");
        std::fs::write(&file, "one\n").unwrap();
        let mut e = editor("one\n");
        e.buf_mut().path = Some(file.clone());
        keys(&mut e, "Atwo\x1b");
        save(&root, e.buf());
        let path = root.join(".vaayu").join("undo").join(key_for(&file));
        assert!(path.exists());

        let mut e_empty = editor("onetwo\n");
        e_empty.buf_mut().path = Some(file.clone());
        save(&root, e_empty.buf()); // no undo history recorded on e_empty
        assert!(!path.exists());
        std::fs::remove_dir_all(root).ok();
    }
}
