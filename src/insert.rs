use std::time::Instant;

use crate::editor::Editor;
use crate::key::Key;

pub fn handle(ed: &mut Editor, key: Key) {
    if ed.config.jk_escape && ed.pending_jk.is_some() {
        ed.pending_jk = None;
        if key == Key::Char('k') {
            leave_insert(ed);
            return;
        } else {
            insert_char(ed, 'j');
            // fall through: handle the key that arrived after the buffered 'j'
        }
    }
    if ed.config.jk_escape && key == Key::Char('j') {
        ed.pending_jk = Some(Instant::now());
        return;
    }

    match key {
        Key::Esc => leave_insert(ed),
        Key::Enter => {
            let (line, col) = ed.cursor();
            let indent: String = ed
                .buf()
                .line_text(line)
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            ed.buf_mut().insert_char(line, col, '\n');
            ed.buf_mut().insert_str(line + 1, 0, &indent);
            ed.set_cursor_insert(line + 1, indent.chars().count());
        }
        Key::Backspace => {
            let (line, col) = ed.cursor();
            if col > 0 {
                let start = ed.buf().char_idx(line, col - 1);
                let end = ed.buf().char_idx(line, col);
                ed.buf_mut().delete_char_range(start, end);
                ed.set_cursor_insert(line, col - 1);
            } else if line > 0 {
                let prev_len = ed.buf().line_len(line - 1);
                let start = ed.buf().char_idx(line - 1, prev_len);
                let end = ed.buf().char_idx(line, 0);
                ed.buf_mut().delete_char_range(start, end);
                ed.set_cursor_insert(line - 1, prev_len);
            }
        }
        Key::Delete => {
            let (line, col) = ed.cursor();
            let start = ed.buf().char_idx(line, col);
            let end = start + 1;
            if end <= ed.buf().rope.len_chars() {
                ed.buf_mut().delete_char_range(start, end);
            }
        }
        Key::Tab => {
            if ed.config.expandtab {
                let (line, col) = ed.cursor();
                let pad = " ".repeat(ed.config.tabstop);
                ed.buf_mut().insert_str(line, col, &pad);
                ed.set_cursor_insert(line, col + ed.config.tabstop);
            } else {
                insert_char(ed, '\t');
            }
        }
        Key::Left => {
            let (line, col) = ed.cursor();
            if col > 0 {
                ed.set_cursor_insert(line, col - 1);
            } else if line > 0 {
                ed.set_cursor_insert(line - 1, ed.buf().line_len(line - 1));
            }
        }
        Key::Right => {
            let (line, col) = ed.cursor();
            ed.set_cursor_insert(line, col + 1);
        }
        Key::Up => {
            let (line, col) = ed.cursor();
            if line > 0 {
                ed.set_cursor_insert(line - 1, col);
            }
        }
        Key::Down => {
            let (line, col) = ed.cursor();
            ed.set_cursor_insert(line + 1, col);
        }
        Key::Char(c) => insert_char(ed, c),
        _ => {}
    }
}

fn insert_char(ed: &mut Editor, c: char) {
    let (line, col) = ed.cursor();
    ed.buf_mut().insert_char(line, col, c);
    ed.set_cursor_insert(line, col + 1);
}

fn leave_insert(ed: &mut Editor) {
    ed.buf_mut().commit_edit();
    ed.finish_change_recording();
    ed.enter_normal();
}
