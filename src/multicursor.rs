//! Multiple cursors (vim-visual-multi style, cursors only -- no
//! per-cursor selections yet).
//!
//! The buffer's own cursor stays the *primary* one: the view follows it and
//! everything that isn't an edit (`:` commands, pickers, Visual mode, leader
//! actions) runs there alone. The *secondary* cursors live here as rope char
//! indices. A Normal-mode command is run on the primary key by key as usual
//! and, once it completes, its keys are replayed at every secondary; an
//! Insert-mode key is run at every cursor. While cursors are active the
//! buffer records each edit (`Buffer::set_edit_log`), and after every run the
//! other cursors are mapped through what that run changed, so a cursor is
//! never left pointing into text another cursor rewrote. All the undo steps
//! one key/command produces across the cursors are squashed into one.
//!
//! Adding cursors: `Ctrl-N` (next occurrence of the word under the cursor, or
//! of the Visual selection), Visual-block/-line `Ctrl-N` (one cursor per
//! line), `,ma` (every occurrence), `,mj`/`,mk` (a cursor on the line
//! below/above). `Esc` in Normal mode (or `,mc`) collapses to the primary;
//! `u`/`Ctrl-R` collapse first and then undo/redo as usual.

use std::time::Instant;

use crate::editor::Editor;
use crate::key::Key;
use crate::mode::{Mode, VisualKind};
use crate::normal::Awaiting;

/// One cursor: its rope char index, its sticky column for `j`/`k`
/// (`Buffer::desired_col`), and the Insert session's start
/// (`Editor::insert_start`, which count-repeated inserts read back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub idx: usize,
    pub want: usize,
    pub anchor: usize,
}

pub struct MultiCursor {
    /// The buffer the cursors index; switching buffers collapses them.
    pub buffer: u64,
    /// Secondary cursors, sorted by `idx`, never at the primary's position.
    pub cursors: Vec<Cursor>,
    /// What `Ctrl-N`/`,ma` look for: literal text, and whether only
    /// whole-word occurrences count (a word under the cursor) or any (a
    /// Visual selection).
    pattern: Option<(String, bool)>,
    /// Keys of the Normal-mode command in flight on the primary, replayed at
    /// every secondary once the command completes.
    keys: Vec<Key>,
    /// Undo depth before `keys` started, for squashing.
    undo_len: usize,
    /// The buffer's `edit_seq` as of the last sync plus the edits drained
    /// from its log since: anything else moving `edit_seq` (undo, reload, an
    /// edit that bypassed the log) means the positions can't be trusted.
    seq: u64,
    /// Set while a command is being replayed, so the keys that command
    /// feeds back in (`.`, `@q`, remaps) run at the current cursor only.
    busy: bool,
}

/// Editor state one cursor's run of a key may change, reset before every
/// cursor replays it so each starts from the same place.
#[derive(Clone)]
struct RunState {
    mode: Mode,
    pending_jk: Option<Instant>,
    insert_repeat: usize,
    insert_open_bof: bool,
    block_insert: Option<crate::visual::BlockInsert>,
}

fn snapshot(ed: &Editor) -> RunState {
    RunState {
        mode: ed.mode,
        pending_jk: ed.pending_jk,
        insert_repeat: ed.insert_repeat,
        insert_open_bof: ed.insert_open_bof,
        block_insert: ed.block_insert,
    }
}

fn restore(ed: &mut Editor, s: &RunState) {
    ed.mode = s.mode;
    ed.pending_jk = s.pending_jk;
    ed.insert_repeat = s.insert_repeat;
    ed.insert_open_bof = s.insert_open_bof;
    ed.block_insert = s.block_insert;
}

fn capture(ed: &Editor) -> Cursor {
    let b = ed.buf();
    Cursor {
        idx: b.char_idx(b.cursor_line, b.cursor_col),
        want: b.desired_col,
        anchor: ed.insert_start,
    }
}

fn place(ed: &mut Editor, c: Cursor) {
    let (line, col) = ed.buf().pos_from_char_idx(c.idx);
    let b = ed.buf_mut();
    b.cursor_line = line;
    b.cursor_col = col;
    b.desired_col = c.want;
    ed.insert_start = c.anchor;
}

/// Maps a char index through edits `(start, removed, inserted)` applied in
/// order: positions before an edit stay, positions after it shift, and a
/// position inside removed text lands where it was removed.
pub fn map_pos(pos: usize, edits: &[(usize, usize, usize)]) -> usize {
    edits.iter().fold(pos, |p, &(start, removed, inserted)| {
        if p < start {
            p
        } else if p >= start + removed {
            p - removed + inserted
        } else {
            start
        }
    })
}

fn map_cursor(c: &mut Cursor, edits: &[(usize, usize, usize)]) {
    c.idx = map_pos(c.idx, edits);
    c.anchor = map_pos(c.anchor, edits);
}

/// Sorts and dedups secondaries, dropping any that landed on the primary.
fn normalize(cursors: &mut Vec<Cursor>, primary: usize) {
    cursors.sort_by_key(|c| c.idx);
    cursors.dedup_by_key(|c| c.idx);
    cursors.retain(|c| c.idx != primary);
}

pub fn count(ed: &Editor) -> usize {
    ed.multi.as_ref().map_or(1, |m| m.cursors.len() + 1)
}

/// Secondary cursor positions as `(line, col)`, for rendering.
pub fn positions(ed: &Editor, buffer: u64) -> Vec<(usize, usize)> {
    match &ed.multi {
        Some(m) if m.buffer == buffer && ed.buf().id == buffer => m
            .cursors
            .iter()
            .map(|c| ed.buf().pos_from_char_idx(c.idx))
            .collect(),
        _ => Vec::new(),
    }
}

/// Collapses to the primary cursor.
pub fn clear(ed: &mut Editor) {
    if let Some(m) = ed.multi.take() {
        if let Some(b) = ed.buffers.iter_mut().find(|b| b.id == m.buffer) {
            b.set_edit_log(false);
        }
    }
}

fn ensure(ed: &mut Editor) {
    if ed.multi.as_ref().is_some_and(|m| m.buffer != ed.buf().id) {
        clear(ed);
    }
    if ed.multi.is_none() {
        // Snippet/linked-editing sessions and the completion popup are
        // single-cursor machinery; drop them rather than half-apply them.
        ed.snippet = None;
        ed.close_completion();
        ed.buf_mut().set_edit_log(true);
        ed.multi = Some(MultiCursor {
            buffer: ed.buf().id,
            cursors: Vec::new(),
            pattern: None,
            keys: Vec::new(),
            undo_len: 0,
            seq: ed.buf().edit_seq,
            busy: false,
        });
    }
}

/// Drains the buffer's edit log and maps every secondary through it.
fn drain(ed: &mut Editor) -> Vec<(usize, usize, usize)> {
    let log = ed.buf_mut().take_edit_log();
    if let Some(m) = &mut ed.multi {
        m.seq += log.len() as u64;
        for c in &mut m.cursors {
            map_cursor(c, &log);
        }
    }
    log
}

/// Runs after every key (`Editor::feed_key`): maps the secondaries through
/// whatever the key edited, and collapses them if they can no longer be
/// trusted (another buffer, or a change the edit log doesn't explain).
pub fn after_key(ed: &mut Editor) {
    let Some(m) = &ed.multi else {
        return;
    };
    if m.busy {
        return;
    }
    if m.buffer != ed.buf().id {
        clear(ed);
        return;
    }
    drain(ed);
    if ed
        .multi
        .as_ref()
        .is_some_and(|m| m.seq != ed.buf().edit_seq)
    {
        clear(ed);
        ed.set_message("multiple cursors cleared (buffer changed)");
        return;
    }
    let normal = !matches!(ed.mode, Mode::Insert);
    let primary = capture(ed).idx;
    let b = &ed.buffers[ed.cur];
    let Some(m) = &mut ed.multi else {
        return;
    };
    let len = b.rope.len_chars();
    for c in &mut m.cursors {
        c.idx = c.idx.min(len);
        if normal {
            // Outside Insert a cursor can't rest on a line's end.
            let (l, col) = b.pos_from_char_idx(c.idx);
            c.idx = b.char_idx(l, b.clamp_col_normal(l, col));
        }
    }
    normalize(&mut m.cursors, primary);
    if m.cursors.is_empty() && m.keys.is_empty() {
        clear(ed);
    }
}

/// Key hook (`Editor::feed_key_inner`, ahead of the mode dispatch). Returns
/// whether the key was fully handled here.
pub fn handle(ed: &mut Editor, key: Key) -> bool {
    match &ed.multi {
        Some(m) if !m.busy && m.buffer == ed.buf().id => {}
        _ => return false,
    }
    if ed.active_file_tree() || ed.active_outline() || ed.active_terminal_id().is_some() {
        return false;
    }
    match ed.mode {
        Mode::Normal => normal(ed, key),
        Mode::Insert => insert(ed, key),
        _ => false,
    }
}

fn normal(ed: &mut Editor, key: Key) -> bool {
    let fresh = ed.multi.as_ref().is_some_and(|m| m.keys.is_empty()) && ed.pending.is_empty();
    if fresh {
        match key {
            Key::Esc => {
                clear(ed);
                ed.set_message("multiple cursors cleared");
                return true;
            }
            // Undo/redo restore whole-buffer snapshots the cursors can't be
            // mapped through: collapse, then let Normal mode undo.
            Key::Char('u') | Key::Ctrl('r') => {
                clear(ed);
                return false;
            }
            // Adds a cursor (normal.rs); macro recording is primary-only.
            Key::Ctrl('n') | Key::Char('q') => return false,
            _ => {}
        }
        if key.as_char().map(|c| c.to_string()) == Some(ed.config.leader.clone()) {
            return false;
        }
    }
    // Leader sequences (and the actions they run) and `q{reg}` are
    // primary-only.
    if matches!(
        ed.pending.awaiting,
        Some(Awaiting::Leader { .. } | Awaiting::MacroRegister)
    ) {
        return false;
    }
    let undo_len = ed.buf().undo_len();
    let start = snapshot(ed);
    if let Some(m) = &mut ed.multi {
        if m.keys.is_empty() {
            m.undo_len = undo_len;
        }
        m.keys.push(key);
        m.busy = true;
    }
    crate::normal::handle(ed, key);
    if let Some(m) = &mut ed.multi {
        m.busy = false;
    }
    drain(ed);
    if !ed.pending.is_empty() && ed.mode == Mode::Normal {
        return true; // the command is still in flight
    }
    let Some(m) = &mut ed.multi else {
        return true;
    };
    let keys = std::mem::take(&mut m.keys);
    let undo_len = m.undo_len;
    // Only edits/motions that stay in Normal or enter Insert are replayed;
    // `:`, `/`, Visual mode, pickers... run at the primary alone.
    if !matches!(ed.mode, Mode::Normal | Mode::Insert) {
        return true;
    }
    let start = RunState {
        mode: Mode::Normal,
        ..start
    };
    replay(ed, &start, |ed| {
        for &k in &keys {
            match ed.mode {
                Mode::Normal => crate::normal::handle(ed, k),
                Mode::Insert => crate::insert::handle(ed, k),
                _ => break,
            }
        }
    });
    ed.buf_mut().squash_undo(undo_len);
    true
}

fn insert(ed: &mut Editor, key: Key) -> bool {
    let undo_len = ed.buf().undo_len();
    let start = snapshot(ed);
    ed.close_completion();
    // Secondaries first, the primary last: its run is the one whose
    // resulting state (mode, messages, recorded dot-repeat) stays.
    let primary = capture(ed);
    if let Some(m) = &mut ed.multi {
        m.busy = true;
    }
    let mut primary = run_secondaries(ed, &start, primary, |ed| crate::insert::handle(ed, key));
    restore(ed, &start);
    place(ed, primary);
    crate::insert::handle(ed, key);
    let log = ed.buf_mut().take_edit_log();
    primary = capture(ed);
    if let Some(m) = &mut ed.multi {
        m.seq += log.len() as u64;
        for c in &mut m.cursors {
            map_cursor(c, &log);
        }
        m.busy = false;
        normalize(&mut m.cursors, primary.idx);
    }
    // Completion is single-cursor; never leave its popup open here.
    ed.close_completion();
    ed.buf_mut().squash_undo(undo_len);
    true
}

/// Replays `run` at every secondary (each starting from `start`), then
/// puts the primary -- mapped through their edits -- back with the state
/// its own run left.
fn replay(ed: &mut Editor, start: &RunState, run: impl FnMut(&mut Editor)) {
    let primary = capture(ed);
    let after = snapshot(ed);
    let message = ed.message.clone();
    if let Some(m) = &mut ed.multi {
        m.busy = true;
    }
    let primary = run_secondaries(ed, start, primary, run);
    restore(ed, &after);
    place(ed, primary);
    ed.message = message;
    if let Some(m) = &mut ed.multi {
        m.busy = false;
        normalize(&mut m.cursors, primary.idx);
    }
}

/// Runs `run` once at each secondary cursor, mapping every other cursor
/// (and `primary`, returned) through each run's edits. Dot-repeat and macro
/// recording belong to the primary's run, so these replay as `replaying`.
fn run_secondaries(
    ed: &mut Editor,
    start: &RunState,
    mut primary: Cursor,
    mut run: impl FnMut(&mut Editor),
) -> Cursor {
    let replaying = std::mem::replace(&mut ed.replaying, true);
    let pending = ed.pending.clone();
    let n = ed.multi.as_ref().map_or(0, |m| m.cursors.len());
    for i in 0..n {
        let Some(c) = ed.multi.as_ref().and_then(|m| m.cursors.get(i).copied()) else {
            break;
        };
        restore(ed, start);
        ed.pending.reset();
        place(ed, c);
        run(ed);
        let log = ed.buf_mut().take_edit_log();
        let now = capture(ed);
        map_cursor(&mut primary, &log);
        if let Some(m) = &mut ed.multi {
            m.seq += log.len() as u64;
            for (j, other) in m.cursors.iter_mut().enumerate() {
                if j == i {
                    *other = now;
                } else {
                    map_cursor(other, &log);
                }
            }
        }
    }
    ed.pending = pending;
    ed.replaying = replaying;
    primary
}

/// Runs `run` at every cursor (used for edits that don't come from a key,
/// e.g. a timed-out jk-escape `j`).
pub fn for_each(ed: &mut Editor, mut run: impl FnMut(&mut Editor)) {
    let start = snapshot(ed);
    let undo_len = ed.buf().undo_len();
    let primary = capture(ed);
    if let Some(m) = &mut ed.multi {
        m.busy = true;
    }
    let primary = run_secondaries(ed, &start, primary, &mut run);
    restore(ed, &start);
    place(ed, primary);
    run(ed);
    let primary = capture(ed);
    drain(ed);
    if let Some(m) = &mut ed.multi {
        m.busy = false;
        normalize(&mut m.cursors, primary.idx);
    }
    ed.buf_mut().squash_undo(undo_len);
}

// ---------------- adding cursors ----------------

/// Every char index where `pat` occurs in `text` (non-overlapping), only at
/// word boundaries when `word`.
pub fn find_all(text: &str, pat: &str, word: bool) -> Vec<usize> {
    if pat.is_empty() {
        return Vec::new();
    }
    let is_word = crate::completion::is_word_char;
    let mut out = Vec::new();
    let mut chars = 0;
    let mut last = 0;
    for (byte, _) in text.match_indices(pat) {
        if byte < last {
            continue;
        }
        if word {
            let before = text[..byte].chars().next_back().is_some_and(is_word);
            let after = text[byte + pat.len()..].chars().next().is_some_and(is_word);
            if before || after {
                continue;
            }
        }
        chars += text[last..byte].chars().count();
        out.push(chars);
        chars += pat.chars().count();
        last = byte + pat.len();
    }
    out
}

/// The word under (or, like `*`, first after) the cursor on its line:
/// `(start col, text)`.
fn word_at_cursor(ed: &Editor) -> Option<(usize, String)> {
    let (line, col) = ed.cursor();
    let chars: Vec<char> = ed.buf().line_text(line).chars().collect();
    let is_word = crate::completion::is_word_char;
    let mut i = col.min(chars.len());
    if !chars.get(i).copied().is_some_and(is_word) {
        i = (i..chars.len()).find(|&j| is_word(chars[j]))?;
    }
    let mut s = i;
    while s > 0 && is_word(chars[s - 1]) {
        s -= 1;
    }
    let mut e = i;
    while e < chars.len() && is_word(chars[e]) {
        e += 1;
    }
    Some((s, chars[s..e].iter().collect()))
}

fn message(ed: &mut Editor) {
    let n = count(ed);
    ed.set_message(format!("{n} cursors"));
}

/// Makes the cursor at `idx` the primary, keeping the old primary as a
/// secondary.
fn push_primary(ed: &mut Editor, idx: usize) {
    let old = capture(ed);
    let (line, col) = ed.buf().pos_from_char_idx(idx);
    ed.set_cursor(line, col);
    if let Some(m) = &mut ed.multi {
        m.cursors.push(old);
        normalize(&mut m.cursors, idx);
    }
}

/// Seeds the search pattern from the word under the cursor, moving the
/// cursor to the word's start. Returns false (with a message) if there is
/// no word.
fn seed_word(ed: &mut Editor) -> bool {
    let Some((start, word)) = word_at_cursor(ed) else {
        ed.set_message("no word under the cursor");
        return false;
    };
    let line = ed.cursor().0;
    ed.set_cursor(line, start);
    ensure(ed);
    if let Some(m) = &mut ed.multi {
        m.pattern = Some((word, true));
    }
    true
}

/// `Ctrl-N` in Normal mode: a new cursor at the next occurrence (wrapping)
/// of the word under the cursor -- or of the pattern the first `Ctrl-N`
/// picked -- which becomes the primary.
pub fn add_next(ed: &mut Editor) {
    let has_pattern = ed
        .multi
        .as_ref()
        .is_some_and(|m| m.buffer == ed.buf().id && m.pattern.is_some());
    if !has_pattern && !seed_word(ed) {
        return;
    }
    let Some((pat, word)) = ed.multi.as_ref().and_then(|m| m.pattern.clone()) else {
        return;
    };
    let hits = find_all(&ed.buf().rope.to_string(), &pat, word);
    let here = capture(ed).idx;
    let taken = |i: usize| {
        i == here
            || ed
                .multi
                .as_ref()
                .is_some_and(|m| m.cursors.iter().any(|c| c.idx == i))
    };
    let next = hits
        .iter()
        .copied()
        .filter(|&i| i > here)
        .chain(hits.iter().copied().filter(|&i| i < here))
        .find(|&i| !taken(i));
    match next {
        Some(i) => {
            push_primary(ed, i);
            message(ed);
        }
        None => {
            ed.set_message(format!("no more matches for {pat:?}"));
            after_key(ed);
        }
    }
}

/// `,ma`: a cursor at every occurrence of the word under the cursor (or
/// the active `Ctrl-N` pattern); the primary stays on its own occurrence.
pub fn add_all(ed: &mut Editor) {
    let has_pattern = ed
        .multi
        .as_ref()
        .is_some_and(|m| m.buffer == ed.buf().id && m.pattern.is_some());
    if !has_pattern && !seed_word(ed) {
        return;
    }
    let Some((pat, word)) = ed.multi.as_ref().and_then(|m| m.pattern.clone()) else {
        return;
    };
    let hits = find_all(&ed.buf().rope.to_string(), &pat, word);
    let here = capture(ed);
    let want = ed.buf().desired_col;
    if let Some(m) = &mut ed.multi {
        m.cursors.extend(hits.into_iter().map(|idx| Cursor {
            idx,
            want,
            anchor: idx,
        }));
        normalize(&mut m.cursors, here.idx);
    }
    message(ed);
    after_key(ed);
}

/// `,mj`/`,mk`: a cursor on the line below/above the primary at its
/// sticky column; the new cursor becomes the primary, so repeating extends
/// the column.
pub fn add_vertical(ed: &mut Editor, down: bool) {
    let (line, _) = ed.cursor();
    let target = if down {
        line + 1
    } else {
        match line.checked_sub(1) {
            Some(l) => l,
            None => return ed.set_message("no line above"),
        }
    };
    if target >= ed.buf().line_count() {
        return ed.set_message("no line below");
    }
    ensure(ed);
    let want = ed.buf().desired_col;
    let col = if want == usize::MAX {
        usize::MAX
    } else {
        crate::grapheme::raw_column(&ed.buf().line_text(target), want, ed.buf().tabstop)
    };
    let col = ed.buf().clamp_col_normal(target, col);
    let idx = ed.buf().char_idx(target, col);
    push_primary(ed, idx);
    ed.buf_mut().desired_col = want;
    message(ed);
}

/// `Ctrl-N` in Visual mode. Charwise: the selection becomes the (literal)
/// pattern, the cursor moves to its start, and the next occurrence gets a
/// cursor. Blockwise/linewise: one cursor per selected line, at the block's
/// left column (lines too short for it are skipped) or the cursor's column.
pub fn from_visual(ed: &mut Editor, kind: VisualKind) {
    let Some(anchor) = ed.visual_anchor else {
        return;
    };
    let cur = ed.cursor();
    let (a, z) = if anchor <= cur {
        (anchor, cur)
    } else {
        (cur, anchor)
    };
    ed.visual_anchor = None;
    ed.pending.reset();
    match kind {
        VisualKind::Char => {
            let start = ed.buf().char_idx(a.0, a.1);
            let end = ed
                .buf()
                .char_idx(z.0, z.1 + 1)
                .min(ed.buf().rope.len_chars());
            let text = ed.buf().text_range(start, end);
            ed.enter_normal();
            if text.is_empty() {
                return;
            }
            ed.set_cursor(a.0, a.1);
            clear(ed);
            ensure(ed);
            if let Some(m) = &mut ed.multi {
                m.pattern = Some((text, false));
            }
            add_next(ed);
        }
        VisualKind::Block | VisualKind::Line => {
            let tab = ed.buf().tabstop;
            let left = if kind == VisualKind::Block {
                let ac = crate::grapheme::cell(&ed.buf().line_text(anchor.0), anchor.1, tab);
                let cc = crate::grapheme::cell(&ed.buf().line_text(cur.0), cur.1, tab);
                Some(ac.min(cc))
            } else {
                None
            };
            ed.enter_normal();
            clear(ed);
            ensure(ed);
            let mut cursors = Vec::new();
            for line in a.0..=z.0 {
                let text = ed.buf().line_text(line);
                let col = match left {
                    Some(cell) => {
                        if cell > 0 && unicode_width::UnicodeWidthStr::width(text.as_str()) <= cell
                        {
                            continue;
                        }
                        crate::grapheme::column(&text, cell, false)
                    }
                    None => cur.1,
                };
                let col = ed.buf().clamp_col_normal(line, col);
                cursors.push((line, col));
            }
            // The primary is the line the cursor was on (or the nearest one
            // that got a cursor).
            let Some(&(pl, pc)) = cursors.iter().min_by_key(|(l, _)| l.abs_diff(cur.0)) else {
                clear(ed);
                return ed.set_message("no line reaches the block's column");
            };
            ed.set_cursor(pl, pc);
            let want = ed.buf().desired_col;
            let primary = capture(ed).idx;
            let idxs: Vec<usize> = cursors
                .iter()
                .map(|&(l, c)| ed.buf().char_idx(l, c))
                .collect();
            if let Some(m) = &mut ed.multi {
                m.cursors = idxs
                    .into_iter()
                    .map(|idx| Cursor {
                        idx,
                        want,
                        anchor: idx,
                    })
                    .collect();
                normalize(&mut m.cursors, primary);
            }
            message(ed);
            after_key(ed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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

    /// Feeds keys; `^N`/`^V`/`^R` are Ctrl-N/V/R, `\x1b` Esc, `\n` Enter,
    /// `\x08` Backspace.
    fn keys(e: &mut Editor, s: &str) {
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            e.feed_key(match c {
                '^' => Key::Ctrl(chars.next().unwrap().to_ascii_lowercase()),
                '\x1b' => Key::Esc,
                '\n' => Key::Enter,
                '\x08' => Key::Backspace,
                _ => Key::Char(c),
            });
        }
    }

    fn text(e: &Editor) -> String {
        e.buf().rope.to_string()
    }

    #[test]
    fn ctrl_n_adds_next_occurrences_and_change_applies_everywhere() {
        let mut e = editor("foo bar foo baz foobar foo\n");
        keys(&mut e, "^N");
        assert_eq!(count(&e), 2);
        assert_eq!(e.cursor(), (0, 8), "the new cursor is the primary");
        keys(&mut e, "^N^N");
        assert_eq!(count(&e), 3, "foobar isn't a whole-word match");
        assert!(e.message.contains("no more matches"));
        keys(&mut e, "ciwqux\x1b");
        assert_eq!(text(&e), "qux bar qux baz foobar qux\n");
        assert_eq!(count(&e), 3, "Esc out of Insert keeps the cursors");
        keys(&mut e, "\x1b");
        assert_eq!(count(&e), 1, "Esc in Normal collapses");
        keys(&mut e, "u");
        assert_eq!(text(&e), "foo bar foo baz foobar foo\n", "one undo step");
    }

    #[test]
    fn insert_typing_backspace_and_enter_at_every_cursor() {
        let mut e = editor("one\ntwo\nthree\n");
        keys(&mut e, ",mj,mj");
        assert_eq!(count(&e), 3);
        keys(&mut e, "Ix-\x08:\x1b");
        assert_eq!(text(&e), "x:one\nx:two\nx:three\n");
        keys(&mut e, "A;\nz\x1b");
        assert_eq!(text(&e), "x:one;\nz\nx:two;\nz\nx:three;\nz\n");
        keys(&mut e, "\x1bu");
        assert_eq!(text(&e), "x:one\nx:two\nx:three\n");
    }

    #[test]
    fn cursors_on_one_line_are_mapped_through_each_others_edits() {
        let mut e = editor("a b a c a\n");
        keys(&mut e, "^N^N");
        assert_eq!(count(&e), 3);
        keys(&mut e, "i-\x1b");
        assert_eq!(text(&e), "-a b -a c -a\n");
        keys(&mut e, "lx");
        assert_eq!(text(&e), "- b - c -\n");
        keys(&mut e, "2x");
        // The last cursor fell back onto the final `-` after its `x`.
        assert_eq!(text(&e), "- - \n");
    }

    #[test]
    fn visual_block_ctrl_n_makes_a_column_of_cursors() {
        let mut e = editor("abc\nabcd\na\nabc\n");
        keys(&mut e, "l^V");
        e.set_cursor(3, 1);
        keys(&mut e, "^N");
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(count(&e), 3, "the short line is skipped");
        keys(&mut e, "x");
        assert_eq!(text(&e), "ac\nacd\na\nac\n");
        keys(&mut e, "u");
        assert_eq!(text(&e), "abc\nabcd\na\nabc\n");
        assert_eq!(count(&e), 1, "undo collapses");
    }

    #[test]
    fn visual_char_ctrl_n_uses_the_selection_literally() {
        let mut e = editor("x.y foo x.yz\n");
        keys(&mut e, "vll^N");
        assert_eq!(count(&e), 2);
        keys(&mut e, "3x");
        assert_eq!(text(&e), " foo z\n");
    }

    #[test]
    fn add_all_dot_repeat_and_dollar() {
        let mut e = editor("let a = 1;\nlet b = 2;\nlet c = 3;\n");
        keys(&mut e, ",ma");
        assert_eq!(count(&e), 3);
        keys(&mut e, "dw");
        assert_eq!(text(&e), "a = 1;\nb = 2;\nc = 3;\n");
        keys(&mut e, ".");
        assert_eq!(text(&e), "= 1;\n= 2;\n= 3;\n");
        keys(&mut e, "$x");
        assert_eq!(text(&e), "= 1\n= 2\n= 3\n");
        keys(&mut e, "u");
        assert_eq!(text(&e), "= 1;\n= 2;\n= 3;\n", "each command undoes once");
    }

    #[test]
    fn open_line_and_counted_insert_at_every_cursor() {
        let mut e = editor("a\nb\n");
        keys(&mut e, ",mj");
        keys(&mut e, "oz\x1b");
        assert_eq!(text(&e), "a\nz\nb\nz\n");
        keys(&mut e, "3a.\x1b");
        assert_eq!(text(&e), "a\nz...\nb\nz...\n");
        keys(&mut e, "\x1bu");
        assert_eq!(text(&e), "a\nz\nb\nz\n");
    }

    #[test]
    fn jk_escape_and_its_timeout_apply_at_every_cursor() {
        let mut e = editor("a\nb\n");
        e.config.jk_escape = true;
        keys(&mut e, ",mjA1jk");
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(text(&e), "a1\nb1\n");
        keys(&mut e, "Aj");
        e.flush_pending_jk();
        keys(&mut e, "2\x1b");
        assert_eq!(text(&e), "a1j2\nb1j2\n");
    }

    #[test]
    fn macros_record_once_and_replay_at_every_cursor() {
        let mut e = editor("ab\nab\n");
        keys(&mut e, ",mjqqxq");
        assert_eq!(text(&e), "b\nb\n");
        assert_eq!(e.macros.get(&'q'), Some(&vec![Key::Char('x')]));
        keys(&mut e, "i12\x1b0@q");
        assert_eq!(text(&e), "2b\n2b\n");
    }

    #[test]
    fn edits_the_log_cannot_explain_collapse_the_cursors() {
        let mut e = editor("ab\nab\n");
        keys(&mut e, ",mj");
        assert_eq!(count(&e), 2);
        // An edit outside the logged primitives' bookkeeping (e.g. a
        // whole-buffer replace) can't be mapped.
        e.buf_mut().edit_seq += 1;
        keys(&mut e, "l");
        assert_eq!(count(&e), 1);
        assert!(e.message.contains("cleared"));
    }

    #[test]
    fn map_pos_shifts_after_and_collapses_inside() {
        // insert 3 chars at 5
        assert_eq!(map_pos(2, &[(5, 0, 3)]), 2);
        assert_eq!(map_pos(5, &[(5, 0, 3)]), 8);
        assert_eq!(map_pos(9, &[(5, 0, 3)]), 12);
        // delete [4, 7)
        assert_eq!(map_pos(3, &[(4, 3, 0)]), 3);
        assert_eq!(map_pos(5, &[(4, 3, 0)]), 4);
        assert_eq!(map_pos(7, &[(4, 3, 0)]), 4);
        assert_eq!(map_pos(10, &[(4, 3, 0)]), 7);
        // replace = delete then insert, applied in order
        assert_eq!(map_pos(10, &[(4, 3, 0), (4, 0, 5)]), 12);
    }

    #[test]
    fn find_all_respects_word_boundaries_and_char_indices() {
        let text = "foo foobar éfoo foo_x foo";
        assert_eq!(find_all(text, "foo", true), vec![0, 22]);
        assert_eq!(find_all(text, "foo", false), vec![0, 4, 12, 16, 22]);
        assert_eq!(find_all("aaaa", "aa", false), vec![0, 2]);
        assert!(find_all("abc", "", false).is_empty());
    }

    #[test]
    fn normalize_sorts_dedups_and_drops_the_primary() {
        let c = |idx| Cursor {
            idx,
            want: 0,
            anchor: 0,
        };
        let mut v = vec![c(9), c(3), c(9), c(5)];
        normalize(&mut v, 5);
        assert_eq!(v.iter().map(|c| c.idx).collect::<Vec<_>>(), vec![3, 9]);
    }
}
