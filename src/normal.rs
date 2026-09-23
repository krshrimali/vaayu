use crate::editor::Editor;
use crate::key::Key;
use crate::mode::{CommandKind, VisualKind};
use crate::motion::{self, Motion, Span};
use crate::operator::{self, OperatorKind};
use crate::textobject::{self, ObjectKind};
use std::time::Instant;

/// Ceiling for any accumulated Vim count (motion/operator repeat, paste
/// count, macro replay count). A long digit prefix could otherwise overflow
/// the accumulation arithmetic, or -- even saturated -- drive a
/// motion/paste/replay loop through an absurd number of iterations and hang
/// the editor; 100,000 is far past any legitimate use but keeps worst-case
/// loop counts bounded to a fraction of a second.
pub const MAX_COUNT: usize = 100_000;

#[derive(Debug, Clone)]
pub enum Awaiting {
    /// `[`/`]` prefix: `d` for diagnostics, `c` for the next/prev changed
    /// git hunk -- both are "jump to the next/prev X" under the same
    /// bracket prefix, matching Vim/plugin convention (`]d`, `]c`).
    Diagnostic(bool),
    GPrefix,
    FindChar {
        forward: bool,
        before: bool,
    },
    Replace,
    TextObject {
        inner: bool,
    },
    RegisterName,
    MarkSet,
    MarkJump {
        exact: bool,
    },
    MacroRegister,
    MacroReplay,
    Leader {
        seq: String,
        since: Instant,
    },
    ZPrefix,
    Surround(crate::surround::Stage),
    /// `ga` -- awaiting either the delimiter (Visual: aligns the
    /// selection) or `p` (Normal: align the surrounding paragraph, then
    /// await the delimiter via `AlignDelim`).
    Align,
    AlignDelim {
        start: usize,
        end: usize,
    },
}

#[derive(Default, Clone)]
pub struct PendingState {
    pub count: Option<usize>,
    pub register: Option<char>,
    pub operator: Option<OperatorKind>,
    pub op_count: Option<usize>,
    pub awaiting: Option<Awaiting>,
}

impl PendingState {
    pub fn is_empty(&self) -> bool {
        self.count.is_none()
            && self.register.is_none()
            && self.operator.is_none()
            && self.awaiting.is_none()
    }

    pub fn total_count(&self) -> usize {
        self.op_count
            .unwrap_or(1)
            .saturating_mul(self.count.unwrap_or(1))
            .min(MAX_COUNT)
    }

    pub fn reset(&mut self) {
        *self = PendingState::default();
    }
}

pub fn handle(ed: &mut Editor, key: Key) {
    // Focused on a terminal pane: there is no visible buffer here to run
    // Vim motions/operators against (the window's `buffer` field is just
    // whatever was active before the pane was opened, kept only so the
    // rest of the window-handling code has a valid id to ignore). Only
    // re-entering the terminal and `:` commands (close/quit/pane nav via
    // Ctrl-W, handled earlier in `Editor::feed_key`) make sense here.
    // The file tree sidebar has its own small, self-contained key handling
    // (see `filetree::handle_key`): its cursor indexes a node list, not a
    // buffer's lines, so generic motion/operator dispatch must never reach
    // it -- there is no `Awaiting`-based fallthrough to get wrong here.
    if ed.active_file_tree() {
        crate::filetree::handle_key(ed, key);
        return;
    }
    if ed.active_outline() {
        crate::outline::handle_key(ed, key);
        return;
    }

    // Once a multi-key sequence is already in flight (e.g. `g` of `gt` was
    // just pressed), every subsequent key must reach `handle_awaiting`
    // regardless of the terminal guard below -- swallowing it here would
    // leave `pending.awaiting` permanently stuck instead of either
    // completing the sequence or being reset by its own catch-all arm.
    if ed.active_terminal_id().is_some() && ed.pending.awaiting.is_none() {
        match key {
            Key::Char('i') | Key::Char('a') => {
                ed.mode = crate::mode::Mode::Terminal;
                return;
            }
            Key::Char(':') => {
                ed.enter_command(CommandKind::Ex);
                return;
            }
            // Falls through to the normal dispatch below: tab navigation
            // (gt/gT/{n}gt) is safe on a terminal pane (it never touches
            // the window's placeholder buffer), so digits and 'g' aren't
            // swallowed the way every other key here is.
            Key::Char(c) if c == 'g' || c.is_ascii_digit() => {}
            _ => return,
        }
    }
    if let Some(awaiting) = ed.pending.awaiting.take() {
        handle_awaiting(ed, awaiting, key);
        return;
    }

    // Count accumulation (leading '0' is a motion, not the start of a count).
    if let Key::Char(c) = key {
        if c.is_ascii_digit() && !(c == '0' && ed.pending.count.is_none()) {
            let d = c.to_digit(10).unwrap() as usize;
            let n = ed
                .pending
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(d)
                .min(MAX_COUNT);
            ed.pending.count = Some(n);
            return;
        }
    }

    // Leader key (comma, by default -- matches this config's `mapleader`).
    if key.as_char().map(|c| c.to_string()) == Some(ed.config.leader.clone())
        && ed.pending.operator.is_none()
    {
        ed.pending.awaiting = Some(Awaiting::Leader {
            seq: String::new(),
            since: Instant::now(),
        });
        return;
    }

    if key == Key::Char('"') && ed.pending.operator.is_none() {
        ed.pending.awaiting = Some(Awaiting::RegisterName);
        return;
    }

    // Operator already pending: only i/a (text object), Esc (cancel), same-char doubling
    // (linewise), or a motion are valid continuations.
    if let Some(op) = ed.pending.operator {
        // `s` is never a valid motion/doubling continuation for d/c/y, so
        // repurpose it the way vim-surround does: `ds`/`cs`/`ys` begin a
        // surround delete/change/add instead of aborting the operator.
        if key == Key::Char('s')
            && matches!(
                op,
                OperatorKind::Delete | OperatorKind::Change | OperatorKind::Yank
            )
        {
            if matches!(op, OperatorKind::Delete | OperatorKind::Change) {
                ed.abort_change_recording();
            }
            ed.pending.operator = None;
            ed.pending.awaiting = Some(Awaiting::Surround(match op {
                OperatorKind::Delete => crate::surround::Stage::Delete,
                OperatorKind::Change => crate::surround::Stage::ChangeFrom,
                _ => crate::surround::Stage::AddOperand,
            }));
            return;
        }
        match key {
            Key::Char('i') => {
                ed.pending.awaiting = Some(Awaiting::TextObject { inner: true });
                return;
            }
            Key::Char('a') => {
                ed.pending.awaiting = Some(Awaiting::TextObject { inner: false });
                return;
            }
            Key::Esc => {
                ed.pending.reset();
                ed.abort_change_recording();
                return;
            }
            _ => {}
        }
        if doubled(op, key) {
            apply_linewise_current(ed);
            return;
        }
        if let Some(motion) = key_to_motion(ed, key) {
            apply_motion_or_operator(ed, motion);
            return;
        }
        match key {
            Key::Char('f') => {
                ed.pending.awaiting = Some(Awaiting::FindChar {
                    forward: true,
                    before: false,
                });
                return;
            }
            Key::Char('F') => {
                ed.pending.awaiting = Some(Awaiting::FindChar {
                    forward: false,
                    before: false,
                });
                return;
            }
            Key::Char('t') => {
                ed.pending.awaiting = Some(Awaiting::FindChar {
                    forward: true,
                    before: true,
                });
                return;
            }
            Key::Char('T') => {
                ed.pending.awaiting = Some(Awaiting::FindChar {
                    forward: false,
                    before: true,
                });
                return;
            }
            Key::Char('g') => {
                ed.pending.awaiting = Some(Awaiting::GPrefix);
                return;
            }
            _ => {
                // Not a valid operator continuation.
                ed.pending.reset();
                ed.abort_change_recording();
                return;
            }
        }
    }

    // No operator pending: try a bare motion first.
    if let Some(motion) = key_to_motion(ed, key) {
        apply_motion_or_operator(ed, motion);
        return;
    }

    match key {
        Key::Char('[') => ed.pending.awaiting = Some(Awaiting::Diagnostic(false)),
        Key::Char(']') => ed.pending.awaiting = Some(Awaiting::Diagnostic(true)),
        Key::Ctrl('o') => ed.jump_history(false),
        Key::Tab | Key::Ctrl('i') => ed.jump_history(true),
        Key::Ctrl('6') => ed.switch_to_alternate(),
        Key::Char('d') => begin_operator(ed, OperatorKind::Delete),
        Key::Char('c') => begin_operator(ed, OperatorKind::Change),
        Key::Char('y') => begin_operator(ed, OperatorKind::Yank),
        Key::Char('>') => begin_operator(ed, OperatorKind::IndentRight),
        Key::Char('<') => begin_operator(ed, OperatorKind::IndentLeft),
        Key::Char('g') => ed.pending.awaiting = Some(Awaiting::GPrefix),
        Key::Char('f') => {
            ed.pending.awaiting = Some(Awaiting::FindChar {
                forward: true,
                before: false,
            })
        }
        Key::Char('F') => {
            ed.pending.awaiting = Some(Awaiting::FindChar {
                forward: false,
                before: false,
            })
        }
        Key::Char('t') => {
            ed.pending.awaiting = Some(Awaiting::FindChar {
                forward: true,
                before: true,
            })
        }
        Key::Char('T') => {
            ed.pending.awaiting = Some(Awaiting::FindChar {
                forward: false,
                before: true,
            })
        }
        Key::Char(';') => repeat_find(ed, false),
        Key::Char('r') => {
            ed.start_change_recording(key);
            ed.pending.awaiting = Some(Awaiting::Replace);
        }
        Key::Char('x') => {
            ed.start_change_recording(key);
            let (line, col) = ed.cursor();
            let end = crate::grapheme::step(
                &ed.buf().line_text(line),
                col,
                ed.pending.total_count(),
                true,
            );
            let start = ed.buf().char_idx(line, col);
            let endi = ed.buf().char_idx(line, end);
            let reg = ed.pending.register;
            let (buf, regs) = ed.buf_and_registers_mut();
            operator::delete_range(buf, regs, reg, start, endi, false);
            let nc = ed.buf().clamp_col_normal(line, col);
            ed.set_cursor(line, nc);
            ed.pending.reset();
            ed.finish_change_recording();
        }
        Key::Char('X') => {
            ed.start_change_recording(key);
            let (line, col) = ed.cursor();
            let target = crate::grapheme::step(
                &ed.buf().line_text(line),
                col,
                ed.pending.total_count(),
                false,
            );
            let n = col - target;
            let start = ed.buf().char_idx(line, col - n);
            let end = ed.buf().char_idx(line, col);
            let reg = ed.pending.register;
            let (buf, regs) = ed.buf_and_registers_mut();
            operator::delete_range(buf, regs, reg, start, end, false);
            ed.set_cursor(line, col - n);
            ed.pending.reset();
            ed.finish_change_recording();
        }
        Key::Char('D') => op_to_line_end(ed, OperatorKind::Delete, key),
        Key::Char('C') => op_to_line_end(ed, OperatorKind::Change, key),
        Key::Char('Y') => {
            let line = ed.cursor().0;
            let n = ed.pending.total_count();
            let l2 = (line + n - 1).min(ed.buf().line_count().saturating_sub(1));
            let start = ed.buf().char_idx(line, 0);
            let end = if l2 + 1 < ed.buf().line_count() {
                ed.buf().char_idx(l2 + 1, 0)
            } else {
                ed.buf().rope.len_chars()
            };
            let reg = ed.pending.register;
            let (buf, regs) = ed.buf_and_registers_mut();
            operator::yank_range(buf, regs, reg, start, end, true);
            ed.pending.reset();
        }
        Key::Char('p') => do_paste(ed, key, true),
        Key::Char('P') => do_paste(ed, key, false),
        Key::Char('u') => {
            ed.buf_mut().undo();
            ed.set_message("undo");
            ed.pending.reset();
        }
        Key::Ctrl('r') => {
            ed.buf_mut().redo();
            ed.set_message("redo");
            ed.pending.reset();
        }
        Key::Ctrl('p') => {
            ed.open_picker();
            ed.pending.reset();
        }
        Key::Char('J') => {
            ed.start_change_recording(key);
            let line = ed.cursor().0;
            let n = ed.pending.total_count().max(2);
            let last = (line + n - 1).min(ed.buf().line_count().saturating_sub(1));
            ed.buf_mut().begin_edit();
            let mut join_col = ed.buf().line_len(line);
            for _ in line..last {
                if line + 1 >= ed.buf().line_count() {
                    break;
                }
                let cur_text = ed.buf().line_text(line);
                let end_of_this = ed.buf().char_idx(line, ed.buf().line_len(line));
                let next_text = ed.buf().line_text(line + 1);
                let trimmed = next_text.trim_start();
                let leading_ws = next_text.chars().count() - trimmed.chars().count();
                let next_start = ed.buf().char_idx(line + 1, 0);
                ed.buf_mut()
                    .delete_char_range(end_of_this, next_start + leading_ws);
                join_col = end_of_this - ed.buf().char_idx(line, 0);
                // Vim inserts a single space in place of the line break, except
                // when the current line is empty or already ends in whitespace,
                // or the joined-on text begins with ')'.
                let add_space = !cur_text.is_empty()
                    && !cur_text.ends_with([' ', '\t'])
                    && !trimmed.starts_with(')');
                if add_space && end_of_this < ed.buf().rope.len_chars() {
                    ed.buf_mut().insert_char(line, join_col, ' ');
                }
            }
            ed.buf_mut().commit_edit();
            ed.set_cursor(line, join_col);
            ed.pending.reset();
            ed.finish_change_recording();
        }
        Key::Char('~') => {
            ed.start_change_recording(key);
            let (line, col) = ed.cursor();
            let n = ed.pending.total_count();
            let len = ed.buf().line_len(line);
            let end = crate::grapheme::step(&ed.buf().line_text(line), col, n, true).min(len);
            if end > col {
                ed.buf_mut().begin_edit();
                let start_idx = ed.buf().char_idx(line, col);
                let end_idx = ed.buf().char_idx(line, end);
                let text = ed.buf_mut().delete_char_range(start_idx, end_idx);
                let toggled: String = text
                    .chars()
                    .map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().collect::<String>()
                        } else {
                            c.to_uppercase().collect::<String>()
                        }
                    })
                    .collect();
                ed.buf_mut().insert_str(line, col, &toggled);
                ed.buf_mut().commit_edit();
                ed.set_cursor(line, end.min(len.saturating_sub(1)));
            }
            ed.pending.reset();
            ed.finish_change_recording();
        }
        Key::Ctrl('a') => increment(ed, 1),
        Key::Ctrl('x') => increment(ed, -1),
        Key::Char('K') => {
            ed.request_hover();
            ed.pending.reset();
        }
        Key::Char('v') => {
            ed.enter_visual(VisualKind::Char);
            ed.pending.reset();
        }
        Key::Char('V') => {
            ed.enter_visual(VisualKind::Line);
            ed.pending.reset();
        }
        Key::Char(':') => {
            ed.enter_command(CommandKind::Ex);
            ed.pending.reset();
        }
        Key::Char('/') => {
            ed.enter_command(CommandKind::SearchFwd);
            ed.pending.reset();
        }
        Key::Char('?') => {
            ed.enter_command(CommandKind::SearchBack);
            ed.pending.reset();
        }
        Key::Char('n') => search_next(ed, true),
        Key::Char('N') => search_next(ed, false),
        Key::Char('.') => {
            let mut keys = ed.last_change.clone();
            let count = ed.pending.count;
            ed.pending.reset();
            if let Some((kind, height, width, op)) = ed.visual_repeat {
                let (l, c) = ed.cursor();
                let end_line = (l + height).min(ed.buf().line_count().saturating_sub(1));
                let end_col = if height == 0 || kind == VisualKind::Block {
                    c + width.saturating_sub(1)
                } else {
                    width.saturating_sub(1)
                };
                let replaying = ed.replaying;
                ed.replaying = true;
                if kind == VisualKind::Block {
                    let left = crate::grapheme::cell(&ed.buf().line_text(l), c, ed.buf().tabstop);
                    crate::visual::apply_block_cells(ed, op, l, end_line, left, left + width);
                } else {
                    apply_operator_motion(
                        ed,
                        op,
                        (l, c),
                        (end_line, end_col),
                        if kind == VisualKind::Line {
                            Span::Linewise
                        } else {
                            Span::Inclusive
                        },
                    );
                }
                if op == OperatorKind::Change {
                    ed.replay(&keys[1.min(keys.len())..]);
                }
                ed.replaying = replaying;
            } else {
                if let Some(n) = count {
                    let start = if keys.first() == Some(&Key::Char('"')) {
                        2
                    } else {
                        0
                    };
                    let mut end = start;
                    while keys
                        .get(end)
                        .is_some_and(|k| k.as_char().is_some_and(|c| c.is_ascii_digit()))
                    {
                        end += 1;
                    }
                    keys.splice(start..end, n.to_string().chars().map(Key::Char));
                }
                ed.replay(&keys);
            }
        }
        Key::Char('q') => {
            if ed.macro_recording.is_none() {
                ed.pending.awaiting = Some(Awaiting::MacroRegister);
            }
        }
        Key::Char('@') => ed.pending.awaiting = Some(Awaiting::MacroReplay),
        Key::Char('m') => ed.pending.awaiting = Some(Awaiting::MarkSet),
        Key::Char('`') => ed.pending.awaiting = Some(Awaiting::MarkJump { exact: true }),
        Key::Char('\'') => ed.pending.awaiting = Some(Awaiting::MarkJump { exact: false }),
        Key::Char('i') => begin_insert(ed, key, |ed| ed.cursor()),
        Key::Char('a') => begin_insert(ed, key, |ed| {
            let (l, c) = ed.cursor();
            (l, (c + 1).min(ed.buf().line_len(l)))
        }),
        Key::Char('I') => begin_insert(ed, key, |ed| {
            (ed.cursor().0, ed.buf().first_non_blank(ed.cursor().0))
        }),
        Key::Char('A') => begin_insert(ed, key, |ed| {
            (ed.cursor().0, ed.buf().line_len(ed.cursor().0))
        }),
        Key::Char('o') => {
            ed.start_change_recording(key);
            let line = ed.cursor().0;
            let indent = ed.auto_indent(line, ed.buf().line_len(line));
            ed.buf_mut().begin_edit();
            let idx = ed.buf().char_idx(line, ed.buf().line_len(line));
            ed.buf_mut().insert_char_at(idx, '\n');
            ed.buf_mut().insert_str_at(idx + 1, &indent);
            // `3o` opens the line once, then on leaving Insert repeats the
            // whole "<newline><indent><typed>" unit; insert_start points at the
            // opening newline so the repeated unit is newline-first.
            ed.insert_repeat = ed.pending.total_count();
            ed.insert_start = idx;
            ed.set_cursor_insert(line + 1, indent.chars().count());
            ed.enter_insert();
            ed.pending.reset();
        }
        Key::Char('O') => {
            ed.start_change_recording(key);
            let line = ed.cursor().0;
            // Open-above copies the current line's indent; the bracket-aware
            // increase only applies when opening *after* a line (split_col 0).
            let indent = ed.auto_indent(line, 0);
            ed.buf_mut().begin_edit();
            if line > 0 {
                // Open above by appending a newline to the previous line, so
                // the repeated `3O` unit is newline-first (like `o`) and stacks
                // into separate lines rather than concatenating onto one.
                let idx = ed.buf().char_idx(line - 1, ed.buf().line_len(line - 1));
                ed.buf_mut().insert_char_at(idx, '\n');
                ed.buf_mut().insert_str_at(idx + 1, &indent);
                ed.insert_repeat = ed.pending.total_count();
                ed.insert_start = idx;
                ed.set_cursor_insert(line, indent.chars().count());
            } else {
                let idx = ed.buf().char_idx(line, 0);
                ed.buf_mut().insert_char_at(idx, '\n');
                ed.buf_mut().insert_str_at(idx, &indent);
                ed.set_cursor_insert(line, indent.chars().count());
            }
            ed.enter_insert();
            ed.pending.reset();
        }
        Key::Char('s') => {
            ed.start_change_recording(key);
            let (line, col) = ed.cursor();
            let n = ed.pending.total_count();
            let end = crate::grapheme::step(&ed.buf().line_text(line), col, n, true);
            ed.buf_mut().begin_edit();
            let s = ed.buf().char_idx(line, col);
            let e = ed.buf().char_idx(line, end);
            let text = ed.buf_mut().delete_char_range(s, e);
            ed.registers.set(ed.pending.register, text, false);
            ed.set_cursor_insert(line, col);
            ed.enter_insert();
            ed.pending.reset();
        }
        Key::Char('S') => {
            // `S` is a synonym for `cc`: a linewise change of `count` lines that
            // preserves the indent, uses a linewise register, and honors the
            // count. Route through the shared linewise-change path so it matches
            // `cc` exactly instead of the old charwise, indent-losing behavior.
            ed.start_change_recording(key);
            let line = ed.cursor().0;
            let n = ed.pending.total_count();
            let l2 = (line + n - 1).min(ed.buf().line_count().saturating_sub(1));
            apply_operator_motion(ed, OperatorKind::Change, (line, 0), (l2, 0), Span::Linewise);
            ed.pending.reset();
        }
        Key::Ctrl('v') => {
            ed.enter_visual(VisualKind::Block);
            ed.pending.reset();
        }
        Key::Ctrl('d') => {
            scroll_cursor(ed, half_page(ed) as isize, true);
            ed.pending.reset();
        }
        Key::Ctrl('u') => {
            scroll_cursor(ed, -(half_page(ed) as isize), true);
            ed.pending.reset();
        }
        Key::Ctrl('f') | Key::PageDown => {
            scroll_cursor(ed, ed.screen_rows.max(1) as isize, false);
            ed.pending.reset();
        }
        Key::Ctrl('b') | Key::PageUp => {
            scroll_cursor(ed, -(ed.screen_rows.max(1) as isize), false);
            ed.pending.reset();
        }
        Key::Ctrl('e') => {
            scroll_view_only(ed, 1);
            ed.pending.reset();
        }
        Key::Ctrl('y') => {
            scroll_view_only(ed, -1);
            ed.pending.reset();
        }
        Key::Char('z') => {
            ed.pending.awaiting = Some(Awaiting::ZPrefix);
        }
        Key::Esc => {
            ed.document_highlights.clear();
            ed.document_colors.clear();
            ed.inlay_hints.clear();
            ed.pending.reset();
        }
        _ => ed.pending.reset(),
    }
}

fn leading_ws(s: &str) -> String {
    s.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

fn op_to_line_end(ed: &mut Editor, op: OperatorKind, key: Key) {
    ed.start_change_recording(key);
    let (line, col) = ed.cursor();
    if let Some((dl, dc, span)) = motion::resolve(
        ed.buf(),
        line,
        col,
        Motion::LineEnd,
        ed.pending.total_count(),
    ) {
        apply_operator_motion(ed, op, (line, col), (dl, dc), span);
    }
    ed.pending.reset();
}

fn do_paste(ed: &mut Editor, key: Key, after: bool) {
    ed.start_change_recording(key);
    let (line, col) = ed.cursor();
    let n = ed.pending.total_count();
    let reg = ed.pending.register;
    let tab = ed.buf().tabstop;
    let (buf, regs) = ed.buf_and_registers_mut();
    if let Some((l, c)) = operator::paste(buf, regs, reg, (line, col), after, n, tab) {
        ed.set_cursor(l, c);
    }
    ed.pending.reset();
    ed.finish_change_recording();
}

fn begin_insert(ed: &mut Editor, key: Key, pos_fn: impl Fn(&mut Editor) -> (usize, usize)) {
    ed.start_change_recording(key);
    let (l, c) = pos_fn(ed);
    ed.insert_repeat = ed.pending.total_count();
    ed.insert_start = ed.buf().char_idx(l, c);
    ed.buf_mut().begin_edit();
    ed.set_cursor_insert(l, c);
    ed.enter_insert();
    ed.pending.reset();
}

pub(crate) fn begin_operator(ed: &mut Editor, op: OperatorKind) {
    ed.pending.op_count = ed.pending.count.take();
    ed.pending.operator = Some(op);
    if matches!(
        op,
        OperatorKind::Delete
            | OperatorKind::Change
            | OperatorKind::IndentRight
            | OperatorKind::IndentLeft
    ) {
        let ch = match op {
            OperatorKind::Delete => 'd',
            OperatorKind::Change => 'c',
            OperatorKind::IndentRight => '>',
            OperatorKind::IndentLeft => '<',
            OperatorKind::Yank => 'y',
            OperatorKind::ToggleCase => '~',
            OperatorKind::Format => 'q',
        };
        ed.start_change_recording(Key::Char(ch));
    }
}

fn op_char(op: OperatorKind) -> char {
    match op {
        OperatorKind::Delete => 'd',
        OperatorKind::Change => 'c',
        OperatorKind::Yank => 'y',
        OperatorKind::ToggleCase => '~',
        OperatorKind::IndentRight => '>',
        OperatorKind::IndentLeft => '<',
        OperatorKind::Format => 'q',
    }
}

fn doubled(op: OperatorKind, key: Key) -> bool {
    key.as_char() == Some(op_char(op))
}

pub(crate) fn key_to_motion(ed: &Editor, key: Key) -> Option<Motion> {
    match key {
        Key::Char('h') | Key::Left => Some(Motion::Left),
        Key::Char('l') | Key::Right => Some(Motion::Right),
        Key::Char('j') | Key::Down => Some(Motion::Down),
        Key::Char('k') | Key::Up => Some(Motion::Up),
        Key::Char('0') => Some(if ed.config.swap_0_and_caret {
            Motion::FirstNonBlank
        } else {
            Motion::LineStart
        }),
        Key::Char('^') | Key::Home => Some(if ed.config.swap_0_and_caret {
            Motion::LineStart
        } else {
            Motion::FirstNonBlank
        }),
        Key::Char('$') | Key::End => Some(Motion::LineEnd),
        Key::Char('w') => Some(Motion::WordFwd(false)),
        Key::Char('W') => Some(Motion::WordFwd(true)),
        Key::Char('b') => Some(Motion::WordBack(false)),
        Key::Char('B') => Some(Motion::WordBack(true)),
        Key::Char('e') => Some(Motion::WordEndFwd(false)),
        Key::Char('E') => Some(Motion::WordEndFwd(true)),
        Key::Char('{') => Some(Motion::ParaBack),
        Key::Char('}') => Some(Motion::ParaFwd),
        Key::Char('G') => Some(match ed.pending.count {
            Some(n) => Motion::GotoLine(n),
            None => Motion::FileEnd,
        }),
        _ => None,
    }
}

pub fn apply_motion_or_operator(ed: &mut Editor, motion: Motion) {
    let (line, mut col) = ed.cursor();
    let is_line_end = matches!(motion, Motion::LineEnd);
    let vertical = matches!(motion, Motion::Up | Motion::Down);
    // `$` is "sticky" like Vim's curswant == MAXCOL: once used, vertical
    // motion follows the end of each line. `desired_col == usize::MAX` is that
    // sentinel (the only place it is read is here).
    let eol_sticky = ed.buf().desired_col == usize::MAX;
    if vertical && ed.pending.operator.is_none() {
        col = if eol_sticky {
            ed.buf().line_len(line).saturating_sub(1)
        } else {
            ed.buf().desired_col
        };
    }
    let desired = col;
    let motion = cw_special_case(ed, motion, line, col);
    let count = ed.pending.total_count();
    if let Some((mut dl, mut dc, mut span)) = motion::resolve(ed.buf(), line, col, motion, count) {
        if let Some(op) = ed.pending.operator {
            // Vim's exclusive-motion rule 1: an exclusive motion whose end is
            // in column 1 of a *later* line has its end moved back to the end
            // of the previous line and becomes inclusive -- so e.g. `dw` on
            // the last word of a line does not delete the line break and join.
            if span == Span::Exclusive && dc == 0 && dl > line {
                let pl = dl - 1;
                dl = pl;
                dc = ed.buf().line_len(pl).saturating_sub(1);
                span = Span::Inclusive;
            }
            apply_operator_motion(ed, op, (line, col), (dl, dc), span);
        } else if vertical && eol_sticky {
            let end = ed.buf().line_len(dl).saturating_sub(1);
            ed.set_cursor(dl, end);
            ed.buf_mut().desired_col = usize::MAX; // keep following the line end
        } else {
            ed.set_cursor(dl, dc);
            if vertical {
                ed.buf_mut().desired_col = desired;
            } else if is_line_end {
                ed.buf_mut().desired_col = usize::MAX; // `$` arms the sticky flag
            }
        }
    }
    ed.pending.reset();
}

/// Vim special-cases `cw`/`cW`: on a non-blank character it behaves like
/// `ce`/`cE` (stops before trailing whitespace) instead of the usual `dw`
/// span that eats the whitespace up to the next word.
fn cw_special_case(ed: &Editor, motion: Motion, line: usize, col: usize) -> Motion {
    if ed.pending.operator != Some(OperatorKind::Change) {
        return motion;
    }
    if let Motion::WordFwd(big) = motion {
        let on_blank = ed
            .buf()
            .line_text(line)
            .chars()
            .nth(col)
            .map(|c| c.is_whitespace())
            .unwrap_or(true);
        if !on_blank {
            return Motion::WordEndFwd(big);
        }
    }
    motion
}

fn apply_linewise_current(ed: &mut Editor) {
    let op = ed.pending.operator.unwrap();
    let line = ed.cursor().0;
    let n = ed.pending.total_count();
    let l2 = (line + n - 1).min(ed.buf().line_count().saturating_sub(1));
    apply_operator_motion(ed, op, (line, 0), (l2, 0), Span::Linewise);
    ed.pending.reset();
}

/// Resolves an operator's (from, to, span) into a (start, end, linewise)
/// char-index range. Shared by the operator dispatch below and by
/// surround's Visual `S`, which needs the same range without immediately
/// performing an operator.
pub(crate) fn span_to_range(
    ed: &Editor,
    from: (usize, usize),
    to: (usize, usize),
    span: Span,
) -> (usize, usize, bool) {
    let buf = ed.buf();
    let from_idx = buf.char_idx(from.0, from.1);
    let to_idx = buf.char_idx(to.0, to.1);
    let (mut start, mut end, linewise) = match span {
        Span::Empty => (from_idx, from_idx, false),
        Span::Exclusive => (from_idx.min(to_idx), from_idx.max(to_idx), false),
        Span::Inclusive => {
            let end = from_idx.max(to_idx);
            let (l, c) = ed.buf().pos_from_char_idx(end);
            (
                from_idx.min(to_idx),
                ed.buf()
                    .char_idx(l, crate::grapheme::step(&ed.buf().line_text(l), c, 1, true)),
                false,
            )
        }
        Span::Linewise => {
            let l1 = from.0.min(to.0);
            let l2 = from.0.max(to.0);
            let s = buf.char_idx(l1, 0);
            let e = if l2 + 1 < buf.line_count() {
                buf.char_idx(l2 + 1, 0)
            } else {
                buf.rope.len_chars()
            };
            (s, e, true)
        }
    };
    if end < start {
        std::mem::swap(&mut start, &mut end);
    }
    (start, end, linewise)
}

pub(crate) fn apply_operator_motion(
    ed: &mut Editor,
    op: OperatorKind,
    from: (usize, usize),
    to: (usize, usize),
    span: Span,
) {
    let (start, end, linewise) = span_to_range(ed, from, to, span);
    let reg = ed.pending.register;

    match op {
        OperatorKind::ToggleCase => {
            ed.buf_mut().begin_edit();
            let text = ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut()
                .insert_str_at(start, &crate::operator::toggle_case(&text));
            ed.buf_mut().commit_edit();
            let (l, c) = ed.buf().pos_from_char_idx(start);
            ed.set_cursor(l, c);
            ed.finish_change_recording();
        }
        OperatorKind::Delete => {
            let (buf, regs) = ed.buf_and_registers_mut();
            operator::delete_range(buf, regs, reg, start, end, linewise);
            let (mut nl, mut nc) = ed.buf().pos_from_char_idx(start);
            if linewise {
                nl = nl.min(ed.buf().line_count().saturating_sub(1));
                nc = ed.buf().first_non_blank(nl);
            }
            ed.set_cursor(nl, nc);
            ed.finish_change_recording();
        }
        OperatorKind::Yank => {
            let (buf, regs) = ed.buf_and_registers_mut();
            operator::yank_range(buf, regs, reg, start, end, linewise);
            let (nl, nc) = ed.buf().pos_from_char_idx(start);
            ed.set_cursor(nl, nc);
        }
        OperatorKind::Change => {
            ed.buf_mut().begin_edit();
            if linewise {
                let indent = leading_ws(&ed.buf().line_text(from.0.min(to.0)));
                let text = ed.buf_mut().delete_char_range(start, end);
                ed.registers.set(reg, text, true);
                ed.buf_mut().insert_char_at(start, '\n');
                ed.buf_mut().insert_str_at(start, &indent);
                let (nl, _) = ed.buf().pos_from_char_idx(start);
                ed.set_cursor_insert(nl, indent.chars().count());
            } else {
                let text = ed.buf_mut().delete_char_range(start, end);
                ed.registers.set(reg, text, false);
                let (nl, nc) = ed.buf().pos_from_char_idx(start);
                ed.set_cursor_insert(nl, nc);
            }
            ed.enter_insert();
        }
        OperatorKind::IndentRight | OperatorKind::IndentLeft => {
            let l1 = from.0.min(to.0);
            let l2 = from.0.max(to.0);
            let sw = ed.buf().shiftwidth;
            operator::indent_lines(
                ed.buf_mut(),
                l1,
                l2,
                matches!(op, OperatorKind::IndentRight),
                sw,
            );
            let fnb = ed.buf().first_non_blank(l1);
            ed.set_cursor(l1, fnb);
            ed.finish_change_recording();
        }
        OperatorKind::Format => {
            let l1 = from.0.min(to.0);
            let l2 = from.0.max(to.0);
            // `.editorconfig` max_line_length overrides the global textwidth.
            let width = ed
                .buf()
                .ec_max_line_length
                .filter(|&n| n > 0)
                .or(Some(ed.config.textwidth).filter(|&n| n > 0))
                .unwrap_or(79);
            let start = ed.buf().char_idx(l1, 0);
            let end = ed.buf().char_idx(l2, ed.buf().line_len(l2));
            let text = ed.buf().rope.slice(start..end).to_string();
            let reflowed = crate::operator::reflow(&text, width);
            if reflowed != text {
                ed.buf_mut().begin_edit();
                ed.buf_mut().delete_char_range(start, end);
                ed.buf_mut().insert_str_at(start, &reflowed);
                ed.buf_mut().commit_edit();
            }
            let fnb = ed.buf().first_non_blank(l1);
            ed.set_cursor(l1, fnb);
            ed.finish_change_recording();
        }
    }
}

fn repeat_find(ed: &mut Editor, reverse: bool) {
    if let Some((ch, before, forward)) = ed.last_find {
        let forward = if reverse { !forward } else { forward };
        apply_motion_or_operator(
            ed,
            Motion::FindChar {
                ch,
                before,
                forward,
            },
        );
    }
}

fn search_next(ed: &mut Editor, same_direction: bool) {
    let (pattern, forward) = match &ed.last_search {
        Some(p) => p.clone(),
        None => return,
    };
    let forward = if same_direction { forward } else { !forward };
    let (line, col) = ed.cursor();
    let mut from = ed.buf().char_idx(line, col);
    let mut found = None;
    for _ in 0..ed.pending.total_count() {
        let result = ed.find_search(&pattern, from, forward);
        let idx = match result {
            Ok(Some(idx)) => idx,
            Ok(None) => break,
            Err(e) => {
                ed.set_message(format!("Search failed: {e}"));
                ed.pending.reset();
                return;
            }
        };
        found = Some(idx);
        from = idx;
    }
    if let Some(idx) = found {
        let (l, c) = ed.buf().pos_from_char_idx(idx);
        ed.set_cursor(l, c);
        recenter_viewport(ed);
    } else {
        ed.set_message(format!("pattern not found: {}", pattern));
    }
    ed.pending.reset();
}

pub(crate) fn object_kind(c: char) -> Option<ObjectKind> {
    match c {
        'w' => Some(ObjectKind::Word(false)),
        'W' => Some(ObjectKind::Word(true)),
        '(' | ')' | 'b' => Some(ObjectKind::Paren),
        '{' | '}' | 'B' => Some(ObjectKind::Brace),
        '[' | ']' => Some(ObjectKind::Bracket),
        '<' | '>' => Some(ObjectKind::Angle),
        '"' => Some(ObjectKind::DoubleQuote),
        '\'' => Some(ObjectKind::SingleQuote),
        '`' => Some(ObjectKind::Backtick),
        'a' => Some(ObjectKind::Argument),
        'p' => Some(ObjectKind::Paragraph),
        _ => None,
    }
}

pub(crate) fn handle_awaiting(ed: &mut Editor, awaiting: Awaiting, key: Key) {
    match awaiting {
        Awaiting::Diagnostic(forward) => {
            match key {
                Key::Char('d') => ed.next_diagnostic(forward),
                Key::Char('c') => ed.next_hunk(forward),
                // `]e` / `[e`: move the current line down / up (vim-unimpaired).
                Key::Char('e') => ed.move_lines(forward, ed.pending.total_count()),
                // `]f` / `[f`: jump to the next / previous function definition.
                Key::Char('f') => ed.goto_function(forward, ed.pending.total_count()),
                // `]s` / `[s`: jump to the next / previous misspelled word.
                Key::Char('s') => ed.spell_nav(forward),
                _ => {}
            }
            ed.pending.reset();
        }
        Awaiting::GPrefix => match key {
            Key::Char('g') => {
                let motion = match ed.pending.count {
                    Some(n) => Motion::GotoLine(n),
                    None => Motion::FileStart,
                };
                apply_motion_or_operator(ed, motion);
            }
            Key::Char('d') => {
                ed.request_definition();
                ed.pending.reset();
            }
            Key::Char('y') => {
                ed.request_type_definition();
                ed.pending.reset();
            }
            Key::Char('I') => {
                ed.request_implementation();
                ed.pending.reset();
            }
            Key::Char('D') => {
                ed.request_declaration();
                ed.pending.reset();
            }
            // camelCase/snake_case/kebab-case-aware subword motions.
            // Bare motions, not real Vim's gw (format)/ge/gE, which this
            // codebase doesn't implement yet -- see NEOVIM_PARITY_PLAN.md.
            Key::Char('w') => apply_motion_or_operator(ed, Motion::SubwordFwd),
            Key::Char('b') => apply_motion_or_operator(ed, Motion::SubwordBack),
            Key::Char('e') => apply_motion_or_operator(ed, Motion::SubwordEndFwd),
            Key::Char('a') => ed.pending.awaiting = Some(Awaiting::Align),
            Key::Char('v') => ed.reselect_visual(),
            // `gq{motion}` / `gqq`: reflow lines to `textwidth`. In Visual mode
            // it reflows the selection immediately; in Normal it leaves the
            // operator pending so the following motion (or a doubled `q`)
            // selects the line range, like d/c/y/>.
            Key::Char('q') => {
                if let crate::mode::Mode::Visual(kind) = ed.mode {
                    crate::visual::apply_to_selection(ed, OperatorKind::Format, kind);
                    ed.pending.reset();
                } else {
                    // Seed dot-repeat with `gq` (the `g` was already consumed by
                    // GPrefix), then let the following motion/doubled-`q` finish it.
                    ed.start_change_recording_seq(&[Key::Char('g'), Key::Char('q')]);
                    begin_operator(ed, OperatorKind::Format);
                }
            }
            Key::Char('t') => {
                match ed.pending.count {
                    Some(n) => ed.switch_tab(n.saturating_sub(1)),
                    None => ed.next_tab(),
                }
                ed.pending.reset();
            }
            Key::Char('T') => {
                ed.prev_tab();
                ed.pending.reset();
            }
            _ => ed.pending.reset(),
        },
        Awaiting::Align => {
            if let crate::mode::Mode::Visual(kind) = ed.mode {
                let range = ed.visual_anchor.and_then(|anchor| {
                    let cursor = ed.cursor();
                    (kind != VisualKind::Block).then_some((anchor.0, cursor.0))
                });
                if let (Some((l1, l2)), Some(delim)) = (range, key.as_char()) {
                    crate::align::align(ed, l1, l2, delim);
                }
                ed.visual_anchor = None;
                ed.pending.reset();
                ed.enter_normal();
            } else if key == Key::Char('p') {
                match paragraph_range(ed) {
                    Some((start, end)) => {
                        ed.pending.awaiting = Some(Awaiting::AlignDelim { start, end })
                    }
                    None => ed.pending.reset(),
                }
            } else {
                ed.pending.reset();
            }
        }
        Awaiting::AlignDelim { start, end } => {
            if let Some(delim) = key.as_char() {
                crate::align::align(ed, start, end, delim);
            }
            ed.pending.reset();
        }
        Awaiting::FindChar { forward, before } => {
            if let Some(ch) = key.as_char() {
                ed.last_find = Some((ch, before, forward));
                apply_motion_or_operator(
                    ed,
                    Motion::FindChar {
                        ch,
                        before,
                        forward,
                    },
                );
            } else {
                ed.pending.reset();
            }
        }
        Awaiting::Replace => {
            if let Some(ch) = key.as_char() {
                let (line, col) = ed.cursor();
                let n = ed.pending.total_count();
                let len = ed.buf().line_len(line);
                if col + n <= len {
                    ed.buf_mut().begin_edit();
                    let start = ed.buf().char_idx(line, col);
                    let end = ed.buf().char_idx(
                        line,
                        crate::grapheme::step(&ed.buf().line_text(line), col, n, true),
                    );
                    ed.buf_mut().delete_char_range(start, end);
                    let rep: String = std::iter::repeat_n(ch, n).collect();
                    ed.buf_mut().insert_str(line, col, &rep);
                    ed.buf_mut().commit_edit();
                    ed.set_cursor(line, col + n - 1);
                }
            }
            ed.pending.reset();
            ed.finish_change_recording();
        }
        Awaiting::TextObject { inner } => {
            if let Some(c) = key.as_char() {
                // Tree-sitter objects: function (`f`) / class (`c`).
                if matches!(c, 'f' | 'c') {
                    if let Some((sl, sc, el, ec)) = ed.tree_object_range(c, inner) {
                        let op = ed.pending.operator.unwrap_or(OperatorKind::Yank);
                        apply_operator_motion(
                            ed,
                            op,
                            (sl, sc),
                            (el, ec),
                            if (sl, sc) > (el, ec) {
                                Span::Empty
                            } else {
                                Span::Inclusive
                            },
                        );
                        ed.pending.reset();
                        return;
                    }
                    ed.pending.reset();
                    ed.abort_change_recording();
                    return;
                }
                if let Some(kind) = object_kind(c) {
                    let (line, col) = ed.cursor();
                    if let Some((sl, sc, el, ec)) =
                        textobject::resolve(ed.buf(), line, col, kind, inner)
                    {
                        let op = ed.pending.operator.unwrap_or(OperatorKind::Yank);
                        // Paragraph objects are linewise (`dip`/`dap` remove
                        // whole lines); the rest are charwise-inclusive.
                        let span = if (sl, sc) > (el, ec) {
                            Span::Empty
                        } else if matches!(kind, ObjectKind::Paragraph) {
                            Span::Linewise
                        } else {
                            Span::Inclusive
                        };
                        apply_operator_motion(ed, op, (sl, sc), (el, ec), span);
                        ed.pending.reset();
                        return;
                    }
                }
            }
            ed.pending.reset();
            ed.abort_change_recording();
        }
        Awaiting::RegisterName => {
            if let Some(c) = key.as_char() {
                ed.pending.register = Some(c);
            }
        }
        Awaiting::MarkSet => {
            if let Some(c) = key.as_char() {
                ed.set_mark(c);
            }
            ed.pending.reset();
        }
        Awaiting::MarkJump { exact } => {
            if let Some(c) = key.as_char() {
                ed.jump_mark(c, exact);
            }
            ed.pending.reset();
        }
        Awaiting::MacroRegister => {
            if let Some(c) = key.as_char() {
                ed.macro_recording = Some((c, Vec::new()));
                ed.set_message(format!("recording @{}", c));
            }
            ed.pending.reset();
        }
        Awaiting::MacroReplay => {
            let reg = match key {
                Key::Char('@') => ed.last_macro_reg,
                _ => key.as_char(),
            };
            if let Some(reg) = reg {
                ed.last_macro_reg = Some(reg);
                if let Some(keys) = ed.macros.get(&reg).cloned() {
                    let n = ed.pending.total_count();
                    ed.pending.reset();
                    for _ in 0..n.min(1000) {
                        ed.replay(&keys);
                        if ed.replay_budget == 0 {
                            break;
                        }
                    }
                    return;
                }
            }
            ed.pending.reset();
        }
        Awaiting::ZPrefix => {
            match key {
                Key::Char('z') => recenter_viewport(ed),
                Key::Char('t') => {
                    let line = ed.cursor().0;
                    ed.buf_mut().top_line = line;
                }
                Key::Char('b') => {
                    let line = ed.cursor().0;
                    let rows = ed.screen_rows.max(1);
                    ed.buf_mut().top_line = line.saturating_sub(rows.saturating_sub(1));
                }
                Key::Char('g') => spell_add(ed),
                Key::Char('=') => spell_suggest(ed),
                Key::Char('a') => ed.toggle_fold(),
                Key::Char('o') => ed.open_fold(),
                Key::Char('c') => ed.close_fold(),
                Key::Char('d') => ed.delete_fold(),
                Key::Char('R') => ed.open_all_folds(),
                Key::Char('M') => ed.close_all_folds(),
                _ => {}
            }
            ed.pending.reset();
        }
        Awaiting::Surround(stage) => crate::surround::handle(ed, stage, key),
        Awaiting::Leader { mut seq, since } => {
            if let Some(c) = key.as_char() {
                seq.push(c);
            } else {
                ed.pending.reset();
                return;
            }
            // User leader remaps take precedence over built-in leader actions;
            // a user remap that is a longer prefix keeps the sequence open.
            if let Some(rhs) = ed.leader_remap(&seq) {
                ed.pending.reset();
                ed.apply_remap(rhs);
                return;
            }
            let user_prefix = ed.leader_remap_prefix(&seq);
            match crate::actions::dispatch(ed, &seq) {
                crate::actions::Lookup::Ran => {
                    if ed.pending.operator.is_none() {
                        ed.pending.reset();
                    }
                }
                crate::actions::Lookup::Prefix => {
                    ed.pending.awaiting = Some(Awaiting::Leader { seq, since })
                }
                crate::actions::Lookup::NoMatch => {
                    if user_prefix {
                        // A longer user leader remap exists: keep waiting.
                        ed.pending.awaiting = Some(Awaiting::Leader { seq, since });
                    } else {
                        ed.set_message(format!("no such mapping: {}{}", ed.config.leader, seq));
                        ed.pending.reset();
                    }
                }
            }
        }
    }
}

/// Ctrl-A/Ctrl-X: increment/decrement the first decimal number at or after
/// the cursor on the current line by `sign * count`, preserving
/// zero-padded width (`007` -> `008`, not `8`) and a leading `-` sign.
/// A vim-compatible no-op if the current line has no number at or after
/// the cursor -- it never searches other lines or wraps.
fn increment(ed: &mut Editor, sign: i64) {
    let (line, col) = ed.cursor();
    let chars: Vec<char> = ed.buf().line_text(line).chars().collect();
    let Some(mut start) = (col..chars.len()).find(|&i| chars[i].is_ascii_digit()) else {
        ed.pending.reset();
        return;
    };
    if start > 0 && chars[start - 1] == '-' {
        start -= 1;
    }
    let digits_start = if chars[start] == '-' {
        start + 1
    } else {
        start
    };
    let mut end = digits_start;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    let width = end - digits_start;
    let raw: String = chars[start..end].iter().collect();
    let Ok(value) = raw.parse::<i64>() else {
        ed.pending.reset();
        return;
    };
    let key = if sign > 0 {
        Key::Ctrl('a')
    } else {
        Key::Ctrl('x')
    };
    ed.start_change_recording(key);
    let count = ed.pending.total_count() as i64;
    let new_value = value.saturating_add(sign.saturating_mul(count));
    let zero_padded = chars[digits_start] == '0' && width > 1;
    let mut text = if zero_padded {
        format!("{:0width$}", new_value.unsigned_abs(), width = width)
    } else {
        new_value.unsigned_abs().to_string()
    };
    if new_value < 0 {
        text = format!("-{text}");
    }
    ed.buf_mut().begin_edit();
    let s = ed.buf().char_idx(line, start);
    let e = ed.buf().char_idx(line, end);
    ed.buf_mut().delete_char_range(s, e);
    ed.buf_mut().insert_str_at(s, &text);
    ed.buf_mut().commit_edit();
    let new_col = start + text.chars().count() - 1;
    ed.set_cursor(line, new_col);
    ed.pending.reset();
    ed.finish_change_recording();
}

/// `zg`: add the word under the cursor to the user dictionary.
fn spell_add(ed: &mut Editor) {
    let (line, col) = ed.cursor();
    let text = ed.buf().line_text(line);
    let Some((_, _, word)) = crate::spell::Dictionary::word_at(&text, col) else {
        ed.set_message("No word under cursor");
        return;
    };
    match ed.ensure_dictionary().add_word(&word) {
        Ok(()) => ed.set_message(format!("Added \"{word}\" to the dictionary")),
        Err(e) => ed.set_message(format!("Could not update dictionary: {e}")),
    }
}

/// `z=`: show spelling suggestions for the word under the cursor as a
/// results list; selecting one replaces it in place.
fn spell_suggest(ed: &mut Editor) {
    let (line, col) = ed.cursor();
    let text = ed.buf().line_text(line);
    let Some((start, end, word)) = crate::spell::Dictionary::word_at(&text, col) else {
        ed.set_message("No word under cursor");
        return;
    };
    if !ed.ensure_dictionary().available() {
        ed.set_message("No dictionary found (looked in /usr/share/dict/words and similar)");
        return;
    }
    let suggestions = ed.ensure_dictionary().suggestions(&word);
    if suggestions.is_empty() {
        ed.set_message(format!("No suggestions for \"{word}\""));
        return;
    }
    let entries = suggestions
        .into_iter()
        .map(|s| {
            let mut e = crate::results::Entry::text(s.clone());
            e.action = Some(serde_json::json!({
                "_vaayu_spell_replace": {"line": line, "start": start, "end": end, "replacement": s}
            }));
            e
        })
        .collect();
    ed.show_results(crate::results::Results::new(
        format!("Suggestions for \"{word}\""),
        entries,
    ));
}

/// The contiguous run of non-blank lines around the cursor, for `gap`.
fn paragraph_range(ed: &Editor) -> Option<(usize, usize)> {
    let line = ed.cursor().0;
    if ed.buf().line_len(line) == 0 {
        return None;
    }
    let mut start = line;
    while start > 0 && ed.buf().line_len(start - 1) != 0 {
        start -= 1;
    }
    let mut end = line;
    let last = ed.buf().line_count().saturating_sub(1);
    while end < last && ed.buf().line_len(end + 1) != 0 {
        end += 1;
    }
    Some((start, end))
}

fn half_page(ed: &Editor) -> usize {
    (ed.screen_rows / 2).max(1)
}

/// Moves the cursor by `delta` lines and scrolls the viewport by the same
/// amount (matching real Vim's Ctrl-D/U/F/B, which scroll the window along
/// with the cursor rather than just moving the cursor within a fixed view).
/// `recenter` additionally centers afterward, for Ctrl-D/U's `zz` habit.
fn scroll_cursor(ed: &mut Editor, delta: isize, recenter: bool) {
    let last = ed.buf().line_count().saturating_sub(1);
    let cur_line = ed.cursor().0 as isize;
    let new_line = (cur_line + delta).clamp(0, last as isize) as usize;
    let col = ed.buf().first_non_blank(new_line);
    ed.set_cursor(new_line, col);

    let rows = ed.screen_rows.max(1) as isize;
    let max_top = (last as isize + 1 - rows).max(0);
    let top = ed.buf().top_line as isize;
    let new_top = (top + delta).clamp(0, max_top);
    ed.buf_mut().top_line = new_top as usize;

    if recenter {
        recenter_viewport(ed);
    }
}

/// Scrolls the viewport by `delta` lines without an explicit cursor motion
/// (Ctrl-E/Ctrl-Y), nudging the cursor only enough to keep it on screen.
fn scroll_view_only(ed: &mut Editor, delta: isize) {
    let last = ed.buf().line_count().saturating_sub(1);
    let rows = ed.screen_rows.max(1) as isize;
    let max_top = (last as isize + 1 - rows).max(0);
    let top = ed.buf().top_line as isize;
    let new_top = (top + delta).clamp(0, max_top) as usize;
    ed.buf_mut().top_line = new_top;

    let (cl, cc) = ed.cursor();
    let bottom = new_top + rows.max(1) as usize - 1;
    if cl < new_top {
        ed.set_cursor(new_top, cc);
    } else if cl > bottom {
        ed.set_cursor(bottom.min(last), cc);
    }
}

pub(crate) fn recenter_viewport(ed: &mut Editor) {
    let line = ed.cursor().0;
    let rows = ed.screen_rows.max(1);
    let last = ed.buf().line_count().saturating_sub(1);
    let max_top = last.saturating_sub(rows.saturating_sub(1));
    ed.buf_mut().top_line = line.saturating_sub(rows / 2).min(max_top);
}
