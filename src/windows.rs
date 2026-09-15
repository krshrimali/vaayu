use crate::{editor::Editor, key::Key};
#[derive(Clone, Debug)]
pub struct Window {
    pub buffer: u64,
    pub cursor: (usize, usize),
    pub top: usize,
    pub wrap_row: usize,
    pub left: usize,
    pub preview: bool,
    pub preview_scroll: usize,
}
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
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
        }
    }
    pub fn store_window(&mut self) {
        if !self.windows.is_empty() {
            let preview = self.windows[self.active_window].preview;
            let scroll = self.windows[self.active_window].preview_scroll;
            let mut w = self.capture_window();
            w.preview = preview;
            w.preview_scroll = scroll;
            self.windows[self.active_window] = w;
        }
    }
    pub fn split_window(&mut self, vertical: bool, preview: bool) {
        if self.windows.len() >= 4 {
            self.set_message("At most four panes are supported");
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
            self.windows.remove(self.active_window);
            self.active_window = self.active_window.min(self.windows.len() - 1);
            let w = self.windows[self.active_window].clone();
            if let Some(i) = self.buffers.iter().position(|b| b.id == w.buffer) {
                self.cur = i;
                self.set_cursor(w.cursor.0, w.cursor.1);
            }
            if self.windows.len() == 1 {
                self.windows.clear();
                self.active_window = 0;
            }
        }
    }
    pub fn window_key(&mut self, key: Key) {
        match key {
            Key::Char('v') => self.split_window(true, false),
            Key::Char('s') => self.split_window(false, false),
            Key::Char('w') | Key::Char('l') | Key::Char('j') => {
                if !self.windows.is_empty() {
                    self.focus_window((self.active_window + 1) % self.windows.len());
                }
            }
            Key::Char('h') | Key::Char('k') => {
                if !self.windows.is_empty() {
                    self.focus_window(
                        (self.active_window + self.windows.len() - 1) % self.windows.len(),
                    );
                }
            }
            Key::Char('c') => self.close_window(),
            Key::Char('o') => {
                self.windows.clear();
                self.active_window = 0;
            }
            _ => {}
        }
    }
    pub fn pane_rects(&self, cols: usize, rows: usize) -> Vec<Rect> {
        let count = self.windows.len().max(1);
        let height = rows.saturating_sub(1);
        (0..count)
            .map(|i| {
                if self.split_vertical {
                    let start = i * cols / count;
                    let end = (i + 1) * cols / count;
                    Rect {
                        x: start,
                        y: 0,
                        width: (end - start).saturating_sub(if i + 1 < count { 1 } else { 0 }),
                        height,
                    }
                } else {
                    let start = i * height / count;
                    let end = (i + 1) * height / count;
                    Rect {
                        x: 0,
                        y: start,
                        width: cols,
                        height: (end - start).saturating_sub(if i + 1 < count { 1 } else { 0 }),
                    }
                }
            })
            .collect()
    }
}
