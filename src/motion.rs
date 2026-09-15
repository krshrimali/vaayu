use crate::buffer::Buffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Empty,
    Inclusive,
    Exclusive,
    Linewise,
}

#[derive(Debug, Clone, Copy)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    FirstNonBlank,
    LineEnd,
    WordFwd(bool),
    WordEndFwd(bool),
    WordBack(bool),
    FileStart,
    FileEnd,
    GotoLine(usize),
    FindChar {
        ch: char,
        before: bool,
        forward: bool,
    },
    ParaFwd,
    ParaBack,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char, big: bool) -> Class {
    if c == '\n' || c.is_whitespace() {
        Class::Space
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

fn fwd_word(buf: &Buffer, idx: usize, big: bool) -> usize {
    let len = buf.rope.len_chars();
    if idx >= len {
        return idx;
    }
    let mut i = idx;
    let start = class(buf.rope.char(i), big);
    if start != Class::Space {
        while i < len && class(buf.rope.char(i), big) == start {
            i += 1;
        }
    }
    while i < len && class(buf.rope.char(i), big) == Class::Space {
        i += 1;
    }
    i
}

fn end_word(buf: &Buffer, idx: usize, big: bool) -> usize {
    let len = buf.rope.len_chars();
    if len == 0 {
        return idx;
    }
    let mut i = (idx + 1).min(len - 1);
    while i < len - 1 && class(buf.rope.char(i), big) == Class::Space {
        i += 1;
    }
    if class(buf.rope.char(i), big) == Class::Space {
        return idx;
    }
    let c = class(buf.rope.char(i), big);
    while i + 1 < len && class(buf.rope.char(i + 1), big) == c {
        i += 1;
    }
    i
}

fn back_word(buf: &Buffer, idx: usize, big: bool) -> usize {
    if idx == 0 {
        return 0;
    }
    let mut i = idx - 1;
    while i > 0 && class(buf.rope.char(i), big) == Class::Space {
        i -= 1;
    }
    if class(buf.rope.char(i), big) != Class::Space {
        let c = class(buf.rope.char(i), big);
        while i > 0 && class(buf.rope.char(i - 1), big) == c {
            i -= 1;
        }
    }
    i
}

/// Resolve a motion from (line, col) applied `count` times.
/// Returns the destination (line, col) plus how an operator should treat the span.
pub fn resolve(
    buf: &Buffer,
    line: usize,
    col: usize,
    motion: Motion,
    count: usize,
) -> Option<(usize, usize, Span)> {
    let count = count.max(1);
    match motion {
        Motion::Left => {
            let nc = col.saturating_sub(count);
            Some((line, nc, Span::Exclusive))
        }
        Motion::Right => {
            let len = buf.line_len(line);
            let nc = (col + count).min(len);
            Some((line, nc, Span::Exclusive))
        }
        Motion::Up => {
            let nl = line.saturating_sub(count);
            Some((nl, col, Span::Linewise))
        }
        Motion::Down => {
            let nl = (line + count).min(buf.line_count().saturating_sub(1));
            Some((nl, col, Span::Linewise))
        }
        Motion::LineStart => Some((line, 0, Span::Exclusive)),
        Motion::FirstNonBlank => Some((line, buf.first_non_blank(line), Span::Exclusive)),
        Motion::LineEnd => {
            let nl = (line + count - 1).min(buf.line_count().saturating_sub(1));
            let len = buf.line_len(nl);
            Some((nl, len.saturating_sub(1), Span::Inclusive))
        }
        Motion::WordFwd(big) => {
            let mut idx = buf.char_idx(line, col);
            for _ in 0..count {
                idx = fwd_word(buf, idx, big);
            }
            let (l, c) = buf.pos_from_char_idx(idx);
            Some((l, c, Span::Exclusive))
        }
        Motion::WordEndFwd(big) => {
            let mut idx = buf.char_idx(line, col);
            for _ in 0..count {
                idx = end_word(buf, idx, big);
            }
            let (l, c) = buf.pos_from_char_idx(idx);
            Some((l, c, Span::Inclusive))
        }
        Motion::WordBack(big) => {
            let mut idx = buf.char_idx(line, col);
            for _ in 0..count {
                idx = back_word(buf, idx, big);
            }
            let (l, c) = buf.pos_from_char_idx(idx);
            Some((l, c, Span::Exclusive))
        }
        Motion::FileStart => Some((0, buf.first_non_blank(0), Span::Linewise)),
        Motion::FileEnd => {
            let l = buf.line_count().saturating_sub(1);
            Some((l, buf.first_non_blank(l), Span::Linewise))
        }
        Motion::GotoLine(n) => {
            let l = n.saturating_sub(1).min(buf.line_count().saturating_sub(1));
            Some((l, buf.first_non_blank(l), Span::Linewise))
        }
        Motion::FindChar {
            ch,
            before,
            forward,
        } => {
            let text: Vec<char> = buf.line_text(line).chars().collect();
            if forward {
                let mut found = None;
                let mut seen = 0;
                for (i, c) in text.iter().enumerate().skip(col + 1) {
                    if *c == ch {
                        seen += 1;
                        if seen == count {
                            found = Some(i);
                            break;
                        }
                    }
                }
                found.map(|i| {
                    let target = if before {
                        i.saturating_sub(1).max(col)
                    } else {
                        i
                    };
                    (line, target, Span::Inclusive)
                })
            } else {
                let mut found = None;
                let mut seen = 0;
                for i in (0..col).rev() {
                    if text[i] == ch {
                        seen += 1;
                        if seen == count {
                            found = Some(i);
                            break;
                        }
                    }
                }
                found.map(|i| {
                    let target = if before { (i + 1).min(col) } else { i };
                    (line, target, Span::Exclusive)
                })
            }
        }
        Motion::ParaFwd => {
            let mut l = line;
            let last = buf.line_count().saturating_sub(1);
            for _ in 0..count {
                loop {
                    if l >= last {
                        l = last;
                        break;
                    }
                    l += 1;
                    if buf.line_len(l) == 0 {
                        break;
                    }
                }
            }
            Some((l, 0, Span::Exclusive))
        }
        Motion::ParaBack => {
            let mut l = line;
            for _ in 0..count {
                loop {
                    if l == 0 {
                        break;
                    }
                    l -= 1;
                    if buf.line_len(l) == 0 {
                        break;
                    }
                }
            }
            Some((l, 0, Span::Exclusive))
        }
    }
}
