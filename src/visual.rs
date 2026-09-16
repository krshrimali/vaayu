use crate::editor::Editor;
use crate::key::Key;
use crate::mode::{CommandKind, Mode, VisualKind};
use crate::motion::{self, Span};
use crate::normal::{self, Awaiting};
use crate::operator::OperatorKind;
use crate::textobject;

pub fn handle(ed: &mut Editor, key: Key) {
    if let Some(awaiting) = ed.pending.awaiting.take() {
        if let Awaiting::TextObject { inner } = awaiting {
            handle_text_object(ed, inner, key);
        } else {
            normal::handle_awaiting(ed, awaiting, key);
        }
        return;
    }

    if key.as_char().map(|c| c.to_string()) == Some(ed.config.leader.clone()) {
        ed.pending.awaiting = Some(Awaiting::Leader {
            seq: String::new(),
            since: std::time::Instant::now(),
        });
        return;
    }
    if let Key::Char(c) = key {
        if c.is_ascii_digit() && !(c == '0' && ed.pending.count.is_none()) {
            let d = c.to_digit(10).unwrap() as usize;
            let n = ed
                .pending
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(d)
                .min(crate::normal::MAX_COUNT);
            ed.pending.count = Some(n);
            return;
        }
    }

    let kind = match ed.mode {
        Mode::Visual(k) => k,
        _ => return,
    };

    match key {
        Key::Char('g') => {
            ed.pending.awaiting = Some(Awaiting::GPrefix);
            return;
        }
        Key::Char('f') | Key::Char('F') | Key::Char('t') | Key::Char('T') => {
            ed.pending.awaiting = Some(Awaiting::FindChar {
                forward: matches!(key, Key::Char('f' | 't')),
                before: matches!(key, Key::Char('t' | 'T')),
            });
            return;
        }
        Key::Char('"') => {
            ed.pending.awaiting = Some(Awaiting::RegisterName);
            return;
        }
        Key::Ctrl('v') => {
            ed.mode = Mode::Visual(VisualKind::Block);
            return;
        }
        Key::Esc => {
            ed.visual_anchor = None;
            ed.pending.reset();
            ed.enter_normal();
            return;
        }
        Key::Char('v') => {
            if kind == VisualKind::Char {
                ed.visual_anchor = None;
                ed.enter_normal();
            } else {
                ed.mode = Mode::Visual(VisualKind::Char);
            }
            return;
        }
        Key::Char('V') => {
            if kind == VisualKind::Line {
                ed.visual_anchor = None;
                ed.enter_normal();
            } else {
                ed.mode = Mode::Visual(VisualKind::Line);
            }
            return;
        }
        Key::Char('o') => {
            if let Some(anchor) = ed.visual_anchor {
                let cur = ed.cursor();
                ed.visual_anchor = Some(cur);
                ed.set_cursor(anchor.0, anchor.1);
            }
            return;
        }
        Key::Char('i') => {
            ed.pending.awaiting = Some(Awaiting::TextObject { inner: true });
            return;
        }
        Key::Char('a') => {
            ed.pending.awaiting = Some(Awaiting::TextObject { inner: false });
            return;
        }
        Key::Char('d') | Key::Char('x') => {
            apply_to_selection(ed, OperatorKind::Delete, kind);
            return;
        }
        Key::Char('c') | Key::Char('s') => {
            apply_to_selection(ed, OperatorKind::Change, kind);
            return;
        }
        Key::Char('y') => {
            apply_to_selection(ed, OperatorKind::Yank, kind);
            return;
        }
        Key::Char('>') => {
            apply_to_selection(ed, OperatorKind::IndentRight, kind);
            return;
        }
        Key::Char('<') => {
            apply_to_selection(ed, OperatorKind::IndentLeft, kind);
            return;
        }
        Key::Char('~') => {
            toggle_case_selection(ed, kind);
            return;
        }
        Key::Char(':') => {
            ed.enter_command(CommandKind::Ex);
            ed.pending.reset();
            return;
        }
        _ => {}
    }

    if let Some(motion) = normal::key_to_motion(ed, key) {
        let (line, col) = ed.cursor();
        let count = ed.pending.total_count();
        if let Some((dl, dc, _)) = motion::resolve(ed.buf(), line, col, motion, count) {
            let dc = if matches!(motion, motion::Motion::Up | motion::Motion::Down) {
                crate::grapheme::raw_column(
                    &ed.buf().line_text(dl),
                    crate::grapheme::cell(&ed.buf().line_text(line), col, ed.config.tabstop),
                    ed.config.tabstop,
                )
            } else {
                dc
            };
            ed.set_cursor(dl, dc);
        }
        ed.pending.reset();
    } else {
        ed.pending.reset();
    }
}

type SelectionSpan = Option<((usize, usize), (usize, usize), Span)>;
fn selection_span(ed: &Editor, kind: VisualKind) -> SelectionSpan {
    let anchor = ed.visual_anchor?;
    let cursor = ed.cursor();
    match kind {
        VisualKind::Char | VisualKind::Block => Some((anchor, cursor, Span::Inclusive)),
        VisualKind::Line => Some((anchor, cursor, Span::Linewise)),
    }
}

fn apply_to_selection(ed: &mut Editor, op: OperatorKind, kind: VisualKind) {
    let Some((anchor, cursor, span)) = selection_span(ed, kind) else {
        ed.enter_normal();
        return;
    };
    if op != OperatorKind::Yank {
        ed.start_change_recording(Key::Char('v'));
        let (a, b) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };
        ed.visual_repeat = Some((
            kind,
            b.0 - a.0,
            if kind == VisualKind::Block {
                crate::grapheme::cell(&ed.buf().line_text(anchor.0), anchor.1, ed.config.tabstop)
                    .abs_diff(crate::grapheme::cell(
                        &ed.buf().line_text(cursor.0),
                        cursor.1,
                        ed.config.tabstop,
                    ))
                    + 1
            } else if a.0 == b.0 {
                b.1 - a.1 + 1
            } else {
                b.1 + 1
            },
            op,
        ));
    }
    if kind == VisualKind::Block {
        apply_block(ed, op, anchor, cursor);
    } else {
        normal::apply_operator_motion(ed, op, anchor, cursor, span);
    }
    ed.visual_anchor = None;
    ed.pending.reset();
    if !matches!(op, OperatorKind::Change) {
        ed.enter_normal();
    }
}

fn toggle_case_selection(ed: &mut Editor, kind: VisualKind) {
    apply_to_selection(ed, OperatorKind::ToggleCase, kind);
}

fn handle_text_object(ed: &mut Editor, inner: bool, key: Key) {
    if let Some(c) = key.as_char() {
        if let Some(kind) = normal::object_kind(c) {
            let (line, col) = ed.cursor();
            if let Some((sl, sc, el, ec)) = textobject::resolve(ed.buf(), line, col, kind, inner) {
                ed.visual_anchor = Some((sl, sc));
                ed.set_cursor(el, ec);
            }
        }
    }
    ed.pending.reset();
}

pub(crate) fn apply_block(
    ed: &mut Editor,
    op: OperatorKind,
    anchor: (usize, usize),
    cursor: (usize, usize),
) {
    let (first, last) = (anchor.0.min(cursor.0), anchor.0.max(cursor.0));
    let a = crate::grapheme::cell(&ed.buf().line_text(anchor.0), anchor.1, ed.config.tabstop);
    let c = crate::grapheme::cell(&ed.buf().line_text(cursor.0), cursor.1, ed.config.tabstop);
    let (left, right) = (a.min(c), a.max(c) + 1);
    apply_block_cells(ed, op, first, last, left, right);
}
pub(crate) fn apply_block_cells(
    ed: &mut Editor,
    op: OperatorKind,
    first: usize,
    last: usize,
    left: usize,
    right: usize,
) {
    if matches!(op, OperatorKind::IndentLeft | OperatorKind::IndentRight) {
        let sw = ed.config.shiftwidth;
        crate::operator::indent_lines(
            ed.buf_mut(),
            first,
            last,
            op == OperatorKind::IndentRight,
            sw,
        );
        ed.finish_change_recording();
        return;
    }
    let mut parts = Vec::new();
    if op != OperatorKind::Yank {
        ed.buf_mut().begin_edit();
    }
    for line in first..=last {
        let old = ed.buf().line_text(line);
        let expanded = crate::grapheme::expand_tabs(&old, ed.config.tabstop);
        if op != OperatorKind::Yank && expanded != old {
            let start = ed.buf().char_idx(line, 0);
            let end = ed.buf().char_idx(line, old.chars().count());
            ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut().insert_str_at(start, &expanded);
        }
        let lc = crate::grapheme::column(&expanded, left, false);
        let rc = crate::grapheme::column(&expanded, right, true);
        let start = ed.buf().char_idx(line, lc);
        let end = ed.buf().char_idx(line, rc);
        let text: String = expanded.chars().skip(lc).take(rc - lc).collect();
        let padded = format!(
            "{}{}",
            text,
            " ".repeat(
                (right - left).saturating_sub(unicode_width::UnicodeWidthStr::width(text.as_str()))
            )
        );
        parts.push(padded);
        if op != OperatorKind::Yank {
            ed.buf_mut().delete_char_range(start, end);
            if op == OperatorKind::ToggleCase {
                ed.buf_mut()
                    .insert_str_at(start, &crate::operator::toggle_case(&text));
            }
        }
    }
    if op != OperatorKind::ToggleCase {
        ed.registers
            .set_block(ed.pending.register, parts.join("\n"), right - left);
    }
    if op == OperatorKind::Change {
        let len = ed.buf().line_len(first);
        let width = unicode_width::UnicodeWidthStr::width(ed.buf().line_text(first).as_str());
        if width < left {
            ed.buf_mut()
                .insert_str(first, len, &" ".repeat(left - width));
        }
        ed.set_cursor_insert(
            first,
            crate::grapheme::column(&ed.buf().line_text(first), left, false),
        );
        ed.block_insert = Some((first, last, left));
        ed.enter_insert();
    } else {
        if op != OperatorKind::Yank {
            ed.buf_mut().commit_edit();
            ed.finish_change_recording();
        }
        ed.set_cursor(
            first,
            crate::grapheme::column(&ed.buf().line_text(first), left, false),
        );
    }
}
