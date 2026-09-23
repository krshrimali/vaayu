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
    SubwordFwd,
    SubwordEndFwd,
    SubwordBack,
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

/// camelCase/snake_case/kebab-case-aware subword classification: `_` and
/// `-` are gaps like whitespace (never part of a subword), and digits get
/// their own class so `var2Name` splits into `var` | `2` | `Name`.
#[derive(PartialEq, Eq, Clone, Copy)]
enum SubClass {
    Gap,
    Upper,
    Digit,
    Lower,
}

fn subclass(c: char) -> SubClass {
    if c == '\n' || c.is_whitespace() || c == '_' || c == '-' {
        SubClass::Gap
    } else if c.is_ascii_digit() {
        SubClass::Digit
    } else if c.is_uppercase() {
        SubClass::Upper
    } else {
        SubClass::Lower
    }
}

/// True if char index `i` is the first character of a subword: a class
/// change from `i - 1`, or the last letter of an acronym run right before
/// it turns into a new capitalized word (`XMLParser` -> `XML` | `Parser`:
/// the second `P` at `i` is Upper following Upper, but `i + 1` is Lower).
fn subword_starts_at(buf: &Buffer, i: usize) -> bool {
    let len = buf.rope.len_chars();
    if i >= len {
        return false;
    }
    let cur = subclass(buf.rope.char(i));
    if cur == SubClass::Gap {
        return false;
    }
    if i == 0 {
        return true;
    }
    let prev = subclass(buf.rope.char(i - 1));
    if prev == SubClass::Gap {
        return true;
    }
    if prev == cur {
        // Same run (e.g. an acronym): only the last upper before a lower
        // starts a new subword ("XMLParser" -> "XML" | "Parser").
        return cur == SubClass::Upper
            && i + 1 < len
            && subclass(buf.rope.char(i + 1)) == SubClass::Lower;
    }
    // A capital continues the subword that started at the previous
    // (upper) character -- "Var" is one subword, not "V" + "ar".
    !(prev == SubClass::Upper && cur == SubClass::Lower)
}

fn fwd_subword(buf: &Buffer, idx: usize) -> usize {
    let len = buf.rope.len_chars();
    let mut i = (idx + 1).min(len);
    while i < len && !subword_starts_at(buf, i) {
        i += 1;
    }
    i
}

fn back_subword(buf: &Buffer, idx: usize) -> usize {
    if idx == 0 {
        return 0;
    }
    let mut i = idx - 1;
    while i > 0 && !subword_starts_at(buf, i) {
        i -= 1;
    }
    i
}

fn end_subword(buf: &Buffer, idx: usize) -> usize {
    let len = buf.rope.len_chars();
    if len == 0 {
        return idx;
    }
    let mut i = (idx + 1).min(len - 1);
    while i < len - 1 && subclass(buf.rope.char(i)) == SubClass::Gap {
        i += 1;
    }
    if subclass(buf.rope.char(i)) == SubClass::Gap {
        return idx;
    }
    while i + 1 < len && !subword_starts_at(buf, i + 1) {
        i += 1;
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
            let nc = crate::grapheme::step(&buf.line_text(line), col, count, false);
            Some((line, nc, Span::Exclusive))
        }
        Motion::Right => {
            let len = buf.line_len(line);
            let nc = crate::grapheme::step(&buf.line_text(line), col, count, true).min(len);
            Some((line, nc, Span::Exclusive))
        }
        Motion::Up => {
            // Fold-aware: each step skips over a closed fold (a no-op mapping
            // to `line-1` when there are no folds).
            let mut nl = line;
            for _ in 0..count {
                nl = buf.visible_line_above(nl);
            }
            Some((nl, col, Span::Linewise))
        }
        Motion::Down => {
            let mut nl = line;
            for _ in 0..count {
                nl = buf.visible_line_below(nl);
            }
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
        Motion::SubwordFwd => {
            let mut idx = buf.char_idx(line, col);
            for _ in 0..count {
                idx = fwd_subword(buf, idx);
            }
            let (l, c) = buf.pos_from_char_idx(idx);
            Some((l, c, Span::Exclusive))
        }
        Motion::SubwordEndFwd => {
            let mut idx = buf.char_idx(line, col);
            for _ in 0..count {
                idx = end_subword(buf, idx);
            }
            let (l, c) = buf.pos_from_char_idx(idx);
            Some((l, c, Span::Inclusive))
        }
        Motion::SubwordBack => {
            let mut idx = buf.char_idx(line, col);
            for _ in 0..count {
                idx = back_subword(buf, idx);
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

#[cfg(test)]
mod subword_tests {
    use super::*;
    use crate::{config::Config, editor::Editor};

    fn buf(text: &str) -> Editor {
        let mut e = Editor::new(Config::default());
        e.buf_mut().rope = ropey::Rope::from_str(text);
        e
    }

    fn fwd_positions(text: &str) -> Vec<usize> {
        let e = buf(text);
        let len = text.chars().count();
        let mut col = 0;
        let mut out = vec![0];
        loop {
            match resolve(e.buf(), 0, col, Motion::SubwordFwd, 1) {
                Some((_, c, _)) if c != col && c < len => {
                    out.push(c);
                    col = c;
                }
                _ => break,
            }
        }
        out
    }

    #[test]
    fn camel_case_splits() {
        assert_eq!(fwd_positions("myVarName"), [0, 2, 5]);
    }

    #[test]
    fn acronym_tail_splits_before_last_upper() {
        // XMLParser -> "XML" | "Parser": the boundary is the second 'P'
        // (index 3), not consumed into the acronym run.
        assert_eq!(fwd_positions("XMLParser"), [0, 3]);
    }

    #[test]
    fn snake_and_kebab_case_split_on_separators() {
        assert_eq!(fwd_positions("snake_case_name"), [0, 6, 11]);
        assert_eq!(fwd_positions("kebab-case-name"), [0, 6, 11]);
    }

    #[test]
    fn digits_get_their_own_subword() {
        assert_eq!(fwd_positions("var2Name"), [0, 3, 4]);
    }

    #[test]
    fn backward_mirrors_forward() {
        let e = buf("myVarName\n");
        let (_, c, _) = resolve(e.buf(), 0, 9, Motion::SubwordBack, 1).unwrap();
        assert_eq!(c, 5);
        let (_, c, _) = resolve(e.buf(), 0, 5, Motion::SubwordBack, 1).unwrap();
        assert_eq!(c, 2);
        let (_, c, _) = resolve(e.buf(), 0, 2, Motion::SubwordBack, 1).unwrap();
        assert_eq!(c, 0);
    }

    #[test]
    fn end_forward_lands_on_last_char_of_each_subword() {
        let e = buf("myVarName\n");
        let (_, c, _) = resolve(e.buf(), 0, 0, Motion::SubwordEndFwd, 1).unwrap();
        assert_eq!(c, 1); // end of "my"
        let (_, c, _) = resolve(e.buf(), 0, 1, Motion::SubwordEndFwd, 1).unwrap();
        assert_eq!(c, 4); // end of "Var"
    }

    #[test]
    fn count_repeats_the_motion() {
        let e = buf("myVarName\n");
        let (_, c, _) = resolve(e.buf(), 0, 0, Motion::SubwordFwd, 2).unwrap();
        assert_eq!(c, 5);
    }
}
