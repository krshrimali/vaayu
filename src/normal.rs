use crate::editor::Editor;
use crate::key::Key;
use crate::mode::{CommandKind, VisualKind};
use crate::motion::{self, Motion, Span};
use crate::operator::{self, OperatorKind};
use crate::textobject::{self, ObjectKind};

/// Ceiling for any accumulated Vim count (motion/operator repeat, paste
/// count, macro replay count). A long digit prefix could otherwise overflow
/// the accumulation arithmetic, or -- even saturated -- drive a
/// motion/paste/replay loop through an absurd number of iterations and hang
/// the editor; 100,000 is far past any legitimate use but keeps worst-case
/// loop counts bounded to a fraction of a second.
pub const MAX_COUNT: usize = 100_000;

#[derive(Debug, Clone)]
pub enum Awaiting {
    Diagnostic(bool),
    GPrefix,
    FindChar { forward: bool, before: bool },
    Replace,
    TextObject { inner: bool },
    RegisterName,
    MarkSet,
    MarkJump { exact: bool },
    MacroRegister,
    MacroReplay,
    Leader(String),
    ZPrefix,
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
        ed.pending.awaiting = Some(Awaiting::Leader(String::new()));
        return;
    }

    if key == Key::Char('"') && ed.pending.operator.is_none() {
        ed.pending.awaiting = Some(Awaiting::RegisterName);
        return;
    }

    // Operator already pending: only i/a (text object), Esc (cancel), same-char doubling
    // (linewise), or a motion are valid continuations.
    if let Some(op) = ed.pending.operator {
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
                let end_of_this = ed.buf().char_idx(line, ed.buf().line_len(line));
                let next_text = ed.buf().line_text(line + 1);
                let trimmed = next_text.trim_start();
                let leading_ws = next_text.chars().count() - trimmed.chars().count();
                let next_start = ed.buf().char_idx(line + 1, 0);
                ed.buf_mut()
                    .delete_char_range(end_of_this, next_start + leading_ws);
                join_col = end_of_this - ed.buf().char_idx(line, 0);
                if end_of_this < ed.buf().rope.len_chars() {
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
                    let left = crate::grapheme::cell(&ed.buf().line_text(l), c, ed.config.tabstop);
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
            let indent = leading_ws(&ed.buf().line_text(line));
            ed.buf_mut().begin_edit();
            let idx = ed.buf().char_idx(line, ed.buf().line_len(line));
            ed.buf_mut().insert_char_at(idx, '\n');
            ed.buf_mut().insert_str_at(idx + 1, &indent);
            ed.set_cursor_insert(line + 1, indent.chars().count());
            ed.enter_insert();
            ed.pending.reset();
        }
        Key::Char('O') => {
            ed.start_change_recording(key);
            let line = ed.cursor().0;
            let indent = leading_ws(&ed.buf().line_text(line));
            ed.buf_mut().begin_edit();
            let idx = ed.buf().char_idx(line, 0);
            ed.buf_mut().insert_char_at(idx, '\n');
            ed.buf_mut().insert_str_at(idx, &indent);
            ed.set_cursor_insert(line, indent.chars().count());
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
            ed.start_change_recording(key);
            let line = ed.cursor().0;
            ed.buf_mut().begin_edit();
            let s = ed.buf().char_idx(line, 0);
            let e = ed.buf().char_idx(line, ed.buf().line_len(line));
            let text = ed.buf_mut().delete_char_range(s, e);
            ed.registers.set(ed.pending.register, text, false);
            ed.set_cursor_insert(line, 0);
            ed.enter_insert();
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
        Key::Esc => ed.pending.reset(),
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
    let tab = ed.config.tabstop;
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

fn begin_operator(ed: &mut Editor, op: OperatorKind) {
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
    let vertical = matches!(motion, Motion::Up | Motion::Down);
    if vertical && ed.pending.operator.is_none() {
        col = ed.buf().desired_col;
    }
    let desired = col;
    let motion = cw_special_case(ed, motion, line, col);
    let count = ed.pending.total_count();
    if let Some((dl, dc, span)) = motion::resolve(ed.buf(), line, col, motion, count) {
        if let Some(op) = ed.pending.operator {
            apply_operator_motion(ed, op, (line, col), (dl, dc), span);
        } else {
            ed.set_cursor(dl, dc);
            if vertical {
                ed.buf_mut().desired_col = desired;
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

pub(crate) fn apply_operator_motion(
    ed: &mut Editor,
    op: OperatorKind,
    from: (usize, usize),
    to: (usize, usize),
    span: Span,
) {
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
            let sw = ed.config.shiftwidth;
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
        _ => None,
    }
}

pub(crate) fn handle_awaiting(ed: &mut Editor, awaiting: Awaiting, key: Key) {
    match awaiting {
        Awaiting::Diagnostic(forward) => {
            if key == Key::Char('d') {
                ed.next_diagnostic(forward);
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
            _ => ed.pending.reset(),
        },
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
                if let Some(kind) = object_kind(c) {
                    let (line, col) = ed.cursor();
                    if let Some((sl, sc, el, ec)) =
                        textobject::resolve(ed.buf(), line, col, kind, inner)
                    {
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
                _ => {}
            }
            ed.pending.reset();
        }
        Awaiting::Leader(mut seq) => {
            if let Some(c) = key.as_char() {
                seq.push(c);
            } else {
                ed.pending.reset();
                return;
            }
            match run_leader(ed, &seq) {
                LeaderResult::Ran => {
                    if ed.pending.operator.is_none() {
                        ed.pending.reset();
                    }
                }
                LeaderResult::Prefix => ed.pending.awaiting = Some(Awaiting::Leader(seq)),
                LeaderResult::NoMatch => {
                    ed.set_message(format!("no such mapping: {}{}", ed.config.leader, seq));
                    ed.pending.reset();
                }
            }
        }
    }
}

enum LeaderResult {
    Ran,
    Prefix,
    NoMatch,
}

const LEADER_CMDS: &[&str] = &[
    "rc", "rf", "rl", "rw", "cq", "ld", "lf", "lr", "la", "lo", "lR", "ls", "ms", "w", "q", "Q",
    "h", "d", "ow", "or", "ol", "R", "e", "ff", "fr", "b", "/", "z", "mp",
];

fn run_leader(ed: &mut Editor, seq: &str) -> LeaderResult {
    match seq {
        "rc" => {
            ed.new_note(false);
            return LeaderResult::Ran;
        }
        "rf" => {
            ed.new_note(true);
            return LeaderResult::Ran;
        }
        "rl" => {
            ed.comments_results();
            return LeaderResult::Ran;
        }
        "rw" => {
            let result = ed.save_notes();
            ed.set_message(match result {
                Ok(()) => "Comments saved".into(),
                Err(e) => e.to_string(),
            });
            return LeaderResult::Ran;
        }
        "cq" => {
            ed.open_quickfix();
            return LeaderResult::Ran;
        }
        "ld" => {
            let r = ed.diagnostic_results();
            ed.show_results(r);
            return LeaderResult::Ran;
        }
        "lf" => {
            ed.request_language("format", None);
            return LeaderResult::Ran;
        }
        "lr" => {
            ed.enter_command(CommandKind::Ex);
            ed.cmdline = "rename ".into();
            return LeaderResult::Ran;
        }
        "la" => {
            ed.request_language("actions", None);
            return LeaderResult::Ran;
        }
        "lo" => {
            ed.request_language("outline", None);
            return LeaderResult::Ran;
        }
        "lR" => {
            ed.request_language("references", None);
            return LeaderResult::Ran;
        }
        "ls" => {
            ed.request_language("signature", None);
            return LeaderResult::Ran;
        }
        "ms" => {
            ed.split_window(true, true);
            return LeaderResult::Ran;
        }
        "w" => {
            match ed.save_current() {
                Ok(()) => ed.set_message("written"),
                Err(e) => ed.set_message(format!("save failed: {}", e)),
            }
            return LeaderResult::Ran;
        }
        "q" => {
            let any_modified = ed.buffers.iter().any(|b| b.is_modified());
            if any_modified {
                ed.set_message("unsaved changes -- ,Q to discard, ,w to save");
            } else {
                ed.should_quit = true;
            }
            return LeaderResult::Ran;
        }
        "Q" => {
            ed.should_quit = true;
            return LeaderResult::Ran;
        }
        "h" => {
            ed.hl_search = false;
            return LeaderResult::Ran;
        }
        "d" => {
            ed.pending.register = Some('_');
            begin_operator(ed, OperatorKind::Delete);
            return LeaderResult::Ran;
        }
        "ow" => {
            ed.config.wrap = !ed.config.wrap;
            ed.set_message(format!("wrap: {}", ed.config.wrap));
            return LeaderResult::Ran;
        }
        "or" => {
            ed.config.relativenumber = !ed.config.relativenumber;
            ed.set_message(format!("relativenumber: {}", ed.config.relativenumber));
            return LeaderResult::Ran;
        }
        "ol" => {
            ed.set_message("Cursor line is indicated by the highlighted line number");
            return LeaderResult::Ran;
        }
        "R" => {
            ed.config = crate::config::Config::load();
            ed.restart_lsp();
            return LeaderResult::Ran;
        }
        "e" => {
            ed.open_picker();
            return LeaderResult::Ran;
        }
        "fr" => {
            let entries = ed
                .recent_files
                .iter()
                .map(|p| {
                    crate::results::Entry::location(
                        p.clone(),
                        0,
                        0,
                        p.file_name().unwrap_or_default().to_string_lossy(),
                    )
                })
                .collect();
            ed.show_results(crate::results::Results::new("Recent files", entries));
            return LeaderResult::Ran;
        }
        "ff" => {
            ed.open_picker();
            return LeaderResult::Ran;
        }
        "mp" => {
            ed.toggle_markdown_preview();
            return LeaderResult::Ran;
        }
        "b" => {
            ed.show_buffers();
            return LeaderResult::Ran;
        }
        "/" => {
            ed.open_grep("");
            return LeaderResult::Ran;
        }
        "z" => {
            ed.config.number = !ed.config.number;
            ed.set_message(if ed.config.number {
                "Zen off"
            } else {
                "Zen on — line numbers hidden"
            });
            return LeaderResult::Ran;
        }
        _ => {}
    }
    if LEADER_CMDS.iter().any(|c| c.starts_with(seq)) {
        LeaderResult::Prefix
    } else {
        LeaderResult::NoMatch
    }
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

fn recenter_viewport(ed: &mut Editor) {
    let line = ed.cursor().0;
    let rows = ed.screen_rows.max(1);
    let last = ed.buf().line_count().saturating_sub(1);
    let max_top = last.saturating_sub(rows.saturating_sub(1));
    ed.buf_mut().top_line = line.saturating_sub(rows / 2).min(max_top);
}
