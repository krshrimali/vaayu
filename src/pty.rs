//! Embedded PTY job supervisor: spawns a real interactive child process
//! (a shell, lazygit, an agent CLI, ...) behind a pseudo-terminal, feeds
//! its output through a real VT100 emulator so it can be rendered inside
//! a pane, and forwards keystrokes to it as raw bytes while in Terminal
//! mode. See `NEOVIM_PARITY_PLAN.md`'s M1.C for the acceptance bar this
//! is scoped against: closing a pane never leaks a child process, resize
//! propagates, output is bounded (via `vt100`'s own scrollback cap), and
//! shutdown is explicit.
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Bounds how much scrollback `vt100` retains per terminal, independent of
/// how much output the child produces -- an unbounded scrollback would let
/// a noisy process (`yes`, a runaway build) grow memory without limit.
const SCROLLBACK_LINES: usize = 5_000;

pub struct PtySession {
    pub id: u64,
    pub title: String,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    reader_handle: Option<std::thread::JoinHandle<()>>,
    last_size: (u16, u16),
    exited: Option<Option<u32>>,
    /// Bumped by the reader thread on every chunk of output processed, so
    /// the main loop can tell a redraw is needed without polling the PTY
    /// itself -- the same revision-comparison pattern already used for
    /// LSP events (`Editor::poll_lsp_events`).
    revision: Arc<AtomicU64>,
    seen_revision: u64,
}

impl PtySession {
    pub fn spawn(
        argv: &[String],
        cwd: &std::path::Path,
        rows: u16,
        cols: u16,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(!argv.is_empty(), "empty command");
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system.openpty(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut cmd = portable_pty::CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.cwd(cwd);
        let child = pair.slave.spawn_command(cmd)?;
        // The parent must not hold the slave side open: with it held, the
        // reader thread never sees EOF after the child exits, so it would
        // block forever instead of noticing the process is gone.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LINES)));
        let revision = Arc::new(AtomicU64::new(0));
        let parser_for_reader = parser.clone();
        let revision_for_reader = revision.clone();
        let reader_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut p) = parser_for_reader.lock() {
                            p.process(&buf[..n]);
                        }
                        revision_for_reader.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            title: argv.join(" "),
            master: pair.master,
            writer,
            child,
            parser,
            reader_handle: Some(reader_handle),
            last_size: (rows, cols),
            exited: None,
            revision,
            seen_revision: 0,
        })
    }

    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows == 0 || cols == 0 || (rows, cols) == self.last_size {
            return;
        }
        self.last_size = (rows, cols);
        let _ = self.master.resize(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.screen_mut().set_size(rows, cols);
        }
    }

    /// Polls whether the child has exited without blocking. Once it has,
    /// the result is cached (a `Child`'s `try_wait` may not be callable
    /// usefully again after reporting exit on some platforms).
    pub fn poll_exit(&mut self) -> Option<Option<u32>> {
        if self.exited.is_none() {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exited = Some(Some(status.exit_code()));
            }
        }
        self.exited
    }

    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        let p = self.parser.lock().unwrap();
        f(p.screen())
    }

    /// Terminates the child and waits for the reader thread to notice EOF
    /// and exit, so no process or thread outlives the pane that owned it.
    pub fn shutdown(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.reader_handle.take() {
            let _ = h.join();
        }
    }
}

/// Encodes an abstracted `Key` back into the raw bytes a real terminal
/// would have sent for it, for forwarding to a PTY's stdin. Covers the
/// common editing/navigation keys; anything not listed (e.g. function
/// keys, which this editor's `Key` enum doesn't model at all) is dropped
/// rather than guessed at.
fn key_to_bytes(key: crate::key::Key) -> Option<Vec<u8>> {
    use crate::key::Key;
    Some(match key {
        Key::Char(c) | Key::Literal(c) => c.to_string().into_bytes(),
        Key::Enter => b"\r".to_vec(),
        Key::Backspace => b"\x7f".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::BackTab => b"\x1b[Z".to_vec(),
        Key::Left => b"\x1b[D".to_vec(),
        Key::Right => b"\x1b[C".to_vec(),
        Key::Up => b"\x1b[A".to_vec(),
        Key::Down => b"\x1b[B".to_vec(),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::Ctrl(c) => {
            let byte = (c as u8).to_ascii_uppercase() ^ 0x40;
            vec![byte]
        }
        Key::Esc => return None, // handled by the caller to leave Terminal mode
    })
}

impl crate::editor::Editor {
    pub fn active_terminal_id(&self) -> Option<u64> {
        self.windows
            .get(self.active_window)
            .and_then(|w| w.terminal)
    }

    /// `:terminal`: spawns `$SHELL` (or `/bin/sh`) in a new horizontal
    /// split and enters Terminal mode immediately, so typing starts
    /// talking to the shell right away.
    pub fn open_terminal(&mut self) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let rows = self.screen_rows.max(1) as u16;
        let cols = self.screen_cols.max(1) as u16;
        match PtySession::spawn(&[shell], &self.project_root, rows, cols) {
            Ok(session) => {
                let id = session.id;
                let title = session.title.clone();
                self.terminals.push(session);
                self.split_window(false, false);
                if let Some(w) = self.windows.get_mut(self.active_window) {
                    w.terminal = Some(id);
                }
                self.mode = crate::mode::Mode::Terminal;
                self.set_message(format!("Terminal: {title} (Esc for pane navigation)"));
            }
            Err(e) => self.set_message(format!("Could not start terminal: {e}")),
        }
    }

    /// Kills and joins the terminal's reader thread -- called when the
    /// pane hosting it closes, so no process or thread outlives its pane.
    pub fn shutdown_terminal(&mut self, id: u64) {
        if let Some(idx) = self.terminals.iter().position(|p| p.id == id) {
            self.terminals.remove(idx).shutdown();
        }
    }

    /// Called once on the way out of the main loop, regardless of which
    /// `:q`/`:qa`/`ZZ` path was used, so a terminal can never outlive the
    /// editor process itself.
    pub fn shutdown_all_terminals(&mut self) {
        for session in self.terminals.drain(..) {
            session.shutdown();
        }
    }

    /// True if any terminal has produced new output (or changed exit
    /// state) since the last check -- lets the main loop redraw a running
    /// shell's output without waiting for the next keystroke, the same
    /// way `poll_lsp_events` does for language server replies.
    pub fn poll_terminals(&mut self) -> bool {
        let mut changed = false;
        for pty in &mut self.terminals {
            if pty.poll_exit().is_some() {
                changed = true;
            }
            let rev = pty.revision.load(Ordering::Relaxed);
            if rev != pty.seen_revision {
                pty.seen_revision = rev;
                changed = true;
            }
        }
        changed
    }
}

/// Dispatches a keystroke while focused on a terminal pane in Terminal
/// mode: Esc leaves to Normal (still focused on the pane, for navigation
/// or `:close`); everything else is forwarded to the child as raw bytes.
pub fn handle_terminal_mode(ed: &mut crate::editor::Editor, key: crate::key::Key) {
    if key == crate::key::Key::Esc {
        ed.mode = crate::mode::Mode::Normal;
        return;
    }
    let Some(id) = ed.active_terminal_id() else {
        ed.mode = crate::mode::Mode::Normal;
        return;
    };
    if let Some(bytes) = key_to_bytes(key) {
        if let Some(pty) = ed.terminals.iter_mut().find(|p| p.id == id) {
            pty.write_input(&bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawns_runs_and_shuts_down_cleanly() {
        let dir = std::env::temp_dir();
        let mut s = PtySession::spawn(
            &["/bin/sh".into(), "-c".into(), "echo hi_from_pty".into()],
            &dir,
            24,
            80,
        )
        .unwrap();
        let start = std::time::Instant::now();
        loop {
            if s.with_screen(|scr| scr.contents().contains("hi_from_pty")) {
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "pty output never arrived");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let start = std::time::Instant::now();
        loop {
            if s.poll_exit().is_some() {
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "child never reported exit");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        s.shutdown();
    }

    #[test]
    fn resize_updates_parser_screen_size() {
        let dir = std::env::temp_dir();
        let mut s = PtySession::spawn(&["/bin/sh".into()], &dir, 24, 80).unwrap();
        s.resize(30, 100);
        let (rows, cols) = s.with_screen(|scr| scr.size());
        assert_eq!((rows, cols), (30, 100));
        s.shutdown();
    }

    #[test]
    fn write_input_reaches_the_child() {
        let dir = std::env::temp_dir();
        let mut s = PtySession::spawn(&["/bin/cat".into()], &dir, 24, 80).unwrap();
        s.write_input(b"echo_back\n");
        let start = std::time::Instant::now();
        loop {
            if s.with_screen(|scr| scr.contents().contains("echo_back")) {
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "input was never echoed back");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        s.shutdown();
    }

    #[test]
    fn kill_leaves_no_process_behind() {
        let dir = std::env::temp_dir();
        let s = PtySession::spawn(&["/bin/sleep".into(), "30".into()], &dir, 24, 80).unwrap();
        let pid = s.child.process_id();
        s.shutdown();
        if let Some(pid) = pid {
            std::thread::sleep(std::time::Duration::from_millis(100));
            // `kill -0` checks liveness without sending a real signal.
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .is_ok_and(|s| s.success());
            assert!(
                !alive,
                "child process {pid} is still running after shutdown"
            );
        }
    }
}
