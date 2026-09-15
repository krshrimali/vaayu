use std::time::Instant;

use crate::editor::Editor;
use crate::key::Key;

pub fn handle(ed: &mut Editor, key: Key) {
    if let Key::Literal(c) = key {
        insert_char(ed, c);
        return;
    }
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

    if ed.completion.as_ref().is_some_and(|c| !c.items.is_empty()) {
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
            let eol = if ed.buf().rope.to_string().contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            ed.buf_mut().insert_str(line, col, eol);
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
                let count = ed.config.tabstop.max(1) - col % ed.config.tabstop.max(1);
                let pad = " ".repeat(count);
                ed.buf_mut().insert_str(line, col, &pad);
                ed.set_cursor_insert(line, col + count);
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
    let Some(comp) = ed.completion.take() else {
        return;
    };
    let Some(item) = comp.items.get(comp.selected) else {
        return;
    };
    let (line, start_col) = comp.start;
    let primary=item.edit.clone().unwrap_or_else(||serde_json::json!({"range":{"start":{"line":line,"character":crate::language::utf16_col(&ed.buf().line_text(line),start_col)},"end":{"line":ed.cursor().0,"character":crate::language::utf16_col(&ed.buf().line_text(ed.cursor().0),ed.cursor().1)}},"newText":item.insert_text}));
    let mut edits = item.additional.clone();
    edits.push(primary.clone());
    let result = (|| -> anyhow::Result<(crate::language::TextEdits, usize)> {
        let main = crate::language::validate_edits(&ed.buf().rope, &[primary])?;
        let start = main[0].0;
        let extra = crate::language::validate_edits(&ed.buf().rope, &item.additional)?;
        let shift: isize = extra
            .iter()
            .filter(|(_, end, _)| *end <= start)
            .map(|(a, b, text)| text.chars().count() as isize - (*b - *a) as isize)
            .sum();
        let caret = (start as isize + main[0].2.chars().count() as isize + shift).max(0) as usize;
        Ok((
            crate::language::validate_edits(&ed.buf().rope, &edits)?,
            caret,
        ))
    })();
    match result {
        Ok((changes, caret)) => {
            for (a, b, text) in changes {
                ed.buf_mut().delete_char_range(a, b);
                ed.buf_mut().insert_str_at(a, &text);
            }
            let (l, c) = ed.buf().pos_from_char_idx(caret);
            ed.set_cursor_insert(l, c);
        }
        Err(e) => ed.set_message(format!("Completion rejected: {e}")),
    }
}

fn insert_char(ed: &mut Editor, c: char) {
    let (line, col) = ed.cursor();
    ed.buf_mut().insert_char(line, col, c);
    ed.set_cursor_insert(line, col + 1);
}

pub(crate) fn leave_insert(ed: &mut Editor) {
    ed.close_completion();
    let count = std::mem::replace(&mut ed.insert_repeat, 1);
    if count > 1 {
        let end = ed.buf().char_idx(ed.cursor().0, ed.cursor().1);
        if end >= ed.insert_start {
            let text = ed.buf().text_range(ed.insert_start, end);
            if text.len().saturating_mul(count) < 16 * 1024 * 1024 {
                let repeat = text.repeat(count - 1);
                ed.buf_mut().insert_str_at(end, &repeat);
                let (l, c) = ed.buf().pos_from_char_idx(end + repeat.chars().count());
                ed.set_cursor_insert(l, c);
            } else {
                ed.set_message("Insert repeat exceeds 16 MiB limit");
            }
        }
    }
    if let Some((first, last, col)) = ed.block_insert.take() {
        if ed.cursor().0 == first {
            let start = ed.buf().char_idx(first, col);
            let end = ed.buf().char_idx(first, ed.cursor().1);
            let text = ed.buf().text_range(start, end);
            for line in first + 1..=last {
                let len = ed.buf().line_len(line);
                if len < col {
                    ed.buf_mut().insert_str(line, len, &" ".repeat(col - len));
                }
                ed.buf_mut().insert_str(line, col, &text);
            }
        }
    }
    ed.buf_mut().commit_edit();
    ed.finish_change_recording();
    let (l, c) = ed.cursor();
    ed.set_cursor_insert(l, c.saturating_sub(1));
    ed.enter_normal();
}
