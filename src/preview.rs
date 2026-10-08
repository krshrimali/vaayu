use crate::editor::Editor;
use crate::key::Key;

pub fn handle(ed: &mut Editor, key: Key) {
    let Some(preview) = &mut ed.markdown_preview else {
        return;
    };
    let rows = ed.terminal_rows.saturating_sub(2).max(1);
    let last = preview.max_scroll(rows);
    let max_left = preview.max_left(ed.screen_cols.saturating_sub(2).max(1));
    match key {
        Key::Esc | Key::Char('q') => {
            ed.markdown_preview = None;
            ed.enter_normal();
        }
        Key::Char('j') | Key::Down => preview.scroll = (preview.scroll + 1).min(last),
        Key::Char('k') | Key::Up => preview.scroll = preview.scroll.saturating_sub(1),
        Key::Ctrl('d') => preview.scroll = (preview.scroll + rows / 2).min(last),
        Key::Ctrl('u') => preview.scroll = preview.scroll.saturating_sub(rows / 2),
        Key::Char('h') | Key::Left => preview.left = preview.left.saturating_sub(4),
        Key::Char('l') | Key::Right => preview.left = (preview.left + 4).min(max_left),
        Key::Char('+') | Key::Char('=') => preview.zoom = (preview.zoom + 25).min(300),
        Key::Char('-') => preview.zoom = preview.zoom.saturating_sub(25).max(50),
        Key::Char('0') => {
            preview.zoom = 100;
            preview.left = 0;
        }
        Key::Char('g') => preview.scroll = 0,
        Key::Char('G') => preview.scroll = last,
        _ => {}
    }
}
