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
    Argument,
    /// `ip`/`ap` -- a paragraph (a run of non-blank lines, or a run of blank
    /// lines). Applied linewise by the caller.
    Paragraph,
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
    } else if big || c.is_alphanumeric() || c == '_' {
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
        ObjectKind::Argument => argument_object(buf, line, col, inner),
        ObjectKind::Paragraph => paragraph_object(buf, line, inner),
    }
}

/// `ip`/`ap`: a paragraph is a maximal run of same-kind lines -- all non-blank,
/// or all blank. `ip` is that run; `ap` also takes the trailing run of the
/// opposite kind (blank lines after a text paragraph), or the leading run when
/// none follows. Returned as a linewise span (columns are placeholders; the
/// caller treats a `Paragraph` object as linewise).
fn paragraph_object(
    buf: &Buffer,
    line: usize,
    inner: bool,
) -> Option<(usize, usize, usize, usize)> {
    let n = buf.line_count();
    if n == 0 {
        return None;
    }
    let is_blank = |l: usize| buf.line_text(l).trim().is_empty();
    let kind = is_blank(line);
    let mut start = line;
    while start > 0 && is_blank(start - 1) == kind {
        start -= 1;
    }
    let mut end = line;
    while end + 1 < n && is_blank(end + 1) == kind {
        end += 1;
    }
    if !inner {
        // `ap`: extend over the following opposite-kind run; if there is none,
        // extend over the preceding one instead.
        let mut e2 = end;
        while e2 + 1 < n && is_blank(e2 + 1) != kind {
            e2 += 1;
        }
        if e2 != end {
            end = e2;
        } else {
            while start > 0 && is_blank(start - 1) != kind {
                start -= 1;
            }
        }
    }
    Some((start, 0, end, 0))
}

/// Finds the innermost `(open ..= close)` pair enclosing char index `idx`,
/// returning `(open_idx, close_idx)`. Nesting of the same bracket is matched.
fn enclosing_pair(buf: &Buffer, idx: usize, open: char, close: char) -> Option<(usize, usize)> {
    let len = buf.rope.len_chars();
    if len == 0 {
        return None;
    }
    let idx = idx.min(len - 1);
    let mut depth: i32 = 0;
    let mut open_idx: Option<usize> = None;
    let mut i = idx as i64;
    while i >= 0 {
        let ch = buf.rope.char(i as usize);
        if ch == close && i as usize != idx {
            depth += 1;
        } else if ch == open {
            if depth == 0 {
                open_idx = Some(i as usize);
                break;
            }
            depth -= 1;
        }
        i -= 1;
    }
    let open_idx = open_idx?;
    let mut depth: i32 = 0;
    let mut j = open_idx + 1;
    while j < len {
        let ch = buf.rope.char(j);
        if ch == open {
            depth += 1;
        } else if ch == close {
            if depth == 0 {
                return Some((open_idx, j));
            }
            depth -= 1;
        }
        j += 1;
    }
    None
}

/// Comma char indices that separate top-level arguments within the half-open
/// region `[from, to)`. Commas inside nested brackets or quotes are skipped.
fn top_level_commas(buf: &Buffer, from: usize, to: usize) -> Vec<usize> {
    let mut commas = Vec::new();
    let mut depth: i32 = 0;
    let mut quote: Option<char> = None;
    let mut i = from;
    while i < to {
        let ch = buf.rope.char(i);
        if let Some(q) = quote {
            if ch == '\\' {
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            }
        } else {
            match ch {
                '"' | '\'' | '`' => quote = Some(ch),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => commas.push(i),
                _ => {}
            }
        }
        i += 1;
    }
    commas
}

/// Argument text object (`aa`/`ia`) within the nearest enclosing `(...)`.
/// `ia` selects the argument under the cursor, trimmed of surrounding
/// whitespace and without any comma. `aa` additionally takes one separating
/// comma: the trailing comma + following whitespace for a non-last argument,
/// or the leading comma for the last argument.
fn argument_object(
    buf: &Buffer,
    line: usize,
    col: usize,
    inner: bool,
) -> Option<(usize, usize, usize, usize)> {
    let len = buf.rope.len_chars();
    if len == 0 {
        return None;
    }
    let idx = buf.char_idx(line, col).min(len - 1);
    let (open_idx, close_idx) = enclosing_pair(buf, idx, '(', ')')?;
    if close_idx <= open_idx + 1 {
        return None; // empty `()` — no arguments
    }
    let commas = top_level_commas(buf, open_idx + 1, close_idx);

    // Delimiter boundaries: the open paren, each top-level comma, the close.
    let mut delims = Vec::with_capacity(commas.len() + 2);
    delims.push(open_idx);
    delims.extend(commas.iter().copied());
    delims.push(close_idx);

    // Slot k holds the argument between delims[k] and delims[k+1].
    let mut k = delims.len() - 2;
    for w in 0..delims.len() - 1 {
        if idx >= delims[w] && idx < delims[w + 1] {
            k = w;
            break;
        }
    }
    let raw_start = delims[k] + 1;
    let raw_end = delims[k + 1]; // exclusive

    let ws = |i: usize| buf.rope.char(i).is_whitespace();
    // Trim whitespace to get the argument content [is, ie] (inclusive).
    let mut is = raw_start;
    while is < raw_end && ws(is) {
        is += 1;
    }
    let mut ie = raw_end.saturating_sub(1);
    while ie >= raw_start && ws(ie) {
        if ie == raw_start {
            break;
        }
        ie -= 1;
    }
    let empty = is >= raw_end || is > ie;

    let span = |a: usize, b: usize| {
        let (sl, sc) = buf.pos_from_char_idx(a);
        let (el, ec) = buf.pos_from_char_idx(b);
        (sl, sc, el, ec)
    };

    if inner {
        if empty {
            let (l, c) = buf.pos_from_char_idx(raw_start);
            return Some((l, c, l, c.saturating_sub(1)));
        }
        return Some(span(is, ie));
    }

    // `aa`: include one separating comma.
    let num_commas = commas.len();
    let has_trailing_comma = k < num_commas; // delims[k+1] is a comma
    let start = if empty { raw_start } else { is };
    if has_trailing_comma {
        // Extend across the trailing comma and any whitespace after it.
        let comma = delims[k + 1];
        let mut end = comma;
        while end + 1 < close_idx && ws(end + 1) {
            end += 1;
        }
        Some(span(start, end))
    } else if k > 0 {
        // Last argument: take the leading comma instead.
        let comma = delims[k];
        let end = if empty { comma } else { ie };
        Some(span(comma, end))
    } else {
        // Sole argument, no comma to take.
        if empty {
            let (l, c) = buf.pos_from_char_idx(raw_start);
            return Some((l, c, l, c.saturating_sub(1)));
        }
        Some(span(is, ie))
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
        .filter(|(i, c)| {
            **c == q && text[..*i].iter().rev().take_while(|c| **c == '\\').count() % 2 == 0
        })
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
