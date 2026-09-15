//! File identity and transactional persistence shared by buffers and review notes.
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(1);
pub fn identity(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c.as_os_str()),
        }
    }
    // Resolve an existing parent even when the file does not yet exist.
    if let (Some(parent), Some(name)) = (out.parent(), out.file_name()) {
        if let Ok(p) = parent.canonicalize() {
            return p.join(name);
        }
    }
    out
}
pub fn uri(path: &Path) -> String {
    url::Url::from_file_path(identity(path))
        .expect("absolute path")
        .to_string()
}
pub fn from_uri(uri: &str) -> Option<PathBuf> {
    url::Url::parse(uri)
        .ok()?
        .to_file_path()
        .ok()
        .map(|p| identity(&p))
}
pub fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> anyhow::Result<()> {
    let path = identity(path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent directory"))?;
    let temp = parent.join(format!(
        ".vaayu-save-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if private { 0o600 } else { 0o666 });
        }
        let mut f = options.open(&temp)?;
        if !private {
            if let Ok(meta) = std::fs::metadata(&path) {
                f.set_permissions(meta.permissions())?;
            }
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&temp, &path)?;
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
