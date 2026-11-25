//! Context menus: the right-click menu over the text, the gutter, the file
//! tree, the tabline, the winbar and terminal panes, and the dropdown a
//! breadcrumb opens. A menu is a list of items painted over everything else
//! (`render::draw_menu`). While one is open it takes every key and mouse
//! event: `j`/`k`/arrows move, Enter runs, `l`/Right opens a submenu,
//! `h`/Left goes back to the parent, Esc closes; the pointer highlights the
//! item under it, a click runs it, and releasing a right-button drag over an
//! item runs it too (press, drag, release, as in desktop menus).
//!
//! Every item runs something that already exists -- a registered action,
//! keys fed as if typed, an Ex command, a file-tree key -- so the menu
//! never grows a second implementation of a feature.
use crate::editor::Editor;
use crate::key::Key;
use crate::mode::Mode;
use crate::windows::Rect;
use unicode_width::UnicodeWidthStr;

/// What an item does when chosen.
#[derive(Clone)]
pub enum Cmd {
    /// A thin rule between groups; never selectable.
    Separator,
    /// An entry of the action registry (`crate::actions`), by id.
    Action(&'static str),
    /// Keys fed in as if typed (`"+y`, `za`, ...).
    Keys(&'static str),
    /// An Ex command line, without the `:`.
    Ex(String),
    /// A key handled by the focused file tree.
    TreeKey(Key),
    /// Moves the cursor to a buffer line and column (breadcrumb dropdowns).
    Jump(usize, usize),
    /// Opens a file.
    Open(std::path::PathBuf),
    /// Shows a path in the file tree.
    Reveal(std::path::PathBuf),
    /// Anything else.
    Call(fn(&mut Editor)),
    /// Opens a submenu beside this item.
    Sub(fn(&Editor) -> Vec<Item>),
}

#[derive(Clone)]
pub struct Item {
    pub label: String,
    /// Right-aligned and dimmed: the key that does the same, or `▸`.
    pub hint: String,
    pub cmd: Cmd,
    pub enabled: bool,
}

impl Item {
    pub fn new(label: impl Into<String>, cmd: Cmd) -> Self {
        let hint = match &cmd {
            Cmd::Action(id) => action_hint(id),
            Cmd::Sub(_) => "▸".into(),
            _ => String::new(),
        };
        Item {
            label: label.into(),
            hint,
            cmd,
            enabled: true,
        }
    }
    pub fn sep() -> Self {
        Item {
            label: String::new(),
            hint: String::new(),
            cmd: Cmd::Separator,
            enabled: false,
        }
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    fn selectable(&self) -> bool {
        self.enabled && !matches!(self.cmd, Cmd::Separator)
    }
}

/// The leader sequence an action is bound to by default, as shown in menus.
fn action_hint(id: &str) -> String {
    crate::actions::find(id)
        .filter(|a| !a.keys.is_empty())
        .map(|a| format!("<leader>{}", a.keys))
        .unwrap_or_default()
}

pub struct Menu {
    pub title: Option<String>,
    pub items: Vec<Item>,
    /// The screen cell the menu opens from: its top-left corner goes just
    /// below and right of it when there is room.
    pub at: (usize, usize),
    pub selected: Option<usize>,
    /// First item shown when the list is taller than the screen.
    pub top: usize,
    /// The menu a submenu was opened from, restored by `h`/Left.
    pub parent: Option<Box<Menu>>,
    /// Right-button press that opened the menu: releasing that button over
    /// an item runs it.
    pub opened_by_right: bool,
}

/// Rows a menu shows at most, so a long dropdown stays a dropdown.
const MAX_ROWS: usize = 32;

impl Menu {
    pub fn new(items: Vec<Item>, at: (usize, usize)) -> Self {
        Menu {
            title: None,
            items: trim_separators(items),
            at,
            selected: None,
            top: 0,
            parent: None,
            opened_by_right: false,
        }
    }

    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Item rows on screen (the rest scroll).
    fn visible(&self, rows: usize) -> usize {
        // Two border rows; keep the message line free.
        self.items
            .len()
            .min(MAX_ROWS)
            .min(rows.saturating_sub(3))
            .max(1)
    }

    fn inner_width(&self) -> usize {
        let widest = self
            .items
            .iter()
            .map(|i| {
                let hint = if i.hint.is_empty() {
                    0
                } else {
                    i.hint.width() + 3
                };
                i.label.width() + hint
            })
            .max()
            .unwrap_or(0);
        let title = self.title.as_ref().map_or(0, |t| t.width() + 2);
        widest.max(title).max(12) + 2
    }

    /// Where the menu (with its border) is drawn on a `cols`x`rows` screen.
    /// Opens below-right of `at`, flips above or left when it would not
    /// fit, and never covers the message line.
    pub fn rect(&self, cols: usize, rows: usize) -> Rect {
        let width = (self.inner_width() + 2).min(cols);
        let height = self.visible(rows) + 2;
        let usable = rows.saturating_sub(1);
        let (ax, ay) = self.at;
        let x = if ax + width <= cols {
            ax
        } else {
            cols.saturating_sub(width)
        };
        let y = if ay + 1 + height <= usable {
            ay + 1
        } else if ay >= height {
            ay - height
        } else {
            usable.saturating_sub(height)
        };
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// The item under screen cell `(x, y)`, if any (separators included).
    pub fn item_at(&self, cols: usize, rows: usize, x: usize, y: usize) -> Option<usize> {
        let r = self.rect(cols, rows);
        if x <= r.x || x + 1 >= r.x + r.width || y <= r.y || y + 1 >= r.y + r.height {
            return None;
        }
        let i = self.top + (y - r.y - 1);
        (i < self.items.len()).then_some(i)
    }

    pub fn contains(&self, cols: usize, rows: usize, x: usize, y: usize) -> bool {
        let r = self.rect(cols, rows);
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    }

    /// Moves the selection `dir` (+1/-1) steps to the next runnable item,
    /// wrapping, and scrolls it into view.
    fn step(&mut self, dir: isize, rows: usize) {
        let n = self.items.len();
        if n == 0 || !self.items.iter().any(Item::selectable) {
            return;
        }
        let mut i = match self.selected {
            Some(i) => i,
            None if dir > 0 => n - 1,
            None => 0,
        };
        loop {
            i = (i as isize + dir).rem_euclid(n as isize) as usize;
            if self.items[i].selectable() {
                break;
            }
        }
        self.select(i, rows);
    }

    fn select(&mut self, i: usize, rows: usize) {
        self.selected = Some(i);
        let vis = self.visible(rows);
        if i < self.top {
            self.top = i;
        } else if i >= self.top + vis {
            self.top = i + 1 - vis;
        }
    }

    fn scroll(&mut self, delta: isize, rows: usize) {
        let max = self.items.len().saturating_sub(self.visible(rows));
        self.top = self.top.saturating_add_signed(delta).min(max);
    }
}

/// Leading, trailing and doubled separators left by items a context
/// filtered out.
fn trim_separators(items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    for item in items {
        let sep = matches!(item.cmd, Cmd::Separator);
        if sep && out.last().is_none_or(|l| matches!(l.cmd, Cmd::Separator)) {
            continue;
        }
        out.push(item);
    }
    while out.last().is_some_and(|l| matches!(l.cmd, Cmd::Separator)) {
        out.pop();
    }
    out
}

fn screen(ed: &Editor) -> (usize, usize) {
    crate::display::size()
        .map(|(c, r)| (c as usize, r as usize))
        .unwrap_or((ed.screen_cols.max(1), ed.terminal_rows.max(1)))
}

/// Opens `menu`, replacing any menu already open.
pub fn open(ed: &mut Editor, mut menu: Menu) {
    if menu.items.is_empty() {
        return;
    }
    // Show the configured leader, not a placeholder.
    let leader = match ed.config.leader.as_str() {
        " " => "Space ".to_string(),
        l => l.to_string(),
    };
    for item in &mut menu.items {
        if let Some(rest) = item.hint.strip_prefix("<leader>") {
            item.hint = format!("{leader}{rest}");
        }
    }
    ed.context_menu_anchor = menu.at;
    ed.context_menu = Some(menu);
}

pub fn close(ed: &mut Editor) {
    ed.context_menu = None;
}

/// A key while a menu is open. Every key is the menu's.
pub fn handle_key(ed: &mut Editor, key: Key) {
    let (_, rows) = screen(ed);
    let Some(menu) = ed.context_menu.as_mut() else {
        return;
    };
    match key {
        Key::Esc | Key::Char('q') | Key::Ctrl('c') => close(ed),
        Key::Down | Key::Char('j') | Key::Ctrl('n') | Key::Tab => menu.step(1, rows),
        Key::Up | Key::Char('k') | Key::Ctrl('p') | Key::BackTab => menu.step(-1, rows),
        Key::Home | Key::Char('g') => {
            menu.selected = None;
            menu.step(1, rows);
        }
        Key::End | Key::Char('G') => {
            menu.selected = None;
            menu.step(-1, rows);
        }
        Key::PageDown | Key::Ctrl('d') => {
            for _ in 0..menu.visible(rows) / 2 {
                menu.step(1, rows);
            }
        }
        Key::PageUp | Key::Ctrl('u') => {
            for _ in 0..menu.visible(rows) / 2 {
                menu.step(-1, rows);
            }
        }
        Key::Left | Key::Char('h') => {
            if let Some(parent) = menu.parent.take() {
                ed.context_menu = Some(*parent);
            }
        }
        Key::Enter | Key::Char(' ') | Key::Char('l') | Key::Right => {
            let is_sub = menu
                .selected
                .and_then(|i| menu.items.get(i))
                .is_some_and(|i| matches!(i.cmd, Cmd::Sub(_)));
            if matches!(key, Key::Char('l') | Key::Right) && !is_sub {
                return;
            }
            if let Some(i) = menu.selected {
                run(ed, i);
            }
        }
        // A letter jumps to the next item starting with it.
        Key::Char(c) => {
            let n = menu.items.len();
            let start = menu.selected.map_or(0, |i| i + 1);
            let hit = (0..n).map(|k| (start + k) % n).find(|&i| {
                let item = &menu.items[i];
                item.selectable()
                    && item
                        .label
                        .chars()
                        .next()
                        .is_some_and(|f| f.eq_ignore_ascii_case(&c))
            });
            if let Some(i) = hit {
                menu.select(i, rows);
            }
        }
        _ => {}
    }
}

/// A mouse event while a menu is open. Returns whether the menu took it;
/// a right press outside the menu closes it and is passed on, so it opens
/// a fresh menu where it landed.
pub fn handle_mouse(ed: &mut Editor, m: crossterm::event::MouseEvent) -> bool {
    use crossterm::event::{MouseButton, MouseEventKind};
    let (cols, rows) = screen(ed);
    let (x, y) = (m.column as usize, m.row as usize);
    let Some(menu) = ed.context_menu.as_mut() else {
        return false;
    };
    let hit = menu.item_at(cols, rows, x, y);
    let inside = menu.contains(cols, rows, x, y);
    match m.kind {
        MouseEventKind::Moved | MouseEventKind::Drag(_) => {
            if let Some(i) = hit.filter(|&i| menu.items[i].selectable()) {
                menu.selected = Some(i);
            }
            true
        }
        MouseEventKind::Down(MouseButton::Left) => {
            match hit {
                Some(i) if menu.items[i].selectable() => run(ed, i),
                _ if inside => {}
                _ => close(ed),
            }
            true
        }
        MouseEventKind::Up(MouseButton::Right) => {
            if let Some(i) = hit.filter(|&i| menu.items[i].selectable()) {
                if menu.opened_by_right {
                    run(ed, i);
                }
            }
            if let Some(menu) = ed.context_menu.as_mut() {
                menu.opened_by_right = false;
            }
            true
        }
        MouseEventKind::Down(MouseButton::Right) => {
            if inside {
                if let Some(i) = hit.filter(|&i| menu.items[i].selectable()) {
                    run(ed, i);
                }
                return true;
            }
            close(ed);
            false
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            if inside {
                let d = if m.kind == MouseEventKind::ScrollDown {
                    1
                } else {
                    -1
                };
                menu.scroll(d, rows);
            } else {
                close(ed);
            }
            true
        }
        MouseEventKind::Up(_) => true,
        _ => {
            close(ed);
            true
        }
    }
}

/// Runs item `i` of the open menu, closing it first (a submenu replaces
/// it instead).
pub fn run(ed: &mut Editor, i: usize) {
    let (cols, rows) = screen(ed);
    let Some(menu) = ed.context_menu.take() else {
        return;
    };
    let Some(item) = menu.items.get(i).filter(|i| i.selectable()).cloned() else {
        ed.context_menu = Some(menu);
        return;
    };
    match item.cmd {
        Cmd::Separator => {}
        Cmd::Sub(build) => {
            let r = menu.rect(cols, rows);
            let row = r.y + 1 + i.saturating_sub(menu.top);
            let items = build(ed);
            let mut sub = Menu::new(items, (r.x + r.width, row.saturating_sub(1)));
            // Beside the parent when it fits, else over its right part.
            let w = sub.rect(cols, rows).width;
            if r.x + r.width + w > cols {
                sub.at.0 = r.x.saturating_sub(w);
            }
            sub.parent = Some(Box::new(menu));
            sub.step(1, rows);
            open(ed, sub);
        }
        Cmd::Action(id) => {
            if let Some(a) = crate::actions::find(id) {
                (a.handler)(ed);
            }
        }
        Cmd::Keys(keys) => feed(ed, keys),
        Cmd::Ex(line) => crate::command::run_ex(ed, &line),
        Cmd::TreeKey(k) => crate::filetree::handle_key(ed, k),
        Cmd::Jump(line, col) => {
            if matches!(ed.mode, Mode::Visual(_)) {
                ed.visual_anchor = None;
                ed.enter_normal();
            }
            ed.push_jump();
            ed.set_cursor(line, col);
        }
        Cmd::Open(path) => {
            if let Err(e) = ed.open_file(path) {
                ed.set_message(e.to_string());
            }
        }
        Cmd::Reveal(path) => ed.tree_reveal(&path),
        Cmd::Call(f) => {
            ed.context_menu_anchor = menu.at;
            f(ed)
        }
    }
}

fn feed(ed: &mut Editor, keys: &str) {
    for c in keys.chars() {
        ed.feed_key(Key::Char(c));
    }
}

// ---------------------------------------------------------------------------
// Menu contents
// ---------------------------------------------------------------------------

fn in_visual(ed: &Editor) -> bool {
    matches!(ed.mode, Mode::Visual(_))
}

/// The right-click menu over a buffer's text or gutter. The caller has
/// already focused the pane and placed the cursor (or kept the selection
/// the click landed in).
pub fn buffer_items(ed: &Editor) -> Vec<Item> {
    let sel = in_visual(ed);
    let has_path = ed.buf().path.is_some();
    let lsp = has_path;
    let readonly = false;
    vec![
        Item::new("Go to Definition", Cmd::Call(|ed| ed.request_definition()))
            .hint("gd")
            .enabled(lsp),
        Item::new("Go to References", Cmd::Action("lsp.references")).enabled(lsp),
        Item::new("Peek Definition", Cmd::Action("peek.definition")).enabled(lsp),
        Item::new("Go to", Cmd::Sub(goto_items)).enabled(lsp),
        Item::sep(),
        Item::new("Rename Symbol…", Cmd::Action("lsp.rename")).enabled(lsp && !readonly),
        Item::new("Code Actions…", Cmd::Action("lsp.actions")).enabled(lsp),
        Item::new(
            if sel {
                "Format Selection"
            } else {
                "Format Document"
            },
            Cmd::Action("lsp.format"),
        )
        .enabled(lsp && !readonly),
        Item::sep(),
        Item::new("Cut", Cmd::Keys(if sel { "\"+d" } else { "\"+dd" }))
            .hint(if sel { "\"+d" } else { "\"+dd" })
            .enabled(!readonly),
        Item::new("Copy", Cmd::Keys(if sel { "\"+y" } else { "\"+yy" })).hint(if sel {
            "\"+y"
        } else {
            "\"+yy"
        }),
        Item::new("Paste", Cmd::Keys(if sel { "\"+p" } else { "\"+P" }))
            .hint(if sel { "\"+p" } else { "\"+P" })
            .enabled(!readonly),
        Item::new("Select All", Cmd::Action("edit.select_all")),
        Item::sep(),
        Item::new("Copy GitHub Permalink", Cmd::Action("git.permalink")).enabled(has_path),
        Item::new("Open on GitHub", Cmd::Call(open_on_github)).enabled(has_path),
        Item::new("Copy Path", Cmd::Sub(copy_path_items)).enabled(has_path),
        Item::new("Git", Cmd::Sub(git_items)).enabled(has_path),
        Item::new("Find", Cmd::Sub(find_items)),
        Item::sep(),
        Item::new("Add Review Comment", Cmd::Action("review.note")),
        Item::new("Send to Agent", Cmd::Action("agent.context_send")),
    ]
}

fn goto_items(_: &Editor) -> Vec<Item> {
    vec![
        Item::new(
            "Type Definition",
            Cmd::Call(|ed| ed.request_type_definition()),
        )
        .hint("gy"),
        Item::new(
            "Implementations",
            Cmd::Call(|ed| ed.request_implementation()),
        )
        .hint("gI"),
        Item::new("Declaration", Cmd::Call(|ed| ed.request_declaration())).hint("gD"),
        Item::sep(),
        Item::new("Peek References", Cmd::Action("peek.references")),
        Item::new("Peek Type Definition", Cmd::Action("peek.type_definition")),
        Item::new("Peek Implementation", Cmd::Action("peek.implementation")),
        Item::new("Show Hover", Cmd::Action("peek.hover")),
        Item::sep(),
        Item::new("Document Outline", Cmd::Action("lsp.outline")),
    ]
}

fn copy_path_items(ed: &Editor) -> Vec<Item> {
    let line = ed.cursor().0 + 1;
    vec![
        Item::new(
            "Copy Relative Path",
            Cmd::Call(|ed| copy_buffer_path(ed, PathForm::Relative)),
        ),
        Item::new(
            "Copy Absolute Path",
            Cmd::Call(|ed| copy_buffer_path(ed, PathForm::Absolute)),
        ),
        Item::new(
            format!("Copy Relative Path:{line}"),
            Cmd::Call(|ed| copy_buffer_path(ed, PathForm::RelativeLine)),
        ),
        Item::new(
            "Copy File Name",
            Cmd::Call(|ed| copy_buffer_path(ed, PathForm::Name)),
        ),
        Item::sep(),
        Item::new("Reveal in Explorer", Cmd::Call(reveal_buffer)),
    ]
}

fn git_items(_: &Editor) -> Vec<Item> {
    vec![
        Item::new("Toggle Line Blame", Cmd::Action("git.blame_toggle")),
        Item::new("Preview Change", Cmd::Action("git.hunk_preview")),
        Item::new("Stage Change", Cmd::Action("git.hunk_stage")),
        Item::new("Revert Change", Cmd::Action("git.hunk_reset")),
        Item::sep(),
        Item::new(
            "Toggle Diff Overlay",
            Cmd::Action("git.diff_overlay_toggle"),
        ),
        Item::new("Git Status", Cmd::Action("git.status")),
    ]
}

fn find_items(ed: &Editor) -> Vec<Item> {
    let what = if in_visual(ed) { "Selection" } else { "Word" };
    vec![
        Item::new(
            format!("Find {what} in Files"),
            Cmd::Action("search.grep_word"),
        ),
        Item::new(
            format!("Replace {what} in Files"),
            Cmd::Action("search.replace_word"),
        ),
        Item::new(
            "Add Cursors to All Occurrences",
            Cmd::Action("cursors.add_all"),
        ),
    ]
}

#[derive(Clone, Copy)]
enum PathForm {
    Relative,
    Absolute,
    RelativeLine,
    Name,
}

fn path_text(ed: &Editor, path: &std::path::Path, form: PathForm) -> String {
    let rel = || {
        path.strip_prefix(&ed.project_root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    match form {
        PathForm::Relative => rel(),
        PathForm::Absolute => path.display().to_string(),
        PathForm::RelativeLine => format!("{}:{}", rel(), ed.cursor().0 + 1),
        PathForm::Name => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// Copies `text` to the system clipboard (the `+` register) and says so.
pub fn copy_text(ed: &mut Editor, text: String) {
    ed.set_message(format!("Copied {text}"));
    ed.registers.set(Some('+'), text, false);
}

fn copy_buffer_path(ed: &mut Editor, form: PathForm) {
    let Some(path) = ed.buf().path.clone() else {
        return ed.set_message("This buffer has no file on disk");
    };
    let text = path_text(ed, &path, form);
    copy_text(ed, text);
}

fn reveal_buffer(ed: &mut Editor) {
    if let Some(path) = ed.buf().path.clone() {
        ed.tree_reveal(&path);
    }
}

/// Opens the permalink of the cursor line (or the Visual selection's
/// lines) in the browser, without touching the clipboard.
fn open_on_github(ed: &mut Editor) {
    let Some(path) = ed.buf().path.clone() else {
        return ed.set_message("This buffer has no file on disk");
    };
    let cursor = ed.cursor().0;
    let start = match (ed.mode, ed.visual_anchor) {
        (Mode::Visual(_), Some(a)) => a.0,
        _ => cursor,
    };
    if in_visual(ed) {
        ed.visual_anchor = None;
        ed.enter_normal();
    }
    match ed.permalink_url(&path, Some((start, cursor)), None) {
        Ok(url) => crate::normal::open_with_system(ed, &url),
        Err(e) => ed.set_message(e),
    }
}

/// The right-click menu of the file tree; the row under the pointer is
/// already selected.
pub fn tree_items(ed: &Editor) -> Vec<Item> {
    let node = ed.file_tree.as_ref().and_then(|t| t.selected());
    let is_dir = node.is_some_and(|n| n.is_dir);
    let any = node.is_some();
    vec![
        Item::new(
            if is_dir { "Expand / Collapse" } else { "Open" },
            Cmd::TreeKey(Key::Enter),
        )
        .hint("o")
        .enabled(any),
        Item::new("Open to the Side", Cmd::TreeKey(Key::Char('s')))
            .hint("s")
            .enabled(any && !is_dir),
        Item::new("Open Below", Cmd::TreeKey(Key::Char('S')))
            .hint("S")
            .enabled(any && !is_dir),
        Item::new("Open in New Tab", Cmd::TreeKey(Key::Char('T')))
            .hint("T")
            .enabled(any && !is_dir),
        Item::new("Preview", Cmd::TreeKey(Key::Char('v')))
            .hint("v")
            .enabled(any && !is_dir),
        Item::sep(),
        Item::new("New File or Folder…", Cmd::TreeKey(Key::Char('a'))).hint("a"),
        Item::new("Rename…", Cmd::TreeKey(Key::Char('r')))
            .hint("r")
            .enabled(any),
        Item::new("Cut", Cmd::TreeKey(Key::Char('x')))
            .hint("x")
            .enabled(any),
        Item::new("Copy", Cmd::TreeKey(Key::Char('y')))
            .hint("y")
            .enabled(any),
        Item::new("Paste", Cmd::TreeKey(Key::Char('p'))).hint("p"),
        Item::sep(),
        Item::new(
            "Copy Relative Path",
            Cmd::Call(|ed| copy_tree_path(ed, PathForm::Relative)),
        )
        .hint("Y")
        .enabled(any),
        Item::new(
            "Copy Absolute Path",
            Cmd::Call(|ed| copy_tree_path(ed, PathForm::Absolute)),
        )
        .enabled(any),
        Item::new(
            "Copy GitHub Permalink",
            Cmd::Call(|ed| tree_permalink(ed, false)),
        )
        .enabled(any),
        Item::new("Open on GitHub", Cmd::Call(|ed| tree_permalink(ed, true))).enabled(any),
        Item::new("Reveal in File Manager", Cmd::Call(reveal_in_file_manager)).enabled(any),
        Item::sep(),
        Item::new("Move to Trash", Cmd::Call(tree_trash))
            .hint("t")
            .enabled(any),
        Item::new("Delete Permanently…", Cmd::Call(confirm_tree_delete))
            .hint("d")
            .enabled(any),
        Item::sep(),
        Item::new("Collapse All", Cmd::TreeKey(Key::Char('W'))).hint("W"),
        Item::new("Refresh", Cmd::TreeKey(Key::Char('R'))).hint("R"),
        Item::new("Toggle Hidden Files", Cmd::TreeKey(Key::Char('.'))).hint("."),
    ]
}

fn tree_path(ed: &Editor) -> Option<std::path::PathBuf> {
    ed.file_tree
        .as_ref()
        .and_then(|t| t.selected())
        .map(|n| n.path.clone())
}

fn copy_tree_path(ed: &mut Editor, form: PathForm) {
    if let Some(path) = tree_path(ed) {
        let text = path_text(ed, &path, form);
        copy_text(ed, text);
    }
}

fn tree_permalink(ed: &mut Editor, open: bool) {
    let Some(path) = tree_path(ed) else { return };
    match ed.permalink_url(&path, None, None) {
        Ok(url) if open => crate::normal::open_with_system(ed, &url),
        Ok(url) => {
            ed.set_message(format!("Copied permalink: {url}"));
            ed.registers.set(Some('+'), url, false);
        }
        Err(e) => ed.set_message(e),
    }
}

fn reveal_in_file_manager(ed: &mut Editor) {
    let Some(path) = tree_path(ed) else { return };
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().map(|p| p.to_path_buf()).unwrap_or(path)
    };
    crate::normal::open_with_system(ed, &dir.display().to_string());
}

/// Trash is undoable (`U` restores), so the menu skips the tree's own
/// press-twice confirmation.
fn tree_trash(ed: &mut Editor) {
    crate::filetree::handle_key(ed, Key::Char('t'));
    crate::filetree::handle_key(ed, Key::Char('t'));
}

fn confirm_tree_delete(ed: &mut Editor) {
    let what = tree_path(ed)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let at = ed.context_menu_anchor;
    let items = vec![
        Item::new("Cancel", Cmd::Call(|_| {})),
        Item::new(
            "Delete Permanently",
            Cmd::Call(|ed| {
                crate::filetree::handle_key(ed, Key::Char('d'));
                crate::filetree::handle_key(ed, Key::Char('d'));
            }),
        ),
    ];
    let mut menu = Menu::new(items, at).titled(format!("Delete {what}? No undo"));
    menu.selected = Some(0);
    open(ed, menu);
}

/// The tabline's menu, for the tab under the pointer (already switched to).
pub fn tab_items(ed: &Editor) -> Vec<Item> {
    let many = ed.tabs.len() > 1;
    vec![
        Item::new("New Tab", Cmd::Action("tabs.new")),
        Item::new("Close Tab", Cmd::Action("tabs.close")).enabled(many),
        Item::new("Close Other Tabs", Cmd::Call(|ed| ed.tab_only())).enabled(many),
        Item::sep(),
        Item::new("Next Tab", Cmd::Action("tabs.next")).enabled(many),
        Item::new("Previous Tab", Cmd::Action("tabs.previous")).enabled(many),
    ]
}

/// A terminal or agent pane's menu.
pub fn terminal_items(ed: &Editor) -> Vec<Item> {
    let selected = ed.term_select.as_ref().is_some_and(|s| s.span().is_some());
    vec![
        Item::new("Copy", Cmd::Call(|ed| ed.copy_term_selection())).enabled(selected),
        Item::new("Paste", Cmd::Call(paste_into_terminal)),
    ]
}

fn paste_into_terminal(ed: &mut Editor) {
    let text = ed.registers.get(Some('+')).map(|e| e.text.clone());
    match text {
        Some(t) if !t.is_empty() => ed.insert_paste(&t),
        _ => ed.set_message("The clipboard is empty"),
    }
}

/// The winbar's menu (path part).
pub fn winbar_items(ed: &Editor) -> Vec<Item> {
    let has_path = ed.buf().path.is_some();
    vec![
        Item::new(
            "Copy Relative Path",
            Cmd::Call(|ed| copy_buffer_path(ed, PathForm::Relative)),
        )
        .enabled(has_path),
        Item::new(
            "Copy Absolute Path",
            Cmd::Call(|ed| copy_buffer_path(ed, PathForm::Absolute)),
        )
        .enabled(has_path),
        Item::new("Copy GitHub Permalink", Cmd::Action("git.permalink")).enabled(has_path),
        Item::new("Open on GitHub", Cmd::Call(open_on_github)).enabled(has_path),
        Item::new("Reveal in Explorer", Cmd::Call(reveal_buffer)).enabled(has_path),
        Item::sep(),
        Item::new("Split Right", Cmd::Action("window.vsplit")),
        Item::new("Split Down", Cmd::Action("window.split")),
        Item::new("Close Pane", Cmd::Ex("close".into())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<Item> {
        (0..n)
            .map(|i| Item::new(format!("item {i}"), Cmd::Call(|_| {})))
            .collect()
    }

    #[test]
    fn separators_never_lead_trail_or_double() {
        let v = trim_separators(vec![
            Item::sep(),
            Item::new("a", Cmd::Call(|_| {})),
            Item::sep(),
            Item::sep(),
            Item::new("b", Cmd::Call(|_| {})),
            Item::sep(),
        ]);
        let labels: Vec<_> = v.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["a", "", "b"]);
    }

    #[test]
    fn rect_opens_below_right_and_flips_at_the_edges() {
        let m = Menu::new(items(4), (5, 2));
        let r = m.rect(80, 24);
        assert_eq!((r.x, r.y, r.height), (5, 3, 6));
        // Near the bottom-right corner it opens above and left.
        let m = Menu::new(items(4), (78, 20));
        let r = m.rect(80, 24);
        assert!(r.x + r.width <= 80);
        assert_eq!(r.y + r.height, 20);
        // Never over the message line, even when nothing fits.
        let m = Menu::new(items(40), (0, 10));
        let r = m.rect(80, 12);
        assert!(r.y + r.height <= 11, "{} {}", r.y, r.height);
    }

    #[test]
    fn item_at_maps_rows_through_the_scroll_offset() {
        let mut m = Menu::new(items(30), (0, 0));
        let r = m.rect(80, 12);
        assert_eq!(m.item_at(80, 12, r.x + 2, r.y + 1), Some(0));
        assert_eq!(m.item_at(80, 12, r.x, r.y + 1), None, "border");
        m.scroll(5, 12);
        assert_eq!(m.item_at(80, 12, r.x + 2, r.y + 1), Some(5));
    }

    #[test]
    fn step_skips_separators_and_disabled_items_and_wraps() {
        let mut m = Menu::new(
            vec![
                Item::new("a", Cmd::Call(|_| {})),
                Item::sep(),
                Item::new("b", Cmd::Call(|_| {})).enabled(false),
                Item::new("c", Cmd::Call(|_| {})),
            ],
            (0, 0),
        );
        m.step(1, 24);
        assert_eq!(m.selected, Some(0));
        m.step(1, 24);
        assert_eq!(m.selected, Some(3));
        m.step(1, 24);
        assert_eq!(m.selected, Some(0));
        m.step(-1, 24);
        assert_eq!(m.selected, Some(3));
    }
}
