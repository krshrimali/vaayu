use crate::editor::Editor;
use crate::key::Key;
use crate::mode::{CommandKind, Mode, VisualKind};
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
    /// `gp` -- goto-preview style peeks: `d`efinition, `t`ype definition,
    /// `i`mplementation, `r`eferences (same as `,pd`/`,pt`/`,pi`/`,pr`).
    PeekPrefix,
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
    /// `Z` prefix: `ZZ` (write and quit), `ZQ` (quit without writing).
    ZZPrefix,
    /// Visual `r`: the character to replace the selection with.
    VisualReplace,
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
    // Terminals encode Ctrl-\\ as byte 0x1c; crossterm may report that as
    // Ctrl-\\ or the traditional Ctrl-4 spelling depending on the backend.
    if matches!(key, Key::Ctrl('\\' | '4')) {
        ed.toggle_terminal_float();
        return;
    }
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
    // The leader still works from the tree (`,e` closes it, `,ff` opens the
    // picker, ...): only a bare leader with nothing else pending starts a
    // sequence here; everything else is the tree's.
    let tree_leader = ed.pending.awaiting.is_some()
        || (ed.pending.is_empty()
            && key.as_char().map(|c| c.to_string()) == Some(ed.config.leader.clone())
            && ed.file_tree.as_ref().is_some_and(|t| t.leader_ok()));
    if ed.active_file_tree() && !tree_leader {
        crate::filetree::handle_key(ed, key);
        return;
    }
    let outline_leader = ed.pending.awaiting.is_some()
        || (ed.pending.is_empty()
            && key.as_char().map(|c| c.to_string()) == Some(ed.config.leader.clone()));
    if ed.active_outline() && !outline_leader {
        crate::outline::handle_key(ed, key);
        return;
    }
    // The review contents (`:reviewstart`) index a file list, like the
    // outline indexes symbols; the leader still works from there.
    if ed.active_review_toc() && !outline_leader {
        crate::review_toc::handle_key(ed, key);
        return;
    }

    // Once a multi-key sequence is already in flight (e.g. `g` of `gt` was
    // just pressed), every subsequent key must reach `handle_awaiting`
    // regardless of the terminal guard below -- swallowing it here would
    // leave `pending.awaiting` permanently stuck instead of either
    // completing the sequence or being reset by its own catch-all arm.
    if let Some(id) = ed.active_terminal_id() {
        // `gg` on a terminal pane scrolls to the oldest output.
        if key == Key::Char('g') && matches!(ed.pending.awaiting, Some(Awaiting::GPrefix)) {
            ed.pending.reset();
            ed.scroll_terminal(id, crate::pty::TerminalScroll::Top);
            return;
        }
    }
    if let Some(id) = ed
        .active_terminal_id()
        .filter(|_| ed.pending.awaiting.is_none())
    {
        use crate::pty::TerminalScroll;
        let count = ed.pending.count.unwrap_or(1) as isize;
        let scroll = match key {
            Key::Char('k') | Key::Up | Key::Ctrl('y') | Key::Ctrl('p') => {
                Some(TerminalScroll::Lines(count))
            }
            Key::Char('j') | Key::Down | Key::Ctrl('e') | Key::Ctrl('n') | Key::Enter => {
                Some(TerminalScroll::Lines(-count))
            }
            Key::Ctrl('u') => Some(TerminalScroll::Half(true)),
            Key::Ctrl('d') => Some(TerminalScroll::Half(false)),
            Key::Ctrl('b') | Key::PageUp => Some(TerminalScroll::Page(true)),
            Key::Ctrl('f') | Key::PageDown | Key::Char(' ') => Some(TerminalScroll::Page(false)),
            Key::Char('G') => Some(TerminalScroll::Bottom),
            _ => None,
        };
        if let Some(scroll) = scroll {
            ed.pending.count = None;
            ed.scroll_terminal(id, scroll);
            return;
        }
        // Searching, selecting or yanking terminal output happens on a
        // captured copy of it in an ordinary buffer. The key (and its count)
        // carries over, so `yy`, `Y`, `3yy` and `yip` yank there at once.
        if matches!(key, Key::Char('/' | '?' | 'v' | 'V' | 'y' | 'Y')) {
            let count = ed.pending.count.take();
            if ed.open_terminal_capture() {
                match key {
                    Key::Char('/') => ed.enter_command(CommandKind::SearchFwd),
                    Key::Char('?') => ed.enter_command(CommandKind::SearchBack),
                    Key::Char(c) => {
                        ed.pending.count = count;
                        crate::normal::handle(ed, Key::Char(c));
                    }
                    _ => {}
                }
            }
            return;
        }
    }
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
            // Operator to a search match: `d/pat`, `c?pat`, `dn`, `y*`.
            Key::Char('/') => {
                ed.enter_command(CommandKind::SearchFwd);
                return;
            }
            Key::Char('?') => {
                ed.enter_command(CommandKind::SearchBack);
                return;
            }
            Key::Char('n') => {
                operator_to_search(ed, |ed| search_next(ed, true));
                return;
            }
            Key::Char('N') => {
                operator_to_search(ed, |ed| search_next(ed, false));
                return;
            }
            Key::Char('*') => {
                operator_to_search(ed, |ed| search_word_under_cursor(ed, true, true));
                return;
            }
            Key::Char('#') => {
                operator_to_search(ed, |ed| search_word_under_cursor(ed, false, true));
                return;
            }
            Key::Char(';') => {
                repeat_find(ed, false);
                return;
            }
            Key::Char(',') => {
                repeat_find(ed, true);
                return;
            }
            Key::Char('[') => {
                ed.pending.awaiting = Some(Awaiting::Diagnostic(false));
                return;
            }
            Key::Char(']') => {
                ed.pending.awaiting = Some(Awaiting::Diagnostic(true));
                return;
            }
            Key::Char('\'') => {
                // Operator to a line mark (`d'a`): linewise.
                ed.pending.awaiting = Some(Awaiting::MarkJump { exact: false });
                return;
            }
            Key::Char('`') => {
                // Operator to an exact mark (`` d`a ``): charwise exclusive.
                ed.pending.awaiting = Some(Awaiting::MarkJump { exact: true });
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
        // `,` is the default leader (handled above); with another leader it
        // repeats the last f/t backwards, as in Vim.
        Key::Char(',') => repeat_find(ed, true),
        Key::Char('=') => begin_operator(ed, OperatorKind::Reindent),
        Key::Char('U') => {
            line_undo(ed);
            ed.pending.reset();
        }
        Key::Char('Z') => ed.pending.awaiting = Some(Awaiting::ZZPrefix),
        Key::Char('&') => {
            ed.pending.reset();
            crate::command::run_ex(ed, "&");
        }
        // Neovim's `Q`: replay the last recorded macro.
        Key::Char('Q') => {
            let n = ed.pending.total_count();
            ed.pending.reset();
            replay_last_recorded(ed, n);
        }
        Key::Char('R') => {
            ed.start_change_recording(key);
            begin_replace(ed);
        }
        Key::Ctrl('g') => {
            ed.pending.reset();
            file_info(ed);
        }
        Key::Ctrl(']') => {
            ed.pending.reset();
            let (l, c) = ed.cursor();
            let id = ed.buf().id;
            ed.tag_stack.push((id, l, c));
            if ed.tag_stack.len() > 50 {
                ed.tag_stack.remove(0);
            }
            ed.push_jump();
            ed.request_definition();
        }
        Key::Ctrl('t') => {
            ed.pending.reset();
            tag_pop(ed);
        }
        Key::Ctrl('^') => {
            ed.pending.reset();
            ed.switch_to_alternate();
        }
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
            for _ in 0..ed.pending.total_count() {
                if !ed.buf_mut().undo() {
                    break;
                }
            }
            ed.set_message("undo");
            ed.pending.reset();
        }
        Key::Ctrl('r') => {
            for _ in 0..ed.pending.total_count() {
                if !ed.buf_mut().redo() {
                    break;
                }
            }
            ed.set_message("redo");
            ed.pending.reset();
        }
        Key::Ctrl('p') => {
            ed.open_picker();
            ed.pending.reset();
        }
        Key::Ctrl('n') => {
            crate::multicursor::add_next(ed);
            ed.pending.reset();
        }
        Key::Char('J') => {
            ed.start_change_recording(key);
            let line = ed.cursor().0;
            let n = ed.pending.total_count().max(2);
            let last = (line + n - 1).min(ed.buf().line_count().saturating_sub(1));
            if last == line {
                ed.pending.reset();
                ed.abort_change_recording();
                return;
            }
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
                    && !trimmed.is_empty()
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
            if ed.active_tour.is_some() {
                ed.toggle_tour_explanation();
            } else if ed
                .float
                .as_ref()
                .is_some_and(|f| f.title == "Hover" && !f.focused)
            {
                ed.focus_float();
            } else {
                ed.request_hover();
            }
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
        Key::Char('*') => search_word_under_cursor(ed, true, true),
        Key::Char('#') => search_word_under_cursor(ed, false, true),
        Key::Char('.') => {
            let mut keys = ed.last_change.clone();
            let count = ed.pending.count;
            ed.pending.reset();
            if let Some(repeat) = ed.visual_repeat {
                let crate::visual::RepeatSelection {
                    kind,
                    height,
                    width,
                    action,
                    ..
                } = repeat;
                let (l, c) = ed.cursor();
                let end_line = (l + height).min(ed.buf().line_count().saturating_sub(1));
                let end_cell = if height == 0 || kind == VisualKind::Block {
                    crate::grapheme::cell(&ed.buf().line_text(l), c, ed.buf().tabstop)
                        + width.saturating_sub(1)
                } else {
                    width.saturating_sub(1)
                };
                let end_col = if repeat.to_eol {
                    let text = ed.buf().line_text(end_line);
                    let len = text.chars().count();
                    if repeat.includes_newline {
                        len
                    } else {
                        crate::grapheme::step(&text, len, 1, false)
                    }
                } else {
                    crate::grapheme::raw_column(
                        &ed.buf().line_text(end_line),
                        end_cell,
                        ed.buf().tabstop,
                    )
                };
                let replaying = ed.replaying;
                ed.replaying = true;
                if let crate::visual::RepeatAction::Paste {
                    before,
                    register,
                    count: original_count,
                } = action
                {
                    if let Some(entry) = ed
                        .registers
                        .get(register)
                        .cloned()
                        .filter(|e| !e.text.is_empty())
                    {
                        let left =
                            crate::grapheme::cell(&ed.buf().line_text(l), c, ed.buf().tabstop);
                        let right = if repeat.to_eol {
                            (l..=end_line)
                                .map(|line| {
                                    crate::grapheme::cell(
                                        &ed.buf().line_text(line),
                                        ed.buf().line_len(line),
                                        ed.buf().tabstop,
                                    )
                                })
                                .max()
                                .unwrap_or(0)
                                .max(left + 1)
                        } else {
                            left + width
                        };
                        crate::visual::paste_block_cells(
                            ed,
                            &entry,
                            before,
                            count.unwrap_or(original_count),
                            (l, end_line, left, right),
                        );
                    }
                    ed.replaying = replaying;
                    return;
                }
                let crate::visual::RepeatAction::Operator(op) = action else {
                    unreachable!()
                };
                if kind == VisualKind::Block {
                    let left = crate::grapheme::cell(&ed.buf().line_text(l), c, ed.buf().tabstop);
                    crate::visual::apply_block_cells(ed, op, l, end_line, left, left + width);
                } else {
                    ed.mode = Mode::Visual(kind);
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
                    if matches!(ed.mode, Mode::Visual(_)) {
                        ed.enter_normal();
                    }
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
                // Opening above the first line: there is no preceding line to
                // hang a newline-first repeat unit on, so record the count and
                // let `leave_insert` prepend the finished line (with its
                // trailing newline) `count-1` more times for `3O`.
                let idx = ed.buf().char_idx(line, 0);
                ed.buf_mut().insert_char_at(idx, '\n');
                ed.buf_mut().insert_str_at(idx, &indent);
                ed.insert_repeat = ed.pending.total_count();
                ed.insert_start = idx;
                ed.insert_open_bof = true;
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
    ed.buf_mut().begin_edit_at((l, c));
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
            | OperatorKind::Reindent
    ) {
        ed.start_change_recording(Key::Char(op_char(op)));
    } else if matches!(
        op,
        OperatorKind::Lower | OperatorKind::Upper | OperatorKind::Rot13 | OperatorKind::ToggleCase
    ) {
        // `g` was consumed by the g-prefix: seed dot-repeat with both keys.
        ed.start_change_recording_seq(&[Key::Char('g'), Key::Char(op_char(op))]);
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
        OperatorKind::Lower => 'u',
        OperatorKind::Upper => 'U',
        OperatorKind::Rot13 => '?',
        OperatorKind::Reindent => '=',
        OperatorKind::Fold => 'f',
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
        Key::Char('%') => Some(
            if ed.pending.count.is_some() || ed.pending.op_count.is_some() {
                Motion::Percentage(ed.pending.total_count())
            } else {
                Motion::MatchPair
            },
        ),
        Key::Char('G') => Some(match ed.pending.count {
            Some(n) => Motion::GotoLine(n),
            None => Motion::FileEnd,
        }),
        Key::Char('(') => Some(Motion::Sentence { forward: false }),
        Key::Char(')') => Some(Motion::Sentence { forward: true }),
        Key::Char('-') => Some(Motion::LineFnb { down: false }),
        Key::Char('+') | Key::Enter => Some(Motion::LineFnb { down: true }),
        Key::Char('_') => Some(Motion::CurrentLineFnb),
        Key::Char('|') => Some(Motion::Column),
        Key::Char('H') => Some(Motion::ToLine(screen_line(ed, 'H'))),
        Key::Char('L') => Some(Motion::ToLine(screen_line(ed, 'L'))),
        Key::Char('M') => Some(Motion::ToLine(screen_line(ed, 'M'))),
        _ => None,
    }
}

/// The buffer line `H` / `L` / `M` move to: `count` lines from the top or
/// bottom of the window (kept `scrolloff` away from an edge that can still
/// scroll), or the middle of the visible text.
fn screen_line(ed: &Editor, which: char) -> usize {
    let top = ed.buf().top_line;
    let rows = ed.screen_rows.max(1);
    let last = ed.buf().line_count().saturating_sub(1);
    let bottom = (top + rows - 1).min(last);
    let n = ed.pending.total_count().saturating_sub(1);
    let so = ed.config.scrolloff.min(rows.saturating_sub(1) / 2);
    let operator = ed.pending.operator.is_some();
    match which {
        'H' => {
            let min = if top > 0 && !operator { top + so } else { top };
            (top + n).max(min).min(bottom)
        }
        'L' => {
            let max = if bottom < last && !operator {
                bottom.saturating_sub(so)
            } else {
                bottom
            };
            bottom.saturating_sub(n).min(max).max(top)
        }
        _ => top + (bottom - top) / 2,
    }
}

pub fn apply_motion_or_operator(ed: &mut Editor, motion: Motion) {
    let (line, col) = ed.cursor();
    let is_line_end = matches!(motion, Motion::LineEnd);
    let vertical = matches!(motion, Motion::Up | Motion::Down);
    // `$` is "sticky" like Vim's curswant == MAXCOL: once used, vertical
    // motion follows the end of each line. `desired_col == usize::MAX` is that
    // sentinel (the only place it is read is here).
    let eol_sticky = ed.buf().desired_col == usize::MAX;
    let desired = ed.buf().desired_col;
    let count = ed.pending.total_count();
    let destination = if let (Some(op), Motion::WordFwd(big)) = (ed.pending.operator, motion) {
        let nonblank = ed
            .buf()
            .line_text(line)
            .chars()
            .nth(col)
            .is_some_and(|c| !c.is_whitespace());
        Some(if op == OperatorKind::Change && nonblank {
            motion::resolve_change_word(ed.buf(), line, col, big, count)
        } else {
            motion::resolve_word_operator(ed.buf(), line, col, big, count)
        })
    } else {
        motion::resolve(ed.buf(), line, col, motion, count)
    };
    if let Some((dl, dc, span)) = destination {
        if ed.pending.operator.is_none()
            && matches!(
                motion,
                Motion::MatchPair
                    | Motion::Percentage(_)
                    | Motion::ToLine(_)
                    | Motion::Sentence { .. }
                    | Motion::Section { .. }
            )
            && (dl, dc) != (line, col)
        {
            ed.push_jump();
        }
        if let Some(op) = ed.pending.operator {
            // A failed vertical motion must not turn into an operator on
            // the current line (e.g. `dk` at the top of the file).
            if vertical && dl == line {
                ed.pending.reset();
                ed.abort_change_recording();
                return;
            }
            if span == Span::Exclusive
                && (dl, dc) == (line, col)
                && matches!(
                    motion,
                    Motion::Left | Motion::WordBack(_) | Motion::SubwordBack
                )
            {
                ed.pending.reset();
                ed.abort_change_recording();
                return;
            }
            apply_operator_motion(ed, op, (line, col), (dl, dc), span);
        } else if vertical && eol_sticky {
            let end = ed.buf().line_len(dl).saturating_sub(1);
            ed.set_cursor(dl, end);
            ed.buf_mut().desired_col = usize::MAX; // keep following the line end
        } else {
            let dc = if vertical {
                crate::grapheme::raw_column(&ed.buf().line_text(dl), desired, ed.buf().tabstop)
            } else {
                dc
            };
            ed.set_cursor(dl, dc);
            if vertical {
                ed.buf_mut().desired_col = desired;
            } else if is_line_end {
                ed.buf_mut().desired_col = usize::MAX; // `$` arms the sticky flag
            }
        }
    } else if ed.pending.operator.is_some() {
        ed.abort_change_recording();
    }
    ed.pending.reset();
}

fn apply_linewise_current(ed: &mut Editor) {
    let op = ed.pending.operator.unwrap();
    let line = ed.cursor().0;
    let n = ed.pending.total_count();
    if n > ed.buf().line_count().saturating_sub(line) {
        ed.pending.reset();
        ed.abort_change_recording();
        return;
    }
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
        Span::Exclusive => {
            let (first, last) = if from <= to { (from, to) } else { (to, from) };
            if first.0 < last.0 && last.1 == 0 {
                if first.1 <= buf.first_non_blank(first.0) {
                    (buf.char_idx(first.0, 0), buf.char_idx(last.0, 0), true)
                } else {
                    // Keep the newline before the exclusive endpoint, in
                    // either direction (`cb` from column zero included).
                    (
                        from_idx.min(to_idx),
                        buf.char_idx(last.0 - 1, buf.line_len(last.0 - 1)),
                        false,
                    )
                }
            } else {
                (from_idx.min(to_idx), from_idx.max(to_idx), false)
            }
        }
        Span::Inclusive => {
            let end = from_idx.max(to_idx);
            let (l, c) = ed.buf().pos_from_char_idx(end);
            (
                from_idx.min(to_idx),
                if matches!(ed.mode, Mode::Visual(VisualKind::Char))
                    && c == buf.line_len(l)
                    && end < buf.rope.len_chars()
                    && l + 1 < buf.line_count()
                {
                    end + 1 // Visual selection of the end-of-line character.
                } else {
                    buf.char_idx(l, crate::grapheme::step(&buf.line_text(l), c, 1, true))
                },
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
    let (mut start, mut end, mut linewise) = span_to_range(ed, from, to, span);
    if op == OperatorKind::Delete && ed.mode == Mode::Normal && !linewise && from.0 != to.0 {
        let (sl, sc) = ed.buf().pos_from_char_idx(start);
        let (el, ec) = ed.buf().pos_from_char_idx(end);
        // A normal delete covering all text on several lines removes those
        // lines, including their indentation and final newline. Explicit
        // characterwise Visual selections keep their selected span.
        if sc <= ed.buf().first_non_blank(sl) && ec == ed.buf().line_len(el) {
            start = ed.buf().char_idx(sl, 0);
            end = if el + 1 < ed.buf().line_count() {
                ed.buf().char_idx(el + 1, 0)
            } else {
                ed.buf().rope.len_chars()
            };
            linewise = true;
        }
    }
    let reg = ed.pending.register;

    match op {
        OperatorKind::ToggleCase
        | OperatorKind::Lower
        | OperatorKind::Upper
        | OperatorKind::Rot13 => {
            let convert = |text: &str| match op {
                OperatorKind::Lower => text.to_lowercase(),
                OperatorKind::Upper => text.to_uppercase(),
                OperatorKind::Rot13 => crate::operator::rot13(text),
                _ => crate::operator::toggle_case(text),
            };
            ed.buf_mut().begin_edit();
            let text = ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut().insert_str_at(start, &convert(&text));
            ed.buf_mut().commit_edit();
            let (l, c) = ed.buf().pos_from_char_idx(start);
            ed.set_cursor(l, c);
            ed.finish_change_recording();
        }
        OperatorKind::Delete => {
            if !linewise {
                let change_start = ed.buf().pos_from_char_idx(start).min(from);
                ed.buf_mut().begin_edit_at(change_start);
            }
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
            let change_start = if linewise {
                from
            } else {
                ed.buf().pos_from_char_idx(start).min(from)
            };
            ed.buf_mut().begin_edit_at(change_start);
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
                if !text.is_empty() {
                    ed.registers.set(reg, text, false);
                }
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
        OperatorKind::Fold => {
            let l1 = from.0.min(to.0);
            let l2 = from.0.max(to.0);
            ed.create_fold(l1, l2);
            ed.set_cursor(l1, ed.buf().first_non_blank(l1));
        }
        OperatorKind::Reindent => {
            let l1 = from.0.min(to.0);
            let l2 = from.0.max(to.0);
            ed.reindent_lines(l1, l2);
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
                repeat: true,
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

/// The word `*`/`#` search for, as Vim picks it: the keyword under the
/// cursor, else the next keyword after it on the line, else the run of
/// non-blank characters there. Returns `(start column, text, is_keyword)`.
pub(crate) fn word_for_star(line: &str, col: usize) -> Option<(usize, String, bool)> {
    let chars: Vec<char> = line.chars().collect();
    let pick = |is_part: &dyn Fn(char) -> bool| -> Option<(usize, String)> {
        let first = (col..chars.len()).find(|&i| is_part(chars[i]))?;
        let mut start = first;
        if first == col {
            while start > 0 && is_part(chars[start - 1]) {
                start -= 1;
            }
        }
        let end = (first..chars.len())
            .find(|&i| !is_part(chars[i]))
            .unwrap_or(chars.len());
        Some((start, chars[start..end].iter().collect()))
    };
    let keyword = |c: char| c.is_alphanumeric() || c == '_';
    if let Some((start, word)) = pick(&keyword) {
        return Some((start, word, true));
    }
    pick(&|c: char| !c.is_whitespace()).map(|(start, word)| (start, word, false))
}

/// `*` / `#`: search forward / backward for the word under the cursor as a
/// whole word; `g*` / `g#` match it inside longer words too. Sets the last
/// search, so `n`/`N` continue it, and takes a count like Vim.
fn search_word_under_cursor(ed: &mut Editor, forward: bool, whole_word: bool) {
    let count = ed.pending.total_count();
    ed.pending.reset();
    let (line, col) = ed.cursor();
    let Some((start, word, is_keyword)) = word_for_star(&ed.buf().line_text(line), col) else {
        ed.set_message("No string under cursor");
        return;
    };
    let mut pattern = if is_keyword {
        // Keyword characters (letters, digits, `_`) are literal in a
        // magic pattern, so only the boundaries need adding.
        if whole_word {
            format!("\\<{word}\\>")
        } else {
            word.clone()
        }
    } else {
        format!("\\V{}", word.replace('\\', "\\\\"))
    };
    // Vim's `*` ignores smartcase: an uppercase word still matches any case
    // when ignorecase is on.
    if ed.config.ignorecase && ed.config.smartcase && word.chars().any(char::is_uppercase) {
        pattern = format!("\\c{pattern}");
    }
    ed.push_jump();
    // Search from the word's start, so `#` mid-word skips the word itself.
    ed.set_cursor(line, start);
    crate::command::push_search_history(ed, &pattern);
    crate::command::run_search(ed, &pattern, forward);
    for _ in 1..count {
        search_next(ed, true);
    }
}

/// Repeat the most recent buffer search in its original direction.
pub(crate) fn repeat_last_search(ed: &mut Editor) {
    search_next(ed, true);
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
        's' => Some(ObjectKind::Sentence),
        't' => Some(ObjectKind::Tag),
        _ => None,
    }
}

pub(crate) fn handle_awaiting(ed: &mut Editor, awaiting: Awaiting, key: Key) {
    match awaiting {
        Awaiting::Diagnostic(forward) => {
            match key {
                Key::Char('d') => ed.next_diagnostic(forward),
                Key::Char('c') => ed.next_hunk(forward),
                // `]r` / `[r`: next / previous open review comment.
                Key::Char('r') => ed.next_comment(forward),
                // `]e` / `[e`: move the current line down / up (vim-unimpaired).
                Key::Char('e') => ed.move_lines(forward, ed.pending.total_count()),
                // `]f` / `[f`: jump to the next / previous function definition.
                Key::Char('f') => ed.goto_function(forward, ed.pending.total_count()),
                // `]F` / `[F`: next / previous file of the review screen.
                Key::Char('F') => ed.review_file_step(forward),
                // `]s` / `[s`: jump to the next / previous misspelled word.
                Key::Char('s') => ed.spell_nav(forward),
                // `]t` / `[t`: next / previous code-tour step.
                Key::Char('t') => ed.tour_step(forward),
                // `]h` / `[h`: highlights within the current tour step.
                Key::Char('h') => ed.tour_highlight_step(forward, ed.pending.total_count()),
                // `]q` ends a tour; `[q` restarts the current tour.
                Key::Char('q') if forward => ed.tour_end(),
                Key::Char('q') => ed.tour_restart(),
                // `[[` `]]` `[]` `][`: sections.
                // The doubled key (`[[`, `]]`) finds starts, the mixed one
                // (`[]`, `][`) finds ends.
                Key::Char('[') => {
                    return apply_motion_or_operator(
                        ed,
                        Motion::Section {
                            forward,
                            end: forward,
                        },
                    )
                }
                Key::Char(']') => {
                    return apply_motion_or_operator(
                        ed,
                        Motion::Section {
                            forward,
                            end: !forward,
                        },
                    )
                }
                // `[(` `[{` `])` `]}`: the enclosing unmatched bracket.
                Key::Char('(') | Key::Char(')') => {
                    return apply_motion_or_operator(
                        ed,
                        Motion::Unmatched {
                            open: '(',
                            close: ')',
                            forward,
                        },
                    )
                }
                Key::Char('{') | Key::Char('}') => {
                    return apply_motion_or_operator(
                        ed,
                        Motion::Unmatched {
                            open: '{',
                            close: '}',
                            forward,
                        },
                    )
                }
                // `[m` `]m` `[M` `]M`: method starts / ends.
                Key::Char('m') => {
                    return apply_motion_or_operator(
                        ed,
                        Motion::Method {
                            forward,
                            end: false,
                        },
                    )
                }
                Key::Char('M') => {
                    return apply_motion_or_operator(ed, Motion::Method { forward, end: true })
                }
                // `[*` `[/` `]*` `]/`: start / end of a C-style comment.
                Key::Char('*') | Key::Char('/') => comment_bound(ed, forward),
                // `[p` `]p`: paste with the indent adjusted to this line.
                Key::Char('p') => paste_indented(ed, !forward),
                Key::Char('P') => paste_indented(ed, false),
                // `['` `]'` / `` [` `` `` ]` ``: previous / next lowercase mark.
                Key::Char('\'') => mark_step(ed, forward, false),
                Key::Char('`') => mark_step(ed, forward, true),
                _ => {}
            }
            ed.pending.reset();
        }
        Awaiting::VisualReplace => {
            if let Some(c) = key.as_char() {
                crate::visual::replace_selection(ed, c);
            }
            ed.pending.reset();
        }
        Awaiting::ZZPrefix => {
            ed.pending.reset();
            match key {
                Key::Char('Z') => crate::command::run_ex(ed, "x"),
                Key::Char('Q') => crate::command::run_ex(ed, "q!"),
                _ => {}
            }
        }
        Awaiting::GPrefix => match key {
            // `gugu` / `gUgU` / `g~g~` / `g?g?`: the doubled form is linewise.
            Key::Char(c @ ('u' | 'U' | '~' | '?'))
                if ed.pending.operator.is_some_and(|op| op_char(op) == c) =>
            {
                apply_linewise_current(ed);
            }
            Key::Char(c @ ('u' | 'U' | '~' | '?')) => {
                let op = match c {
                    'u' => OperatorKind::Lower,
                    'U' => OperatorKind::Upper,
                    '~' => OperatorKind::ToggleCase,
                    _ => OperatorKind::Rot13,
                };
                if let Mode::Visual(kind) = ed.mode {
                    crate::visual::apply_to_selection(ed, op, kind);
                    ed.pending.reset();
                } else {
                    begin_operator(ed, op);
                }
            }
            Key::Char('J') if matches!(ed.mode, Mode::Visual(_)) => {
                ed.pending.reset();
                crate::visual::join_selection(ed, false);
            }
            Key::Char('J') => {
                join_lines_raw(ed);
                ed.pending.reset();
            }
            Key::Char('i') => {
                ed.pending.reset();
                resume_insert(ed);
            }
            Key::Char(';') => change_step(ed, true),
            Key::Char(',') => change_step(ed, false),
            Key::Char('&') => {
                ed.pending.reset();
                crate::command::run_ex(ed, "g&");
            }
            Key::Char('_') => apply_motion_or_operator(ed, Motion::LastNonBlank),
            Key::Char('m') => {
                let width = if ed.view_width == 0 {
                    80
                } else {
                    ed.view_width
                };
                apply_motion_or_operator(
                    ed,
                    Motion::LineMiddle {
                        of_text: false,
                        width,
                    },
                )
            }
            Key::Char('M') => apply_motion_or_operator(
                ed,
                Motion::LineMiddle {
                    of_text: true,
                    width: 0,
                },
            ),
            Key::Char('o') => {
                ed.push_jump();
                apply_motion_or_operator(ed, Motion::ByteOffset)
            }
            Key::Char('E') => {
                apply_motion_or_operator(ed, Motion::WordEndBack(true));
            }
            Key::Char('n') => select_match(ed, true),
            Key::Char('N') => select_match(ed, false),
            Key::Char('f') => {
                ed.pending.reset();
                goto_file(ed, false);
            }
            Key::Char('F') => {
                ed.pending.reset();
                goto_file(ed, true);
            }
            Key::Char('x') => {
                ed.pending.reset();
                open_external(ed);
            }
            Key::Char('8') => {
                ed.pending.reset();
                char_info(ed, true);
            }
            Key::Char('<') => {
                ed.pending.reset();
                ed.resume();
            }
            Key::Char('-') => {
                ed.pending.reset();
                crate::command::run_ex(ed, "undo");
            }
            Key::Char('+') => {
                ed.pending.reset();
                crate::command::run_ex(ed, "redo");
            }
            Key::Char('R') => {
                ed.pending.reset();
                ed.start_change_recording_seq(&[Key::Char('g'), Key::Char('R')]);
                begin_replace(ed);
            }
            Key::Char('Q') => {
                ed.pending.reset();
                ed.enter_command(CommandKind::Ex);
            }
            Key::Char('*') => search_word_under_cursor(ed, true, false),
            Key::Char('#') => search_word_under_cursor(ed, false, false),
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
            Key::Char('p') if ed.mode == crate::mode::Mode::Normal => {
                ed.pending.awaiting = Some(Awaiting::PeekPrefix);
            }
            Key::Char('P') => {
                ed.close_float();
                ed.pending.reset();
            }
            _ => ed.pending.reset(),
        },
        Awaiting::PeekPrefix => {
            let kind = match key {
                Key::Char('d') => Some("peekDefinition"),
                Key::Char('t') => Some("peekTypeDefinition"),
                Key::Char('i') => Some("peekImplementation"),
                Key::Char('r') => Some("peekReferences"),
                _ => None,
            };
            if let Some(kind) = kind {
                ed.request_language(kind, None);
            }
            ed.pending.reset();
        }
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
                        repeat: false,
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
                    if inner && matches!(kind, ObjectKind::Word(_)) && ed.buf().line_len(line) == 0
                    {
                        if ed.pending.operator == Some(OperatorKind::Change) {
                            apply_operator_motion(
                                ed,
                                OperatorKind::Change,
                                (line, col),
                                (line, col),
                                Span::Empty,
                            );
                        } else {
                            ed.abort_change_recording();
                        }
                        ed.pending.reset();
                        return;
                    }
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
                ed.sync_readonly_registers();
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
                if let Some(op) = ed.pending.operator {
                    // Operator to a mark: `d'a`/`c'a`/`y'a` (linewise) or the
                    // backtick forms (charwise, exclusive). Only within the
                    // current buffer; otherwise abort the pending operator.
                    match ed.marks.get(&c).cloned() {
                        Some(loc) if loc.buffer == ed.buf().id => {
                            let (line, col) = ed.cursor();
                            let tl = loc.line.min(ed.buf().line_count().saturating_sub(1));
                            if exact {
                                let tc = loc.col.min(ed.buf().line_len(tl));
                                apply_operator_motion(
                                    ed,
                                    op,
                                    (line, col),
                                    (tl, tc),
                                    Span::Exclusive,
                                );
                            } else {
                                apply_operator_motion(ed, op, (line, 0), (tl, 0), Span::Linewise);
                            }
                        }
                        _ => ed.abort_change_recording(),
                    }
                } else {
                    ed.jump_mark(c, exact);
                }
            } else if ed.pending.operator.is_some() {
                ed.abort_change_recording();
            }
            ed.pending.reset();
        }
        Awaiting::MacroRegister => {
            match key.as_char() {
                // `q:` is Vim's command-line window: the history picker.
                Some(':') => {
                    ed.pending.reset();
                    crate::command::run_ex(ed, "chistory");
                    return;
                }
                // Only writable registers can hold a macro (`q.`, `q%`
                // and the like are no-ops in Vim).
                Some(c) if c.is_ascii_alphanumeric() || c == '"' => {
                    ed.macro_recording = Some((c, Vec::new()));
                    ed.set_message(format!("recording @{}", c));
                }
                _ => {}
            }
            ed.pending.reset();
        }
        Awaiting::MacroReplay => {
            let reg = match key {
                Key::Char('@') => ed.last_macro_reg,
                _ => key.as_char(),
            };
            // `@:` repeats the last command-line command (and makes `@@` do
            // the same).
            if reg == Some(':') {
                ed.last_macro_reg = Some(':');
                let n = ed.pending.total_count();
                ed.pending.reset();
                match ed.command_history.last().cloned() {
                    Some(cmd) => {
                        for _ in 0..n.min(1000) {
                            crate::command::run_ex(ed, &cmd);
                        }
                    }
                    None => ed.set_message("E30: no previous command line"),
                }
                return;
            }
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
                Key::Char('R') | Key::Char('r') => ed.open_all_folds(),
                Key::Char('M') | Key::Char('m') => ed.close_all_folds(),
                Key::Char('O') => ed.open_fold(),
                Key::Char('C') => ed.close_fold(),
                Key::Char('A') => ed.toggle_fold(),
                Key::Char('v') | Key::Char('x') => reveal_cursor_folds(ed),
                Key::Char('E') => {
                    ed.buf_mut().folds.clear();
                    ed.set_message("All folds deleted");
                }
                Key::Char('D') => ed.delete_fold(),
                Key::Char('j') => fold_step(ed, true),
                Key::Char('k') => fold_step(ed, false),
                // `zf{motion}`: create a fold over the lines it covers.
                Key::Char('f') => {
                    begin_operator(ed, OperatorKind::Fold);
                    return;
                }
                // `zF`: a fold over `count` lines.
                Key::Char('F') => {
                    let line = ed.cursor().0;
                    let n = ed.pending.total_count();
                    let end = (line + n - 1).min(ed.buf().line_count().saturating_sub(1));
                    ed.create_fold(line, end);
                }
                // `z<Enter>` `z.` `z-`: like `zt` `zz` `zb`, cursor to the
                // first non-blank.
                Key::Enter => {
                    let line = ed.cursor().0;
                    ed.buf_mut().top_line = line;
                    let c = ed.buf().first_non_blank(line);
                    ed.set_cursor(line, c);
                }
                Key::Char('.') => {
                    recenter_viewport(ed);
                    let line = ed.cursor().0;
                    let c = ed.buf().first_non_blank(line);
                    ed.set_cursor(line, c);
                }
                Key::Char('-') => {
                    let line = ed.cursor().0;
                    let rows = ed.screen_rows.max(1);
                    ed.buf_mut().top_line = line.saturating_sub(rows.saturating_sub(1));
                    let c = ed.buf().first_non_blank(line);
                    ed.set_cursor(line, c);
                }
                // Horizontal scrolling (only with `nowrap`, as in Vim).
                Key::Char('l') | Key::Right => hscroll(ed, ed.pending.total_count() as isize),
                Key::Char('h') | Key::Left => hscroll(ed, -(ed.pending.total_count() as isize)),
                Key::Char('L') => hscroll(ed, (hwidth(ed) / 2) as isize),
                Key::Char('H') => hscroll(ed, -((hwidth(ed) / 2) as isize)),
                Key::Char('s') => hscroll_to_cursor(ed, true),
                Key::Char('e') => hscroll_to_cursor(ed, false),
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

// ---------------------------------------------------------------------
// Vim keys added for parity: helpers behind the bindings above.
// ---------------------------------------------------------------------

/// Runs a search motion (`n`, `N`, `*`, `#`) for the pending operator and
/// applies the operator from the cursor to the match (exclusive), like
/// `dn` or `y*`.
fn operator_to_search(ed: &mut Editor, search: impl FnOnce(&mut Editor)) {
    let Some(op) = ed.pending.operator else {
        return;
    };
    let register = ed.pending.register;
    let origin = ed.cursor();
    search(ed);
    let target = ed.cursor();
    ed.set_cursor(origin.0, origin.1);
    ed.pending.reset();
    if target == origin {
        ed.abort_change_recording();
        return;
    }
    ed.pending.register = register;
    apply_operator_motion(ed, op, origin, target, Span::Exclusive);
    ed.pending.reset();
}

/// Completes `d/pat`-style commands once the search has moved the cursor:
/// applies the pending operator from `origin` to the match (exclusive).
pub(crate) fn finish_search_operator(ed: &mut Editor, origin: (usize, usize)) {
    let Some(op) = ed.pending.operator else {
        return;
    };
    let target = ed.cursor();
    ed.set_cursor(origin.0, origin.1);
    if target == origin {
        ed.abort_change_recording();
        ed.pending.reset();
        return;
    }
    apply_operator_motion(ed, op, origin, target, Span::Exclusive);
    ed.pending.reset();
}

/// `U`: restore the last changed line to how it was before the latest
/// run of changes on it. `U` again undoes the `U`.
fn line_undo(ed: &mut Editor) {
    let Some((line, text)) = ed.buf().uline.clone() else {
        ed.set_message("No line to undo");
        return;
    };
    if line >= ed.buf().line_count() {
        return;
    }
    let current = ed.buf().line_text(line);
    if current == text {
        return;
    }
    ed.set_cursor(line, 0);
    ed.buf_mut().begin_edit();
    let start = ed.buf().char_idx(line, 0);
    let end = ed.buf().char_idx(line, ed.buf().line_len(line));
    ed.buf_mut().delete_char_range(start, end);
    ed.buf_mut().insert_str_at(start, &text);
    ed.buf_mut().commit_edit();
    ed.buf_mut().uline = Some((line, current));
    let col = ed.buf().first_non_blank(line);
    ed.set_cursor(line, col);
}

/// Neovim's `Q`: replay the most recently recorded macro `n` times.
fn replay_last_recorded(ed: &mut Editor, n: usize) {
    let Some(reg) = ed.last_recorded_macro else {
        ed.set_message("No macro recorded yet");
        return;
    };
    let Some(keys) = ed.macros.get(&reg).cloned() else {
        return;
    };
    for _ in 0..n.min(1000) {
        ed.replay(&keys);
        if ed.replay_budget == 0 {
            break;
        }
    }
}

/// `R` / `gR`: enter Replace mode at the cursor.
fn begin_replace(ed: &mut Editor) {
    let (l, c) = ed.cursor();
    ed.insert_repeat = 1;
    ed.insert_start = ed.buf().char_idx(l, c);
    ed.buf_mut().begin_edit_at((l, c));
    ed.replace_mode = Some(Vec::new());
    ed.set_cursor_insert(l, c);
    ed.enter_insert();
    ed.pending.reset();
}

pub(crate) fn show_file_info(ed: &mut Editor) {
    file_info(ed);
}

/// `Ctrl-G`: file name, modified flag, line count and position.
fn file_info(ed: &mut Editor) {
    let name = match &ed.buf().path {
        Some(p) => p
            .strip_prefix(&ed.project_root)
            .unwrap_or(p)
            .display()
            .to_string(),
        None => "[No Name]".into(),
    };
    let lines = ed.buf().line_count();
    let line = ed.cursor().0 + 1;
    let pct = (line * 100).checked_div(lines).unwrap_or(0);
    let modified = if ed.buf().is_modified() {
        " [Modified]"
    } else {
        ""
    };
    ed.set_message(format!(
        "\"{name}\"{modified} {lines} line{} --{pct}%-- line {line} of {lines}",
        if lines == 1 { "" } else { "s" }
    ));
}

/// `Ctrl-T`: back to where the last `Ctrl-]` was pressed.
fn tag_pop(ed: &mut Editor) {
    let Some((id, line, col)) = ed.tag_stack.pop() else {
        ed.set_message("E73: Tag stack empty");
        return;
    };
    let Some(i) = ed.buffers.iter().position(|b| b.id == id) else {
        ed.set_message("E73: Tag stack empty");
        return;
    };
    if i != ed.cur {
        ed.note_alternate_buffer();
        ed.cur = i;
        ed.touch_buffer_mru(id);
    }
    let line = line.min(ed.buf().line_count().saturating_sub(1));
    ed.set_cursor(line, col.min(ed.buf().line_len(line)));
}

/// `[*` / `]*`: the start of the C-style comment before the cursor, or the
/// end of the one after it.
fn comment_bound(ed: &mut Editor, forward: bool) {
    let (line, col) = ed.cursor();
    let text = ed.buf().rope.to_string();
    let idx = ed.buf().char_idx(line, col);
    let byte = ed.buf().rope.char_to_byte(idx);
    let found = if forward {
        text.get(byte + 1..)
            .and_then(|t| t.find("*/"))
            .map(|b| byte + 1 + b + 1)
    } else {
        text.get(..byte).and_then(|t| t.rfind("/*"))
    };
    match found {
        Some(b) => {
            ed.push_jump();
            let (l, c) = ed.buf().pos_from_char_idx(ed.buf().rope.byte_to_char(b));
            ed.set_cursor(l, c);
        }
        None => ed.set_message("No comment found"),
    }
}

/// `]p` / `[p` (and `]P`): paste a linewise register with its indent
/// changed to match the current line; charwise text pastes as `p`/`P`.
fn paste_indented(ed: &mut Editor, before: bool) {
    let reg = ed.pending.register;
    let Some(entry) = ed.registers.get(reg).cloned() else {
        return;
    };
    if !entry.linewise {
        do_paste(ed, Key::Char(if before { 'P' } else { 'p' }), !before);
        return;
    }
    let line = ed.cursor().0;
    let target = leading_ws(&ed.buf().line_text(line));
    let lines: Vec<&str> = entry.text.trim_end_matches('\n').split('\n').collect();
    let common = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').count())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for l in &lines {
        if l.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&target);
            out.extend(l.chars().skip(common));
            out.push('\n');
        }
    }
    let count = ed.pending.total_count();
    let out = out.repeat(count.min(1000));
    ed.start_change_recording(Key::Char(if before { '[' } else { ']' }));
    ed.buf_mut().begin_edit();
    let at_line = if before { line } else { line + 1 };
    let at = if at_line < ed.buf().line_count() {
        ed.buf().char_idx(at_line, 0)
    } else {
        // Pasting below the last line: start a new line first.
        let end = ed.buf().rope.len_chars();
        if !ed.buf().rope.to_string().ends_with('\n') {
            ed.buf_mut().insert_str_at(end, "\n");
        }
        ed.buf().rope.len_chars()
    };
    ed.buf_mut().insert_str_at(at, &out);
    ed.buf_mut().commit_edit();
    let (l, _) = ed.buf().pos_from_char_idx(at);
    let c = ed.buf().first_non_blank(l);
    ed.set_cursor(l, c);
    ed.pending.reset();
    ed.finish_change_recording();
}

/// `]'` / `['` (line) and `` ]` `` / `` [` `` (exact): the next / previous
/// lowercase mark in this buffer.
fn mark_step(ed: &mut Editor, forward: bool, exact: bool) {
    let id = ed.buf().id;
    let (line, col) = ed.cursor();
    let mut marks: Vec<(usize, usize)> = ed
        .marks
        .iter()
        .filter(|(c, loc)| c.is_ascii_lowercase() && loc.buffer == id)
        .map(|(_, loc)| (loc.line, loc.col))
        .collect();
    marks.sort_unstable();
    let here = if exact {
        (line, col)
    } else {
        (line, usize::MAX)
    };
    let found = if forward {
        marks
            .into_iter()
            .find(|m| if exact { *m > here } else { m.0 > line })
    } else {
        marks
            .into_iter()
            .rev()
            .find(|m| if exact { *m < (line, col) } else { m.0 < line })
    };
    match found {
        Some((l, c)) => {
            let l = l.min(ed.buf().line_count().saturating_sub(1));
            let c = if exact {
                c.min(ed.buf().line_len(l))
            } else {
                ed.buf().first_non_blank(l)
            };
            ed.set_cursor(l, c);
        }
        None => ed.set_message("No more marks"),
    }
}

/// `gJ`: join lines without inserting or removing spaces.
fn join_lines_raw(ed: &mut Editor) {
    ed.start_change_recording_seq(&[Key::Char('g'), Key::Char('J')]);
    let line = ed.cursor().0;
    let n = ed.pending.total_count().max(2);
    let last = (line + n - 1).min(ed.buf().line_count().saturating_sub(1));
    if last == line {
        ed.abort_change_recording();
        return;
    }
    ed.buf_mut().begin_edit();
    let mut join_col = 0;
    for _ in line..last {
        let len = ed.buf().line_len(line);
        join_col = len;
        let end = ed.buf().char_idx(line, len);
        ed.buf_mut().delete_char_range(end, end + 1);
    }
    ed.buf_mut().commit_edit();
    let col = join_col.min(ed.buf().line_len(line).saturating_sub(1));
    ed.set_cursor(line, col);
    ed.finish_change_recording();
}

/// `gi`: Insert where Insert mode was last left in this buffer.
fn resume_insert(ed: &mut Editor) {
    let id = ed.buf().id;
    let pos = ed
        .last_insert_pos
        .filter(|(b, _, _)| *b == id)
        .map(|(_, l, c)| (l, c));
    match pos {
        Some((l, c)) => {
            let l = l.min(ed.buf().line_count().saturating_sub(1));
            let c = c.min(ed.buf().line_len(l));
            begin_insert(ed, Key::Char('i'), move |_| (l, c));
        }
        None => begin_insert(ed, Key::Char('i'), |ed| ed.cursor()),
    }
}

/// `g;` (older) / `g,` (newer): step through this buffer's change list.
fn change_step(ed: &mut Editor, older: bool) {
    let count = ed.pending.total_count();
    ed.pending.reset();
    let len = ed.buf().changelist.len();
    if len == 0 {
        ed.set_message("E664: Changelist is empty");
        return;
    }
    let idx = ed.buf().change_idx.unwrap_or(len);
    let next = if older {
        if idx == 0 {
            ed.set_message("E662: At start of changelist");
            return;
        }
        idx.saturating_sub(count)
    } else {
        if idx + 1 >= len {
            ed.set_message("E663: At end of changelist");
            return;
        }
        (idx + count).min(len - 1)
    };
    let (l, c) = ed.buf().changelist[next];
    ed.buf_mut().change_idx = Some(next);
    let l = l.min(ed.buf().line_count().saturating_sub(1));
    let c = ed.buf().clamp_col_normal(l, c);
    ed.set_cursor(l, c);
}

/// The match of the last search under or after (`gn`) / before (`gN`) the
/// cursor, as an inclusive char range.
fn match_range(ed: &mut Editor, forward: bool) -> Option<(usize, usize)> {
    let (pattern, _) = ed.last_search.clone()?;
    let ic = ed.config.ignorecase;
    let sc = ed.config.smartcase;
    let re = crate::search::compile(&pattern, ic, sc).ok()?;
    let text = ed.buf().rope.to_string();
    let (line, col) = ed.cursor();
    let idx = ed.buf().char_idx(line, col);
    let rope = &ed.buf().rope;
    let ranges: Vec<(usize, usize)> = re
        .find_iter(&text)
        .filter_map(|m| m.ok())
        .filter(|m| m.end() > m.start())
        .map(|m| (rope.byte_to_char(m.start()), rope.byte_to_char(m.end()) - 1))
        .collect();
    let found = if forward {
        ranges
            .iter()
            .copied()
            .find(|&(_, e)| e >= idx)
            .or_else(|| ranges.first().copied())
    } else {
        ranges
            .iter()
            .copied()
            .rev()
            .find(|&(s, _)| s <= idx)
            .or_else(|| ranges.last().copied())
    };
    found
}

/// `gn` / `gN`: select the next / previous match of the last search, or
/// with an operator pending (`cgn`, `dgn`) apply it to that match.
fn select_match(ed: &mut Editor, forward: bool) {
    let Some((start, end)) = match_range(ed, forward) else {
        if ed.pending.operator.is_some() {
            ed.abort_change_recording();
        }
        ed.pending.reset();
        ed.set_message("No match for the last search");
        return;
    };
    let s = ed.buf().pos_from_char_idx(start);
    let e = ed.buf().pos_from_char_idx(end);
    if let Some(op) = ed.pending.operator {
        apply_operator_motion(ed, op, s, e, Span::Inclusive);
        ed.pending.reset();
        return;
    }
    ed.pending.reset();
    let (anchor, cursor) = if forward { (s, e) } else { (e, s) };
    if !matches!(ed.mode, Mode::Visual(_)) {
        ed.set_cursor(anchor.0, anchor.1);
        ed.enter_visual(VisualKind::Char);
    }
    ed.set_cursor(cursor.0, cursor.1);
}

/// The file name under the cursor (Vim's `isfname` characters) and the
/// line number after it (`name:12`, `name(12)`, `name line 12`), if any.
pub(crate) fn file_under_cursor(line: &str, col: usize) -> Option<(String, Option<usize>)> {
    let chars: Vec<char> = line.chars().collect();
    let is_fname = |c: char| c.is_alphanumeric() || "/._+,#$%~=-@".contains(c);
    let mut at = col.min(chars.len().saturating_sub(1));
    if chars.is_empty() {
        return None;
    }
    if !is_fname(chars[at]) {
        at = (at..chars.len()).find(|&i| is_fname(chars[i]))?;
    }
    let mut start = at;
    while start > 0 && is_fname(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at;
    while end < chars.len() && is_fname(chars[end]) {
        end += 1;
    }
    let name: String = chars[start..end].iter().collect();
    let name = name.trim_end_matches(['.', ',']).to_string();
    let rest: String = chars[end..].iter().collect();
    let digits = |s: &str| -> Option<usize> {
        let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        d.parse().ok()
    };
    let line_no = rest
        .strip_prefix(':')
        .and_then(digits)
        .or_else(|| rest.strip_prefix('(').and_then(digits))
        .or_else(|| rest.trim_start().strip_prefix("line ").and_then(digits));
    (!name.is_empty()).then_some((name, line_no))
}

/// `gf` / `gF` (and `Ctrl-W f`): open the file under the cursor, looked up
/// as given, next to the current file, then in the project; `gF` also
/// jumps to the line number after the name.
pub(crate) fn goto_file(ed: &mut Editor, with_line: bool) -> bool {
    let (line, col) = ed.cursor();
    let Some((name, line_no)) = file_under_cursor(&ed.buf().line_text(line), col) else {
        ed.set_message("E446: No file name under cursor");
        return false;
    };
    let expanded = match name.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| std::path::PathBuf::from(&name)),
        None => std::path::PathBuf::from(&name),
    };
    let mut candidates = vec![expanded.clone()];
    if expanded.is_relative() {
        if let Some(dir) = ed.buf().path.as_ref().and_then(|p| p.parent()) {
            candidates.insert(0, dir.join(&expanded));
        }
        candidates.push(ed.project_root.join(&expanded));
    }
    let Some(path) = candidates.into_iter().find(|p| p.is_file()) else {
        ed.set_message(format!("E447: Can't find file \"{name}\" in path"));
        return false;
    };
    ed.push_jump();
    if let Err(e) = ed.open_file(path) {
        ed.set_message(e.to_string());
        return false;
    }
    if with_line {
        if let Some(n) = line_no {
            let l = n
                .saturating_sub(1)
                .min(ed.buf().line_count().saturating_sub(1));
            let c = ed.buf().first_non_blank(l);
            ed.set_cursor(l, c);
        }
    }
    true
}

/// `gx`: open the URL or file under the cursor with the system opener.
fn open_external(ed: &mut Editor) {
    let (line, col) = ed.cursor();
    let chars: Vec<char> = ed.buf().line_text(line).chars().collect();
    if chars.is_empty() {
        return;
    }
    let col = col.min(chars.len() - 1);
    let mut start = col;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let raw: String = chars[start..end].iter().collect();
    // Markdown links `[text](url)` and surrounding punctuation.
    let raw = raw
        .rsplit_once("](")
        .map(|(_, u)| u.to_string())
        .unwrap_or(raw);
    let target = raw
        .trim_matches(|c: char| "<>()[]{}\"'`,;".contains(c))
        .trim_end_matches('.')
        .to_string();
    let is_url = target.contains("://") || target.starts_with("mailto:");
    let path = ed
        .buf()
        .path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join(&target))
        .filter(|p| p.exists());
    if target.is_empty() || (!is_url && path.is_none()) {
        ed.set_message("No URL or file under cursor");
        return;
    }
    let arg = if is_url {
        target.clone()
    } else {
        path.unwrap().display().to_string()
    };
    open_with_system(ed, &arg);
}

/// Opens `arg` (a URL or path) with the system opener (`xdg-open`/`open`)
/// in the background.
pub(crate) fn open_with_system(ed: &mut Editor, arg: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(opener)
        .arg(arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            ed.set_message(format!("Opened {arg}"));
        }
        Err(e) => ed.set_message(format!("{opener}: {e}")),
    }
}

/// `g8` (UTF-8 bytes in hex) and `ga` / `:ascii` (decimal, hex, octal) of
/// the character under the cursor.
pub(crate) fn char_info(ed: &mut Editor, utf8: bool) {
    let (line, col) = ed.cursor();
    let Some(c) = ed.buf().line_text(line).chars().nth(col) else {
        ed.set_message("NUL");
        return;
    };
    if utf8 {
        let mut b = [0u8; 4];
        let hex: Vec<String> = c
            .encode_utf8(&mut b)
            .bytes()
            .map(|x| format!("{x:02x}"))
            .collect();
        ed.set_message(hex.join(" "));
    } else {
        let n = c as u32;
        ed.set_message(format!("<{c}> {n}, Hex {n:02x}, Oct {n:03o}, U+{n:04X}"));
    }
}

/// `zv`: open every fold that hides the cursor line.
fn reveal_cursor_folds(ed: &mut Editor) {
    let line = ed.cursor().0;
    for f in ed.buf_mut().folds.iter_mut() {
        if f.start <= line && line <= f.end {
            f.closed = false;
        }
    }
}

/// `zj`: the start of the next fold; `zk`: the end of the previous one.
fn fold_step(ed: &mut Editor, down: bool) {
    let line = ed.cursor().0;
    let folds = &ed.buf().folds;
    let target = if down {
        folds.iter().map(|f| f.start).filter(|&s| s > line).min()
    } else {
        folds.iter().map(|f| f.end).filter(|&e| e < line).max()
    };
    match target {
        Some(l) => {
            let c = ed.buf().first_non_blank(l);
            ed.set_cursor(l, c);
        }
        None => ed.set_message("No more folds"),
    }
}

fn hwidth(ed: &Editor) -> usize {
    if ed.view_width == 0 {
        80
    } else {
        ed.view_width
    }
}

/// `zl` / `zh` / `zL` / `zH`: scroll the view sideways by `delta` columns
/// (`nowrap` only), keeping the cursor on screen like Vim.
pub(crate) fn hscroll(ed: &mut Editor, delta: isize) {
    if ed.config.wrap {
        return;
    }
    let width = hwidth(ed);
    let left = (ed.buf().left_col as isize + delta).max(0) as usize;
    ed.buf_mut().left_col = left;
    let (line, col) = ed.cursor();
    let text = ed.buf().line_text(line);
    let tab = ed.buf().tabstop;
    let cell = crate::grapheme::cell(&text, col, tab);
    let max = ed.buf().line_len(line).saturating_sub(1);
    if cell < left {
        let c = crate::grapheme::raw_column(&text, left, tab).min(max);
        ed.set_cursor(line, c);
    } else if cell >= left + width {
        let c = crate::grapheme::raw_column(&text, left + width - 1, tab).min(max);
        ed.set_cursor(line, c);
    }
    ed.buf_mut().left_col = left;
    ed.store_window();
}

/// `zs` / `ze`: scroll sideways so the cursor is at the left / right edge.
fn hscroll_to_cursor(ed: &mut Editor, at_start: bool) {
    if ed.config.wrap {
        return;
    }
    let (line, col) = ed.cursor();
    let text = ed.buf().line_text(line);
    let cell = crate::grapheme::cell(&text, col, ed.buf().tabstop);
    let left = if at_start {
        cell
    } else {
        (cell + 1).saturating_sub(hwidth(ed))
    };
    ed.buf_mut().left_col = left;
    ed.store_window();
}
