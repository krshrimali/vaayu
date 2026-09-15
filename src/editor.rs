use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::buffer::Buffer;
use crate::config::Config;
use crate::key::Key;
use crate::mode::{CommandKind, Mode, VisualKind};
use crate::normal::PendingState;
use crate::registers::Registers;

pub struct Editor {
    pub buffers: Vec<Buffer>,
    pub cur: usize,
    pub mode: Mode,
    pub config: Config,
    pub registers: Registers,
    pub message: String,
    pub should_quit: bool,

    pub pending: PendingState,
    pub visual_anchor: Option<(usize, usize)>,
    pub cmdline: String,

    pub last_search: Option<(String, bool)>,
    pub last_find: Option<(char, bool, bool)>,

    pub macro_recording: Option<(char, Vec<Key>)>,
    pub last_macro_reg: Option<char>,
    pub macros: HashMap<char, Vec<Key>>,

    pub recording_change: bool,
    pub cmd_keys: Vec<Key>,
    pub last_change: Vec<Key>,
    pub replaying: bool,

    pub pending_jk: Option<Instant>,
    pub screen_rows: usize,
    pub hl_search: bool,
}

impl Editor {
    pub fn new(config: Config) -> Editor {
        Editor {
            buffers: vec![Buffer::empty()],
            cur: 0,
            mode: Mode::Normal,
            config,
            registers: Registers::new(),
            message: String::from("anvil -- type :help-less, :w to save, :q to quit"),
            should_quit: false,
            pending: PendingState::default(),
            visual_anchor: None,
            cmdline: String::new(),
            last_search: None,
            last_find: None,
            macro_recording: None,
            last_macro_reg: None,
            macros: HashMap::new(),
            recording_change: false,
            cmd_keys: Vec::new(),
            last_change: Vec::new(),
            replaying: false,
            pending_jk: None,
            screen_rows: 24,
            hl_search: true,
        }
    }

    pub fn open_file(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let buf = Buffer::from_path(path)?;
        if self.buffers.len() == 1 && self.buffers[0].path.is_none() && !self.buffers[0].modified {
            self.buffers[0] = buf;
        } else {
            self.buffers.push(buf);
            self.cur = self.buffers.len() - 1;
        }
        Ok(())
    }

    pub fn buf(&self) -> &Buffer {
        &self.buffers[self.cur]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.cur]
    }

    pub fn buf_and_registers_mut(&mut self) -> (&mut Buffer, &mut Registers) {
        (&mut self.buffers[self.cur], &mut self.registers)
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.buf().cursor_line, self.buf().cursor_col)
    }

    pub fn set_cursor(&mut self, line: usize, col: usize) {
        let b = self.buf_mut();
        let line = line.min(b.line_count().saturating_sub(1));
        let col = match b.clamp_col_normal(line, col) {
            c => c,
        };
        b.cursor_line = line;
        b.cursor_col = col;
    }

    pub fn set_cursor_insert(&mut self, line: usize, col: usize) {
        let b = self.buf_mut();
        let line = line.min(b.line_count().saturating_sub(1));
        let col = b.clamp_col_insert(line, col);
        b.cursor_line = line;
        b.cursor_col = col;
    }

    /// Single entry point for every key: main loop and macro/dot replay funnel through here.
    pub fn feed_key(&mut self, key: Key) {
        // Macro recording: a bare 'q' in Normal mode with nothing pending stops recording
        // instead of being processed as a command.
        if self.macro_recording.is_some()
            && matches!(self.mode, Mode::Normal)
            && self.pending.is_empty()
            && key == Key::Char('q')
        {
            if let Some((reg, keys)) = self.macro_recording.take() {
                self.macros.insert(reg, keys);
                self.message = format!("recorded @{}", reg);
            }
            return;
        }
        if let Some((_, keys)) = &mut self.macro_recording {
            keys.push(key);
        }

        // Dot-repeat recording: capture the raw keys of the in-flight change command.
        if self.recording_change {
            self.cmd_keys.push(key);
        }

        match self.mode {
            Mode::Normal => crate::normal::handle(self, key),
            Mode::Insert => crate::insert::handle(self, key),
            Mode::Visual(_) => crate::visual::handle(self, key),
            Mode::Command(_) => crate::command::handle(self, key),
        }
    }

    pub fn start_change_recording(&mut self, first_key: Key) {
        if !self.replaying {
            self.recording_change = true;
            self.cmd_keys = vec![first_key];
        }
    }

    pub fn finish_change_recording(&mut self) {
        if self.recording_change && !self.replaying {
            self.last_change = self.cmd_keys.clone();
        }
        self.recording_change = false;
    }

    pub fn abort_change_recording(&mut self) {
        self.recording_change = false;
        self.cmd_keys.clear();
    }

    pub fn replay(&mut self, keys: &[Key]) {
        let was_replaying = self.replaying;
        self.replaying = true;
        for k in keys.to_vec() {
            self.feed_key(k);
        }
        self.replaying = was_replaying;
    }

    pub fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
    }

    pub fn enter_normal(&mut self) {
        self.mode = Mode::Normal;
        let (l, c) = self.cursor();
        let nc = self.buf().clamp_col_normal(l, c);
        self.buf_mut().cursor_col = nc;
    }

    pub fn enter_visual(&mut self, kind: VisualKind) {
        self.visual_anchor = Some(self.cursor());
        self.mode = Mode::Visual(kind);
    }

    pub fn enter_command(&mut self, kind: CommandKind) {
        self.cmdline.clear();
        self.mode = Mode::Command(kind);
    }

    pub fn set_message<S: Into<String>>(&mut self, msg: S) {
        self.message = msg.into();
    }

    /// Called by the main loop when the jk-escape timeout elapses with no
    /// follow-up key: the buffered 'j' becomes a literal character.
    pub fn flush_pending_jk(&mut self) {
        if self.pending_jk.take().is_some() && matches!(self.mode, Mode::Insert) {
            let (line, col) = self.cursor();
            self.buf_mut().insert_char(line, col, 'j');
            self.set_cursor_insert(line, col + 1);
        }
    }
}
