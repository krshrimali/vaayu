//! Minimap: a strip on the right of each buffer pane showing a compressed
//! picture of the whole file, modelled on VS Code's `editor.minimap`.
//!
//! Strip layout, left to right:
//!
//! ```text
//! │ ⣿⣿⣶⣤⡀      ●
//! ^ ^          ^
//! | body       overview lane: diagnostics, search hits, cursor
//! git lane: added/modified/removed, else a plain separator
//! ```
//!
//! The body draws the file as Braille dots (4 lines and 2 source-column
//! groups per cell, coloured by syntax) or, with
//! `minimap_render_characters = false`, as half blocks (2 lines per cell).
//! `MARK:` comments and `#region` markers become labels. A tinted slider
//! covers the visible lines. You can click, drag the slider, or use the wheel.
//!
//! `minimap_size` follows VS Code: `proportional` keeps a fixed density and
//! scrolls the minimap with the editor when the file is taller than the
//! strip, `fit` shrinks a tall file so all of it fits, and `fill` stretches or
//! shrinks the file to fill the strip's full height.
//!
//! [`MinimapGeom`] is the single row↔line mapping shared by drawing and the
//! mouse, so a click always lands on the line that was drawn there.

use crate::buffer::Buffer;
use crate::editor::Editor;
use crossterm::{
    cursor::MoveTo,
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
};
use std::cell::RefCell;
use std::io;
use unicode_width::UnicodeWidthChar;

/// Narrowest usable strip: git lane, two body cells, overview lane.
pub const MIN_WIDTH: usize = 4;
pub const MAX_WIDTH: usize = 60;

/// How file lines map onto minimap rows (VS Code `editor.minimap.size`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Proportional,
    Fit,
    Fill,
}

impl Size {
    pub fn parse(s: &str) -> Option<Size> {
        match s.trim() {
            "proportional" | "" => Some(Size::Proportional),
            "fit" => Some(Size::Fit),
            "fill" => Some(Size::Fill),
            _ => None,
        }
    }
}

/// The strip width for a pane whose text area (after the gutter) is
/// `avail` cells, or 0 when the minimap is off or the pane is too narrow to
/// keep at least as much text as it gives the minimap.
pub fn strip_width(ed: &Editor, avail: usize) -> usize {
    if !ed.config.minimap {
        return 0;
    }
    let w = ed.config.minimap_width.clamp(MIN_WIDTH, MAX_WIDTH);
    if avail > w * 2 {
        w
    } else {
        0
    }
}

/// Source lines one minimap row holds at its base density.
fn base_lines_per_row(ed: &Editor) -> usize {
    if ed.config.minimap_render_characters {
        4
    } else {
        2
    }
}

/// Row↔line mapping for one pane's minimap. Row `r` covers lines
/// `[(r + offset) * per, (r + offset + 1) * per)`. `per` may be below 1 in
/// `fill` mode, where one line then spans several rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapGeom {
    pub per: f64,
    pub offset: usize,
    pub total: usize,
    pub rows: usize,
    /// True when a proportional minimap is taller than the strip and
    /// scrolls with the editor. Slider dragging then works on the scroll
    /// ratio instead of mapping rows straight to lines.
    pub scrolls: bool,
}

impl MinimapGeom {
    pub fn new(ed: &Editor, total: usize, top_line: usize, visible: usize, rows: usize) -> Self {
        let total = total.max(1);
        let rows = rows.max(1);
        let base = base_lines_per_row(ed) as f64;
        let fitted = total as f64 / rows as f64;
        let size = Size::parse(&ed.config.minimap_size).unwrap_or(Size::Proportional);
        let (per, offset, scrolls) = match size {
            Size::Fill => (fitted, 0, false),
            Size::Fit => (base.max(fitted), 0, false),
            Size::Proportional => {
                let height = total.div_ceil(base as usize);
                if height <= rows {
                    (base, 0, false)
                } else {
                    // The VS Code rule: the minimap scrolls by the same
                    // fraction of its overflow as the editor has scrolled
                    // of its own.
                    let max_scroll = total.saturating_sub(visible).max(1);
                    let overflow = height - rows;
                    let off = (top_line.min(max_scroll) as f64 * overflow as f64
                        / max_scroll as f64)
                        .round() as usize;
                    (base, off.min(overflow), true)
                }
            }
        };
        MinimapGeom {
            per,
            offset,
            total,
            rows,
            scrolls,
        }
    }

    /// The `[lo, hi)` line range of minimap row `row`, or `None` past EOF.
    pub fn row_lines(&self, row: usize) -> Option<(usize, usize)> {
        let lo = ((row + self.offset) as f64 * self.per).floor() as usize;
        if lo >= self.total {
            return None;
        }
        let hi = (((row + self.offset + 1) as f64 * self.per).floor() as usize)
            .max(lo + 1)
            .min(self.total);
        Some((lo, hi))
    }

    /// The row that shows `line` (its first row when one line spans
    /// several), or `None` when that part of the minimap is scrolled out
    /// of view.
    pub fn line_row(&self, line: usize) -> Option<usize> {
        let abs = if self.per >= 1.0 {
            (line as f64 / self.per).floor() as usize
        } else {
            (line as f64 / self.per).ceil() as usize
        };
        let row = abs.checked_sub(self.offset)?;
        (row < self.rows).then_some(row)
    }

    /// Like `line_row`, but clamps lines above or below the scrolled
    /// window to its first or last row. Used for multi-line ranges.
    fn line_row_clamped(&self, line: usize) -> usize {
        let abs = if self.per >= 1.0 {
            (line as f64 / self.per).floor() as usize
        } else {
            (line as f64 / self.per).ceil() as usize
        };
        abs.saturating_sub(self.offset).min(self.rows - 1)
    }

    /// The rows the slider covers for visible lines `[first, last]`, as
    /// `(top_row, height)`. The height is always at least 1.
    pub fn slider(&self, first: usize, last: usize) -> (usize, usize) {
        let mut top = None;
        let mut bottom = 0;
        for row in 0..self.rows {
            let Some((lo, hi)) = self.row_lines(row) else {
                break;
            };
            if hi > first && lo <= last {
                top.get_or_insert(row);
                bottom = row;
            }
        }
        match top {
            Some(t) => (t, bottom - t + 1),
            None => (self.line_row_clamped(first), 1),
        }
    }
}

/// What one sampled source line contributes to the body: per "dot column"
/// (source-column group), the colour of the first visible character in
/// it, or `None` where the line has nothing there.
type DotLine = Vec<Option<Color>>;

/// Sample `src` into `cols` dot columns spanning display columns
/// `0..max_column`. Tabs expand to `tabstop`, and wide characters take
/// two columns. `class_at(byte)` supplies a syntax colour.
fn dot_line(
    src: &str,
    cols: usize,
    max_column: usize,
    tabstop: usize,
    mut class_at: impl FnMut(usize) -> Option<Color>,
) -> DotLine {
    let mut out = vec![None; cols];
    if cols == 0 {
        return out;
    }
    let max_column = max_column.max(1);
    let mut x = 0usize;
    for (byte, c) in src.char_indices() {
        if x >= max_column {
            break;
        }
        let w = match c {
            '\t' => tabstop.max(1) - x % tabstop.max(1),
            '\n' | '\r' => 0,
            c => c.width().unwrap_or(0),
        };
        if !c.is_whitespace() && w > 0 {
            let dot = x * cols / max_column;
            if let Some(slot) = out.get_mut(dot) {
                if slot.is_none() {
                    *slot = Some(class_at(byte).unwrap_or(Color::Reset));
                }
            }
        }
        x += w;
    }
    out
}

/// Braille dot bit for (dot row 0..4, dot column 0..2).
const BRAILLE_BITS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Turn the sampled lines of one minimap row into body cells: a glyph and
/// the foreground colour of the first ink in that cell (`None` = default).
fn compose_cells(lines: &[DotLine], body_w: usize, braille: bool) -> Vec<(char, Option<Color>)> {
    let mut out = Vec::with_capacity(body_w);
    for cell in 0..body_w {
        let mut bits = 0u32;
        let mut color = None;
        if braille {
            for (dy, line) in lines.iter().enumerate().take(4) {
                for (dx, bit) in BRAILLE_BITS[dy].iter().enumerate() {
                    if let Some(Some(c)) = line.get(cell * 2 + dx) {
                        bits |= bit;
                        color.get_or_insert(*c);
                    }
                }
            }
            let glyph = if bits == 0 {
                ' '
            } else {
                char::from_u32(0x2800 + bits).unwrap_or(' ')
            };
            out.push((glyph, color.filter(|c| *c != Color::Reset)));
        } else {
            let top = lines.first().and_then(|l| l.get(cell).copied().flatten());
            let bot = lines.get(1).and_then(|l| l.get(cell).copied().flatten());
            let glyph = match (top.is_some(), bot.is_some()) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            out.push((glyph, top.or(bot).filter(|c| *c != Color::Reset)));
        }
    }
    out
}

/// Per-row decorations: everything the lanes and body overlays need.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct RowMarks {
    /// 0 none, 1 removed, 2 added, 3 modified (higher wins).
    git: u8,
    /// 0 none, 1 info, 2 warning, 3 error.
    diag: u8,
    search: bool,
    cursor: bool,
    selected: bool,
}

type SearchKey = (u64, u64, String, bool, bool);
type SearchCache = Option<(SearchKey, Vec<usize>)>;
type SectionCache = Option<((u64, u64), Vec<(usize, String)>)>;

thread_local! {
    /// (buffer id, edit seq, pattern, ignorecase, smartcase) -> lines with a
    /// search match. Rescanning the whole file every frame would be wasteful.
    static SEARCH_LINES: RefCell<SearchCache> =
        const { RefCell::new(None) };
    /// (buffer id, edit seq) -> section headers.
    static SECTIONS: RefCell<SectionCache> =
        const { RefCell::new(None) };
}

/// Cap on whole-file search scans for the minimap. Past this, matches are
/// marked only for lines the minimap samples anyway.
const SEARCH_SCAN_LIMIT: usize = 200_000;

fn search_lines(ed: &Editor, b: &Buffer) -> Vec<usize> {
    let Some(pat) = ed.incsearch.clone().or_else(|| {
        ed.last_search
            .as_ref()
            .filter(|_| ed.hl_search)
            .map(|(p, _)| p.clone())
    }) else {
        return Vec::new();
    };
    if pat.is_empty() || b.line_count() > SEARCH_SCAN_LIMIT {
        return Vec::new();
    }
    let key = (
        b.id,
        b.edit_seq,
        pat.clone(),
        ed.config.ignorecase,
        ed.config.smartcase,
    );
    if let Some(hit) = SEARCH_LINES.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    }) {
        return hit;
    }
    let Ok(re) = crate::search::compile(&pat, ed.config.ignorecase, ed.config.smartcase) else {
        return Vec::new();
    };
    let lines: Vec<usize> = b
        .rope
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            let s = l.to_string();
            re.is_match(s.trim_end_matches(['\n', '\r']))
                .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();
    SEARCH_LINES.with(|c| *c.borrow_mut() = Some((key, lines.clone())));
    lines
}

/// A section-header label on `line`, if any: VS Code's `MARK:` comments
/// (`// MARK: Setup`, `# MARK: - Parsing`) and region markers
/// (`#region Name`, `// #region Name`, `// region Name`).
pub fn section_label(line: &str) -> Option<String> {
    let t = line.trim_start();
    let body = ["//", "#", "--", "/*", ";;", "%"]
        .iter()
        .find_map(|p| t.strip_prefix(p))
        .unwrap_or(t)
        .trim_start_matches(['/', '#', '*', '!', '-', ';'])
        .trim_start();
    let rest = if let Some(r) = body.strip_prefix("MARK:") {
        r.trim_start().trim_start_matches('-').trim()
    } else if t.starts_with("#region") || t.starts_with("#pragma region") {
        t.trim_start_matches("#pragma")
            .trim_start()
            .trim_start_matches("#region")
            .trim()
    } else {
        body.strip_prefix("#region")
            .or_else(|| body.strip_prefix("region "))?
            .trim()
    };
    let rest = rest.trim_end_matches("*/").trim_end();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn sections(b: &Buffer) -> Vec<(usize, String)> {
    let key = (b.id, b.edit_seq);
    if let Some(hit) = SECTIONS.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    }) {
        return hit;
    }
    let mut out = Vec::new();
    for (i, l) in b.rope.lines().enumerate() {
        // Cheap pre-filter before allocating the line.
        let hit = l
            .chunks()
            .any(|ch| ch.contains("MARK:") || ch.contains("region"));
        if hit {
            if let Some(label) = section_label(&l.to_string()) {
                out.push((i, label));
            }
        }
    }
    SECTIONS.with(|c| *c.borrow_mut() = Some((key, out.clone())));
    out
}

/// Inputs for drawing one pane's minimap.
pub struct Strip<'a> {
    pub buf: &'a Buffer,
    /// The pane's own cursor (line, col).
    pub cursor: (usize, usize),
    /// Logical lines currently on screen, inclusive.
    pub visible: (usize, usize),
    /// Absolute terminal column of the strip's left edge.
    pub x: usize,
    /// Absolute terminal row of the first minimap row.
    pub y: usize,
    pub width: usize,
    pub rows: usize,
    /// Whether to draw the slider (always, or only on hover / drag).
    pub show_slider: bool,
    /// Whether this pane's buffer is the focused one (git signs, syntax
    /// and the visual selection only exist for it).
    pub focused: bool,
}

pub fn draw(frame: &mut [Vec<u8>], ed: &Editor, s: &Strip) -> io::Result<()> {
    let b = s.buf;
    let total = b.line_count();
    let vis_count = s.visible.1.saturating_sub(s.visible.0) + 1;
    let geom = MinimapGeom::new(ed, total, s.visible.0, vis_count, s.rows);
    let braille = ed.config.minimap_render_characters;
    let body_w = s.width.saturating_sub(2);
    let dot_cols = if braille { body_w * 2 } else { body_w };
    let max_column = ed.config.minimap_max_column.max(1);
    let theme = &ed.theme;

    // ---- per-row decorations, bucketed once ----
    let mut marks = vec![RowMarks::default(); s.rows];
    let mut mark_range = |lo: usize, hi: usize, f: &mut dyn FnMut(&mut RowMarks)| {
        if hi < lo || geom.rows == 0 {
            return;
        }
        // Skip ranges wholly scrolled out of view.
        let abs = |l: usize| (l as f64 / geom.per).floor() as usize;
        if abs(hi) < geom.offset || abs(lo) >= geom.offset + geom.rows {
            return;
        }
        let a = geom.line_row_clamped(lo);
        let z = geom.line_row_clamped(hi);
        for m in &mut marks[a..=z] {
            f(m);
        }
    };
    if s.focused {
        if let Some(g) = ed.git.as_ref() {
            for (&line, sign) in &g.signs {
                let v = match sign {
                    crate::gitdiff::Sign::Removed => 1,
                    crate::gitdiff::Sign::Added => 2,
                    crate::gitdiff::Sign::Modified => 3,
                };
                mark_range(line, line, &mut |m| m.git = m.git.max(v));
            }
        }
    }
    if let Some(ds) = b.path.as_ref().and_then(|p| ed.diagnostics.get(p)) {
        for d in ds {
            let v = match d.severity {
                crate::lsp::Severity::Error => 3,
                crate::lsp::Severity::Warning => 2,
                crate::lsp::Severity::Info => 1,
                crate::lsp::Severity::Hint => continue,
            };
            mark_range(d.line, d.end_line.max(d.line), &mut |m| {
                m.diag = m.diag.max(v)
            });
        }
    }
    for line in search_lines(ed, b) {
        mark_range(line, line, &mut |m| m.search = true);
    }
    mark_range(s.cursor.0, s.cursor.0, &mut |m| m.cursor = true);
    if s.focused {
        if let (crate::mode::Mode::Visual(_), Some(anchor)) = (&ed.mode, ed.visual_anchor) {
            let (a, z) = if anchor.0 <= s.cursor.0 {
                (anchor.0, s.cursor.0)
            } else {
                (s.cursor.0, anchor.0)
            };
            mark_range(a, z, &mut |m| m.selected = true);
        }
    }
    let section_rows: Vec<(usize, String)> = if ed.config.minimap_section_headers {
        sections(b)
            .into_iter()
            .filter_map(|(line, label)| geom.line_row(line).map(|r| (r, label)))
            .collect()
    } else {
        Vec::new()
    };

    let syntax = s.focused.then_some(ed.syntax.as_ref()).flatten();
    let slider = s.show_slider.then(|| geom.slider(s.visible.0, s.visible.1));

    for (row, &m) in marks.iter().enumerate() {
        let y = s.y + row;
        let Some(dest) = frame.get_mut(y) else {
            continue;
        };
        queue!(dest, MoveTo(s.x as u16, y as u16))?;
        // Git lane (doubles as the separator from the text).
        let (gch, gcol) = match m.git {
            3 => ('▐', theme.git_change),
            2 => ('▐', theme.git_add),
            1 => ('▁', theme.git_delete),
            _ => ('│', theme.muted),
        };
        queue!(dest, SetForegroundColor(gcol), Print(gch), ResetColor)?;

        let Some((lo, hi)) = geom.row_lines(row) else {
            queue!(dest, Print(" ".repeat(s.width.saturating_sub(1))))?;
            continue;
        };
        let in_slider = slider.is_some_and(|(t, h)| row >= t && row < t + h);
        let bg = if m.selected {
            Some(theme.selection_bg)
        } else if in_slider {
            Some(theme.minimap_view_bg)
        } else {
            None
        };
        if let Some(bg) = bg {
            queue!(dest, SetBackgroundColor(bg))?;
        }

        if let Some((_, label)) = section_rows.iter().find(|(r, _)| *r == row) {
            // A section header replaces the row's dots with its label.
            let text: String = label.chars().take(body_w).collect();
            let pad = body_w.saturating_sub(text.chars().map(|c| c.width().unwrap_or(1)).sum());
            queue!(
                dest,
                SetForegroundColor(theme.accent),
                SetAttribute(Attribute::Bold),
                Print(&text),
                SetAttribute(Attribute::NormalIntensity),
                Print(" ".repeat(pad))
            )?;
        } else {
            // Sample one line per dot row (4 for Braille, 2 for blocks).
            let sub = if braille { 4 } else { 2 };
            let span = (hi - lo) as f64;
            let mut sampled: Vec<DotLine> = Vec::with_capacity(sub);
            let mut last = None;
            for k in 0..sub {
                let line = lo + (k as f64 * span / sub as f64).floor() as usize;
                // In `fill` mode one line may span rows and sub-rows; draw
                // it only once per row rather than as a solid block.
                if line >= hi || last == Some(line) {
                    sampled.push(vec![None; dot_cols]);
                    continue;
                }
                last = Some(line);
                let text = b.line_text(line);
                let (start, end) = b.line_byte_range(line);
                let spans: Vec<(usize, usize, Color)> = syntax
                    .map(|syn| {
                        syn.spans_in(start, end)
                            .map(|(a, z, c)| (a - start, z - start, theme.syntax(c)))
                            .collect()
                    })
                    .unwrap_or_default();
                let class_at = |byte: usize| {
                    spans
                        .iter()
                        .find(|(a, z, _)| byte >= *a && byte < *z)
                        .map(|(_, _, c)| *c)
                };
                sampled.push(dot_line(&text, dot_cols, max_column, b.tabstop, class_at));
            }
            let default_fg = if m.cursor {
                theme.accent
            } else {
                theme.minimap_fg
            };
            for (glyph, color) in compose_cells(&sampled, body_w, braille) {
                queue!(
                    dest,
                    SetForegroundColor(color.unwrap_or(default_fg)),
                    Print(glyph)
                )?;
            }
        }

        // Overview lane: diagnostics, then search hits, then the cursor.
        let lane = match (m.diag, m.search, m.cursor) {
            (3, ..) => Some(('●', theme.error)),
            (2, ..) => Some(('●', theme.warning)),
            (1, ..) => Some(('•', theme.info)),
            (_, true, _) => Some(('■', theme.search_bg)),
            (_, _, true) => Some(('◂', theme.accent)),
            _ => None,
        };
        match lane {
            Some((ch, c)) => queue!(dest, SetForegroundColor(c), Print(ch))?,
            None => queue!(dest, Print(' '))?,
        }
        queue!(dest, ResetColor, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

/// Where a minimap drag started: which pane, how far below the slider's
/// top the pointer grabbed it, and the slider height then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub pane: usize,
    pub grab: usize,
    pub slider_h: usize,
}

/// A pointer position resolved against a pane's minimap.
pub struct Hit {
    pub pane: usize,
    /// Minimap row under the pointer (may be past the drawn rows while
    /// dragging, and is clamped then).
    pub row: usize,
    pub geom: MinimapGeom,
    pub slider: (usize, usize),
    pub visible: usize,
}

/// Resolve terminal cell (x, y) to a minimap. With `pane` set (dragging),
/// only that pane's minimap is considered and x/y are clamped to it.
pub fn hit(
    ed: &Editor,
    cols: usize,
    rows: usize,
    x: usize,
    y: usize,
    pane: Option<usize>,
) -> Option<Hit> {
    let rects = ed.pane_rects(cols, rows);
    let idx = match pane {
        Some(p) => p,
        None => rects
            .iter()
            .position(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)?,
    };
    let r = *rects.get(idx)?;
    let w = if ed.windows.is_empty() {
        ed.capture_window()
    } else {
        let w = ed.windows.get(idx)?.clone();
        if w.file_tree || w.outline || w.preview || w.terminal.is_some() {
            return None;
        }
        // The focused pane's live state is on the buffer, not the window.
        if idx == ed.active_window {
            ed.capture_window()
        } else {
            w
        }
    };
    let b = ed.buffers.iter().find(|b| b.id == w.buffer)?;
    let info = crate::render::minimap_rect(ed, b, &w, r)?;
    if pane.is_none()
        && (x < info.x || x >= info.x + info.width || y < info.y || y >= info.y + info.rows)
    {
        return None;
    }
    let row = y.saturating_sub(info.y).min(info.rows.saturating_sub(1));
    let visible = info.visible.1.saturating_sub(info.visible.0) + 1;
    let geom = MinimapGeom::new(ed, b.line_count(), info.visible.0, visible, info.rows);
    let slider = geom.slider(info.visible.0, info.visible.1);
    Some(Hit {
        pane: idx,
        row,
        geom,
        slider,
        visible,
    })
}

/// The top line a drag to `row` asks for, given where the slider was
/// grabbed.
pub fn drag_target(h: &Hit, d: &Drag) -> usize {
    let max_top = h.geom.total.saturating_sub(h.visible);
    let row = h.row.saturating_sub(d.grab);
    if h.geom.scrolls {
        // The minimap itself moves while scrolling, so map the slider's
        // travel to the editor's scroll range by ratio (as VS Code does).
        let travel = h.geom.rows.saturating_sub(d.slider_h).max(1);
        ((row.min(travel) as f64 / travel as f64) * max_top as f64).round() as usize
    } else {
        h.geom
            .row_lines(row)
            .map(|(lo, _)| lo)
            .unwrap_or(max_top)
            .min(max_top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed() -> Editor {
        let cfg = crate::config::Config {
            clipboard_unnamedplus: false,
            jk_escape: false,
            minimap: true,
            ..crate::config::Config::default()
        };
        Editor::new(cfg)
    }

    #[test]
    fn proportional_geometry_scrolls_with_the_editor_when_the_file_is_tall() {
        let e = ed();
        // 40 lines at 4 per row = 10 rows: fits in 20, so no scrolling.
        let g = MinimapGeom::new(&e, 40, 0, 10, 20);
        assert!(!g.scrolls);
        assert_eq!(g.row_lines(0), Some((0, 4)));
        assert_eq!(g.row_lines(9), Some((36, 40)));
        assert_eq!(g.row_lines(10), None);
        assert_eq!(g.line_row(37), Some(9));
        // 1000 lines = 250 rows in a 50-row strip. Scrolled to the end, the
        // minimap shows its last 50 rows.
        let g = MinimapGeom::new(&e, 1000, 980, 20, 50);
        assert!(g.scrolls);
        assert_eq!(g.offset, 200);
        assert_eq!(g.row_lines(49), Some((996, 1000)));
        assert_eq!(g.line_row(0), None, "scrolled out of view");
        let g0 = MinimapGeom::new(&e, 1000, 0, 20, 50);
        assert_eq!(g0.offset, 0);
    }

    #[test]
    fn fit_and_fill_never_scroll() {
        let mut e = ed();
        e.config.minimap_size = "fit".into();
        let g = MinimapGeom::new(&e, 1000, 500, 20, 50);
        assert!(!g.scrolls);
        assert_eq!(g.per, 20.0);
        assert_eq!(g.row_lines(49), Some((980, 1000)));
        // A short file keeps its base density under `fit`...
        let g = MinimapGeom::new(&e, 10, 0, 10, 50);
        assert_eq!(g.per, 4.0);
        // ...but `fill` stretches it over the whole strip.
        e.config.minimap_size = "fill".into();
        let g = MinimapGeom::new(&e, 10, 0, 10, 50);
        assert_eq!(g.per, 0.2);
        assert_eq!(g.row_lines(0), Some((0, 1)));
        assert_eq!(g.line_row(1), Some(5));
        assert_eq!(g.row_lines(5), Some((1, 2)));
    }

    #[test]
    fn slider_covers_the_visible_lines() {
        let e = ed();
        let g = MinimapGeom::new(&e, 400, 40, 20, 100);
        assert_eq!(g.slider(40, 59), (10, 5));
    }

    #[test]
    fn braille_cells_encode_indentation_and_length() {
        // Two lines, one dot column per 2 source columns over 8 columns.
        let a = dot_line("ab", 4, 8, 4, |_| None);
        let b = dot_line("    cd", 4, 8, 4, |_| None);
        assert_eq!(a, vec![Some(Color::Reset), None, None, None]);
        assert_eq!(b, vec![None, None, Some(Color::Reset), None]);
        let cells = compose_cells(&[a, b], 2, true);
        // Cell 0: dot (row0,col0) = 0x01; cell 1: dot (row1,col0) = 0x02.
        assert_eq!(cells[0].0, '\u{2801}');
        assert_eq!(cells[1].0, '\u{2802}');
        // Tabs expand to the tabstop.
        let t = dot_line("\tx", 4, 8, 4, |_| None);
        assert_eq!(t, vec![None, None, Some(Color::Reset), None]);
        // Block mode: top line only = upper half, both = full.
        let cells = compose_cells(&[vec![Some(Color::Reset)], vec![None]], 1, false);
        assert_eq!(cells[0].0, '▀');
        let cells = compose_cells(
            &[vec![Some(Color::Red)], vec![Some(Color::Reset)]],
            1,
            false,
        );
        assert_eq!(cells[0], ('█', Some(Color::Red)));
    }

    #[test]
    fn section_labels_follow_vscode_markers() {
        assert_eq!(section_label("// MARK: Setup").as_deref(), Some("Setup"));
        assert_eq!(
            section_label("    // MARK: - Parsing").as_deref(),
            Some("Parsing")
        );
        assert_eq!(section_label("# MARK: helpers").as_deref(), Some("helpers"));
        assert_eq!(section_label("#region Imports").as_deref(), Some("Imports"));
        assert_eq!(section_label("// #region Tests").as_deref(), Some("Tests"));
        assert_eq!(section_label("/* MARK: Box */").as_deref(), Some("Box"));
        assert_eq!(section_label("let region = 3;"), None);
        assert_eq!(section_label("// MARK:"), None);
        assert_eq!(section_label("// just a comment"), None);
    }

    #[test]
    fn drag_maps_rows_to_top_lines() {
        let e = ed();
        let geom = MinimapGeom::new(&e, 40, 0, 10, 20);
        let h = Hit {
            pane: 0,
            row: 5,
            geom,
            slider: (0, 3),
            visible: 10,
        };
        let d = Drag {
            pane: 0,
            grab: 1,
            slider_h: 3,
        };
        // Row 4 starts at line 16.
        assert_eq!(drag_target(&h, &d), 16);
        // Clamped so the last page stays full.
        let far = Hit { row: 19, ..h };
        assert_eq!(drag_target(&far, &d), 30);
    }
}
