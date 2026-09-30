//! Vi-style single-line editing for the "query bar" every search picker
//! uses (the project file picker, the live-grep query, a Results list's
//! `f` filter): a cursor position plus an Insert/Normal sub-mode, so the
//! typed text can be edited in place -- word motions, `x`, mid-string
//! paste -- instead of only ever appending to or popping the last
//! character, which is all `String::push`/`String::pop` allowed before.
//!
//! The field's own text keeps living wherever it already did
//! (`FilePicker::query`, `Results::query`, `Results::filter`); this only
//! adds a `QueryCursor` alongside it and a `handle` that every such field's
//! key handler calls first, before falling back to its own Enter/Esc/list-
//! navigation handling for whatever `handle` didn't consume.
use crate::key::Key;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    Word,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// How many past (text, cursor) snapshots `u` can step back through --
/// bounded so a very long editing session doesn't grow this without limit
/// (mirrors `command.rs`'s own `MAX_HISTORY` reasoning for the same kind
/// of unbounded-Vec concern).
const MAX_UNDO: usize = 100;

/// Cursor position (a char index), Insert/Normal sub-mode, and undo
/// history for a one-line text field. In Insert sub-mode the cursor sits
/// *between* characters (`0..=len`, like every other text cursor in this
/// editor); in Normal sub-mode it sits *on* one (`0..=len.saturating_sub(1)`),
/// matching how a real buffer's Normal-mode cursor never rests past the
/// last character on a line.
///
/// `undo_stack` records one (text, cursor) snapshot per discrete edit --
/// each keystroke in Insert sub-mode included, unlike a real buffer's
/// Insert mode, which groups a whole typing run into one undo step. For a
/// short one-line field the finer granularity is harmless (and arguably
/// more precise); it just means `u` after typing "hello" takes five
/// presses to fully unwind, not one.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryCursor {
    pub pos: usize,
    pub insert: bool,
    undo_stack: Vec<(String, usize)>,
}

impl Default for QueryCursor {
    fn default() -> Self {
        QueryCursor {
            pos: 0,
            insert: true,
            undo_stack: Vec::new(),
        }
    }
}

impl QueryCursor {
    /// Cursor at the end of `text`, in Insert sub-mode -- the state a
    /// freshly opened or reopened query field has always started in.
    pub fn at_end(text: &str) -> Self {
        QueryCursor {
            pos: text.chars().count(),
            insert: true,
            undo_stack: Vec::new(),
        }
    }
}

/// Records `text`/`qc.pos` as an undo point before a mutation is applied.
/// Call this with the *pre-mutation* text, before writing the change back.
fn push_undo(qc: &mut QueryCursor, text: &str) {
    qc.undo_stack.push((text.to_string(), qc.pos));
    if qc.undo_stack.len() > MAX_UNDO {
        qc.undo_stack.remove(0);
    }
}

/// `w`: the start of the next word (or end-of-line if there isn't one).
fn word_forward(chars: &[char], pos: usize) -> usize {
    let len = chars.len();
    let mut i = pos;
    if i >= len {
        return len;
    }
    let start = class(chars[i]);
    if start != Class::Blank {
        while i < len && class(chars[i]) == start {
            i += 1;
        }
    }
    while i < len && class(chars[i]) == Class::Blank {
        i += 1;
    }
    i
}

/// `b`: the start of the previous word.
fn word_backward(chars: &[char], pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut i = pos - 1;
    while i > 0 && class(chars[i]) == Class::Blank {
        i -= 1;
    }
    if class(chars[i]) != Class::Blank {
        let c = class(chars[i]);
        while i > 0 && class(chars[i - 1]) == c {
            i -= 1;
        }
    }
    i
}

/// `e`: the end of the current word, or of the next one if the cursor is
/// already on the last character of this one.
fn word_end(chars: &[char], pos: usize) -> usize {
    let len = chars.len();
    if len == 0 {
        return 0;
    }
    let mut i = (pos + 1).min(len);
    while i < len && class(chars[i]) == Class::Blank {
        i += 1;
    }
    if i < len {
        let c = class(chars[i]);
        while i + 1 < len && class(chars[i + 1]) == c {
            i += 1;
        }
    }
    i.min(len - 1)
}

/// Strips characters a one-line query field must never contain: a pasted
/// newline could otherwise submit the field (as Enter) or corrupt the
/// single-row display, so it (and a lone CR) is dropped -- the same
/// sanitizing `Editor::insert_paste` already applies to a Command-mode
/// prompt.
pub fn sanitize_paste(text: &str) -> String {
    text.chars().filter(|&c| c != '\n' && c != '\r').collect()
}

/// Inserts already-sanitized `pasted` at the cursor and advances it past
/// the inserted text (clamped to the last character when in Normal
/// sub-mode, same as every other cursor-moving command below). Returns
/// whether anything was actually inserted, so callers only refilter /
/// reschedule when the text really changed.
pub fn paste_at(text: &mut String, qc: &mut QueryCursor, pasted: &str) -> bool {
    if pasted.is_empty() {
        return false;
    }
    push_undo(qc, text);
    let mut chars: Vec<char> = text.chars().collect();
    let pos = qc.pos.min(chars.len());
    let inserted: Vec<char> = pasted.chars().collect();
    let n = inserted.len();
    chars.splice(pos..pos, inserted);
    let len = chars.len();
    *text = chars.into_iter().collect();
    qc.pos = if qc.insert {
        (pos + n).min(len)
    } else {
        (pos + n).min(len.saturating_sub(1))
    };
    true
}

/// Feeds one key to a query field. `text` is the field's own string;
/// `qc` tracks this field's cursor/sub-mode. Returns `None` for any key
/// this module doesn't own (Enter, Esc while already in Normal sub-mode,
/// Ctrl-modified list-navigation keys, ...) so the caller's existing
/// handling for those still applies unchanged. Returns `Some(changed)`
/// for a key it did consume, `changed` true only when `text` itself was
/// modified (a pure cursor move is `Some(false)`).
pub fn handle(text: &mut String, qc: &mut QueryCursor, key: Key) -> Option<bool> {
    let mut chars: Vec<char> = text.chars().collect();
    let max_insert = chars.len();
    let max_normal = chars.len().saturating_sub(1);
    qc.pos = qc.pos.min(if qc.insert { max_insert } else { max_normal });

    if qc.insert {
        return match key {
            Key::Char(c) | Key::Literal(c) => {
                push_undo(qc, text);
                chars.insert(qc.pos, c);
                qc.pos += 1;
                *text = chars.into_iter().collect();
                Some(true)
            }
            Key::Backspace => {
                if qc.pos == 0 {
                    return Some(false);
                }
                push_undo(qc, text);
                qc.pos -= 1;
                chars.remove(qc.pos);
                *text = chars.into_iter().collect();
                Some(true)
            }
            Key::Delete => {
                if qc.pos >= chars.len() {
                    return Some(false);
                }
                push_undo(qc, text);
                chars.remove(qc.pos);
                *text = chars.into_iter().collect();
                Some(true)
            }
            // Readline/vim-insert-mode word-delete and clear-to-start --
            // available without leaving Insert sub-mode (unlike `x`/`D`,
            // which are Normal-sub-mode-only), since these are common
            // enough mid-typing edits that requiring an Esc round-trip
            // first would defeat the point.
            Key::Ctrl('w') => {
                let start = word_backward(&chars, qc.pos);
                if start >= qc.pos {
                    return Some(false);
                }
                push_undo(qc, text);
                chars.drain(start..qc.pos);
                qc.pos = start;
                *text = chars.into_iter().collect();
                Some(true)
            }
            Key::Ctrl('u') => {
                if qc.pos == 0 {
                    return Some(false);
                }
                push_undo(qc, text);
                chars.drain(0..qc.pos);
                qc.pos = 0;
                *text = chars.into_iter().collect();
                Some(true)
            }
            Key::Left => {
                qc.pos = qc.pos.saturating_sub(1);
                Some(false)
            }
            Key::Right => {
                qc.pos = (qc.pos + 1).min(chars.len());
                Some(false)
            }
            Key::Home => {
                qc.pos = 0;
                Some(false)
            }
            Key::End => {
                qc.pos = chars.len();
                Some(false)
            }
            Key::Esc => {
                qc.insert = false;
                qc.pos = qc.pos.saturating_sub(1);
                Some(false)
            }
            _ => None,
        };
    }

    // Normal sub-mode: letters are commands, not text -- an unrecognized
    // one is still consumed (a no-op) rather than leaking through as if
    // typed, matching how a real buffer's Normal mode swallows unmapped
    // keys instead of inserting them.
    match key {
        Key::Char('i') => qc.insert = true,
        Key::Char('a') => {
            qc.insert = true;
            qc.pos = (qc.pos + 1).min(chars.len());
        }
        Key::Char('I') => {
            qc.insert = true;
            qc.pos = 0;
        }
        Key::Char('A') => {
            qc.insert = true;
            qc.pos = chars.len();
        }
        Key::Char('h') | Key::Left | Key::Backspace => qc.pos = qc.pos.saturating_sub(1),
        Key::Char('l') | Key::Right => qc.pos = (qc.pos + 1).min(max_normal),
        Key::Char('0') | Key::Home => qc.pos = 0,
        Key::Char('$') | Key::End => qc.pos = max_normal,
        Key::Char('w') => qc.pos = word_forward(&chars, qc.pos).min(max_normal),
        Key::Char('b') => qc.pos = word_backward(&chars, qc.pos),
        Key::Char('e') => qc.pos = word_end(&chars, qc.pos),
        Key::Char('x') | Key::Delete => {
            if qc.pos < chars.len() {
                push_undo(qc, text);
                chars.remove(qc.pos);
                qc.pos = qc.pos.min(chars.len().saturating_sub(1));
                *text = chars.into_iter().collect();
                return Some(true);
            }
        }
        Key::Char('X') => {
            if qc.pos > 0 {
                push_undo(qc, text);
                qc.pos -= 1;
                chars.remove(qc.pos);
                *text = chars.into_iter().collect();
                return Some(true);
            }
        }
        Key::Char('D') => {
            if qc.pos < chars.len() {
                push_undo(qc, text);
                chars.truncate(qc.pos);
                qc.pos = qc.pos.min(chars.len().saturating_sub(1));
                *text = chars.into_iter().collect();
                return Some(true);
            }
        }
        Key::Char('C') => {
            push_undo(qc, text);
            chars.truncate(qc.pos);
            qc.insert = true;
            *text = chars.into_iter().collect();
            return Some(true);
        }
        Key::Char('p') => {
            let clip = crate::clipboard::paste().unwrap_or_default();
            let sanitized = sanitize_paste(&clip);
            return Some(paste_at(text, qc, &sanitized));
        }
        Key::Char('u') => {
            let Some((old_text, old_pos)) = qc.undo_stack.pop() else {
                return Some(false);
            };
            let changed = *text != old_text;
            *text = old_text;
            qc.pos = old_pos.min(text.chars().count().saturating_sub(1));
            return Some(changed);
        }
        // Any other letter is a no-op, swallowed the way a real buffer's
        // Normal mode swallows an unmapped key -- but a non-`Char` key
        // (Down/Up, Ctrl-anything, Enter, ...) is left for the caller,
        // so list navigation and the rest of the picker/results key
        // handling still work while this sub-mode is active.
        Key::Char(_) => {}
        _ => return None,
    }
    Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(text: &mut String, qc: &mut QueryCursor, key: Key) -> Option<bool> {
        handle(text, qc, key)
    }

    fn at(pos: usize, insert: bool) -> QueryCursor {
        QueryCursor {
            pos,
            insert,
            ..QueryCursor::default()
        }
    }

    #[test]
    fn typing_inserts_at_cursor_not_just_at_the_end() {
        let mut text = "helloworld".to_string();
        let mut qc = at(5, true);
        assert_eq!(feed(&mut text, &mut qc, Key::Char(' ')), Some(true));
        assert_eq!(text, "hello world");
        assert_eq!(qc.pos, 6);
    }

    #[test]
    fn esc_enters_normal_sub_mode_without_closing() {
        let mut text = "abc".to_string();
        let mut qc = QueryCursor::at_end(&text);
        assert_eq!(feed(&mut text, &mut qc, Key::Esc), Some(false));
        assert!(!qc.insert);
        // A second Esc is unhandled here -- the caller (picker/results)
        // is the one that closes the field.
        assert_eq!(feed(&mut text, &mut qc, Key::Esc), None);
    }

    #[test]
    fn word_motions_move_between_words() {
        let mut text = "foo bar baz".to_string();
        let mut qc = at(0, false);
        feed(&mut text, &mut qc, Key::Char('w'));
        assert_eq!(qc.pos, 4); // start of "bar"
        feed(&mut text, &mut qc, Key::Char('w'));
        assert_eq!(qc.pos, 8); // start of "baz"
        feed(&mut text, &mut qc, Key::Char('b'));
        assert_eq!(qc.pos, 4); // back to "bar"
        feed(&mut text, &mut qc, Key::Char('e'));
        assert_eq!(qc.pos, 6); // end of "bar"
    }

    #[test]
    fn x_deletes_the_char_under_the_cursor() {
        let mut text = "abcd".to_string();
        let mut qc = at(1, false);
        assert_eq!(feed(&mut text, &mut qc, Key::Char('x')), Some(true));
        assert_eq!(text, "acd");
        assert_eq!(qc.pos, 1);
    }

    #[test]
    fn i_a_reenter_insert_at_and_after_the_cursor() {
        let mut text = "ac".to_string();
        let mut qc = at(0, false);
        feed(&mut text, &mut qc, Key::Char('a'));
        assert!(qc.insert);
        assert_eq!(qc.pos, 1);
        feed(&mut text, &mut qc, Key::Char('b'));
        assert_eq!(text, "abc");
    }

    #[test]
    fn paste_at_inserts_mid_string_and_advances_cursor() {
        let mut text = "foobaz".to_string();
        let mut qc = at(3, true);
        assert!(paste_at(&mut text, &mut qc, "bar"));
        assert_eq!(text, "foobarbaz");
        assert_eq!(qc.pos, 6);
    }

    #[test]
    fn sanitize_paste_strips_newlines_and_carriage_returns() {
        assert_eq!(sanitize_paste("foo\r\nbar\n"), "foobar");
    }

    #[test]
    fn unrecognized_normal_key_is_consumed_not_typed() {
        let mut text = "abc".to_string();
        let mut qc = at(0, false);
        assert_eq!(feed(&mut text, &mut qc, Key::Char('z')), Some(false));
        assert_eq!(text, "abc");
    }

    #[test]
    fn ctrl_w_deletes_the_previous_word_in_insert_sub_mode() {
        let mut text = "foo bar".to_string();
        let mut qc = at(7, true);
        assert_eq!(feed(&mut text, &mut qc, Key::Ctrl('w')), Some(true));
        assert_eq!(text, "foo ");
        assert_eq!(qc.pos, 4);
    }

    #[test]
    fn ctrl_u_clears_to_the_start_in_insert_sub_mode() {
        let mut text = "foo bar".to_string();
        let mut qc = at(7, true);
        assert_eq!(feed(&mut text, &mut qc, Key::Ctrl('u')), Some(true));
        assert_eq!(text, "");
        assert_eq!(qc.pos, 0);
    }

    #[test]
    fn u_undoes_the_last_normal_mode_edit() {
        let mut text = "abcd".to_string();
        let mut qc = at(0, false);
        feed(&mut text, &mut qc, Key::Char('x'));
        assert_eq!(text, "bcd");
        assert_eq!(feed(&mut text, &mut qc, Key::Char('u')), Some(true));
        assert_eq!(text, "abcd");
        assert_eq!(qc.pos, 0);
    }

    #[test]
    fn u_undoes_typed_characters_one_at_a_time() {
        let mut text = String::new();
        let mut qc = at(0, true);
        feed(&mut text, &mut qc, Key::Char('h'));
        feed(&mut text, &mut qc, Key::Char('i'));
        assert_eq!(text, "hi");
        feed(&mut text, &mut qc, Key::Esc); // -> Normal sub-mode
        assert_eq!(feed(&mut text, &mut qc, Key::Char('u')), Some(true));
        assert_eq!(text, "h");
        assert_eq!(feed(&mut text, &mut qc, Key::Char('u')), Some(true));
        assert_eq!(text, "");
    }

    #[test]
    fn u_with_an_empty_undo_stack_is_a_no_op() {
        let mut text = "abc".to_string();
        let mut qc = at(0, false);
        assert_eq!(feed(&mut text, &mut qc, Key::Char('u')), Some(false));
        assert_eq!(text, "abc");
    }

    #[test]
    fn normal_mode_p_pastes_clipboard_at_cursor() {
        // clipboard::paste() is stubbed to None under `cfg(test)`, so this
        // only proves the no-op path never types the letter 'p' into the
        // field -- clipboard-backed paste itself is exercised through
        // `paste_at` directly above.
        let mut text = "ac".to_string();
        let mut qc = at(0, false);
        assert_eq!(feed(&mut text, &mut qc, Key::Char('p')), Some(false));
        assert_eq!(text, "ac");
    }
}
