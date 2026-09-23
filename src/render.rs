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
        SetUnderlineColor,
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
/// Background for the `colorcolumn` ruler (a dark red, distinct from the
/// cursorline tint so the two are visible together).
const COLORCOLUMN_BG: Color = Color::AnsiValue(52);
/// Background for a differing line in diff mode (a dark green).
const DIFF_BG: Color = Color::AnsiValue(22);
/// Maps an LSP semantic token type name to a palette index, or `None` to leave
/// the base (tree-sitter) color (e.g. variables/parameters we don't recolor).
pub(crate) fn semantic_index(name: &str) -> Option<u8> {
    Some(match name {
        "keyword" | "modifier" => 0,
        "type" | "class" | "struct" | "enum" | "interface" | "typeParameter" | "namespace" => 1,
        "function" | "method" | "macro" | "decorator" => 2,
        "string" => 3,
        "comment" => 4,
        "number" => 5,
        _ => return None,
    })
}

/// Resolves a semantic-token palette index to a color (theme-aware).
fn semantic_color(ed: &Editor, index: u8) -> Color {
    match index {
        0 => ed.theme.keyword,
        1 => Color::Yellow,
        2 => Color::Blue,
        3 => ed.theme.string,
        4 => ed.theme.comment,
        5 => ed.theme.number,
        _ => Color::Reset,
    }
}

/// Background for sticky-scroll context header rows.
const STICKY_BG: Color = Color::AnsiValue(238);
/// Background tint for inccommand live substitute-preview overlay rows.
const INCCOMMAND_BG: Color = Color::AnsiValue(23);
/// Background for the per-pane winbar (path + breadcrumb) top row.
const WINBAR_BG: Color = Color::AnsiValue(237);
/// Background tint for a closed fold's summary (foldtext) row.
const FOLD_BG: Color = Color::AnsiValue(238);
/// Total width of the minimap strip (separator column + body).
const MINIMAP_W: usize = 12;
/// Background tint for the minimap rows covering the current viewport.
const MINIMAP_VIEW_BG: Color = Color::AnsiValue(238);
/// Source-column span a minimap body maps across (lines wider than this
/// just fill to the right edge). Roughly a conventional code width.
const MINIMAP_SCALE: usize = 80;
/// Node kinds shown in the sticky-scroll header (functions, classes/impls).
const STICKY_KINDS: &[&str] = &[
    "function_item",
    "function_declaration",
    "function_definition",
    "method_declaration",
    "method_definition",
    "impl_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "mod_item",
    "class_declaration",
    "class_definition",
    "interface_declaration",
];
/// Foreground palette for rainbow brackets, cycled by nesting depth.
const RAINBOW: &[Color] = &[
    Color::Yellow,
    Color::Magenta,
    Color::Cyan,
    Color::Green,
    Color::Blue,
    Color::Red,
    Color::DarkYellow,
];

/// Bracket positions `(line, col, depth-palette-index)` for the whole buffer,
/// colored so a matching `(`/`)` pair shares a depth. Cached per `(buffer,
/// edit_seq)`; commas/strings are not skipped (a simple raw scan).
pub(crate) fn rainbow_brackets(ed: &Editor, b: &Buffer) -> std::rc::Rc<Vec<(usize, usize, u8)>> {
    if let Some((bid, seq, v)) = ed.rainbow_cache.borrow().as_ref() {
        if *bid == b.id && *seq == b.edit_seq {
            return v.clone();
        }
    }
    let n = RAINBOW.len() as i32;
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let (mut line, mut col) = (0usize, 0usize);
    // Brackets inside strings and comments aren't delimiters: skip them (and
    // don't let them affect nesting depth). Only the current buffer has a live
    // tree here, so filtering applies there; other panes color all brackets.
    let skip_ranges: Vec<(usize, usize)> = if b.id == ed.buf().id {
        ed.syntax
            .as_ref()
            .map(|syn| {
                syn.spans_in(0, b.rope.len_bytes())
                    .filter(|(_, _, c)| {
                        matches!(c, crate::syntax::HlClass::Comment | crate::syntax::HlClass::String)
                    })
                    .map(|(s, e, _)| (s, e))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let in_skip = |bpos: usize| skip_ranges.iter().any(|&(s, e)| bpos >= s && bpos < e);
    let mut bytepos = 0usize;
    for ch in b.rope.chars() {
        let blen = ch.len_utf8();
        match ch {
            '\n' => {
                line += 1;
                col = 0;
                bytepos += blen;
                continue;
            }
            '(' | '[' | '{' if !in_skip(bytepos) => {
                out.push((line, col, depth.rem_euclid(n) as u8));
                depth += 1;
            }
            ')' | ']' | '}' if !in_skip(bytepos) => {
                depth = (depth - 1).max(0);
                out.push((line, col, depth.rem_euclid(n) as u8));
            }
            _ => {}
        }
        col += 1;
        bytepos += blen;
    }
    let rc = std::rc::Rc::new(out);
    *ed.rainbow_cache.borrow_mut() = Some((b.id, b.edit_seq, rc.clone()));
    rc
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
    /// Hash of the closed folds, so toggling a fold invalidates the cache.
    folds: u64,
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
            // Genuinely zero-width graphemes (lone combining marks, ZWJ,
            // ZWSP) must stay width 0 -- forcing them to 1 causes premature
            // wrap and cursor drift. A base char + combining mark grapheme
            // still measures the base's width here, and control chars were
            // already remapped to a width-1 replacement above, so nothing
            // that needs a cell ends up at width 0.
            let width = UnicodeWidthStr::width(text.as_str());
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
    clip_tab(text, width, 4)
}
/// `clip` with an explicit tab width, so buffer-derived rows expand tabs at
/// the buffer's own tabstop instead of a hardcoded 4.
fn clip_tab(text: &str, width: usize, tab: usize) -> String {
    let mut s = String::new();
    let mut cells = 0;
    for g in glyphs(text, tab) {
        if cells + g.width > width {
            break;
        }
        s.push_str(&g.text);
        cells += g.width;
    }
    s
}
fn pad(text: &str, width: usize) -> String {
    pad_tab(text, width, 4)
}
fn pad_tab(text: &str, width: usize, tab: usize) -> String {
    let s = clip_tab(text, width, tab);
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
/// Parse a Vim-style `listchars` string (`tab:xy,trail:z`) into
/// `(tab_lead, tab_fill, trail)`, falling back to `>`,`-`,`·` for anything
/// missing or malformed.
pub(crate) fn parse_listchars(s: &str) -> (char, char, char) {
    let (mut lead, mut fill, mut trail) = ('>', '-', '·');
    for item in s.split(',') {
        let Some((key, val)) = item.split_once(':') else {
            continue;
        };
        let mut chars = val.chars();
        match key.trim() {
            "tab" => {
                if let Some(a) = chars.next() {
                    lead = a;
                    fill = chars.next().unwrap_or(a);
                }
            }
            "trail" => {
                if let Some(a) = chars.next() {
                    trail = a;
                }
            }
            _ => {}
        }
    }
    (lead, fill, trail)
}
/// Parse a Vim-style `fillchars` string (`eob:x,vert:y`) into
/// `(end_of_buffer, vertical_separator)`, defaulting to `~` and `│`.
pub(crate) fn parse_fillchars(s: &str) -> (char, char) {
    let (mut eob, mut vert) = ('~', '│');
    for item in s.split(',') {
        let Some((key, val)) = item.split_once(':') else {
            continue;
        };
        let Some(c) = val.chars().next() else { continue };
        match key.trim() {
            "eob" => eob = c,
            "vert" => vert = c,
            _ => {}
        }
    }
    (eob, vert)
}
fn gutter(ed: &Editor, b: &Buffer, width: usize) -> usize {
    if ed.zen {
        return 0; // focus mode: no line-number/sign gutter
    }
    let foldcol = if ed.config.foldcolumn { 1 } else { 0 };
    (number_width(ed, b) + 2 + foldcol).min(width.saturating_sub(1))
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
            if cells >= left {
                // Once a glyph doesn't fit at the right margin, stop adding
                // glyphs entirely -- continuing would let a later narrower
                // glyph slip into the gap a skipped wide glyph left, which
                // breaks the cell-to-source-column correspondence (e.g.
                // "AB你C" in width 3 must clip to "AB", not "ABC").
                if used + g.width > width {
                    break;
                }
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
        folds: b.folds_stamp(),
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
        // Skip lines hidden inside a closed fold (all but the fold's first row).
        if b.line_hidden(line) {
            continue;
        }
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
    // A cursor left inside a closed fold (by any motion that isn't fold-aware)
    // snaps to the fold's first, visible line before the viewport is captured.
    ed.clamp_cursor_folds();
    ed.store_window();
    let rects = ed.pane_rects(cols, rows);
    // Keep the sidebar viewports following their cursors (the panes scroll
    // independently of the buffer). Each sidebar knows only its own cursor;
    // the pane height lives here in the render pipeline.
    let tree_h = ed
        .windows
        .iter()
        .zip(&rects)
        .find_map(|(w, r)| w.file_tree.then_some(r.height));
    if let (Some(h), Some(t)) = (tree_h, ed.file_tree.as_mut()) {
        t.ensure_visible(h);
    }
    let outline_h = ed
        .windows
        .iter()
        .zip(&rects)
        .find_map(|(w, r)| w.outline.then_some(r.height));
    if let (Some(h), Some(o)) = (outline_h, ed.outline.as_mut()) {
        o.ensure_visible(h);
    }
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
/// (selected, searched, doc-highlighted, foreground color, diagnostic
/// underline color) for one glyph run in a rendered row.
type GlyphStyle = (bool, bool, bool, bool, Color, Option<Color>, bool, bool);
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
    /// The active window's cursor line with `cursorline` on — this row gets a
    /// tinted background. Part of the cache key so it repaints as the cursor
    /// moves and differs between an active and an inactive split.
    cursorline: bool,
    /// This row is a differing line in diff mode (row-level highlight).
    diff_line: bool,
    /// The `colorcolumn` ruler column (0 = off). In the key so toggling or
    /// moving the ruler repaints cached rows.
    colorcolumn: usize,
    /// `list` (listchars) state — in the key so toggling repaints cached rows.
    list: bool,
    relative: Option<usize>,
    selection: Selection,
    search: Option<(String, bool, bool)>,
    /// Char-column ranges on this exact line from `document_highlights`
    /// (see `draw_pane`'s `doc_highlighted` gate and range-clipping
    /// comment), so the row cache invalidates when they change.
    doc_ranges: Vec<(usize, usize)>,
    /// Document-color literal spans on this row with their RGB, so a color
    /// change (or new documentColor response) repaints the row.
    color_ranges: Vec<(usize, usize, (u8, u8, u8))>,
    /// Rainbow bracket `(col, depth)` on this row (empty when disabled).
    rainbow: Vec<(usize, u8)>,
    /// Semantic-token `(start, end, palette)` spans on this row.
    sem_ranges: Vec<(usize, usize, u8, bool)>,
    /// Misspelled-word char ranges on this row (for the spell underline).
    spell_ranges: Vec<(usize, usize)>,
    /// TODO/FIXME/etc. keyword ranges on this row `(start, end, color index)`.
    todo_ranges: Vec<(usize, usize, u8)>,
    /// The line-blame virtual text for this exact row, when `blame_toggle`
    /// is on and this is the buffer's current line -- `None` otherwise,
    /// so the cache invalidates correctly across toggling, cursor moves,
    /// and the async blame data first arriving.
    blame: Option<String>,
    /// This row's code-lens virtual text (titles of every lens whose range
    /// starts on this line, joined), or `None` -- see the `code_lens`
    /// local in `draw_pane` for why it's computed once per row rather
    /// than filtered out of `ed.code_lenses` at paint time.
    code_lens: Option<String>,
    /// `(col, label)` inlay hints on this exact row, sorted by `col` --
    /// see the `line_hints` local in `draw_pane` for where these get
    /// spliced into the glyph run instead of appended after it.
    inlay_hints: Vec<(usize, String)>,
    /// Char-column ranges on this exact row carrying a diagnostic, with
    /// its severity (for the underline color) -- see `diag_ranges` in
    /// `draw_pane`, built the same clip-to-this-line way `doc_ranges`
    /// already is.
    diag_ranges: Vec<(usize, usize, crate::lsp::Severity)>,
    /// This row's own diagnostic message as virtual text (current line
    /// only, and only when `diagnostics_virtual_text` and the
    /// Insert-mode update policy both allow it right now) -- `None`
    /// otherwise.
    diag_text: Option<String>,
    marker: char,
    sign: char,
    /// Foldcolumn marker for this row (`+`/`-`/space, or `\0` when off) -- in
    /// the key so toggling a fold repaints its start row's gutter marker.
    fold_marker: char,
    /// `,gd` diff overlay: this row's changed-word char-column ranges
    /// and any HEAD line(s) removed immediately before it -- see the
    /// `word_diff_ranges`/`deleted_before` locals in `draw_pane`.
    word_diff_ranges: Vec<(usize, usize)>,
    deleted_before: Vec<String>,
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
    /// A Results-list or file-picker preview pane's source lines, keyed by
    /// path and validated against the file's own mtime -- without this,
    /// `cached_preview_source` would re-read and re-split the whole file
    /// from disk on every single frame the preview stays open on it (a
    /// cursor move within the *same* file, an unrelated redraw elsewhere
    /// on screen, scrolling the preview itself), not just when the
    /// selection actually moves to a different file. Cleared outright
    /// once it grows past a small bound rather than tracked as an LRU --
    /// the same simple "clear when it gets too big" policy `LayoutCache`
    /// already uses for its own per-line cache.
    preview_source:
        std::collections::HashMap<std::path::PathBuf, (std::time::SystemTime, Vec<String>)>,
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
            preview_source: Default::default(),
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
/// Overlays live toast notifications (newest at top) in the top-right corner,
/// each on its own row over the pane content. No-op when disabled/empty.
fn draw_toasts(frame: &mut [Vec<u8>], ed: &Editor, width: usize, height: usize) -> io::Result<()> {
    if !ed.config.notifications || ed.toasts.is_empty() || width < 12 || height < 2 {
        return Ok(());
    }
    let ttl = std::time::Duration::from_secs(4);
    let live: Vec<&String> = ed
        .toasts
        .iter()
        .filter(|(t, _)| t.elapsed() < ttl)
        .map(|(_, m)| m)
        .collect();
    // Newest first, capped so toasts never cover the whole screen.
    let show = live.len().min(5).min(height.saturating_sub(1));
    let box_w = (width / 3).clamp(20, 50).min(width.saturating_sub(2));
    for (i, msg) in live.iter().rev().take(show).enumerate() {
        let text = format!(" {} ", clip(msg, box_w.saturating_sub(2)));
        plain_row(
            frame,
            i,
            width - box_w,
            box_w,
            &text,
            Color::DarkCyan,
        )?;
    }
    Ok(())
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

/// Compress one source line into a `w`-cell minimap silhouette: a bar of
/// block glyphs spanning from the line's first to its last non-whitespace
/// column, source columns `0..MINIMAP_SCALE` scaled across the width. Blank
/// (all spaces) lines — and cells outside the bar — render as spaces, so the
/// result reads as the file's indentation/length shape. Always exactly `w`
/// display cells wide (block and space are both width 1).
fn minimap_shape(src: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    let chars: Vec<char> = src.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
    let lo = chars.iter().position(|c| !c.is_whitespace());
    let hi = chars.iter().rposition(|c| !c.is_whitespace());
    let (lo, hi) = match (lo, hi) {
        (Some(lo), Some(hi)) => (lo, hi),
        _ => return " ".repeat(w),
    };
    let mut out = String::with_capacity(w);
    for i in 0..w {
        let col = i * MINIMAP_SCALE / w;
        out.push(if col >= lo && col <= hi { '▪' } else { ' ' });
    }
    out
}

/// Draw the minimap strip into the reserved right-hand columns of a pane: a
/// dim vertical separator followed by a per-row `minimap_shape`, with the
/// rows covering the current viewport (the logical lines on screen) tinted.
#[allow(clippy::too_many_arguments)]
fn draw_minimap(
    frame: &mut [Vec<u8>],
    b: &Buffer,
    display: &[DisplayRow],
    r: Rect,
    gw: usize,
    width: usize,
    map_w: usize,
    n: usize,
    top_off: usize,
) -> io::Result<()> {
    let total = b.line_count();
    let map_x = r.x + gw + width;
    let body_w = map_w.saturating_sub(1);
    let vis_first = display.first().map(|d| d.line).unwrap_or(0);
    let vis_last = display.last().map(|d| d.line).unwrap_or(0);
    for row in 0..n {
        let y = r.y + top_off + row;
        // Linear scale: minimap row -> source line. When the file fits, one
        // source line per minimap row; otherwise proportional.
        let line = if total <= n { row } else { row * total / n };
        let Some(dest) = frame.get_mut(y) else { continue };
        queue!(
            dest,
            MoveTo(map_x as u16, y as u16),
            SetForegroundColor(Color::DarkGrey),
            Print("│"),
            ResetColor
        )?;
        if line >= total {
            queue!(dest, Print(" ".repeat(body_w)))?;
            continue;
        }
        let in_view = line >= vis_first && line <= vis_last;
        let shape = minimap_shape(&b.line_text(line), body_w);
        if in_view {
            queue!(dest, SetBackgroundColor(MINIMAP_VIEW_BG))?;
        }
        queue!(
            dest,
            SetForegroundColor(Color::Grey),
            Print(&shape),
            ResetColor
        )?;
    }
    Ok(())
}

/// Draw the winbar into a pane's top row: the buffer's project-relative path
/// and, when a tree-sitter tree is available for the current buffer, the
/// enclosing function/class declaration as a breadcrumb.
fn draw_winbar(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    b: &Buffer,
    w: &Window,
    r: Rect,
    _gw: usize,
) -> io::Result<()> {
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
    let mut bar = format!(" {name}");
    if b.id == ed.buf().id {
        if let Some(syn) = &ed.syntax {
            let total = b.rope.len_bytes();
            let cursor_byte = b.line_byte_range(w.cursor.0).0.min(total);
            if let Some(&sb) = syn.context_starts(cursor_byte, STICKY_KINDS).last() {
                let ci = b.rope.byte_to_char(sb.min(total));
                let line = b.pos_from_char_idx(ci).0;
                let sym = b.line_text(line).trim().to_string();
                if !sym.is_empty() {
                    bar.push_str("  ›  ");
                    bar.push_str(&sym);
                }
            }
        }
    }
    let shown = clip_tab(&bar, r.width, b.tabstop);
    plain_row(frame, r.y, r.x, r.width, &shown, WINBAR_BG)?;
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
        cursor = draw_results(&mut frame, ed, cache, width, height)?;
        bar = ed
            .results
            .as_ref()
            .is_some_and(|r| r.search_input.is_some());
    } else if matches!(ed.mode, Mode::Picker) {
        cursor = draw_picker(&mut frame, ed, cache, width, height)?;
        bar = true;
    } else if matches!(ed.mode, Mode::MarkdownPreview) {
        draw_full_preview(&mut frame, ed, width, height)?;
    } else {
        if ed.tabs.len() > 1 {
            draw_tabline(&mut frame, ed, width)?;
        }
        let (_eob, vert) = parse_fillchars(&ed.config.fillchars);
        let vert_s = vert.to_string();
        let rects = ed.pane_rects(width, height);
        for (i, rect) in rects.iter().copied().enumerate() {
            if rect.x + rect.width < width {
                for y in rect.y..rect.y + rect.height {
                    plain_row(&mut frame, y, rect.x + rect.width, 1, &vert_s, Color::DarkGrey)?;
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
        // Wildmenu: the Tab-completion candidates, shown on the row above the
        // command line with the selected one bracketed.
        if matches!(ed.mode, Mode::Command(CommandKind::Ex))
            && ed.cmdline_completion_index.is_some()
            && !ed.cmdline_completions.is_empty()
            && height >= 2
        {
            let sel = ed.cmdline_completion_index.unwrap();
            let toks: Vec<String> = ed
                .cmdline_completions
                .iter()
                .enumerate()
                .map(|(i, full)| {
                    let tok = full.rsplit(' ').next().unwrap_or(full);
                    if i == sel {
                        format!("[{tok}]")
                    } else {
                        tok.to_string()
                    }
                })
                .collect();
            let row = toks.join("  ");
            plain_row(&mut frame, height - 2, 0, width, &clip(&row, width), Color::DarkBlue)?;
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
    draw_toasts(&mut frame, ed, width, height)?;
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
    // Minimap: reserve a fixed strip on the right for a compressed overview,
    // but only when the pane is wide enough to keep a usable content column.
    let map_w = if ed.config.minimap && r.width.saturating_sub(gw) > MINIMAP_W * 2 {
        MINIMAP_W
    } else {
        0
    };
    let width = r.width.saturating_sub(gw).saturating_sub(map_w).max(1);
    // Winbar reserves the pane's top row (never in zen, and only when there's
    // room to keep at least one content row). Content is then offset down by it.
    let top_off = if ed.config.winbar && !ed.zen && r.height > 2 {
        1
    } else {
        0
    };
    // Zen mode reclaims the per-pane status row for buffer content.
    let n = if ed.zen {
        r.height
    } else {
        r.height.saturating_sub(1).saturating_sub(top_off)
    };
    let (display, cursor) = layout(ed, b, w, width, n);
    let mut source_cache = std::collections::HashMap::new();
    // Prefer the in-progress incsearch pattern (live `/`/`?` preview) over the
    // last submitted search when highlighting.
    let search = ed
        .incsearch
        .clone()
        .or_else(|| {
            ed.last_search
                .as_ref()
                .filter(|_| ed.hl_search)
                .map(|(p, _)| p.clone())
        })
        .and_then(|p| {
            crate::search::compile(&p, ed.config.ignorecase, ed.config.smartcase).ok()
        });
    // Only paint `document_highlights` while they're still for this exact
    // buffer and it hasn't been edited since the request -- a stale set
    // would otherwise highlight whatever now sits at those old positions.
    let doc_highlighted = ed.document_highlights_buffer == Some(b.id)
        && ed.document_highlights_edit_seq == b.edit_seq;
    let colors_live = ed.document_colors_buffer == Some(b.id)
        && ed.document_colors_edit_seq == b.edit_seq;
    let sem_live = ed.config.semantic_tokens
        && ed.semantic_tokens_buffer == Some(b.id)
        && ed.semantic_tokens_edit_seq == b.edit_seq;
    let large = ed.config.large_file_kb > 0 && b.rope.len_bytes() > ed.config.large_file_kb * 1024;
    let (lc_lead, lc_fill, lc_trail) = parse_listchars(&ed.config.listchars);
    let (eob_char, _) = parse_fillchars(&ed.config.fillchars);
    let eob = eob_char.to_string();
    let rainbow = if ed.config.rainbow && !large {
        Some(rainbow_brackets(ed, b))
    } else {
        None
    };
    let spell_live = ed.spell_spans_buffer == Some(b.id) && ed.spell_spans_edit_seq == b.edit_seq;
    let todo_live = ed.todo_spans_buffer == Some(b.id) && ed.todo_spans_edit_seq == b.edit_seq;
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
        let y = r.y + top_off + row;
        let dest = &mut target.frame[y];
        queue!(dest, MoveTo(r.x as u16, y as u16))?;
        let content_start = dest.len();
        let Some(d) = display.get(row) else {
            queue!(
                dest,
                SetForegroundColor(Color::DarkGrey),
                Print(pad(&eob, r.width)),
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
        // `,gd`'s diff overlay: char-column ranges to highlight as a
        // changed word, and HEAD content of any line(s) removed
        // immediately before this one -- both straight from the same
        // `GitGutter` data the gutter sign above already reads, gated
        // the same "only for the current buffer" way.
        let (word_diff_ranges, deleted_before): (Vec<(usize, usize)>, Vec<String>) =
            if ed.diff_overlay && b.id == ed.buf().id {
                let git = ed.git.as_ref();
                (
                    git.and_then(|g| g.word_diff.get(&d.line))
                        .cloned()
                        .unwrap_or_default(),
                    git.and_then(|g| g.deleted_before.get(&d.line))
                        .cloned()
                        .unwrap_or_default(),
                )
            } else {
                (Vec::new(), Vec::new())
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
        // Foldcolumn marker: `+` where a closed fold starts, `-` where an open
        // fold starts, blank otherwise. `\0` when the foldcolumn is off.
        let fold_marker = if ed.config.foldcolumn {
            if b.closed_fold_starting_at(d.line).is_some() {
                '+'
            } else if b.folds.iter().any(|f| !f.closed && f.start == d.line) {
                '-'
            } else {
                ' '
            }
        } else {
            '\0'
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
        // Document-color literals on this row (single-line spans), each with
        // its own RGB — the glyphs in the span are painted in that color.
        let color_ranges: Vec<(usize, usize, (u8, u8, u8))> = if colors_live {
            ed.document_colors
                .iter()
                .filter(|&&(l, _, _, _)| l == d.line)
                .map(|&(_, c1, c2, rgb)| (c1, c2, rgb))
                .collect()
        } else {
            Vec::new()
        };
        // Misspelled-word char ranges on this row (for the spell underline).
        let spell_ranges: Vec<(usize, usize)> = if spell_live {
            ed.spell_spans
                .iter()
                .filter(|&&(l, _, _)| l == d.line)
                .map(|&(_, s, e)| (s, e))
                .collect()
        } else {
            Vec::new()
        };
        // TODO/FIXME/etc. keyword ranges on this row (start, end, color index).
        let todo_ranges: Vec<(usize, usize, u8)> = if todo_live {
            ed.todo_spans
                .iter()
                .filter(|&&(l, _, _, _)| l == d.line)
                .map(|&(_, s, e, c)| (s, e, c))
                .collect()
        } else {
            Vec::new()
        };
        // Semantic-token spans on this row: (start_col, end_col, palette).
        let sem_row: Vec<(usize, usize, u8, bool)> = if sem_live {
            ed.semantic_tokens
                .iter()
                .filter(|&&(l, _, _, _, _)| l == d.line)
                .map(|&(_, c1, c2, p, dep)| (c1, c2, p, dep))
                .collect()
        } else {
            Vec::new()
        };
        // Rainbow bracket colors on this row: (col, palette index).
        let rainbow_row: Vec<(usize, u8)> = rainbow
            .as_ref()
            .map(|v| {
                v.iter()
                    .filter(|&&(l, _, _)| l == d.line)
                    .map(|&(_, c, depth)| (c, depth))
                    .collect()
            })
            .unwrap_or_default();
        // Every diagnostic whose line range covers this row, clipped to
        // it the same multi-line way `doc_ranges` above already is --
        // `Diagnostic::col`/`end_col` are raw UTF-16 units (parsed once,
        // outside any buffer context), so they're converted here against
        // this row's own text, the same as every other per-line LSP
        // column already is in this file.
        let diag_ranges: Vec<(usize, usize, crate::lsp::Severity)> = b
            .path
            .as_ref()
            .and_then(|p| ed.diagnostics.get(p))
            .map(|ds| {
                ds.iter()
                    .filter(|d2| d.line >= d2.line && d.line <= d2.end_line)
                    .map(|d2| {
                        let text = b.line_text(d.line);
                        let (c1, c2) = if d2.line == d2.end_line {
                            (
                                crate::language::utf16_to_col(&text, d2.col),
                                crate::language::utf16_to_col(&text, d2.end_col),
                            )
                        } else if d.line == d2.line {
                            (crate::language::utf16_to_col(&text, d2.col), usize::MAX)
                        } else if d.line == d2.end_line {
                            (0, crate::language::utf16_to_col(&text, d2.end_col))
                        } else {
                            (0, usize::MAX)
                        };
                        (c1, c2, d2.severity)
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The cursor's own line's diagnostic message, spelled out in
        // full (not just the gutter's bare E/W/I marker) -- deliberately
        // *not* re-gated on Insert mode here: `diagnostics_update_in_insert`
        // already decided, at the data level (see
        // `Editor::refresh_visible_diagnostics`), whether `ed.diagnostics`
        // itself reflects the newest server data yet; whatever it
        // currently holds should render normally either way, matching
        // Neovim's own update_in_insert semantics (existing diagnostics
        // stay visible while typing, only *new* ones wait).
        let diag_text =
            (ed.config.diagnostics_virtual_text && d.line == w.cursor.0 && b.id == ed.buf().id)
                .then(|| {
                    diag.map(|d2| {
                        format!(
                            "{:?}{}: {}",
                            d2.severity,
                            crate::lsp::code_source_label(d2),
                            d2.message
                        )
                    })
                })
                .flatten();
        let blame = (ed.blame_toggle
            && b.id == ed.buf().id
            && d.line == w.cursor.0
            && ed.line_blame_path.as_ref() == b.path.as_ref())
        .then_some(ed.line_blame.as_ref())
        .flatten()
        .and_then(|lines| lines.get(d.line))
        .cloned();
        let code_lens =
            if ed.code_lenses_buffer == Some(b.id) && ed.code_lenses_edit_seq == b.edit_seq {
                let titles: Vec<&str> = ed
                    .code_lenses
                    .iter()
                    .filter(|(line, _, _)| *line == d.line)
                    .map(|(_, title, _)| title.as_str())
                    .collect();
                (!titles.is_empty()).then(|| titles.join(" · "))
            } else {
                None
            };
        let mut line_hints: Vec<(usize, String)> =
            if ed.inlay_hints_buffer == Some(b.id) && ed.inlay_hints_edit_seq == b.edit_seq {
                ed.inlay_hints
                    .iter()
                    .filter(|(line, _, _)| *line == d.line)
                    .map(|(_, col, label)| (*col, label.clone()))
                    .collect()
            } else {
                Vec::new()
            };
        line_hints.sort_by_key(|(col, _)| *col);
        let cursorline = active && ed.config.cursorline && d.line == w.cursor.0;
        let diff_line = ed
            .diff_lines
            .get(&b.id)
            .is_some_and(|s| s.contains(&d.line));
        // A single row-level background: diff highlight wins over cursorline.
        let row_bg: Option<Color> = if diff_line {
            Some(DIFF_BG)
        } else if cursorline {
            Some(ed.theme.cursorline_bg)
        } else {
            None
        };
        let sig = RowSignature {
            buffer: b.id,
            content: d.content,
            syntax: if b.id == ed.buf().id {
                ed.syntax_stamp
            } else {
                0
            },
            doc_ranges: doc_ranges.clone(),
            color_ranges: color_ranges.clone(),
            rainbow: rainbow_row.clone(),
            sem_ranges: sem_row.clone(),
            spell_ranges: spell_ranges.clone(),
            todo_ranges: todo_ranges.clone(),
            blame: blame.clone(),
            code_lens: code_lens.clone(),
            inlay_hints: line_hints.clone(),
            diag_ranges: diag_ranges.clone(),
            diag_text: diag_text.clone(),
            line: d.line,
            start: d.start,
            width: r.width,
            gutter: gw,
            current: d.line == w.cursor.0,
            cursorline,
            diff_line,
            colorcolumn: ed.config.colorcolumn,
            list: ed.config.list,
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
                .incsearch
                .clone()
                .or_else(|| {
                    ed.last_search
                        .as_ref()
                        .filter(|_| ed.hl_search)
                        .map(|(p, _)| p.clone())
                })
                .map(|p| (p, ed.config.ignorecase, ed.config.smartcase)),
            marker,
            sign,
            fold_marker,
            word_diff_ranges: word_diff_ranges.clone(),
            deleted_before: deleted_before.clone(),
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
            // Soft-wrap continuation row: show the configured `showbreak`
            // marker (empty by default, so the continued row's gutter is blank).
            ed.config.showbreak.clone()
        } else if ed.config.number {
            if ed.config.relativenumber && d.line != w.cursor.0 {
                d.line.abs_diff(w.cursor.0).to_string()
            } else {
                (d.line + 1).to_string()
            }
        } else {
            String::new()
        };
        let fc = if ed.config.foldcolumn { 1 } else { 0 };
        let margin = if gw >= 2 {
            let fold_col = if fc > 0 {
                fold_marker.to_string()
            } else {
                String::new()
            };
            format!(
                "{}{}{}{:>width$} ",
                fold_col,
                marker,
                sign,
                number,
                width = gw.saturating_sub(3 + fc)
            )
        } else {
            " ".repeat(gw)
        };
        if let Some(bg) = row_bg {
            queue!(dest, SetBackgroundColor(bg))?;
        }
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
        let mut runs: Vec<(GlyphStyle, String)> = Vec::new();
        // Inlay hints splice into the glyph run itself (unlike code-lens/
        // blame text, which only ever appends after it) since a hint's
        // whole point is sitting at its own position among the real
        // characters -- a type hint right after the variable it
        // describes, say. `hint_idx` walks `line_hints` (sorted by
        // column) in lockstep with the glyphs so each hint is spliced in
        // right before the first glyph at or past its column.
        let hint_style = (false, false, false, false, Color::DarkGrey, None, false, false);
        let mut hint_idx = 0;
        let mut splice_hints_up_to =
            |col: usize, runs: &mut Vec<(GlyphStyle, String)>, used: &mut usize| {
                while hint_idx < line_hints.len() && line_hints[hint_idx].0 <= col {
                    let label = &line_hints[hint_idx].1;
                    if let Some((prev, text)) = runs.last_mut() {
                        if *prev == hint_style {
                            text.push_str(label);
                        } else {
                            runs.push((hint_style, label.clone()));
                        }
                    } else {
                        runs.push((hint_style, label.clone()));
                    }
                    *used += label.width();
                    hint_idx += 1;
                }
            };
        // `list` (listchars) reveals tabs (`>`, `-`) and trailing whitespace
        // (`·`). `d.text` is the whole logical line, so `trail_start` — the
        // column after its last non-blank char — is the logical trailing
        // threshold even for a wrapped segment (its glyphs keep full-line cols).
        let (row_chars, trail_start): (Vec<char>, usize) = if ed.config.list {
            let rc: Vec<char> = d.text.chars().collect();
            let ts = rc.iter().rposition(|c| !c.is_whitespace()).map_or(0, |p| p + 1);
            (rc, ts)
        } else {
            (Vec::new(), 0)
        };
        let mut prev_col: Option<usize> = None;
        for g in d.glyphs.iter() {
            splice_hints_up_to(g.col, &mut runs, &mut used);
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
            let word_diff_hl = word_diff_ranges
                .iter()
                .any(|(a, z)| g.col >= *a && g.col < *z);
            let color = spans
                .iter()
                .find(|(a, z, _)| g.col >= *a && g.col < *z)
                .map(|(_, _, c)| ed.theme.syntax(*c))
                .unwrap_or(Color::Reset);
            // Semantic tokens refine the base tree-sitter color when present.
            let color = sem_row
                .iter()
                .find(|(a, z, _, _)| g.col >= *a && g.col < *z)
                .map(|&(_, _, pal, _)| semantic_color(ed, pal))
                .unwrap_or(color);
            // A `deprecated` semantic token draws struck-through.
            let sem_strike = sem_row
                .iter()
                .any(|&(a, z, _, dep)| dep && g.col >= a && g.col < z);
            // Worst-severity diagnostic covering this glyph, if any --
            // same "pick the one that most needs attention" rule the
            // gutter marker above already applies per line, just also
            // per-column here so two diagnostics on one line don't
            // average out to whichever happened to be found first.
            let diag_underline = diag_ranges
                .iter()
                .filter(|(a, z, _)| g.col >= *a && g.col < *z)
                .map(|(_, _, sev)| *sev)
                .min_by_key(|s| match s {
                    crate::lsp::Severity::Error => 0,
                    crate::lsp::Severity::Warning => 1,
                    crate::lsp::Severity::Info => 2,
                    crate::lsp::Severity::Hint => 3,
                })
                .map(|sev| match sev {
                    crate::lsp::Severity::Error => Color::Red,
                    crate::lsp::Severity::Warning => Color::Yellow,
                    crate::lsp::Severity::Info => Color::Blue,
                    crate::lsp::Severity::Hint => Color::DarkGrey,
                });
            // Spell underline (magenta), only where no diagnostic already
            // underlines the glyph so diagnostics stay visually dominant.
            let diag_underline = diag_underline.or_else(|| {
                spell_ranges
                    .iter()
                    .any(|(a, z)| g.col >= *a && g.col < *z)
                    .then_some(Color::Magenta)
            });
            // The colorcolumn ruler falls on the glyph starting at that display
            // cell (`used` is this glyph's start column, before it advances).
            let colorcol = ed.config.colorcolumn > 0 && used == ed.config.colorcolumn - 1;
            // documentColor: paint a color literal's glyphs in its own RGB.
            let color = color_ranges
                .iter()
                .find(|(a, z, _)| g.col >= *a && g.col < *z)
                .map(|&(_, _, (cr, cg, cb))| Color::Rgb {
                    r: cr,
                    g: cg,
                    b: cb,
                })
                .unwrap_or(color);
            // rainbow: color a bracket glyph by its nesting depth.
            let color = rainbow_row
                .iter()
                .find(|(c, _)| *c == g.col)
                .map(|&(_, depth)| RAINBOW[depth as usize % RAINBOW.len()])
                .unwrap_or(color);
            // Inline TODO keywords: recolor the keyword glyphs so they stand out
            // against the comment color.
            let color = todo_ranges
                .iter()
                .find(|(a, z, _)| g.col >= *a && g.col < *z)
                .map(|&(_, _, c)| match c {
                    0 => Color::Yellow,
                    1 => Color::Red,
                    _ => Color::Magenta,
                })
                .unwrap_or(color);
            // listchars substitution (dimmed): tab lead/fill, or trailing ws.
            let (gtext, color) = if ed.config.list {
                let src = row_chars.get(g.col).copied();
                if src == Some('\t') {
                    let lead = prev_col != Some(g.col); // first cell of this tab
                    ((if lead { lc_lead } else { lc_fill }).to_string(), Color::DarkGrey)
                } else if g.col >= trail_start && src.is_some_and(|c| c == ' ' || c == '\t') {
                    (lc_trail.to_string(), Color::DarkGrey)
                } else {
                    (g.text.clone(), color)
                }
            } else {
                (g.text.clone(), color)
            };
            prev_col = Some(g.col);
            let style = (
                selected,
                searched,
                doc_hl,
                word_diff_hl,
                color,
                diag_underline,
                colorcol,
                sem_strike,
            );
            if let Some((prev, text)) = runs.last_mut() {
                if *prev == style {
                    text.push_str(&gtext);
                } else {
                    runs.push((style, gtext));
                }
            } else {
                runs.push((style, gtext));
            }
            used += g.width;
        }
        // Any hints positioned at or past end-of-line (there being no
        // glyph left to splice in front of) still need to show.
        splice_hints_up_to(usize::MAX, &mut runs, &mut used);
        for (
            (selected, searched, doc_hl, word_diff_hl, color, diag_underline, colorcol, strike),
            text,
        ) in runs
        {
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
            } else if doc_hl || word_diff_hl {
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
            } else if word_diff_hl {
                // `,gd`'s diff overlay: the word(s) that actually changed
                // within an otherwise-unchanged line, distinct from the
                // gutter's whole-line "modified" sign.
                queue!(dest, SetBackgroundColor(Color::DarkMagenta))?;
            } else if colorcol {
                queue!(dest, SetBackgroundColor(COLORCOLUMN_BG))?;
            } else if let Some(bg) = row_bg {
                queue!(dest, SetBackgroundColor(bg))?;
            }
            // An underline attribute, not a background swap, so it
            // layers on top of any of the above instead of replacing
            // them -- a diagnostic under a search match or a selection
            // still shows both.
            if let Some(underline) = diag_underline {
                queue!(
                    dest,
                    SetAttribute(Attribute::Underlined),
                    SetUnderlineColor(underline)
                )?;
            }
            // A `deprecated` semantic token is struck through (layered on top
            // of any background/underline above).
            if strike {
                queue!(dest, SetAttribute(Attribute::CrossedOut))?;
            }
            queue!(
                dest,
                SetForegroundColor(fg),
                Print(text),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
        }
        if let Some(text) = &diag_text {
            let remaining = width.saturating_sub(used);
            if remaining > 2 {
                let color = match diag.map(|d2| d2.severity) {
                    Some(crate::lsp::Severity::Error) => Color::Red,
                    Some(crate::lsp::Severity::Warning) => Color::Yellow,
                    Some(crate::lsp::Severity::Info) => Color::Blue,
                    _ => Color::DarkGrey,
                };
                let shown = clip_tab(&format!("  {text}"), remaining, b.tabstop);
                queue!(dest, SetForegroundColor(color), Print(&shown), ResetColor)?;
                used += shown.width();
            }
        }
        if let Some(text) = &code_lens {
            let remaining = width.saturating_sub(used);
            if remaining > 2 {
                let shown = clip_tab(&format!("  » {text}"), remaining, b.tabstop);
                queue!(
                    dest,
                    SetForegroundColor(Color::DarkCyan),
                    Print(&shown),
                    ResetColor
                )?;
                used += shown.width();
            }
        }
        if let Some(text) = &blame {
            let remaining = width.saturating_sub(used);
            if remaining > 2 {
                let shown = clip_tab(&format!("  {text}"), remaining, b.tabstop);
                queue!(
                    dest,
                    SetForegroundColor(Color::DarkGrey),
                    Print(&shown),
                    ResetColor
                )?;
                used += shown.width();
            }
        }
        if !deleted_before.is_empty() {
            let remaining = width.saturating_sub(used);
            if remaining > 2 {
                // Only the first removed line's own text is shown --
                // this is a compact one-line annotation, not the full
                // ghost-line rendering a GUI editor's floating window
                // could afford; `,gh` still shows the complete hunk for
                // anyone who wants the whole picture.
                let label = if deleted_before.len() > 1 {
                    format!("  -{} lines: {}", deleted_before.len(), deleted_before[0])
                } else {
                    format!("  -{}", deleted_before[0])
                };
                let shown = clip_tab(&label, remaining, b.tabstop);
                queue!(
                    dest,
                    SetForegroundColor(Color::Red),
                    Print(&shown),
                    ResetColor
                )?;
                used += shown.width();
            }
        }
        // The colorcolumn ruler, when it falls past end-of-line, splits the
        // trailing pad into [pad .. ruler), the ruler cell, and [ruler .. end).
        let ruler = if ed.config.colorcolumn > used
            && ed.config.colorcolumn - 1 < width
        {
            Some(ed.config.colorcolumn - 1)
        } else {
            None
        };
        let fill = |dest: &mut Vec<u8>, n: usize| -> std::io::Result<()> {
            if n == 0 {
                return Ok(());
            }
            if let Some(bg) = row_bg {
                queue!(
                    dest,
                    SetBackgroundColor(bg),
                    Print(" ".repeat(n)),
                    ResetColor
                )
            } else {
                queue!(dest, Print(" ".repeat(n)))
            }
        };
        match ruler {
            Some(rc) => {
                fill(dest, rc - used)?;
                queue!(dest, SetBackgroundColor(COLORCOLUMN_BG), Print(" "), ResetColor)?;
                fill(dest, width.saturating_sub(rc + 1))?;
            }
            None => fill(dest, width.saturating_sub(used))?,
        }
        cache.composed.insert(sig, dest[content_start..].to_vec());
    }
    // Sticky scroll: pin the enclosing function/class declaration lines that
    // have scrolled off the top of this pane, overlaying the first rows.
    if ed.config.sticky_scroll && !ed.zen && b.id == ed.buf().id && w.top > 0 {
        if let Some(syn) = &ed.syntax {
            let total = b.rope.len_bytes();
            let top_byte = b.line_byte_range(w.top).0.min(total);
            let mut lines: Vec<usize> = syn
                .context_starts(top_byte, STICKY_KINDS)
                .into_iter()
                .map(|sb| {
                    let ci = b.rope.byte_to_char(sb.min(total));
                    b.pos_from_char_idx(ci).0
                })
                .filter(|&l| l < w.top)
                .collect();
            lines.dedup();
            let k = lines.len().min(3).min(r.height.saturating_sub(2));
            for (i, &line) in lines.iter().take(k).enumerate() {
                let body = clip_tab(&b.line_text(line), r.width.saturating_sub(gw), b.tabstop);
                let text = format!("{}{}", " ".repeat(gw), body);
                plain_row(target.frame, r.y + top_off + i, r.x, r.width, &text, STICKY_BG)?;
            }
        }
    }
    // inccommand: overlay the live `:s` replacement preview onto each visible
    // affected line's content area (keeping its gutter), tinted to signal it's
    // a preview of an unsubmitted substitute.
    if !ed.sub_preview.is_empty() && b.id == ed.buf().id {
        for (row, d) in display.iter().enumerate().take(n) {
            if d.start != 0 {
                continue; // only the first wrap segment of a line
            }
            if let Some(preview) = ed.sub_preview.get(&d.line) {
                let shown = clip_tab(preview, width, b.tabstop);
                plain_row(
                    target.frame,
                    r.y + top_off + row,
                    r.x + gw,
                    width,
                    &shown,
                    INCCOMMAND_BG,
                )?;
            }
        }
    }
    // Closed folds: overlay each fold-start row with its foldtext (the first
    // line + a hidden-line count), tinted -- a post-loop overlay, so no
    // RowSignature change (the inner lines are already gone from `display`).
    if !b.folds.is_empty() && !ed.zen {
        for (row, d) in display.iter().enumerate().take(n) {
            if d.start != 0 {
                continue;
            }
            if let Some(f) = b.closed_fold_starting_at(d.line) {
                let count = f.end.saturating_sub(f.start) + 1;
                let head = b.line_text(d.line);
                let foldtext = format!("{}  ⋯ {count} lines", head.trim_end());
                let shown = clip_tab(&foldtext, width, b.tabstop);
                plain_row(
                    target.frame,
                    r.y + top_off + row,
                    r.x + gw,
                    width,
                    &shown,
                    FOLD_BG,
                )?;
            }
        }
    }
    // Minimap strip on the right, drawn last so it overlays cleanly (including
    // over any sticky-scroll header rows) in its reserved columns.
    if map_w > 0 {
        draw_minimap(target.frame, b, &display, r, gw, width, map_w, n, top_off)?;
    }
    // Winbar: the pane's top chrome row (path + enclosing-symbol breadcrumb).
    if top_off > 0 {
        draw_winbar(target.frame, ed, b, w, r, gw)?;
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
    // A persistent progress indicator in the status line, not the
    // message line: unlike `set_message`, this is recomputed fresh every
    // frame straight from `ed.lsp_progress`, so it can never be silently
    // clobbered by some unrelated action's own message the way the
    // message-line version already could be. Only on the active pane
    // (progress is global to the session, not per-buffer, so showing it
    // on every split would just duplicate the same text); clipped to a
    // fixed width so a long title/message can't push the cursor position
    // segment off the edge of a narrow terminal.
    let progress = if active {
        ed.format_lsp_progress()
    } else {
        None
    }
    .map(|p| format!("{} · ", clip(&p, 40)))
    .unwrap_or_default();
    // Non-Unix line endings are surfaced the way Vim's default ruler does:
    // `[dos]`/`[mac]`, with plain Unix left unmarked; a non-UTF-8 encoding
    // (`[latin1]`, `[utf-16le]`, …) is shown alongside.
    let eol = match b.fileformat {
        crate::buffer::FileFormat::Unix => "",
        crate::buffer::FileFormat::Dos => " [dos]",
        crate::buffer::FileFormat::Mac => " [mac]",
    };
    let ff = if b.encoding == crate::buffer::Encoding::Utf8 {
        eol.to_string()
    } else {
        format!("{eol} [{}]", b.encoding.name())
    };
    let right = format!(" {}{}:{} ", progress, w.cursor.0 + 1, w.cursor.1 + 1);
    let mode_label = if active { ed.mode.label() } else { "BUFFER" };
    let left = if ed.config.statusline.is_empty() {
        format!(
            " {} {}{}{}",
            mode_label,
            name,
            if b.is_modified() { " [+]" } else { "" },
            ff,
        )
    } else {
        let ftype = b
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("");
        format!(
            " {} ",
            expand_statusline(
                &ed.config.statusline,
                &StatusInfo {
                    mode: mode_label,
                    name: &name,
                    line: w.cursor.0 + 1,
                    col: w.cursor.1 + 1,
                    total: b.line_count(),
                    modified: b.is_modified(),
                    ftype,
                },
            )
        )
    };
    let label = format!(
        "{}{}",
        pad(&left, r.width.saturating_sub(right.width())),
        right
    );
    if !ed.zen {
        plain_row(
            target.frame,
            r.y + r.height - 1,
            r.x,
            r.width,
            &label,
            if active {
                ed.theme.statusline_active_bg
            } else {
                ed.theme.statusline_inactive_bg
            },
        )?;
    }
    Ok(cursor.map(|(y, x)| (r.x + gw + x, r.y + top_off + y)))
}
/// The values a statusline format string can reference.
pub(crate) struct StatusInfo<'a> {
    pub mode: &'a str,
    pub name: &'a str,
    pub line: usize,
    pub col: usize,
    pub total: usize,
    pub modified: bool,
    pub ftype: &'a str,
}

/// Expands a Vim-like statusline format string. Supported: `%f`/`%F` file
/// name, `%l` line, `%c` col, `%L` total lines, `%m` modified flag, `%y`
/// filetype, `%p` percent, `%M` mode, `%%` literal. Unknown `%x` passes
/// through verbatim.
pub(crate) fn expand_statusline(fmt: &str, s: &StatusInfo) -> String {
    let StatusInfo {
        mode,
        name,
        line,
        col,
        total,
        modified,
        ftype,
    } = *s;
    let pct = if total <= 1 {
        100
    } else {
        (line.saturating_sub(1) * 100) / total.saturating_sub(1)
    };
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('f') | Some('F') => out.push_str(name),
            Some('l') => out.push_str(&line.to_string()),
            Some('c') => out.push_str(&col.to_string()),
            Some('L') => out.push_str(&total.to_string()),
            Some('m') => out.push_str(if modified { "[+]" } else { "" }),
            Some('y') => out.push_str(ftype),
            Some('p') => out.push_str(&format!("{pct}%")),
            Some('M') => out.push_str(mode),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
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
/// Loads a preview pane's source lines for `path`, memoized in `cache`
/// across frames -- see `FrameCache::preview_source`'s own doc comment for
/// why this matters. An open buffer's content is never cached (already
/// cheap in memory, and must always reflect unsaved edits a stat-based
/// staleness check can't see), and a large file `,gp`-style either read
/// once now this session or that's changed on disk gets its stat checked
/// (cheap) rather than its whole content re-parsed (not cheap) as long as
/// its `mtime` hasn't moved.
fn cached_preview_source(
    ed: &Editor,
    cache: &mut FrameCache,
    path: &std::path::Path,
) -> Vec<String> {
    if ed.buffers.iter().any(|b| b.path.as_deref() == Some(path)) {
        return ed.preview_source_lines(path);
    }
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some(mtime) = mtime {
        if let Some((cached_mtime, lines)) = cache.preview_source.get(path) {
            if *cached_mtime == mtime {
                return lines.clone();
            }
        }
    }
    let lines = ed.preview_source_lines(path);
    if let Some(mtime) = mtime {
        if cache.preview_source.len() > 64 {
            cache.preview_source.clear();
        }
        cache
            .preview_source
            .insert(path.to_path_buf(), (mtime, lines.clone()));
    }
    lines
}
fn draw_results(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    cache: &mut FrameCache,
    width: usize,
    height: usize,
) -> io::Result<(usize, usize)> {
    let Some(r) = &ed.results else {
        return Ok((0, 0));
    };
    let selected = r.selected.len();
    let count = if r.filter.is_empty() {
        format!("{}", r.entries.len())
    } else {
        format!("{}/{}", r.entries.len(), r.all_entries.len())
    };
    plain_row(
        frame,
        0,
        0,
        width,
        &format!(
            " {}{}  · {} results · {} selected{}",
            if r.quickfix { "QUICKFIX / " } else { "" },
            r.title,
            count,
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
        // Don't reserve more list space than there are entries to
        // show (the list still caps out at half the available rows
        // for a genuinely long list, so it keeps scrolling exactly as
        // before) -- otherwise a short list leaves a dead blank gap
        // between it and the preview pane instead of giving that room
        // to the preview, which is what earns the extra space here in
        // the first place.
        let total = height.saturating_sub(4);
        let max_list = (total / 2).max(1);
        let list_needed = r.entries.len().clamp(1, max_list);
        total.saturating_sub(list_needed).max(4)
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
    } else if r.filter_input {
        format!("Filter: {}", r.filter)
    } else {
        format!(
            " {}{}{}",
            if r.quickfix {
                "Search / ? · n N"
            } else {
                "Ctrl-Q → quickfix"
            },
            if r.live { " · i edit grep query" } else { "" },
            if r.filter.is_empty() {
                " · f filter"
            } else {
                " · f filter (active)"
            }
        )
    };
    plain_row(frame, 1, 0, width, &prompt, Color::Reset)?;
    let detail_y = 2 + list_rows;
    if detail_rows > 0 {
        // When the file-content preview is on, separate it from the results
        // list with the same labelled rule the file picker uses, so the
        // preview region is visibly distinct. (The always-on detail line shown
        // on tall terminals without preview is not a file preview, so it keeps
        // no border.)
        let (content_y, content_rows) = if r.preview {
            let label = "── preview ";
            let lw = UnicodeWidthStr::width(label);
            let border = if lw < width {
                format!("{label}{}", "─".repeat(width - lw))
            } else {
                "─".repeat(width)
            };
            plain_row(frame, detail_y, 0, width, &border, Color::DarkGrey)?;
            (detail_y + 1, detail_rows.saturating_sub(1))
        } else {
            (detail_y, detail_rows)
        };
        let path = r.entries.get(r.cursor).and_then(|e| e.path.clone());
        let preview = path.and_then(|p| {
            let source = cached_preview_source(ed, cache, &p);
            r.preview_rows(
                &source,
                content_rows,
                width.saturating_sub(2),
                crate::results::PREVIEW_CONTEXT_BEFORE,
            )
        });
        if let Some(rows) = preview {
            for (i, row) in rows.iter().enumerate() {
                plain_row(
                    frame,
                    content_y + i,
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
            for (i, line) in detail.lines().take(content_rows).enumerate() {
                plain_row(
                    frame,
                    content_y + i,
                    0,
                    width,
                    &format!("  {line}"),
                    Color::DarkGrey,
                )?;
            }
        }
    }
    let footer = if r.git_status && r.preview {
        "q close · p preview off · w wrap · Ctrl-e/y scroll · s/u/D/c/C/r git actions"
    } else if r.git_status {
        "q close · s stage · u unstage · D discard · c/C commit/amend · r refresh · p preview"
    } else if r.entries.iter().any(|e| e.note_id.is_some()) {
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
    cache: &mut FrameCache,
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
    // Same "don't reserve more list space than there are entries to
    // show" rule `draw_results` applies to its own preview pane -- see
    // its own comment for why.
    let detail_rows = if height < 10 {
        0
    } else if p.preview {
        let total = height.saturating_sub(2);
        let max_list = (total / 2).max(1);
        let list_needed = p.matches.len().clamp(1, max_list);
        total.saturating_sub(list_needed).max(4)
    } else {
        0
    };
    let list_rows = height.saturating_sub(2 + detail_rows);
    let start = p.selected.saturating_sub(list_rows.saturating_sub(1));
    for (i, (_, path)) in p.matches.iter().skip(start).take(list_rows).enumerate() {
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
    let preview_y = 1 + list_rows;
    if detail_rows > 0 {
        // A clear, labelled rule separating the file list from the preview
        // pane, so the preview (toggled with Ctrl-r) is visibly distinct
        // rather than blending into the list above it.
        let label = "── preview ";
        let lw = UnicodeWidthStr::width(label);
        let border = if lw < width {
            format!("{label}{}", "─".repeat(width - lw))
        } else {
            "─".repeat(width)
        };
        plain_row(frame, preview_y, 0, width, &border, Color::DarkGrey)?;
        let content_y = preview_y + 1;
        let content_rows = detail_rows.saturating_sub(1);
        let source = p
            .matches
            .get(p.selected)
            .map(|(_, rel)| cached_preview_source(ed, cache, &ed.project_root.join(rel)))
            .unwrap_or_default();
        let shown = source.iter().skip(p.preview_scroll).take(content_rows);
        for (i, line) in shown.enumerate() {
            plain_row(
                frame,
                content_y + i,
                0,
                width,
                &clip(line, width),
                Color::Reset,
            )?;
        }
        let filled = source
            .len()
            .saturating_sub(p.preview_scroll)
            .min(content_rows);
        for i in filled..content_rows {
            plain_row(frame, content_y + i, 0, width, "", Color::Reset)?;
        }
    }
    plain_row(
        frame,
        height - 1,
        0,
        width,
        &format!(
            "{}/{} files{} · Enter open · Ctrl-Q quickfix · Ctrl-r preview{} · Esc close",
            p.matches.len(),
            p.stats.matched,
            if ed.search_job.files_rx.is_some() {
                " · scanning…"
            } else {
                ""
            },
            if p.preview {
                " (on) · Ctrl-e/y scroll"
            } else {
                ""
            },
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
        let node_idx = tree.top + y;
        let text = match tree.nodes.get(node_idx) {
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
        let selected = active && tree.cursor == node_idx;
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
        let node_idx = outline.top + y;
        let text = match outline.nodes.get(node_idx) {
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
        let selected = outline.cursor == node_idx;
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
                    ed.theme.syntax(class)
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
        crossterm::event::EnableFocusChange,
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
        crossterm::event::DisableFocusChange,
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
    // Match draw_pane: a winbar shifts content down one row (never in zen).
    let top_off = if ed.config.winbar && !ed.zen && rect.height > 2 {
        1
    } else {
        0
    };
    let content_rows = rect.height.saturating_sub(1).saturating_sub(top_off);
    let (display, _) = layout(ed, b, &w, pane_width, content_rows);
    let row_in_pane = y.checked_sub(rect.y + top_off)?;
    let d = display.get(row_in_pane)?;
    // `x_off` is pane-relative but each glyph's `cell` is absolute within
    // the source line, so offset the click by the row's own start cell --
    // otherwise wrapped continuation rows and horizontally-scrolled nowrap
    // rows (w.left > 0) map clicks `d.start` columns too far left.
    let x_off = x.saturating_sub(rect.x + gw) + d.start;
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

    #[test]
    fn parse_fillchars_reads_eob_and_vert() {
        assert_eq!(parse_fillchars("eob: ,vert:┃"), (' ', '┃'));
        assert_eq!(parse_fillchars("vert:|"), ('~', '|'));
        assert_eq!(parse_fillchars(""), ('~', '│'));
    }
    #[test]
    fn parse_listchars_reads_tab_and_trail() {
        assert_eq!(parse_listchars("tab:▸·,trail:•"), ('▸', '·', '•'));
        // A single tab char sets both lead and fill.
        assert_eq!(parse_listchars("tab:>"), ('>', '>', '·'));
        // Missing keys fall back to defaults; unknown keys are ignored.
        assert_eq!(parse_listchars("eol:¬"), ('>', '-', '·'));
        assert_eq!(parse_listchars(""), ('>', '-', '·'));
    }
    #[test]
    fn minimap_shape_reflects_indentation_and_length() {
        // Always exactly the requested width.
        assert_eq!(minimap_shape("hello", 12).chars().count(), 12);
        // A blank line is all spaces.
        assert_eq!(minimap_shape("   ", 12), " ".repeat(12));
        assert_eq!(minimap_shape("", 8), " ".repeat(8));
        // An indented line starts its bar past the left edge; a line flush to
        // column 0 starts at the first cell.
        let flush = minimap_shape("code", 10);
        assert_eq!(flush.chars().next(), Some('▪'), "col-0 line fills cell 0");
        let indented = minimap_shape(&format!("{}code", " ".repeat(40)), 10);
        assert_eq!(
            indented.chars().next(),
            Some(' '),
            "a deeply indented line leaves the left cells blank"
        );
        assert!(
            indented.contains('▪'),
            "the indented line still shows its code bar"
        );
    }

    #[test]
    fn cached_preview_source_reuses_content_while_the_files_mtime_is_unchanged() {
        let dir = std::env::temp_dir().join(format!(
            "vaayu-preview-cache-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "one\n").unwrap();
        let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();

        let ed = crate::editor::Editor::new(crate::config::Config::default());
        let mut cache = FrameCache::new();
        assert_eq!(
            cached_preview_source(&ed, &mut cache, &file),
            vec!["one".to_string()]
        );

        // Overwrite the content but pin the mtime back to what it was --
        // the cache should still serve the old content rather than
        // re-reading a file whose stat says nothing changed.
        std::fs::write(&file, "two\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        assert_eq!(
            cached_preview_source(&ed, &mut cache, &file),
            vec!["one".to_string()],
            "an unchanged mtime should reuse the cached content"
        );

        // Now genuinely bump the mtime forward -- the cache must
        // invalidate and pick up the new content.
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(mtime + std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            cached_preview_source(&ed, &mut cache, &file),
            vec!["two".to_string()],
            "a changed mtime should invalidate the cache"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cached_preview_source_always_reads_an_open_buffer_fresh() {
        // An open buffer's unsaved edits should show up immediately,
        // never served from a stale disk-backed cache entry -- even one
        // for the same path from before the buffer was opened.
        let dir = std::env::temp_dir().join(format!(
            "vaayu-preview-cache-buf-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("b.txt");
        std::fs::write(&file, "on disk\n").unwrap();

        let mut ed = crate::editor::Editor::new(crate::config::Config::default());
        let mut cache = FrameCache::new();
        assert_eq!(
            cached_preview_source(&ed, &mut cache, &file),
            vec!["on disk".to_string()]
        );

        ed.open_file(file.clone()).unwrap();
        ed.buf_mut().rope = ropey::Rope::from_str("unsaved edit\n");
        assert_eq!(
            cached_preview_source(&ed, &mut cache, &file),
            // ropey counts a trailing newline as an extra final empty line.
            vec!["unsaved edit".to_string(), String::new()],
            "an open buffer's live content should never be served from the disk cache"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
