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
    Format,
}

/// Reflow `text` to `width` display columns (`gq`). Blank (whitespace-only)
/// lines split it into paragraphs, each reflowed independently and rejoined
/// with the blank lines preserved. Each paragraph keeps the leading
/// whitespace of its first line as the indent for every produced line; words
/// are packed greedily. The result has no trailing newline.
pub fn reflow(text: &str, width: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            if !para.is_empty() {
                out.push(reflow_paragraph(&para.join("\n"), width));
                para.clear();
            }
            out.push(String::new());
        } else {
            para.push(line);
        }
    }
    if !para.is_empty() {
        out.push(reflow_paragraph(&para.join("\n"), width));
    }
    out.join("\n")
}

/// Detect a comment leader (`//`, `#`, `;`, `%`, `--`, or a ` * ` block-comment
/// continuation) at the start of `s` (already indent-stripped). Requires the
/// leader be followed by whitespace or end-of-line so `*ptr` / `#include`-style
/// tokens aren't mistaken for one. Returns "" when there's no leader.
fn comment_leader(s: &str) -> &'static str {
    for lead in ["///", "//", "#", ";", "%", "--", "*"] {
        if let Some(rest) = s.strip_prefix(lead) {
            if rest.is_empty() || rest.starts_with([' ', '\t']) {
                return lead;
            }
        }
    }
    ""
}

fn reflow_paragraph(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    let indent: String = text
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    // A comment block keeps its leader on every wrapped line; the words come
    // from each source line with the leader stripped.
    let first_line = text.lines().next().unwrap_or("");
    let leader = comment_leader(first_line.trim_start());
    let prefix = if leader.is_empty() {
        indent.clone()
    } else {
        format!("{indent}{leader} ")
    };
    let words: Vec<&str> = if leader.is_empty() {
        text.split_whitespace().collect()
    } else {
        text.lines()
            .flat_map(|l| {
                let l = l.trim_start();
                let l = l.strip_prefix(leader).unwrap_or(l);
                l.split_whitespace()
            })
            .collect()
    };
    if words.is_empty() {
        return text.to_string();
    }
    // Tabs are rare in reflowed prose; count each prefix char as one column.
    let prefix_w = prefix.chars().count();
    let limit = width.max(prefix_w + 1);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_w = 0usize;
    for w in words {
        let ww = UnicodeWidthStr::width(w);
        if line.is_empty() {
            line.push_str(&prefix);
            line.push_str(w);
            line_w = prefix_w + ww;
        } else if line_w + 1 + ww <= limit {
            line.push(' ');
            line.push_str(w);
            line_w += 1 + ww;
        } else {
            lines.push(std::mem::take(&mut line));
            line.push_str(&prefix);
            line.push_str(w);
            line_w = prefix_w + ww;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.join("\n")
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
    // A no-op delete (empty range: `x` on an empty line, `d0` at column 0,
    // `d$` on an empty line, ...) must not clobber the target/unnamed
    // register, matching Vim -- it would otherwise destroy a previous yank.
    if !text.is_empty() {
        registers.set(reg, text.clone(), linewise);
    }
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
    if !text.is_empty() {
        registers.set(reg, text, linewise);
    }
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
            // Vim leaves blank lines unchanged under `>`: no indent is added
            // to an empty or whitespace-only line (avoids trailing-whitespace
            // pollution when a selection spans blank lines).
            if text.trim().is_empty() {
                continue;
            }
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
    // A linewise register must end in a newline *before* it is repeated, so a
    // counted linewise paste stacks whole lines instead of concatenating the
    // last (newline-less) copy into the next. This happens when `yy` yanks the
    // final line of a file that has no trailing newline.
    if entry.linewise && !entry.text.is_empty() && !entry.text.ends_with('\n') {
        entry.text.push('\n');
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
