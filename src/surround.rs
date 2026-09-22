//! vim-surround style add/delete/change of an enclosing pair.
//!
//! `ds`/`cs`/`ys` are reached by intercepting `s` as an operator
//! continuation in `normal.rs` (it is never a valid motion or doubling
//! character for d/c/y). Visual `S` is handled directly in `visual.rs`,
//! which computes the selection's char range and jumps straight to
//! `Stage::AddDelim`.
//!
//! Scope for this slice: `ds`/`cs` and the text-object operand of `ys`
//! recognize the standard bracket/quote characters (reusing
//! `textobject::resolve`, so multi-line brackets and same-line quotes work
//! exactly like `di(`/`da"` do already). Plain-motion operands
//! (`ysw"`, `ys$)`) are not implemented, nor are counts, dot-repeat or
//! registers -- see NEOVIM_PARITY_PLAN.md's progress log.
use crate::editor::Editor;
use crate::key::Key;
use crate::normal::Awaiting;
use crate::textobject::{self, ObjectKind};

#[derive(Debug, Clone)]
pub enum Stage {
    /// `ds` -- awaiting the delimiter character to remove.
    Delete,
    /// `cs` -- awaiting the delimiter character to replace.
    ChangeFrom,
    /// `cs<from>` resolved -- awaiting the replacement character.
    ChangeTo(char),
    /// `ys` -- awaiting `i`/`a` (text object) or `s` (whole line).
    AddOperand,
    /// `ys(i|a)` -- awaiting the object character (`w`, `"`, `(`, ...).
    AddTextObject { inner: bool },
    /// Operand resolved to a char range -- awaiting the delimiter to wrap it in.
    AddDelim { start: usize, end: usize },
}

/// Delimiter characters `ds`/`cs` can search for -- real enclosing pairs
/// only, never a bare text object like a word (which has no delimiter
/// characters to strip).
fn delim_kind_for(c: char) -> Option<ObjectKind> {
    Some(match c {
        '(' | ')' | 'b' => ObjectKind::Paren,
        '{' | '}' | 'B' => ObjectKind::Brace,
        '[' | ']' | 'r' => ObjectKind::Bracket,
        '<' | '>' => ObjectKind::Angle,
        '"' => ObjectKind::DoubleQuote,
        '\'' => ObjectKind::SingleQuote,
        '`' => ObjectKind::Backtick,
        _ => return None,
    })
}

/// Operand characters `ys(i|a)` can wrap -- the same delimiter set plus a
/// plain word, since `ysiw"` is the single most common surround command.
fn operand_kind_for(c: char) -> Option<ObjectKind> {
    Some(match c {
        'w' => ObjectKind::Word(false),
        'W' => ObjectKind::Word(true),
        _ => return delim_kind_for(c),
    })
}

/// The two literal strings to insert for a requested surround character.
/// Typing the *open* half of a bracket pads with a space inside (`( x )`,
/// matching vim-surround); the close half, an alias (`b`/`B`/`r`), or any
/// other character does not.
fn insert_delims(c: char) -> (String, String) {
    match c {
        '(' => ("( ".into(), " )".into()),
        ')' | 'b' => ("(".into(), ")".into()),
        '[' => ("[ ".into(), " ]".into()),
        ']' | 'r' => ("[".into(), "]".into()),
        '{' => ("{ ".into(), " }".into()),
        '}' | 'B' => ("{".into(), "}".into()),
        '<' => ("< ".into(), " >".into()),
        '>' => ("<".into(), ">".into()),
        c => (c.to_string(), c.to_string()),
    }
}

fn whole_line_span(ed: &Editor) -> Option<(usize, usize)> {
    let (line, _) = ed.cursor();
    let text = ed.buf().line_text(line);
    let first = text.chars().take_while(|c| c.is_whitespace()).count();
    let trimmed_len = text.trim_end().chars().count();
    if trimmed_len <= first {
        return None;
    }
    let base = ed.buf().char_idx(line, 0);
    Some((base + first, base + trimmed_len))
}

/// Wraps the exclusive char range `[start, end)` in the delimiters for `c`.
pub(crate) fn wrap(ed: &mut Editor, start: usize, end: usize, c: char) {
    let (open, close) = insert_delims(c);
    ed.buf_mut().begin_edit();
    ed.buf_mut().insert_str_at(end, &close);
    ed.buf_mut().insert_str_at(start, &open);
    ed.buf_mut().commit_edit();
    let (l, col) = ed.buf().pos_from_char_idx(start);
    ed.set_cursor(l, col);
}

fn delete(ed: &mut Editor, c: char) {
    let Some(kind) = delim_kind_for(c) else {
        ed.set_message(format!("no such surround: {c}"));
        return;
    };
    let (line, col) = ed.cursor();
    let Some((sl, sc, el, ec)) = textobject::resolve(ed.buf(), line, col, kind, false) else {
        return;
    };
    let start = ed.buf().char_idx(sl, sc);
    let end_incl = ed.buf().char_idx(el, ec);
    if end_incl <= start {
        return;
    }
    let bracket_like = matches!(
        kind,
        ObjectKind::Paren | ObjectKind::Brace | ObjectKind::Bracket | ObjectKind::Angle
    );
    let inside_start = start + 1;
    let inside_end = end_incl;
    let trims_padding = bracket_like
        && inside_end > inside_start + 1
        && ed.buf().rope.char(inside_start) == ' '
        && ed.buf().rope.char(inside_end - 1) == ' ';
    let (open_del_end, close_del_start) = if trims_padding {
        (inside_start + 1, inside_end - 1)
    } else {
        (inside_start, inside_end)
    };
    ed.buf_mut().begin_edit();
    ed.buf_mut()
        .delete_char_range(close_del_start, end_incl + 1);
    ed.buf_mut().delete_char_range(start, open_del_end);
    ed.buf_mut().commit_edit();
    let (l, c2) = ed.buf().pos_from_char_idx(start);
    ed.set_cursor(l, c2);
}

fn change(ed: &mut Editor, from: char, to: char) {
    let Some(kind) = delim_kind_for(from) else {
        ed.set_message(format!("no such surround: {from}"));
        return;
    };
    let (line, col) = ed.cursor();
    let Some((sl, sc, el, ec)) = textobject::resolve(ed.buf(), line, col, kind, false) else {
        return;
    };
    let start = ed.buf().char_idx(sl, sc);
    let end_incl = ed.buf().char_idx(el, ec); // char index of the closing delimiter
    if end_incl <= start {
        return;
    }
    // Strip one space of existing inner padding for bracket pairs, so the
    // replacement's own padding rule (an open bracket pads, a close bracket or
    // quote does not) applies cleanly without doubling or orphaning spaces --
    // e.g. `cs({` on `( x )` yields `{ x }`, and `cs(}` yields `{x}`.
    let bracket_like = matches!(
        kind,
        ObjectKind::Paren | ObjectKind::Brace | ObjectKind::Bracket | ObjectKind::Angle
    );
    let inside_start = start + 1;
    let trims_padding = bracket_like
        && end_incl > inside_start + 1
        && ed.buf().rope.char(inside_start) == ' '
        && ed.buf().rope.char(end_incl - 1) == ' ';
    let (content_start, content_end) = if trims_padding {
        (inside_start + 1, end_incl - 1)
    } else {
        (inside_start, end_incl)
    };
    let content = ed.buf().text_range(content_start, content_end);
    let (open, close) = insert_delims(to);
    ed.buf_mut().begin_edit();
    ed.buf_mut().delete_char_range(start, end_incl + 1);
    ed.buf_mut()
        .insert_str_at(start, &format!("{open}{content}{close}"));
    ed.buf_mut().commit_edit();
    let (l, c2) = ed.buf().pos_from_char_idx(start);
    ed.set_cursor(l, c2);
}

pub fn handle(ed: &mut Editor, stage: Stage, key: Key) {
    match stage {
        Stage::Delete => {
            if let Some(c) = key.as_char() {
                delete(ed, c);
            }
            ed.pending.reset();
        }
        Stage::ChangeFrom => {
            if let Some(c) = key.as_char() {
                ed.pending.awaiting = Some(Awaiting::Surround(Stage::ChangeTo(c)));
            } else {
                ed.pending.reset();
            }
        }
        Stage::ChangeTo(from) => {
            if let Some(to) = key.as_char() {
                change(ed, from, to);
            }
            ed.pending.reset();
        }
        Stage::AddOperand => match key {
            Key::Char('s') => {
                if let Some((start, end)) = whole_line_span(ed) {
                    ed.pending.awaiting = Some(Awaiting::Surround(Stage::AddDelim { start, end }));
                } else {
                    ed.pending.reset();
                }
            }
            Key::Char('i') => {
                ed.pending.awaiting =
                    Some(Awaiting::Surround(Stage::AddTextObject { inner: true }));
            }
            Key::Char('a') => {
                ed.pending.awaiting =
                    Some(Awaiting::Surround(Stage::AddTextObject { inner: false }));
            }
            _ => ed.pending.reset(),
        },
        Stage::AddTextObject { inner } => {
            let Some(kind) = key.as_char().and_then(operand_kind_for) else {
                ed.pending.reset();
                return;
            };
            let (line, col) = ed.cursor();
            let Some((sl, sc, el, ec)) = textobject::resolve(ed.buf(), line, col, kind, inner)
            else {
                ed.pending.reset();
                return;
            };
            let start = ed.buf().char_idx(sl, sc);
            let end = ed.buf().char_idx(el, ec) + 1;
            if end <= start {
                ed.pending.reset();
                return;
            }
            ed.pending.awaiting = Some(Awaiting::Surround(Stage::AddDelim { start, end }));
        }
        Stage::AddDelim { start, end } => {
            if let Some(c) = key.as_char() {
                wrap(ed, start, end, c);
            }
            ed.pending.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{config::Config, editor::Editor, key::Key};

    fn editor(text: &str) -> Editor {
        let cfg = Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..Config::default()
        };
        let mut e = Editor::new(cfg);
        e.buf_mut().rope = ropey::Rope::from_str(text);
        e.buf_mut().mark_saved();
        e
    }
    fn keys(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.feed_key(match c {
                '\x1b' => Key::Esc,
                '\n' => Key::Enter,
                _ => Key::Char(c),
            });
        }
    }

    #[test]
    fn add_around_inner_word() {
        let mut e = editor("say hello world\n");
        keys(&mut e, "wysiw\"");
        assert_eq!(e.buf().rope.to_string(), "say \"hello\" world\n");
    }

    #[test]
    fn add_paren_pads_open_variant() {
        let mut e = editor("hello\n");
        keys(&mut e, "ysiw(");
        assert_eq!(e.buf().rope.to_string(), "( hello )\n");
        let mut e = editor("hello\n");
        keys(&mut e, "ysiw)");
        assert_eq!(e.buf().rope.to_string(), "(hello)\n");
    }

    #[test]
    fn add_around_text_object_includes_delimiters() {
        let mut e = editor("(hello)\n");
        keys(&mut e, "ysa(\"");
        assert_eq!(e.buf().rope.to_string(), "\"(hello)\"\n");
    }

    #[test]
    fn yss_wraps_whole_line() {
        let mut e = editor("  hello world  \n");
        keys(&mut e, "yss*");
        assert_eq!(e.buf().rope.to_string(), "  *hello world*  \n");
    }

    #[test]
    fn delete_quotes() {
        let mut e = editor("say \"hello\" world\n");
        keys(&mut e, "fhds\"");
        assert_eq!(e.buf().rope.to_string(), "say hello world\n");
    }

    #[test]
    fn delete_paren_trims_padding() {
        let mut e = editor("( hello )\n");
        keys(&mut e, "ds(");
        assert_eq!(e.buf().rope.to_string(), "hello\n");
    }

    #[test]
    fn change_quote_to_paren() {
        let mut e = editor("say 'hello' world\n");
        keys(&mut e, "fhcs'\"");
        assert_eq!(e.buf().rope.to_string(), "say \"hello\" world\n");
    }

    #[test]
    fn change_paren_to_bracket_alias() {
        let mut e = editor("(hello)\n");
        keys(&mut e, "csbr");
        assert_eq!(e.buf().rope.to_string(), "[hello]\n");
    }

    #[test]
    fn delete_multiline_brace() {
        let mut e = editor("fn f() {\n    x\n}\n");
        keys(&mut e, "jds{");
        assert_eq!(e.buf().rope.to_string(), "fn f() \n    x\n\n");
    }

    #[test]
    fn no_such_surround_leaves_buffer_unchanged() {
        let mut e = editor("hello\n");
        keys(&mut e, "dsq");
        assert_eq!(e.buf().rope.to_string(), "hello\n");
    }

    #[test]
    fn visual_surround_wraps_selection() {
        let mut e = editor("hello world\n");
        keys(&mut e, "vllll"); // select "hello" (5 chars, v starts at h)
        e.feed_key(Key::Char('S'));
        e.feed_key(Key::Char('"'));
        assert_eq!(e.buf().rope.to_string(), "\"hello\" world\n");
    }
}
