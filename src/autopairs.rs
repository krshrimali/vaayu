//! Autopairs: insert matching close characters, skip over a closing
//! character already at the cursor, delete an empty pair as one Backspace,
//! and expand Enter inside a bracket pair to an indented blank line.
//!
//! Deliberately only hooked into typed `Key::Char` input in `insert.rs`,
//! never into `insert_char`/`insert_paste` directly, so pasted text (which
//! arrives as `Key::Literal` or through `Editor::insert_paste`) is never
//! auto-paired -- matching the plan's paste-suppression requirement without
//! extra bookkeeping.
use crate::editor::Editor;

struct Pair {
    open: char,
    close: char,
}

const PAIRS: &[Pair] = &[
    Pair {
        open: '(',
        close: ')',
    },
    Pair {
        open: '[',
        close: ']',
    },
    Pair {
        open: '{',
        close: '}',
    },
    Pair {
        open: '"',
        close: '"',
    },
    Pair {
        open: '\'',
        close: '\'',
    },
    Pair {
        open: '`',
        close: '`',
    },
];

fn pair_for_open(c: char) -> Option<&'static Pair> {
    PAIRS.iter().find(|p| p.open == c)
}

fn is_close_char(c: char) -> bool {
    PAIRS.iter().any(|p| p.close == c)
}

fn char_at(ed: &Editor, line: usize, col: usize) -> Option<char> {
    ed.buf().line_text(line).chars().nth(col)
}

/// Handles a typed character. Returns true if it fully handled the
/// keystroke (inserted a pair, or skipped over an existing close
/// character) and the caller should not insert `c` itself.
pub fn on_char(ed: &mut Editor, c: char) -> bool {
    if !ed.config.autopairs {
        return false;
    }
    let (line, col) = ed.cursor();
    let before = col.checked_sub(1).and_then(|p| char_at(ed, line, p));
    let after = char_at(ed, line, col);

    if is_close_char(c) && after == Some(c) {
        ed.set_cursor_insert(line, col + 1);
        return true;
    }

    if let Some(pair) = pair_for_open(c) {
        if pair.open == pair.close {
            // Symmetric (quote) pairs: don't pair mid-word, right after a
            // `\` escape inside a string, or for the `'` that starts a Rust
            // lifetime (`&'a T`), where a bare character is wanted.
            let word_before = before.is_some_and(|b| b.is_alphanumeric() || b == '_');
            let word_after = after.is_some_and(|a| a.is_alphanumeric() || a == '_');
            let escaped = before == Some('\\');
            let lifetime = c == '\'' && before == Some('&');
            if word_before || word_after || escaped || lifetime {
                return false;
            }
        }
        ed.buf_mut().insert_char(line, col, c);
        ed.buf_mut().insert_char(line, col + 1, pair.close);
        ed.set_cursor_insert(line, col + 1);
        return true;
    }
    false
}

/// Backspace between an empty pair (`(|)`) deletes both characters as one
/// edit instead of leaving the dangling close behind. Returns true if it
/// handled the Backspace.
pub fn on_backspace(ed: &mut Editor) -> bool {
    if !ed.config.autopairs {
        return false;
    }
    let (line, col) = ed.cursor();
    if col == 0 {
        return false;
    }
    let (Some(before), Some(after)) = (char_at(ed, line, col - 1), char_at(ed, line, col)) else {
        return false;
    };
    if !PAIRS.iter().any(|p| p.open == before && p.close == after) {
        return false;
    }
    let start = ed.buf().char_idx(line, col - 1);
    let end = ed.buf().char_idx(line, col + 1);
    ed.buf_mut().delete_char_range(start, end);
    ed.set_cursor_insert(line, col - 1);
    true
}

/// Enter between a bracket pair (`{|}`) expands to an indented blank line
/// framed by the pair. Quote pairs are excluded -- splitting a string
/// across lines this way is never what typing Enter inside one means.
/// Returns true if it handled the Enter.
pub fn on_enter(ed: &mut Editor) -> bool {
    if !ed.config.autopairs {
        return false;
    }
    let (line, col) = ed.cursor();
    let (Some(before), Some(after)) = (
        col.checked_sub(1).and_then(|p| char_at(ed, line, p)),
        char_at(ed, line, col),
    ) else {
        return false;
    };
    if !PAIRS
        .iter()
        .any(|p| p.open == before && p.close == after && p.open != p.close)
    {
        return false;
    }
    let indent: String = ed
        .buf()
        .line_text(line)
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let inner_indent = format!("{indent}{}", " ".repeat(ed.buf().shiftwidth.max(1)));
    let eol = if ed.buf().rope.to_string().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    ed.buf_mut()
        .insert_str(line, col, &format!("{eol}{inner_indent}{eol}{indent}"));
    ed.set_cursor_insert(line + 1, inner_indent.chars().count());
    true
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
                '\u{8}' => Key::Backspace,
                _ => Key::Char(c),
            });
        }
    }

    #[test]
    fn bracket_pairs_and_skips_over_close() {
        let mut e = editor("\n");
        keys(&mut e, "i(");
        assert_eq!(e.buf().rope.to_string(), "()\n");
        assert_eq!(e.cursor(), (0, 1));
        keys(&mut e, "x)");
        assert_eq!(e.buf().rope.to_string(), "(x)\n");
        assert_eq!(e.cursor(), (0, 3));
    }

    #[test]
    fn quote_pairs_on_empty_line() {
        let mut e = editor("\n");
        keys(&mut e, "i\"");
        assert_eq!(e.buf().rope.to_string(), "\"\"\n");
    }

    #[test]
    fn quote_not_paired_mid_word() {
        let mut e = editor("dont\n");
        // 0lll moves onto 't' (col 3); i inserts right before it, i.e. right
        // after 'n' -- a word-adjacent quote must not pair.
        keys(&mut e, "0llli'");
        assert_eq!(e.buf().rope.to_string(), "don't\n");
    }

    #[test]
    fn lifetime_apostrophe_is_not_paired() {
        let mut e = editor("&\n");
        keys(&mut e, "A'a");
        assert_eq!(e.buf().rope.to_string(), "&'a\n");
    }

    #[test]
    fn backspace_deletes_empty_pair_together() {
        let mut e = editor("\n");
        keys(&mut e, "i(");
        keys(&mut e, "\u{8}");
        assert_eq!(e.buf().rope.to_string(), "\n");
    }

    #[test]
    fn enter_expands_inside_braces_with_indent() {
        let mut e = editor("fn f() {}\n");
        keys(&mut e, "A"); // append at end of line puts cursor after '}'
        for _ in 0..1 {
            e.feed_key(Key::Left);
        }
        keys(&mut e, "\n");
        let text = e.buf().rope.to_string();
        assert_eq!(text, "fn f() {\n    \n}\n");
    }

    #[test]
    fn pasted_text_is_never_paired() {
        let mut e = editor("\n");
        keys(&mut e, "i");
        e.insert_paste("(x");
        assert_eq!(e.buf().rope.to_string(), "(x\n");
    }
}
