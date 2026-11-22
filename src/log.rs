//! Persistent message log (`persist_messages`).
//!
//! Every new status-line message is appended, one line each, to
//! `vaayu.log` in the per-user state directory
//! (`~/.local/state/vaayu/` on Linux, the data directory elsewhere). The
//! file rotates by size into `vaayu.log.1` .. `vaayu.log.N`. `:Log` reads
//! them all back, oldest first.
//!
//! Line format: `<rfc3339 timestamp> [pid <pid>] [<tag>] <text>`, with
//! backslashes and newlines in the text escaped so each entry stays on one
//! line. A write error disables logging for the session; it never breaks
//! editing.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Default log file location: `<state dir>/vaayu/vaayu.log`, falling back
/// to the data directory on platforms without an XDG state dir.
pub fn log_path() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .map(|d| d.join("vaayu").join("vaayu.log"))
}

pub struct Logger {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    /// Rotate once the current file grows past this many bytes; 0 never rotates.
    max_bytes: u64,
    /// Rotated files kept beside the current one.
    keep: usize,
}

impl Logger {
    /// Opens (creating if needed) the log at `path` for appending, with a
    /// private parent directory and a 0600 file.
    pub fn open(path: PathBuf, max_kb: u64, keep: usize) -> anyhow::Result<Logger> {
        if let Some(dir) = path.parent() {
            crate::review_store::ensure_private_dir(dir)?;
        }
        let mut logger = Logger {
            path,
            file: None,
            size: 0,
            max_bytes: max_kb.saturating_mul(1024),
            keep,
        };
        logger.reopen()?;
        logger.rotate_if_needed()?;
        Ok(logger)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn reopen(&mut self) -> anyhow::Result<()> {
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&self.path)?;
        self.size = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    /// Appends one `[tag] text` entry.
    pub fn write(&mut self, tag: &str, text: &str) -> anyhow::Result<()> {
        let line = format_line(tag, text);
        let file = match self.file.as_mut() {
            Some(f) => f,
            None => anyhow::bail!("log file is not open"),
        };
        file.write_all(line.as_bytes())?;
        self.size += line.len() as u64;
        self.rotate_if_needed()
    }

    fn rotate_if_needed(&mut self) -> anyhow::Result<()> {
        if self.max_bytes == 0 || self.size <= self.max_bytes {
            return Ok(());
        }
        self.file = None;
        if self.keep == 0 {
            std::fs::remove_file(&self.path)?;
        } else {
            let _ = std::fs::remove_file(rotated(&self.path, self.keep));
            for n in (1..self.keep).rev() {
                let from = rotated(&self.path, n);
                if from.exists() {
                    std::fs::rename(&from, rotated(&self.path, n + 1))?;
                }
            }
            std::fs::rename(&self.path, rotated(&self.path, 1))?;
        }
        self.reopen()
    }

    /// Every logged line across the rotated files and the current one,
    /// oldest first.
    pub fn read_all(&self) -> Vec<String> {
        read_all(&self.path, self.keep)
    }
}

/// Where the panic hook writes, set while the editor's own log is open.
/// A global because the hook cannot reach the `Editor`.
static PANIC_LOG: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Points the panic hook at `path`, or turns panic logging off with `None`.
pub fn set_panic_log(path: Option<PathBuf>) {
    if let Ok(mut slot) = PANIC_LOG.lock() {
        *slot = path;
    }
}

/// Most backtrace lines written per panic.
const PANIC_BACKTRACE_LINES: usize = 80;

/// Installs a panic hook that appends `[panic]` lines (the message, then
/// the backtrace one frame line each) to the log set by `set_panic_log`,
/// then runs the previous hook. Covers background threads too.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = PANIC_LOG.try_lock().ok().and_then(|slot| slot.clone());
        if let Some(path) = path {
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string();
            let mut lines = vec![format!("thread '{thread}' {info}")];
            let backtrace = std::backtrace::Backtrace::force_capture().to_string();
            lines.extend(
                backtrace
                    .lines()
                    .take(PANIC_BACKTRACE_LINES)
                    .map(|l| format!("  {}", l.trim_end())),
            );
            let _ = append_lines(&path, "panic", &lines);
        }
        previous(info);
    }));
}

/// One-off append without a `Logger` (the panic hook's path).
fn append_lines(path: &Path, tag: &str, lines: &[String]) -> std::io::Result<()> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    let text: String = lines.iter().map(|l| format_line(tag, l)).collect();
    file.write_all(text.as_bytes())
}

fn rotated(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// Reads `path.N` .. `path.1` then `path`, oldest first. Missing files are
/// skipped.
pub fn read_all(path: &Path, keep: usize) -> Vec<String> {
    let mut files: Vec<PathBuf> = (1..=keep).rev().map(|n| rotated(path, n)).collect();
    files.push(path.to_path_buf());
    files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|s| s.lines().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

fn format_line(tag: &str, text: &str) -> String {
    let ts = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    format!("{ts} [pid {}] [{tag}] {escaped}\n", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vaayu-log-test-{name}-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_tagged_single_line_entries() {
        let dir = temp_dir("write");
        let path = dir.join("vaayu.log");
        let mut log = Logger::open(path.clone(), 1024, 3).unwrap();
        log.write("msg", "hello").unwrap();
        log.write("msg", "two\nlines").unwrap();
        let lines = log.read_all();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("[msg] hello"), "{lines:?}");
        assert!(lines[0].contains(&format!("[pid {}]", std::process::id())));
        assert!(lines[1].ends_with("[msg] two\\nlines"), "{lines:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn log_file_and_dir_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perm");
        let path = dir.join("vaayu.log");
        let mut log = Logger::open(path.clone(), 1024, 3).unwrap();
        log.write("msg", "x").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir), 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotates_past_the_size_limit_and_keeps_n_files() {
        let dir = temp_dir("rotate");
        let path = dir.join("vaayu.log");
        // 1 KB limit: each ~600-byte entry forces a rotation every other write.
        let mut log = Logger::open(path.clone(), 1, 2).unwrap();
        let big = "x".repeat(600);
        for i in 0..10 {
            log.write("msg", &format!("{i} {big}")).unwrap();
        }
        assert!(rotated(&path, 1).exists());
        assert!(rotated(&path, 2).exists());
        assert!(!rotated(&path, 3).exists());
        let lines = log.read_all();
        // Oldest first, and the newest entry is last.
        assert!(lines.last().unwrap().contains("[msg] 9 "));
        let nums: Vec<usize> = lines
            .iter()
            .map(|l| {
                let body = l.split("[msg] ").nth(1).unwrap();
                body.split(' ').next().unwrap().parse().unwrap()
            })
            .collect();
        assert!(nums.windows(2).all(|w| w[0] < w[1]), "{nums:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn panic_lines_append_one_entry_per_line() {
        let dir = temp_dir("panic");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vaayu.log");
        append_lines(&path, "panic", &["boom".into(), "  0: frame".into()]).unwrap();
        let lines = read_all(&path, 0);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("[panic] boom"));
        assert!(lines[1].ends_with("[panic]   0: frame"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn panic_hook_writes_message_and_backtrace_to_the_log() {
        let dir = temp_dir("hook");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vaayu.log");
        install_panic_hook();
        set_panic_log(Some(path.clone()));
        let joined = std::thread::Builder::new()
            .name("log-hook-test".into())
            .spawn(|| panic!("hook test boom"))
            .unwrap()
            .join();
        set_panic_log(None);
        assert!(joined.is_err());
        let lines = read_all(&path, 0);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("[panic] thread 'log-hook-test'")
                    && l.contains("hook test boom")),
            "{lines:?}"
        );
        assert!(lines.len() > 1, "backtrace lines follow the message");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwritable_path_fails_to_open() {
        let dir = temp_dir("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("file");
        std::fs::write(&blocker, "").unwrap();
        // A regular file where the directory should be.
        assert!(Logger::open(blocker.join("vaayu.log"), 1024, 3).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
