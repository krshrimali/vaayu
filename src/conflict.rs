//! Git merge-conflict resolution — the practical form of a 3-way merge in the
//! editor. Detects conflict blocks written by git:
//!
//! ```text
//! <<<<<<< HEAD
//! ours
//! ||||||| base        (optional, diff3 style)
//! base
//! =======
//! theirs
//! >>>>>>> branch
//! ```
//!
//! and offers keep-ours / keep-theirs / keep-both resolution plus next/prev
//! navigation. Line-oriented and edit-based, so it touches no core engine path.

use crate::editor::Editor;

/// Which side(s) of a conflict to keep when resolving.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Ours,
    Theirs,
    Both,
}

/// One parsed conflict block, all 0-based line numbers. `base` is the optional
/// diff3 `|||||||` marker line; the base section (if any) is always dropped on
/// resolve (it's the common ancestor, never a resolution choice).
pub(crate) struct Conflict {
    pub start: usize,
    pub base: Option<usize>,
    pub sep: usize,
    pub end: usize,
}

impl Editor {
    /// Scan forward from line `from` for the next well-formed conflict block.
    fn find_conflict_from(&self, from: usize) -> Option<Conflict> {
        let b = self.buf();
        let n = b.line_count();
        let mut i = from;
        while i < n {
            if b.line_text(i).starts_with("<<<<<<<") {
                let start = i;
                let (mut base, mut sep) = (None, None);
                let mut j = start + 1;
                while j < n {
                    let t = b.line_text(j);
                    if t.starts_with("<<<<<<<") {
                        break; // a new start before this one closed — malformed
                    } else if base.is_none() && sep.is_none() && t.starts_with("|||||||") {
                        base = Some(j);
                    } else if sep.is_none() && t.starts_with("=======") {
                        sep = Some(j);
                    } else if t.starts_with(">>>>>>>") {
                        if let Some(sep) = sep {
                            return Some(Conflict {
                                start,
                                base,
                                sep,
                                end: j,
                            });
                        }
                        break;
                    }
                    j += 1;
                }
                i = start + 1; // malformed block: resume scanning past its start
                continue;
            }
            i += 1;
        }
        None
    }

    /// The conflict block the cursor line is inside, if any.
    fn conflict_containing_cursor(&self) -> Option<Conflict> {
        let line = self.cursor().0;
        let mut from = 0;
        while let Some(c) = self.find_conflict_from(from) {
            if c.start <= line && line <= c.end {
                return Some(c);
            }
            if c.start > line {
                break; // blocks are ordered; none contains the cursor
            }
            from = c.end + 1;
        }
        None
    }

    /// `:conflictours`/`:conflicttheirs`/`:conflictboth` — resolve the conflict
    /// under the cursor by keeping the chosen side(s) and removing the markers.
    pub fn resolve_conflict(&mut self, keep: Keep) {
        let Some(c) = self.conflict_containing_cursor() else {
            self.set_message("Not inside a merge conflict (<<<<<<< … >>>>>>>)");
            return;
        };
        let b = self.buf();
        let ours: Vec<String> = ((c.start + 1)..c.base.unwrap_or(c.sep))
            .map(|l| b.line_text(l))
            .collect();
        let theirs: Vec<String> = ((c.sep + 1)..c.end).map(|l| b.line_text(l)).collect();
        let kept: Vec<String> = match keep {
            Keep::Ours => ours,
            Keep::Theirs => theirs,
            Keep::Both => ours.into_iter().chain(theirs).collect(),
        };
        // Replace the whole block (lines start..=end, including end's newline).
        let start_char = b.char_idx(c.start, 0);
        let end_char = if c.end + 1 < b.line_count() {
            b.char_idx(c.end + 1, 0)
        } else {
            b.rope.len_chars()
        };
        let replacement = if kept.is_empty() {
            String::new()
        } else {
            format!("{}\n", kept.join("\n"))
        };
        self.buf_mut().begin_edit();
        self.buf_mut().delete_char_range(start_char, end_char);
        self.buf_mut().insert_str_at(start_char, &replacement);
        self.buf_mut().commit_edit();
        let line = c.start.min(self.buf().line_count().saturating_sub(1));
        self.set_cursor(line, 0);
        let label = match keep {
            Keep::Ours => "ours",
            Keep::Theirs => "theirs",
            Keep::Both => "both",
        };
        self.set_message(format!("Conflict resolved (kept {label})"));
    }

    /// `:conflictnext`/`:conflictprev` — jump to the next/previous conflict's
    /// `<<<<<<<` marker (from the cursor line, not wrapping).
    pub fn goto_conflict(&mut self, forward: bool) {
        let line = self.cursor().0;
        let target = if forward {
            self.find_conflict_from(line + 1).map(|c| c.start)
        } else {
            // Last block that starts strictly before the cursor line.
            let mut prev = None;
            let mut from = 0;
            while let Some(c) = self.find_conflict_from(from) {
                if c.start >= line {
                    break;
                }
                prev = Some(c.start);
                from = c.end + 1;
            }
            prev
        };
        match target {
            Some(l) => self.set_cursor(l, 0),
            None => self.set_message("No more merge conflicts"),
        }
    }
}
