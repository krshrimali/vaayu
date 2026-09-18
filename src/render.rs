//! Cell-aware pane composition with cached terminal rows.
use crate::{
    buffer::Buffer,
    editor::Editor,
    mode::{CommandKind, Mode, VisualKind},
    windows::{Rect, Window},
};
use crossterm::{
    cursor::{Hide, MoveTo, SetCursorStyle, Show},
    execute, queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
#[derive(Clone, Debug)]
struct Glyph {
    cell: usize,
    text: String,
    col: usize,
    width: usize,
}
#[derive(Clone)]
struct DisplayRow {
    line: usize,
    text: std::rc::Rc<str>,
    glyphs: std::rc::Rc<Vec<Glyph>>,
    start: usize,
    content: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct LayoutKey {
    buffer: u64,
    revision: u64,
    top: usize,
    wrap_row: usize,
    width: usize,
    rows: usize,
    wrap: bool,
    left: usize,
    tab: usize,
    insert: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct ViewportRow {
    buffer: u64,
    line: usize,
    start: usize,
    content: u64,
}

type Parts = std::rc::Rc<Vec<(usize, std::rc::Rc<Vec<Glyph>>)>>;
struct CachedViewport {
    key: LayoutKey,
    display: std::rc::Rc<Vec<DisplayRow>>,
    cursor_at: (usize, usize),
    cursor: Option<(usize, usize)>,
}
struct CachedLine {
    revision: u64,
    text: std::rc::Rc<str>,
    parts: Parts,
    content: u64,
}
#[derive(Default)]
pub struct LayoutCache {
    entries: std::collections::HashMap<(u64, usize, usize, bool, usize, usize), CachedLine>,
    next_content: u64,
    viewport: Option<CachedViewport>,
}
impl LayoutCache {
    fn line(
        &mut self,
        b: &Buffer,
        line: usize,
        width: usize,
        wrap: bool,
        left: usize,
        tab: usize,
    ) -> (u64, std::rc::Rc<str>, Parts) {
        let key = (b.id, line, width, wrap, left, tab);
        if let Some(c) = self.entries.get(&key) {
            if c.revision == b.edit_seq {
                return (c.content, c.text.clone(), c.parts.clone());
            }
        }
        let text = b.line_text(line);
        if let Some(c) = self.entries.get_mut(&key) {
            if c.text.as_ref() == text {
                c.revision = b.edit_seq;
                return (c.content, c.text.clone(), c.parts.clone());
            }
        }
        let parts: Parts = std::rc::Rc::new(
            row_parts(&text, width, tab, wrap, left)
                .into_iter()
                .map(|(start, gs)| (start, std::rc::Rc::new(gs)))
                .collect(),
        );
        if self.entries.len() > 2000 {
            self.entries.clear();
        }
        self.next_content = self.next_content.wrapping_add(1);
        let content = self.next_content;
        let text: std::rc::Rc<str> = text.into();
        self.entries.insert(
            key,
            CachedLine {
                revision: b.edit_seq,
                text: text.clone(),
                parts: parts.clone(),
                content,
            },
        );
        (content, text, parts)
    }
}

fn glyphs(text: &str, tabstop: usize) -> Vec<Glyph> {
    let mut out = Vec::new();
    let mut col = 0;
    let mut cells = 0;
    for g in text.graphemes(true) {
        if g == "\t" {
            let width = tabstop.max(1) - cells % tabstop.max(1);
            for offset in 0..width {
                out.push(Glyph {
                    cell: cells + offset,
                    text: " ".into(),
                    col,
                    width: 1,
                });
            }
            cells += width;
        } else {
            let text: String = g
                .chars()
                .map(|c| if c.is_control() { '�' } else { c })
                .collect();
            let width = UnicodeWidthStr::width(text.as_str()).max(1);
            out.push(Glyph {
                cell: cells,
                text,
                col,
                width,
            });
            cells += width;
        }
        col += g.chars().count();
    }
    out
}
pub fn clip(text: &str, width: usize) -> String {
    let mut s = String::new();
    let mut cells = 0;
    for g in glyphs(text, 4) {
        if cells + g.width > width {
            break;
        }
        s.push_str(&g.text);
        cells += g.width;
    }
    s
}
fn pad(text: &str, width: usize) -> String {
    let s = clip(text, width);
    let n = UnicodeWidthStr::width(s.as_str());
    format!("{}{}", s, " ".repeat(width.saturating_sub(n)))
}
fn number_width(ed: &Editor, b: &Buffer) -> usize {
    if ed.config.number {
        (b.line_count().to_string().len() + 1).max(4)
    } else {
        0
    }
}
fn gutter(ed: &Editor, b: &Buffer, width: usize) -> usize {
    (number_width(ed, b) + 2).min(width.saturating_sub(1))
}
fn row_parts(
    text: &str,
    width: usize,
    tab: usize,
    wrap: bool,
    left: usize,
) -> Vec<(usize, Vec<Glyph>)> {
    let width = width.max(1);
    let all = glyphs(text, tab);
    if !wrap {
        let mut cells = 0;
        let mut used = 0;
        let mut out = Vec::new();
        for g in all {
            let end = cells + g.width;
            if cells >= left && used + g.width <= width {
                used += g.width;
                out.push(g);
            }
            cells = end;
        }
        return vec![(left, out)];
    }
    let mut rows = vec![(0, Vec::new())];
    let mut used = 0;
    let mut total = 0;
    for mut g in all {
        if g.width > width {
            g.text = "�".into();
            g.width = 1;
        }
        if used + g.width > width {
            rows.push((total, Vec::new()));
            used = 0;
        }
        used += g.width;
        total += g.width;
        rows.last_mut().unwrap().1.push(g);
    }
    rows
}
fn layout(
    ed: &Editor,
    b: &Buffer,
    w: &Window,
    width: usize,
    rows: usize,
) -> (std::rc::Rc<Vec<DisplayRow>>, Option<(usize, usize)>) {
    let key = LayoutKey {
        buffer: b.id,
        revision: b.edit_seq,
        top: w.top,
        wrap_row: w.wrap_row,
        width,
        rows,
        wrap: ed.config.wrap,
        left: w.left,
        tab: b.tabstop,
        insert: matches!(ed.mode, Mode::Insert),
    };
    {
        let mut cache = ed.layout_cache.borrow_mut();
        if let Some(cached) = cache.viewport.as_mut().filter(|cached| cached.key == key) {
            if cached.cursor_at != w.cursor {
                cached.cursor_at = w.cursor;
                cached.cursor = display_cursor(&cached.display, w, width);
            }
            return (cached.display.clone(), cached.cursor);
        }
    }
    let mut display = Vec::new();
    let end_line = b.line_count().max(if matches!(ed.mode, Mode::Insert) {
        b.rope.len_lines()
    } else {
        0
    });
    'lines: for line in w.top..end_line {
        let (content, text, parts) =
            ed.layout_cache
                .borrow_mut()
                .line(b, line, width, ed.config.wrap, w.left, b.tabstop);
        for (i, (start, gs)) in parts.iter().enumerate() {
            if line == w.top && i < w.wrap_row {
                continue;
            }
            display.push(DisplayRow {
                line,
                text: text.clone(),
                glyphs: gs.clone(),
                start: *start,
                content,
            });
            if display.len() >= rows {
                break 'lines;
            }
        }
    }
    let display = std::rc::Rc::new(display);
    let cursor = display_cursor(&display, w, width);
    ed.layout_cache.borrow_mut().viewport = Some(CachedViewport {
        key,
        display: display.clone(),
        cursor_at: w.cursor,
        cursor,
    });
    (display, cursor)
}

fn display_cursor(display: &[DisplayRow], w: &Window, width: usize) -> Option<(usize, usize)> {
    let first = display.iter().position(|row| row.line == w.cursor.0)?;
    let (screen_row, row) = display
        .iter()
        .enumerate()
        .skip(first)
        .take_while(|(_, row)| row.line == w.cursor.0)
        .filter(|(_, row)| {
            row.glyphs
                .first()
                .is_none_or(|glyph| glyph.col <= w.cursor.1)
        })
        .last()
        .unwrap_or((first, &display[first]));
    let x = row
        .glyphs
        .iter()
        .filter(|glyph| glyph.col < w.cursor.1)
        .map(|glyph| glyph.width)
        .sum::<usize>();
    Some((screen_row, x.min(width.saturating_sub(1))))
}
pub fn prepare_view(ed: &mut Editor, cols: usize, rows: usize) {
    ed.screen_cols = cols;
    ed.store_window();
    let rects = ed.pane_rects(cols, rows);
    let terminal_rects: Vec<(u64, usize, usize)> = ed
        .windows
        .iter()
        .zip(rects.iter())
        .filter_map(|(w, r)| w.terminal.map(|id| (id, r.height, r.width)))
        .collect();
    for (id, height, width) in terminal_rects {
        if let Some(pty) = ed.terminals.iter_mut().find(|p| p.id == id) {
            pty.resize(height as u16, width as u16);
        }
    }
    let rect = rects[ed.active_window.min(rects.len() - 1)];
    let count = rect.height.saturating_sub(1).max(1);
    ed.screen_rows = count;
    let width = rect
        .width
        .saturating_sub(gutter(ed, ed.buf(), rect.width))
        .max(1);
    let mut w = ed.capture_window();
    if !ed.config.wrap {
        let cells = glyphs(&ed.buf().line_text(w.cursor.0), ed.buf().tabstop)
            .iter()
            .filter(|g| g.col < w.cursor.1)
            .map(|g| g.width)
            .sum::<usize>();
        if cells < w.left {
            w.left = cells;
        } else if cells >= w.left + width {
            w.left = cells - width + 1;
        }
        w.wrap_row = 0;
    }
    if w.cursor.0 < w.top {
        w.top = w.cursor.0;
        w.wrap_row = 0;
    }
    if layout(ed, ed.buf(), &w, width, count).1.is_none() {
        if w.cursor.0 >= w.top && w.cursor.0 - w.top < count {
            w.top = w.cursor.0;
        } else {
            w.top = w.cursor.0.saturating_sub(count / 2);
        }
        w.wrap_row = 0;
        if layout(ed, ed.buf(), &w, width, count).1.is_none() {
            w.top = w.cursor.0;
            let text = ed.buf().line_text(w.cursor.0);
            let cells = glyphs(&text, ed.buf().tabstop)
                .iter()
                .filter(|g| g.col < w.cursor.1)
                .map(|g| g.width)
                .sum::<usize>();
            w.wrap_row = (cells / width).saturating_sub(count / 2);
        }
    }
    let b = ed.buf_mut();
    b.top_line = w.top;
    b.top_wrap = w.wrap_row;
    b.left_col = w.left;
    ed.store_window();
}
type Selection = Option<((usize, usize), (usize, usize), VisualKind)>;
#[derive(Clone, PartialEq, Eq, Hash)]
struct RowSignature {
    buffer: u64,
    /// Identity of the immutable glyph vector for this exact rendered line.
    /// Unchanged lines keep their Rc when another line advances the global
    /// revision, so a one-line edit does not invalidate the whole viewport.
    content: u64,
    syntax: u64,
    line: usize,
    start: usize,
    width: usize,
    gutter: usize,
    current: bool,
    relative: Option<usize>,
    selection: Selection,
    search: Option<(String, bool, bool)>,
    /// Char-column ranges on this exact line from `document_highlights`
    /// (see `draw_pane`'s `doc_highlighted` gate and range-clipping
    /// comment), so the row cache invalidates when they change.
    doc_ranges: Vec<(usize, usize)>,
    /// The line-blame virtual text for this exact row, when `blame_toggle`
    /// is on and this is the buffer's current line -- `None` otherwise,
    /// so the cache invalidates correctly across toggling, cursor moves,
    /// and the async blame data first arriving.
    blame: Option<String>,
    marker: char,
    sign: char,
}
pub struct FrameCache {
    rows: Vec<Vec<u8>>,
    scratch: Vec<Vec<u8>>,
    dims: (u16, u16),
    /// Rendered row bodies keyed by semantic content rather than screen
    /// position. Scrolling can then move a line to another terminal row
    /// without rebuilding all of its styling and glyph runs.
    composed: std::collections::HashMap<RowSignature, Vec<u8>>,
    logical: Vec<Option<RowSignature>>,
    viewport: Vec<ViewportRow>,
}
impl FrameCache {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            scratch: Vec::new(),
            dims: (0, 0),
            composed: Default::default(),
            logical: Vec::new(),
            viewport: Vec::new(),
        }
    }
}

impl RowSignature {
    fn same_scroll_content(&self, other: &Self) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        // Moving the viewport changes which line owns the active-line gutter
        // color. That line is repainted after the terminal scroll; it should
        // not prevent detecting that every other row shifted intact.
        a.current = false;
        b.current = false;
        a == b
    }
}

/// Returns a positive amount when content moved upward (terminal SU), or a
/// negative amount when it moved downward (SD). Require all but at most two
/// overlapping rows to agree; those two are the old and new active rows.
fn detect_scroll(
    old: &[Option<RowSignature>],
    new: &[Option<RowSignature>],
    body: usize,
) -> Option<isize> {
    if old.len() < body || new.len() < body {
        return None;
    }
    for amount in 1..body {
        let overlap = body - amount;
        if overlap < 3 {
            break;
        }
        let up = (0..overlap)
            .filter(|y| match (&new[*y], &old[*y + amount]) {
                (Some(a), Some(b)) => a.same_scroll_content(b),
                _ => false,
            })
            .count();
        if up >= overlap.saturating_sub(2) {
            return Some(amount as isize);
        }
        let down = (0..overlap)
            .filter(|y| match (&new[*y + amount], &old[*y]) {
                (Some(a), Some(b)) => a.same_scroll_content(b),
                _ => false,
            })
            .count();
        if down >= overlap.saturating_sub(2) {
            return Some(-(amount as isize));
        }
    }
    None
}

fn detect_view_scroll(old: &[ViewportRow], new: &[ViewportRow]) -> Option<isize> {
    let body = old.len().min(new.len());
    for amount in 1..body {
        let overlap = body - amount;
        if overlap < 3 {
            break;
        }
        if new[..overlap] == old[amount..amount + overlap] {
            return Some(amount as isize);
        }
        if new[amount..amount + overlap] == old[..overlap] {
            return Some(-(amount as isize));
        }
    }
    None
}

fn emit_scroll<W: Write>(out: &mut W, height: usize, shift: isize) -> io::Result<()> {
    let amount = shift.unsigned_abs();
    write!(
        out,
        "\x1b[1;{}r\x1b[1;1H\x1b[{}{}\x1b[r",
        height - 1,
        amount,
        if shift > 0 { 'M' } else { 'L' }
    )?;
    out.flush()
}
fn plain_row(
    rows: &mut [Vec<u8>],
    y: usize,
    x: usize,
    width: usize,
    text: &str,
    bg: Color,
) -> io::Result<()> {
    if let Some(row) = rows.get_mut(y) {
        // A hardcoded White foreground only makes sense paired with a
        // deliberately dark, explicit background (e.g. a selected row's
        // DarkCyan). The plain/default case (`bg == Reset`, the
        // overwhelming majority of rows in any list) must leave the
        // foreground at the terminal's own default too -- forcing White
        // text on a Reset background renders as invisible white-on-white
        // on any light-background terminal theme.
        let fg = if bg == Color::Reset {
            Color::Reset
        } else {
            Color::White
        };
        queue!(
            row,
            MoveTo(x as u16, y as u16),
            SetBackgroundColor(bg),
            SetForegroundColor(fg),
            Print(pad(text, width)),
            ResetColor,
            SetAttribute(Attribute::Reset)
        )?;
    }
    Ok(())
}
pub fn draw<W: Write>(
    out: &mut W,
    ed: &Editor,
    cols: u16,
    rows: u16,
    cache: &mut FrameCache,
) -> io::Result<()> {
    let (width, height) = (cols as usize, rows as usize);
    if width == 0 || height == 0 {
        return Ok(());
    }
    let mut frame = std::mem::take(&mut cache.scratch);
    frame.resize_with(height, Vec::new);
    frame.truncate(height);
    for row in &mut frame {
        row.clear();
    }
    let mut logical = vec![None; height];
    let mut cursor = (0, 0);
    let mut bar = false;
    let scroll_eligible = cache.dims == (cols, rows)
        && height > 2
        && matches!(ed.mode, Mode::Normal)
        && ed.completion.is_none()
        && !ed.config.relativenumber
        && ed.window_layout.is_none()
        && ed.windows.len() <= 1
        // The DECSTBM scroll-region trick below hardcodes row 1 as the
        // first content row; a tabline moves content down by one, so this
        // fast path is skipped (falling back to the still-correct
        // full-row diff) whenever more than one tab exists.
        && ed.tabs.len() <= 1;
    let mut viewport = Vec::new();
    let early_scroll = if scroll_eligible {
        let rect = ed.pane_rects(width, height)[0];
        let w = if ed.windows.is_empty() {
            ed.capture_window()
        } else {
            ed.windows[0].clone()
        };
        ed.buffers.iter().find(|b| b.id == w.buffer).and_then(|b| {
            let gw = gutter(ed, b, rect.width);
            let pane_width = rect.width.saturating_sub(gw).max(1);
            let (display, _) = layout(ed, b, &w, pane_width, rect.height.saturating_sub(1));
            viewport = display
                .iter()
                .map(|row| ViewportRow {
                    buffer: b.id,
                    line: row.line,
                    start: row.start,
                    content: row.content,
                })
                .collect();
            detect_view_scroll(&cache.viewport, &viewport)
        })
    } else {
        None
    };
    if let Some(shift) = early_scroll {
        emit_scroll(out, height, shift)?;
    }
    if matches!(ed.mode, Mode::Results) {
        cursor = draw_results(&mut frame, ed, width, height)?;
        bar = ed
            .results
            .as_ref()
            .is_some_and(|r| r.search_input.is_some());
    } else if matches!(ed.mode, Mode::Picker) {
        cursor = draw_picker(&mut frame, ed, width, height)?;
        bar = true;
    } else if matches!(ed.mode, Mode::MarkdownPreview) {
        draw_full_preview(&mut frame, ed, width, height)?;
    } else {
        if ed.tabs.len() > 1 {
            draw_tabline(&mut frame, ed, width)?;
        }
        let rects = ed.pane_rects(width, height);
        for (i, rect) in rects.iter().copied().enumerate() {
            if rect.x + rect.width < width {
                for y in rect.y..rect.y + rect.height {
                    plain_row(&mut frame, y, rect.x + rect.width, 1, "│", Color::DarkGrey)?;
                }
            }
            if rect.y + rect.height < height.saturating_sub(1) {
                plain_row(
                    &mut frame,
                    rect.y + rect.height,
                    rect.x,
                    rect.width,
                    &"─".repeat(rect.width),
                    Color::DarkGrey,
                )?;
            }
            let w = if ed.windows.is_empty() {
                ed.capture_window()
            } else {
                ed.windows[i].clone()
            };
            let active = i == ed.active_window;
            if w.file_tree {
                if let Some(tree) = &ed.file_tree {
                    draw_file_tree_pane(&mut frame, ed, tree, rect, active)?;
                }
                continue;
            }
            if w.outline {
                if let Some(outline) = &ed.outline {
                    draw_outline_pane(&mut frame, outline, rect, active)?;
                }
                continue;
            }
            if let Some(id) = w.terminal {
                if let Some(pty) = ed.terminals.iter().find(|p| p.id == id) {
                    if let Some(c) = draw_terminal_pane(&mut frame, pty, rect)? {
                        if active {
                            cursor = c;
                        }
                    }
                }
                continue;
            }
            let Some(b) = ed.buffers.iter().find(|b| b.id == w.buffer) else {
                continue;
            };
            if w.preview {
                draw_preview_pane(&mut frame, ed, b, &w, rect)?;
            } else if let Some(c) = draw_pane(
                &mut PaneTarget {
                    frame: &mut frame,
                    logical: &mut logical,
                },
                ed,
                b,
                &w,
                rect,
                active,
                cache,
            )? {
                if active {
                    cursor = c;
                }
            }
        }
        let message = match ed.mode {
            Mode::Command(k) => format!(
                "{}{}",
                match k {
                    CommandKind::Ex => ':',
                    CommandKind::SearchFwd => '/',
                    CommandKind::SearchBack => '?',
                },
                ed.cmdline
            ),
            _ => ed.message.clone(),
        };
        plain_row(&mut frame, height - 1, 0, width, &message, Color::Reset)?;
        if matches!(ed.mode, Mode::Command(_)) {
            cursor = (clip(&message, width.saturating_sub(1)).width(), height - 1);
            bar = true;
        } else {
            bar = matches!(ed.mode, Mode::Insert);
        }
        let completion_delay_elapsed = ed.completion_since.is_some_and(|t| {
            t.elapsed() >= std::time::Duration::from_millis(ed.config.completion_delay_ms)
        });
        if let Some(comp) = &ed.completion {
            if !comp.items.is_empty() && completion_delay_elapsed {
                let pane = ed.pane_rects(width, height)[ed.active_window];
                let visible = comp.items.len().min(8).min(pane.height.saturating_sub(2));
                let first = comp.selected.saturating_sub(visible.saturating_sub(1));
                let w = 40.min(pane.width);
                let x = cursor.0.min(pane.x + pane.width - w);
                let y = if cursor.1 + 1 + visible < pane.y + pane.height {
                    cursor.1 + 1
                } else {
                    cursor.1.saturating_sub(visible)
                };
                for (i, item) in comp.items.iter().skip(first).take(visible).enumerate() {
                    plain_row(
                        &mut frame,
                        y + i,
                        x,
                        w,
                        &format!(
                            " {} {}{}",
                            match (item.kind, &item.source) {
                                (Some(k), _) => crate::completion::kind_label(k),
                                (None, crate::completion::Source::Lsp) => "lsp",
                                (None, crate::completion::Source::Path) => "path",
                                (None, crate::completion::Source::Buffer) => "buf",
                            },
                            item.label,
                            item.detail
                                .as_ref()
                                .map(|d| format!(" · {d}"))
                                .unwrap_or_default()
                        ),
                        if first + i == comp.selected {
                            Color::DarkCyan
                        } else {
                            Color::DarkBlue
                        },
                    )?;
                }
                // Documentation preview: extra rows directly below the
                // item list showing the *selected* item's multi-line
                // `documentation`, when it has one and there's room --
                // most items don't, so this doesn't grow the popup for
                // nothing. Distinct from `detail` (a short signature/type
                // shown inline on each row already).
                if let Some(doc) = comp
                    .items
                    .get(comp.selected)
                    .and_then(crate::completion::item_documentation)
                {
                    let doc_y = y + visible;
                    let doc_rows = (pane.y + pane.height)
                        .saturating_sub(doc_y)
                        .min(5)
                        .min(doc.lines().count() + 1);
                    if doc_rows > 1 {
                        plain_row(&mut frame, doc_y, x, w, " Docs:", Color::DarkGrey)?;
                        for (i, line) in doc.lines().take(doc_rows - 1).enumerate() {
                            plain_row(
                                &mut frame,
                                doc_y + 1 + i,
                                x,
                                w,
                                &format!(" {line}"),
                                Color::DarkGrey,
                            )?;
                        }
                    }
                }
            }
        }
        if let Some(crate::normal::Awaiting::Leader { seq, since }) = &ed.pending.awaiting {
            if since.elapsed()
                >= std::time::Duration::from_millis(ed.config.whichkey_delay_ms.max(1))
            {
                draw_whichkey(&mut frame, ed, seq, width, height)?;
            }
        }
    }
    if cache.dims != (cols, rows) {
        queue!(out, Clear(ClearType::All))?;
        cache.rows.clear();
        cache.logical.clear();
        cache.dims = (cols, rows);
    }
    if scroll_eligible {
        if let Some(shift) =
            early_scroll.or_else(|| detect_scroll(&cache.logical, &logical, height - 1))
        {
            let amount = shift.unsigned_abs();
            // DECSTBM confines delete/insert-line to the editor body,
            // preserving the message row. DL/IL are supported more
            // consistently inside margins than SU/SD. The normal cursor
            // command below restores the final position.
            if early_scroll.is_none() {
                emit_scroll(out, height, shift)?;
            }
            let old_rows = cache.rows.clone();
            let old_logical = cache.logical.clone();
            cache.rows = vec![Vec::new(); height];
            for (y, rendered) in frame.iter().enumerate().take(height - 1) {
                let source = if shift > 0 {
                    y.checked_add(amount).filter(|source| *source < height - 1)
                } else {
                    y.checked_sub(amount)
                };
                if let Some(source) = source {
                    if old_logical.get(source) == logical.get(y) {
                        cache.rows[y] = rendered.clone();
                    }
                }
            }
            if let Some(status) = old_rows.get(height - 1) {
                cache.rows[height - 1] = status.clone();
            }
        }
    }
    for (y, row) in frame.iter().enumerate() {
        if cache.rows.get(y) != Some(row) {
            queue!(
                out,
                MoveTo(0, y as u16),
                ResetColor,
                SetAttribute(Attribute::Reset),
                Clear(ClearType::UntilNewLine)
            )?;
            out.write_all(row)?;
        }
    }
    let previous = std::mem::replace(&mut cache.rows, frame);
    cache.scratch = previous;
    cache.logical = logical;
    cache.viewport = viewport;
    if cache.composed.len() > 8192 {
        cache.composed.clear();
    }
    if matches!(ed.mode, Mode::MarkdownPreview) {
        queue!(out, Hide)?;
    } else {
        queue!(
            out,
            MoveTo(
                cursor.0.min(width - 1) as u16,
                cursor.1.min(height - 1) as u16
            ),
            SetCursorStyle::SteadyBlock,
            Show
        )?;
        if bar {
            queue!(out, SetCursorStyle::SteadyBar)?;
        }
    }
    out.flush()
}
struct PaneTarget<'a> {
    frame: &'a mut [Vec<u8>],
    logical: &'a mut [Option<RowSignature>],
}

fn draw_pane(
    target: &mut PaneTarget<'_>,
    ed: &Editor,
    b: &Buffer,
    w: &Window,
    r: Rect,
    active: bool,
    cache: &mut FrameCache,
) -> io::Result<Option<(usize, usize)>> {
    if r.width == 0 || r.height == 0 {
        return Ok(None);
    }
    let gw = gutter(ed, b, r.width);
    let width = r.width.saturating_sub(gw).max(1);
    let n = r.height.saturating_sub(1);
    let (display, cursor) = layout(ed, b, w, width, n);
    let mut source_cache = std::collections::HashMap::new();
    let search = ed
        .last_search
        .as_ref()
        .filter(|_| ed.hl_search)
        .and_then(|(p, _)| {
            crate::search::compile(p, ed.config.ignorecase, ed.config.smartcase).ok()
        });
    // Only paint `document_highlights` while they're still for this exact
    // buffer and it hasn't been edited since the request -- a stale set
    // would otherwise highlight whatever now sits at those old positions.
    let doc_highlighted = ed.document_highlights_buffer == Some(b.id)
        && ed.document_highlights_edit_seq == b.edit_seq;
    let selection = if active {
        ed.visual_anchor
            .filter(|_| matches!(ed.mode, Mode::Visual(_)))
            .map(|a| {
                if a <= w.cursor {
                    (a, w.cursor)
                } else {
                    (w.cursor, a)
                }
            })
    } else {
        None
    };
    let selection = selection.or_else(|| {
        if !active {
            return None;
        }
        ed.snippet
            .as_ref()
            .filter(|s| s.selected)
            .and_then(|s| s.stops.get(s.current))
            .filter(|(a, b)| b > a)
            .map(|(a, z)| (b.pos_from_char_idx(*a), b.pos_from_char_idx(z - 1)))
    });
    for row in 0..n {
        let y = r.y + row;
        let dest = &mut target.frame[y];
        queue!(dest, MoveTo(r.x as u16, y as u16))?;
        let content_start = dest.len();
        let Some(d) = display.get(row) else {
            queue!(
                dest,
                SetForegroundColor(Color::DarkGrey),
                Print(pad("~", r.width)),
                ResetColor
            )?;
            continue;
        };
        let diag = b
            .path
            .as_ref()
            .and_then(|p| ed.diagnostics.get(p))
            .and_then(|ds| {
                ds.iter()
                    .filter(|d2| d2.line == d.line)
                    .min_by_key(|d2| match d2.severity {
                        crate::lsp::Severity::Error => 0,
                        crate::lsp::Severity::Warning => 1,
                        _ => 2,
                    })
            });
        let sign = if b.id == ed.buf().id {
            ed.git
                .as_ref()
                .and_then(|g| g.signs.get(&d.line))
                .map(|s| match s {
                    crate::gitdiff::Sign::Added => '+',
                    crate::gitdiff::Sign::Modified => '~',
                    crate::gitdiff::Sign::Removed => '-',
                })
                .unwrap_or(' ')
        } else {
            ' '
        };
        let annotation = b.path.as_ref().is_some_and(|p| {
            ed.notes
                .items
                .iter()
                .any(|n| ed.project_root.join(&n.file) == *p && (n.whole_file || n.start == d.line))
        });
        let marker = if let Some(d) = diag {
            match d.severity {
                crate::lsp::Severity::Error => 'E',
                crate::lsp::Severity::Warning => 'W',
                _ => 'I',
            }
        } else if annotation {
            '●'
        } else {
            ' '
        };
        // Clips each `document_highlights` range to this line: a single-
        // line range keeps its own start/end columns; a multi-line one
        // covers from its start column to end-of-line on its first line,
        // the whole line on any line strictly between, and from
        // start-of-line to its end column on its last line.
        let doc_ranges: Vec<(usize, usize)> = if doc_highlighted {
            ed.document_highlights
                .iter()
                .filter_map(|&(l1, c1, l2, c2)| {
                    if d.line < l1 || d.line > l2 {
                        None
                    } else if l1 == l2 {
                        Some((c1, c2))
                    } else if d.line == l1 {
                        Some((c1, usize::MAX))
                    } else if d.line == l2 {
                        Some((0, c2))
                    } else {
                        Some((0, usize::MAX))
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let blame = (ed.blame_toggle
            && b.id == ed.buf().id
            && d.line == w.cursor.0
            && ed.line_blame_path.as_ref() == b.path.as_ref())
        .then_some(ed.line_blame.as_ref())
        .flatten()
        .and_then(|lines| lines.get(d.line))
        .cloned();
        let sig = RowSignature {
            buffer: b.id,
            content: d.content,
            syntax: if b.id == ed.buf().id {
                ed.syntax_stamp
            } else {
                0
            },
            doc_ranges: doc_ranges.clone(),
            blame: blame.clone(),
            line: d.line,
            start: d.start,
            width: r.width,
            gutter: gw,
            current: d.line == w.cursor.0,
            relative: if ed.config.relativenumber {
                Some(w.cursor.0)
            } else {
                None
            },
            selection: selection.map(|(a, z)| {
                (
                    a,
                    z,
                    if let Mode::Visual(k) = ed.mode {
                        k
                    } else {
                        VisualKind::Char
                    },
                )
            }),
            search: ed
                .last_search
                .as_ref()
                .filter(|_| ed.hl_search)
                .map(|(p, _)| (p.clone(), ed.config.ignorecase, ed.config.smartcase)),
            marker,
            sign,
        };
        target.logical[y] = Some(sig.clone());
        if let Some(bytes) = cache.composed.get(&sig) {
            dest.extend_from_slice(bytes);
            continue;
        }
        let text = d.text.as_ref();
        let (spans, matches) = source_cache.entry(d.line).or_insert_with(|| {
            let mut spans = Vec::new();
            if b.id == ed.buf().id {
                if let Some(syn) = &ed.syntax {
                    let (start, end) = b.line_byte_range(d.line);
                    for (s, e, class) in syn.spans_in(start, end) {
                        let a = safe_boundary(text, s.saturating_sub(start));
                        let z = safe_boundary(text, e.saturating_sub(start));
                        spans.push((text[..a].chars().count(), text[..z].chars().count(), class));
                    }
                }
            }
            let matches: Vec<_> = search
                .as_ref()
                .into_iter()
                .flat_map(|re| {
                    re.find_iter(text).filter_map(Result::ok).map(|m| {
                        (
                            text[..m.start()].chars().count(),
                            text[..m.end()].chars().count(),
                        )
                    })
                })
                .collect();
            (spans, matches)
        });
        let number = if d.start > 0 && ed.config.wrap {
            "↪".into()
        } else if ed.config.number {
            if ed.config.relativenumber && d.line != w.cursor.0 {
                d.line.abs_diff(w.cursor.0).to_string()
            } else {
                (d.line + 1).to_string()
            }
        } else {
            String::new()
        };
        let margin = if gw >= 2 {
            format!(
                "{}{}{:>width$} ",
                marker,
                sign,
                number,
                width = gw.saturating_sub(3)
            )
        } else {
            " ".repeat(gw)
        };
        queue!(
            dest,
            SetForegroundColor(if d.line == w.cursor.0 {
                Color::Yellow
            } else {
                Color::DarkGrey
            }),
            Print(pad(&margin, gw)),
            ResetColor
        )?;
        let mut used = 0;
        let mut runs: Vec<((bool, bool, bool, Color), String)> = Vec::new();
        for g in d.glyphs.iter() {
            let selected = selection.is_some_and(|(a, z)| {
                d.line >= a.0
                    && d.line <= z.0
                    && (if matches!(ed.mode, Mode::Visual(VisualKind::Block)) {
                        let ac = crate::grapheme::cell(&b.line_text(a.0), a.1, b.tabstop);
                        let zc = crate::grapheme::cell(&b.line_text(z.0), z.1, b.tabstop);
                        let gc = g.cell;
                        gc >= ac.min(zc) && gc <= ac.max(zc)
                    } else {
                        (matches!(ed.mode, Mode::Visual(VisualKind::Line))
                            || ((d.line > a.0 || g.col >= a.1) && (d.line < z.0 || g.col <= z.1)))
                    })
            });
            let searched = matches.iter().any(|(a, z)| g.col >= *a && g.col < *z);
            let doc_hl = doc_ranges.iter().any(|(a, z)| g.col >= *a && g.col < *z);
            let color = spans
                .iter()
                .find(|(a, z, _)| g.col >= *a && g.col < *z)
                .map(|(_, _, c)| match c {
                    crate::syntax::HlClass::Comment => Color::DarkGrey,
                    crate::syntax::HlClass::String => Color::Green,
                    crate::syntax::HlClass::Number => Color::Magenta,
                    crate::syntax::HlClass::Keyword => Color::Cyan,
                })
                .unwrap_or(Color::Reset);
            let style = (selected, searched, doc_hl, color);
            if let Some((prev, text)) = runs.last_mut() {
                if *prev == style {
                    text.push_str(&g.text);
                } else {
                    runs.push((style, g.text.clone()));
                }
            } else {
                runs.push((style, g.text.clone()));
            }
            used += g.width;
        }
        for ((selected, searched, doc_hl, color), text) in runs {
            // A highlight background overrides the foreground too --
            // otherwise arbitrary syntax coloring (e.g. a Cyan keyword)
            // sits on top of it and can clash badly (cyan-on-yellow,
            // magenta-on-blue) regardless of the terminal's theme. Plain
            // Visual selection stays a pure Reverse (swaps whatever
            // colors are already there, so it always matches the theme
            // by construction) with no separate foreground override.
            let fg = if selected {
                color
            } else if searched {
                Color::Black
            } else if doc_hl {
                Color::White
            } else {
                color
            };
            if selected {
                queue!(dest, SetAttribute(Attribute::Reverse))?;
            } else if searched {
                queue!(dest, SetBackgroundColor(Color::DarkYellow))?;
            } else if doc_hl {
                queue!(dest, SetBackgroundColor(Color::DarkBlue))?;
            }
            queue!(
                dest,
                SetForegroundColor(fg),
                Print(text),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
        }
        if let Some(text) = &blame {
            let remaining = width.saturating_sub(used);
            if remaining > 2 {
                let shown = clip(&format!("  {text}"), remaining);
                queue!(
                    dest,
                    SetForegroundColor(Color::DarkGrey),
                    Print(&shown),
                    ResetColor
                )?;
                used += shown.width();
            }
        }
        queue!(dest, Print(" ".repeat(width.saturating_sub(used))))?;
        cache.composed.insert(sig, dest[content_start..].to_vec());
    }
    let name = b
        .path
        .as_ref()
        .map(|p| {
            p.strip_prefix(&ed.project_root)
                .unwrap_or(p)
                .display()
                .to_string()
        })
        .unwrap_or_else(|| b.name());
    let right = format!(" {}:{} ", w.cursor.0 + 1, w.cursor.1 + 1);
    let left = format!(
        " {} {}{}",
        if active { ed.mode.label() } else { "BUFFER" },
        name,
        if b.is_modified() { " [+]" } else { "" }
    );
    let label = format!(
        "{}{}",
        pad(&left, r.width.saturating_sub(right.width())),
        right
    );
    plain_row(
        target.frame,
        r.y + r.height - 1,
        r.x,
        r.width,
        &label,
        if active {
            Color::DarkBlue
        } else {
            Color::DarkGrey
        },
    )?;
    Ok(cursor.map(|(y, x)| (r.x + gw + x, r.y + y)))
}
fn safe_boundary(s: &str, offset: usize) -> usize {
    let mut i = offset.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
/// The tab line: one row across the top listing "1 2 3 ...", the active
/// tab shown reverse-video. Only ever drawn -- and only ever reserved
/// space by `pane_rects` -- once a second tab exists, so a single-tab
/// session's layout is completely unaffected by this feature existing.
fn draw_tabline(frame: &mut [Vec<u8>], ed: &Editor, width: usize) -> io::Result<()> {
    let Some(row) = frame.get_mut(0) else {
        return Ok(());
    };
    queue!(
        row,
        MoveTo(0, 0),
        SetBackgroundColor(Color::DarkGrey),
        SetForegroundColor(Color::White),
        Print(" ".repeat(width)),
        MoveTo(0, 0)
    )?;
    let mut used = 0;
    for i in 0..ed.tabs.len() {
        let label = format!(" {} ", i + 1);
        if used + label.chars().count() > width {
            break;
        }
        if i == ed.active_tab {
            queue!(
                row,
                SetAttribute(Attribute::Reverse),
                Print(&label),
                SetAttribute(Attribute::NoReverse)
            )?;
        } else {
            queue!(row, Print(&label))?;
        }
        used += label.chars().count();
    }
    queue!(row, ResetColor, SetAttribute(Attribute::Reset))?;
    Ok(())
}

/// Which-key style prefix popup: lists every registered action whose default
/// key sequence continues `seq`, anchored to the bottom-right corner above
/// the message line. Only reachable once the caller has already confirmed
/// the configured delay elapsed, so a completed mapping never flashes it.
fn draw_whichkey(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    seq: &str,
    width: usize,
    height: usize,
) -> io::Result<()> {
    let items = crate::actions::matching(seq);
    if items.is_empty() {
        return Ok(());
    }
    let max_rows = height.saturating_sub(2); // keep the message line and >=1 header row
    if max_rows == 0 {
        return Ok(());
    }
    let visible = items.len().min(max_rows);
    let w = items
        .iter()
        .take(visible)
        .map(|a| a.keys.chars().count() + a.title.chars().count() + 6)
        .max()
        .unwrap_or(16)
        .clamp(16, width.max(16));
    let x = width.saturating_sub(w);
    let y = height.saturating_sub(visible + 2);
    plain_row(
        frame,
        y,
        x,
        w,
        &format!(" {}{}", ed.config.leader, seq),
        Color::DarkYellow,
    )?;
    for (i, a) in items.iter().take(visible).enumerate() {
        plain_row(
            frame,
            y + 1 + i,
            x,
            w,
            &format!(" {}{:<6}{}", ed.config.leader, a.keys, a.title),
            Color::DarkGrey,
        )?;
    }
    Ok(())
}
fn draw_results(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    width: usize,
    height: usize,
) -> io::Result<(usize, usize)> {
    let Some(r) = &ed.results else {
        return Ok((0, 0));
    };
    let selected = r.selected.len();
    plain_row(
        frame,
        0,
        0,
        width,
        &format!(
            " {}{}  · {} results · {} selected{}",
            if r.quickfix { "QUICKFIX / " } else { "" },
            r.title,
            r.entries.len(),
            selected,
            if r.busy { " · searching…" } else { "" }
        ),
        Color::DarkBlue,
    )?;
    if height < 4 {
        return Ok((0, 0));
    }
    let detail_rows = if height < 8 {
        0
    } else if r.preview {
        // A real file-content preview earns more room than the plain
        // detail/text strip; still leaves at least 2 rows for the list.
        (height / 2).clamp(4, height.saturating_sub(6))
    } else if height >= 12 {
        4
    } else {
        0
    };
    let list_rows = height.saturating_sub(4 + detail_rows);
    let first = r.cursor.saturating_sub(list_rows.saturating_sub(1));
    for i in 0..list_rows {
        let idx = first + i;
        let text = r
            .entries
            .get(idx)
            .map(|e| {
                format!(
                    "{} {:>4}  {}",
                    if r.selected.contains(&idx) {
                        "●"
                    } else {
                        " "
                    },
                    idx + 1,
                    e.display(&ed.project_root)
                )
            })
            .unwrap_or_default();
        plain_row(
            frame,
            i + 2,
            0,
            width,
            &text,
            if idx == r.cursor {
                Color::DarkCyan
            } else {
                Color::Reset
            },
        )?;
    }
    let prompt = if let Some(forward) = r.search_input {
        format!("{}{}", if forward { '/' } else { '?' }, r.query)
    } else {
        format!(
            " {}{}",
            if r.quickfix {
                "Search / ? · n N"
            } else {
                "Ctrl-Q → quickfix"
            },
            if r.live { " · i edit grep query" } else { "" }
        )
    };
    plain_row(frame, 1, 0, width, &prompt, Color::Reset)?;
    let detail_y = 2 + list_rows;
    if detail_rows > 0 {
        let path = r.entries.get(r.cursor).and_then(|e| e.path.clone());
        let preview = path.and_then(|p| {
            let source = ed.preview_source_lines(&p);
            r.preview_rows(&source, detail_rows, width.saturating_sub(2), 1)
        });
        if let Some(rows) = preview {
            for (i, row) in rows.iter().enumerate() {
                plain_row(
                    frame,
                    detail_y + i,
                    0,
                    width,
                    &format!("{} {}", if row.is_match { ">" } else { " " }, row.text),
                    if row.is_match {
                        Color::DarkCyan
                    } else {
                        Color::DarkGrey
                    },
                )?;
            }
        } else {
            let detail = r
                .entries
                .get(r.cursor)
                .map(|e| {
                    if e.detail.is_empty() {
                        e.text.as_str()
                    } else {
                        e.detail.as_str()
                    }
                })
                .unwrap_or("No results");
            for (i, line) in detail.lines().take(detail_rows).enumerate() {
                plain_row(
                    frame,
                    detail_y + i,
                    0,
                    width,
                    &format!("  {line}"),
                    Color::DarkGrey,
                )?;
            }
        }
    }
    let footer = if r.entries.iter().any(|e| e.note_id.is_some()) {
        "q close · e edit · R resolve · A agent · Tab select · y/Y copy · /? search · Ctrl-Q"
    } else if r.preview {
        "q close · Enter open · p preview off · w wrap · Ctrl-e/y scroll · Ctrl-Q quickfix"
    } else {
        "q close · Enter open · A agent · Tab select · y/Y copy · /? search · Ctrl-Q quickfix"
    };
    plain_row(frame, height - 2, 0, width, footer, Color::DarkBlue)?;
    plain_row(
        frame,
        height - 1,
        0,
        width,
        r.error.as_deref().unwrap_or(&ed.message),
        Color::Reset,
    )?;
    Ok(if r.search_input.is_some() {
        (clip(&prompt, width.saturating_sub(1)).width(), 1)
    } else {
        (0, (r.cursor - first + 2).min(height - 1))
    })
}
fn draw_picker(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    width: usize,
    height: usize,
) -> io::Result<(usize, usize)> {
    let Some(p) = &ed.file_picker else {
        return Ok((0, 0));
    };
    plain_row(
        frame,
        0,
        0,
        width,
        &format!("> {}", p.query),
        Color::DarkBlue,
    )?;
    let rows = height.saturating_sub(2);
    let start = p.selected.saturating_sub(rows.saturating_sub(1));
    for (i, (_, path)) in p.matches.iter().skip(start).take(rows).enumerate() {
        plain_row(
            frame,
            i + 1,
            0,
            width,
            path,
            if i + start == p.selected {
                Color::DarkCyan
            } else {
                Color::Reset
            },
        )?;
    }
    plain_row(
        frame,
        height - 1,
        0,
        width,
        &format!(
            "{} files{} · Enter open · Ctrl-Q quickfix · Esc close",
            p.matches.len(),
            if ed.search_job.files_rx.is_some() {
                " · scanning…"
            } else {
                ""
            }
        ),
        Color::DarkBlue,
    )?;
    Ok((
        clip(&format!("> {}", p.query), width.saturating_sub(1)).width(),
        0,
    ))
}
/// Renders the file tree sidebar: one row per visible node, indented by
/// depth, folders marked with `▸`/`▾` for collapsed/expanded. The
/// selected row is reverse-video only when this pane is active, matching
/// how the results list distinguishes focus.
/// The worst diagnostic severity under `path` -- for a file, its own
/// diagnostics; for a directory, any descendant's (even an unexpanded
/// one, since diagnostics are keyed by full path regardless of what the
/// lazily-built tree has loaded) -- rendered as the same E/W/I letters
/// the buffer gutter already uses.
pub(crate) fn tree_diagnostic_marker(
    ed: &Editor,
    path: &std::path::Path,
    is_dir: bool,
) -> Option<char> {
    let rank = |s: crate::lsp::Severity| match s {
        crate::lsp::Severity::Error => 0,
        crate::lsp::Severity::Warning => 1,
        _ => 2,
    };
    let worst = if is_dir {
        ed.diagnostics
            .iter()
            .filter(|(p, ds)| !ds.is_empty() && p.starts_with(path))
            .flat_map(|(_, ds)| ds.iter())
            .map(|d| d.severity)
            .min_by_key(|s| rank(*s))
    } else {
        ed.diagnostics
            .get(path)
            .into_iter()
            .flat_map(|ds| ds.iter())
            .map(|d| d.severity)
            .min_by_key(|s| rank(*s))
    };
    worst.map(|s| match s {
        crate::lsp::Severity::Error => 'E',
        crate::lsp::Severity::Warning => 'W',
        _ => 'I',
    })
}

/// A file's own `git status` letter, or (for a directory) a generic `*`
/// if any descendant has one -- `git_status` has no severity ordering
/// the way diagnostics do, so a directory doesn't try to pick a "worst"
/// specific letter among modified/added/untracked/etc, just flags that
/// something under it changed.
fn tree_git_marker(
    tree: &crate::filetree::FileTree,
    path: &std::path::Path,
    is_dir: bool,
) -> Option<char> {
    if is_dir {
        tree.git_status
            .keys()
            .any(|p| p.starts_with(path))
            .then_some('*')
    } else {
        tree.git_status.get(path).copied()
    }
}

fn draw_file_tree_pane(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    tree: &crate::filetree::FileTree,
    rect: Rect,
    active: bool,
) -> io::Result<Option<(usize, usize)>> {
    let mut cursor = None;
    for y in 0..rect.height {
        let Some(row) = frame.get_mut(rect.y + y) else {
            continue;
        };
        let text = match tree.nodes.get(y) {
            Some(n) => {
                let marker = if n.is_dir {
                    if tree.expanded.contains(&n.path) {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };
                let diag = tree_diagnostic_marker(ed, &n.path, n.is_dir)
                    .map(|c| format!(" {c}"))
                    .unwrap_or_default();
                let git = tree_git_marker(tree, &n.path, n.is_dir)
                    .map(|c| format!(" {c}"))
                    .unwrap_or_default();
                let bookmark = if tree.bookmarks.contains(&n.path) {
                    " \u{2605}"
                } else {
                    ""
                };
                format!(
                    "{}{}{}{}{}{}",
                    "  ".repeat(n.depth),
                    marker,
                    n.name,
                    diag,
                    git,
                    bookmark
                )
            }
            None => String::new(),
        };
        let selected = active && tree.cursor == y;
        if selected {
            cursor = Some((rect.x + 1, rect.y + y));
            queue!(
                row,
                MoveTo(rect.x as u16, (rect.y + y) as u16),
                SetAttribute(Attribute::Reverse),
                Print(pad(&text, rect.width)),
                SetAttribute(Attribute::NoReverse)
            )?;
        } else {
            queue!(
                row,
                MoveTo(rect.x as u16, (rect.y + y) as u16),
                Print(pad(&text, rect.width))
            )?;
        }
    }
    Ok(cursor)
}

/// Renders the outline/symbol sidebar: one row per symbol, indented by
/// nesting depth, prefixed with a collapse marker (for symbols that have
/// children) and its kind.
fn draw_outline_pane(
    frame: &mut [Vec<u8>],
    outline: &crate::outline::Outline,
    rect: Rect,
    active: bool,
) -> io::Result<Option<(usize, usize)>> {
    let mut cursor = None;
    for y in 0..rect.height {
        let Some(row) = frame.get_mut(rect.y + y) else {
            continue;
        };
        let text = match outline.nodes.get(y) {
            Some(n) => {
                let marker = if !outline.has_children(n) {
                    "  "
                } else if outline.collapsed.contains(&(n.name.clone(), n.line)) {
                    "▸ "
                } else {
                    "▾ "
                };
                format!("{}{}{} {}", "  ".repeat(n.depth), marker, n.kind, n.name)
            }
            None => String::new(),
        };
        // Highlighted independent of pane focus: follow-cursor (see
        // `Editor::ensure_outline_follow`) tracks the buffer's cursor while
        // the buffer pane, not the sidebar, has focus, and the highlight is
        // the whole point of that feature. The blinking terminal cursor
        // itself still only appears when this pane actually has focus.
        let selected = outline.cursor == y;
        if selected {
            if active {
                cursor = Some((rect.x + 1, rect.y + y));
            }
            queue!(
                row,
                MoveTo(rect.x as u16, (rect.y + y) as u16),
                SetAttribute(Attribute::Reverse),
                Print(pad(&text, rect.width)),
                SetAttribute(Attribute::NoReverse)
            )?;
        } else {
            queue!(
                row,
                MoveTo(rect.x as u16, (rect.y + y) as u16),
                Print(pad(&text, rect.width))
            )?;
        }
    }
    Ok(cursor)
}

fn vt100_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::AnsiValue(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb { r, g, b },
    }
}

/// Renders an embedded PTY's current screen into `rect`, returning the
/// pane-relative cursor position if the terminal's own cursor is visible.
/// Colors/bold/underline/inverse come straight from `vt100`'s parsed
/// attributes; this is a real terminal emulator's output, not a guess.
fn draw_terminal_pane(
    frame: &mut [Vec<u8>],
    pty: &crate::pty::PtySession,
    rect: Rect,
) -> io::Result<Option<(usize, usize)>> {
    let mut cursor = None;
    pty.with_screen(|screen| -> io::Result<()> {
        let (rows, cols) = screen.size();
        for y in 0..rect.height.min(rows as usize) {
            if let Some(row) = frame.get_mut(rect.y + y) {
                queue!(row, MoveTo(rect.x as u16, (rect.y + y) as u16))?;
                for x in 0..rect.width.min(cols as usize) {
                    let Some(cell) = screen.cell(y as u16, x as u16) else {
                        queue!(row, Print(" "))?;
                        continue;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let text = if cell.contents().is_empty() {
                        " ".to_string()
                    } else {
                        cell.contents().to_string()
                    };
                    queue!(
                        row,
                        SetForegroundColor(vt100_color(cell.fgcolor())),
                        SetBackgroundColor(vt100_color(cell.bgcolor())),
                        SetAttribute(if cell.bold() {
                            Attribute::Bold
                        } else {
                            Attribute::NormalIntensity
                        }),
                        SetAttribute(if cell.underline() {
                            Attribute::Underlined
                        } else {
                            Attribute::NoUnderline
                        }),
                        SetAttribute(if cell.inverse() {
                            Attribute::Reverse
                        } else {
                            Attribute::NoReverse
                        }),
                        Print(&text)
                    )?;
                }
                queue!(row, ResetColor, SetAttribute(Attribute::Reset))?;
            }
        }
        if !screen.hide_cursor() {
            let (cy, cx) = screen.cursor_position();
            if (cy as usize) < rect.height && (cx as usize) < rect.width {
                cursor = Some((rect.x + cx as usize, rect.y + cy as usize));
            }
        }
        Ok(())
    })?;
    Ok(cursor)
}

fn draw_preview_pane(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    b: &Buffer,
    w: &Window,
    r: Rect,
) -> io::Result<()> {
    if r.height == 0 || r.width == 0 {
        return Ok(());
    }
    let mut previews = ed.preview_panes.borrow_mut();
    let preview = previews
        .entry(b.id)
        .or_insert_with(crate::markdown::Preview::new);
    let width = r.width.saturating_sub(2).max(1);
    if preview.needs_refresh(b.edit_seq, width) {
        preview.refresh(&b.rope.to_string(), b.edit_seq, width);
    }
    for row in 0..r.height.saturating_sub(1) {
        let y = r.y + row;
        let mut used = 0;
        queue!(frame[y], MoveTo(r.x as u16, y as u16))?;
        if let Some(line) = preview.lines.get(w.preview_scroll + row) {
            for span in line {
                let text = clip(&span.text, r.width.saturating_sub(used));
                used += text.width();
                let color = if let Some(class) = span.style.syntax {
                    match class {
                        crate::syntax::HlClass::Keyword => Color::Cyan,
                        crate::syntax::HlClass::Number => Color::Magenta,
                        crate::syntax::HlClass::String => Color::Green,
                        crate::syntax::HlClass::Comment => Color::DarkGrey,
                    }
                } else if span.style.heading > 0 {
                    Color::Cyan
                } else if span.style.code_block || span.style.inline_code {
                    Color::Green
                } else if span.style.dim {
                    Color::DarkGrey
                } else {
                    Color::Reset
                };
                queue!(frame[y], SetForegroundColor(color))?;
                if span.style.bold {
                    queue!(frame[y], SetAttribute(Attribute::Bold))?;
                }
                if span.style.italic {
                    queue!(frame[y], SetAttribute(Attribute::Italic))?;
                }
                if span.style.strike {
                    queue!(frame[y], SetAttribute(Attribute::CrossedOut))?;
                }
                queue!(
                    frame[y],
                    Print(text),
                    ResetColor,
                    SetAttribute(Attribute::Reset)
                )?;
            }
        }
        queue!(frame[y], Print(" ".repeat(r.width.saturating_sub(used))))?;
    }
    plain_row(
        frame,
        r.y + r.height - 1,
        r.x,
        r.width,
        &format!(
            " PREVIEW · {}",
            b.path
                .as_ref()
                .and_then(|p| p.file_name())
                .unwrap_or_default()
                .to_string_lossy()
        ),
        Color::DarkBlue,
    )?;
    let _ = ed;
    Ok(())
}
fn draw_full_preview(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    width: usize,
    height: usize,
) -> io::Result<()> {
    let mut w = ed.capture_window();
    w.preview = true;
    w.preview_scroll = ed.markdown_preview.as_ref().map(|p| p.scroll).unwrap_or(0);
    draw_preview_pane(
        frame,
        ed,
        ed.buf(),
        &w,
        Rect {
            x: 0,
            y: 0,
            width,
            height: height.saturating_sub(1),
        },
    )?;
    plain_row(
        frame,
        height - 1,
        0,
        width,
        "q/Esc close · j/k scroll · :vpreview for side-by-side",
        Color::Reset,
    )
}
pub fn setup_terminal() -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    if let Err(e) = execute!(
        io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::event::EnableMouseCapture,
        Hide
    ) {
        let _ = crossterm::terminal::disable_raw_mode();
        return Err(e);
    }
    Ok(())
}
pub fn teardown_terminal() -> io::Result<()> {
    let result = execute!(
        io::stdout(),
        Show,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen
    );
    let raw = crossterm::terminal::disable_raw_mode();
    result.and(raw)
}

/// Maps a terminal cell (`x`, `y`, both 0-based) to the pane index, buffer
/// line and char column it displays, or `None` if it's outside any pane
/// (a border, the message line, or a non-editing mode like Results). Reuses
/// the exact layout `draw` uses, so a click always lands where the
/// character it's drawn on top of actually is.
pub fn locate_click(
    ed: &Editor,
    cols: usize,
    rows: usize,
    x: usize,
    y: usize,
) -> Option<(usize, usize, usize)> {
    if !matches!(ed.mode, Mode::Normal | Mode::Insert | Mode::Visual(_)) {
        return None;
    }
    let rects = ed.pane_rects(cols, rows);
    let (pane, rect) = rects
        .iter()
        .enumerate()
        .find(|(_, r)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)?;
    let w = if ed.windows.is_empty() {
        ed.capture_window()
    } else {
        ed.windows[pane].clone()
    };
    let b = ed.buffers.iter().find(|b| b.id == w.buffer)?;
    let gw = gutter(ed, b, rect.width);
    let pane_width = rect.width.saturating_sub(gw).max(1);
    let (display, _) = layout(ed, b, &w, pane_width, rect.height.saturating_sub(1));
    let row_in_pane = y.checked_sub(rect.y)?;
    let d = display.get(row_in_pane)?;
    let x_off = x.saturating_sub(rect.x + gw);
    let mut col = d
        .glyphs
        .iter()
        .find(|g| x_off < g.cell + g.width)
        .map(|g| g.col);
    if col.is_none() {
        col = d.glyphs.last().map(|g| g.col + 1);
    }
    Some((pane, d.line, col.unwrap_or(0)))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn render_row(bg: Color) -> Vec<u8> {
        let mut rows = vec![Vec::new()];
        plain_row(&mut rows, 0, 0, 5, "hi", bg).unwrap();
        rows.into_iter().next().unwrap()
    }

    /// crossterm's ANSI backend renders named colors as 256-color-palette
    /// SGR codes (`38;5;<n>` foreground / `48;5;<n>` background), not the
    /// basic 16-color `30-37`/`40-47` range -- confirmed empirically
    /// rather than assumed, since guessing wrong here would make these
    /// tests vacuously pass regardless of whether the underlying bug
    /// (forcing White-on-Reset) was actually present.
    const WHITE_FG: &str = "38;5;15";
    const DARK_CYAN_BG: &str = "48;5;6";

    #[test]
    fn plain_row_leaves_the_default_foreground_alone_on_a_reset_background() {
        let text = String::from_utf8_lossy(&render_row(Color::Reset)).into_owned();
        assert!(
            !text.contains(WHITE_FG),
            "a Reset background must not force a White foreground -- that \
             renders as invisible white-on-white on a light-background \
             terminal theme. Got: {text:?}"
        );
    }

    #[test]
    fn plain_row_forces_a_contrasting_foreground_on_an_explicit_background() {
        let text = String::from_utf8_lossy(&render_row(Color::DarkCyan)).into_owned();
        assert!(
            text.contains(DARK_CYAN_BG) && text.contains(WHITE_FG),
            "an explicit highlight background (e.g. a selected row) should \
             still force White text for contrast. Got: {text:?}"
        );
    }
}
