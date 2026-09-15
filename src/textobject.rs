use crate::buffer::Buffer;

#[derive(Debug, Clone, Copy)]
pub enum ObjectKind {
    Word(bool),
    Paren,
    Brace,
    Bracket,
    Angle,
    DoubleQuote,
    SingleQuote,
    Backtick,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if big {
        Class::Word
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// Returns an inclusive (start_line, start_col, end_line, end_col) span.
pub fn resolve(
    buf: &Buffer,
    line: usize,
    col: usize,
    kind: ObjectKind,
    inner: bool,
) -> Option<(usize, usize, usize, usize)> {
    match kind {
        ObjectKind::Word(big) => word_object(buf, line, col, big, inner),
        ObjectKind::Paren => bracket_object(buf, line, col, '(', ')', inner),
        ObjectKind::Brace => bracket_object(buf, line, col, '{', '}', inner),
        ObjectKind::Bracket => bracket_object(buf, line, col, '[', ']', inner),
        ObjectKind::Angle => bracket_object(buf, line, col, '<', '>', inner),
        ObjectKind::DoubleQuote => quote_object(buf, line, col, '"', inner),
        ObjectKind::SingleQuote => quote_object(buf, line, col, '\'', inner),
        ObjectKind::Backtick => quote_object(buf, line, col, '`', inner),
    }
}

fn word_object(
    buf: &Buffer,
    line: usize,
    col: usize,
    big: bool,
    inner: bool,
) -> Option<(usize, usize, usize, usize)> {
    let text: Vec<char> = buf.line_text(line).chars().collect();
    if text.is_empty() {
        return None;
    }
    let col = col.min(text.len() - 1);
    let c0 = class(text[col], big);
    let mut start = col;
    while start > 0 && class(text[start - 1], big) == c0 {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < text.len() && class(text[end + 1], big) == c0 {
        end += 1;
    }
    if inner {
        return Some((line, start, line, end));
    }
    let mut end2 = end;
    let mut extended = false;
    while end2 + 1 < text.len() && class(text[end2 + 1], big) == Class::Space {
        end2 += 1;
        extended = true;
    }
    if extended {
        return Some((line, start, line, end2));
    }
    let mut start2 = start;
    while start2 > 0 && class(text[start2 - 1], big) == Class::Space {
        start2 -= 1;
    }
    Some((line, start2, line, end))
}

fn bracket_object(
    buf: &Buffer,
    line: usize,
    col: usize,
    open: char,
    close: char,
    inner: bool,
) -> Option<(usize, usize, usize, usize)> {
    let idx = buf.char_idx(line, col);
    let len = buf.rope.len_chars();
    if len == 0 {
        return None;
    }
    let idx = idx.min(len - 1);

    let mut depth: i32 = 0;
    let mut open_idx: Option<usize> = None;
    let mut i = idx as i64;
    loop {
        if i < 0 {
            break;
        }
        let ch = buf.rope.char(i as usize);
        if ch == close && i as usize != idx {
            depth += 1;
        } else if ch == open {
            if depth == 0 {
                open_idx = Some(i as usize);
                break;
            } else {
                depth -= 1;
            }
        }
        i -= 1;
    }
    let open_idx = open_idx?;

    let mut depth: i32 = 0;
    let mut close_idx: Option<usize> = None;
    let mut j = open_idx + 1;
    while j < len {
        let ch = buf.rope.char(j);
        if ch == open {
            depth += 1;
        } else if ch == close {
            if depth == 0 {
                close_idx = Some(j);
                break;
            } else {
                depth -= 1;
            }
        }
        j += 1;
    }
    let close_idx = close_idx?;

    let (start_idx, end_idx) = if inner {
        if close_idx > open_idx + 1 {
            (open_idx + 1, close_idx - 1)
        } else {
            // empty pair `()`: nothing inside
            (open_idx + 1, open_idx)
        }
    } else {
        (open_idx, close_idx)
    };
    if inner && start_idx > end_idx {
        let (l, c) = buf.pos_from_char_idx(start_idx);
        return Some((l, c, l, c.saturating_sub(1)));
    }
    let (sl, sc) = buf.pos_from_char_idx(start_idx);
    let (el, ec) = buf.pos_from_char_idx(end_idx);
    Some((sl, sc, el, ec))
}

fn quote_object(
    buf: &Buffer,
    line: usize,
    col: usize,
    q: char,
    inner: bool,
) -> Option<(usize, usize, usize, usize)> {
    let text: Vec<char> = buf.line_text(line).chars().collect();
    let positions: Vec<usize> = text
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == q)
        .map(|(i, _)| i)
        .collect();
    if positions.len() < 2 {
        return None;
    }
    let mut pair = None;
    let mut k = 0;
    while k + 1 < positions.len() {
        let a = positions[k];
        let b = positions[k + 1];
        if col >= a && col <= b {
            pair = Some((a, b));
            break;
        }
        if a > col {
            pair = Some((a, b));
            break;
        }
        k += 2;
    }
    let (a, b) = pair?;
    if inner {
        if b > a + 1 {
            Some((line, a + 1, line, b - 1))
        } else {
            Some((line, a + 1, line, a))
        }
    } else {
        Some((line, a, line, b))
    }
}
