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
        1 => ed.theme.type_,
        2 => ed.theme.function,
        3 => ed.theme.string,
        4 => ed.theme.comment,
        5 => ed.theme.number,
        _ => Color::Reset,
    }
}

/// Total width of the minimap strip (separator column + body).
const MINIMAP_W: usize = 12;
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
                        matches!(
                            c,
                            crate::syntax::HlClass::Comment | crate::syntax::HlClass::String
                        )
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
/// The enclosing declaration lines to pin as sticky-scroll context for a pane
/// whose top visible line is `top`. Uses tree-sitter when a grammar is present,
/// else the LSP `documentSymbol` fallback (`Editor::sticky_symbols`), so a
/// grammarless-but-served buffer still gets a context header. Outermost first.
pub(crate) fn sticky_context_lines(ed: &Editor, b: &Buffer, top: usize) -> Vec<usize> {
    let mut lines: Vec<usize> = if let Some(syn) = &ed.syntax {
        let total = b.rope.len_bytes();
        let top_byte = b.line_byte_range(top).0.min(total);
        syn.context_starts(top_byte, STICKY_KINDS)
            .into_iter()
            .map(|sb| {
                let ci = b.rope.byte_to_char(sb.min(total));
                b.pos_from_char_idx(ci).0
            })
            .filter(|&l| l < top)
            .collect()
    } else if ed.sticky_symbols_buffer == Some(b.id) {
        let mut ls: Vec<usize> = ed
            .sticky_symbols
            .iter()
            .filter(|(s, e)| *s < top && *e >= top)
            .map(|(s, _)| *s)
            .collect();
        ls.sort_unstable(); // outermost (smallest start) first
        ls
    } else {
        Vec::new()
    };
    lines.dedup();
    lines
}

/// Conceal spans for one line: for each compiled rule, every match becomes a
/// `(start_col, end_col, cchar)` char-column range (`cchar` = `None` to hide,
/// `Some` to replace the whole match with that one char). Sorted by start.
pub(crate) fn conceal_line_ranges(
    rules: &[(regex::Regex, Option<char>)],
    text: &str,
) -> Vec<(usize, usize, Option<char>)> {
    let mut out = Vec::new();
    for (re, cchar) in rules {
        for m in re.find_iter(text) {
            let a = text[..m.start()].chars().count();
            let z = text[..m.end()].chars().count();
            if z > a {
                out.push((a, z, *cchar));
            }
        }
    }
    out.sort_by_key(|(a, _, _)| *a);
    out
}

/// Pick a readable foreground (black or white) for text drawn on a color
/// swatch, using Rec. 601 luma. Non-RGB colors default to white.
fn contrast_on(bg: Color) -> Color {
    match bg {
        Color::Rgb { r, g, b } => {
            let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            if luma > 140.0 {
                Color::Black
            } else {
                Color::White
            }
        }
        _ => Color::White,
    }
}

pub(crate) fn parse_fillchars(s: &str) -> (char, char) {
    let (mut eob, mut vert) = ('~', '│');
    for item in s.split(',') {
        let Some((key, val)) = item.split_once(':') else {
            continue;
        };
        let Some(c) = val.chars().next() else {
            continue;
        };
        match key.trim() {
            "eob" => eob = c,
            "vert" => vert = c,
            _ => {}
        }
    }
    (eob, vert)
}
/// A gutter component, for `statuscolumn` ordering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GutterComp {
    Fold,
    Diag,
    Git,
    Num,
}
/// Parse `statuscolumn` (space-separated `fold`/`diag`/`git`/`num`) into an
/// ordered component list. Empty → the default order; `Fold` is included only
/// when `foldcolumn` is on (it owns no cell otherwise).
fn parse_statuscolumn(s: &str, foldcolumn: bool) -> Vec<GutterComp> {
    if s.trim().is_empty() {
        let mut v = Vec::new();
        if foldcolumn {
            v.push(GutterComp::Fold);
        }
        v.extend([GutterComp::Diag, GutterComp::Git, GutterComp::Num]);
        return v;
    }
    let mut v = Vec::new();
    for tok in s.split_whitespace() {
        match tok {
            "fold" if foldcolumn => v.push(GutterComp::Fold),
            "fold" => {}
            "diag" | "sign" => v.push(GutterComp::Diag),
            "git" => v.push(GutterComp::Git),
            "num" | "number" => v.push(GutterComp::Num),
            _ => {}
        }
    }
    v
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
    ed.sync_file_tree();
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
        t.ensure_visible(h.saturating_sub(crate::filetree::HEADER_ROWS));
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
    // Use the same content geometry `draw_pane` renders with (minimap strip and
    // winbar row included), so the cursor never scrolls off-screen.
    let dims = pane_dims(ed, ed.buf(), rect);
    let count = dims.rows.max(1);
    ed.screen_rows = count;
    let width = dims.width;
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
type GlyphStyle = (
    bool,
    bool,
    bool,
    bool,
    Color,
    Option<Color>,
    bool,
    bool,
    Option<Color>,
    bool,
);
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
    /// This row is within the active code tour's highlighted step range.
    tour_hl: bool,
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
    /// Whether `colorswatch` is on (paints color literals as background chips),
    /// so toggling `:set colorswatch` repaints rows that carry color literals.
    color_swatch: bool,
    /// Rainbow bracket `(col, depth)` on this row (empty when disabled).
    rainbow: Vec<(usize, u8)>,
    /// Semantic-token `(start, end, palette)` spans on this row.
    sem_ranges: Vec<(usize, usize, u8, bool, bool)>,
    /// Misspelled-word char ranges on this row (for the spell underline).
    spell_ranges: Vec<(usize, usize)>,
    /// TODO/FIXME/etc. keyword ranges on this row `(start, end, color index)`.
    todo_ranges: Vec<(usize, usize, u8)>,
    /// Injected-language syntax spans on this row `(start, end, class)`, so a
    /// fence-marker edit that recolors an otherwise-unchanged line repaints it.
    inject_ranges: Vec<(usize, usize, crate::syntax::HlClass)>,
    /// Conceal spans on this row `(start_col, end_col, cchar)` — empty on the
    /// revealed cursor line, so moving onto/off a line repaints it.
    conceal: Vec<(usize, usize, Option<char>)>,
    /// Inline ghost-text suggestion drawn after this row's content, or `None`.
    ghost: Option<String>,
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
    /// The theme `(bg, fg)` the on-screen rows were painted with (see
    /// `paint_base`); a change repaints every row.
    base: Option<(Color, Color)>,
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
            base: None,
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
/// Word-wrap `text` (respecting existing newlines) to `width` columns, hard
/// breaking any single word longer than the width. Char-count based, which is
/// close enough for a description panel.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut w = 0usize;
        for word in para.split_whitespace() {
            let ww = word.chars().count();
            if w > 0 && w + 1 + ww > width {
                out.push(std::mem::take(&mut line));
                w = 0;
            }
            if w > 0 {
                line.push(' ');
                w += 1;
            }
            if ww > width {
                for ch in word.chars() {
                    if w >= width {
                        out.push(std::mem::take(&mut line));
                        w = 0;
                    }
                    line.push(ch);
                    w += 1;
                }
            } else {
                line.push_str(word);
                w += ww;
            }
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// A bottom panel for the active code tour: the current step's title, its
/// (wrapped) description, and a navigation hint. Only shown in the editing
/// modes, above the status line, so tour descriptions are readable in full
/// rather than truncated onto the message line.
fn draw_tour_panel(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    width: usize,
    height: usize,
) -> io::Result<()> {
    if !matches!(ed.mode, Mode::Normal | Mode::Insert | Mode::Visual(_)) {
        return Ok(());
    }
    // Yield the bottom rows to a completion popup or a which-key/pending-key
    // popup, which also draw there; the tour panel would otherwise cover them.
    if ed.completion.as_ref().is_some_and(|c| !c.items.is_empty()) || ed.pending.awaiting.is_some()
    {
        return Ok(());
    }
    let Some((tour, idx)) = &ed.active_tour else {
        return Ok(());
    };
    let idx = *idx;
    let Some(step) = tour.steps.get(idx) else {
        return Ok(());
    };
    if height < 5 || width < 10 {
        return Ok(());
    }
    let inner = width.saturating_sub(1);
    let mut body = wrap_text(&tour_markdown_lite(&step.description), inner);
    body.truncate(6);
    let body_rows = body.len().max(1);
    // header + body + hint, but never more than half the screen. Sits *above*
    // the status line (height-2) and message line (height-1), so neither is
    // covered.
    let panel_h = (body_rows + 2)
        .min(height.saturating_sub(2))
        .min(height / 2 + 2);
    if panel_h < 3 {
        return Ok(());
    }
    let y0 = height.saturating_sub(2 + panel_h);
    let title = if tour.title.is_empty() {
        "Tour"
    } else {
        tour.title.as_str()
    };
    let total = tour.steps.len();
    let dots = tour_progress_dots(idx, total);
    let header = format!(" {}  —  step {}/{}  {}", title, idx + 1, total, dots);
    plain_row(frame, y0, 0, width, &header, ed.theme.bar_bg)?;
    for (i, l) in body.iter().enumerate() {
        if 1 + i >= panel_h.saturating_sub(1) {
            break;
        }
        plain_row(
            frame,
            y0 + 1 + i,
            0,
            width,
            &format!(" {l}"),
            ed.theme.fold_bg,
        )?;
    }
    plain_row(
        frame,
        y0 + panel_h - 1,
        0,
        width,
        " ]t next · [t prev · ,tx explain · :tourend",
        ed.theme.panel_bg,
    )?;
    Ok(())
}

/// A ●/○ progress bar for the tour panel header, capped at `DOT_CAP` dots.
/// Past the cap, the window slides to stay centered on `idx` so the dots
/// keep reflecting true relative progress instead of latching all-filled
/// (a fixed `0..total.min(DOT_CAP)` window would render every dot filled
/// forever once `idx` passed the cap).
fn tour_progress_dots(idx: usize, total: usize) -> String {
    const DOT_CAP: usize = 20;
    if total <= DOT_CAP {
        return (0..total)
            .map(|i| if i <= idx { '●' } else { '○' })
            .collect();
    }
    let start = idx.saturating_sub(DOT_CAP / 2).min(total - DOT_CAP);
    (start..start + DOT_CAP)
        .map(|i| if i <= idx { '●' } else { '○' })
        .collect()
}

/// Strip the noisiest Markdown markers from a tour description so it reads
/// cleanly in the plain-text panel (bold `**`, inline-code backticks, a leading
/// heading `#`), while leaving the words intact.
fn tour_markdown_lite(s: &str) -> String {
    s.replace("**", "")
        .replace('`', "")
        .lines()
        .map(|l| l.trim_start_matches('#').trim_start())
        .collect::<Vec<_>>()
        .join("\n")
}

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
        plain_row(frame, i, width - box_w, box_w, &text, ed.theme.selection_bg)?;
    }
    Ok(())
}
/// The progress stack (`src/progress.rs`): one row per running or recently
/// finished job, right-aligned in the bottom-right corner of the pane area
/// just above the status line, newest at the bottom. Rows share one width
/// so the block's left edge is straight; the source is right-aligned and
/// dimmed. Uses at most half the pane area's rows.
fn draw_progress(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    width: usize,
    height: usize,
) -> io::Result<()> {
    if !ed.config.progress || width < 24 || height < 4 {
        return Ok(());
    }
    let rows = ed.progress.rows(std::time::Instant::now());
    if rows.is_empty() {
        return Ok(());
    }
    let root = ed.layout_root_rect(width, height);
    let status_row = usize::from(!ed.zen && !ed.config.global_statusline);
    let usable = root.height.saturating_sub(status_row);
    let show = rows.len().min(usable / 2);
    if show == 0 {
        return Ok(());
    }
    let bottom = root.y + usable - 1;
    let rows = &rows[rows.len() - show..];
    // " ✓ text  source "
    let needed = rows
        .iter()
        .map(|r| {
            let src = UnicodeWidthStr::width(r.source.as_str());
            4 + UnicodeWidthStr::width(r.text.as_str()) + if src > 0 { src + 2 } else { 0 }
        })
        .max()
        .unwrap_or(0);
    let box_w = needed.min((width * 2 / 5).clamp(24, 60)).min(width);
    let x = width - box_w;
    for (i, r) in rows.iter().enumerate() {
        let y = bottom + 1 + i - show;
        let Some(line) = frame.get_mut(y) else {
            continue;
        };
        use crate::progress::Phase;
        let (icon, icon_fg, text_fg) = match r.phase {
            Phase::Active(c) => (c, ed.theme.accent, Color::Reset),
            Phase::Done { ok: true } => ('✓', ed.theme.success, Color::Reset),
            Phase::Done { ok: false } => ('✗', ed.theme.error, Color::Reset),
            Phase::Fading { ok } => (if ok { '✓' } else { '✗' }, ed.theme.muted, ed.theme.muted),
            Phase::More => (' ', ed.theme.muted, ed.theme.muted),
        };
        // Leave the source its full width and clip the text first; only a
        // source wider than the whole box gets clipped itself.
        let src = clip(&r.source, box_w.saturating_sub(5));
        let src_w = UnicodeWidthStr::width(src.as_str());
        let text_w = box_w.saturating_sub(4 + if src_w > 0 { src_w + 2 } else { 0 });
        queue!(
            line,
            MoveTo(x as u16, y as u16),
            SetBackgroundColor(Color::Reset),
            Print(" "),
            SetForegroundColor(icon_fg),
            Print(icon),
            Print(" "),
            SetForegroundColor(text_fg),
            Print(pad(&r.text, text_w)),
            SetForegroundColor(ed.theme.muted),
            Print(if src_w > 0 {
                format!("  {src} ")
            } else {
                " ".into()
            }),
            ResetColor,
            SetAttribute(Attribute::Reset)
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
    ed: &Editor,
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
        let Some(dest) = frame.get_mut(y) else {
            continue;
        };
        queue!(
            dest,
            MoveTo(map_x as u16, y as u16),
            SetForegroundColor(ed.theme.muted),
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
            queue!(dest, SetBackgroundColor(ed.theme.minimap_view_bg))?;
        }
        queue!(
            dest,
            SetForegroundColor(ed.theme.minimap_fg),
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
            // Full breadcrumb path: every enclosing declaration, outermost
            // first (e.g. `impl Foo › fn bar()`), each trimmed to the text
            // before its opening brace and clipped.
            for sb in syn.context_starts(cursor_byte, STICKY_KINDS) {
                let ci = b.rope.byte_to_char(sb.min(total));
                let line = b.pos_from_char_idx(ci).0;
                let decl = b.line_text(line);
                let crumb = decl.trim().split('{').next().unwrap_or("").trim();
                if !crumb.is_empty() {
                    bar.push_str("  ›  ");
                    bar.push_str(&clip(crumb, 40));
                }
            }
        }
    }
    let shown = clip_tab(&bar, r.width, b.tabstop);
    plain_row(frame, r.y, r.x, r.width, &shown, ed.theme.winbar_bg)?;
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
        && ed.tabs.len() <= 1
        // The fast path scrolls the content region wholesale, but these overlays
        // are painted on top of it without being recorded in `logical`, so the
        // scroll-reuse diff can't see them change. A winbar also shifts the
        // first content row off row 1. Fall back to the full-row diff for any of
        // them so a scrolled-but-stale sticky header / foldtext / minimap can't
        // linger on screen.
        && !ed.config.sticky_scroll
        && !ed.config.minimap
        && (!ed.config.winbar || ed.zen)
        && ed.active_tour.is_none()
        && ed.float.is_none()
        && !ed.buf().folds.iter().any(|f| f.closed);
    let mut viewport = Vec::new();
    let early_scroll = if scroll_eligible {
        let rect = ed.pane_rects(width, height)[0];
        let w = if ed.windows.is_empty() {
            ed.capture_window()
        } else {
            ed.windows[0].clone()
        };
        ed.buffers.iter().find(|b| b.id == w.buffer).and_then(|b| {
            let d = pane_dims(ed, b, rect);
            let (display, _) = layout(ed, b, &w, d.width, d.rows);
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
        // A bar cursor only while actually typing into the query/filter
        // field (Insert sub-mode); Normal sub-mode within it gets the
        // same block cursor a real buffer's Normal mode uses, so the
        // shape keeps meaning "Insert vs Normal" everywhere in the app.
        bar = ed
            .results
            .as_ref()
            .is_some_and(|r| (r.search_input.is_some() || r.filter_input) && r.qcursor.insert);
    } else if matches!(ed.mode, Mode::Far) {
        cursor = draw_far(&mut frame, ed, cache, width, height)?;
        bar = ed
            .far
            .as_ref()
            .is_some_and(|f| f.editing.is_some() && f.qcursor.insert);
    } else if matches!(ed.mode, Mode::Picker) {
        cursor = draw_picker(&mut frame, ed, cache, width, height)?;
        bar = ed.file_picker.as_ref().is_some_and(|p| p.qcursor.insert);
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
                    plain_row(
                        &mut frame,
                        y,
                        rect.x + rect.width,
                        1,
                        &vert_s,
                        ed.theme.panel_bg,
                    )?;
                }
            }
            if rect.y + rect.height < height.saturating_sub(1) {
                plain_row(
                    &mut frame,
                    rect.y + rect.height,
                    rect.x,
                    rect.width,
                    &"─".repeat(rect.width),
                    ed.theme.panel_bg,
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
        // Global statusline: one shared line for the active window, drawn just
        // above the message line (the pane rects reserved a row for it).
        if ed.config.global_statusline && height >= 2 {
            let w = if ed.windows.is_empty() {
                ed.capture_window()
            } else {
                ed.windows[ed.active_window.min(ed.windows.len() - 1)].clone()
            };
            let b = ed
                .buffers
                .iter()
                .find(|b| b.id == w.buffer)
                .unwrap_or(ed.buf());
            let label = statusline_label(ed, b, &w, width, true);
            plain_row(
                &mut frame,
                height - 2,
                0,
                width,
                &label,
                ed.theme.statusline_active_bg,
            )?;
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
            // Column tracks `cmdline_qcursor.pos` (a char index into
            // `cmdline`, offset by the leading `:`/`/`/`?`), not just the
            // end of the line -- same reasoning as the picker/results
            // query bars' own cursor-column fix.
            let upto: String = message.chars().take(1 + ed.cmdline_qcursor.pos).collect();
            cursor = (clip(&upto, width.saturating_sub(1)).width(), height - 1);
            bar = true;
        } else {
            bar = matches!(ed.mode, Mode::Insert);
        }
        // Wildmenu: the Tab-completion candidates, shown on the row above the
        // command line with the selected one bracketed.
        let wildmenu_sel = ed.cmdline_completion_index.filter(|_| {
            matches!(ed.mode, Mode::Command(CommandKind::Ex))
                && !ed.cmdline_completions.is_empty()
                && height >= 2
        });
        if let Some(sel) = wildmenu_sel {
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
            plain_row(
                &mut frame,
                height - 2,
                0,
                width,
                &clip(&row, width),
                ed.theme.bar_bg,
            )?;
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
                            ed.theme.selection_bg
                        } else {
                            ed.theme.bar_bg
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
                        plain_row(&mut frame, doc_y, x, w, " Docs:", ed.theme.panel_bg)?;
                        for (i, line) in doc.lines().take(doc_rows - 1).enumerate() {
                            plain_row(
                                &mut frame,
                                doc_y + 1 + i,
                                x,
                                w,
                                &format!(" {line}"),
                                ed.theme.panel_bg,
                            )?;
                        }
                    }
                }
            }
        }
        if let Some(f) = &ed.float {
            if let Some(c) = draw_float(&mut frame, ed, cache, f, cursor, width, height)? {
                cursor = c;
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
    draw_tour_panel(&mut frame, ed, width, height)?;
    draw_toasts(&mut frame, ed, width, height)?;
    draw_progress(&mut frame, ed, width, height)?;
    // Rows are composed against the terminal's default colors; a scheme
    // with its own background/foreground is applied here, as the rows go out,
    // so the row caches stay theme-independent.
    let base = ed.theme.base(ed.config.transparent);
    if cache.base != base {
        cache.rows.clear();
        cache.base = base;
    }
    let base_sgr = base.map(|(bg, fg)| (sgr(SetBackgroundColor(bg)), sgr(SetForegroundColor(fg))));
    let mut painted = Vec::new();
    for (y, row) in frame.iter().enumerate() {
        if cache.rows.get(y) != Some(row) {
            queue!(
                out,
                MoveTo(0, y as u16),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
            match &base_sgr {
                Some((bg, fg)) => {
                    out.write_all(bg)?;
                    out.write_all(fg)?;
                    queue!(out, Clear(ClearType::UntilNewLine))?;
                    painted.clear();
                    paint_base(row, bg, fg, &mut painted);
                    out.write_all(&painted)?;
                }
                None => {
                    queue!(out, Clear(ClearType::UntilNewLine))?;
                    out.write_all(row)?;
                }
            }
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
/// The escape sequence a single crossterm command writes.
fn sgr(cmd: impl crossterm::Command) -> Vec<u8> {
    let mut v = Vec::new();
    let _ = queue!(v, cmd);
    v
}

/// Copies a composed row into `out`, re-pointing every "default color"
/// at the theme's: a full SGR reset is followed by `bg`/`fg`, and the
/// default-background / default-foreground SGRs become `bg` / `fg`. With
/// `transparent`, `bg` is itself the default-background SGR, so the
/// terminal's background shows through while `fg` still applies.
fn paint_base(row: &[u8], bg: &[u8], fg: &[u8], out: &mut Vec<u8>) {
    const RESET: &[u8] = b"\x1b[0m";
    const DEFAULT_BG: &[u8] = b"\x1b[49m";
    const DEFAULT_FG: &[u8] = b"\x1b[39m";
    let mut i = 0;
    while i < row.len() {
        let rest = &row[i..];
        if row[i] == 0x1b {
            if rest.starts_with(RESET) {
                out.extend_from_slice(RESET);
                out.extend_from_slice(bg);
                out.extend_from_slice(fg);
                i += RESET.len();
                continue;
            } else if rest.starts_with(DEFAULT_BG) {
                out.extend_from_slice(bg);
                i += DEFAULT_BG.len();
                continue;
            } else if rest.starts_with(DEFAULT_FG) {
                out.extend_from_slice(fg);
                i += DEFAULT_FG.len();
                continue;
            }
        }
        out.push(row[i]);
        i += 1;
    }
}

struct PaneTarget<'a> {
    frame: &'a mut [Vec<u8>],
    logical: &'a mut [Option<RowSignature>],
}

/// The content region a pane actually renders into. Computed in one place so
/// the scroll math in `prepare_view` and the early-scroll path agrees exactly
/// with what `draw_pane` draws -- otherwise, with a minimap strip or a winbar
/// row reserved, they disagree about which lines fit and the cursor can scroll
/// off-screen.
struct PaneDims {
    /// Line-number gutter width.
    gw: usize,
    /// Minimap strip width reserved on the right (0 when off/too narrow).
    map_w: usize,
    /// Usable content width (after gutter and any minimap strip).
    width: usize,
    /// Rows the winbar reserves at the top (0 or 1).
    top_off: usize,
    /// Content rows (after the status row and winbar).
    rows: usize,
}

fn pane_dims(ed: &Editor, b: &Buffer, r: Rect) -> PaneDims {
    let gw = gutter(ed, b, r.width);
    // Minimap reserves a fixed strip on the right, but only when the pane is
    // wide enough to keep a usable content column.
    let map_w = if ed.config.minimap && r.width.saturating_sub(gw) > MINIMAP_W * 2 {
        MINIMAP_W
    } else {
        0
    };
    let width = r.width.saturating_sub(gw).saturating_sub(map_w).max(1);
    // Winbar reserves the pane's top row (never in zen, and only when there's
    // room to keep at least one content row).
    let top_off = if ed.config.winbar && !ed.zen && r.height > 2 {
        1
    } else {
        0
    };
    // Zen mode and a global statusline both reclaim the per-pane status row.
    let status_row = if ed.zen || ed.config.global_statusline {
        0
    } else {
        1
    };
    // No `.max(1)` here: `draw_pane` treats a zero content height as "nothing
    // fits" (returns no cursor). Callers that need a floor apply it themselves.
    let rows = r.height.saturating_sub(status_row).saturating_sub(top_off);
    PaneDims {
        gw,
        map_w,
        width,
        top_off,
        rows,
    }
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
    let PaneDims {
        gw,
        map_w,
        width,
        top_off,
        rows: n,
    } = pane_dims(ed, b, r);
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
        .and_then(|p| crate::search::compile(&p, ed.config.ignorecase, ed.config.smartcase).ok());
    // Only paint `document_highlights` while they're still for this exact
    // buffer and it hasn't been edited since the request -- a stale set
    // would otherwise highlight whatever now sits at those old positions.
    let doc_highlighted = ed.document_highlights_buffer == Some(b.id)
        && ed.document_highlights_edit_seq == b.edit_seq;
    let colors_live =
        ed.document_colors_buffer == Some(b.id) && ed.document_colors_edit_seq == b.edit_seq;
    let sem_live = ed.config.semantic_tokens
        && ed.semantic_tokens_buffer == Some(b.id)
        && ed.semantic_tokens_edit_seq == b.edit_seq;
    let large = ed.config.large_file_kb > 0
        && b.rope.len_bytes() > ed.config.large_file_kb.saturating_mul(1024);
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
                SetForegroundColor(ed.theme.muted),
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
        // Another step of the active tour lands on this line of this
        // buffer -- keyed by buffer id (not gated to the focused pane the
        // way `sign` above is), so it still shows in an unfocused split.
        let tour_marker = ed
            .tour_markers
            .as_ref()
            .is_some_and(|(bid, lines)| *bid == b.id && lines.contains(&d.line));
        let marker = if let Some(d) = diag {
            match d.severity {
                crate::lsp::Severity::Error => 'E',
                crate::lsp::Severity::Warning => 'W',
                _ => 'I',
            }
        } else if let Some(m) = ed.tests.mark(b.path.as_deref(), d.text.as_ref()) {
            m
        } else if annotation {
            '●'
        } else if tour_marker {
            '◇'
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
        // Injected-language spans on this row (byte ranges → char columns),
        // e.g. a fenced code block's embedded highlighting in Markdown.
        let inject_ranges: Vec<(usize, usize, crate::syntax::HlClass)> =
            if ed.injection_buffer == Some(b.id) && ed.injection_edit_seq == b.edit_seq {
                let (start, end) = b.line_byte_range(d.line);
                let text = d.text.as_ref();
                ed.injection_spans
                    .iter()
                    .filter(|&&(s, e, _)| e > start && s < end)
                    .map(|&(s, e, class)| {
                        let a = safe_boundary(text, s.saturating_sub(start));
                        let z = safe_boundary(text, e.min(end).saturating_sub(start));
                        (text[..a].chars().count(), text[..z].chars().count(), class)
                    })
                    .collect()
            } else {
                Vec::new()
            };
        // Conceal spans on this row: matches of the compiled conceal rules,
        // hidden (`None`) or replaced by a cchar. Empty on the revealed line
        // (the active window's cursor line) so it always shows its real text.
        let conceal: Vec<(usize, usize, Option<char>)> = if ed.config.conceal
            && !ed.conceal_compiled.is_empty()
            && !(active && d.line == w.cursor.0)
        {
            conceal_line_ranges(&ed.conceal_compiled, d.text.as_ref())
        } else {
            Vec::new()
        };
        // Inline ghost-text suggestion for this row (cursor line only).
        let ghost_str: Option<String> = ed
            .ghost
            .as_ref()
            .filter(|(gl, _, _)| *gl == d.line && b.id == ed.buf().id)
            .map(|(_, _, t)| t.clone());
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
        // Semantic-token spans on this row: (start_col, end_col, palette,
        // deprecated→strike, readonly→italic).
        let sem_row: Vec<(usize, usize, u8, bool, bool)> = if sem_live {
            ed.semantic_tokens
                .iter()
                .filter(|&&(l, _, _, _, _, _)| l == d.line)
                .map(|&(_, c1, c2, p, dep, ro)| (c1, c2, p, dep, ro))
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
        // Within the active tour's current step range (highlight the code the
        // step points at).
        let tour_hl = ed
            .tour_highlight
            .is_some_and(|(bid, s, e)| bid == b.id && d.line >= s && d.line <= e);
        // A single row-level background: diff highlight wins over the tour
        // highlight, which wins over cursorline. Diff colors by side — the first
        // diffed buffer (old) red, the second (new) green.
        let row_bg: Option<Color> = if diff_line {
            if ed.diff_buffers.first() == Some(&b.id) {
                Some(ed.theme.diff_del_bg)
            } else {
                Some(ed.theme.diff_add_bg)
            }
        } else if tour_hl {
            Some(ed.theme.tour_bg)
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
            conceal: conceal.clone(),
            color_swatch: ed.config.colorswatch,
            rainbow: rainbow_row.clone(),
            sem_ranges: sem_row.clone(),
            spell_ranges: spell_ranges.clone(),
            todo_ranges: todo_ranges.clone(),
            inject_ranges: inject_ranges.clone(),
            ghost: ghost_str.clone(),
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
            tour_hl,
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
            // Injected-language spans (e.g. rust in a Markdown fence) for this
            // row, already resolved to char columns (see `inject_ranges`).
            spans.extend(inject_ranges.iter().copied());
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
        let margin = if gw >= 2 {
            // Assemble the gutter in the configured component order; `pad` below
            // fixes the final width, so a partial/custom order stays safe.
            let comps = parse_statuscolumn(&ed.config.statuscolumn, ed.config.foldcolumn);
            let single = comps.iter().filter(|c| **c != GutterComp::Num).count();
            let num_width = gw.saturating_sub(1 + single);
            let mut margin = String::new();
            for comp in &comps {
                match comp {
                    GutterComp::Fold => margin.push(fold_marker),
                    GutterComp::Diag => margin.push(marker),
                    GutterComp::Git => margin.push(sign),
                    GutterComp::Num => margin.push_str(&format!("{number:>num_width$}")),
                }
            }
            margin.push(' ');
            margin
        } else {
            " ".repeat(gw)
        };
        if let Some(bg) = row_bg {
            queue!(dest, SetBackgroundColor(bg))?;
        }
        queue!(
            dest,
            SetForegroundColor(if d.line == w.cursor.0 {
                ed.theme.line_nr_current
            } else {
                ed.theme.line_nr
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
        let hint_style = (
            false,
            false,
            false,
            false,
            ed.theme.muted,
            None,
            false,
            false,
            None,
            false,
        );
        let mut hint_idx = 0;
        let mut splice_hints_up_to =
            |col: usize, runs: &mut Vec<(GlyphStyle, String)>, used: &mut usize| {
                while hint_idx < line_hints.len() && line_hints[hint_idx].0 <= col {
                    // Inlay hints are spliced *among* the glyphs, which are
                    // already clipped to the pane width, so budget each hint
                    // against the remaining cells -- otherwise a hint on a full
                    // row spills past the pane into the minimap/separator.
                    if *used >= width {
                        break;
                    }
                    let shown = clip(&line_hints[hint_idx].1, width - *used);
                    if shown.is_empty() {
                        break;
                    }
                    let shown_w = shown.width();
                    if let Some((prev, text)) = runs.last_mut() {
                        if *prev == hint_style {
                            text.push_str(&shown);
                        } else {
                            runs.push((hint_style, shown));
                        }
                    } else {
                        runs.push((hint_style, shown));
                    }
                    *used += shown_w;
                    hint_idx += 1;
                }
            };
        // `list` (listchars) reveals tabs (`>`, `-`) and trailing whitespace
        // (`·`). `d.text` is the whole logical line, so `trail_start` — the
        // column after its last non-blank char — is the logical trailing
        // threshold even for a wrapped segment (its glyphs keep full-line cols).
        let (row_chars, trail_start): (Vec<char>, usize) = if ed.config.list {
            let rc: Vec<char> = d.text.chars().collect();
            let ts = rc
                .iter()
                .rposition(|c| !c.is_whitespace())
                .map_or(0, |p| p + 1);
            (rc, ts)
        } else {
            (Vec::new(), 0)
        };
        let mut prev_col: Option<usize> = None;
        for g in d.glyphs.iter() {
            splice_hints_up_to(g.col, &mut runs, &mut used);
            // Conceal: a matched glyph is hidden, or the match's first glyph is
            // replaced by its cchar (dim) and the rest hidden. Never runs on the
            // revealed line (conceal is empty there).
            if let Some(&(start, _end, cchar)) =
                conceal.iter().find(|(a, z, _)| g.col >= *a && g.col < *z)
            {
                if let (Some(ch), true) = (cchar, g.col == start) {
                    let style: GlyphStyle = (
                        false,
                        false,
                        false,
                        false,
                        ed.theme.muted,
                        None,
                        false,
                        false,
                        None,
                        false,
                    );
                    let s = ch.to_string();
                    match runs.last_mut() {
                        Some((prev, text)) if *prev == style => text.push_str(&s),
                        _ => runs.push((style, s)),
                    }
                    used += 1;
                }
                // Hidden char (or a non-first char of a cchar match): drop it,
                // taking zero display columns.
                prev_col = Some(g.col);
                continue;
            }
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
                .find(|(a, z, _, _, _)| g.col >= *a && g.col < *z)
                .map(|&(_, _, pal, _, _)| semantic_color(ed, pal))
                .unwrap_or(color);
            // A `deprecated` semantic token draws struck-through; a `readonly`
            // one draws italic.
            let sem_strike = sem_row
                .iter()
                .any(|&(a, z, _, dep, _)| dep && g.col >= a && g.col < z);
            let sem_italic = sem_row
                .iter()
                .any(|&(a, z, _, _, ro)| ro && g.col >= a && g.col < z);
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
                .map(|sev| ed.theme.severity(sev));
            // Spell underline (magenta), only where no diagnostic already
            // underlines the glyph so diagnostics stay visually dominant.
            let diag_underline = diag_underline.or_else(|| {
                spell_ranges
                    .iter()
                    .any(|(a, z)| g.col >= *a && g.col < *z)
                    .then_some(ed.theme.special)
            });
            // The colorcolumn ruler falls on whichever glyph *covers* that
            // display cell (`used` is this glyph's start column, before it
            // advances). Testing the whole `used..used+width` span -- not just
            // the start -- keeps the ruler visible when a double-width glyph
            // straddles the column.
            let colorcol = ed.config.colorcolumn > 0
                && used < ed.config.colorcolumn
                && ed.config.colorcolumn <= used + g.width;
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
                    0 => ed.theme.warning,
                    1 => ed.theme.error,
                    _ => ed.theme.special,
                })
                .unwrap_or(color);
            // listchars substitution (dimmed): tab lead/fill, or trailing ws.
            let (gtext, color) = if ed.config.list {
                let src = row_chars.get(g.col).copied();
                if src == Some('\t') {
                    let lead = prev_col != Some(g.col); // first cell of this tab
                    (
                        (if lead { lc_lead } else { lc_fill }).to_string(),
                        ed.theme.muted,
                    )
                } else if g.col >= trail_start && src.is_some_and(|c| c == ' ' || c == '\t') {
                    (lc_trail.to_string(), ed.theme.muted)
                } else {
                    (g.text.clone(), color)
                }
            } else {
                (g.text.clone(), color)
            };
            // Color swatch: with `:set colorswatch`, a documentColor literal is
            // painted as a chip — its own RGB as the *background* (with a
            // luminance-contrasted foreground) rather than only the foreground.
            let swatch = if ed.config.colorswatch {
                color_ranges
                    .iter()
                    .find(|(a, z, _)| g.col >= *a && g.col < *z)
                    .map(|&(_, _, (cr, cg, cb))| Color::Rgb {
                        r: cr,
                        g: cg,
                        b: cb,
                    })
            } else {
                None
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
                swatch,
                sem_italic,
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
            (
                selected,
                searched,
                doc_hl,
                word_diff_hl,
                color,
                diag_underline,
                colorcol,
                strike,
                swatch,
                italic,
            ),
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
            } else if let Some(sw) = swatch {
                // Contrast the chip's text against its own color so it reads.
                contrast_on(sw)
            } else {
                color
            };
            if selected {
                queue!(dest, SetAttribute(Attribute::Reverse))?;
            } else if searched {
                queue!(dest, SetBackgroundColor(ed.theme.search_bg))?;
            } else if doc_hl {
                queue!(dest, SetBackgroundColor(ed.theme.doc_highlight_bg))?;
            } else if word_diff_hl {
                // `,gd`'s diff overlay: the word(s) that actually changed
                // within an otherwise-unchanged line, distinct from the
                // gutter's whole-line "modified" sign.
                queue!(dest, SetBackgroundColor(ed.theme.word_diff_bg))?;
            } else if let Some(sw) = swatch {
                // documentColor chip (`:set colorswatch`): the literal's own
                // RGB as its background.
                queue!(dest, SetBackgroundColor(sw))?;
            } else if colorcol {
                queue!(dest, SetBackgroundColor(ed.theme.colorcolumn_bg))?;
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
            // of any background/underline above); a `readonly` one is italic.
            if strike {
                queue!(dest, SetAttribute(Attribute::CrossedOut))?;
            }
            if italic {
                queue!(dest, SetAttribute(Attribute::Italic))?;
            }
            queue!(
                dest,
                SetForegroundColor(fg),
                Print(text),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
        }
        // Inline ghost-text: dimmed, right after the content (at the cursor).
        if let Some(text) = &ghost_str {
            let remaining = width.saturating_sub(used);
            if remaining > 0 {
                let shown = clip_tab(text, remaining, b.tabstop);
                queue!(
                    dest,
                    SetForegroundColor(ed.theme.muted),
                    Print(&shown),
                    ResetColor
                )?;
                used += shown.width();
            }
        }
        if let Some(text) = &diag_text {
            let remaining = width.saturating_sub(used);
            if remaining > 2 {
                let color = diag
                    .map(|d2| ed.theme.severity(d2.severity))
                    .unwrap_or(ed.theme.muted);
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
                    SetForegroundColor(ed.theme.muted),
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
                    SetForegroundColor(ed.theme.git_delete),
                    Print(&shown),
                    ResetColor
                )?;
                used += shown.width();
            }
        }
        // The colorcolumn ruler, when it falls past end-of-line, splits the
        // trailing pad into [pad .. ruler), the ruler cell, and [ruler .. end).
        let ruler = if ed.config.colorcolumn > used && ed.config.colorcolumn - 1 < width {
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
                queue!(
                    dest,
                    SetBackgroundColor(ed.theme.colorcolumn_bg),
                    Print(" "),
                    ResetColor
                )?;
                fill(dest, width.saturating_sub(rc + 1))?;
            }
            None => fill(dest, width.saturating_sub(used))?,
        }
        cache.composed.insert(sig, dest[content_start..].to_vec());
    }
    // Sticky scroll: pin the enclosing function/class declaration lines that
    // have scrolled off the top of this pane, overlaying the first rows.
    if ed.config.sticky_scroll && !ed.zen && b.id == ed.buf().id && w.top > 0 {
        let lines = sticky_context_lines(ed, b, w.top);
        let k = lines.len().min(3).min(r.height.saturating_sub(2));
        for (i, &line) in lines.iter().take(k).enumerate() {
            let body = clip_tab(&b.line_text(line), r.width.saturating_sub(gw), b.tabstop);
            let text = format!("{}{}", " ".repeat(gw), body);
            plain_row(
                target.frame,
                r.y + top_off + i,
                r.x,
                r.width,
                &text,
                ed.theme.sticky_bg,
            )?;
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
                    ed.theme.inccommand_bg,
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
                    ed.theme.fold_bg,
                )?;
            }
        }
    }
    // Minimap strip on the right, drawn last so it overlays cleanly (including
    // over any sticky-scroll header rows) in its reserved columns.
    if map_w > 0 {
        draw_minimap(
            target.frame,
            ed,
            b,
            &display,
            r,
            gw,
            width,
            map_w,
            n,
            top_off,
        )?;
    }
    // Winbar: the pane's top chrome row (path + enclosing-symbol breadcrumb).
    if top_off > 0 {
        draw_winbar(target.frame, ed, b, w, r, gw)?;
    }
    // Per-pane status line, unless zen (no chrome) or a global statusline is
    // configured (one shared line drawn by `draw` instead).
    if !ed.zen && !ed.config.global_statusline {
        let label = statusline_label(ed, b, w, r.width, active);
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

/// Build a window's status-line text (left segment padded, `line:col` ruler on
/// the right). Shared by the per-pane statusline and the global statusline.
pub(crate) fn statusline_label(
    ed: &Editor,
    b: &Buffer,
    w: &Window,
    width: usize,
    active: bool,
) -> String {
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
    // Persistent LSP progress indicator (active pane only, clipped).
    let progress = if active {
        ed.format_lsp_progress()
    } else {
        None
    }
    .map(|p| format!("{} · ", clip(&p, 40)))
    .unwrap_or_default();
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
            " {} {}{}{}{}",
            mode_label,
            name,
            if b.is_modified() { " [+]" } else { "" },
            if b.disk_changed {
                " [changed on disk]"
            } else {
                ""
            },
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
    format!(
        "{}{}",
        pad(&left, width.saturating_sub(right.width())),
        right
    )
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
        SetBackgroundColor(ed.theme.panel_bg),
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
        ed.theme.title_bg,
    )?;
    for (i, a) in items.iter().take(visible).enumerate() {
        plain_row(
            frame,
            y + 1 + i,
            x,
            w,
            &format!(" {}{:<6}{}", ed.config.leader, a.keys, a.title),
            ed.theme.panel_bg,
        )?;
    }
    Ok(())
}
/// Paints the floating window (see `src/float.rs`) anchored at the screen
/// cell `anchor` (the active cursor). Returns where the terminal cursor
/// belongs when the float is focused: on the selected list row or the
/// previewed line, so the hardware cursor isn't left blinking in the
/// buffer underneath.
fn draw_float(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    cache: &mut FrameCache,
    f: &crate::float::Float,
    anchor: (usize, usize),
    width: usize,
    height: usize,
) -> io::Result<Option<(usize, usize)>> {
    use crate::float::RowStyle;
    if width < 12 || height < 5 {
        return Ok(None);
    }
    let mut source = f
        .selected_entry()
        .and_then(|e| e.path.as_deref())
        .map(|p| cached_preview_source(ed, cache, p))
        .unwrap_or_default();
    // An open buffer's lines include the empty "line" after a final
    // newline; don't preview it as a real one.
    if source.len() > 1 && source.last().is_some_and(String::is_empty) {
        source.pop();
    }
    let w = crate::float::outer_width(width);
    let inner_h = f
        .wanted_height(source.len())
        .min(height.saturating_sub(3).max(1));
    let r = crate::float::place(anchor, w, inner_h + 2, width, height);
    let inner_w = r.width.saturating_sub(2);
    let inner_h = r.height.saturating_sub(2);
    let border = if f.focused {
        ed.theme.accent
    } else {
        ed.theme.muted
    };
    // `─ title ─────` and `─ hints ───` labels, clipped to the box.
    let label_rule = |label: &str| {
        let label = clip(&format!("─ {label} "), inner_w);
        let lw = UnicodeWidthStr::width(label.as_str());
        format!("{label}{}", "─".repeat(inner_w.saturating_sub(lw)))
    };
    let hints = if !f.focused {
        ",pf focus · Esc close".to_string()
    } else if f.selected_entry().is_some() {
        "q close · Enter open · s/v split · j/k".to_string()
    } else {
        "q close · j/k scroll".to_string()
    };
    float_border_row(
        frame,
        r.y,
        r.x,
        &format!("╭{}╮", label_rule(&f.title)),
        border,
        ed.theme.float_bg,
    )?;
    let rows = f.rows(&source, &ed.project_root, inner_h);
    let tab = ed.buf().tabstop.max(1);
    let mut focus = None;
    for i in 0..inner_h {
        let y = r.y + 1 + i;
        let (text, bg) = match rows.get(i) {
            Some(row) => {
                let bg = match row.style {
                    RowStyle::Text | RowStyle::ListItem => ed.theme.float_bg,
                    RowStyle::Target | RowStyle::ListSelected => ed.theme.selection_bg,
                    RowStyle::Rule => ed.theme.panel_bg,
                };
                if matches!(row.style, RowStyle::Target | RowStyle::ListSelected) && focus.is_none()
                {
                    focus = Some((r.x + 1, y));
                }
                let text = if row.style == RowStyle::Rule {
                    let label = format!("── {} ", row.text);
                    let lw = UnicodeWidthStr::width(label.as_str());
                    format!("{label}{}", "─".repeat(inner_w.saturating_sub(lw)))
                } else {
                    format!(" {}", row.text)
                };
                (text, bg)
            }
            None => (String::new(), ed.theme.float_bg),
        };
        float_border_row(frame, y, r.x, "│", border, ed.theme.float_bg)?;
        if let Some(line) = frame.get_mut(y) {
            let fg = if bg == ed.theme.float_bg {
                Color::Reset
            } else {
                Color::White
            };
            queue!(
                line,
                MoveTo((r.x + 1) as u16, y as u16),
                SetBackgroundColor(bg),
                SetForegroundColor(fg),
                Print(pad_tab(&text, inner_w, tab)),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
        }
        float_border_row(frame, y, r.x + 1 + inner_w, "│", border, ed.theme.float_bg)?;
    }
    float_border_row(
        frame,
        r.y + r.height - 1,
        r.x,
        &format!("╰{}╯", label_rule(&hints)),
        border,
        ed.theme.float_bg,
    )?;
    Ok(f.focused.then(|| focus.unwrap_or((r.x + 1, r.y + 1))))
}
fn float_border_row(
    frame: &mut [Vec<u8>],
    y: usize,
    x: usize,
    text: &str,
    color: Color,
    bg: Color,
) -> io::Result<()> {
    if let Some(row) = frame.get_mut(y) {
        queue!(
            row,
            MoveTo(x as u16, y as u16),
            SetBackgroundColor(bg),
            SetForegroundColor(color),
            Print(text),
            ResetColor,
            SetAttribute(Attribute::Reset)
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
        ed.theme.bar_bg,
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
                ed.theme.selection_bg
            } else {
                Color::Reset
            },
        )?;
    }
    let prompt = if let Some(forward) = r.search_input {
        format!(
            "{}{}{}",
            if forward { '/' } else { '?' },
            r.query,
            if r.live && r.grep_fixed {
                "  [fixed]"
            } else {
                ""
            }
        )
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
            plain_row(frame, detail_y, 0, width, &border, ed.theme.panel_bg)?;
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
                        ed.theme.selection_bg
                    } else {
                        ed.theme.panel_bg
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
                    ed.theme.panel_bg,
                )?;
            }
        }
    }
    let footer = if r.search_input.is_some() && r.live {
        if r.qcursor.insert {
            "Esc query-normal (h/w/b move, i/a insert, y yank, u undo) · Ctrl-f fixed-string · ↑↓ history"
        } else {
            "Esc close · h/w/b move · y yank · u undo · p paste · Ctrl-f fixed-string"
        }
    } else if r.search_input.is_some() || r.filter_input {
        if r.qcursor.insert {
            "Esc query-normal (h/w/b move, i/a insert, y yank, u undo) · Esc Esc close"
        } else {
            "Esc close · h/w/b move · y yank · u undo · p paste"
        }
    } else if r.git_status && r.preview {
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
    plain_row(frame, height - 2, 0, width, footer, ed.theme.bar_bg)?;
    plain_row(
        frame,
        height - 1,
        0,
        width,
        r.error.as_deref().unwrap_or(&ed.message),
        Color::Reset,
    )?;
    Ok(if r.search_input.is_some() || r.filter_input {
        // Column tracks `qcursor.pos` (a char index into `query`/`filter`,
        // not the end of the string) so the terminal cursor lands where
        // Normal/Insert sub-mode editing actually is, matching how a real
        // buffer's cursor is never just parked at end-of-line.
        let prefix_chars = if r.search_input.is_some() {
            1 // '/' or '?'
        } else {
            "Filter: ".chars().count()
        };
        let upto: String = prompt.chars().take(prefix_chars + r.qcursor.pos).collect();
        (clip(&upto, width.saturating_sub(1)).width(), 1)
    } else {
        (0, (r.cursor - first + 2).min(height - 1))
    })
}
/// The `:far` replace screen: a summary bar, the three input fields, the
/// match list grouped by file (each match shown as the line it becomes),
/// and -- given the room -- a `-`/`+` preview of the line under the cursor
/// in its surrounding source.
fn draw_far(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    cache: &mut FrameCache,
    width: usize,
    height: usize,
) -> io::Result<(usize, usize)> {
    use crate::far::{Field, Row};
    let Some(far) = &ed.far else {
        return Ok((0, 0));
    };
    plain_row(
        frame,
        0,
        0,
        width,
        &format!(
            " Search & replace · {}{} match(es) in {} file(s) · {} selected{}",
            far.match_count(),
            if far.truncated { "+" } else { "" },
            far.files.len(),
            far.enabled_count(),
            if far.busy { " · searching…" } else { "" }
        ),
        ed.theme.bar_bg,
    )?;
    if height < 8 {
        return Ok((0, 0));
    }
    let mut cursor = None;
    for (i, field) in Field::ALL.into_iter().enumerate() {
        let editing = far.editing == Some(field);
        let prefix = format!("{}{} ", if editing { "▸" } else { " " }, field.label());
        let text = far.text(field);
        if text.is_empty() && !editing {
            let hint = match field {
                Field::Search => "(regex, as in :s -- s to edit)",
                Field::Replace => "(\\1 / & for groups; empty deletes -- r to edit)",
                Field::Files => "(all files · e.g. *.rs !tests/** -- f to edit)",
            };
            plain_row(frame, 1 + i, 0, width, &prefix, Color::Reset)?;
            let x = prefix.width().min(width);
            float_border_row(
                frame,
                1 + i,
                x,
                &pad(hint, width - x),
                ed.theme.muted,
                Color::Reset,
            )?;
        } else {
            plain_row(
                frame,
                1 + i,
                0,
                width,
                &format!("{prefix}{text}"),
                Color::Reset,
            )?;
        }
        if editing {
            let upto: String = text.chars().take(far.qcursor.pos).collect();
            let x = clip(&format!("{prefix}{upto}"), width.saturating_sub(1)).width();
            cursor = Some((x, 1 + i));
        }
    }
    let options = format!(
        " {} · {}",
        if far.fixed { "literal" } else { "regex" },
        far.case.label()
    );
    match &far.error {
        Some(e) => {
            plain_row(frame, 4, 0, width, &options, Color::Reset)?;
            let x = (options.width() + 3).min(width);
            float_border_row(
                frame,
                4,
                x,
                &pad(e, width - x),
                ed.theme.error,
                Color::Reset,
            )?;
        }
        None => float_border_row(
            frame,
            4,
            0,
            &pad(&options, width),
            ed.theme.muted,
            Color::Reset,
        )?,
    }
    let avail = height.saturating_sub(5 + 2);
    let preview_rows = if avail >= 12 { (avail / 3).max(4) } else { 0 };
    let list_rows = avail - preview_rows;
    let rows = far.rows();
    let first = far.cursor.saturating_sub(list_rows.saturating_sub(1));
    for i in 0..list_rows {
        let y = 5 + i;
        let idx = first + i;
        let Some(row) = rows.get(idx).copied() else {
            let text = if idx == 0 && far.search.is_empty() {
                "  Type a search pattern to list matches"
            } else if idx == 0 && !far.busy && far.error.is_none() {
                "  No matches"
            } else {
                ""
            };
            plain_row(frame, y, 0, width, text, Color::Reset)?;
            continue;
        };
        let (text, fg) = match row {
            Row::File(fi) => {
                let f = &far.files[fi];
                let on = far.enabled_in(fi);
                let mark = if on == f.matches.len() {
                    "[x]"
                } else if on == 0 {
                    "[ ]"
                } else {
                    "[-]"
                };
                let rel = f.path.strip_prefix(&ed.project_root).unwrap_or(&f.path);
                (
                    format!("{mark} {}  ({on}/{})", rel.display(), f.matches.len()),
                    ed.theme.accent,
                )
            }
            Row::Match(fi, mi) => {
                let f = &far.files[fi];
                let m = &f.matches[mi];
                let on = far.is_enabled(&f.path, m);
                let line = if on { m.new_line() } else { m.old.clone() };
                (
                    format!(
                        "    {} {:>5}:{:<3} {}",
                        if on { "[x]" } else { "[ ]" },
                        m.line + 1,
                        m.col() + 1,
                        line.trim_start().replace('\n', "⏎")
                    ),
                    if on { Color::Reset } else { ed.theme.muted },
                )
            }
        };
        if idx == far.cursor {
            plain_row(frame, y, 0, width, &text, ed.theme.selection_bg)?;
        } else {
            float_border_row(frame, y, 0, &pad(&text, width), fg, Color::Reset)?;
        }
    }
    if preview_rows > 0 {
        let y0 = 5 + list_rows;
        let target = match rows.get(far.cursor).copied() {
            Some(Row::Match(fi, mi)) => Some((fi, mi)),
            Some(Row::File(fi)) => Some((fi, 0)),
            None => None,
        };
        let label = match target {
            Some((fi, mi)) => {
                let f = &far.files[fi];
                let rel = f.path.strip_prefix(&ed.project_root).unwrap_or(&f.path);
                format!("── preview · {}:{} ", rel.display(), f.matches[mi].line + 1)
            }
            None => "── preview ".to_string(),
        };
        let lw = UnicodeWidthStr::width(label.as_str());
        let rule = format!(
            "{}{}",
            clip(&label, width),
            "─".repeat(width.saturating_sub(lw))
        );
        plain_row(frame, y0, 0, width, &rule, ed.theme.panel_bg)?;
        let body = preview_rows - 1;
        let mut lines: Vec<(String, Color)> = Vec::new();
        if let Some((fi, mi)) = target {
            let f = &far.files[fi];
            let m = &f.matches[mi];
            let source = cached_preview_source(ed, cache, &f.path);
            // What applying would make of the whole line: every selected
            // occurrence on it, not just the one under the cursor.
            let on: Vec<_> = f
                .matches
                .iter()
                .filter(|o| o.line == m.line && far.is_enabled(&f.path, o))
                .cloned()
                .collect();
            let before = body.saturating_sub(2) / 2;
            for l in m.line.saturating_sub(before)..m.line {
                lines.push((
                    format!("  {}", source.get(l).map_or("", |s| s)),
                    ed.theme.muted,
                ));
            }
            if on.is_empty() {
                lines.push((format!("  {}", m.old), Color::Reset));
            } else {
                lines.push((format!("- {}", m.old), ed.theme.git_delete));
                let new = crate::far::replace_line(&m.old, &on);
                for part in new.split('\n') {
                    lines.push((format!("+ {part}"), ed.theme.git_add));
                }
            }
            let mut l = m.line + 1;
            while lines.len() < body && l < source.len() {
                lines.push((format!("  {}", source[l]), ed.theme.muted));
                l += 1;
            }
        }
        let tab = ed.buf().tabstop.max(1);
        for i in 0..body {
            let (text, fg) = lines
                .get(i)
                .cloned()
                .unwrap_or((String::new(), Color::Reset));
            float_border_row(
                frame,
                y0 + 1 + i,
                0,
                &pad_tab(&text, width, tab),
                fg,
                Color::Reset,
            )?;
        }
    }
    let footer = if far.editing.is_some() {
        "Tab/S-Tab next/prev field · Enter list · Esc query-normal (Esc Esc list)"
    } else {
        "q close · Space toggle · a all · s/r/f edit · c case · F literal · R replace · U undo · Enter open"
    };
    plain_row(frame, height - 2, 0, width, footer, ed.theme.bar_bg)?;
    plain_row(frame, height - 1, 0, width, &ed.message, Color::Reset)?;
    Ok(cursor.unwrap_or((0, (5 + far.cursor - first).min(height - 1))))
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
        ed.theme.bar_bg,
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
        let idx = i + start;
        let marker = if p.marked.contains(&idx) {
            "● "
        } else {
            "  "
        };
        plain_row(
            frame,
            i + 1,
            0,
            width,
            &format!("{marker}{path}"),
            if idx == p.selected {
                ed.theme.selection_bg
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
        plain_row(frame, preview_y, 0, width, &border, ed.theme.panel_bg)?;
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
            "{}/{} files{}{} · Enter open · Tab mark · Ctrl-Q quickfix{} · Ctrl-r preview{} · {}",
            p.matches.len(),
            p.stats.matched,
            if p.marked.is_empty() {
                String::new()
            } else {
                format!(" ({} marked)", p.marked.len())
            },
            if ed.search_job.files_rx.is_some() {
                " · scanning…"
            } else {
                ""
            },
            if p.marked.is_empty() { "" } else { " (marked)" },
            if p.preview {
                " (on) · Ctrl-e/y scroll"
            } else {
                ""
            },
            if p.qcursor.insert {
                "Esc query-normal (h/w/b move, i/a insert, y yank, u undo) · Esc Esc close"
            } else {
                "Esc close"
            },
        ),
        ed.theme.bar_bg,
    )?;
    Ok((
        {
            let upto: String = p.query.chars().take(p.qcursor.pos).collect();
            clip(&format!("> {}", upto), width.saturating_sub(1)).width()
        },
        0,
    ))
}
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

/// One styled run of a file tree row.
struct TreeSeg {
    text: String,
    fg: Option<Color>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

impl TreeSeg {
    fn new(text: impl Into<String>, fg: Option<Color>) -> Self {
        TreeSeg {
            text: text.into(),
            fg,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
        }
    }
    fn width(&self) -> usize {
        UnicodeWidthStr::width(self.text.as_str())
    }
}

/// Writes `segs` as one `width`-cell row at (x, y) on `bg`, truncating
/// whatever doesn't fit.
fn tree_row(
    row: &mut Vec<u8>,
    x: usize,
    y: usize,
    width: usize,
    segs: &[TreeSeg],
    bg: Option<Color>,
) -> io::Result<()> {
    queue!(row, MoveTo(x as u16, y as u16))?;
    if let Some(bg) = bg {
        queue!(row, SetBackgroundColor(bg))?;
    }
    let mut used = 0;
    for s in segs {
        if used >= width {
            break;
        }
        let text = clip(&s.text, width - used);
        used += UnicodeWidthStr::width(text.as_str());
        queue!(row, SetForegroundColor(s.fg.unwrap_or(Color::Reset)))?;
        if s.bold {
            queue!(row, SetAttribute(Attribute::Bold))?;
        }
        if s.dim {
            queue!(row, SetAttribute(Attribute::Dim))?;
        }
        if s.italic {
            queue!(row, SetAttribute(Attribute::Italic))?;
        }
        if s.underline {
            queue!(row, SetAttribute(Attribute::Underlined))?;
        }
        queue!(row, Print(text))?;
        if s.bold || s.dim {
            queue!(row, SetAttribute(Attribute::NormalIntensity))?;
        }
        if s.italic {
            queue!(row, SetAttribute(Attribute::NoItalic))?;
        }
        if s.underline {
            queue!(row, SetAttribute(Attribute::NoUnderline))?;
        }
    }
    queue!(
        row,
        SetForegroundColor(Color::Reset),
        Print(" ".repeat(width.saturating_sub(used))),
        SetAttribute(Attribute::Reset),
        ResetColor
    )
}

/// `~`-shortened display of the tree root for the header.
fn tree_root_label(root: &std::path::Path) -> String {
    let full = root.display().to_string();
    match dirs::home_dir().map(|h| h.display().to_string()) {
        Some(home) if full.starts_with(&home) && home.len() > 1 => {
            format!("~{}", &full[home.len()..])
        }
        _ => full,
    }
}

/// Renders the file tree sidebar: a header (root, live filter, view flags,
/// marks/clipboard counts) above one row per visible node -- indent guides,
/// an expander arrow, an optional Nerd Font icon, and the name colored by
/// kind/git status (open buffers bold, the current file underlined, filter
/// hits highlighted) -- with git / diagnostic / bookmark / unsaved badges
/// right-aligned. Per-directory diagnostic roll-ups are computed once per
/// frame here and git roll-ups once per refresh, so a row costs O(1)
/// lookups no matter how many files have markers. `?` swaps the list
/// for the key reference.
fn draw_file_tree_pane(
    frame: &mut [Vec<u8>],
    ed: &Editor,
    tree: &crate::filetree::FileTree,
    rect: Rect,
    active: bool,
) -> io::Result<Option<(usize, usize)>> {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    if rect.height == 0 || rect.width == 0 {
        return Ok(None);
    }
    let icons = ed.config.tree_icons;
    let mut cursor = None;
    let dirc = Some(ed.theme.tree_dir);
    let guide = Some(ed.theme.muted);

    // Header.
    let header = crate::filetree::HEADER_ROWS.min(rect.height);
    if header > 0 {
        if let Some(row) = frame.get_mut(rect.y) {
            let mut segs = Vec::new();
            let mut right = Vec::new();
            if tree.show_help {
                let mut s = TreeSeg::new(" Tree keys", Some(ed.theme.accent));
                s.bold = true;
                segs.push(s);
                segs.push(TreeSeg::new(" (j/k scroll)", guide));
            } else {
                let name = tree
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| tree_root_label(&tree.root));
                let mut s = TreeSeg::new(
                    if icons {
                        format!(" \u{f07c} {name}")
                    } else {
                        format!(" {name}")
                    },
                    Some(ed.theme.accent),
                );
                s.bold = true;
                segs.push(s);
                if !tree.filter.is_empty() || tree.filter_input {
                    let mut q = TreeSeg::new(format!("  /{}", tree.filter), Some(ed.theme.warning));
                    q.bold = true;
                    if active && tree.filter_input {
                        let x = segs.iter().map(TreeSeg::width).sum::<usize>() + q.width();
                        cursor = Some((rect.x + x.min(rect.width.saturating_sub(1)), rect.y));
                    }
                    segs.push(q);
                    segs.push(TreeSeg::new(format!(" {}", tree.matched.len()), guide));
                } else if tree.root != ed.project_root {
                    segs.push(TreeSeg::new(
                        format!(" {}", tree_root_label(&tree.root)),
                        guide,
                    ));
                }
                if !tree.marked.is_empty() {
                    right.push(TreeSeg::new(
                        format!("✓{} ", tree.marked.len()),
                        Some(ed.theme.special),
                    ));
                }
                if let Some((items, cut)) = &tree.clipboard {
                    right.push(TreeSeg::new(
                        format!("{}{} ", if *cut { "✂" } else { "⎘" }, items.len()),
                        Some(ed.theme.accent),
                    ));
                }
                if tree.show_hidden {
                    right.push(TreeSeg::new("H ", guide));
                }
                if tree.show_ignored {
                    right.push(TreeSeg::new("I ", guide));
                }
            }
            let rw: usize = right.iter().map(TreeSeg::width).sum();
            let lw: usize = segs.iter().map(TreeSeg::width).sum();
            if rw > 0 && lw + rw < rect.width {
                segs.push(TreeSeg::new(" ".repeat(rect.width - lw - rw), None));
                segs.extend(right);
            }
            tree_row(row, rect.x, rect.y, rect.width, &segs, None)?;
        }
    }
    let list_y = rect.y + header;
    let list_h = rect.height - header;

    if tree.show_help {
        let entries = crate::filetree::HELP;
        let colw = 10;
        for y in 0..list_h {
            let Some(row) = frame.get_mut(list_y + y) else {
                continue;
            };
            let segs = match entries.get(tree.help_scroll + y) {
                Some((k, d)) => vec![
                    TreeSeg::new(format!(" {:<w$}", k, w = colw - 1), Some(ed.theme.warning)),
                    TreeSeg::new(*d, None),
                ],
                None => Vec::new(),
            };
            tree_row(row, rect.x, list_y + y, rect.width, &segs, None)?;
        }
        return Ok(cursor);
    }

    // Per-frame lookups.
    let diag_rank = |s: crate::lsp::Severity| match s {
        crate::lsp::Severity::Error => 0u8,
        crate::lsp::Severity::Warning => 1,
        _ => 2,
    };
    let mut diag_dirs: HashMap<&Path, u8> = HashMap::new();
    for (p, ds) in &ed.diagnostics {
        let Some(worst) = ds.iter().map(|d| diag_rank(d.severity)).min() else {
            continue;
        };
        for a in p.ancestors().skip(1) {
            if !a.starts_with(&tree.root) {
                break;
            }
            match diag_dirs.get(a) {
                Some(&have) if have <= worst => break,
                _ => {
                    diag_dirs.insert(a, worst);
                }
            }
        }
    }
    let mut open_bufs: HashSet<&Path> = HashSet::new();
    let mut dirty: HashSet<&Path> = HashSet::new();
    for b in &ed.buffers {
        if let Some(p) = &b.path {
            open_bufs.insert(p);
            if b.is_modified() {
                dirty.insert(p);
            }
        }
    }
    let current: Option<&Path> = ed
        .windows
        .get(tree.last_edit_window)
        .and_then(|w| ed.buffers.iter().find(|b| b.id == w.buffer))
        .or_else(|| ed.buffers.get(ed.cur))
        .and_then(|b| b.path.as_deref());
    let cut: HashSet<&Path> = match &tree.clipboard {
        Some((items, true)) => items.iter().map(std::path::PathBuf::as_path).collect(),
        _ => HashSet::new(),
    };
    let row_bg = if active {
        ed.theme.tree_cursor_bg
    } else {
        ed.theme.cursorline_bg
    };

    if tree.nodes.is_empty() && list_h > 0 {
        if let Some(row) = frame.get_mut(list_y) {
            let msg = if tree.filter.is_empty() {
                "  (empty)"
            } else {
                "  no matches"
            };
            let mut s = TreeSeg::new(msg, guide);
            s.italic = true;
            tree_row(row, rect.x, list_y, rect.width, &[s], None)?;
        }
    }
    for y in 0..list_h {
        let Some(row) = frame.get_mut(list_y + y) else {
            continue;
        };
        let idx = tree.top + y;
        let Some(n) = tree.nodes.get(idx) else {
            if !(tree.nodes.is_empty() && y == 0) {
                tree_row(row, rect.x, list_y + y, rect.width, &[], None)?;
            }
            continue;
        };
        let selected = tree.cursor == idx;
        let mut segs: Vec<TreeSeg> = Vec::with_capacity(8);
        let mut sign = if tree.marked.contains(&n.path) {
            TreeSeg::new("✓", Some(ed.theme.special))
        } else if selected && active {
            TreeSeg::new("▌", Some(ed.theme.tree_dir))
        } else {
            TreeSeg::new(" ", None)
        };
        sign.bold = true;
        segs.push(sign);
        if n.depth > 0 {
            let mut g = String::with_capacity(n.depth * 4);
            for l in 0..n.depth {
                g.push_str(if l + 1 == n.depth {
                    if n.last {
                        "└ "
                    } else {
                        "├ "
                    }
                } else if l < 64 && n.rails & (1u64 << l) != 0 {
                    "│ "
                } else {
                    "  "
                });
            }
            segs.push(TreeSeg::new(g, guide));
        }
        let arrow = if !n.is_dir {
            "  "
        } else if icons {
            if n.open {
                "\u{f47c} "
            } else {
                "\u{f460} "
            }
        } else if n.open {
            "▾ "
        } else {
            "▸ "
        };
        segs.push(TreeSeg::new(arrow, guide));
        let name_col = segs.iter().map(TreeSeg::width).sum::<usize>() + if icons { 2 } else { 0 };
        if icons {
            let (glyph, color) = crate::filetree::icon(&n.name, n.is_dir, n.open, n.link.is_some());
            let mut s = TreeSeg::new(format!("{glyph} "), Some(color));
            s.dim = n.ignored;
            segs.push(s);
        }
        let git = tree.git_marker(&n.path, n.is_dir);
        let hit = tree.matched.contains(&n.path);
        let mut name = TreeSeg::new(
            n.name.clone(),
            if n.ignored || cut.contains(n.path.as_path()) {
                Some(ed.theme.muted)
            } else if hit {
                Some(ed.theme.warning)
            } else if n.is_dir {
                dirc
            } else if n.link.is_some() {
                Some(ed.theme.accent)
            } else {
                git.map(|c| ed.theme.git(c))
            },
        );
        name.bold = n.is_dir || hit || open_bufs.contains(n.path.as_path());
        name.underline = current == Some(n.path.as_path());
        name.italic = cut.contains(n.path.as_path());
        segs.push(name);
        if let Some(l) = &n.link {
            segs.push(TreeSeg::new(format!(" → {}", l.display()), guide));
        }

        // Right-aligned badges.
        let mut right: Vec<TreeSeg> = Vec::new();
        if dirty.contains(n.path.as_path()) {
            right.push(TreeSeg::new(" ●", Some(ed.theme.warning)));
        }
        if tree.bookmarks.contains(&n.path) {
            right.push(TreeSeg::new(" \u{2605}", Some(ed.theme.warning)));
        }
        if let Some(c) = git {
            right.push(TreeSeg::new(format!(" {c}"), Some(ed.theme.git(c))));
        }
        let diag = if n.is_dir {
            diag_dirs.get(n.path.as_path()).copied()
        } else {
            tree_diagnostic_marker(ed, &n.path, false).map(|c| match c {
                'E' => 0,
                'W' => 1,
                _ => 2,
            })
        };
        if let Some(r) = diag {
            let (c, col) = match r {
                0 => ('E', ed.theme.error),
                1 => ('W', ed.theme.warning),
                _ => ('I', ed.theme.accent),
            };
            let mut s = TreeSeg::new(format!(" {c}"), Some(col));
            s.bold = true;
            right.push(s);
        }
        if !right.is_empty() {
            right.push(TreeSeg::new(" ", None));
        }
        let rw: usize = right.iter().map(TreeSeg::width).sum();
        let avail = rect.width.saturating_sub(rw);
        let lw: usize = segs.iter().map(TreeSeg::width).sum();
        if lw > avail {
            // Truncate from the end of the left part, marking it with `…`.
            let mut budget = avail.saturating_sub(1);
            for s in segs.iter_mut() {
                let w = s.width();
                if w <= budget {
                    budget -= w;
                } else {
                    s.text = clip(&s.text, budget);
                    budget = 0;
                }
            }
            segs.retain(|s| !s.text.is_empty());
            segs.push(TreeSeg::new("…", guide));
        }
        let lw: usize = segs.iter().map(TreeSeg::width).sum();
        if !right.is_empty() && lw + rw <= rect.width {
            segs.push(TreeSeg::new(" ".repeat(rect.width - lw - rw), None));
            segs.extend(right);
        }
        let bg = selected.then_some(row_bg);
        tree_row(row, rect.x, list_y + y, rect.width, &segs, bg)?;
        if selected && active && cursor.is_none() {
            cursor = Some((
                rect.x + name_col.min(rect.width.saturating_sub(1)),
                list_y + y,
            ));
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
        let limit = rect.width.min(cols as usize);
        for y in 0..rect.height.min(rows as usize) {
            if let Some(row) = frame.get_mut(rect.y + y) {
                queue!(row, MoveTo(rect.x as u16, (rect.y + y) as u16))?;
                for x in 0..limit {
                    let Some(cell) = screen.cell(y as u16, x as u16) else {
                        queue!(row, Print(" "))?;
                        continue;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    if cell.is_wide() && x + 1 >= limit {
                        // A double-width glyph at the last column would spill
                        // past the pane into the divider / neighbouring pane;
                        // blank it instead.
                        queue!(row, Print(" "))?;
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
                    ed.theme.accent
                } else if span.style.code_block || span.style.inline_code {
                    ed.theme.string
                } else if span.style.dim {
                    ed.theme.muted
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
        ed.theme.bar_bg,
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
    // Match draw_pane exactly (minimap strip, winbar row, zen/global statusline)
    // so a click maps to the glyph actually under the pointer.
    let PaneDims {
        gw,
        width: pane_width,
        top_off,
        rows: content_rows,
        ..
    } = pane_dims(ed, b, *rect);
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
    fn paint_base_repoints_default_colors_at_the_theme() {
        let bg = sgr(SetBackgroundColor(Color::Rgb { r: 1, g: 2, b: 3 }));
        let fg = sgr(SetForegroundColor(Color::Rgb { r: 4, g: 5, b: 6 }));
        // The sequences crossterm writes for the defaults paint_base matches.
        assert_eq!(sgr(ResetColor), b"\x1b[0m");
        assert_eq!(sgr(SetAttribute(Attribute::Reset)), b"\x1b[0m");
        assert_eq!(sgr(SetBackgroundColor(Color::Reset)), b"\x1b[49m");
        assert_eq!(sgr(SetForegroundColor(Color::Reset)), b"\x1b[39m");
        let mut row = Vec::new();
        queue!(
            row,
            SetBackgroundColor(Color::Reset),
            SetForegroundColor(Color::Reset),
            Print("a"),
            SetBackgroundColor(Color::DarkBlue),
            Print("b"),
            ResetColor,
            Print("c")
        )
        .unwrap();
        let mut out = Vec::new();
        paint_base(&row, &bg, &fg, &mut out);
        let mut want = Vec::new();
        want.extend_from_slice(&bg);
        want.extend_from_slice(&fg);
        want.extend_from_slice(b"a");
        want.extend_from_slice(&sgr(SetBackgroundColor(Color::DarkBlue)));
        want.extend_from_slice(b"b\x1b[0m");
        want.extend_from_slice(&bg);
        want.extend_from_slice(&fg);
        want.extend_from_slice(b"c");
        assert_eq!(out, want);
        // Text that merely looks like an escape is copied verbatim.
        let mut out = Vec::new();
        paint_base(b"[0m 49m \x1b[1m", &bg, &fg, &mut out);
        assert_eq!(out, b"[0m 49m \x1b[1m");
    }

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
    fn tour_markdown_lite_strips_noise() {
        assert_eq!(tour_markdown_lite("**bold** and `code`"), "bold and code");
        assert_eq!(tour_markdown_lite("# Heading\nbody"), "Heading\nbody");
    }
    #[test]
    fn tour_progress_dots_unchanged_at_or_under_the_cap() {
        assert_eq!(tour_progress_dots(0, 3), "●○○");
        assert_eq!(tour_progress_dots(2, 3), "●●●");
        assert_eq!(tour_progress_dots(19, 20), "●".repeat(20));
    }
    #[test]
    fn tour_progress_dots_slides_a_window_past_the_cap() {
        let total = 30;
        // Always exactly DOT_CAP (20) dots, never a stale all-filled bar.
        for idx in [0, 5, 15, 19, 20, 25, 29] {
            let dots = tour_progress_dots(idx, total);
            assert_eq!(dots.chars().count(), 20, "idx={idx}");
            let filled = dots.chars().filter(|&c| c == '●').count();
            assert!((1..=20).contains(&filled), "idx={idx} filled={filled}");
            // Not every dot filled unless we're actually at/near the end.
            if idx < total - 1 {
                assert!(
                    dots.contains('○'),
                    "idx={idx} should still show remaining steps: {dots}"
                );
            }
        }
        // At the very last step, the whole (windowed) bar reads as filled.
        assert_eq!(tour_progress_dots(29, total), "●".repeat(20));
        // At the very first step, the window starts at 0 and shows mostly empty.
        assert_eq!(tour_progress_dots(0, total), format!("●{}", "○".repeat(19)));
    }
    #[test]
    fn wrap_text_wraps_words_and_hard_breaks_long_ones() {
        assert_eq!(wrap_text("one two three", 7), vec!["one two", "three"]);
        // A word longer than the width is hard-broken.
        assert_eq!(wrap_text("abcdefgh", 3), vec!["abc", "def", "gh"]);
        // Existing newlines are preserved as paragraph breaks.
        assert_eq!(wrap_text("a\nb", 10), vec!["a", "b"]);
    }
    #[test]
    fn pane_dims_reserve_winbar_row_and_minimap_strip() {
        // The geometry `prepare_view`/the scroll path use must match what
        // `draw_pane` renders: a winbar row and a minimap strip both shrink the
        // content region. (Regression: scroll math ignored both, so the cursor
        // could scroll off-screen.)
        let cfg = crate::config::Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            ..crate::config::Config::default()
        };
        let mut ed = Editor::new(cfg);
        ed.buf_mut().rope = ropey::Rope::from_str("hello\nworld\n");
        let r = crate::windows::Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 30,
        };
        let base = pane_dims(&ed, ed.buf(), r);
        assert_eq!(base.top_off, 0);
        assert_eq!(base.map_w, 0);

        ed.config.winbar = true;
        let wb = pane_dims(&ed, ed.buf(), r);
        assert_eq!(wb.top_off, 1);
        assert_eq!(wb.rows, base.rows - 1, "winbar reserves one content row");
        ed.config.winbar = false;

        ed.config.minimap = true;
        let mm = pane_dims(&ed, ed.buf(), r);
        assert_eq!(mm.map_w, MINIMAP_W);
        assert_eq!(
            mm.width,
            base.width - MINIMAP_W,
            "minimap narrows the content column"
        );
    }
    #[test]
    fn parse_statuscolumn_orders_gutter_components() {
        use GutterComp::*;
        assert_eq!(parse_statuscolumn("", false), vec![Diag, Git, Num]);
        assert_eq!(parse_statuscolumn("", true), vec![Fold, Diag, Git, Num]);
        assert_eq!(
            parse_statuscolumn("num git diag", false),
            vec![Num, Git, Diag]
        );
        // `fold` is dropped when the foldcolumn is off (it owns no cell).
        assert_eq!(parse_statuscolumn("fold num", false), vec![Num]);
        assert_eq!(parse_statuscolumn("fold num", true), vec![Fold, Num]);
    }
    #[test]
    fn parse_fillchars_reads_eob_and_vert() {
        assert_eq!(parse_fillchars("eob: ,vert:┃"), (' ', '┃'));
        assert_eq!(parse_fillchars("vert:|"), ('~', '|'));
        assert_eq!(parse_fillchars(""), ('~', '│'));
    }
    #[test]
    fn contrast_on_picks_readable_text_for_a_swatch() {
        // Light chip -> black text; dark chip -> white text.
        assert_eq!(
            contrast_on(Color::Rgb {
                r: 255,
                g: 255,
                b: 0
            }),
            Color::Black,
            "yellow is bright, use black text"
        );
        assert_eq!(
            contrast_on(Color::Rgb { r: 0, g: 0, b: 255 }),
            Color::White,
            "blue is dark, use white text"
        );
        assert_eq!(contrast_on(Color::Rgb { r: 0, g: 0, b: 0 }), Color::White);
        // A non-RGB color (shouldn't happen for a swatch) defaults to white.
        assert_eq!(contrast_on(Color::Reset), Color::White);
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
