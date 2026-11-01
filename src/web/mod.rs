//! `vaayu --web`: the editor as a local web application. The whole editor runs
//! here, on the main thread and through the same main loop as the terminal; the
//! tab only paints the frames `render::draw` produces and reports input. See
//! `assets/app.js` for the other half and ARCHITECTURE.md for the reasoning.
pub mod launch;
mod protocol;
mod server;

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::event::Event;

use crate::editor::Editor;
use crate::frontend::Frontend;
use server::{Hub, Inbound};

/// With no tab attached and nothing that could be lost, a session stops by
/// itself after this long so an abandoned one doesn't run forever.
const IDLE_EXIT: Duration = Duration::from_secs(30 * 60);
/// The grid before a tab has reported its own.
const INITIAL_GRID: (u16, u16) = (120, 40);
const BROWSER_KEYS_HINT: &str =
    "Browsers keep Ctrl-W/T/N: press Alt-W/T/N, or F11 for fullscreen where Ctrl works";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub file: Option<String>,
    pub port: Option<u16>,
    pub no_open: bool,
    /// Started by `:web`: outlives the terminal that launched it.
    pub detached: bool,
}

impl Options {
    /// `None` unless the arguments ask for the web frontend.
    pub fn parse(args: &[String]) -> anyhow::Result<Option<Options>> {
        if !args.iter().any(|a| a == "--web") {
            return Ok(None);
        }
        let mut options = Options::default();
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--web" => {}
                "--no-open" => options.no_open = true,
                "--web-detached" => options.detached = true,
                "--port" => {
                    let value = rest.next().context("--port needs a number")?;
                    options.port = Some(value.parse().context("--port needs a number")?);
                }
                file if options.file.is_none() && !file.starts_with("--") => {
                    options.file = Some(file.to_string());
                }
                other => anyhow::bail!("unexpected argument with --web: {other}"),
            }
        }
        Ok(Some(options))
    }
}

/// Frame bytes for the current tab. A frame leaves in one message on `flush`.
pub struct FrameWriter {
    hub: Arc<Hub>,
    frame: Vec<u8>,
    drawn: bool,
}

impl Write for FrameWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.frame.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.frame.is_empty() {
            self.drawn = true;
            self.hub.send_frame(std::mem::take(&mut self.frame));
        }
        Ok(())
    }
}

pub struct WebFrontend {
    output: FrameWriter,
    hub: Arc<Hub>,
    inbound: Receiver<Inbound>,
    pending: Option<Event>,
    opens: Vec<String>,
    repaint: bool,
    title: String,
    detached_since: Option<Instant>,
    stop: Arc<AtomicBool>,
    interrupted: bool,
    state_file: std::path::PathBuf,
}

impl WebFrontend {
    pub fn start(ed: &mut Editor, options: Options) -> anyhow::Result<Self> {
        crate::display::set_grid(INITIAL_GRID.0, INITIAL_GRID.1);
        ed.set_message(BROWSER_KEYS_HINT);
        if ed.config.mermaid_preview == crate::mermaid::Mode::Auto {
            ed.graphics.borrow_mut().assume_available();
        }

        let stop = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())?;
        signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())?;
        #[cfg(unix)]
        {
            // SIGHUP's default action would end a detached session with its
            // terminal; a foreground one should stop cleanly along with it.
            let hangup = if options.detached {
                Arc::new(AtomicBool::new(false))
            } else {
                stop.clone()
            };
            signal_hook::flag::register(signal_hook::consts::SIGHUP, hangup)?;
        }

        let font = protocol::Font {
            family: ed.config.web_font_family.clone(),
            size: ed.config.web_font_size,
        };
        let (hub, inbound, port, token) = server::start(options.port, font)?;
        let clipboard_hub = hub.clone();
        crate::clipboard::use_web(move |text| clipboard_hub.send_text(protocol::clipboard(text)));

        let url = format!("http://127.0.0.1:{port}/?token={token}");
        let state_file = launch::write_state(&ed.project_root, port, &token)?;
        launch::announce(&url, &options);
        Ok(Self {
            output: FrameWriter {
                hub: hub.clone(),
                frame: Vec::new(),
                drawn: false,
            },
            hub,
            inbound,
            pending: None,
            opens: Vec::new(),
            repaint: false,
            title: String::new(),
            detached_since: Some(Instant::now()),
            stop,
            interrupted: false,
            state_file,
        })
    }

    /// Whether a signal, rather than the user quitting, ended the session.
    pub fn interrupted(&self) -> bool {
        self.interrupted
    }

    /// Delivers everything already written and tells the tab it is over.
    pub fn finish(&mut self) {
        let _ = std::fs::remove_file(&self.state_file);
        let _ = self.output.flush();
        self.hub.send_text(protocol::bye());
        self.hub.drain(Duration::from_secs(2));
    }

    fn take_event(&mut self, inbound: Inbound) -> Option<Event> {
        match inbound {
            Inbound::Attached { cols, rows, cell } => {
                self.detached_since = None;
                self.repaint = true;
                self.title.clear();
                crate::display::set_cell_pixels(cell.0.max(1), cell.1.max(1));
                crate::display::set_grid(cols, rows);
                Some(Event::Resize(cols, rows))
            }
            Inbound::Detached => {
                self.detached_since = Some(Instant::now());
                None
            }
            Inbound::Input(event) => {
                if let Event::Resize(cols, rows) = event {
                    crate::display::set_grid(cols, rows);
                }
                Some(event)
            }
            Inbound::Clipboard(text) => {
                crate::clipboard::set_web_mirror(text);
                None
            }
            Inbound::Open(path) => {
                self.opens.push(path);
                None
            }
        }
    }
}

impl Frontend for WebFrontend {
    type Output = FrameWriter;

    fn output(&mut self) -> &mut FrameWriter {
        &mut self.output
    }

    fn size(&self) -> io::Result<(u16, u16)> {
        crate::display::size()
    }

    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.pending.is_some() {
            return Ok(true);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.inbound.recv_timeout(left) {
                Ok(inbound) => {
                    if let Some(event) = self.take_event(inbound) {
                        self.pending = Some(event);
                        return Ok(true);
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Ok(false),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other("web server stopped"));
                }
            }
        }
    }

    fn read(&mut self) -> io::Result<Event> {
        loop {
            if let Some(event) = self.pending.take() {
                return Ok(event);
            }
            self.poll(Duration::from_secs(3600))?;
        }
    }

    fn take_repaint(&mut self) -> bool {
        std::mem::take(&mut self.repaint)
    }

    fn service(&mut self, ed: &mut Editor) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            self.interrupted = true;
            ed.should_quit = true;
            return true;
        }
        let mut work = false;
        for path in std::mem::take(&mut self.opens) {
            if let Err(e) = ed.open_file(path.into()) {
                ed.set_message(format!("web: {e}"));
            }
            work = true;
        }
        if std::mem::take(&mut self.output.drawn) {
            let title = launch::title(ed);
            if title != self.title {
                self.hub.send_text(protocol::title(&title));
                self.title = title;
            }
        }
        if self
            .detached_since
            .is_some_and(|since| since.elapsed() >= IDLE_EXIT)
            && !ed.has_unsaved_or_running()
        {
            ed.should_quit = true;
            return true;
        }
        work
    }
}
