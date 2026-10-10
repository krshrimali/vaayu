use crate::editor::Editor;
use crate::key::Key;
use crate::mode::{CommandKind, Mode, VisualKind};
use crate::motion::{self, Span};
use crate::normal::{self, Awaiting};
use crate::operator::OperatorKind;
use crate::textobject;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy)]
pub struct BlockInsert {
    pub first: usize,
    pub last: usize,
    pub column: usize,
    pub pad_short: bool,
    pub at_eol: bool,
    pub short_eol: bool,
    /// Original character column of the selection's left edge on the first
    /// row. I/A return here; c stays on the inserted text.
    pub return_column: Option<usize>,
    pub start: usize,
    pub prepared_seq: u64,
}

#[derive(Clone, Copy)]
pub struct RepeatSelection {
    pub kind: VisualKind,
    pub height: usize,
    pub width: usize,
    pub op: OperatorKind,
    pub to_eol: bool,
    pub includes_newline: bool,
}

/// Place an insertion on a display-cell boundary, splitting only a tab that
/// contains that boundary. Wide glyphs remain whole, with padding as needed.
pub(crate) fn prepare_block_insert(
    ed: &mut Editor,
    line: usize,
    col: usize,
    pad: bool,
) -> Option<usize> {
    let old = ed.buf().line_text(line);
    let width = crate::grapheme::cell(&old, old.chars().count(), ed.buf().tabstop);
    if width < col && !pad {
        return None;
    }
    let (mut before, _, after) = crate::grapheme::split_cells(&old, col, col, ed.buf().tabstop);
    if width < col {
        before.push_str(&" ".repeat(col - width));
    }
    let at = before.chars().count();
    let replacement = format!("{before}{after}");
    if replacement != old {
        let start = ed.buf().char_idx(line, 0);
        let end = start + old.chars().count();
        ed.buf_mut().delete_char_range(start, end);
        ed.buf_mut().insert_str_at(start, &replacement);
    }
    Some(at)
}

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

    // Record the live selection on every visual-mode keystroke so `gv` can
    // reselect it: the operator/Esc key that ends visual mode captures the
    // final span just before it is consumed.
    if let Some(anchor) = ed.visual_anchor {
        ed.last_visual = Some((kind, anchor, ed.cursor()));
    }

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
        Key::Ctrl('n') => {
            crate::multicursor::from_visual(ed, kind);
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
        Key::Char('p') | Key::Char('P') => {
            paste_over_selection(ed, kind);
            return;
        }
        Key::Char('I') if kind == VisualKind::Block => {
            block_insert_edge(ed, false);
            return;
        }
        Key::Char('A') if kind == VisualKind::Block => {
            block_insert_edge(ed, true);
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
        Key::Char('S') if kind != VisualKind::Block => {
            if let Some((anchor, cursor, span)) = selection_span(ed, kind) {
                let (start, end, _) = normal::span_to_range(ed, anchor, cursor, span);
                ed.visual_anchor = None;
                ed.enter_normal();
                ed.pending.awaiting = Some(Awaiting::Surround(crate::surround::Stage::AddDelim {
                    start,
                    end,
                }));
            } else {
                ed.enter_normal();
            }
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
        let vertical = matches!(motion, motion::Motion::Up | motion::Motion::Down);
        let desired = ed.buf().desired_col;
        if let Some((dl, dc, _)) = motion::resolve(ed.buf(), line, col, motion, count) {
            let dc = if vertical && desired == usize::MAX {
                if kind == VisualKind::Block {
                    ed.buf().line_len(dl)
                } else {
                    ed.buf().line_len(dl).saturating_sub(1)
                }
            } else if vertical {
                crate::grapheme::raw_column(&ed.buf().line_text(dl), desired, ed.buf().tabstop)
            } else {
                dc
            };
            ed.set_cursor(dl, dc);
            if vertical
                || (desired == usize::MAX
                    && matches!(motion, motion::Motion::Right)
                    && ((kind == VisualKind::Block && dc >= ed.buf().line_len(dl))
                        || (kind == VisualKind::Char && (dl, dc) == (line, col))))
            {
                ed.buf_mut().desired_col = desired;
            } else if matches!(motion, motion::Motion::LineEnd) {
                ed.buf_mut().desired_col = usize::MAX;
            }
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

pub(crate) fn apply_to_selection(ed: &mut Editor, op: OperatorKind, kind: VisualKind) {
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
        let cells = |(line, col)| {
            let text = ed.buf().line_text(line);
            let start = crate::grapheme::cell(&text, col, ed.buf().tabstop);
            let end = crate::grapheme::cell(
                &text,
                crate::grapheme::step(&text, col, 1, true),
                ed.buf().tabstop,
            )
            .max(start + 1);
            (start, end)
        };
        let (a_start, a_end) = cells(a);
        let (b_start, b_end) = cells(b);
        ed.visual_repeat = Some(RepeatSelection {
            kind,
            height: b.0 - a.0,
            width: if kind == VisualKind::Block {
                a_end.max(b_end) - a_start.min(b_start)
            } else if a.0 == b.0 {
                b_end - a_start
            } else {
                b_end
            },
            op,
            to_eol: kind == VisualKind::Char && ed.buf().desired_col == usize::MAX,
            includes_newline: b.1 >= ed.buf().line_len(b.0),
        });
    }
    if kind == VisualKind::Block {
        apply_block(ed, op, anchor, cursor);
    } else {
        normal::apply_operator_motion(ed, op, anchor, cursor, span);
    }
    ed.pending.reset();
    // Retain the selection after Visual >/< the way a `gv`-after-indent
    // remap does, so repeated presses keep indenting the same block --
    // only for Char/Line kinds; Block indent leaves the selection as-is.
    if matches!(op, OperatorKind::IndentLeft | OperatorKind::IndentRight)
        && kind != VisualKind::Block
    {
        let l2 = anchor.0.max(cursor.0);
        ed.visual_anchor = Some((l2, ed.buf().first_non_blank(l2)));
    } else {
        ed.visual_anchor = None;
        if !matches!(op, OperatorKind::Change) {
            ed.enter_normal();
        }
    }
}

fn toggle_case_selection(ed: &mut Editor, kind: VisualKind) {
    apply_to_selection(ed, OperatorKind::ToggleCase, kind);
}

/// Visual-block `I`/`A`: enter Insert at the block's left (`I`) or right (`A`)
/// edge on the first line; on leaving Insert, `leave_insert`'s `block_insert`
/// replication repeats the typed text down every line of the block.
fn block_insert_edge(ed: &mut Editor, append: bool) {
    let Some(anchor) = ed.visual_anchor else {
        ed.enter_normal();
        return;
    };
    let cursor = ed.cursor();
    let (first, last) = (anchor.0.min(cursor.0), anchor.0.max(cursor.0));
    let a = crate::grapheme::cell(&ed.buf().line_text(anchor.0), anchor.1, ed.buf().tabstop);
    let c = crate::grapheme::cell(&ed.buf().line_text(cursor.0), cursor.1, ed.buf().tabstop);
    let end_cell = |(line, col)| {
        let text = ed.buf().line_text(line);
        crate::grapheme::cell(
            &text,
            crate::grapheme::step(&text, col, 1, true),
            ed.buf().tabstop,
        )
        .max(crate::grapheme::cell(&text, col, ed.buf().tabstop) + 1)
    };
    let at_eol = ed.buf().desired_col == usize::MAX;
    let col = if append && at_eol {
        crate::grapheme::cell(
            &ed.buf().line_text(first),
            ed.buf().line_len(first),
            ed.buf().tabstop,
        )
    } else if append {
        end_cell(anchor).max(end_cell(cursor))
    } else {
        a.min(c)
    };
    ed.start_change_recording(Key::Char(if append { 'A' } else { 'I' }));
    ed.buf_mut().begin_edit();
    let text = ed.buf().line_text(first);
    let floor = crate::grapheme::raw_column(&text, col, ed.buf().tabstop);
    let at = if !append {
        floor
    } else if text.chars().nth(floor) == Some('\t')
        && crate::grapheme::cell(&text, floor, ed.buf().tabstop) < col
    {
        floor + 1
    } else {
        prepare_block_insert(ed, first, col, true).unwrap_or(ed.buf().line_len(first))
    };
    ed.set_cursor_insert(first, at);
    ed.block_insert = Some(BlockInsert {
        first,
        last,
        column: col,
        pad_short: append,
        at_eol: append && at_eol,
        short_eol: at_eol,
        return_column: Some(crate::grapheme::raw_column(
            &text,
            a.min(c),
            ed.buf().tabstop,
        )),
        start: ed.buf().char_idx(
            first,
            if append && at > floor && text.chars().nth(floor) == Some('\t') {
                floor
            } else {
                at
            },
        ),
        prepared_seq: ed.buf().edit_seq,
    });
    ed.visual_anchor = None;
    ed.pending.reset();
    ed.enter_insert();
}

/// Visual-mode `p`/`P`: replace the selection with the register's contents (the
/// replaced text goes to the unnamed register, as in Vim). Char/Line kinds are
/// supported; Block just exits Visual mode.
fn paste_over_selection(ed: &mut Editor, kind: VisualKind) {
    if kind == VisualKind::Block {
        ed.visual_anchor = None;
        ed.pending.reset();
        ed.enter_normal();
        return;
    }
    // Capture the register to paste *before* the delete overwrites the unnamed
    // register (the common `p` from unnamed would otherwise paste what it just
    // deleted).
    let reg = ed.pending.register;
    let Some(entry) = ed.registers.get(reg).cloned() else {
        ed.visual_anchor = None;
        ed.pending.reset();
        ed.enter_normal();
        return;
    };
    let Some((anchor, cursor, span)) = selection_span(ed, kind) else {
        ed.enter_normal();
        return;
    };
    let (start, end, linewise_sel) = normal::span_to_range(ed, anchor, cursor, span);
    ed.start_change_recording(Key::Char('p'));
    ed.buf_mut().begin_edit();
    let deleted = ed.buf_mut().delete_char_range(start, end);
    let mut text = entry.text;
    if entry.linewise && !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    ed.buf_mut().insert_str_at(start, &text);
    ed.buf_mut().commit_edit();
    // The replaced text becomes the unnamed/numbered delete, like `d`.
    ed.registers.delete(None, deleted, linewise_sel);
    let (l, c) = ed.buf().pos_from_char_idx(start);
    ed.set_cursor(l, c);
    ed.visual_anchor = None;
    ed.pending.reset();
    ed.enter_normal();
    ed.finish_change_recording();
}

fn handle_text_object(ed: &mut Editor, inner: bool, key: Key) {
    if let Some(c) = key.as_char() {
        // Tree-sitter objects: function (`f`) / class (`c`).
        if matches!(c, 'f' | 'c') {
            if let Some((sl, sc, el, ec)) = ed.tree_object_range(c, inner) {
                if (sl, sc) <= (el, ec) {
                    ed.visual_anchor = Some((sl, sc));
                    ed.set_cursor(el, ec);
                }
            }
            ed.pending.reset();
            return;
        }
        if let Some(kind) = normal::object_kind(c) {
            let (line, col) = ed.cursor();
            if let Some((sl, sc, el, ec)) = textobject::resolve(ed.buf(), line, col, kind, inner) {
                // An empty inner object (e.g. `vi(` on `()`) resolves to a
                // reversed range; don't set an inverted visual selection.
                if (sl, sc) <= (el, ec) {
                    // A paragraph object is linewise: select whole lines.
                    if matches!(kind, textobject::ObjectKind::Paragraph) {
                        ed.mode = Mode::Visual(VisualKind::Line);
                    }
                    ed.visual_anchor = Some((sl, sc));
                    ed.set_cursor(el, ec);
                }
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
    let a = crate::grapheme::cell(&ed.buf().line_text(anchor.0), anchor.1, ed.buf().tabstop);
    let c = crate::grapheme::cell(&ed.buf().line_text(cursor.0), cursor.1, ed.buf().tabstop);
    let a_end = crate::grapheme::cell(
        &ed.buf().line_text(anchor.0),
        crate::grapheme::step(&ed.buf().line_text(anchor.0), anchor.1, 1, true),
        ed.buf().tabstop,
    )
    .max(a + 1);
    let c_end = crate::grapheme::cell(
        &ed.buf().line_text(cursor.0),
        crate::grapheme::step(&ed.buf().line_text(cursor.0), cursor.1, 1, true),
        ed.buf().tabstop,
    )
    .max(c + 1);
    let (left, right) = (a.min(c), a_end.max(c_end));
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
    let insert_col =
        crate::grapheme::raw_column(&ed.buf().line_text(first), left, ed.buf().tabstop);
    if matches!(op, OperatorKind::IndentLeft | OperatorKind::IndentRight) {
        let sw = ed.buf().shiftwidth;
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
        let (before, text, after) =
            crate::grapheme::split_cells(&old, left, right, ed.buf().tabstop);
        parts.push(text.clone());
        if op != OperatorKind::Yank {
            let replacement = if op == OperatorKind::ToggleCase {
                // Case changes do not remove cells: preserve whole tabs and
                // glyphs even when only part of their display width overlaps.
                let mut cell = 0;
                old.graphemes(true)
                    .map(|g| {
                        let start = cell;
                        cell += if g == "\t" {
                            ed.buf().tabstop.max(1) - cell % ed.buf().tabstop.max(1)
                        } else {
                            unicode_width::UnicodeWidthStr::width(g)
                        };
                        if start < right && cell > left {
                            crate::operator::toggle_case(g)
                        } else {
                            g.to_owned()
                        }
                    })
                    .collect::<String>()
            } else {
                format!("{before}{after}")
            };
            if replacement != old {
                let start = ed.buf().char_idx(line, 0);
                let end = ed.buf().char_idx(line, old.chars().count());
                ed.buf_mut().delete_char_range(start, end);
                ed.buf_mut().insert_str_at(start, &replacement);
            }
        }
    }
    if op != OperatorKind::ToggleCase {
        ed.registers
            .set_block(ed.pending.register, parts.join("\n"), right - left);
    }
    if op == OperatorKind::Change {
        let len = ed.buf().line_len(first);
        let width = crate::grapheme::cell(&ed.buf().line_text(first), len, ed.buf().tabstop);
        if width < left {
            ed.buf_mut()
                .insert_str(first, len, &" ".repeat(left - width));
        }
        ed.set_cursor_insert(first, insert_col);
        ed.block_insert = Some(BlockInsert {
            first,
            last,
            column: left,
            pad_short: true,
            at_eol: false,
            short_eol: false,
            return_column: None,
            start: ed.buf().char_idx(first, ed.cursor().1),
            prepared_seq: ed.buf().edit_seq,
        });
        ed.enter_insert();
    } else {
        if op != OperatorKind::Yank {
            ed.buf_mut().commit_edit();
            ed.finish_change_recording();
        }
        ed.set_cursor(
            first,
            crate::grapheme::raw_column(&ed.buf().line_text(first), left, ed.buf().tabstop),
        );
    }
}
