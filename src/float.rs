//! Floating window primitive -- a bordered, scrollable, dismissable box
//! painted over the panes -- and the LSP peek views (goto-preview style)
//! built on it: definition / type definition / implementation as a source
//! preview, references (or any multi-location answer) as a list above a
//! preview of the selected location.
//!
//! A float opens focused: keys go to it (scroll, select, open, close)
//! until it's closed or unfocused with Tab / Ctrl-W, after which it stays
//! up as a read-only overlay until the cursor moves, the mode changes or
//! Esc is pressed (`,pf` focuses it again). Only one float exists at a
//! time; opening another replaces it.
use crate::editor::Editor;
use crate::key::Key;
use crate::results::{Entry, Results};
use crate::windows::Rect;
use std::path::Path;

/// Content rows a float shows at most (the border adds two more).
pub const MAX_INNER_HEIGHT: usize = 14;
/// Lines of context kept above a peeked location when it's first shown.
pub const PEEK_CONTEXT_BEFORE: usize = 2;

#[derive(Clone, Debug)]
pub enum FloatBody {
    /// Plain scrollable text (hover documentation, ...).
    Text(Vec<String>),
    /// One or more locations: a single one previews its source; several
    /// show a list above a preview of the selected entry.
    Peek {
        entries: Vec<Entry>,
        selected: usize,
    },
}

#[derive(Clone, Debug)]
pub struct Float {
    pub title: String,
    pub body: FloatBody,
    /// First visible line of the text / previewed source.
    pub top: usize,
    pub focused: bool,
    /// `(buffer id, line, col)` when opened: an unfocused float closes as
    /// soon as the cursor leaves this spot.
    pub anchor: (u64, usize, usize),
    /// Placed beside the file tree sidebar, level with its cursor row,
    /// instead of under the text cursor (the tree's `v` preview).
    pub beside_tree: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStyle {
    Text,
    /// The previewed location's own line.
    Target,
    ListItem,
    ListSelected,
    /// Separator between the location list and the preview.
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloatRow {
    pub text: String,
    pub style: RowStyle,
}

impl Float {
    pub fn text(title: impl Into<String>, lines: Vec<String>, anchor: (u64, usize, usize)) -> Self {
        Float {
            title: title.into(),
            body: FloatBody::Text(lines),
            top: 0,
            focused: true,
            anchor,
            beside_tree: false,
        }
    }

    pub fn peek(
        title: impl Into<String>,
        entries: Vec<Entry>,
        anchor: (u64, usize, usize),
    ) -> Self {
        let top = entries
            .first()
            .map_or(0, |e| e.line.saturating_sub(PEEK_CONTEXT_BEFORE));
        Float {
            title: title.into(),
            body: FloatBody::Peek {
                entries,
                selected: 0,
            },
            top,
            focused: true,
            anchor,
            beside_tree: false,
        }
    }

    /// The entry a peek is previewing (and Enter would open).
    pub fn selected_entry(&self) -> Option<&Entry> {
        match &self.body {
            FloatBody::Peek { entries, selected } => entries.get(*selected),
            FloatBody::Text(_) => None,
        }
    }

    /// Rows of the location list shown above the preview: none for a
    /// single location, otherwise up to half the content height (at least
    /// one, leaving room for the rule and a preview row) -- the same split
    /// the results list uses with its preview on.
    pub fn list_rows(&self, height: usize) -> usize {
        match &self.body {
            FloatBody::Peek { entries, .. } if entries.len() > 1 => entries
                .len()
                .min((height / 2).max(1))
                .min(height.saturating_sub(2)),
            _ => 0,
        }
    }

    /// Content rows needed to show everything without scrolling, capped at
    /// `MAX_INNER_HEIGHT` -- a two-line hover doesn't get a 14-row box.
    pub fn wanted_height(&self, source_len: usize) -> usize {
        let want = match &self.body {
            FloatBody::Text(lines) => lines.len(),
            // The whole file, not just what's below `top`, so the box
            // doesn't change height as j/k moves the selection.
            FloatBody::Peek { entries, .. } if entries.len() > 1 => {
                self.list_rows(MAX_INNER_HEIGHT) + 1 + source_len
            }
            FloatBody::Peek { .. } => source_len.saturating_sub(self.top),
        };
        want.clamp(1, MAX_INNER_HEIGHT)
    }

    /// The content rows for a `height`-row float. `source` is the selected
    /// entry's file (ignored for a text float); `root` shortens list paths.
    pub fn rows(&self, source: &[String], root: &Path, height: usize) -> Vec<FloatRow> {
        let row = |text: String, style| FloatRow { text, style };
        let mut out = Vec::with_capacity(height);
        let (lines, target) = match &self.body {
            FloatBody::Text(lines) => (lines.as_slice(), None),
            FloatBody::Peek { entries, selected } => {
                let list = self.list_rows(height);
                if list > 0 {
                    let first = selected.saturating_sub(list - 1);
                    for (i, e) in entries.iter().enumerate().skip(first).take(list) {
                        let style = if i == *selected {
                            RowStyle::ListSelected
                        } else {
                            RowStyle::ListItem
                        };
                        out.push(row(list_label(e, root), style));
                    }
                    out.push(row(
                        format!("{}/{}", selected + 1, entries.len()),
                        RowStyle::Rule,
                    ));
                }
                (source, entries.get(*selected).map(|e| e.line))
            }
        };
        let numbered = target.is_some();
        for (n, line) in lines
            .iter()
            .enumerate()
            .skip(self.top)
            .take(height.saturating_sub(out.len()))
        {
            let style = if Some(n) == target {
                RowStyle::Target
            } else {
                RowStyle::Text
            };
            let text = if numbered {
                format!("{:>4} {line}", n + 1)
            } else {
                line.clone()
            };
            out.push(row(text, style));
        }
        out
    }

    /// Scrolls the text / preview by `delta` lines, keeping at least the
    /// last line visible.
    pub fn scroll(&mut self, delta: isize, len: usize) {
        let top = self.top as isize + delta;
        self.top = top.clamp(0, len.saturating_sub(1) as isize) as usize;
    }

    /// Moves a multi-location peek's selection, re-centering the preview
    /// on the newly selected entry. A text float or single location
    /// scrolls instead (returns false so the caller can).
    pub fn select(&mut self, delta: isize) -> bool {
        let FloatBody::Peek { entries, selected } = &mut self.body else {
            return false;
        };
        if entries.len() < 2 {
            return false;
        }
        let next = (*selected as isize + delta).clamp(0, entries.len() as isize - 1) as usize;
        *selected = next;
        self.top = entries[next].line.saturating_sub(PEEK_CONTEXT_BEFORE);
        true
    }
}

/// `path:line  text` for a location list row.
fn list_label(e: &Entry, root: &Path) -> String {
    match &e.path {
        Some(p) => format!(
            "{}:{}  {}",
            p.strip_prefix(root).unwrap_or(p).display(),
            e.line + 1,
            e.text.trim()
        ),
        None => e.text.clone(),
    }
}

/// Smallest float worth shrinking to rather than covering the cursor's
/// line: the border plus two content rows.
const MIN_HEIGHT: usize = 4;

/// Where a `w`x`h` float (border included) goes on a `screen_w`x`screen_h`
/// screen: just below the cursor cell if it fits above the message line,
/// else just above it, else shrunk into the roomier of the two sides, and
/// only when even that is too cramped, over the cursor line (clamped to
/// the top). Its left edge follows the cursor column, shifted left to stay
/// on screen.
pub fn place(cursor: (usize, usize), w: usize, h: usize, screen_w: usize, screen_h: usize) -> Rect {
    let w = w.min(screen_w);
    // The bottom row is the message line; never cover it.
    let usable = screen_h.saturating_sub(1);
    let mut h = h.min(usable);
    let x = cursor.0.min(screen_w - w);
    let below = usable.saturating_sub(cursor.1 + 1);
    let above = cursor.1.min(usable);
    let y = if h <= below {
        cursor.1 + 1
    } else if h <= above {
        cursor.1 - h
    } else if below.max(above) >= MIN_HEIGHT {
        if below >= above {
            h = below;
            cursor.1 + 1
        } else {
            h = above;
            0
        }
    } else {
        usable - h
    };
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Outer width for a float on a `screen_w`-column screen: most of the
/// screen, capped so a wide terminal doesn't get a wall of text.
pub fn outer_width(screen_w: usize) -> usize {
    (screen_w * 4 / 5).clamp(screen_w.min(24), 100)
}

/// Key handling while a float is focused. Everything is consumed -- a
/// focused float is modal, like the results list -- except that `:` falls
/// through so commands still work (the float is unfocused first).
pub fn handle(ed: &mut Editor, key: Key) -> bool {
    let Some(f) = ed.float.as_mut() else {
        return false;
    };
    let half = (MAX_INNER_HEIGHT / 2) as isize;
    let source_len = |ed: &Editor| -> usize {
        match ed.float.as_ref().map(|f| &f.body) {
            Some(FloatBody::Text(lines)) => lines.len(),
            _ => ed
                .float
                .as_ref()
                .and_then(Float::selected_entry)
                .and_then(|e| e.path.clone())
                .map_or(0, |p| ed.preview_source_lines(&p).len()),
        }
    };
    match key {
        Key::Esc | Key::Char('q') => ed.float = None,
        Key::Tab | Key::Ctrl('w') => f.focused = false,
        Key::Char(':') => {
            f.focused = false;
            return false;
        }
        Key::Char('j') | Key::Down => {
            if !f.select(1) {
                let len = source_len(ed);
                if let Some(f) = ed.float.as_mut() {
                    f.scroll(1, len);
                }
            }
        }
        Key::Char('k') | Key::Up => {
            if !f.select(-1) {
                let len = source_len(ed);
                if let Some(f) = ed.float.as_mut() {
                    f.scroll(-1, len);
                }
            }
        }
        Key::Ctrl('e') | Key::Ctrl('y') | Key::Ctrl('d') | Key::Ctrl('u') => {
            let delta = match key {
                Key::Ctrl('e') => 1,
                Key::Ctrl('y') => -1,
                Key::Ctrl('d') => half,
                _ => -half,
            };
            let len = source_len(ed);
            if let Some(f) = ed.float.as_mut() {
                f.scroll(delta, len);
            }
        }
        Key::Enter | Key::Char('o') => open_selected(ed, None),
        Key::Char('s') => open_selected(ed, Some(false)),
        Key::Char('v') => open_selected(ed, Some(true)),
        _ => {}
    }
    true
}

/// Opens the peek's selected location (in the current window, or a
/// horizontal/vertical split) and closes the float. Goes through the
/// results list so the jump is recorded and the locations stay
/// retrievable afterwards (`:resume`, Ctrl-Q), exactly like `gd`'s list.
fn open_selected(ed: &mut Editor, split: Option<bool>) {
    let Some(f) = ed.float.take() else {
        return;
    };
    let FloatBody::Peek { entries, selected } = f.body else {
        // A text float has nothing to open; Enter just dismisses it.
        return;
    };
    let mut r = Results::new(f.title, entries);
    r.cursor = selected;
    ed.results = Some(r);
    match split {
        None => ed.open_result(),
        Some(vertical) => ed.open_result_split(vertical),
    }
}

impl Editor {
    /// Opens a peek float over the cursor for `entries` (already resolved
    /// to char columns and line text), or reports there's nothing to show.
    pub fn open_peek(&mut self, title: &str, entries: Vec<Entry>) {
        // A late reply after the user moved on to Insert, a picker, ...:
        // a float would only cover what they're doing now.
        if self.mode != crate::mode::Mode::Normal {
            return;
        }
        if entries.is_empty() {
            self.set_message(format!("No {} found", title.to_lowercase()));
            return;
        }
        let title = if entries.len() > 1 {
            format!("{title} — {}", entries.len())
        } else {
            title.to_string()
        };
        self.float = Some(Float::peek(title, entries, self.float_anchor()));
    }

    /// Opens a plain text float over the cursor.
    pub fn open_text_float(&mut self, title: &str, text: &str) {
        if self.mode != crate::mode::Mode::Normal {
            return;
        }
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.iter().all(|l| l.trim().is_empty()) {
            self.set_message(format!("No {}", title.to_lowercase()));
            return;
        }
        self.float = Some(Float::text(title, lines, self.float_anchor()));
    }

    pub fn float_anchor(&self) -> (u64, usize, usize) {
        let (line, col) = self.cursor();
        (self.buf().id, line, col)
    }

    /// `,pf`: refocus a float left open in the background.
    pub fn focus_float(&mut self) {
        match self.float.as_mut() {
            Some(f) => f.focused = true,
            None => self.set_message("No floating window"),
        }
    }

    pub fn close_float(&mut self) {
        self.float = None;
    }

    /// Called after every key: any float closes once Normal mode is left
    /// (a command, Insert, a picker), and an unfocused one is also tied to
    /// where it was opened, closing once the cursor moves or the buffer
    /// changes -- like a hover in other editors.
    pub fn maybe_dismiss_float(&mut self) {
        let Some(f) = &self.float else {
            return;
        };
        if self.mode != crate::mode::Mode::Normal || (!f.focused && self.float_anchor() != f.anchor)
        {
            self.float = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(line: usize, text: &str) -> Entry {
        Entry::location(PathBuf::from("/p/src/a.rs"), line, 0, text)
    }

    fn src(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line{i}")).collect()
    }

    #[test]
    fn single_peek_previews_source_from_just_above_the_target() {
        let f = Float::peek("Definition", vec![entry(10, "")], (0, 0, 0));
        let rows = f.rows(&src(30), Path::new("/p"), 5);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].text, "   9 line8");
        assert_eq!(rows[2].text, "  11 line10");
        assert_eq!(rows[2].style, RowStyle::Target);
        assert!(rows
            .iter()
            .enumerate()
            .all(|(i, r)| (i == 2) == (r.style == RowStyle::Target)));
    }

    #[test]
    fn target_near_the_top_of_the_file_does_not_underflow() {
        let f = Float::peek("Definition", vec![entry(0, "")], (0, 0, 0));
        assert_eq!(f.top, 0);
        let rows = f.rows(&src(3), Path::new("/p"), 10);
        assert_eq!(rows.len(), 3, "never pads past the end of the file");
        assert_eq!(rows[0].style, RowStyle::Target);
    }

    #[test]
    fn references_show_a_list_rule_and_preview_of_the_selection() {
        let mut f = Float::peek(
            "References",
            vec![entry(1, "  a()"), entry(5, "b"), entry(9, "c")],
            (0, 0, 0),
        );
        let rows = f.rows(&src(20), Path::new("/p"), 12);
        assert_eq!(f.list_rows(12), 3);
        assert_eq!(rows[0].text, "src/a.rs:2  a()");
        assert_eq!(rows[0].style, RowStyle::ListSelected);
        assert_eq!(rows[1].style, RowStyle::ListItem);
        assert_eq!(
            rows[3],
            FloatRow {
                text: "1/3".into(),
                style: RowStyle::Rule
            }
        );
        assert_eq!(rows.len(), 12);
        assert!(rows
            .iter()
            .any(|r| r.style == RowStyle::Target && r.text.ends_with("line1")));

        assert!(f.select(1));
        assert_eq!(f.top, 3, "preview re-centers on the new selection");
        let rows = f.rows(&src(20), Path::new("/p"), 12);
        assert_eq!(rows[1].style, RowStyle::ListSelected);
        assert!(rows
            .iter()
            .any(|r| r.style == RowStyle::Target && r.text.ends_with("line5")));

        // Clamped at both ends.
        f.select(10);
        assert!(matches!(f.body, FloatBody::Peek { selected: 2, .. }));
        f.select(-10);
        assert!(matches!(f.body, FloatBody::Peek { selected: 0, .. }));
    }

    #[test]
    fn a_long_reference_list_scrolls_to_keep_the_selection_visible() {
        let entries: Vec<Entry> = (0..20).map(|i| entry(i, "x")).collect();
        let mut f = Float::peek("References", entries, (0, 0, 0));
        f.select(15);
        let rows = f.rows(&src(30), Path::new("/p"), 12);
        let list = f.list_rows(12);
        assert_eq!(list, 6);
        assert_eq!(rows[list - 1].style, RowStyle::ListSelected);
        assert!(rows[list - 1].text.starts_with("src/a.rs:16"));
        assert_eq!(rows[list].text, "16/20");
    }

    #[test]
    fn single_location_and_text_floats_scroll_instead_of_selecting() {
        let mut f = Float::peek("Definition", vec![entry(4, "")], (0, 0, 0));
        assert!(!f.select(1));
        f.scroll(3, 10);
        assert_eq!(f.top, 5);
        f.scroll(100, 10);
        assert_eq!(f.top, 9, "keeps the last line visible");
        f.scroll(-100, 10);
        assert_eq!(f.top, 0);

        let mut t = Float::text("Hover", vec!["a".into(), "b".into()], (0, 0, 0));
        assert!(!t.select(1));
        assert_eq!(t.wanted_height(0), 2);
        t.scroll(1, 2);
        let rows = t.rows(&[], Path::new("/"), 5);
        assert_eq!(
            rows,
            vec![FloatRow {
                text: "b".into(),
                style: RowStyle::Text
            }]
        );
    }

    #[test]
    fn placement_prefers_below_then_above_and_stays_on_screen() {
        // Room below the cursor.
        let r = place((10, 2), 30, 8, 80, 24);
        assert_eq!((r.x, r.y, r.width, r.height), (10, 3, 30, 8));
        // Too close to the bottom: goes above.
        let r = place((70, 20), 30, 8, 80, 24);
        assert_eq!((r.x, r.y), (50, 12));
        // Neither fits: shrunk into the roomier side (below here, above
        // there) rather than covering the cursor line.
        let r = place((0, 5), 30, 20, 80, 24);
        assert_eq!((r.y, r.height), (6, 17));
        let r = place((0, 15), 30, 20, 80, 24);
        assert_eq!((r.y, r.height), (0, 15));
        let r = place((0, 0), 200, 50, 40, 12);
        assert_eq!((r.x, r.y, r.width, r.height), (0, 1, 40, 10));
        // Too cramped either side: over the cursor line, above the
        // message row.
        let r = place((0, 2), 30, 6, 30, 7);
        assert_eq!((r.y, r.height), (0, 6));
    }

    #[test]
    fn outer_width_is_most_of_a_narrow_screen_and_capped_on_a_wide_one() {
        assert_eq!(outer_width(40), 32);
        assert_eq!(outer_width(20), 20);
        assert_eq!(outer_width(300), 100);
    }
}
