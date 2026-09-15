use crate::editor::Editor;
use crate::key::Key;

pub fn handle(ed: &mut Editor, key: Key) {
    let Some(preview) = &mut ed.markdown_preview else { return };
    let rows = ed.screen_rows.max(1);
    let last = preview.lines.len().saturating_sub(1);
    match key {
        Key::Esc | Key::Char('q') => {
            ed.markdown_preview = None;
            ed.enter_normal();
        }
        Key::Char('j') | Key::Down => preview.scroll = (preview.scroll + 1).min(last),
        Key::Char('k') | Key::Up => preview.scroll = preview.scroll.saturating_sub(1),
        Key::Ctrl('d') => preview.scroll = (preview.scroll + rows / 2).min(last),
        Key::Ctrl('u') => preview.scroll = preview.scroll.saturating_sub(rows / 2),
        Key::Char('g') => preview.scroll = 0,
        Key::Char('G') => preview.scroll = last,
        _ => {}
    }
}
