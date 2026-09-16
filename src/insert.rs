use std::time::Instant;

use crate::editor::Editor;
use crate::key::Key;

pub fn handle(ed: &mut Editor, key: Key) {
    if ed.snippet.is_some() {
        if key == Key::Tab || key == Key::BackTab {
            ed.snippet_next(key == Key::BackTab);
            return;
        }
        if matches!(key, Key::Esc | Key::Left | Key::Right | Key::Up | Key::Down) {
            ed.sync_snippet_mirrors();
            ed.snippet = None;
        }
    }
    let mut session = ed.snippet.take();
    let before = session.as_ref().map(|_| ed.buf().rope.clone());
    let mut consumed = false;
    if let Some(s) = &mut session {
        if s.selected
            && matches!(
                key,
                Key::Char(_) | Key::Literal(_) | Key::Backspace | Key::Delete
            )
        {
            let (a, b) = s.stops[s.current];
            ed.buf_mut().delete_char_range(a, b);
            let (l, c) = ed.buf().pos_from_char_idx(a);
            ed.set_cursor_insert(l, c);
            s.selected = false;
            if matches!(key, Key::Backspace | Key::Delete) {
                consumed = true;
            }
        }
    }
    if !consumed {
        handle_inner(ed, key);
    }
    if let (Some(mut s), Some(before)) = (session, before) {
        let old: Vec<_> = before.chars().collect();
        let new: Vec<_> = ed.buf().rope.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let end = old.len() - suffix;
        s.shift(prefix, end, new.len() - prefix - suffix, Some(s.current));
        ed.snippet = Some(s);
        ed.close_completion();
    }
}
fn handle_inner(ed: &mut Editor, key: Key) {
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
        Key::Enter if crate::autopairs::on_enter(ed) => {}
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
        Key::Backspace if crate::autopairs::on_backspace(ed) => {
            ed.update_completion();
        }
        Key::Backspace => {
            let (line, col) = ed.cursor();
            if col > 0 {
                let prev = crate::grapheme::step(&ed.buf().line_text(line), col, 1, false);
                let start = ed.buf().char_idx(line, prev);
                let end = ed.buf().char_idx(line, col);
                ed.buf_mut().delete_char_range(start, end);
                // `prev` is already the grapheme boundary immediately before
                // the old cursor. Recomputing from `col` after shortening the
                // line skips one extra grapheme and makes repeated Backspace
                // leave alternating characters behind.
                ed.set_cursor_insert(line, prev);
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
            let end = if col < ed.buf().line_len(line) {
                ed.buf().char_idx(
                    line,
                    crate::grapheme::step(&ed.buf().line_text(line), col, 1, true),
                )
            } else {
                start + 1
            };
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
                ed.set_cursor_insert(
                    line,
                    crate::grapheme::step(&ed.buf().line_text(line), col, 1, false),
                );
            } else if line > 0 {
                ed.set_cursor_insert(line - 1, ed.buf().line_len(line - 1));
            }
            ed.close_completion();
        }
        Key::Right => {
            let (line, col) = ed.cursor();
            ed.set_cursor_insert(
                line,
                crate::grapheme::step(&ed.buf().line_text(line), col, 1, true),
            );
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
        Key::Char(c) if crate::autopairs::on_char(ed, c) => {
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

pub(crate) fn accept_completion(ed: &mut Editor) {
    let raw = ed
        .completion
        .as_ref()
        .and_then(|c| c.items.get(c.selected))
        .and_then(|i| i.raw.clone());
    if let Some(raw) = raw {
        if ed.resolve_completion(raw) {
            return;
        }
    }

    let Some(comp) = ed.completion.take() else {
        return;
    };
    let Some(mut item) = comp.items.get(comp.selected).cloned() else {
        return;
    };
    let mut stops = Vec::new();
    let mut mirrors = Vec::new();
    if item.snippet {
        let mut vars = std::collections::BTreeMap::new();
        if let Some(p) = &ed.buf().path {
            vars.insert(
                "TM_FILENAME".into(),
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
            vars.insert("TM_FILEPATH".into(), p.display().to_string());
        }
        vars.insert("TM_LINE_NUMBER".into(), (ed.cursor().0 + 1).to_string());
        match crate::snippet::expand(&item.insert_text, &vars) {
            Ok(expansion) => {
                item.insert_text = expansion.text;
                stops = expansion.stops;
                mirrors = expansion.mirrors;
                if let Some(edit) = &mut item.edit {
                    edit["newText"] = serde_json::json!(item.insert_text);
                }
            }
            Err(e) => {
                ed.set_message(e.to_string());
                return;
            }
        }
    }
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
            if !stops.is_empty() {
                let base = caret.saturating_sub(item.insert_text.chars().count());
                let stops: Vec<_> = stops
                    .into_iter()
                    .map(|(a, b)| (a + base, b + base))
                    .collect();
                let (l, c) = ed.buf().pos_from_char_idx(stops[0].0);
                ed.snippet = Some(crate::snippet::Session {
                    mirrors: mirrors
                        .into_iter()
                        .map(|g| g.into_iter().map(|(a, b)| (a + base, b + base)).collect())
                        .collect(),
                    stops,
                    current: 0,
                    selected: true,
                });
                ed.set_cursor_insert(l, c);
            }
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
    ed.sync_snippet_mirrors();
    ed.snippet = None;
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
            let first_col = crate::grapheme::column(&ed.buf().line_text(first), col, false);
            let start = ed.buf().char_idx(first, first_col);
            let end = ed.buf().char_idx(first, ed.cursor().1);
            let text = ed.buf().text_range(start, end);
            for line in first + 1..=last {
                let len = ed.buf().line_len(line);
                let width =
                    unicode_width::UnicodeWidthStr::width(ed.buf().line_text(line).as_str());
                if width < col {
                    ed.buf_mut().insert_str(line, len, &" ".repeat(col - width));
                }
                let at = crate::grapheme::column(&ed.buf().line_text(line), col, false);
                ed.buf_mut().insert_str(line, at, &text);
            }
        }
    }
    ed.buf_mut().commit_edit();
    ed.finish_change_recording();
    let (l, c) = ed.cursor();
    ed.set_cursor_insert(l, c.saturating_sub(1));
    ed.enter_normal();
}
