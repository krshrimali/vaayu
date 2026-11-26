use std::time::Instant;

use crate::editor::Editor;
use crate::key::Key;

pub fn handle(ed: &mut Editor, key: Key) {
    // Leaving an auto-indented line untouched (Esc, Enter) drops its
    // indent, so no trailing whitespace is left behind.
    let dropped = if matches!(key, Key::Esc | Key::Enter | Key::Ctrl('j') | Key::Ctrl('m')) {
        drop_unused_autoindent(ed)
    } else {
        if !matches!(key, Key::Ctrl('o')) {
            ed.ai_line = None;
        }
        None
    };
    handle_key(ed, key);
    // Enter on an untouched indented line: the new line keeps that indent
    // (Vim moves it down rather than losing it).
    if let Some(indent) = dropped.filter(|_| key != Key::Esc) {
        let line = ed.cursor().0;
        if ed.mode == crate::mode::Mode::Insert && ed.buf().line_text(line).is_empty() {
            ed.buf_mut().insert_str(line, 0, &indent);
            ed.set_cursor_insert(line, indent.chars().count());
            note_autoindent(ed, line, &indent);
        }
    }
    // Signature help as you type: `(` and `,` ask the server, `)` clears.
    if ed.mode == crate::mode::Mode::Insert {
        match key {
            Key::Char(c @ ('(' | ',')) => ed.auto_signature_help(c),
            Key::Char(')') | Key::Esc => ed.clear_signature_help(),
            _ => {}
        }
    } else {
        ed.clear_signature_help();
    }
}

/// Clears the cursor line when it holds only the indent auto-indent gave it.
fn drop_unused_autoindent(ed: &mut Editor) -> Option<String> {
    let (buffer, line) = ed.ai_line.take()?;
    let (cur, _) = ed.cursor();
    if buffer != ed.buf().id || cur != line {
        return None;
    }
    let len = ed.buf().line_len(line);
    let text = ed.buf().line_text(line);
    if len == 0 || !text.trim().is_empty() {
        return None;
    }
    let s = ed.buf().char_idx(line, 0);
    ed.buf_mut().delete_char_range(s, s + len);
    ed.set_cursor_insert(line, 0);
    Some(text)
}

/// Marks `line` as holding only auto-indent (when it has any).
pub(crate) fn note_autoindent(ed: &mut Editor, line: usize, indent: &str) {
    ed.ai_line = (!indent.is_empty()).then_some((ed.buf().id, line));
}

fn handle_key(ed: &mut Editor, key: Key) {
    if ed.snippet.is_some() {
        if key == Key::Tab || key == Key::BackTab {
            ed.snippet_next(key == Key::BackTab);
            return;
        }
        if matches!(key, Key::Ctrl('n') | Key::Ctrl('p'))
            && ed
                .snippet
                .as_ref()
                .is_some_and(|s| s.selected && s.choices.contains_key(&s.current))
        {
            ed.snippet_cycle_choice(key == Key::Ctrl('n'));
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
        let linked = s.linked;
        ed.snippet = Some(s);
        ed.close_completion();
        // Linked editing mirrors live on every keystroke (snippets only sync on
        // Tab/Esc); a plain edit outside the active range is left alone.
        if linked {
            ed.sync_linked_live();
        }
    }
}
/// An Insert-mode key that takes an argument.
#[derive(Debug, Clone)]
pub enum InsertPending {
    /// `Ctrl-R {reg}`.
    Register,
    /// `Ctrl-V {key}` (or `Ctrl-V u1234` / `x41` / `065`): the code typed so far.
    Literal(String),
    /// `Ctrl-K {a}{b}`: the first character, once typed.
    Digraph(Option<char>),
    /// `Ctrl-X {key}`: a completion sub-mode.
    CtrlX,
}

fn handle_inner(ed: &mut Editor, key: Key) {
    if let Some(pending) = ed.insert_pending.take() {
        insert_pending_key(ed, pending, key);
        return;
    }
    if let Key::Literal(c) = key {
        insert_char(ed, c);
        return;
    }
    if ed.replace_mode.is_some() && replace_key(ed, key) {
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
    // Ghost-text: Ctrl-l accepts the inline suggestion (a dedicated key so it
    // never competes with Tab/completion).
    if key == Key::Ctrl('l') && ed.ghost.is_some() {
        ed.accept_ghost();
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
            // Ctrl-E closes the popup and keeps typing (Vim's
            // `i_CTRL-E`); Esc closes it and leaves Insert, as in Vim.
            Key::Ctrl('e') => {
                ed.close_completion();
                return;
            }
            Key::Esc => {
                ed.close_completion();
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

    // Abbreviations expand when a non-keyword character ends the word.
    if matches!(key, Key::Char(c) if !crate::completion::is_word_char(c))
        || matches!(key, Key::Enter | Key::Esc | Key::Tab)
    {
        expand_abbreviation(ed);
    }
    match key {
        Key::Enter | Key::Ctrl('j') | Key::Ctrl('m') => ed.insert_typed.push('\n'),
        Key::Backspace | Key::Ctrl('h') => {
            ed.insert_typed.pop();
        }
        _ => {}
    }
    match key {
        Key::Ctrl('h') => return handle_inner(ed, Key::Backspace),
        Key::Ctrl('j') | Key::Ctrl('m') => return handle_inner(ed, Key::Enter),
        Key::Ctrl('w') => {
            delete_word_before(ed);
            ed.close_completion();
            return;
        }
        Key::Ctrl('u') => {
            delete_to_line_start(ed);
            ed.close_completion();
            return;
        }
        Key::Ctrl('t') | Key::Ctrl('d') => {
            shift_line(ed, key == Key::Ctrl('t'));
            return;
        }
        Key::Ctrl('r') => {
            ed.insert_pending = Some(InsertPending::Register);
            return;
        }
        Key::Ctrl('v') | Key::Ctrl('q') => {
            ed.insert_pending = Some(InsertPending::Literal(String::new()));
            return;
        }
        Key::Ctrl('k') => {
            ed.insert_pending = Some(InsertPending::Digraph(None));
            return;
        }
        Key::Ctrl('x') => {
            ed.insert_pending = Some(InsertPending::CtrlX);
            ed.set_message("-- ^X mode (^F file, ^L line, ^N/^P word)");
            return;
        }
        Key::Ctrl('a') | Key::Ctrl('@') => {
            let text = ed.last_insert_text.clone();
            insert_text(ed, &text);
            if key == Key::Ctrl('@') {
                leave_insert(ed);
            }
            return;
        }
        Key::Ctrl('e') | Key::Ctrl('y') => {
            copy_from_adjacent_line(ed, key == Key::Ctrl('e'));
            return;
        }
        Key::Ctrl('o') => {
            ed.close_completion();
            ed.insert_oneshot = true;
            ed.mode = crate::mode::Mode::Normal;
            ed.set_message("-- (insert) --");
            return;
        }
        Key::Ctrl('n') | Key::Ctrl('p') => {
            open_word_completion(ed, key == Key::Ctrl('p'));
            return;
        }
        _ => {}
    }
    match key {
        Key::Esc => leave_insert(ed),
        Key::Enter if crate::autopairs::on_enter(ed) => {}
        Key::Enter => {
            // The rope is always \n-only (line endings are re-applied on save
            // per the buffer's fileformat), so a newline is a bare "\n".
            let (line, col) = ed.cursor();
            let indent = ed.auto_indent(line, col);
            ed.buf_mut().insert_str(line, col, "\n");
            ed.buf_mut().insert_str(line + 1, 0, &indent);
            ed.set_cursor_insert(line + 1, indent.chars().count());
            // Only a line left empty after the split is auto-indent alone.
            if ed.buf().line_text(line + 1).trim().is_empty() {
                note_autoindent(ed, line + 1, &indent);
            }
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
            } else if line + 1 < ed.buf().line_count() {
                // At end of line: delete the entire line break by joining the
                // next line -- this removes a CRLF `\r\n` pair, not just the
                // `\r` (which the old `start + 1` left a bare `\n` behind for).
                ed.buf().char_idx(line + 1, 0)
            } else {
                start
            };
            if end > start && end <= ed.buf().rope.len_chars() {
                ed.buf_mut().delete_char_range(start, end);
            }
            ed.close_completion();
        }
        Key::Tab => {
            if ed.buf().expandtab {
                let (line, col) = ed.cursor();
                let tabstop = ed.buf().tabstop.max(1);
                // Pad to the next tab stop measured in DISPLAY columns, so a
                // preceding tab or wide character still lands on a real stop.
                let cell = crate::grapheme::cell(&ed.buf().line_text(line), col, tabstop);
                let count = tabstop - cell % tabstop;
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
            // Electric dedent: a closing bracket typed alone on its line lines
            // up with the block it closes (smartindent).
            ed.electric_dedent(c);
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
    let mut mirror_transforms = Vec::new();
    let mut choices = std::collections::BTreeMap::new();
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
            vars.insert(
                "TM_FILENAME_BASE".into(),
                p.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
            vars.insert("TM_FILEPATH".into(), p.display().to_string());
            vars.insert(
                "TM_DIRECTORY".into(),
                p.parent().unwrap_or(p).display().to_string(),
            );
        }
        vars.insert("TM_LINE_NUMBER".into(), (ed.cursor().0 + 1).to_string());
        // The pre-insertion content of the cursor's own line -- read
        // before `expand()`/the edit below touch anything, matching
        // VSCode's own TM_CURRENT_LINE semantics (the line as it was
        // when the snippet triggered, not after).
        vars.insert("TM_CURRENT_LINE".into(), ed.buf().line_text(ed.cursor().0));
        let expansion = crate::snippet::expand(&item.insert_text, &vars);
        item.insert_text = expansion.text;
        stops = expansion.stops;
        mirrors = expansion.mirrors;
        mirror_transforms = expansion.mirror_transforms;
        choices = expansion.choices;
        if let Some(edit) = &mut item.edit {
            edit["newText"] = serde_json::json!(item.insert_text);
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
                    mirror_transforms,
                    stops,
                    choices,
                    current: 0,
                    selected: true,
                    linked: false,
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
    ed.insert_typed.push(c);
}

pub(crate) fn leave_insert(ed: &mut Editor) {
    ed.replace_mode = None;
    ed.insert_pending = None;
    ed.insert_oneshot = false;
    if !ed.insert_typed.is_empty() {
        ed.last_insert_text = std::mem::take(&mut ed.insert_typed);
    }
    {
        let (l, c) = ed.cursor();
        let id = ed.buf().id;
        ed.last_insert_pos = Some((id, l, c));
    }
    ed.sync_snippet_mirrors();
    ed.snippet = None;
    ed.close_completion();
    let count = std::mem::replace(&mut ed.insert_repeat, 1);
    let open_bof = std::mem::take(&mut ed.insert_open_bof);
    if count > 1 && open_bof {
        // `3O` on the first line: the opened line runs from `insert_start` to
        // just past the newline that terminates it. Prepend that whole unit
        // `count-1` more times so the copies stack above the original.
        let end = ed.buf().char_idx(ed.cursor().0, ed.cursor().1);
        let total = ed.buf().rope.len_chars();
        let unit_end = (end + 1).min(total); // include the trailing newline
        if unit_end > ed.insert_start {
            let text = ed.buf().text_range(ed.insert_start, unit_end);
            if text.len().saturating_mul(count) < 16 * 1024 * 1024 {
                let repeat = text.repeat(count - 1);
                let at = ed.insert_start;
                ed.buf_mut().insert_str_at(at, &repeat);
                // Cursor lands on the last of the opened lines (the original).
                let (l, c) = ed.buf().pos_from_char_idx(end + repeat.chars().count());
                ed.set_cursor_insert(l, c);
            } else {
                ed.set_message("Insert repeat exceeds 16 MiB limit");
            }
        }
    } else if count > 1 {
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
    let mut block_return = None;
    if let Some(block) = ed.block_insert.take() {
        let crate::visual::BlockInsert {
            first,
            last,
            column: col,
            ..
        } = block;
        if ed.cursor().0 == first {
            let start = block.start;
            let end = ed.buf().char_idx(first, ed.cursor().1);
            let text = ed.buf().text_range(start, end);
            let typed = ed.buf().edit_seq != block.prepared_seq;
            for line in first + 1..=last {
                if text.is_empty() {
                    break;
                }
                let width = crate::grapheme::cell(
                    &ed.buf().line_text(line),
                    ed.buf().line_len(line),
                    ed.buf().tabstop,
                );
                let col = if block.at_eol {
                    width
                } else if block.short_eol {
                    col.min(width)
                } else {
                    col
                };
                if let Some(at) =
                    crate::visual::prepare_block_insert(ed, line, col, block.pad_short)
                {
                    ed.buf_mut().insert_str(line, at, &text);
                }
            }
            block_return = block.return_column.map(|col| {
                (
                    first,
                    if !typed && !block.pad_short {
                        crate::grapheme::step(&ed.buf().line_text(first), col, 1, false)
                    } else {
                        col
                    },
                )
            });
        }
    }
    ed.buf_mut().commit_edit();
    ed.finish_change_recording();
    let (l, c) = ed.cursor();
    ed.set_cursor_insert(l, c.saturating_sub(1));
    if let Some((line, col)) = block_return {
        ed.set_cursor(line, col);
    }
    // Leaving Insert re-anchors the sticky `$` column (curswant) to the real
    // cursor column, so a later j/k after e.g. `A`/`o` does not jump to
    // end-of-line just because `$` was pressed before entering Insert.
    let (l, fc) = ed.cursor();
    ed.buf_mut().desired_col =
        crate::grapheme::cursor_cell(&ed.buf().line_text(l), fc, ed.buf().tabstop);
    ed.enter_normal();
    ed.fire_event(crate::events::Event::InsertLeave);
}

/// Inserts `text` at the cursor (it may span lines) and moves past it.
fn insert_text(ed: &mut Editor, text: &str) {
    if text.is_empty() {
        return;
    }
    let (line, col) = ed.cursor();
    let idx = ed.buf().char_idx(line, col);
    ed.buf_mut().insert_str_at(idx, text);
    let (l, c) = ed.buf().pos_from_char_idx(idx + text.chars().count());
    ed.set_cursor_insert(l, c);
    ed.insert_typed.push_str(text);
}

/// Handles the key after `Ctrl-R`, `Ctrl-V`, `Ctrl-K` or `Ctrl-X`.
fn insert_pending_key(ed: &mut Editor, pending: InsertPending, key: Key) {
    if key == Key::Esc {
        ed.message.clear();
        return;
    }
    match pending {
        InsertPending::Register => {
            // `Ctrl-R =`: the expression register, read on a `=` prompt.
            if key == Key::Char('=') {
                ed.expr_prompt = true;
                ed.enter_command(crate::mode::CommandKind::Ex);
                return;
            }
            let Some(reg) = (match key {
                Key::Char(c) => Some(c),
                // `Ctrl-R Ctrl-W`: the word under the cursor.
                Key::Ctrl('w') => {
                    let (line, col) = ed.cursor();
                    let text = ed.buf().line_text(line);
                    if let Some((_, w, _)) =
                        crate::normal::word_for_star(&text, col.saturating_sub(1))
                    {
                        insert_text(ed, &w);
                    }
                    None
                }
                _ => None,
            }) else {
                return;
            };
            ed.sync_readonly_registers();
            let reg = if reg == '"' { None } else { Some(reg) };
            if let Some(entry) = ed.registers.get(reg).cloned() {
                insert_text(ed, &entry.text);
            }
        }
        InsertPending::Literal(mut code) => {
            let c = match key {
                Key::Char(c) => c,
                Key::Literal(c) => c,
                Key::Tab => '\t',
                Key::Enter => '\r',
                Key::Ctrl(c) => {
                    let b = (c as u8).to_ascii_uppercase();
                    if (b'@'..=b'_').contains(&b) {
                        (b - b'@') as char
                    } else {
                        return;
                    }
                }
                Key::Backspace => '\u{8}',
                _ => return,
            };
            // Numeric forms: `u` + 4 hex, `U` + 8 hex, `x` + 2 hex, `o` + 3
            // octal, or 3 decimal digits.
            let (radix, len) = match code.chars().next() {
                Some('u') => (16, 5),
                Some('U') => (16, 9),
                Some('x') | Some('X') => (16, 3),
                Some('o') | Some('O') => (8, 4),
                Some(_) => (10, 3),
                None => (0, 0),
            };
            if code.is_empty() {
                if matches!(c, 'u' | 'U' | 'x' | 'X' | 'o' | 'O') || c.is_ascii_digit() {
                    code.push(c);
                    ed.insert_pending = Some(InsertPending::Literal(code));
                    return;
                }
                insert_char(ed, c);
                return;
            }
            if c.is_digit(radix) {
                code.push(c);
                if code.len() < len {
                    ed.insert_pending = Some(InsertPending::Literal(code));
                    return;
                }
            }
            let digits = if radix == 10 { &code[..] } else { &code[1..] };
            if let Some(ch) = u32::from_str_radix(digits, radix)
                .ok()
                .and_then(char::from_u32)
            {
                insert_char(ed, ch);
            }
            // A key that ended the code early is typed as itself.
            if !c.is_digit(radix) {
                handle_inner(ed, key);
            }
        }
        InsertPending::Digraph(first) => {
            let Some(c) = key.as_char() else {
                return;
            };
            match first {
                None => ed.insert_pending = Some(InsertPending::Digraph(Some(c))),
                Some(a) => {
                    let ch = digraph(a, c).or_else(|| digraph(c, a)).unwrap_or(c);
                    insert_char(ed, ch);
                }
            }
        }
        InsertPending::CtrlX => {
            ed.message.clear();
            match key {
                Key::Ctrl('f') => complete_path(ed),
                Key::Ctrl('l') => complete_line(ed),
                Key::Ctrl('n') | Key::Ctrl('p') => open_word_completion(ed, key == Key::Ctrl('p')),
                _ => handle_inner(ed, key),
            }
        }
    }
}

/// Replace mode (`R`): typed characters overwrite, Backspace restores.
/// Returns whether the key was handled here.
fn replace_key(ed: &mut Editor, key: Key) -> bool {
    match key {
        Key::Char(c) => {
            let (line, col) = ed.cursor();
            let len = ed.buf().line_len(line);
            let old = if col < len {
                let idx = ed.buf().char_idx(line, col);
                let old = ed.buf().rope.char(idx);
                ed.buf_mut().delete_char_range(idx, idx + 1);
                Some(old)
            } else {
                None
            };
            if let Some(stack) = ed.replace_mode.as_mut() {
                stack.push(old);
            }
            insert_char(ed, c);
            true
        }
        Key::Backspace | Key::Ctrl('h') => {
            let (line, col) = ed.cursor();
            let popped = ed.replace_mode.as_mut().and_then(|s| s.pop());
            if col == 0 {
                return true;
            }
            let idx = ed.buf().char_idx(line, col - 1);
            match popped {
                Some(Some(orig)) => {
                    ed.buf_mut().delete_char_range(idx, idx + 1);
                    ed.buf_mut().insert_str_at(idx, &orig.to_string());
                }
                Some(None) => {
                    ed.buf_mut().delete_char_range(idx, idx + 1);
                }
                // Before the replaced text: just move left, like Vim.
                None => {}
            }
            ed.insert_typed.pop();
            ed.set_cursor_insert(line, col - 1);
            true
        }
        _ => false,
    }
}

/// `Ctrl-W`: delete the word (or run of punctuation) before the cursor,
/// and the whitespace before it; at column 0, join with the line above.
fn delete_word_before(ed: &mut Editor) {
    let (line, col) = ed.cursor();
    if col == 0 {
        if line > 0 {
            handle_inner(ed, Key::Backspace);
        }
        return;
    }
    let chars: Vec<char> = ed.buf().line_text(line).chars().collect();
    let col = col.min(chars.len());
    let mut start = col;
    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    if start > 0 {
        let word = crate::completion::is_word_char(chars[start - 1]);
        while start > 0
            && !chars[start - 1].is_whitespace()
            && crate::completion::is_word_char(chars[start - 1]) == word
        {
            start -= 1;
        }
    }
    let s = ed.buf().char_idx(line, start);
    let e = ed.buf().char_idx(line, col);
    ed.buf_mut().delete_char_range(s, e);
    for _ in start..col {
        ed.insert_typed.pop();
    }
    ed.set_cursor_insert(line, start);
}

/// `Ctrl-U`: delete back to the indent (or to column 0 when already there).
fn delete_to_line_start(ed: &mut Editor) {
    let (line, col) = ed.cursor();
    // As in Vim: first only what this Insert session typed on the line,
    // then (pressed again) the rest before the cursor.
    let here = ed.buf().char_idx(line, col);
    let line_start = ed.buf().char_idx(line, 0);
    if ed.insert_start > line_start && ed.insert_start < here {
        let at = ed.insert_start;
        ed.buf_mut().delete_char_range(at, here);
        ed.insert_typed.clear();
        let (l, c) = ed.buf().pos_from_char_idx(at);
        ed.set_cursor_insert(l, c);
        return;
    }
    let indent = ed.buf().first_non_blank(line);
    let indent = if ed.buf().line_text(line).trim().is_empty() {
        0
    } else {
        indent
    };
    let start = if col > indent { indent } else { 0 };
    if start == col {
        if col == 0 && line > 0 {
            handle_inner(ed, Key::Backspace);
        }
        return;
    }
    let s = ed.buf().char_idx(line, start);
    let e = ed.buf().char_idx(line, col);
    ed.buf_mut().delete_char_range(s, e);
    ed.insert_typed.clear();
    ed.set_cursor_insert(line, start);
}

/// `Ctrl-T` / `Ctrl-D`: indent / dedent the line by `shiftwidth`, keeping
/// the cursor on the same character.
fn shift_line(ed: &mut Editor, right: bool) {
    let (line, col) = ed.cursor();
    let before = ed.buf().line_len(line);
    let sw = ed.buf().shiftwidth;
    crate::operator::indent_lines(ed.buf_mut(), line, line, right, sw);
    let after = ed.buf().line_len(line);
    let new_col = (col as isize + after as isize - before as isize).max(0) as usize;
    ed.set_cursor_insert(line, new_col.min(after));
}

/// `Ctrl-E` / `Ctrl-Y`: copy the character at the same screen column from
/// the line below / above.
fn copy_from_adjacent_line(ed: &mut Editor, below: bool) {
    let (line, col) = ed.cursor();
    let other = if below {
        line + 1
    } else if line > 0 {
        line - 1
    } else {
        return;
    };
    if other >= ed.buf().line_count() {
        return;
    }
    let tab = ed.buf().tabstop;
    let cell = crate::grapheme::cell(&ed.buf().line_text(line), col, tab);
    let text = ed.buf().line_text(other);
    let at = crate::grapheme::raw_column(&text, cell, tab);
    if crate::grapheme::cell(&text, at, tab) != cell {
        return;
    }
    if let Some(c) = text.chars().nth(at) {
        insert_char(ed, c);
    }
}

/// `Ctrl-N` / `Ctrl-P` with no popup open: complete the word before the
/// cursor from the buffer (and language server), selecting the first /
/// last candidate.
fn open_word_completion(ed: &mut Editor, last: bool) {
    let enabled = ed.config.completion_enabled;
    ed.config.completion_enabled = true;
    ed.update_completion();
    ed.config.completion_enabled = enabled;
    match ed.completion.as_mut() {
        Some(c) if !c.items.is_empty() => {
            if last {
                c.selected = c.items.len() - 1;
            }
        }
        _ => ed.set_message("Pattern not found"),
    }
}

/// `Ctrl-X Ctrl-F`: complete the file name before the cursor.
fn complete_path(ed: &mut Editor) {
    let (line, col) = ed.cursor();
    let chars: Vec<char> = ed.buf().line_text(line).chars().collect();
    let col = col.min(chars.len());
    let mut start = col;
    while start > 0 && !chars[start - 1].is_whitespace() && !"\"'`(<[{=,".contains(chars[start - 1])
    {
        start -= 1;
    }
    let prefix: String = chars[start..col].iter().collect();
    let base = ed
        .buf()
        .path
        .as_ref()
        .and_then(|p| p.parent())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| ed.project_root.clone());
    let (prefix_for_lookup, base) = match prefix.strip_prefix("~/") {
        Some(rest) => (rest.to_string(), dirs::home_dir().unwrap_or(base)),
        None => (prefix.clone(), base),
    };
    let mut items = crate::completion::path_candidates(&prefix_for_lookup, &base);
    if prefix.starts_with("~/") {
        for i in &mut items {
            i.insert_text = format!("~/{}", i.insert_text);
        }
    }
    show_items(ed, (line, start), items);
}

/// `Ctrl-X Ctrl-L`: complete the whole line from other lines that start
/// with the text typed on this one.
fn complete_line(ed: &mut Editor) {
    let (line, col) = ed.cursor();
    let text: String = ed.buf().line_text(line).chars().take(col).collect();
    let typed = text.trim_start();
    let start = text.chars().count() - typed.chars().count();
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    let n = ed.buf().line_count();
    // Nearest lines first.
    let order = (1..n).flat_map(|d| [line.checked_sub(d), Some(line + d).filter(|&l| l < n)]);
    for l in order.flatten() {
        let candidate = ed.buf().line_text(l);
        let body = candidate.trim();
        if body.is_empty() || body == typed || !body.starts_with(typed) {
            continue;
        }
        if seen.insert(body.to_string()) {
            items.push(crate::completion::Item {
                label: body.to_string(),
                insert_text: body.to_string(),
                detail: Some(format!("line {}", l + 1)),
                source: crate::completion::Source::Buffer,
                edit: None,
                raw: None,
                snippet: false,
                additional: Vec::new(),
                kind: None,
            });
        }
        if items.len() >= 50 {
            break;
        }
    }
    show_items(ed, (line, start), items);
}

fn show_items(ed: &mut Editor, start: (usize, usize), items: Vec<crate::completion::Item>) {
    if items.is_empty() {
        ed.set_message("Pattern not found");
        return;
    }
    ed.next_request_id += 1;
    ed.completion = Some(crate::completion::CompletionState {
        start,
        items,
        selected: 0,
        request_id: ed.next_request_id,
    });
}

/// Expands an `:abbreviate`d word that ends at the cursor.
fn expand_abbreviation(ed: &mut Editor) {
    if ed.abbreviations.is_empty() {
        return;
    }
    let (line, col) = ed.cursor();
    let chars: Vec<char> = ed.buf().line_text(line).chars().collect();
    let col = col.min(chars.len());
    let mut start = col;
    while start > 0 && crate::completion::is_word_char(chars[start - 1]) {
        start -= 1;
    }
    if start == col {
        return;
    }
    let word: String = chars[start..col].iter().collect();
    let Some(rhs) = ed.abbreviations.get(&word).cloned() else {
        return;
    };
    let s = ed.buf().char_idx(line, start);
    let e = ed.buf().char_idx(line, col);
    ed.buf_mut().delete_char_range(s, e);
    ed.buf_mut().insert_str_at(s, &rhs);
    let (l, c) = ed.buf().pos_from_char_idx(s + rhs.chars().count());
    ed.set_cursor_insert(l, c);
}

/// A subset of Vim's RFC 1345 digraphs (`Ctrl-K a b`).
fn digraph(a: char, b: char) -> Option<char> {
    let table: &[(&str, char)] = &[
        ("a'", 'á'),
        ("e'", 'é'),
        ("i'", 'í'),
        ("o'", 'ó'),
        ("u'", 'ú'),
        ("y'", 'ý'),
        ("A'", 'Á'),
        ("E'", 'É'),
        ("I'", 'Í'),
        ("O'", 'Ó'),
        ("U'", 'Ú'),
        ("a!", 'à'),
        ("e!", 'è'),
        ("i!", 'ì'),
        ("o!", 'ò'),
        ("u!", 'ù'),
        ("A!", 'À'),
        ("E!", 'È'),
        ("I!", 'Ì'),
        ("O!", 'Ò'),
        ("U!", 'Ù'),
        ("a>", 'â'),
        ("e>", 'ê'),
        ("i>", 'î'),
        ("o>", 'ô'),
        ("u>", 'û'),
        ("A>", 'Â'),
        ("E>", 'Ê'),
        ("I>", 'Î'),
        ("O>", 'Ô'),
        ("U>", 'Û'),
        ("a:", 'ä'),
        ("e:", 'ë'),
        ("i:", 'ï'),
        ("o:", 'ö'),
        ("u:", 'ü'),
        ("y:", 'ÿ'),
        ("A:", 'Ä'),
        ("E:", 'Ë'),
        ("I:", 'Ï'),
        ("O:", 'Ö'),
        ("U:", 'Ü'),
        ("a?", 'ã'),
        ("n?", 'ñ'),
        ("o?", 'õ'),
        ("A?", 'Ã'),
        ("N?", 'Ñ'),
        ("O?", 'Õ'),
        ("c,", 'ç'),
        ("C,", 'Ç'),
        ("ss", 'ß'),
        ("ae", 'æ'),
        ("AE", 'Æ'),
        ("o/", 'ø'),
        ("O/", 'Ø'),
        ("aa", 'å'),
        ("AA", 'Å'),
        ("oe", 'œ'),
        ("OE", 'Œ'),
        ("Eu", '€'),
        ("Pd", '£'),
        ("Ye", '¥'),
        ("Ct", '¢'),
        ("Co", '©'),
        ("Rg", '®'),
        ("TM", '™'),
        ("SE", '§'),
        ("PI", '¶'),
        ("DG", '°'),
        ("My", 'µ'),
        ("+-", '±'),
        ("*X", '×'),
        ("-:", '÷'),
        ("12", '½'),
        ("14", '¼'),
        ("34", '¾'),
        ("1S", '¹'),
        ("2S", '²'),
        ("3S", '³'),
        ("<<", '«'),
        (">>", '»'),
        ("!I", '¡'),
        ("?I", '¿'),
        ("-N", '–'),
        ("-M", '—'),
        ("'6", '‘'),
        ("'9", '’'),
        ("\"6", '“'),
        ("\"9", '”'),
        (".M", '·'),
        ("..", '…'),
        ("->", '→'),
        ("<-", '←'),
        ("-!", '↑'),
        ("-v", '↓'),
        ("=>", '⇒'),
        ("<=", '≤'),
        (">=", '≥'),
        ("!=", '≠'),
        ("?=", '≅'),
        ("=3", '≡'),
        ("00", '∞'),
        ("RT", '√'),
        ("OK", '✓'),
        ("XX", '✗'),
        ("a*", 'α'),
        ("b*", 'β'),
        ("g*", 'γ'),
        ("d*", 'δ'),
        ("e*", 'ε'),
        ("z*", 'ζ'),
        ("y*", 'η'),
        ("h*", 'θ'),
        ("i*", 'ι'),
        ("k*", 'κ'),
        ("l*", 'λ'),
        ("m*", 'μ'),
        ("n*", 'ν'),
        ("c*", 'ξ'),
        ("o*", 'ο'),
        ("p*", 'π'),
        ("r*", 'ρ'),
        ("s*", 'σ'),
        ("t*", 'τ'),
        ("u*", 'υ'),
        ("f*", 'φ'),
        ("x*", 'χ'),
        ("q*", 'ψ'),
        ("w*", 'ω'),
        ("D*", 'Δ'),
        ("G*", 'Γ'),
        ("L*", 'Λ'),
        ("P*", 'Π'),
        ("S*", 'Σ'),
        ("F*", 'Φ'),
        ("W*", 'Ω'),
        ("NS", '\u{a0}'),
    ];
    let key: String = [a, b].iter().collect();
    table.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
}
