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
    glyphs: std::rc::Rc<Vec<Glyph>>,
    start: usize,
}

type Parts = std::rc::Rc<Vec<(usize, std::rc::Rc<Vec<Glyph>>)>>;
struct CachedLine {
    revision: u64,
    text: String,
    parts: Parts,
}
#[derive(Default)]
pub struct LayoutCache {
    entries: std::collections::HashMap<(u64, usize, usize, bool, usize, usize), CachedLine>,
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
    ) -> Parts {
        let key = (b.id, line, width, wrap, left, tab);
        if let Some(c) = self.entries.get(&key) {
            if c.revision == b.edit_seq {
                return c.parts.clone();
            }
        }
        let text = b.line_text(line);
        if let Some(c) = self.entries.get_mut(&key) {
            if c.text == text {
                c.revision = b.edit_seq;
                return c.parts.clone();
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
        self.entries.insert(
            key,
            CachedLine {
                revision: b.edit_seq,
                text,
                parts: parts.clone(),
            },
        );
        parts
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
) -> (Vec<DisplayRow>, Option<(usize, usize)>) {
    let mut display = Vec::new();
    let mut cursor = None;
    let end_line = b.line_count().max(if matches!(ed.mode, Mode::Insert) {
        b.rope.len_lines()
    } else {
        0
    });
    for line in w.top..end_line {
        let parts = ed.layout_cache.borrow_mut().line(
            b,
            line,
            width,
            ed.config.wrap,
            w.left,
            ed.config.tabstop,
        );
        let cursor_cells = if line == w.cursor.0 {
            if ed.config.wrap {
                parts
                    .iter()
                    .flat_map(|(_, gs)| gs.iter())
                    .filter(|g| g.col < w.cursor.1)
                    .map(|g| g.width)
                    .sum::<usize>()
            } else {
                glyphs(&b.line_text(line), ed.config.tabstop)
                    .iter()
                    .filter(|g| g.col < w.cursor.1)
                    .map(|g| g.width)
                    .sum::<usize>()
            }
        } else {
            0
        };
        let mut cursor_part = 0;
        for (i, (start, _)) in parts.iter().enumerate() {
            if *start <= cursor_cells {
                cursor_part = i;
            }
        }
        for (i, (start, gs)) in parts.iter().enumerate() {
            if line == w.top && i < w.wrap_row {
                continue;
            }
            if line == w.cursor.0 && i == cursor_part {
                cursor = Some((
                    display.len(),
                    cursor_cells
                        .saturating_sub(*start)
                        .min(width.saturating_sub(1)),
                ));
            }
            display.push(DisplayRow {
                line,
                glyphs: gs.clone(),
                start: *start,
            });
            if display.len() >= rows {
                return (display, cursor);
            }
        }
    }
    (display, cursor)
}
pub fn prepare_view(ed: &mut Editor, cols: usize, rows: usize) {
    ed.screen_cols = cols;
    ed.store_window();
    let rects = ed.pane_rects(cols, rows);
    let rect = rects[ed.active_window.min(rects.len() - 1)];
    let count = rect.height.saturating_sub(1).max(1);
    ed.screen_rows = count;
    let width = rect
        .width
        .saturating_sub(gutter(ed, ed.buf(), rect.width))
        .max(1);
    let mut w = ed.capture_window();
    if !ed.config.wrap {
        let cells = glyphs(&ed.buf().line_text(w.cursor.0), ed.config.tabstop)
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
            let cells = glyphs(&text, ed.config.tabstop)
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
#[derive(PartialEq, Eq)]
struct RowSignature {
    buffer: u64,
    revision: u64,
    syntax: u64,
    line: usize,
    start: usize,
    width: usize,
    gutter: usize,
    current: bool,
    relative: Option<usize>,
    selection: Selection,
    search: Option<(String, bool, bool)>,
    marker: char,
    sign: char,
}
pub struct FrameCache {
    rows: Vec<Vec<u8>>,
    dims: (u16, u16),
    composed: std::collections::HashMap<(usize, usize), (RowSignature, Vec<u8>)>,
}
impl FrameCache {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            dims: (0, 0),
            composed: Default::default(),
        }
    }
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
        queue!(
            row,
            MoveTo(x as u16, y as u16),
            SetBackgroundColor(bg),
            SetForegroundColor(Color::White),
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
    let mut frame = vec![Vec::new(); height];
    let mut cursor = (0, 0);
    let mut bar = false;
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
            let Some(b) = ed.buffers.iter().find(|b| b.id == w.buffer) else {
                continue;
            };
            if w.preview {
                draw_preview_pane(&mut frame, ed, b, &w, rect)?;
            } else if let Some(c) = draw_pane(&mut frame, ed, b, &w, rect, active, cache)? {
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
        if let Some(comp) = &ed.completion {
            if !comp.items.is_empty() {
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
                            if item.source == crate::completion::Source::Lsp {
                                "lsp"
                            } else {
                                "buf"
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
            }
        }
    }
    if cache.dims != (cols, rows) {
        queue!(out, Clear(ClearType::All))?;
        cache.rows.clear();
        cache.dims = (cols, rows);
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
    cache.rows = frame;
    if cache.composed.len() > height * 5 {
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
fn draw_pane(
    frame: &mut [Vec<u8>],
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
        let dest = &mut frame[y];
        let byte_start = dest.len();
        queue!(dest, MoveTo(r.x as u16, y as u16))?;
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
        let sig = RowSignature {
            buffer: b.id,
            revision: b.edit_seq,
            syntax: if b.id == ed.buf().id {
                ed.syntax_stamp
            } else {
                0
            },
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
        if let Some((old, bytes)) = cache.composed.get(&(r.x, y)) {
            if old == &sig {
                dest.truncate(byte_start);
                dest.extend_from_slice(bytes);
                continue;
            }
        }
        let (text, spans, matches) = source_cache.entry(d.line).or_insert_with(|| {
            let text = b.line_text(d.line);
            let mut spans = Vec::new();
            if b.id == ed.buf().id {
                if let Some(syn) = &ed.syntax {
                    let (start, end) = b.line_byte_range(d.line);
                    for (s, e, class) in syn.spans_in(start, end) {
                        let a = safe_boundary(&text, s.saturating_sub(start));
                        let z = safe_boundary(&text, e.saturating_sub(start));
                        spans.push((text[..a].chars().count(), text[..z].chars().count(), class));
                    }
                }
            }
            let matches: Vec<_> = search
                .as_ref()
                .into_iter()
                .flat_map(|re| {
                    re.find_iter(&text).filter_map(Result::ok).map(|m| {
                        (
                            text[..m.start()].chars().count(),
                            text[..m.end()].chars().count(),
                        )
                    })
                })
                .collect();
            (text, spans, matches)
        });
        let _ = text;
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
        let mut runs: Vec<((bool, bool, Color), String)> = Vec::new();
        for g in d.glyphs.iter() {
            let selected = selection.is_some_and(|(a, z)| {
                d.line >= a.0
                    && d.line <= z.0
                    && (if matches!(ed.mode, Mode::Visual(VisualKind::Block)) {
                        let ac = crate::grapheme::cell(&b.line_text(a.0), a.1, ed.config.tabstop);
                        let zc = crate::grapheme::cell(&b.line_text(z.0), z.1, ed.config.tabstop);
                        let gc = g.cell;
                        gc >= ac.min(zc) && gc <= ac.max(zc)
                    } else {
                        (matches!(ed.mode, Mode::Visual(VisualKind::Line))
                            || ((d.line > a.0 || g.col >= a.1) && (d.line < z.0 || g.col <= z.1)))
                    })
            });
            let searched = matches.iter().any(|(a, z)| g.col >= *a && g.col < *z);
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
            let style = (selected, searched, color);
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
        for ((selected, searched, color), text) in runs {
            if selected {
                queue!(dest, SetAttribute(Attribute::Reverse))?;
            } else if searched {
                queue!(dest, SetBackgroundColor(Color::DarkYellow))?;
            }
            queue!(
                dest,
                SetForegroundColor(color),
                Print(text),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
        }
        queue!(dest, Print(" ".repeat(width.saturating_sub(used))))?;
        cache
            .composed
            .insert((r.x, y), (sig, dest[byte_start..].to_vec()));
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
        frame,
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
    let detail_rows = if height >= 12 { 4 } else { 0 };
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
    let footer = if r.entries.iter().any(|e| e.note_id.is_some()) {
        "q close · e edit · R resolve · A agent · Tab select · y/Y copy · /? search · Ctrl-Q"
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
        crossterm::event::DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen
    );
    let raw = crossterm::terminal::disable_raw_mode();
    result.and(raw)
}
