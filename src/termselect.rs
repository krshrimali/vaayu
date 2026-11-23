//! Selecting text in terminal panes with the mouse: drag to select and copy
//! (to the `+` register, so the system clipboard and the unnamed register),
//! double-click to copy a word, triple-click a line. The editor takes the
//! mouse, so the host terminal's own selection cannot reach these panes.
//!
//! A press that is released without moving is a click: it opens the link
//! under it (`crate::termlinks`), so a drag that starts on a path selects the
//! path instead of opening it.
use crate::termlinks::Link;

/// A selection in terminal `terminal`, in cells of its program area as
/// `(row, col)`; both ends inclusive. Drawn only while the view is still
/// scrolled `offset` lines into history, as it was when made.
#[derive(Clone, Debug)]
pub struct TermSelect {
    pub terminal: u64,
    pub pane: usize,
    pub offset: usize,
    pub anchor: (u16, u16),
    pub head: (u16, u16),
    /// The button is still down and drags extend the selection.
    pub dragging: bool,
    /// The link under the press, opened on release if the pointer did not
    /// move.
    pub link: Option<Link>,
}

impl TermSelect {
    /// The selected cells, first to last, or `None` while a press has not
    /// moved.
    pub fn span(&self) -> Option<Span> {
        if self.dragging && self.anchor == self.head {
            return None;
        }
        Some(if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        })
    }
}

/// A selection's first and last cell, `(row, col)` each.
pub type Span = ((u16, u16), (u16, u16));

/// Whether `(row, col)` lies in the stream selection `span` (whole rows
/// between its ends, as terminals select).
pub fn span_contains(span: Span, row: u16, col: u16) -> bool {
    let ((r0, c0), (r1, c1)) = span;
    (r0..=r1).contains(&row) && (row != r0 || col >= c0) && (row != r1 || col <= c1)
}

/// Characters a double-click word stops at besides whitespace: box drawing
/// and the frames agents draw around their output.
fn word_break(c: char) -> bool {
    c.is_whitespace() || "│┃|╭╮╰╯─━┌┐└┘⎿├┤═║\"'`()[]{}<>,;".contains(c)
}

impl crate::pty::PtySession {
    /// The text of the stream selection `start..=end` (view cells): rows the
    /// program soft-wrapped run on without a newline, trailing blanks of
    /// each row are dropped.
    pub fn selection_text(&self, start: (u16, u16), end: (u16, u16)) -> String {
        self.with_screen(|screen| {
            let (_, cols) = screen.size();
            let mut out = String::new();
            for r in start.0..=end.0 {
                let from = if r == start.0 { start.1 } else { 0 };
                let to = if r == end.0 {
                    end.1.min(cols.saturating_sub(1))
                } else {
                    cols.saturating_sub(1)
                };
                let mut row = String::new();
                for c in from..=to {
                    match screen.cell(r, c) {
                        Some(cell) if cell.is_wide_continuation() => {}
                        Some(cell) if !cell.contents().is_empty() => row.push_str(cell.contents()),
                        _ => row.push(' '),
                    }
                }
                let wrapped = r < end.0 && screen.row_wrapped(r);
                if wrapped {
                    out.push_str(&row);
                } else {
                    out.push_str(row.trim_end());
                    if r < end.0 {
                        out.push('\n');
                    }
                }
            }
            out
        })
    }

    /// The word around view cell `(row, col)`: its first and last cell.
    pub fn word_span(&self, row: u16, col: u16) -> Option<((u16, u16), (u16, u16))> {
        self.with_screen(|screen| {
            let (_, cols) = screen.size();
            let ch = |c: u16| {
                screen
                    .cell(row, c)
                    .and_then(|cell| cell.contents().chars().next())
                    .unwrap_or(' ')
            };
            if word_break(ch(col)) {
                return None;
            }
            let mut s = col;
            while s > 0 && !word_break(ch(s - 1)) {
                s -= 1;
            }
            let mut e = col;
            while e + 1 < cols && !word_break(ch(e + 1)) {
                e += 1;
            }
            // A sentence's full stop or a trailing colon is not part of it.
            while e > s && ".,:;!?".contains(ch(e)) {
                e -= 1;
            }
            Some(((row, s), (row, e)))
        })
    }

    /// The logical line around view row `row` (soft-wrapped rows joined).
    pub fn line_span(&self, row: u16) -> ((u16, u16), (u16, u16)) {
        self.with_screen(|screen| {
            let (rows, cols) = screen.size();
            let mut first = row;
            while first > 0 && screen.row_wrapped(first - 1) {
                first -= 1;
            }
            let mut last = row;
            while last + 1 < rows && screen.row_wrapped(last) {
                last += 1;
            }
            ((first, 0), (last, cols.saturating_sub(1)))
        })
    }
}

impl crate::editor::Editor {
    /// Whether the mouse selects text in terminal pane `pane` rather than
    /// going to its program: always in agent panes, and in any terminal
    /// whose program did not ask for mouse reports.
    pub(crate) fn terminal_selects(&self, pane: usize) -> bool {
        self.windows
            .get(pane)
            .and_then(|w| w.terminal)
            .and_then(|id| self.terminals.iter().find(|p| p.id == id))
            .is_some_and(|p| p.agent_kind.is_some() || !p.mouse_state().0)
    }

    /// A press at view cell `(row, col)` of terminal pane `pane`: starts a
    /// selection, or on a double/triple click selects and copies a word or a
    /// line. `link` is the link under the press, opened if it is released
    /// without moving.
    pub(crate) fn term_select_down(&mut self, pane: usize, row: u16, col: u16, link: Option<Link>) {
        let Some(id) = self.windows.get(pane).and_then(|w| w.terminal) else {
            return;
        };
        let Some(pty) = self.terminals.iter().find(|p| p.id == id) else {
            return;
        };
        let now = std::time::Instant::now();
        let count = match self.term_last_click {
            Some(c)
                if c.y == row as usize
                    && c.x == col as usize
                    && now.duration_since(c.at) < crate::mouse::MULTI_CLICK =>
            {
                c.count % 3 + 1
            }
            _ => 1,
        };
        self.term_last_click = Some(crate::mouse::ChromeClick {
            at: now,
            y: row as usize,
            x: col as usize,
            count,
        });
        let span = match count {
            2 => pty.word_span(row, col),
            3 => Some(pty.line_span(row)),
            _ => None,
        };
        let offset = pty.scroll_offset();
        self.term_select = Some(match span {
            Some((a, b)) => TermSelect {
                terminal: id,
                pane,
                offset,
                anchor: a,
                head: b,
                dragging: false,
                link: None,
            },
            None => TermSelect {
                terminal: id,
                pane,
                offset,
                anchor: (row, col),
                head: (row, col),
                dragging: true,
                link: if count == 1 { link } else { None },
            },
        });
        if span.is_some() {
            self.copy_term_selection();
        }
    }

    /// A drag to view cell `(row, col)` (already clamped to the pane)
    /// extends the selection; once it moves the press is no longer a click.
    pub(crate) fn term_select_drag(&mut self, row: u16, col: u16) {
        if let Some(s) = self.term_select.as_mut().filter(|s| s.dragging) {
            s.head = (row, col);
            if s.head != s.anchor {
                s.link = None;
            }
        }
    }

    /// The button is released: copies a dragged selection, or opens the link
    /// a click (no movement) landed on. Returns whether a selection was
    /// being made.
    pub(crate) fn term_select_up(&mut self, cols: usize, rows: usize) -> bool {
        let Some(s) = self.term_select.as_mut().filter(|s| s.dragging) else {
            return false;
        };
        s.dragging = false;
        if s.anchor == s.head {
            let (pane, link) = (s.pane, s.link.take());
            self.term_select = None;
            if let Some(link) = link {
                self.terminal_hover = None;
                self.open_terminal_link(pane, link, cols, rows);
            }
            return true;
        }
        self.copy_term_selection();
        true
    }

    /// Copies the selection to the `+` register (and so the clipboard and
    /// the unnamed register).
    fn copy_term_selection(&mut self) {
        let Some(sel) = self.term_select.clone() else {
            return;
        };
        let Some((a, b)) = sel.span() else {
            return;
        };
        let Some(pty) = self.terminals.iter().find(|p| p.id == sel.terminal) else {
            return;
        };
        let text = pty.selection_text(a, b);
        if text.trim().is_empty() {
            self.term_select = None;
            return;
        }
        let n = text.chars().count();
        let lines = text.lines().count();
        self.registers.set(Some('+'), text, false);
        self.set_message(if lines > 1 {
            format!("Copied {lines} lines ({n} chars)")
        } else {
            format!("Copied {n} char{}", if n == 1 { "" } else { "s" })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_orders_ends_and_contains_whole_middle_rows() {
        let s = TermSelect {
            terminal: 1,
            pane: 0,
            offset: 0,
            anchor: (3, 4),
            head: (1, 6),
            dragging: false,
            link: None,
        };
        let span = s.span().unwrap();
        assert_eq!(span, ((1, 6), (3, 4)));
        assert!(!span_contains(span, 1, 5));
        assert!(span_contains(span, 1, 6));
        assert!(span_contains(span, 2, 0));
        assert!(span_contains(span, 2, 200));
        assert!(span_contains(span, 3, 4));
        assert!(!span_contains(span, 3, 5));
        let pressed = TermSelect {
            dragging: true,
            head: (3, 4),
            ..s
        };
        assert_eq!(
            pressed.span(),
            None,
            "a press that has not moved selects nothing"
        );
    }
}
