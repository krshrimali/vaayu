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

    if ed.completion.is_some() {
        match key {
            Key::Down | Key::Ctrl('n') => {
                if let Some(c) = &mut ed.completion {
                    if c.selected + 1 < c.items.len() {
                        c.selected += 1;
                    }
                }
                return;
            }
            Key::Up | Key::Ctrl('p') => {
                if let Some(c) = &mut ed.completion {
                    c.selected = c.selected.saturating_sub(1);
                }
                return;
            }
            Key::Tab | Key::Enter => {
                accept_completion(ed);
                return;
            }
            Key::Esc => {
                ed.close_completion();
                return;
            }
            Key::Char(c) if crate::completion::is_word_char(c) => {
                insert_char(ed, c);
                ed.update_completion();
                return;
            }
            Key::Backspace => {}
            _ => {
                ed.close_completion();
            }
        }
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
            ed.update_completion();
        }
        Key::Delete => {
            let (line, col) = ed.cursor();
            let start = ed.buf().char_idx(line, col);
            let end = start + 1;
            if end <= ed.buf().rope.len_chars() {
                ed.buf_mut().delete_char_range(start, end);
            }
            ed.close_completion();
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
            ed.close_completion();
        }
        Key::Right => {
            let (line, col) = ed.cursor();
            ed.set_cursor_insert(line, col + 1);
            ed.close_completion();
        }
        Key::Up => {
            let (line, col) = ed.cursor();
            if line > 0 {
                ed.set_cursor_insert(line - 1, col);
            }
            ed.close_completion();
        }
        Key::Down => {
            let (line, col) = ed.cursor();
            ed.set_cursor_insert(line + 1, col);
            ed.close_completion();
        }
        Key::Char(c) => {
            insert_char(ed, c);
            if crate::completion::is_word_char(c) {
                ed.update_completion();
            } else {
                ed.close_completion();
            }
        }
        _ => {}
    }
}

fn accept_completion(ed: &mut Editor) {
    let Some(comp) = ed.completion.take() else { return };
    let Some(item) = comp.items.get(comp.selected) else { return };
    let (line, start_col) = comp.start;
    let cur_col = ed.cursor().1;
    let s = ed.buf().char_idx(line, start_col);
    let e = ed.buf().char_idx(line, cur_col.max(start_col));
    ed.buf_mut().delete_char_range(s, e);
    let text = item.insert_text.clone();
    ed.buf_mut().insert_str(line, start_col, &text);
    ed.set_cursor_insert(line, start_col + text.chars().count());
}

fn insert_char(ed: &mut Editor, c: char) {
    let (line, col) = ed.cursor();
    ed.buf_mut().insert_char(line, col, c);
    ed.set_cursor_insert(line, col + 1);
}

fn leave_insert(ed: &mut Editor) {
    ed.close_completion();
    ed.buf_mut().commit_edit();
    ed.finish_change_recording();
    ed.enter_normal();
}
