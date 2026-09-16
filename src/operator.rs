use crate::buffer::Buffer;
use crate::registers::Registers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorKind {
    ToggleCase,
    Delete,
    Change,
    Yank,
    IndentRight,
    IndentLeft,
}

/// Delete (and optionally yank) a char range, returning the removed text.
/// `start`/`end` are char indices, `end` exclusive.
pub fn delete_range(
    buf: &mut Buffer,
    registers: &mut Registers,
    reg: Option<char>,
    start: usize,
    end: usize,
    linewise: bool,
) -> String {
    buf.begin_edit();
    let text = buf.delete_char_range(start, end);
    buf.commit_edit();
    registers.set(reg, text.clone(), linewise);
    text
}

pub fn yank_range(
    buf: &Buffer,
    registers: &mut Registers,
    reg: Option<char>,
    start: usize,
    end: usize,
    linewise: bool,
) {
    let text = buf.text_range(start, end);
    registers.set(reg, text, linewise);
}

/// Shift a line range left/right by one shiftwidth.
pub fn indent_lines(
    buf: &mut Buffer,
    start_line: usize,
    end_line: usize,
    right: bool,
    shiftwidth: usize,
) {
    buf.begin_edit();
    for line in start_line..=end_line {
        if line >= buf.line_count() {
            break;
        }
        let text = buf.line_text(line);
        if right {
            let pad = " ".repeat(shiftwidth);
            buf.insert_str(line, 0, &pad);
        } else {
            let to_strip = text
                .chars()
                .take(shiftwidth)
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            if to_strip > 0 {
                let start = buf.char_idx(line, 0);
                buf.delete_char_range(start, start + to_strip);
            }
        }
    }
    buf.commit_edit();
}

pub fn paste(
    buf: &mut Buffer,
    registers: &mut Registers,
    reg: Option<char>,
    position: (usize, usize),
    after: bool,
    count: usize,
    tab: usize,
) -> Option<(usize, usize)> {
    let (line, col) = position;
    let mut entry = registers.get(reg)?.clone();
    if entry.block_width.is_some() {
        let target_col = if after {
            crate::grapheme::step(&buf.line_text(line), col, 1, true)
        } else {
            col
        };
        let target = crate::grapheme::cell(&buf.line_text(line), target_col, tab);
        buf.begin_edit();
        for (i, text) in entry.text.lines().enumerate() {
            let line = line + i;
            while line >= buf.line_count() {
                let end = buf.rope.len_chars();
                buf.insert_char_at(end, '\n');
            }
            let old = buf.line_text(line);
            let mut text_line = crate::grapheme::expand_tabs(&old, tab);
            let width = unicode_width::UnicodeWidthStr::width(text_line.as_str());
            if width < target {
                text_line.push_str(&" ".repeat(target - width));
            }
            let at = crate::grapheme::column(&text_line, target, false);
            let start = buf.char_idx(line, 0);
            let end = buf.char_idx(line, old.chars().count());
            buf.delete_char_range(start, end);
            buf.insert_str_at(start, &text_line);
            buf.insert_str(line, at, &text.repeat(count.min(10000)));
        }
        buf.commit_edit();
        return Some((
            line,
            crate::grapheme::column(&buf.line_text(line), target, false),
        ));
    }
    entry.text = entry.text.repeat(count.min(10000));
    if entry.text.is_empty() {
        return None;
    }
    buf.begin_edit();
    let new_pos = if entry.linewise {
        let insert_line = if after { line + 1 } else { line };
        let idx = if insert_line >= buf.line_count() {
            buf.rope.len_chars()
        } else {
            buf.char_idx(insert_line, 0)
        };
        let mut text = entry.text.clone();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        if idx > 0 && idx == buf.rope.len_chars() && buf.rope.char(idx - 1) != '\n' {
            buf.insert_char_at(idx, '\n');
        }
        let idx = if insert_line >= buf.line_count() {
            buf.rope.len_chars()
        } else {
            buf.char_idx(insert_line, 0)
        };
        buf.insert_str_at(idx, &text);
        (insert_line, buf.first_non_blank(insert_line))
    } else {
        let col = if after {
            crate::grapheme::step(&buf.line_text(line), col, 1, true)
        } else {
            col
        };
        let idx = buf.char_idx(line, col);
        buf.insert_str_at(idx, &entry.text);
        buf.pos_from_char_idx(idx + entry.text.chars().count().saturating_sub(1))
    };
    buf.commit_edit();
    Some(new_pos)
}

pub fn toggle_case(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_uppercase() {
                c.to_lowercase().collect::<String>()
            } else {
                c.to_uppercase().collect::<String>()
            }
        })
        .collect()
}
