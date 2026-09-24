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
    /// `Some("claude")`/`Some("codex")`/... for a long-lived agent
    /// session started via `:claude`/`:codex`/`:agent <name>`; `None`
    /// for a plain `:terminal`/lazygit session. Lets `toggle_agent_session`
    /// find "the claude session" (if any) among every currently running
    /// `PtySession` without a separate, easy-to-desync tracking list.
    pub agent_kind: Option<String>,
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
            agent_kind: None,
        })
    }

    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
    }

    /// Writes `text` as a bracketed paste (`\x1b[200~...\x1b[201~`), the
    /// same wrapping a real terminal emulator already adds around a
    /// keyboard paste -- without it, a multi-line string written via
    /// plain `write_input` has each of its own newlines land on the
    /// child exactly like the user pressing Enter after every line,
    /// which a line-buffered shell executes one command at a time and a
    /// chat-style CLI would submit one message at a time instead of
    /// receiving the whole block as one turn. Readline/rustyline-based
    /// programs (most interactive CLIs, including a real agent session)
    /// already understand this sequence; one that doesn't just sees an
    /// unrecognized escape sequence around otherwise-unchanged text,
    /// degrading to the same per-line behavior `write_input` alone
    /// would have had -- never worse.
    pub fn write_pasted_input(&mut self, text: &str) {
        let _ = self.writer.write_all(b"\x1b[200~");
        let _ = self.writer.write_all(text.as_bytes());
        let _ = self.writer.write_all(b"\x1b[201~");
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
        // Recover from a poisoned lock (a panic in the reader thread while it
        // held the parser) instead of unwrapping -- the screen data is still
        // readable, and a render must never take down the whole editor.
        let p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        f(p.screen())
    }

    /// Terminates the child and waits for the reader thread to notice EOF
    /// and exit, so no process or thread outlives the pane that owned it.
    ///
    /// Guaranteed not to hang `:q`/`:qa`, even when a detached grandchild
    /// (e.g. `setsid sleep 300 &`) keeps the PTY slave open after the
    /// direct child is killed: such a grandchild lives in its own session,
    /// so no process-group kill of the child would reach it, and the
    /// reader's `read()` on the master would otherwise block forever
    /// waiting for an EOF that never comes. We (1) close our own master
    /// handle before joining so the common case (no grandchild) sees EOF
    /// promptly, and (2) bound the join with a short timeout, abandoning a
    /// still-stuck reader thread rather than blocking editor exit on it.
    /// An abandoned reader is harmless: it holds one blocked `read()` on a
    /// dead pane's pty and exits on its own if the grandchild ever closes
    /// the slave; the process is exiting regardless.
    pub fn shutdown(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Drop our master handle before the join (see above): in the
        // ordinary case this is enough for the reader to observe EOF and
        // return on its own.
        drop(self.master);
        if let Some(h) = self.reader_handle.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = h.join();
                let _ = tx.send(());
            });
            // If the reader hasn't finished within the grace period a
            // grandchild is holding the slave open; stop waiting so exit
            // can proceed instead of hanging forever in join().
            let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
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

    /// `:termsend` / REPL: write `text` (with a trailing newline so it runs) to
    /// a terminal — the one focused in the active window if any, else the most
    /// recently opened. Sends without leaving the current buffer.
    pub fn send_to_terminal(&mut self, text: &str) {
        let target = self
            .active_terminal_id()
            .or_else(|| self.terminals.last().map(|t| t.id));
        let Some(id) = target else {
            self.set_message("No terminal to send to — :terminal opens one");
            return;
        };
        let Some(term) = self.terminals.iter_mut().find(|t| t.id == id) else {
            self.set_message("No terminal to send to — :terminal opens one");
            return;
        };
        let mut payload = text.trim_end_matches('\n').to_string();
        payload.push('\n');
        term.write_input(payload.as_bytes());
        self.set_message("Sent to terminal");
    }

    /// `:termsend`: send the current line, or (in Visual/line ranges) the given
    /// inclusive line range, to the terminal.
    pub fn termsend_lines(&mut self, from: usize, to: usize) {
        let (lo, hi) = (from.min(to), from.max(to));
        let last = self.buf().line_count().saturating_sub(1);
        let mut text = String::new();
        for l in lo..=hi.min(last) {
            text.push_str(&self.buf().line_text(l));
            text.push('\n');
        }
        self.send_to_terminal(&text);
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
                if self.attach_terminal_pane(id, false) {
                    self.set_message(format!("Terminal: {title} (Esc for pane navigation)"));
                } else {
                    // Pane limit hit (split_window already set the message);
                    // roll the just-spawned shell back so it isn't leaked.
                    self.shutdown_terminal(id);
                }
            }
            Err(e) => self.set_message(format!("Could not start terminal: {e}")),
        }
    }

    /// `:lazygit`/`,gl`: spawns `lazygit` in a new split, reusing
    /// `open_terminal`'s exact machinery -- external, optional (Scope
    /// rule 5), and fails visibly (a missing binary surfaces as a normal
    /// spawn error, the same as any other external tool this editor
    /// shells out to) rather than blocking normal editing.
    pub fn open_lazygit(&mut self) {
        let rows = self.screen_rows.max(1) as u16;
        let cols = self.screen_cols.max(1) as u16;
        match PtySession::spawn(&["lazygit".to_string()], &self.project_root, rows, cols) {
            Ok(session) => {
                let id = session.id;
                self.terminals.push(session);
                if self.attach_terminal_pane(id, false) {
                    self.set_message("lazygit (Esc for pane navigation)");
                } else {
                    self.shutdown_terminal(id);
                }
            }
            Err(e) => self.set_message(format!("Could not start lazygit: {e}")),
        }
    }

    /// Reattaches an already-running, currently pane-less terminal (see
    /// `detach_window`) into a new split in the current tab -- the same
    /// spawn-time split/mode-switch shape `open_terminal`/`open_lazygit`
    /// use, minus the spawn itself. Resizes it to the current screen
    /// size on the way back in, in case a resize happened while it was
    /// detached (nothing was resizing it -- `poll_terminals`/window
    /// layout only resize *attached* panes).
    fn reattach_terminal(&mut self, id: u64, vertical: bool) {
        if !self.attach_terminal_pane(id, vertical) {
            return; // pane limit; leave the session detached rather than hijack
        }
        let rows = self.screen_rows.max(1) as u16;
        let cols = self.screen_cols.max(1) as u16;
        if let Some(pty) = self.terminals.iter_mut().find(|p| p.id == id) {
            pty.resize(rows, cols);
        }
    }

    /// Split (vertical/horizontal) and attach terminal `id` to the new pane in
    /// Terminal mode. Returns false *without* attaching when the split hit the
    /// pane limit, so callers never repurpose the focused editing pane (and can
    /// roll a freshly-spawned session back).
    fn attach_terminal_pane(&mut self, id: u64, vertical: bool) -> bool {
        let before = self.windows.len();
        self.split_window(vertical, false);
        if self.windows.len() <= before {
            return false;
        }
        if let Some(w) = self.windows.get_mut(self.active_window) {
            w.terminal = Some(id);
        }
        self.mode = crate::mode::Mode::Terminal;
        true
    }

    /// `:claude`/`:codex`/`:agent <name>`: starts, reattaches, or
    /// detaches (toggles) a named long-lived agent terminal session --
    /// Phase 6 item 1's "select/toggle/detach" bar. "Interrupt" needs no
    /// separate code path: a real pty's own line discipline already
    /// turns a Ctrl-C typed in Terminal mode into a genuine SIGINT for
    /// whatever the session is running, the same as it would for any
    /// other terminal (see `ctrl_c_sends_sigint_to_the_foreground_child`).
    /// "Per-tab association" falls out of the existing tab/window model
    /// once detach-without-kill exists at all: a reattached session only
    /// gets a window pointer in *this* tab, so switching tabs naturally
    /// hides it (without killing it) exactly like detaching would.
    pub fn toggle_agent_session(&mut self, kind: &str) {
        let existing = self
            .terminals
            .iter()
            .find(|p| p.agent_kind.as_deref() == Some(kind))
            .map(|p| p.id);
        if let Some(id) = existing {
            if let Some(idx) = self.windows.iter().position(|w| w.terminal == Some(id)) {
                self.active_window = idx;
                self.detach_window();
                self.set_message(format!("{kind} detached (still running)"));
            } else {
                self.reattach_terminal(id, false);
                self.set_message(format!("{kind} reattached (Esc for pane navigation)"));
            }
            return;
        }
        self.spawn_agent_session(kind, false);
    }

    /// Spawn a fresh agent CLI session for `kind` (argv from
    /// `config.agent_commands`, falling back to a bare `kind` on PATH) into a
    /// new split -- vertical for a right-hand sidebar, horizontal otherwise --
    /// attach it to the new pane, and focus it in Terminal mode. Returns the
    /// new session id, or `None` if the CLI could not be started (a message is
    /// set either way). Shared by `toggle_agent_session` and `ensure_ai_sidebar`.
    fn spawn_agent_session(&mut self, kind: &str, vertical: bool) -> Option<u64> {
        let cmd = self
            .config
            .agent_commands
            .get(kind)
            .cloned()
            .unwrap_or_else(|| vec![kind.to_string()]);
        let rows = self.screen_rows.max(1) as u16;
        let cols = self.screen_cols.max(1) as u16;
        match PtySession::spawn(&cmd, &self.project_root, rows, cols) {
            Ok(mut session) => {
                session.agent_kind = Some(kind.to_string());
                let id = session.id;
                self.terminals.push(session);
                if self.attach_terminal_pane(id, vertical) {
                    self.set_message(format!("{kind} (Esc for pane navigation)"));
                    Some(id)
                } else {
                    self.shutdown_terminal(id);
                    None
                }
            }
            Err(e) => {
                self.set_message(format!("Could not start {kind}: {e}"));
                None
            }
        }
    }

    /// Ensure a `claude` agent session is attached in this tab so the AI-prompt
    /// feature has somewhere to send to: reuse it if already attached, reattach
    /// it (into a right-hand vertical split) if detached, else spawn `claude` in
    /// a right-hand vertical split. Returns whether a session is now available.
    pub fn ensure_ai_sidebar(&mut self) -> bool {
        let existing = self
            .terminals
            .iter()
            .find(|p| p.agent_kind.as_deref() == Some("claude"))
            .map(|p| p.id);
        if let Some(id) = existing {
            if !self.windows.iter().any(|w| w.terminal == Some(id)) {
                self.reattach_terminal(id, true);
            }
            return true;
        }
        self.spawn_agent_session("claude", true).is_some()
    }

    /// `:agents`: every currently running agent session (started via
    /// `:claude`/`:codex`/`:agent`), attached-in-this-tab or detached;
    /// Enter reattaches the selected one here (or just focuses it, if
    /// it's already attached in this tab).
    pub fn list_agent_sessions(&mut self) {
        let entries: Vec<crate::results::Entry> = self
            .terminals
            .iter()
            .filter_map(|p| {
                let kind = p.agent_kind.as_deref()?;
                let attached = self.windows.iter().any(|w| w.terminal == Some(p.id));
                let mut e = crate::results::Entry::text(format!(
                    "{kind} — {}",
                    if attached {
                        "attached in this tab"
                    } else {
                        "detached"
                    }
                ));
                e.action = Some(serde_json::json!({"_vaayu_agent_reattach": p.id}));
                Some(e)
            })
            .collect();
        if entries.is_empty() {
            self.set_message("No agent sessions running");
            return;
        }
        self.show_results(crate::results::Results::new(
            "Agent sessions — Enter attaches one here",
            entries,
        ));
    }

    /// Dispatches a `_vaayu_agent_reattach`-tagged entry from
    /// `list_agent_sessions`: reattaches `id` here, or just focuses it
    /// if it's already attached in this tab (reattaching an
    /// already-attached session would otherwise open a second, redundant
    /// pane onto the same output).
    pub fn reattach_agent_session(&mut self, id: u64) {
        if let Some(idx) = self.windows.iter().position(|w| w.terminal == Some(id)) {
            self.active_window = idx;
            self.mode = crate::mode::Mode::Terminal;
            return;
        }
        self.reattach_terminal(id, false);
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
        let mut just_exited: Option<(String, Option<u32>)> = None;
        for pty in &mut self.terminals {
            // Only the *transition* to exited counts as a change: `poll_exit`
            // caches and returns `Some` forever after the child dies, so
            // checking `is_some()` every tick would keep the main loop redrawing
            // at 100% CPU while a finished terminal pane stays open.
            let was_running = pty.exited.is_none();
            if let Some(code) = pty.poll_exit() {
                if was_running {
                    changed = true;
                    just_exited = Some((pty.title.clone(), code));
                }
            }
            let rev = pty.revision.load(Ordering::Relaxed);
            if rev != pty.seen_revision {
                pty.seen_revision = rev;
                changed = true;
            }
        }
        if let Some((title, code)) = just_exited {
            let status = code
                .map(|c| format!("exited ({c})"))
                .unwrap_or_else(|| "exited".into());
            // Replaces the stale "Terminal: … (Esc for pane navigation)" line
            // and tells the user the child is gone and how to close the pane.
            self.set_message(format!("{title} {status} -- Esc, then Ctrl-W c to close"));
        }
        changed
    }
}

/// Dispatches a keystroke while focused on a terminal pane in Terminal
/// mode: Esc leaves to Normal (still focused on the pane, for navigation
/// or `:close`); everything else is forwarded to the child as raw bytes.
pub fn handle_terminal_mode(ed: &mut crate::editor::Editor, key: crate::key::Key) {
    use crate::key::Key;
    // jk-escape (when enabled): a buffered `j` followed by `k` leaves Terminal
    // mode for Normal (pane navigation / scrolling), like in Insert mode; any
    // other key flushes the `j` to the child first, then falls through.
    if ed.config.jk_escape && ed.pending_jk.take().is_some() {
        if key == Key::Char('k') {
            ed.mode = crate::mode::Mode::Normal;
            return;
        }
        write_active_terminal(ed, b"j");
        // fall through to send `key` below
    } else if ed.config.jk_escape && key == Key::Char('j') {
        // Buffer the `j`; the main loop flushes it after `timeoutlen_ms` (via
        // flush_pending_jk) if no `k` follows, so a lone `j` still reaches the
        // child.
        ed.pending_jk = Some(std::time::Instant::now());
        return;
    }
    if key == Key::Esc {
        ed.mode = crate::mode::Mode::Normal;
        return;
    }
    // Ctrl-W is the window-command prefix even from Terminal mode, so pane
    // navigation (`Ctrl-W h/j/k/l/w/c` ...) works without pressing Esc first.
    // The next key is handled as a window command by `feed_key`.
    if key == Key::Ctrl('w') {
        ed.mode = crate::mode::Mode::Normal;
        ed.window_prefix = true;
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

/// Write raw bytes to the terminal focused in the active window, if any.
fn write_active_terminal(ed: &mut crate::editor::Editor, bytes: &[u8]) {
    if let Some(id) = ed.active_terminal_id() {
        if let Some(pty) = ed.terminals.iter_mut().find(|p| p.id == id) {
            pty.write_input(bytes);
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
    fn write_pasted_input_wraps_the_text_in_bracketed_paste_markers() {
        // A bare `cat` (no readline, no bracketed-paste awareness) just
        // echoes every byte back verbatim, including the markers
        // themselves -- confirming this function actually puts them on
        // the wire around the text, not that a specific child consumes
        // them (that half depends entirely on the child's own input
        // handling: a readline-based program strips them internally and
        // would show nothing extra at all; a plain shell has no concept
        // of them and, like `cat` here, would echo them as literal
        // bytes -- either way, strictly no worse than `write_input`
        // alone would have been for a program that doesn't understand
        // this convention).
        let dir = std::env::temp_dir();
        let mut s = PtySession::spawn(&["/bin/cat".into()], &dir, 24, 80).unwrap();
        s.write_pasted_input("PASTE_MARKER_TEXT\n");
        let start = std::time::Instant::now();
        loop {
            if s.with_screen(|scr| {
                let c = scr.contents();
                c.contains("PASTE_MARKER_TEXT") && c.contains("200~") && c.contains("201~")
            }) {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 5,
                "pasted input was never echoed back"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        s.shutdown();
    }

    #[test]
    fn ctrl_c_sends_sigint_to_the_foreground_child() {
        // A real pty's own line discipline (ISIG) generates SIGINT for a
        // raw 0x03 byte, entirely independent of anything this module
        // does -- confirming that empirically here, rather than assuming
        // it, since it's the one thing Terminal-mode `,gl`/`:terminal`
        // input forwarding depends on for interrupt-without-kill to work
        // at all.
        let dir = std::env::temp_dir();
        let mut s = PtySession::spawn(&["/bin/sleep".into(), "30".into()], &dir, 24, 80).unwrap();
        s.write_input(&[0x03]);
        let start = std::time::Instant::now();
        loop {
            if s.poll_exit().is_some() {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 5,
                "sleep 30 should have been interrupted by SIGINT almost \
                 immediately, not left running toward its full duration"
            );
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
