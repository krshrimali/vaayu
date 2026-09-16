use crate::{editor::Editor, key::Key};
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Window {
    pub buffer: u64,
    pub cursor: (usize, usize),
    pub top: usize,
    pub wrap_row: usize,
    pub left: usize,
    pub preview: bool,
    pub preview_scroll: usize,
    /// `Some(id)` when this pane shows an embedded PTY job instead of
    /// `buffer`'s text. Never persisted: a saved session restores plain
    /// buffer panes only, never resurrects a process (see
    /// NEOVIM_PARITY_PLAN.md's "session round trips... without restoring
    /// unsafe jobs").
    #[serde(skip)]
    pub terminal: Option<u64>,
}
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Layout {
    Leaf(usize),
    Split {
        vertical: bool,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}
impl Layout {
    fn split(&mut self, active: usize, next: usize, vertical: bool) {
        match self {
            Self::Leaf(i) if *i == active => {
                *self = Self::Split {
                    vertical,
                    first: Box::new(Self::Leaf(active)),
                    second: Box::new(Self::Leaf(next)),
                }
            }
            Self::Split { first, second, .. } => {
                first.split(active, next, vertical);
                second.split(active, next, vertical);
            }
            _ => {}
        }
    }
    fn remove(self, index: usize) -> Option<Self> {
        match self {
            Self::Leaf(i) => {
                if i == index {
                    None
                } else {
                    Some(Self::Leaf(if i > index { i - 1 } else { i }))
                }
            }
            Self::Split {
                vertical,
                first,
                second,
            } => match (first.remove(index), second.remove(index)) {
                (Some(a), Some(b)) => Some(Self::Split {
                    vertical,
                    first: Box::new(a),
                    second: Box::new(b),
                }),
                (a, b) => a.or(b),
            },
        }
    }
    fn rects(&self, r: Rect, out: &mut [Rect]) {
        match self {
            Self::Leaf(i) => {
                if let Some(slot) = out.get_mut(*i) {
                    *slot = r;
                }
            }
            Self::Split {
                vertical,
                first,
                second,
            } => {
                let mut a = r;
                let mut b = r;
                if *vertical {
                    let half = r.width / 2;
                    a.width = half.saturating_sub(1);
                    b.x += half;
                    b.width -= half;
                } else {
                    let half = r.height / 2;
                    a.height = half.saturating_sub(1);
                    b.y += half;
                    b.height -= half;
                }
                first.rects(a, out);
                second.rects(b, out);
            }
        }
    }
}
impl Editor {
    pub fn capture_window(&self) -> Window {
        Window {
            buffer: self.buf().id,
            cursor: self.cursor(),
            top: self.buf().top_line,
            wrap_row: self.buf().top_wrap,
            left: self.buf().left_col,
            preview: false,
            preview_scroll: 0,
            terminal: None,
        }
    }
    pub fn store_window(&mut self) {
        if !self.windows.is_empty() {
            let preview = self.windows[self.active_window].preview;
            let scroll = self.windows[self.active_window].preview_scroll;
            let terminal = self.windows[self.active_window].terminal;
            let mut w = self.capture_window();
            w.preview = preview;
            w.preview_scroll = scroll;
            w.terminal = terminal;
            self.windows[self.active_window] = w;
        }
    }
    pub fn split_window(&mut self, vertical: bool, preview: bool) {
        if self.windows.len() >= 32 {
            self.set_message("At most 32 panes are supported");
            return;
        }
        if preview && !self.is_markdown_buffer() {
            self.set_message("Markdown preview needs a .md buffer");
            return;
        }
        self.store_window();
        if self.windows.is_empty() {
            self.windows.push(self.capture_window());
        }
        self.split_vertical = vertical;
        let layout = self.window_layout.get_or_insert(Layout::Leaf(0));
        layout.split(self.active_window, self.windows.len(), vertical);
        let mut w = self.capture_window();
        w.preview = preview;
        self.windows.push(w);
        if !preview {
            self.focus_window(self.windows.len() - 1);
        }
    }
    pub fn focus_window(&mut self, index: usize) {
        if self.windows.is_empty() {
            return;
        }
        self.store_window();
        self.active_window = index.min(self.windows.len() - 1);
        let w = self.windows[self.active_window].clone();
        if let Some(i) = self.buffers.iter().position(|b| b.id == w.buffer) {
            self.cur = i;
            self.set_cursor(w.cursor.0, w.cursor.1);
            let b = self.buf_mut();
            b.top_line = w.top;
            b.top_wrap = w.wrap_row;
            b.left_col = w.left;
        }
        self.close_completion();
        self.enter_normal();
    }
    pub fn close_window(&mut self) {
        if self.windows.len() > 1 {
            if let Some(id) = self.windows[self.active_window].terminal {
                self.shutdown_terminal(id);
            }
            self.window_layout = self
                .window_layout
                .take()
                .and_then(|l| l.remove(self.active_window));
            self.windows.remove(self.active_window);
            self.active_window = self.active_window.min(self.windows.len() - 1);
            let w = self.windows[self.active_window].clone();
            if let Some(i) = self.buffers.iter().position(|b| b.id == w.buffer) {
                self.cur = i;
                self.set_cursor(w.cursor.0, w.cursor.1);
            }
            if self.windows.len() == 1 {
                self.windows.clear();
                self.window_layout = None;
                self.active_window = 0;
            }
        }
    }
    pub fn window_key(&mut self, key: Key) {
        match key {
            Key::Char('v') => self.split_window(true, false),
            Key::Char('s') => self.split_window(false, false),
            Key::Char('w') => {
                if !self.windows.is_empty() {
                    self.focus_window((self.active_window + 1) % self.windows.len());
                }
            }
            Key::Char(dir @ ('h' | 'j' | 'k' | 'l')) => {
                let rects = self.pane_rects(self.screen_cols, self.screen_rows);
                let r = rects[self.active_window];
                let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
                let next = rects
                    .iter()
                    .enumerate()
                    .filter(|(i, other)| {
                        *i != self.active_window
                            && match dir {
                                'h' => other.x + other.width <= r.x,
                                'l' => other.x >= r.x + r.width,
                                'k' => other.y + other.height <= r.y,
                                _ => other.y >= r.y + r.height,
                            }
                    })
                    .min_by_key(|(_, o)| {
                        let (ox, oy) = (o.x + o.width / 2, o.y + o.height / 2);
                        if matches!(dir, 'h' | 'l') {
                            x.abs_diff(ox) + 4 * y.abs_diff(oy)
                        } else {
                            y.abs_diff(oy) + 4 * x.abs_diff(ox)
                        }
                    })
                    .map(|(i, _)| i);
                if let Some(i) = next {
                    self.focus_window(i);
                }
            }
            Key::Char('c') => self.close_window(),
            Key::Char('o') => {
                self.windows.clear();
                self.window_layout = None;
                self.active_window = 0;
            }
            _ => {}
        }
    }
    pub fn pane_rects(&self, cols: usize, rows: usize) -> Vec<Rect> {
        let r = Rect {
            x: 0,
            y: 0,
            width: cols,
            height: rows.saturating_sub(1),
        };
        let mut out = vec![r; self.windows.len().max(1)];
        if let Some(layout) = &self.window_layout {
            layout.rects(r, &mut out);
        }
        out
    }
}
