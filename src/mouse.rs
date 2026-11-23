//! Mouse support: click to position the cursor and focus a pane, drag to
//! start a Visual selection, wheel to scroll, Ctrl-click for
//! go-to-definition, and drag a split's border to resize it. Markdown previews
//! handle wheel scrolling independently from their source buffer. Statuslines
//! and the message line are selectable: drag to select and copy, double-click
//! copies a word, triple-click the whole row. So is the output of agent and
//! terminal panes (`crate::termselect`).
use crate::editor::Editor;
use crate::mode::{Mode, VisualKind};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

pub fn handle(ed: &mut Editor, m: MouseEvent) {
    if matches!(ed.mode, Mode::MarkdownPreview)
        && !matches!(
            m.kind,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        )
    {
        return;
    }
    // Underline the terminal link under the pointer (agent output, compiler
    // errors, diffs) so it reads as clickable.
    if m.kind == MouseEventKind::Moved {
        let (cols, rows) = grid(ed);
        ed.hover_terminal_link(cols, rows, m.column as usize, m.row as usize, m.modifiers);
    }
    // Releases, drags, motion and other buttons over the focused terminal go
    // to its program when it asked for mouse reports (the left press is
    // routed through `down`, which also handles focusing).
    let other_terminal_event = !matches!(
        m.kind,
        MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollUp
    );
    if other_terminal_event
        && ed.mode == Mode::Terminal
        && ed.resize_drag.is_none()
        && !ed.chrome_select.is_some_and(|s| s.dragging)
        && !ed.term_select.as_ref().is_some_and(|s| s.dragging)
        && pane_at(ed, &m) == Some(ed.active_window)
        && terminal_mouse(ed, &m)
    {
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => down(ed, m),
        MouseEventKind::Drag(MouseButton::Left) => drag(ed, m),
        MouseEventKind::Up(MouseButton::Left) => {
            ed.resize_drag = None;
            ed.minimap_drag = None;
            chrome_up(ed);
            let (cols, rows) = grid(ed);
            ed.term_select_up(cols, rows);
        }
        MouseEventKind::Moved => hover(ed, &m),
        MouseEventKind::ScrollDown => wheel(ed, &m, 3),
        MouseEventKind::ScrollUp => wheel(ed, &m, -3),
        _ => {}
    }
}

/// The terminal grid size, preferring the live ioctl and falling back to the
/// last-known screen dimensions (the same source `locate`/`pane_at` use).
fn grid(ed: &Editor) -> (usize, usize) {
    crate::display::size()
        .map(|(c, r)| (c as usize, r as usize))
        .unwrap_or((ed.screen_cols.max(1), ed.terminal_rows.max(1)))
}

fn locate(ed: &Editor, m: &MouseEvent) -> Option<(usize, usize, usize)> {
    let (cols, rows) = grid(ed);
    crate::render::locate_click(ed, cols, rows, m.column as usize, m.row as usize)
}

/// The pane index under the pointer, using the same rect hit-test `locate`
/// does but without requiring an editing mode or a resolved line/column --
/// wheel scrolling only needs to know which split it is over.
fn pane_at(ed: &Editor, m: &MouseEvent) -> Option<usize> {
    let (cols, rows) = grid(ed);
    let (x, y) = (m.column as usize, m.row as usize);
    ed.pane_rects(cols, rows)
        .iter()
        .position(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
}

fn down(ed: &mut Editor, m: MouseEvent) {
    // Tour controls use the same commands as the keyboard. Consume their
    // presses so dragging a control cannot select hidden buffer text.
    let (cols, rows) = grid(ed);
    if ed.tour_chrome_rows(rows) > 0
        && matches!(ed.mode, Mode::Normal | Mode::Insert | Mode::Visual(_))
    {
        let footer = rows - 2 - usize::from(ed.config.global_statusline);
        if m.row as usize == footer {
            use unicode_width::UnicodeWidthStr;
            let mut x = 0;
            for control in ed.tour_controls(cols) {
                let end = x + control.label.width();
                if (m.column as usize) >= x
                    && (m.column as usize) < end
                    && !control.command.is_empty()
                {
                    ed.enter_normal();
                    crate::command::run_ex(ed, control.command);
                    break;
                }
                x = end + 2;
            }
            ed.mouse_down_at = None;
            return;
        }
    }
    // A press on a split divider starts a drag-resize rather than a click.
    let (cols, rows) = grid(ed);
    if let Some(path) = ed.divider_at(cols, rows, m.column as usize, m.row as usize) {
        ed.resize_drag = Some(path);
        ed.mouse_down_at = None;
        return;
    }
    ed.resize_drag = None;
    ed.minimap_drag = None;
    if chrome_down(ed, &m) {
        ed.mouse_down_at = None;
        return;
    }
    ed.chrome_select = None;
    ed.chrome_last_click = None;
    if minimap_down(ed, &m) {
        ed.mouse_down_at = None;
        return;
    }
    let editing = matches!(
        ed.mode,
        Mode::Normal | Mode::Insert | Mode::Visual(_) | Mode::Terminal
    );
    ed.term_select = None;
    // In an agent pane (or a program that leaves the mouse alone) a press
    // starts a text selection; released without moving, it opens the link
    // under it.
    if editing {
        if let Some(p) = pane_at(ed, &m).filter(|&p| ed.terminal_selects(p)) {
            if let Some((x, y)) = terminal_cell(ed, p, &m) {
                let link = ed
                    .terminal_link_at(
                        cols,
                        rows,
                        m.column as usize,
                        m.row as usize,
                        crate::termlinks::CLICK_HISTORY,
                    )
                    .filter(|(lp, _)| *lp == p)
                    .map(|(_, link)| link);
                if p != ed.active_window || ed.mode != Mode::Terminal {
                    if matches!(ed.mode, Mode::Visual(_) | Mode::Insert) {
                        ed.visual_anchor = None;
                        ed.enter_normal();
                    }
                    ed.focus_pane_buffer(p);
                    ed.mode = Mode::Terminal;
                }
                ed.term_select_down(p, y as u16, x as u16, link);
                ed.mouse_down_at = None;
                return;
            }
        }
    }
    // A file reference, diff row or URL in a terminal pane opens it (here,
    // a Ctrl-click in a program that takes the mouse itself).
    if editing && ed.click_terminal_link(cols, rows, m.column as usize, m.row as usize, m.modifiers)
    {
        ed.mouse_down_at = None;
        return;
    }
    if terminal_mouse(ed, &m) {
        return;
    }
    // A click outside the focused terminal leaves Terminal mode, so the
    // pane under the pointer takes the click as usual.
    if ed.mode == Mode::Terminal {
        ed.mode = Mode::Normal;
    }
    // The file tree sidebar maps rows to nodes, not buffer lines.
    if let Some(p) = pane_at(ed, &m).filter(|&p| ed.windows.get(p).is_some_and(|w| w.file_tree)) {
        if matches!(ed.mode, Mode::Normal | Mode::Insert | Mode::Visual(_)) {
            if !matches!(ed.mode, Mode::Normal) {
                ed.enter_normal();
            }
            let r = ed.pane_rects(cols, rows)[p];
            ed.mouse_down_at = None;
            ed.tree_click(
                p,
                (m.row as usize).saturating_sub(r.y),
                (m.column as usize).saturating_sub(r.x),
            );
        }
        return;
    }
    let Some((pane, line, col)) = locate(ed, &m) else {
        return;
    };
    if matches!(ed.mode, Mode::Visual(_)) {
        ed.visual_anchor = None;
        ed.enter_normal();
    }
    // Focus the clicked pane the same way keyboard focus does, so the active
    // buffer/cursor context follows the pane -- otherwise `set_cursor` would
    // move the cursor in the previously-active buffer and swap panes' text.
    ed.focus_pane_buffer(pane);
    ed.set_cursor(line, col);
    if m.modifiers.contains(KeyModifiers::CONTROL) {
        ed.request_definition();
        ed.mouse_down_at = None;
    } else {
        ed.mouse_down_at = Some((line, col));
    }
}

fn drag(ed: &mut Editor, m: MouseEvent) {
    // A drag begun on a divider resizes the split instead of selecting text.
    if let Some(path) = ed.resize_drag.clone() {
        let (cols, rows) = grid(ed);
        ed.drag_divider_to(&path, cols, rows, m.column as usize, m.row as usize);
        return;
    }
    if let Some(pane) = ed
        .term_select
        .as_ref()
        .filter(|s| s.dragging)
        .map(|s| s.pane)
    {
        // Clamp to the pane's program area, so dragging past an edge
        // selects to it.
        let (cols, rows) = grid(ed);
        if let Some(r) = ed.pane_rects(cols, rows).get(pane).copied() {
            let body = crate::render::terminal_body_rect(ed, r);
            if body.width > 0 && body.height > 0 {
                let x = (m.column as usize).clamp(body.x, body.x + body.width - 1) - body.x;
                let y = (m.row as usize).clamp(body.y, body.y + body.height - 1) - body.y;
                ed.term_select_drag(y as u16, x as u16);
            }
        }
        return;
    }
    if let Some(sel) = ed.chrome_select.filter(|s| s.dragging) {
        let x = ed
            .selectable_rows
            .borrow()
            .iter()
            .find(|r| r.y == sel.y)
            .map(|r| (m.column as usize).clamp(r.x, r.x + r.width.max(1) - 1));
        if let (Some(x), Some(s)) = (x, ed.chrome_select.as_mut()) {
            s.head = x;
        }
        return;
    }
    if let Some(d) = ed.minimap_drag {
        let (cols, rows) = grid(ed);
        let (x, y) = (m.column as usize, m.row as usize);
        if let Some(h) = crate::minimap::hit(ed, cols, rows, x, y, Some(d.pane)) {
            set_pane_top(ed, d.pane, crate::minimap::drag_target(&h, &d));
        }
        return;
    }
    let Some((pane, line, col)) = locate(ed, &m) else {
        return;
    };
    ed.focus_pane_buffer(pane);
    if !matches!(ed.mode, Mode::Visual(_)) {
        let Some(anchor) = ed.mouse_down_at else {
            return;
        };
        if anchor == (line, col) {
            return; // hasn't moved yet: not a drag
        }
        ed.visual_anchor = Some(anchor);
        ed.mode = Mode::Visual(VisualKind::Char);
    }
    ed.set_cursor(line, col);
}

/// A text selection on a selectable chrome row (see
/// `render::SelectableRow`), in screen columns. `anchor` is where the press
/// landed and `head` follows the drag; both ends are inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromeSelect {
    pub y: usize,
    pub anchor: usize,
    pub head: usize,
    /// The button is still down and drags extend the selection.
    pub dragging: bool,
}

impl ChromeSelect {
    /// The selected `[from, to)` columns if this selection is on row `y`.
    /// A press that has not moved selects nothing.
    pub fn span_on(&self, y: usize) -> Option<(usize, usize)> {
        (y == self.y && (self.anchor != self.head || !self.dragging))
            .then(|| (self.anchor.min(self.head), self.anchor.max(self.head) + 1))
    }
}

/// The last press on a chrome row: when, where, and how many presses in a
/// row landed there (1 = single, 2 = double, 3 = triple click).
#[derive(Clone, Copy, Debug)]
pub struct ChromeClick {
    pub at: std::time::Instant,
    pub y: usize,
    pub x: usize,
    pub count: u8,
}

pub(crate) const MULTI_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// A press on a statusline or the message line starts a text selection
/// there. A double click selects the word under the pointer and a triple
/// click the whole row; both copy at once. Returns whether the press
/// landed on such a row.
fn chrome_down(ed: &mut Editor, m: &MouseEvent) -> bool {
    let (x, y) = (m.column as usize, m.row as usize);
    let Some(row) = ed
        .selectable_rows
        .borrow()
        .iter()
        .find(|r| r.y == y && x >= r.x && x < r.x + r.width)
        .cloned()
    else {
        return false;
    };
    let now = std::time::Instant::now();
    let count = match ed.chrome_last_click {
        Some(c) if c.y == y && c.x == x && now.duration_since(c.at) < MULTI_CLICK => {
            c.count % 3 + 1
        }
        _ => 1,
    };
    ed.chrome_last_click = Some(ChromeClick {
        at: now,
        y,
        x,
        count,
    });
    let span = match count {
        2 => row.word_at(x),
        3 => Some((row.x, row.x + row.width)),
        _ => None,
    };
    ed.chrome_select = Some(match span {
        Some((from, to)) => ChromeSelect {
            y,
            anchor: from,
            head: to - 1,
            dragging: false,
        },
        None => ChromeSelect {
            y,
            anchor: x,
            head: x,
            dragging: count == 1,
        },
    });
    if span.is_some() {
        copy_chrome_selection(ed, &row);
    }
    true
}

/// Releasing a drag over a chrome row copies what it selected.
fn chrome_up(ed: &mut Editor) {
    let Some(sel) = ed.chrome_select.as_mut().filter(|s| s.dragging) else {
        return;
    };
    sel.dragging = false;
    if sel.anchor == sel.head {
        // A plain click selects nothing.
        ed.chrome_select = None;
        return;
    }
    let y = sel.y;
    let row = ed
        .selectable_rows
        .borrow()
        .iter()
        .find(|r| r.y == y)
        .cloned();
    match row {
        Some(row) => copy_chrome_selection(ed, &row),
        None => ed.chrome_select = None,
    }
}

/// Copies the selected part of `row` to the `+` register (and so the system
/// clipboard and the unnamed register), trimmed of the row's padding.
fn copy_chrome_selection(ed: &mut Editor, row: &crate::render::SelectableRow) {
    let Some((from, to)) = ed.chrome_select.and_then(|s| s.span_on(row.y)) else {
        return;
    };
    let text = row.slice(from, to).trim().to_string();
    if text.is_empty() {
        ed.chrome_select = None;
        return;
    }
    let n = text.chars().count();
    ed.registers.set(Some('+'), text, false);
    // Reporting the copy rewrites the message line, so a selection on that
    // row would highlight text that is no longer there.
    if ed.chrome_select.is_some_and(|s| s.y + 1 == grid(ed).1) {
        ed.chrome_select = None;
    }
    ed.set_message(format!("Copied {n} char{}", if n == 1 { "" } else { "s" }));
}

/// Tracks which pane's minimap the pointer is over, for
/// `minimap_show_slider = "mouseover"`.
fn hover(ed: &mut Editor, m: &MouseEvent) {
    if !ed.config.minimap {
        ed.minimap_hover = None;
        return;
    }
    let (cols, rows) = grid(ed);
    ed.minimap_hover = crate::minimap::hit(ed, cols, rows, m.column as usize, m.row as usize, None)
        .map(|h| h.pane);
}

/// A press on a minimap. On the slider it starts a drag from where it was
/// grabbed. Elsewhere it centres the view on the clicked line and starts a
/// drag from the slider's middle (VS Code behaviour). Returns whether the
/// press was on a minimap.
fn minimap_down(ed: &mut Editor, m: &MouseEvent) -> bool {
    if !ed.config.minimap || !matches!(ed.mode, Mode::Normal | Mode::Insert | Mode::Visual(_)) {
        return false;
    }
    let (cols, rows) = grid(ed);
    let Some(h) = crate::minimap::hit(ed, cols, rows, m.column as usize, m.row as usize, None)
    else {
        return false;
    };
    let (top, height) = h.slider;
    let grab = if h.row >= top && h.row < top + height {
        h.row - top
    } else {
        let Some((lo, hi)) = h.geom.row_lines(h.row) else {
            // Below the end of a short file: nothing to jump to.
            return true;
        };
        let line = (lo + hi) / 2;
        let max_top = h.geom.total.saturating_sub(h.visible);
        set_pane_top(ed, h.pane, line.saturating_sub(h.visible / 2).min(max_top));
        height / 2
    };
    ed.minimap_drag = Some(crate::minimap::Drag {
        pane: h.pane,
        grab,
        slider_h: height,
    });
    true
}

/// Scroll pane `p` so `top` is its first line. The focused pane keeps its
/// cursor on screen (as wheel scrolling does), an unfocused pane just
/// moves its stored viewport. Neither changes focus, matching VS Code,
/// where using the minimap scrolls without moving the caret.
fn set_pane_top(ed: &mut Editor, p: usize, top: usize) {
    let focused = ed.windows.is_empty() || p == ed.active_window;
    if focused {
        let last = ed.buf().line_count().saturating_sub(1);
        ed.buf_mut().top_line = top.min(last);
        ed.buf_mut().top_wrap = 0;
        crate::render::clamp_cursor_to_view(ed);
    } else if let Some(w) = ed.windows.get(p) {
        let last = ed
            .buffers
            .iter()
            .find(|b| b.id == w.buffer)
            .map(|b| b.line_count().saturating_sub(1))
            .unwrap_or(0);
        ed.windows[p].top = top.min(last);
        ed.windows[p].wrap_row = 0;
    }
}

/// The rect height of pane `p`, for scrolling a sidebar whose viewport is
/// bounded by the pane it's drawn in.
fn pane_height(ed: &Editor, p: usize) -> Option<usize> {
    let (cols, rows) = grid(ed);
    ed.pane_rects(cols, rows).get(p).map(|r| r.height)
}

/// Routes a wheel event to whichever split the pointer is over, rather than
/// always the active pane. Sidebar panes (file tree / outline) have their own
/// viewport and are scrolled even when focused; the active buffer pane (and the
/// no-split case) keeps the cursor-aware `scroll`; an inactive buffer pane's
/// viewport lives in its `Window` entry and is scrolled there directly.
fn wheel(ed: &mut Editor, m: &MouseEvent, delta: isize) {
    if matches!(ed.mode, Mode::MarkdownPreview) {
        let (cols, rows) = grid(ed);
        if m.column as usize >= cols || m.row as usize >= rows.saturating_sub(1) {
            return;
        }
        if let Some(preview) = &mut ed.markdown_preview {
            preview.scroll = preview
                .scroll
                .saturating_add_signed(delta)
                .min(preview.max_scroll(rows.saturating_sub(2)));
        }
        return;
    }
    let Some(p) = pane_at(ed, m) else {
        scroll(ed, delta);
        return;
    };
    if let Some(w) = ed.windows.get(p).cloned() {
        if let Some(id) = w.terminal {
            terminal_wheel(ed, p, id, m, delta);
            return;
        }
        if w.file_tree {
            if let (Some(h), Some(t)) = (pane_height(ed, p), ed.file_tree.as_mut()) {
                t.scroll(delta, h.saturating_sub(crate::filetree::HEADER_ROWS));
            }
            return;
        }
        if w.outline {
            if let (Some(h), Some(o)) = (pane_height(ed, p), ed.outline.as_mut()) {
                o.scroll(delta, h);
            }
            return;
        }
        if w.preview {
            scroll_pane(ed, p, delta);
            return;
        }
        if p != ed.active_window {
            scroll_pane(ed, p, delta);
            return;
        }
    }
    scroll(ed, delta);
}

/// The wheel over a terminal pane. A program that asked for mouse reports
/// gets the wheel itself (when the view is live); a full-screen program on
/// the alternate screen gets arrow keys, as terminals do; otherwise the
/// pane scrolls through its own history.
fn terminal_wheel(ed: &mut Editor, pane: usize, id: u64, m: &MouseEvent, delta: isize) {
    let smooth = ed.config.smooth_scroll;
    let Some(pty) = ed.terminals.iter_mut().find(|p| p.id == id) else {
        return;
    };
    let (mouse, alternate) = pty.mouse_state();
    let live = pty.scroll_offset() == 0;
    if mouse && live {
        if let Some((x, y)) = terminal_cell(ed, pane, m) {
            let pty = ed.terminals.iter_mut().find(|p| p.id == id).unwrap();
            if let Some(bytes) = pty.mouse_report(m.kind, m.modifiers, x, y) {
                pty.write_input(&bytes);
            }
        }
        return;
    }
    if alternate && live {
        let arrow: &[u8] = if delta < 0 { b"\x1b[A" } else { b"\x1b[B" };
        for _ in 0..delta.unsigned_abs() {
            pty.write_input(arrow);
        }
        return;
    }
    // Wheel up (negative delta) looks further back into history.
    if smooth {
        pty.scroll_smooth(-delta);
    } else {
        pty.scroll_by(-delta);
        ed.refresh_terminal_hover();
    }
}

/// The pointer's cell relative to terminal pane `pane`'s program area, or
/// `None` when it is outside it (on the pane's statusline).
fn terminal_cell(ed: &Editor, pane: usize, m: &MouseEvent) -> Option<(usize, usize)> {
    let (cols, rows) = grid(ed);
    let r = *ed.pane_rects(cols, rows).get(pane)?;
    let body = crate::render::terminal_body_rect(ed, r);
    let (x, y) = (m.column as usize, m.row as usize);
    (x >= body.x && x < body.x + body.width && y >= body.y && y < body.y + body.height)
        .then(|| (x - body.x, y - body.y))
}

/// A press, release or drag over a terminal pane. Pressing an unfocused
/// terminal pane focuses it in Terminal mode; on the focused one, a program
/// that asked for mouse reports receives the event. Returns whether the
/// event was a terminal pane's.
fn terminal_mouse(ed: &mut Editor, m: &MouseEvent) -> bool {
    let Some(p) = pane_at(ed, m) else {
        return false;
    };
    let Some(id) = ed.windows.get(p).and_then(|w| w.terminal) else {
        return false;
    };
    let focused = p == ed.active_window && ed.mode == Mode::Terminal;
    if matches!(m.kind, MouseEventKind::Down(_)) && !focused {
        if !matches!(
            ed.mode,
            Mode::Normal | Mode::Insert | Mode::Visual(_) | Mode::Terminal
        ) {
            return false;
        }
        if matches!(ed.mode, Mode::Visual(_) | Mode::Insert) {
            ed.visual_anchor = None;
            ed.enter_normal();
        }
        ed.focus_pane_buffer(p);
        ed.mode = Mode::Terminal;
        ed.mouse_down_at = None;
        return true;
    }
    if focused {
        if let Some((x, y)) = terminal_cell(ed, p, m) {
            if let Some(pty) = ed.terminals.iter_mut().find(|t| t.id == id) {
                if pty.mouse_state().0 {
                    if let Some(bytes) = pty.mouse_report(m.kind, m.modifiers, x, y) {
                        pty.write_input(&bytes);
                    }
                }
            }
        }
        ed.mouse_down_at = None;
        return true;
    }
    false
}

/// Scrolls an inactive pane by editing its stored `Window` viewport. No
/// cursor "keep it visible" nudge is needed here (unlike the active pane):
/// `prepare_view` only re-centres the active window, so an inactive pane
/// renders its `top`/`preview_scroll` verbatim and stays put.
fn scroll_pane(ed: &mut Editor, p: usize, delta: isize) {
    let w = ed.windows[p].clone();
    if w.terminal.is_some() || w.file_tree || w.outline {
        return;
    }
    if w.preview {
        let max = ed.preview_bounds(p).0;
        ed.windows[p].preview_scroll = if delta < 0 {
            w.preview_scroll.saturating_sub(delta.unsigned_abs())
        } else {
            (w.preview_scroll + delta as usize).min(max)
        };
        return;
    }
    let Some(last) = ed
        .buffers
        .iter()
        .find(|b| b.id == w.buffer)
        .map(|b| b.line_count().saturating_sub(1))
    else {
        return;
    };
    ed.windows[p].top = if delta < 0 {
        w.top.saturating_sub(delta.unsigned_abs())
    } else {
        (w.top + delta as usize).min(last)
    };
}

/// Scrolling moves the viewport, not the cursor -- except that the cursor
/// must never end up scrolled off-screen (`prepare_view`'s own "keep the
/// cursor visible" pass would otherwise immediately snap the viewport
/// right back to the cursor's line, undoing the scroll on the very next
/// frame), so it's nudged back onto the new top/bottom edge when needed.
fn scroll(ed: &mut Editor, delta: isize) {
    let last = ed.buf().line_count().saturating_sub(1);
    let top = if delta < 0 {
        ed.buf().top_line.saturating_sub(delta.unsigned_abs())
    } else {
        (ed.buf().top_line + delta as usize).min(last)
    };
    ed.buf_mut().top_line = top;
    // The new top is a buffer line, not a continuation of the old top line.
    ed.buf_mut().top_wrap = 0;
    crate::render::clamp_cursor_to_view(ed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, editor::Editor};

    fn editor(text: &str) -> Editor {
        let cfg = Config {
            number: false,
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..Config::default()
        };
        let mut e = Editor::new(cfg);
        e.buf_mut().rope = ropey::Rope::from_str(text);
        e.buf_mut().mark_saved();
        e
    }
    fn ev(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }
    // Gutter is 2 columns wide with number=false (see render::gutter).
    const GUTTER: u16 = 2;

    /// Draws one frame so `selectable_rows` holds what is on screen.
    fn draw(e: &mut Editor, cols: u16, rows: u16) {
        e.screen_cols = cols as usize;
        e.terminal_rows = rows as usize;
        crate::render::prepare_view(e, cols as usize, rows as usize);
        let mut cache = crate::render::FrameCache::new();
        crate::render::draw(&mut Vec::new(), e, cols, rows, &mut cache).unwrap();
    }

    fn status_row(e: &Editor) -> crate::render::SelectableRow {
        e.selectable_rows
            .borrow()
            .iter()
            .find(|r| r.text.contains("NORMAL"))
            .cloned()
            .expect("statusline is selectable")
    }

    fn plus(e: &Editor) -> Option<String> {
        e.registers
            .list()
            .into_iter()
            .find(|(k, _)| *k == '+')
            .map(|(_, r)| r.text)
    }

    #[test]
    fn statusline_drag_selects_and_copies_its_text() {
        let mut e = editor("hello\n");
        draw(&mut e, 40, 10);
        let row = status_row(&e);
        let start = row.text.find("NORMAL").unwrap();
        let (x, y) = ((row.x + start) as u16, row.y as u16);
        handle(&mut e, ev(MouseEventKind::Down(MouseButton::Left), x, y));
        handle(
            &mut e,
            ev(MouseEventKind::Drag(MouseButton::Left), x + 5, y),
        );
        // The buffer selection is untouched: this is chrome, not text.
        assert!(!matches!(e.mode, Mode::Visual(_)));
        assert_eq!(
            e.chrome_select.unwrap().span_on(row.y),
            Some((x as usize, x as usize + 6))
        );
        handle(&mut e, ev(MouseEventKind::Up(MouseButton::Left), x + 5, y));
        assert_eq!(plus(&e).as_deref(), Some("NORMAL"));
        // The selection stays highlighted until the next key or click.
        draw(&mut e, 40, 10);
        assert!(e.chrome_select.is_some());
        handle(
            &mut e,
            ev(MouseEventKind::Down(MouseButton::Left), GUTTER, 0),
        );
        assert!(e.chrome_select.is_none());
    }

    #[test]
    fn statusline_plain_click_copies_nothing() {
        let mut e = editor("hello\n");
        draw(&mut e, 40, 10);
        let row = status_row(&e);
        let (x, y) = ((row.x + 3) as u16, row.y as u16);
        handle(&mut e, ev(MouseEventKind::Down(MouseButton::Left), x, y));
        handle(&mut e, ev(MouseEventKind::Up(MouseButton::Left), x, y));
        assert!(e.chrome_select.is_none());
        assert_eq!(plus(&e), None);
        assert_eq!(e.cursor(), (0, 0));
    }

    #[test]
    fn statusline_double_click_copies_a_word_and_triple_click_the_row() {
        let mut e = editor("hello\n");
        draw(&mut e, 40, 10);
        let row = status_row(&e);
        let x = (row.x + row.text.find("NORMAL").unwrap() + 2) as u16;
        let y = row.y as u16;
        let press = |e: &mut Editor| {
            handle(e, ev(MouseEventKind::Down(MouseButton::Left), x, y));
            handle(e, ev(MouseEventKind::Up(MouseButton::Left), x, y));
        };
        press(&mut e);
        press(&mut e);
        assert_eq!(plus(&e).as_deref(), Some("NORMAL"));
        press(&mut e);
        assert_eq!(plus(&e).as_deref(), Some(row.text.trim()));
    }

    #[test]
    fn selectable_row_slices_and_finds_words_by_cell() {
        let row = crate::render::SelectableRow {
            y: 0,
            x: 2,
            width: 15,
            text: " 日本 src/a.rs ".into(),
        };
        assert_eq!(row.slice(3, 7), "日本");
        assert_eq!(row.word_at(9), Some((8, 16)));
        assert_eq!(row.slice(8, 16), "src/a.rs");
        assert_eq!(row.word_at(2), None);
    }

    #[test]
    fn click_positions_cursor_on_first_line() {
        let mut e = editor("hello world\nsecond line\n");
        down(
            &mut e,
            ev(MouseEventKind::Down(MouseButton::Left), GUTTER + 6, 0),
        );
        assert_eq!(e.cursor(), (0, 6));
    }

    #[test]
    fn click_positions_cursor_on_second_line() {
        let mut e = editor("hello world\nsecond line\n");
        down(
            &mut e,
            ev(MouseEventKind::Down(MouseButton::Left), GUTTER + 3, 1),
        );
        assert_eq!(e.cursor(), (1, 3));
    }

    #[test]
    fn drag_after_down_enters_visual_and_extends_selection() {
        let mut e = editor("hello world\n");
        down(
            &mut e,
            ev(MouseEventKind::Down(MouseButton::Left), GUTTER, 0),
        );
        assert!(!matches!(e.mode, Mode::Visual(_)));
        drag(
            &mut e,
            ev(MouseEventKind::Drag(MouseButton::Left), GUTTER + 4, 0),
        );
        assert!(matches!(e.mode, Mode::Visual(VisualKind::Char)));
        assert_eq!(e.visual_anchor, Some((0, 0)));
        assert_eq!(e.cursor(), (0, 4));
    }

    #[test]
    fn scroll_moves_the_viewport() {
        let text: String = (0..50).map(|i| format!("line {i}\n")).collect();
        let mut e = editor(&text);
        e.buf_mut().top_line = 10;
        scroll(&mut e, 3);
        assert_eq!(e.buf().top_line, 13);
        scroll(&mut e, -5);
        assert_eq!(e.buf().top_line, 8);
    }

    #[test]
    fn scroll_pulls_the_cursor_back_onto_screen_but_no_further_than_needed() {
        // Scrolling far enough to leave the cursor's line above the new
        // viewport must move the cursor onto the new top edge -- otherwise
        // prepare_view's own "keep the cursor visible" pass would snap the
        // viewport right back, undoing the scroll on the next frame.
        let text: String = (0..50).map(|i| format!("line {i}\n")).collect();
        let mut e = editor(&text);
        e.buf_mut().top_line = 10;
        scroll(&mut e, 3); // top -> 13; cursor (line 0) is now above it
        assert_eq!(e.buf().top_line, 13);
        assert_eq!(e.cursor().0, 13);
        scroll(&mut e, -5); // top -> 8; cursor (13) is still in view
        assert_eq!(e.buf().top_line, 8);
        assert_eq!(e.cursor().0, 13);
    }

    #[test]
    fn scrolling_up_through_wrapped_lines_never_bounces() {
        for (cols, rows, chrome) in [(80, 24, false), (80, 24, true), (26, 10, false)] {
            for split in [false, true] {
                let text: String = (0..80)
                    .map(|i| format!("line {i:03}: {}\n", "x".repeat(240)))
                    .collect();
                let mut ed = editor(&text);
                ed.config.number = true;
                ed.config.winbar = chrome;
                ed.config.minimap = chrome;
                ed.set_cursor(79, 200);
                ed.buf_mut().top_line = 79;
                ed.buf_mut().top_wrap = 1;
                if split {
                    ed.split_window(false, false);
                }
                crate::render::prepare_view(&mut ed, cols, rows);
                let mut cache = crate::render::FrameCache::new();
                let mut terminal = vt100::Parser::new(rows as u16, cols as u16, 0);

                for step in 0..30 {
                    let previous_top = ed.buf().top_line;
                    scroll(&mut ed, -3);
                    let requested = (ed.buf().top_line, ed.buf().top_wrap);
                    crate::render::prepare_view(&mut ed, cols, rows);
                    assert_eq!(
                        (ed.buf().top_line, ed.buf().top_wrap),
                        requested,
                        "redraw undid scroll: {cols}x{rows}, chrome={chrome}, split={split}, step={step}"
                    );
                    assert!(ed.buf().top_line < previous_top || previous_top == 0);

                    let mut out = Vec::new();
                    crate::render::draw(&mut out, &ed, cols as u16, rows as u16, &mut cache)
                        .unwrap();
                    terminal.process(&out);
                    let (y, x) = terminal.screen().cursor_position();
                    let (pane, line, col) =
                        crate::render::locate_click(&ed, cols, rows, x as usize, y as usize)
                            .expect("cursor must be inside the displayed text");
                    assert_eq!(pane, ed.active_window);
                    assert_eq!((line, col), ed.cursor(), "cursor must land on its glyph");
                }
                assert_eq!(
                    ed.buf().top_line,
                    0,
                    "wheel must reach the start of the file"
                );
            }
        }
    }

    #[test]
    fn click_outside_any_pane_is_ignored() {
        let mut e = editor("hi\n");
        let before = e.cursor();
        down(
            &mut e,
            ev(MouseEventKind::Down(MouseButton::Left), 500, 500),
        );
        assert_eq!(e.cursor(), before);
    }

    #[test]
    fn full_markdown_preview_wheel_is_bounded_and_preserves_source_view() {
        let mut e = editor(&"source row\n".repeat(100));
        e.screen_cols = 100;
        e.screen_rows = 22;
        e.buf_mut().top_line = 7;
        e.set_cursor(8, 1);
        let original = (e.cursor(), e.buf().top_line);
        let mut preview = crate::markdown::Preview::new();
        preview.lines = vec![Vec::new(); 400];
        e.markdown_preview = Some(preview);
        e.mode = Mode::MarkdownPreview;

        handle(&mut e, ev(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(e.markdown_preview.as_ref().unwrap().scroll, 3);
        handle(&mut e, ev(MouseEventKind::ScrollUp, 5, 5));
        assert_eq!(e.markdown_preview.as_ref().unwrap().scroll, 0);
        wheel(&mut e, &ev(MouseEventKind::ScrollDown, 5, 5), isize::MAX);
        let last = e
            .markdown_preview
            .as_ref()
            .unwrap()
            .max_scroll(grid(&e).1.saturating_sub(2));
        assert_eq!(e.markdown_preview.as_ref().unwrap().scroll, last);
        wheel(&mut e, &ev(MouseEventKind::ScrollUp, 5, 5), -isize::MAX);
        assert_eq!(e.markdown_preview.as_ref().unwrap().scroll, 0);
        handle(&mut e, ev(MouseEventKind::Down(MouseButton::Left), 5, 5));
        handle(&mut e, ev(MouseEventKind::ScrollDown, 500, 500));
        assert_eq!(e.markdown_preview.as_ref().unwrap().scroll, 0);
        assert_eq!((e.cursor(), e.buf().top_line), original);
        assert_eq!(e.mode, Mode::MarkdownPreview);
    }

    #[test]
    fn split_markdown_preview_wheel_routes_by_pointer_for_either_focus() {
        let mut e = editor(&"source row\n".repeat(100));
        e.screen_cols = 100;
        e.screen_rows = 22;
        e.terminal_rows = 24;
        e.buf_mut().path = Some(std::env::temp_dir().join("mouse-preview.md"));
        e.split_window(true, true);
        let (cols, rows) = grid(&e);
        e.screen_cols = cols;
        e.terminal_rows = rows;
        e.screen_rows = rows.saturating_sub(2);
        let rect = e.pane_rects(cols, rows)[1];
        let mut preview = crate::markdown::Preview::new();
        preview.lines = vec![Vec::new(); 400];
        e.preview_panes.borrow_mut().insert(
            (e.buf().id, rect.width.saturating_sub(2).max(1), 100),
            preview,
        );
        let event = ev(
            MouseEventKind::ScrollDown,
            (rect.x + 1) as u16,
            (rect.y + 1) as u16,
        );

        for focused in [false, true] {
            if focused {
                e.focus_window(1);
            }
            e.windows[1].preview_scroll = 0;
            let original = (e.cursor(), e.buf().top_line, e.active_window);
            handle(&mut e, event);
            assert_eq!(e.windows[1].preview_scroll, 3);
            handle(
                &mut e,
                MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    ..event
                },
            );
            assert_eq!(e.windows[1].preview_scroll, 0);
            wheel(&mut e, &event, 10_000);
            assert_eq!(e.windows[1].preview_scroll, e.preview_bounds(1).0);
            wheel(&mut e, &event, -10_000);
            assert_eq!(e.windows[1].preview_scroll, 0);
            assert_eq!((e.cursor(), e.buf().top_line, e.active_window), original);
        }
    }

    fn minimap_editor() -> Editor {
        let text: String = (0..400).map(|i| format!("fn line_{i}() {{}}\n")).collect();
        let mut e = editor(&text);
        e.config.minimap = true;
        e.screen_cols = 80;
        e.terminal_rows = 24;
        e
    }

    #[test]
    fn clicking_the_minimap_jumps_and_dragging_the_slider_scrolls() {
        let mut e = minimap_editor();
        let x = 80 - e.config.minimap_width as u16 + 3;
        // Row 20 covers lines 80..84 (4 lines per Braille row).
        handle(&mut e, ev(MouseEventKind::Down(MouseButton::Left), x, 20));
        let top = e.buf().top_line;
        assert!(
            (60..=82).contains(&top),
            "centred near line 82, got top {top}"
        );
        let cur = e.cursor().0;
        assert!(cur >= top, "cursor pulled into view: {cur} vs top {top}");
        assert!(
            matches!(e.mode, Mode::Normal),
            "a minimap click never selects"
        );
        assert!(e.minimap_drag.is_some());
        // Dragging the slider to the top scrolls back to the start.
        handle(&mut e, ev(MouseEventKind::Drag(MouseButton::Left), x, 0));
        assert_eq!(e.buf().top_line, 0);
        assert!(
            matches!(e.mode, Mode::Normal),
            "a minimap drag never selects"
        );
        handle(&mut e, ev(MouseEventKind::Up(MouseButton::Left), x, 0));
        assert!(e.minimap_drag.is_none());
        // A click in the text area still places the cursor as before.
        handle(
            &mut e,
            ev(MouseEventKind::Down(MouseButton::Left), GUTTER + 1, 2),
        );
        assert_eq!(e.cursor(), (2, 1));
        assert!(e.minimap_drag.is_none());
    }

    #[test]
    fn hovering_the_minimap_is_tracked_for_mouseover_sliders() {
        let mut e = minimap_editor();
        let x = 80 - e.config.minimap_width as u16 + 3;
        handle(&mut e, ev(MouseEventKind::Moved, x, 5));
        assert_eq!(e.minimap_hover, Some(0));
        handle(&mut e, ev(MouseEventKind::Moved, 10, 5));
        assert_eq!(e.minimap_hover, None);
    }
}
