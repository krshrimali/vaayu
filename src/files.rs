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
    atomic_write_mode(path, bytes, private, None)
}
pub fn atomic_write_mode(
    path: &Path,
    bytes: &[u8],
    private: bool,
    mode: Option<std::fs::Permissions>,
) -> anyhow::Result<()> {
    if private {
        anyhow::ensure!(
            !std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
            "Private file is a symlink"
        );
    }
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
        if let Some(mode) = mode {
            f.set_permissions(mode)?;
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

/// Hold an OS lock for the complete read/check/replace transaction. The lock
/// inode stays in place; dropping the descriptor releases it after crashes too.
pub struct StoreLock(std::fs::File);
impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
pub fn private_lock(dir: &Path, name: &str) -> anyhow::Result<StoreLock> {
    if !dir.exists() {
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        match b.create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::ensure!(
        !std::fs::symlink_metadata(dir)?.file_type().is_symlink(),
        "Private directory is a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = dir.join(name);
    anyhow::ensure!(
        !std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()),
        "Lock is a symlink"
    );
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let f = o.open(path)?;
    f.try_lock()
        .map_err(|e| anyhow::anyhow!("Private store is busy: {e}"))?;
    Ok(StoreLock(f))
}
