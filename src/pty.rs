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

/// Answers the terminal capability probes emitted by interactive shells such
/// as fish. The embedded VT screen parses terminal output but is not an outer
/// terminal, so without these replies shells can wait for a response that
/// never arrives before showing their first prompt.
fn terminal_query_replies(pending: &mut Vec<u8>) -> Vec<Vec<u8>> {
    const ST: &[u8] = b"\x1b\\";
    const FIXED_PROBES: &[&[u8]] = &[
        b"\x1b[0c",
        b"\x1b[c",
        b"\x1b[6n",
        b"\x1b[?u",
        b"\x1b[>0q",
        b"\x1b]11;?\x07",
        b"\x1b]11;?\x1b\\",
    ];
    let mut replies = Vec::new();
    let mut i = 0;
    while i < pending.len() {
        let rest = &pending[i..];
        if rest.starts_with(b"\x1b[0c") || rest.starts_with(b"\x1b[c") {
            replies.push(b"\x1b[?1;2c".to_vec());
            i += if rest.starts_with(b"\x1b[0c") { 4 } else { 3 };
        } else if rest.starts_with(b"\x1b[6n") {
            replies.push(b"\x1b[1;1R".to_vec());
            i += 4;
        } else if rest.starts_with(b"\x1b[?u") {
            replies.push(b"\x1b[?0u".to_vec());
            i += 4;
        } else if rest.starts_with(b"\x1b[>0q") {
            replies.push(b"\x1bP>|vaayu 0.1.0\x1b\\".to_vec());
            i += 5;
        } else if rest.starts_with(b"\x1b]11;?") {
            let end = rest
                .iter()
                .position(|b| *b == 7)
                .or_else(|| rest.windows(2).position(|w| w == ST).map(|p| p + 1));
            if let Some(end) = end {
                replies.push(b"\x1b]11;rgb:2020/2020/2020\x1b\\".to_vec());
                i += end + 1;
            } else {
                break;
            }
        } else if rest.starts_with(b"\x1bP+q") {
            let Some(end) = rest.windows(2).position(|w| w == ST) else {
                break;
            };
            let requested = &rest[4..end];
            let response = if requested == b"696e646e" {
                Some([b"\x1bP1+r".as_slice(), requested, ST].concat())
            } else if requested == b"71756572792d6f732d6e616d65" {
                let os_hex = os_name()
                    .bytes()
                    .flat_map(|b| [hex_digit(b >> 4), hex_digit(b & 15)])
                    .collect::<Vec<_>>();
                Some([b"\x1bP1+r".as_slice(), requested, b"=", &os_hex, ST].concat())
            } else {
                Some([b"\x1bP0+r".as_slice(), requested, ST].concat())
            };
            if let Some(response) = response {
                replies.push(response);
            }
            i += end + 2;
        } else if rest[0] == 0x1b && FIXED_PROBES.iter().any(|probe| probe.starts_with(rest)) {
            // Keep a query split across two PTY reads so the next chunk can
            // complete it instead of silently losing the partial prefix.
            break;
        } else {
            i += 1;
        }
    }
    pending.drain(..i);
    if pending.len() > 256 {
        pending.drain(..pending.len() - 256);
    }
    replies
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "Darwin",
        "freebsd" => "FreeBSD",
        "openbsd" => "OpenBSD",
        "netbsd" => "NetBSD",
        "dragonfly" => "DragonFly",
        other => other,
    }
}

fn hex_digit(n: u8) -> u8 {
    b"0123456789abcdef"[n as usize]
}

/// How long a freshly-spawned agent CLI must produce NO new output before we
/// treat it as settled at its input prompt and deliver a deferred send. Long
/// enough to sit through a launcher's brief pauses, short enough to feel prompt.
const AGENT_SETTLE: std::time::Duration = std::time::Duration::from_millis(1500);
/// Hard cap on how long a deferred send waits for quiescence, in case the CLI
/// never fully goes quiet (e.g. an animated status line). Generous so a slow
/// corporate agent launcher (auth/proxy/setup) still delivers rather than
/// pasting mid-startup.
const AGENT_MAX_WAIT: std::time::Duration = std::time::Duration::from_millis(60_000);
/// Pause between pasting a prompt and pressing Enter for `ai_auto_submit`.
pub(crate) const AGENT_SUBMIT_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// A prompt queued for a just-spawned agent sidebar, delivered by
/// `flush_pending_agent_send` once the CLI's output goes quiet (it reached its
/// prompt). `last_rev`/`last_change_at` track the output-quiescence timer.
pub struct PendingAgentSend {
    pub id: u64,
    pub text: String,
    /// Press Enter after pasting (`ai_auto_submit`).
    pub submit: bool,
    pub spawned_at: std::time::Instant,
    pub last_rev: u64,
    pub last_change_at: std::time::Instant,
}

/// Result of `ensure_ai_sidebar`: whether the configured agent was already
/// running (send now) or freshly spawned (defer the send past its TUI startup).
pub enum SidebarState {
    /// The CLI could not be started.
    Missing,
    /// An existing, already-initialized agent session (its id) was
    /// reused/reattached -- safe to send to immediately, and the caller must
    /// target this id rather than "any attached agent".
    Reused(u64),
    /// A new session was spawned (still starting); the id to defer a send to.
    Spawned(u64),
}

/// Bounds how much scrollback `vt100` retains per terminal, independent of
/// how much output the child produces -- an unbounded scrollback would let
/// a noisy process (`yes`, a runaway build) grow memory without limit.
const SCROLLBACK_LINES: usize = 5_000;

pub struct PtySession {
    pub id: u64,
    pub title: String,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    reader_handle: Option<std::thread::JoinHandle<()>>,
    last_size: (u16, u16),
    exited: Option<Option<u32>>,
    /// When the child was started, for the `[job]` log line on exit.
    started: std::time::Instant,
    /// Bumped by the reader thread on every chunk of output processed, so
    /// the main loop can tell a redraw is needed without polling the PTY
    /// itself -- the same revision-comparison pattern already used for
    /// LSP events (`Editor::poll_lsp_events`).
    revision: Arc<AtomicU64>,
    seen_revision: u64,
    /// `Some("claude")`/`Some("codex")`/... for a long-lived agent
    /// session started via `:claude`/`:codex`/`:agent <name>`; `None`
    /// for a plain `:terminal`/lazygit session. Lets `toggle_agent_session`
    /// find a named session (if any) among every currently running
    /// `PtySession` without a separate, easy-to-desync tracking list.
    pub agent_kind: Option<String>,
    /// Which session of its kind this is (1, 2, ...), so several sessions of
    /// one agent can run side by side and be told apart: `claude`,
    /// `claude #2`. 0 for a plain terminal.
    pub agent_slot: usize,
    /// Lines of a smooth scroll still to play out (positive = toward older
    /// output), advanced a few lines a frame by `tick_scroll`.
    scroll_pending: isize,
    /// When `tick_scroll` last moved the view.
    scroll_step_at: std::time::Instant,
}

/// The shortest gap between two steps of a smooth scroll (about 120 steps a
/// second at most; the main loop's idle tick paces it in practice).
const SCROLL_FRAME: std::time::Duration = std::time::Duration::from_millis(8);

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
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LINES)));
        let revision = Arc::new(AtomicU64::new(0));
        let parser_for_reader = parser.clone();
        let revision_for_reader = revision.clone();
        let writer_for_reader = writer.clone();
        let reader_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut query_tail = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        query_tail.extend_from_slice(&buf[..n]);
                        let replies = terminal_query_replies(&mut query_tail);
                        if !replies.is_empty() {
                            if let Ok(mut writer) = writer_for_reader.lock() {
                                for reply in replies {
                                    let _ = writer.write_all(&reply);
                                }
                            }
                        }
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
            started: std::time::Instant::now(),
            revision,
            seen_revision: 0,
            agent_kind: None,
            agent_slot: 0,
            scroll_pending: 0,
            scroll_step_at: std::time::Instant::now(),
        })
    }

    /// Writes raw bytes to the child. Input means the user is talking to the
    /// program again, so a scrolled-back view snaps to the live screen first.
    pub fn write_input(&mut self, bytes: &[u8]) {
        self.scroll_pending = 0;
        self.scroll_to_bottom();
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(bytes);
        }
    }

    /// Whether the child has exited, as last observed by `poll_exit`.
    pub fn has_exited(&self) -> bool {
        self.exited.is_some()
    }

    /// The exit code once the child is gone (`Some(None)` when the platform
    /// reported no code).
    pub fn exit_status(&self) -> Option<Option<u32>> {
        self.exited
    }

    /// How many lines the view is scrolled back from the live screen.
    pub fn scroll_offset(&self) -> usize {
        self.with_screen(|s| s.scrollback())
    }

    /// How many lines of history sit above the live screen.
    pub fn scrollback_len(&self) -> usize {
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let at = p.screen().scrollback();
        p.screen_mut().set_scrollback(usize::MAX);
        let len = p.screen().scrollback();
        p.screen_mut().set_scrollback(at);
        len
    }

    /// Scrolls the view `delta` lines into history (positive = older output,
    /// negative = back toward the live screen), clamped to what exists.
    /// Returns the new offset. New output keeps a scrolled view where it is.
    pub fn scroll_by(&mut self, delta: isize) -> usize {
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let at = p.screen().scrollback();
        p.screen_mut()
            .set_scrollback(at.saturating_add_signed(delta));
        p.screen().scrollback()
    }

    /// The name shown for this session: its agent kind, numbered from the
    /// second session of a kind on (`claude #2`), else its command line.
    pub fn display_name(&self) -> String {
        match &self.agent_kind {
            Some(kind) if self.agent_slot > 1 => format!("{kind} #{}", self.agent_slot),
            Some(kind) => kind.clone(),
            None => self.title.clone(),
        }
    }

    /// Queues a smooth scroll of `delta` lines (positive = older output),
    /// played out by `tick_scroll`; the first step is taken at once so the
    /// view answers immediately. Reversing direction drops what was left of
    /// the old one, so the wheel feels direct.
    pub fn scroll_smooth(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }
        if self.scroll_pending.signum() != delta.signum() {
            if self.scroll_pending == 0 {
                // Nothing in flight: the first step may go right away.
                self.scroll_step_at = std::time::Instant::now() - SCROLL_FRAME;
            }
            self.scroll_pending = 0;
        }
        self.scroll_pending = self.scroll_pending.saturating_add(delta);
        self.tick_scroll(std::time::Instant::now());
    }

    /// Advances a smooth scroll by about a third of what is left (at least a
    /// line), at most once per `SCROLL_FRAME`. Returns whether the view
    /// moved. Hitting either end of the history ends the animation.
    pub fn tick_scroll(&mut self, now: std::time::Instant) -> bool {
        if self.scroll_pending == 0
            || now.saturating_duration_since(self.scroll_step_at) < SCROLL_FRAME
        {
            return false;
        }
        self.scroll_step_at = now;
        let left = self.scroll_pending.unsigned_abs();
        let step = (left.div_ceil(3)).max(1) as isize * self.scroll_pending.signum();
        let before = self.scroll_offset();
        let after = self.scroll_by(step);
        if after == before {
            self.scroll_pending = 0;
            return false;
        }
        self.scroll_pending -= step;
        true
    }

    /// The rows in view, preceded by up to `above` rows of history just
    /// above them (oldest first), as cells for link detection. Returns the
    /// rows and how many of them are history.
    pub fn link_rows(&self, above: usize) -> (Vec<crate::termlinks::Row>, usize) {
        fn read(screen: &vt100::Screen, r: u16, cols: u16) -> crate::termlinks::Row {
            let mut cells = Vec::new();
            for c in 0..cols {
                let Some(cell) = screen.cell(r, c) else {
                    continue;
                };
                if cell.is_wide_continuation() || cell.contents().is_empty() {
                    continue;
                }
                let width = if cell.is_wide() { 2 } else { 1 };
                cells.push((cell.contents().to_string(), c, width));
            }
            crate::termlinks::Row {
                cells,
                wrapped: screen.row_wrapped(r),
            }
        }
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let at = p.screen().scrollback();
        let (rows, cols) = p.screen().size();
        let mut history: Vec<crate::termlinks::Row> = Vec::new();
        let mut offset = at;
        let mut want = above;
        while want > 0 {
            let step = want.min(rows.max(1) as usize);
            p.screen_mut().set_scrollback(offset + step);
            let got = p.screen().scrollback().saturating_sub(offset);
            if got == 0 {
                break;
            }
            // The `got` rows that scrolled in at the top sit just above the
            // previous view's first row.
            let chunk: Vec<_> = (0..got).map(|r| read(p.screen(), r as u16, cols)).collect();
            history.splice(0..0, chunk);
            offset += got;
            want -= got;
            if got < step {
                break;
            }
        }
        p.screen_mut().set_scrollback(at);
        let n = history.len();
        let screen = p.screen();
        history.extend((0..rows).map(|r| read(screen, r, cols)));
        (history, n)
    }

    /// Scrolls to the oldest retained line.
    pub fn scroll_to_top(&mut self) {
        self.scroll_pending = 0;
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        p.screen_mut().set_scrollback(usize::MAX);
    }

    /// Returns the view to the live screen.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_pending = 0;
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        if p.screen().scrollback() != 0 {
            p.screen_mut().set_scrollback(0);
        }
    }

    /// The whole retained output (history plus the live screen) as plain
    /// text, rows the program soft-wrapped joined back into one line, and
    /// the line index of the top row currently in view. Used to open the
    /// output in a buffer for search, Visual selection and yanking.
    pub fn capture_text(&self) -> (String, usize) {
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let at = p.screen().scrollback();
        p.screen_mut().set_scrollback(usize::MAX);
        let max = p.screen().scrollback();
        let (rows, cols) = p.screen().size();
        let rows = rows as usize;
        let mut lines: Vec<(String, bool)> = vec![(String::new(), false); max + rows];
        let mut offset = max;
        loop {
            p.screen_mut().set_scrollback(offset);
            let screen = p.screen();
            for (i, text) in screen.rows(0, cols).enumerate() {
                if let Some(slot) = lines.get_mut(max - offset + i) {
                    *slot = (text, screen.row_wrapped(i as u16));
                }
            }
            if offset == 0 {
                break;
            }
            offset = offset.saturating_sub(rows.max(1));
        }
        p.screen_mut().set_scrollback(at);
        drop(p);
        // Join soft-wrapped rows; remember where the current view's top row
        // landed among the joined lines.
        let view_top_row = max - at;
        let mut out: Vec<String> = Vec::new();
        let mut view_line = 0;
        let mut continuing = false;
        for (row, (text, wrapped)) in lines.into_iter().enumerate() {
            if row == view_top_row {
                view_line = out.len().saturating_sub(usize::from(continuing));
            }
            if continuing {
                if let Some(last) = out.last_mut() {
                    last.push_str(&text);
                }
            } else {
                out.push(text);
            }
            continuing = wrapped;
        }
        for line in &mut out {
            let trimmed = line.trim_end().len();
            line.truncate(trimmed);
        }
        while out.len() > 1 && out.last().is_some_and(|l| l.is_empty()) {
            out.pop();
        }
        let view_line = view_line.min(out.len().saturating_sub(1));
        let mut text = out.join("\n");
        text.push('\n');
        (text, view_line)
    }

    /// Whether the program asked for mouse reports, and whether it is on the
    /// alternate screen (a full-screen TUI with no scrollback of its own).
    pub fn mouse_state(&self) -> (bool, bool) {
        self.with_screen(|s| {
            (
                s.mouse_protocol_mode() != vt100::MouseProtocolMode::None,
                s.alternate_screen(),
            )
        })
    }

    /// Whether the program enabled application cursor keys (DECCKM).
    pub fn application_cursor(&self) -> bool {
        self.with_screen(|s| s.application_cursor())
    }

    /// Encodes a mouse event at pane-relative cell (`x`, `y`) the way the
    /// program asked for (X10/UTF-8/SGR), or `None` when it did not ask for
    /// this kind of event at all.
    pub fn mouse_report(
        &self,
        kind: crossterm::event::MouseEventKind,
        modifiers: crossterm::event::KeyModifiers,
        x: usize,
        y: usize,
    ) -> Option<Vec<u8>> {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
        use vt100::{MouseProtocolEncoding, MouseProtocolMode};
        let (mode, encoding) =
            self.with_screen(|s| (s.mouse_protocol_mode(), s.mouse_protocol_encoding()));
        let button_code = |b: MouseButton| match b {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        };
        let (mut code, release) = match kind {
            MouseEventKind::Down(b) => (button_code(b), false),
            MouseEventKind::Up(b) => {
                if mode == MouseProtocolMode::Press {
                    return None;
                }
                (button_code(b), true)
            }
            MouseEventKind::Drag(b) => {
                if !matches!(
                    mode,
                    MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
                ) {
                    return None;
                }
                (button_code(b) + 32, false)
            }
            MouseEventKind::Moved => {
                if mode != MouseProtocolMode::AnyMotion {
                    return None;
                }
                (3 + 32, false)
            }
            MouseEventKind::ScrollUp => (64, false),
            MouseEventKind::ScrollDown => (65, false),
            MouseEventKind::ScrollLeft => (66, false),
            MouseEventKind::ScrollRight => (67, false),
        };
        if mode == MouseProtocolMode::None {
            return None;
        }
        if modifiers.contains(KeyModifiers::SHIFT) {
            code += 4;
        }
        if modifiers.contains(KeyModifiers::ALT) {
            code += 8;
        }
        if modifiers.contains(KeyModifiers::CONTROL) {
            code += 16;
        }
        let (cx, cy) = (x + 1, y + 1);
        Some(match encoding {
            MouseProtocolEncoding::Sgr => {
                format!("\x1b[<{code};{cx};{cy}{}", if release { 'm' } else { 'M' }).into_bytes()
            }
            legacy => {
                // X10-style reports have no release button of their own.
                let code = if release { 3 | (code & !3) } else { code };
                let mut out = b"\x1b[M".to_vec();
                let mut push = |v: usize| {
                    let v = v + 32;
                    if legacy == MouseProtocolEncoding::Utf8 {
                        if let Some(c) = char::from_u32(v as u32) {
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                    } else {
                        out.push(v.min(255) as u8);
                    }
                };
                push(code);
                push(cx);
                push(cy);
                out
            }
        })
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
        self.scroll_to_bottom();
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(b"\x1b[200~");
            let _ = writer.write_all(text.as_bytes());
            let _ = writer.write_all(b"\x1b[201~");
        }
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
            // Shrinking keeps the screen's top rows, which would cut off the
            // newest output (where the cursor is) and lose it for good. Do
            // what terminals do instead: scroll the rows above into history
            // until the cursor row fits, then shrink.
            let (old_rows, _) = p.screen().size();
            let (cy, cx) = p.screen().cursor_position();
            if rows < old_rows && cy >= rows && !p.screen().alternate_screen() {
                let lift = cy - (rows - 1);
                let mut seq = format!("\x1b[{old_rows};1H").into_bytes();
                seq.extend(std::iter::repeat_n(b'\n', lift as usize));
                seq.extend(format!("\x1b[{};{}H", cy - lift + 1, cx + 1).into_bytes());
                p.process(&seq);
            }
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

    /// Monotonic counter of output chunks the reader thread has processed --
    /// used to tell when a freshly-spawned CLI has started drawing.
    pub fn output_revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
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
/// would have sent for it, for forwarding to a PTY's stdin. Used when no
/// raw crossterm event is at hand (macros, tests); live keys go through
/// `encode_key_event`, which also keeps Alt/Shift/Ctrl modifiers.
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
        Key::Ctrl(c) => vec![ctrl_byte(c)?],
        Key::Esc => b"\x1b".to_vec(),
    })
}

/// The control byte for Ctrl+`c` (Ctrl-A = 0x01 ... Ctrl-_ = 0x1f,
/// Ctrl-Space/Ctrl-@ = NUL, Ctrl-? = DEL).
fn ctrl_byte(c: char) -> Option<u8> {
    match c {
        ' ' | '@' | '2' => Some(0),
        '?' | '8' => Some(0x7f),
        '3' => Some(0x1b),
        '4' => Some(0x1c),
        '5' => Some(0x1d),
        '6' => Some(0x1e),
        '7' | '/' => Some(0x1f),
        c if c.is_ascii() => {
            let b = (c as u8).to_ascii_uppercase();
            (0x40..=0x5f).contains(&b).then_some(b ^ 0x40)
        }
        _ => None,
    }
}

/// Encodes a live key event for a PTY, keeping the modifiers `Key` drops:
/// Alt sends an ESC prefix, Shift/Alt+Enter send ESC CR (the newline-in-input
/// chord agent CLIs such as Claude Code accept), modified arrows and
/// function keys use xterm's `CSI 1;m` forms. `app_cursor` follows the
/// program's DECCKM mode for unmodified arrows.
pub(crate) fn encode_key_event(
    ev: crossterm::event::KeyEvent,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let shift = ev.modifiers.contains(KeyModifiers::SHIFT);
    let alt = ev.modifiers.contains(KeyModifiers::ALT);
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let m = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let esc_if = |alt: bool, mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    let cursor = |letter: char| {
        if m > 1 {
            format!("\x1b[1;{m}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |n: u8| {
        if m > 1 {
            format!("\x1b[{n};{m}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    Some(match ev.code {
        KeyCode::Char(c) if ctrl => esc_if(alt, vec![ctrl_byte(c)?]),
        KeyCode::Char(c) => esc_if(alt, c.to_string().into_bytes()),
        KeyCode::Enter if shift || alt => b"\x1b\r".to_vec(),
        KeyCode::Enter if ctrl => b"\n".to_vec(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab if shift => b"\x1b[Z".to_vec(),
        KeyCode::Tab => esc_if(alt, b"\t".to_vec()),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if ctrl => esc_if(alt, vec![0x08]),
        KeyCode::Backspace => esc_if(alt, vec![0x7f]),
        KeyCode::Esc => esc_if(alt, vec![0x1b]),
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let letter = (b'P' + n - 1) as char;
            if m > 1 {
                format!("\x1b[1;{m}{letter}").into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            }
        }
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize]),
        _ => return None,
    })
}

/// The bytes for `key` sent to terminal `id`: the live event's own encoding
/// when the main loop recorded one for this key, else `key_to_bytes`.
fn terminal_bytes(
    ed: &mut crate::editor::Editor,
    id: u64,
    key: crate::key::Key,
) -> Option<Vec<u8>> {
    let app_cursor = ed
        .terminals
        .iter()
        .find(|p| p.id == id)
        .is_some_and(|p| p.application_cursor());
    ed.raw_key
        .take()
        .and_then(|ev| encode_key_event(ev, app_cursor))
        .or_else(|| key_to_bytes(key))
}

pub(crate) fn write_terminal_key(ed: &mut crate::editor::Editor, id: u64, key: crate::key::Key) {
    if let Some(bytes) = terminal_bytes(ed, id, key) {
        if let Some(pty) = ed.terminals.iter_mut().find(|p| p.id == id) {
            pty.write_input(&bytes);
        }
    }
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
        let restarted = self.reap_exited_agent(kind);
        if let Some(id) = self.current_agent(kind) {
            let name = self.terminal_name(id);
            if let Some(idx) = self.windows.iter().position(|w| w.terminal == Some(id)) {
                self.active_window = idx;
                self.detach_window();
                self.set_message(format!("{name} detached (still running)"));
            } else {
                let vertical = self.place_agent_pane();
                self.reattach_terminal(id, vertical);
                self.last_agent = Some(id);
                self.set_message(format!("{name} reattached (Esc for pane navigation)"));
            }
            return;
        }
        if self.spawn_agent_session(kind).is_some() && restarted {
            self.set_message(format!("{kind} had exited; started a new session"));
        }
    }

    /// If the `kind` agent session has exited, closes the panes showing it
    /// and drops it, so the next use starts a fresh one instead of writing to
    /// a dead process. Returns whether one was reaped.
    pub fn reap_exited_agent(&mut self, kind: &str) -> bool {
        for pty in &mut self.terminals {
            pty.poll_exit();
        }
        let exited: Vec<u64> = self
            .terminals
            .iter()
            .filter(|p| p.agent_kind.as_deref() == Some(kind) && p.has_exited())
            .map(|p| p.id)
            .collect();
        for &id in &exited {
            self.drop_terminal(id);
        }
        !exited.is_empty()
    }

    /// The session of `kind` that `:ai`, `:aistart`, `:aistop`,
    /// `:claude` and friends act on when several are running: the agent
    /// pane focused last, else the newest one shown in this tab, else the
    /// newest one at all.
    pub(crate) fn current_agent(&self, kind: &str) -> Option<u64> {
        let of_kind = |p: &&PtySession| p.agent_kind.as_deref() == Some(kind);
        if let Some(id) = self
            .last_agent
            .filter(|&id| self.terminals.iter().filter(of_kind).any(|p| p.id == id))
        {
            return Some(id);
        }
        let shown = |p: &&PtySession| self.windows.iter().any(|w| w.terminal == Some(p.id));
        self.terminals
            .iter()
            .rev()
            .filter(of_kind)
            .find(shown)
            .or_else(|| self.terminals.iter().rev().find(of_kind))
            .map(|p| p.id)
    }

    /// The display name of terminal `id` (`claude`, `claude #2`, or its
    /// command line).
    pub(crate) fn terminal_name(&self, id: u64) -> String {
        self.terminals
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.display_name())
            .unwrap_or_default()
    }

    /// The smallest slot number no running `kind` session uses.
    fn next_agent_slot(&self, kind: &str) -> usize {
        (1..)
            .find(|n| {
                !self
                    .terminals
                    .iter()
                    .any(|p| p.agent_kind.as_deref() == Some(kind) && p.agent_slot == *n)
            })
            .unwrap_or(1)
    }

    /// Picks where a new agent pane goes and focuses the pane to split.
    /// With an agent pane already in this tab, the new one shares its
    /// space -- below it when agents open to the right, beside it when they
    /// open below -- so several agents stack in one sidebar and the code
    /// keeps its width. Otherwise the pane opens per `agent_split_vertical`.
    /// A `:vertical`/`:horizontal` modifier splits the current pane its way.
    /// Returns whether to split vertically.
    fn place_agent_pane(&mut self) -> bool {
        if let Some(vertical) = self.split_override {
            return vertical;
        }
        let is_agent = |ed: &Self, id: u64| {
            ed.terminals
                .iter()
                .any(|p| p.id == id && p.agent_kind.is_some())
        };
        let anchor = self
            .last_agent
            .and_then(|id| self.windows.iter().position(|w| w.terminal == Some(id)))
            .or_else(|| {
                self.windows
                    .iter()
                    .rposition(|w| w.terminal.is_some_and(|id| is_agent(self, id)))
            });
        match anchor {
            Some(idx) => {
                if idx != self.active_window {
                    self.focus_pane_buffer(idx);
                }
                !self.config.ai_split_vertical()
            }
            None => self.config.ai_split_vertical(),
        }
    }

    /// Closes every pane in this tab that shows terminal `id`, then stops it.
    /// The focused pane stays focused when it was not one of them.
    fn drop_terminal(&mut self, id: u64) {
        self.pending_agent_send.retain(|p| p.id != id);
        let keep = self
            .windows
            .get(self.active_window)
            .filter(|w| w.terminal != Some(id))
            .map(|w| w.buffer);
        let was_terminal_mode = self.mode == crate::mode::Mode::Terminal;
        while let Some(idx) = self.windows.iter().position(|w| w.terminal == Some(id)) {
            if self.windows.len() > 1 {
                self.active_window = idx;
                // `close_window` also shuts the terminal down.
                self.close_window();
            } else {
                self.windows[idx].terminal = None;
                self.windows.clear();
                self.window_layout = None;
                self.active_window = 0;
            }
        }
        self.shutdown_terminal(id);
        if let Some(buffer) = keep {
            if let Some(idx) = self
                .windows
                .iter()
                .position(|w| w.buffer == buffer && w.terminal.is_none())
            {
                self.focus_pane_buffer(idx);
            }
        }
        if was_terminal_mode && self.active_terminal_id().is_none() {
            self.mode = crate::mode::Mode::Normal;
        }
    }

    /// Runs `f` with `split_override` set to `split` (when given), restoring
    /// the previous value afterwards.
    pub(crate) fn with_split_override<R>(
        &mut self,
        split: Option<bool>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let saved = self.split_override;
        if split.is_some() {
            self.split_override = split;
        }
        let out = f(self);
        self.split_override = saved;
        out
    }

    /// `:aistart` and the picker's Start/Focus: opens (or reattaches, or
    /// restarts if it had exited) the configured AI agent in the sidebar and
    /// focuses it, sending nothing.
    pub fn open_ai_sidebar(&mut self) {
        if let SidebarState::Spawned(id) | SidebarState::Reused(id) = self.ensure_ai_sidebar() {
            self.reattach_agent_session(id);
            self.set_message(format!(
                "Opened {} sidebar (Esc for pane navigation)",
                self.config.ai_agent_label()
            ));
        }
    }

    /// `:aistop` / `:agentstop <name>`: stops an agent session and closes its
    /// panes.
    pub fn stop_agent_session(&mut self, kind: &str) {
        let Some(id) = self.current_agent(kind) else {
            self.set_message(format!("No {kind} session is running"));
            return;
        };
        let name = self.terminal_name(id);
        self.drop_terminal(id);
        let left = self
            .terminals
            .iter()
            .filter(|p| p.agent_kind.as_deref() == Some(kind))
            .count();
        self.set_message(if left == 0 {
            format!("Stopped {name}")
        } else {
            format!("Stopped {name} ({left} more {kind} running)")
        });
    }

    /// `:aistop!`: stops every `kind` session and closes their panes.
    pub fn stop_all_agent_sessions(&mut self, kind: &str) {
        let ids: Vec<u64> = self
            .terminals
            .iter()
            .filter(|p| p.agent_kind.as_deref() == Some(kind))
            .map(|p| p.id)
            .collect();
        if ids.is_empty() {
            self.set_message(format!("No {kind} session is running"));
            return;
        }
        for &id in &ids {
            self.drop_terminal(id);
        }
        self.set_message(format!(
            "Stopped {} {kind} session{}",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" }
        ));
    }

    /// `:ainew` and the picker's New session: starts another session of
    /// the configured agent alongside any running ones and focuses it.
    pub fn new_ai_session(&mut self) -> Option<u64> {
        let kind = self.config.ai_agent().to_string();
        self.reap_exited_agent(&kind);
        let id = self.spawn_agent_session(&kind)?;
        let name = self.terminal_name(id);
        self.set_message(format!(
            "Started {name} (Esc for pane navigation; :ai now sends here)"
        ));
        Some(id)
    }

    /// `:airestart`: stops the configured AI agent (if running) and starts a
    /// fresh session in the sidebar.
    pub fn restart_ai_agent(&mut self) {
        let kind = self.config.ai_agent().to_string();
        if let Some(id) = self.current_agent(&kind) {
            self.drop_terminal(id);
        }
        if self.spawn_agent_session(&kind).is_some() {
            self.set_message(format!(
                "Restarted {} (Esc for pane navigation)",
                self.config.ai_agent_label()
            ));
        }
    }

    /// Text rows of the pane showing terminal `id` in this tab (its height
    /// less the statusline), for page-sized scrolling.
    pub(crate) fn terminal_pane_rows(&self, id: u64) -> usize {
        let rects = self.pane_rects(self.screen_cols, self.terminal_rows);
        self.windows
            .iter()
            .zip(rects)
            .find(|(w, _)| w.terminal == Some(id))
            .map(|(_, r)| crate::render::terminal_body_rect(self, r).height)
            .unwrap_or(self.screen_rows)
            .max(1)
    }

    /// Scrolls terminal `id`'s view through its history.
    pub fn scroll_terminal(&mut self, id: u64, how: TerminalScroll) {
        let rows = self.terminal_pane_rows(id) as isize;
        let smooth = self.config.smooth_scroll;
        let Some(pty) = self.terminals.iter_mut().find(|p| p.id == id) else {
            return;
        };
        let mut by = |n: isize| {
            if smooth {
                pty.scroll_smooth(n);
            } else {
                pty.scroll_by(n);
            }
        };
        match how {
            // A line at a time (`j`/`k`) is already as smooth as it gets.
            TerminalScroll::Lines(n) => {
                pty.scroll_by(n);
            }
            TerminalScroll::Half(up) => {
                let n = (rows / 2).max(1);
                by(if up { n } else { -n });
            }
            TerminalScroll::Page(up) => {
                let n = (rows - 1).max(1);
                by(if up { n } else { -n });
            }
            TerminalScroll::Top => pty.scroll_to_top(),
            TerminalScroll::Bottom => pty.scroll_to_bottom(),
        }
    }

    /// A key with no `Key` equivalent (F1-F12, Insert, ...) typed while a
    /// terminal has focus: encode and forward it rather than dropping it.
    pub fn forward_unmodeled_terminal_key(&mut self, ev: crossterm::event::KeyEvent) {
        let float_terminal = self.float.as_ref().and_then(|f| match &f.body {
            crate::float::FloatBody::Terminal { terminal_id } if f.focused => Some(*terminal_id),
            _ => None,
        });
        let id = match float_terminal {
            Some(id) => Some(id),
            None if self.mode == crate::mode::Mode::Terminal => self.active_terminal_id(),
            None => None,
        };
        let Some(id) = id else {
            return;
        };
        self.terminal_literal_next = false;
        if let Some(pty) = self.terminals.iter_mut().find(|p| p.id == id) {
            if let Some(bytes) = encode_key_event(ev, pty.application_cursor()) {
                pty.write_input(&bytes);
            }
        }
    }

    fn scroll_terminal_page(&mut self, id: u64, up: bool) {
        self.scroll_terminal(id, TerminalScroll::Page(up));
    }

    /// `:termcapture` (and `/`, `?`, `v`, `V`, `y` on a terminal pane in
    /// Normal mode): opens the terminal's whole output -- history and screen
    /// -- as a plain buffer in a split below, cursor on the line that was at
    /// the bottom of the view, so ordinary search, Visual selection and
    /// yanking work on it.
    pub fn open_terminal_capture(&mut self) -> bool {
        let id = self
            .active_terminal_id()
            .or_else(|| self.windows.iter().find_map(|w| w.terminal));
        let Some(id) = id else {
            self.set_message("No terminal to capture -- focus a terminal pane first");
            return false;
        };
        let rows = self.terminal_pane_rows(id);
        let Some(pty) = self.terminals.iter().find(|p| p.id == id) else {
            return false;
        };
        let title = pty.display_name();
        let (text, view_top) = pty.capture_text();
        if let Some(idx) = self.windows.iter().position(|w| w.terminal == Some(id)) {
            self.active_window = idx;
        }
        let before = self.windows.len();
        self.split_window(false, false);
        if self.windows.len() <= before {
            return false;
        }
        let mut b = crate::buffer::Buffer::empty();
        b.rope = ropey::Rope::from_str(&text);
        b.mark_saved();
        let buffer_id = b.id;
        self.buffers.push(b);
        self.cur = self.buffers.len() - 1;
        if let Some(w) = self.windows.get_mut(self.active_window) {
            w.terminal = None;
            w.buffer = buffer_id;
        }
        self.invalidate_index_caches();
        self.mode = crate::mode::Mode::Normal;
        let last = self.buf().line_count().saturating_sub(1);
        self.set_cursor((view_top + rows.saturating_sub(1)).min(last), 0);
        self.set_message(format!(
            "Output of {title} as a buffer: search, select and yank as usual; :q closes it"
        ));
        true
    }

    /// Spawn a fresh agent CLI session for `kind` (argv from
    /// `config.agent_commands`, falling back to a bare `kind` on PATH) into a
    /// new split -- vertical to the right, horizontal below (see
    /// `agent_split_vertical`) --
    /// attach it to the new pane, and focus it in Terminal mode. Returns the
    /// new session id, or `None` if the CLI could not be started (a message is
    /// set either way). Shared by `toggle_agent_session` and `ensure_ai_sidebar`.
    fn spawn_agent_session(&mut self, kind: &str) -> Option<u64> {
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
                session.agent_slot = self.next_agent_slot(kind);
                let id = session.id;
                let name = session.display_name();
                self.terminals.push(session);
                let vertical = self.place_agent_pane();
                if self.attach_terminal_pane(id, vertical) {
                    self.last_agent = Some(id);
                    self.set_message(format!("{name} (Esc for pane navigation)"));
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

    /// Ensure the configured AI agent is attached in this tab so the AI-prompt
    /// feature has somewhere to send to: reuse it if already attached, reattach
    /// it if detached, else spawn it -- in a new split whose direction comes
    /// from `agent_split_vertical`. The returned state tells the caller whether
    /// the session was already running (safe to send to immediately) or freshly
    /// spawned (its TUI is still starting, so a send must be deferred).
    pub fn ensure_ai_sidebar(&mut self) -> SidebarState {
        let kind = self.config.ai_agent().to_string();
        self.reap_exited_agent(&kind);
        if let Some(id) = self.current_agent(&kind) {
            if !self.windows.iter().any(|w| w.terminal == Some(id)) {
                let vertical = self.place_agent_pane();
                self.reattach_terminal(id, vertical);
            }
            self.last_agent = Some(id);
            return SidebarState::Reused(id);
        }
        match self.spawn_agent_session(&kind) {
            Some(id) => SidebarState::Spawned(id),
            None => SidebarState::Missing,
        }
    }

    /// Deliver a `pending_agent_send` once its freshly-spawned CLI has actually
    /// settled at its input prompt, detected by *output quiescence*: the child
    /// produced output and then went quiet for `AGENT_SETTLE` with no further
    /// output. This is robust to slow / multi-step startup (auth, corporate
    /// wrappers, banners) that a fixed delay would race -- we wait for whatever
    /// setup the agent launcher does to finish and the REPL to stop drawing,
    /// however long that takes, capped by `AGENT_MAX_WAIT`. Returns whether it
    /// delivered (so the caller redraws).
    pub fn flush_pending_agent_send(&mut self) -> bool {
        let mut work = self.flush_pending_agent_submit();
        let queued = std::mem::take(&mut self.pending_agent_send);
        let mut keep = Vec::new();
        for pending in queued {
            match self.deliver_pending_agent_send(pending) {
                Ok(()) => work = true,
                Err(pending) => keep.push(pending),
            }
        }
        // Anything queued while delivering goes after what was waiting.
        keep.append(&mut self.pending_agent_send);
        self.pending_agent_send = keep;
        work
    }

    /// Delivers one queued prompt if its session is ready (or drops it, with
    /// a message, if the session is gone); hands it back when not yet.
    fn deliver_pending_agent_send(
        &mut self,
        mut p: PendingAgentSend,
    ) -> Result<(), PendingAgentSend> {
        let id = p.id;
        let Some(pty) = self.terminals.iter_mut().find(|t| t.id == id) else {
            // The sidebar was closed during the defer window; drop the queued
            // prompt and say so (the earlier "Sent" message was optimistic).
            self.set_message("Queued AI prompt discarded (sidebar closed)");
            return Ok(());
        };
        if pty.poll_exit().is_some() {
            let name = pty.display_name();
            self.registers.set(Some('+'), p.text, false);
            self.set_message(format!(
                "{name} exited before the prompt could be sent; copied it to the + register"
            ));
            return Ok(());
        }
        let rev = pty.output_revision();
        let now = std::time::Instant::now();
        if rev != p.last_rev {
            p.last_rev = rev;
            p.last_change_at = now;
        }
        // Ready once the child has drawn something and then gone quiet (sitting
        // at its prompt), or after the hard cap regardless.
        let quiet_for = now.duration_since(p.last_change_at);
        let waited = now.duration_since(p.spawned_at);
        let ready = (rev > 0 && quiet_for >= AGENT_SETTLE) || waited >= AGENT_MAX_WAIT;
        if !ready {
            return Err(p);
        }
        pty.write_pasted_input(&p.text);
        let name = pty.display_name();
        if p.submit {
            self.pending_agent_submit.push((id, now));
        }
        if self.message.starts_with("Prompt queued") {
            self.set_message(format!(
                "Prompt delivered to {name}{}",
                if p.submit {
                    " and submitted"
                } else {
                    " — press Enter there to submit"
                }
            ));
        }
        Ok(())
    }

    /// Presses Enter in an agent session a moment after a prompt was pasted
    /// (`ai_auto_submit`). The pause lets the CLI finish taking in the paste,
    /// which some TUIs otherwise swallow the Enter into.
    fn flush_pending_agent_submit(&mut self) -> bool {
        let (due, waiting): (Vec<_>, Vec<_>) = self
            .pending_agent_submit
            .drain(..)
            .partition(|(_, at)| at.elapsed() >= AGENT_SUBMIT_DELAY);
        self.pending_agent_submit = waiting;
        for (id, _) in &due {
            if let Some(pty) = self.terminals.iter_mut().find(|p| p.id == *id) {
                pty.write_input(b"\r");
            }
        }
        !due.is_empty()
    }

    /// `:agents`: every currently running agent session (started via
    /// `:claude`/`:codex`/`:agent`), attached-in-this-tab or detached;
    /// Enter reattaches the selected one here (or just focuses it, if
    /// it's already attached in this tab).
    pub fn list_agent_sessions(&mut self) {
        for pty in &mut self.terminals {
            pty.poll_exit();
        }
        let entries: Vec<crate::results::Entry> = self
            .terminals
            .iter()
            .filter_map(|p| {
                p.agent_kind.as_deref()?;
                let attached = self.windows.iter().any(|w| w.terminal == Some(p.id));
                let mut e = crate::results::Entry::text(format!(
                    "{} — {}{}",
                    p.display_name(),
                    if p.has_exited() {
                        "exited (Enter starts a new session)"
                    } else if attached {
                        "attached in this tab"
                    } else {
                        "detached"
                    },
                    if self.last_agent == Some(p.id) {
                        " · :ai sends here"
                    } else {
                        ""
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
        let mut results =
            crate::results::Results::new("Agent sessions — Enter attaches one here", entries);
        results.menu = true;
        self.show_results(results);
    }

    /// Dispatches a `_vaayu_agent_reattach`-tagged entry from
    /// `list_agent_sessions`: reattaches `id` here, or just focuses it
    /// if it's already attached in this tab (reattaching an
    /// already-attached session would otherwise open a second, redundant
    /// pane onto the same output).
    pub fn reattach_agent_session(&mut self, id: u64) {
        let exited_kind = self
            .terminals
            .iter()
            .find(|p| p.id == id && p.has_exited())
            .and_then(|p| p.agent_kind.clone());
        if let Some(kind) = exited_kind {
            self.drop_terminal(id);
            if self.spawn_agent_session(&kind).is_some() {
                self.set_message(format!("{kind} had exited; started a new session"));
            }
            return;
        }
        if let Some(idx) = self.windows.iter().position(|w| w.terminal == Some(id)) {
            self.focus_pane_buffer(idx);
            self.mode = crate::mode::Mode::Terminal;
            return;
        }
        let agent = self
            .terminals
            .iter()
            .any(|p| p.id == id && p.agent_kind.is_some());
        let vertical = if agent {
            self.place_agent_pane()
        } else {
            self.split_override.unwrap_or(false)
        };
        self.reattach_terminal(id, vertical);
        if agent {
            self.last_agent = Some(id);
        }
    }

    /// Kills and joins the terminal's reader thread -- called when the
    /// pane hosting it closes, so no process or thread outlives its pane.
    pub fn shutdown_terminal(&mut self, id: u64) {
        if self.last_agent == Some(id) {
            self.last_agent = None;
        }
        if self
            .terminal_hover
            .as_ref()
            .is_some_and(|h| h.terminal == id)
        {
            self.terminal_hover = None;
        }
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

    /// Advances every terminal's smooth scroll by one frame. Returns whether
    /// any view moved (so the main loop redraws).
    pub fn tick_terminal_scroll(&mut self) -> bool {
        let now = std::time::Instant::now();
        let mut moved = false;
        for pty in &mut self.terminals {
            moved |= pty.tick_scroll(now);
        }
        if moved {
            self.refresh_terminal_hover();
        }
        moved
    }

    /// True if any terminal has produced new output (or changed exit
    /// state) since the last check -- lets the main loop redraw a running
    /// shell's output without waiting for the next keystroke, the same
    /// way `poll_lsp_events` does for language server replies.
    pub fn poll_terminals(&mut self) -> bool {
        let mut changed = false;
        let mut just_exited: Vec<(String, String, Option<u32>, std::time::Duration)> = Vec::new();
        for pty in &mut self.terminals {
            // Only the *transition* to exited counts as a change: `poll_exit`
            // caches and returns `Some` forever after the child dies, so
            // checking `is_some()` every tick would keep the main loop redrawing
            // at 100% CPU while a finished terminal pane stays open.
            let was_running = pty.exited.is_none();
            if let Some(code) = pty.poll_exit() {
                if was_running {
                    changed = true;
                    let kind = pty.agent_kind.clone().unwrap_or_else(|| "terminal".into());
                    just_exited.push((kind, pty.title.clone(), code, pty.started.elapsed()));
                }
            }
            let rev = pty.revision.load(Ordering::Relaxed);
            if rev != pty.seen_revision {
                pty.seen_revision = rev;
                changed = true;
            }
        }
        if changed {
            self.refresh_terminal_hover();
        }
        for (kind, title, code, elapsed) in &just_exited {
            let detail = code.map(|c| format!("exit {c}")).unwrap_or_default();
            self.log_job(kind, title, *code == Some(0), *elapsed, &detail);
        }
        if let Some((_, title, code, _)) = just_exited.pop() {
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
/// mode: Esc leaves to Normal (still focused on the pane, for navigation,
/// scrolling or `:close`); Ctrl-W starts a window command; Ctrl-\ sends the
/// next key to the child literally (so Esc, Ctrl-W and Ctrl-\ itself can
/// reach it); Shift-PageUp/PageDown scroll the output; everything else is
/// forwarded to the child as raw bytes.
pub fn handle_terminal_mode(ed: &mut crate::editor::Editor, key: crate::key::Key) {
    use crate::key::Key;
    if ed.terminal_literal_next {
        ed.terminal_literal_next = false;
        ed.pending_jk = None;
        if let Some(id) = ed.active_terminal_id() {
            write_terminal_key(ed, id, key);
        }
        return;
    }
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
    } else if ed.config.jk_escape && key == Key::Char('j') && ed.active_terminal_id().is_some() {
        // Buffer the `j` (only when a terminal is actually focused, so a stray
        // `j` can't be stranded); the main loop flushes it after `timeoutlen_ms`
        // (via flush_pending_jk) if no `k` follows, so a lone `j` still reaches
        // the child.
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
    if matches!(key, Key::Ctrl('\\' | '4')) {
        ed.terminal_literal_next = true;
        ed.set_message("Ctrl-\\: the next key goes to the terminal as typed");
        return;
    }
    let Some(id) = ed.active_terminal_id() else {
        ed.mode = crate::mode::Mode::Normal;
        return;
    };
    let shifted = ed
        .raw_key
        .is_some_and(|ev| ev.modifiers.contains(crossterm::event::KeyModifiers::SHIFT));
    if shifted && matches!(key, Key::PageUp | Key::PageDown) {
        ed.raw_key = None;
        let up = key == Key::PageUp;
        ed.scroll_terminal_page(id, up);
        return;
    }
    write_terminal_key(ed, id, key);
}

/// Scroll amounts for a terminal pane's history view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalScroll {
    /// Lines toward older output (positive) or the live screen (negative).
    Lines(isize),
    /// Half a pane toward older (`true`) or newer output.
    Half(bool),
    /// A full pane toward older (`true`) or newer output.
    Page(bool),
    Top,
    Bottom,
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
    fn terminal_queries_reply_to_fish_startup_checks() {
        let mut pending =
            b"\x1b[0c\x1bP+q696e646e\x1b\\\x1bP+q71756572792d6f732d6e616d65\x1b\\".to_vec();
        let replies = terminal_query_replies(&mut pending);
        assert_eq!(pending, b"");
        assert_eq!(replies[0], b"\x1b[?1;2c");
        assert_eq!(replies[1], b"\x1bP1+r696e646e\x1b\\");
        assert_eq!(
            replies[2],
            b"\x1bP1+r71756572792d6f732d6e616d65=4c696e7578\x1b\\"
        );
    }

    #[test]
    fn terminal_query_parser_keeps_split_probe_until_complete() {
        let mut pending = b"\x1b[0".to_vec();
        assert!(terminal_query_replies(&mut pending).is_empty());
        pending.extend_from_slice(b"c");
        assert_eq!(
            terminal_query_replies(&mut pending),
            vec![b"\x1b[?1;2c".to_vec()]
        );
        assert!(pending.is_empty());
    }

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
    fn shrinking_keeps_the_newest_output_and_moves_the_rest_to_history() {
        let mut pty = PtySession::spawn(
            &["sh".into(), "-c".into(), "seq 1 20; exec cat".into()],
            std::path::Path::new("/"),
            30,
            40,
        )
        .unwrap();
        let start = std::time::Instant::now();
        while !pty.with_screen(|s| s.contents().contains("20")) {
            assert!(start.elapsed().as_secs() < 5, "seq output");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(pty.scrollback_len(), 0);
        pty.resize(8, 40);
        let shown = pty.with_screen(|s| s.contents());
        assert!(
            shown.contains("\n20"),
            "the newest line stays in view: {shown:?}"
        );
        assert!(!shown.contains("\n5\n"), "older lines left the screen");
        assert!(pty.scrollback_len() > 0, "they went to history");
        let (text, _) = pty.capture_text();
        assert!(text.starts_with("1\n2\n3\n"), "nothing was lost: {text:?}");
        pty.shutdown();
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
