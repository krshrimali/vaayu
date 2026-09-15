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
        }
        return;
    }

    if let Key::Char(c) = key {
        if c.is_ascii_digit() && !(c == '0' && ed.pending.count.is_none()) {
            let d = c.to_digit(10).unwrap() as usize;
            let n = ed.pending.count.unwrap_or(0).saturating_mul(10).saturating_add(d).min(crate::normal::MAX_COUNT);
            ed.pending.count = Some(n);
            return;
        }
    }

    let kind = match ed.mode {
        Mode::Visual(k) => k,
        _ => return,
    };

    match key {
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
            ed.set_cursor(dl, dc);
        }
        ed.pending.reset();
    } else {
        ed.pending.reset();
    }
}

fn selection_span(ed: &Editor, kind: VisualKind) -> Option<((usize, usize), (usize, usize), Span)> {
    let anchor = ed.visual_anchor?;
    let cursor = ed.cursor();
    match kind {
        VisualKind::Char => Some((anchor, cursor, Span::Inclusive)),
        VisualKind::Line => Some((anchor, cursor, Span::Linewise)),
    }
}

fn apply_to_selection(ed: &mut Editor, op: OperatorKind, kind: VisualKind) {
    let Some((anchor, cursor, span)) = selection_span(ed, kind) else {
        ed.enter_normal();
        return;
    };
    ed.start_change_recording(Key::Char('v'));
    normal::apply_operator_motion(ed, op, anchor, cursor, span);
    ed.visual_anchor = None;
    ed.pending.reset();
    if !matches!(op, OperatorKind::Change) {
        ed.enter_normal();
    }
}

fn toggle_case_selection(ed: &mut Editor, kind: VisualKind) {
    let Some((anchor, cursor, span)) = selection_span(ed, kind) else {
        ed.enter_normal();
        return;
    };
    ed.start_change_recording(Key::Char('~'));
    let buf = ed.buf();
    let a = buf.char_idx(anchor.0, anchor.1);
    let c = buf.char_idx(cursor.0, cursor.1);
    let (start, end) = match span {
        Span::Linewise => {
            let l1 = anchor.0.min(cursor.0);
            let l2 = anchor.0.max(cursor.0);
            let s = buf.char_idx(l1, 0);
            let e = if l2 + 1 < buf.line_count() { buf.char_idx(l2 + 1, 0) } else { buf.rope.len_chars() };
            (s, e)
        }
        _ => (a.min(c), a.max(c) + 1),
    };
    ed.buf_mut().begin_edit();
    let text = ed.buf_mut().delete_char_range(start, end);
    let toggled: String = text
        .chars()
        .map(|ch| {
            if ch.is_uppercase() {
                ch.to_lowercase().next().unwrap()
            } else if ch.is_lowercase() {
                ch.to_uppercase().next().unwrap()
            } else {
                ch
            }
        })
        .collect();
    ed.buf_mut().insert_str_at(start, &toggled);
    ed.buf_mut().commit_edit();
    let (l, c) = ed.buf().pos_from_char_idx(start);
    ed.set_cursor(l, c);
    ed.visual_anchor = None;
    ed.pending.reset();
    ed.finish_change_recording();
    ed.enter_normal();
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
