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
    /// `true` when this pane shows the file tree sidebar instead of
    /// `buffer`'s text. Never persisted, same reasoning as `terminal`.
    #[serde(skip)]
    pub file_tree: bool,
    /// `true` when this pane shows the outline/symbol sidebar. Never
    /// persisted, same reasoning as `terminal`/`file_tree`.
    #[serde(skip)]
    pub outline: bool,
}
/// A tab's saved pane-tree state, restored into the live
/// `windows`/`window_layout`/`active_window`/`cur` fields on switch. Never
/// persisted across sessions, same as terminals: a saved session restores
/// tab 1's layout only (see `crate::session`).
#[derive(Clone, Default)]
pub struct Tab {
    pub windows: Vec<Window>,
    pub window_layout: Option<Layout>,
    pub active_window: usize,
    pub cur: usize,
}

#[derive(Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
pub fn default_ratio() -> f32 {
    0.5
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Layout {
    Leaf(usize),
    Split {
        vertical: bool,
        first: Box<Layout>,
        second: Box<Layout>,
        /// Fraction of the split's space given to `first` (0..1). Defaulted for
        /// sessions written before resizable splits existed.
        #[serde(default = "default_ratio")]
        ratio: f32,
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
                    ratio: 0.5,
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
                ratio,
            } => match (first.remove(index), second.remove(index)) {
                (Some(a), Some(b)) => Some(Self::Split {
                    vertical,
                    first: Box::new(a),
                    second: Box::new(b),
                    ratio,
                }),
                (a, b) => a.or(b),
            },
        }
    }
    /// Whether pane `index` is anywhere in this subtree.
    fn contains(&self, index: usize) -> bool {
        match self {
            Self::Leaf(i) => *i == index,
            Self::Split { first, second, .. } => {
                first.contains(index) || second.contains(index)
            }
        }
    }
    /// Grow the side containing `active` by `delta` at the nearest ancestor
    /// split of the given orientation. Returns true if a resize happened.
    fn resize(&mut self, active: usize, vertical: bool, delta: f32) -> bool {
        if let Self::Split {
            vertical: v,
            first,
            second,
            ratio,
        } = self
        {
            let in_first = first.contains(active);
            let in_second = second.contains(active);
            // Deepest matching split wins: try children first.
            if in_first && first.resize(active, vertical, delta) {
                return true;
            }
            if in_second && second.resize(active, vertical, delta) {
                return true;
            }
            if *v == vertical && (in_first || in_second) {
                let d = if in_first { delta } else { -delta };
                *ratio = (*ratio + d).clamp(0.1, 0.9);
                return true;
            }
        }
        false
    }
    /// Reset every split back to an even 50/50.
    fn equalize(&mut self) {
        if let Self::Split {
            first,
            second,
            ratio,
            ..
        } = self
        {
            *ratio = 0.5;
            first.equalize();
            second.equalize();
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
                ratio,
            } => {
                let mut a = r;
                let mut b = r;
                // `split_at` is the first pane's size plus the 1-cell separator.
                // At the default 0.5 this is exactly the old `size / 2`, so an
                // un-resized layout renders byte-identically to before.
                let ratio = *ratio;
                let split = |size: usize| -> usize {
                    if (ratio - 0.5).abs() < 1e-6 {
                        size / 2
                    } else {
                        ((size as f32) * ratio).round() as usize
                    }
                    .clamp(1, size.saturating_sub(1).max(1))
                };
                if *vertical {
                    let at = split(r.width);
                    a.width = at.saturating_sub(1);
                    b.x += at;
                    b.width -= at;
                } else {
                    let at = split(r.height);
                    a.height = at.saturating_sub(1);
                    b.y += at;
                    b.height -= at;
                }
                first.rects(a, out);
                second.rects(b, out);
            }
        }
    }
    pub fn resize_active(&mut self, active: usize, vertical: bool, delta: f32) -> bool {
        self.resize(active, vertical, delta)
    }
    pub fn equalize_all(&mut self) {
        self.equalize()
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
            file_tree: false,
            outline: false,
        }
    }
    pub fn store_window(&mut self) {
        if !self.windows.is_empty() {
            let preview = self.windows[self.active_window].preview;
            let scroll = self.windows[self.active_window].preview_scroll;
            let terminal = self.windows[self.active_window].terminal;
            let file_tree = self.windows[self.active_window].file_tree;
            let outline = self.windows[self.active_window].outline;
            let mut w = self.capture_window();
            w.preview = preview;
            w.preview_scroll = scroll;
            w.terminal = terminal;
            w.file_tree = file_tree;
            w.outline = outline;
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
    /// Switches focus to pane `index` and syncs the active buffer/cursor
    /// context (which buffer is current, its cursor, and its scroll/wrap
    /// offsets) to that pane. Split out of `focus_window` so the mouse
    /// handler can reuse the exact same buffer-sync on a click without also
    /// resetting mode/completion state, which it manages itself.
    pub fn focus_pane_buffer(&mut self, index: usize) {
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
    }
    pub fn focus_window(&mut self, index: usize) {
        if self.windows.is_empty() {
            return;
        }
        self.focus_pane_buffer(index);
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
    /// The detach half of an agent session's toggle (`,gc`/`:claude`
    /// etc.): removes the active window's pane exactly like
    /// `close_window`, but never kills a terminal it hosts -- the
    /// terminal keeps running in `self.terminals`, just no longer
    /// referenced by any window in this tab, until something (the same
    /// toggle, or `:agents`) reattaches it. A no-op with just one
    /// window/pane, same as `close_window`'s own refusal to close the
    /// last one.
    pub fn detach_window(&mut self) {
        if self.windows.len() > 1 {
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
        self.enter_normal();
    }
    fn capture_tab(&self) -> Tab {
        Tab {
            windows: self.windows.clone(),
            window_layout: self.window_layout.clone(),
            active_window: self.active_window,
            cur: self.cur,
        }
    }

    fn load_tab(&mut self, tab: Tab) {
        self.windows = tab.windows;
        self.window_layout = tab.window_layout;
        self.active_window = tab.active_window;
        self.cur = tab.cur.min(self.buffers.len().saturating_sub(1));
        if let Some(w) = self.windows.get(self.active_window) {
            if let Some(i) = self.buffers.iter().position(|b| b.id == w.buffer) {
                self.cur = i;
            }
        }
    }

    /// `:tabnew`: opens a new tab showing the current buffer, after the
    /// active one (matching `:tabnew`'s placement in real Vim).
    pub fn new_tab(&mut self) {
        self.store_window();
        let cur = self.cur;
        self.tabs[self.active_tab] = self.capture_tab();
        self.tabs.insert(
            self.active_tab + 1,
            Tab {
                windows: Vec::new(),
                window_layout: None,
                active_window: 0,
                cur,
            },
        );
        self.active_tab += 1;
        self.windows.clear();
        self.window_layout = None;
        self.active_window = 0;
        self.cur = cur;
    }

    /// `:tabclose`/`gT`'s sibling: kills any terminals in the closing
    /// tab's panes first, so a tab full of PTYs can never leak them.
    pub fn close_tab(&mut self) {
        if self.tabs.len() <= 1 {
            self.set_message("Cannot close the last tab");
            return;
        }
        let terminals: Vec<u64> = self.windows.iter().filter_map(|w| w.terminal).collect();
        for id in terminals {
            self.shutdown_terminal(id);
        }
        self.tabs.remove(self.active_tab);
        self.active_tab = self.active_tab.min(self.tabs.len() - 1);
        let tab = self.tabs[self.active_tab].clone();
        self.load_tab(tab);
    }

    pub fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }
        self.store_window();
        self.tabs[self.active_tab] = self.capture_tab();
        self.active_tab = index;
        let tab = self.tabs[index].clone();
        self.load_tab(tab);
    }

    /// `:tabonly`: closes every tab except the current one, killing any
    /// terminals that were running in the discarded tabs' panes.
    pub fn tab_only(&mut self) {
        self.store_window();
        self.tabs[self.active_tab] = self.capture_tab();
        let survivor = self.tabs.remove(self.active_tab);
        let terminals: Vec<u64> = self
            .tabs
            .iter()
            .flat_map(|t| t.windows.iter().filter_map(|w| w.terminal))
            .collect();
        for id in terminals {
            self.shutdown_terminal(id);
        }
        self.tabs = vec![survivor];
        self.active_tab = 0;
    }

    pub fn next_tab(&mut self) {
        self.switch_tab((self.active_tab + 1) % self.tabs.len());
    }

    pub fn prev_tab(&mut self) {
        self.switch_tab((self.active_tab + self.tabs.len() - 1) % self.tabs.len());
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
                // `pane_rects` expects the full terminal size (it subtracts
                // the message line and tabline itself); `self.screen_rows`
                // is already the active pane's body height, so passing it
                // here would subtract `1 + tabline` a second time and yield
                // rects ~2 rows short, picking the wrong neighbour near
                // edges. Use the real terminal size, like the mouse handler.
                let (cols, rows) = crossterm::terminal::size()
                    .map(|(c, r)| (c as usize, r as usize))
                    .unwrap_or((self.screen_cols.max(1), self.screen_rows.max(1) + 2));
                let rects = self.pane_rects(cols, rows);
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
            // Resize the active split: `>`/`<` width (vertical split), `+`/`-`
            // height (horizontal split), `=` equalize all.
            Key::Char('>') => self.resize_split(true, 0.05),
            Key::Char('<') => self.resize_split(true, -0.05),
            Key::Char('+') => self.resize_split(false, 0.05),
            Key::Char('-') => self.resize_split(false, -0.05),
            Key::Char('=') => {
                if let Some(l) = &mut self.window_layout {
                    l.equalize_all();
                }
            }
            _ => {}
        }
    }

    fn resize_split(&mut self, vertical: bool, delta: f32) {
        let active = self.active_window;
        if let Some(l) = &mut self.window_layout {
            l.resize_active(active, vertical, delta);
        }
    }
    pub fn pane_rects(&self, cols: usize, rows: usize) -> Vec<Rect> {
        let tabline = usize::from(self.tabs.len() > 1);
        let r = Rect {
            x: 0,
            y: tabline,
            width: cols,
            height: rows.saturating_sub(1 + tabline),
        };
        let mut out = vec![r; self.windows.len().max(1)];
        if let Some(layout) = &self.window_layout {
            layout.rects(r, &mut out);
        }
        out
    }
}
