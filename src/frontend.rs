//! What the main loop needs from the thing displaying the editor. Everything
//! above this line (dispatch, jobs, rendering) is shared, so the terminal and
//! the browser get the same editor and the same frames.
use std::io::{self, Stdout, Write};
use std::time::Duration;

use crossterm::event::{self, Event};

use crate::editor::Editor;

pub trait Frontend {
    type Output: Write;

    fn output(&mut self) -> &mut Self::Output;
    fn size(&self) -> io::Result<(u16, u16)>;
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;

    /// True once when the display lost its contents, so nothing already on it
    /// may be reused by the row diff.
    fn take_repaint(&mut self) -> bool {
        false
    }

    /// Runs once per frame and idle wake-up. True asks for a redraw.
    fn service(&mut self, _ed: &mut Editor) -> bool {
        false
    }
}

pub struct TerminalFrontend {
    stdout: Stdout,
}

impl TerminalFrontend {
    pub fn new() -> Self {
        Self {
            stdout: io::stdout(),
        }
    }
}

impl Frontend for TerminalFrontend {
    type Output = Stdout;

    fn output(&mut self) -> &mut Stdout {
        &mut self.stdout
    }

    fn size(&self) -> io::Result<(u16, u16)> {
        crossterm::terminal::size()
    }

    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}
