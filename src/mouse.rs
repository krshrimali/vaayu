//! Mouse support: click to position the cursor and focus a pane, drag to
//! start a Visual selection, wheel to scroll, Ctrl-click for
//! go-to-definition. Only active in editing modes (Normal/Insert/Visual);
//! Results/Picker/Markdown-preview ignore the mouse entirely for this
//! slice. Resizing a split by dragging its border is not implemented --
//! that is unrelated to terminal resize (SIGWINCH), which already works.
use crate::editor::Editor;
use crate::mode::{Mode, VisualKind};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

pub fn handle(ed: &mut Editor, m: MouseEvent) {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => down(ed, m),
        MouseEventKind::Drag(MouseButton::Left) => drag(ed, m),
        MouseEventKind::ScrollDown => scroll(ed, 3),
        MouseEventKind::ScrollUp => scroll(ed, -3),
        _ => {}
    }
}

fn locate(ed: &Editor, m: &MouseEvent) -> Option<(usize, usize, usize)> {
    let (cols, rows) = crossterm::terminal::size()
        .map(|(c, r)| (c as usize, r as usize))
        .unwrap_or((ed.screen_cols.max(1), ed.screen_rows.max(1) + 2));
    crate::render::locate_click(ed, cols, rows, m.column as usize, m.row as usize)
}

fn down(ed: &mut Editor, m: MouseEvent) {
    let Some((pane, line, col)) = locate(ed, &m) else {
        return;
    };
    if matches!(ed.mode, Mode::Visual(_)) {
        ed.visual_anchor = None;
        ed.enter_normal();
    }
    ed.active_window = pane;
    ed.set_cursor(line, col);
    if m.modifiers.contains(KeyModifiers::CONTROL) {
        ed.request_definition();
        ed.mouse_down_at = None;
    } else {
        ed.mouse_down_at = Some((line, col));
    }
}

fn drag(ed: &mut Editor, m: MouseEvent) {
    let Some((pane, line, col)) = locate(ed, &m) else {
        return;
    };
    ed.active_window = pane;
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

/// Scrolling moves the viewport, not the cursor -- except that the cursor
/// must never end up scrolled off-screen (`prepare_view`'s own "keep the
/// cursor visible" pass would otherwise immediately snap the viewport
/// right back to the cursor's line, undoing the scroll on the very next
/// frame), so it's nudged back onto the new top/bottom edge when needed.
fn scroll(ed: &mut Editor, delta: isize) {
    let last = ed.buf().line_count().saturating_sub(1);
    let rows = ed.screen_rows.max(1);
    let top = if delta < 0 {
        ed.buf().top_line.saturating_sub(delta.unsigned_abs())
    } else {
        (ed.buf().top_line + delta as usize).min(last)
    };
    ed.buf_mut().top_line = top;
    let bottom = (top + rows.saturating_sub(1)).min(last);
    let (line, col) = ed.cursor();
    if line < top {
        ed.set_cursor(top, col);
    } else if line > bottom {
        ed.set_cursor(bottom, col);
    }
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
    fn click_outside_any_pane_is_ignored() {
        let mut e = editor("hi\n");
        let before = e.cursor();
        down(
            &mut e,
            ev(MouseEventKind::Down(MouseButton::Left), 500, 500),
        );
        assert_eq!(e.cursor(), before);
    }
}
