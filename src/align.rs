//! Delimiter alignment (`mini.align`-style): pad every line's first
//! occurrence of a delimiter character so they all land in the same
//! column, as one undo transaction. Lines without the delimiter are left
//! untouched. Reached via `ga` (Visual: aligns the selection; Normal:
//! `gap<char>` aligns the paragraph around the cursor -- see `normal.rs`'s
//! `Awaiting::Align`/`AlignDelim`).
use crate::editor::Editor;

pub fn align(ed: &mut Editor, l1: usize, l2: usize, delim: char) {
    let (l1, l2) = (
        l1.min(l2),
        l1.max(l2).min(ed.buf().line_count().saturating_sub(1)),
    );
    let tab = ed.buf().tabstop;
    let mut max_col = 0usize;
    // Per line: (char index of the delimiter, its display column). Aligning by
    // display column (not char index) so tabs and wide chars — CJK, emoji —
    // line up on screen.
    let mut cols: Vec<Option<(usize, usize)>> = Vec::with_capacity(l2 - l1 + 1);
    for line in l1..=l2 {
        let text = ed.buf().line_text(line);
        let entry = text.chars().position(|c| c == delim).map(|ci| {
            let disp = crate::grapheme::cell(&text, ci, tab);
            max_col = max_col.max(disp);
            (ci, disp)
        });
        cols.push(entry);
    }
    if cols.iter().all(Option::is_none) {
        return;
    }
    ed.buf_mut().begin_edit();
    for (i, line) in (l1..=l2).enumerate() {
        if let Some((ci, disp)) = cols[i] {
            let pad = max_col - disp;
            if pad > 0 {
                ed.buf_mut().insert_str(line, ci, &" ".repeat(pad));
            }
        }
    }
    ed.buf_mut().commit_edit();
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
    fn aligns_by_first_delimiter() {
        let mut e = editor("a = 1\nbb = 2\nccc = 3\n");
        super::align(&mut e, 0, 2, '=');
        assert_eq!(e.buf().rope.to_string(), "a   = 1\nbb  = 2\nccc = 3\n");
    }

    #[test]
    fn aligns_by_display_column_not_char_index() {
        // `中` is two display cells wide, so aligning by char index would leave
        // the `=` signs visually ragged; aligning by display column fixes it.
        let mut e = editor("x = 1\n中 = 2\n");
        super::align(&mut e, 0, 1, '=');
        assert_eq!(e.buf().rope.to_string(), "x  = 1\n中 = 2\n");
    }

    #[test]
    fn lines_without_the_delimiter_are_untouched() {
        let mut e = editor("a = 1\nno delimiter here\nccc = 3\n");
        super::align(&mut e, 0, 2, '=');
        let text = e.buf().rope.to_string();
        assert_eq!(text, "a   = 1\nno delimiter here\nccc = 3\n");
    }

    #[test]
    fn no_delimiter_anywhere_is_a_no_op() {
        let mut e = editor("a\nb\nc\n");
        super::align(&mut e, 0, 2, '=');
        assert_eq!(e.buf().rope.to_string(), "a\nb\nc\n");
    }

    #[test]
    fn one_undo_transaction() {
        let mut e = editor("a = 1\nbb = 2\n");
        super::align(&mut e, 0, 1, '=');
        assert_eq!(e.buf().rope.to_string(), "a  = 1\nbb = 2\n");
        keys(&mut e, "u");
        assert_eq!(e.buf().rope.to_string(), "a = 1\nbb = 2\n");
    }

    #[test]
    fn visual_ga_aligns_selection() {
        let mut e = editor("a = 1\nbb = 2\nccc = 3\n");
        keys(&mut e, "VGga=");
        assert_eq!(e.buf().rope.to_string(), "a   = 1\nbb  = 2\nccc = 3\n");
    }

    #[test]
    fn normal_gap_aligns_paragraph() {
        let mut e = editor("a = 1\nbb = 2\n\nunrelated = x\n");
        keys(&mut e, "gap=");
        assert_eq!(
            e.buf().rope.to_string(),
            "a  = 1\nbb = 2\n\nunrelated = x\n"
        );
    }
}
