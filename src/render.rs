use std::io::{self, Write};

use crossterm::cursor::{MoveTo, SetCursorStyle};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{execute, queue};
use unicode_width::UnicodeWidthStr;

use crate::editor::Editor;
use crate::mode::{CommandKind, Mode, VisualKind};

pub fn adjust_viewport(ed: &mut Editor, rows: usize) {
    ed.screen_rows = rows;
    let scrolloff = ed.config.scrolloff.min(rows / 2);
    let line = ed.buf().cursor_line;
    let top = ed.buf().top_line;
    let last_line = ed.buf().line_count().saturating_sub(1);

    let mut new_top = top;
    if line < top + scrolloff {
        new_top = line.saturating_sub(scrolloff);
    } else if rows > 0 && line + scrolloff >= top + rows {
        new_top = (line + scrolloff + 1).saturating_sub(rows);
    }
    let max_top = last_line.saturating_sub(rows.saturating_sub(1)).max(0);
    new_top = new_top.min(max_top.max(0));
    ed.buf_mut().top_line = new_top;
}

pub fn draw<W: Write>(out: &mut W, ed: &Editor, term_cols: u16, term_rows: u16) -> io::Result<()> {
    if matches!(ed.mode, Mode::Picker) {
        return draw_picker(out, ed, term_cols, term_rows);
    }

    let rows = term_rows.saturating_sub(2) as usize; // status + message line
    let sign_w = if ed.git.is_some() { 1 } else { 0 };
    let gutter_w = sign_w
        + if ed.config.number {
            (ed.buf().line_count().to_string().len() + 1).max(4)
        } else {
            0
        };
    let text_cols = (term_cols as usize).saturating_sub(gutter_w);

    queue!(out, Clear(ClearType::All))?;

    let sel = selection_bounds(ed);
    let search_re = build_search_regex(ed);

    for row in 0..rows {
        let line_idx = ed.buf().top_line + row;
        queue!(out, MoveTo(0, row as u16))?;
        if line_idx >= ed.buf().line_count() {
            queue!(out, SetForegroundColor(Color::DarkGrey), Print("~"), ResetColor)?;
            continue;
        }

        if sign_w > 0 {
            let sign = ed.git.as_ref().and_then(|g| g.signs.get(&line_idx)).copied();
            match sign {
                Some(crate::gitdiff::Sign::Added) => {
                    queue!(out, SetForegroundColor(Color::Green), Print("\u{258e}"), ResetColor)?
                }
                Some(crate::gitdiff::Sign::Modified) => {
                    queue!(out, SetForegroundColor(Color::Yellow), Print("\u{258e}"), ResetColor)?
                }
                Some(crate::gitdiff::Sign::Removed) => {
                    queue!(out, SetForegroundColor(Color::Red), Print("\u{2581}"), ResetColor)?
                }
                None => queue!(out, Print(" "))?,
            }
        }

        if gutter_w > sign_w {
            let num = if ed.config.relativenumber && line_idx != ed.buf().cursor_line {
                (line_idx as isize - ed.buf().cursor_line as isize).unsigned_abs()
            } else {
                line_idx + 1
            };
            let text = format!("{:>width$} ", num, width = gutter_w - 1);
            let color = if line_idx == ed.buf().cursor_line { Color::Yellow } else { Color::DarkGrey };
            queue!(out, SetForegroundColor(color), Print(&text), ResetColor)?;
        }

        let line_text = ed.buf().line_text(line_idx);
        let display: String = line_text.chars().take(text_cols.max(1)).collect();
        let syn_spans = syntax_spans_for_line(ed, &line_text, line_idx);
        draw_line_with_highlights(out, &display, line_idx, sel, search_re.as_ref(), &syn_spans)?;
    }

    draw_completion_popup(out, ed, gutter_w, rows, term_cols)?;

    draw_statusline(out, ed, term_cols, term_rows.saturating_sub(2))?;
    draw_messageline(out, ed, term_rows.saturating_sub(1))?;

    let (cl, cc) = (ed.buf().cursor_line, ed.buf().cursor_col);
    let screen_row = cl.saturating_sub(ed.buf().top_line);
    let screen_col = gutter_w + cc;
    queue!(out, MoveTo(screen_col as u16, screen_row.min(rows.saturating_sub(1).max(0)) as u16))?;

    match ed.mode {
        Mode::Insert => queue!(out, SetCursorStyle::SteadyBar)?,
        _ => queue!(out, SetCursorStyle::SteadyBlock)?,
    }

    out.flush()
}

type Sel = Option<((usize, usize), (usize, usize), VisualKind)>;

fn selection_bounds(ed: &Editor) -> Sel {
    if let Mode::Visual(kind) = ed.mode {
        if let Some(anchor) = ed.visual_anchor {
            let cursor = (ed.buf().cursor_line, ed.buf().cursor_col);
            let (a, b) = if (anchor.0, anchor.1) <= (cursor.0, cursor.1) { (anchor, cursor) } else { (cursor, anchor) };
            return Some((a, b, kind));
        }
    }
    None
}

fn build_search_regex(ed: &Editor) -> Option<regex::Regex> {
    if !ed.hl_search {
        return None;
    }
    let (pattern, _) = ed.last_search.as_ref()?;
    if pattern.is_empty() {
        return None;
    }
    let ci = ed.config.ignorecase && !(ed.config.smartcase && pattern.chars().any(|c| c.is_uppercase()));
    regex::RegexBuilder::new(pattern).case_insensitive(ci).build().ok()
}

fn draw_line_with_highlights<W: Write>(
    out: &mut W,
    text: &str,
    line_idx: usize,
    sel: Sel,
    search_re: Option<&regex::Regex>,
    syn_spans: &[(usize, usize, crate::syntax::HlClass)],
) -> io::Result<()> {
    let chars: Vec<char> = text.chars().collect();
    let mut syn_cols: Vec<Option<crate::syntax::HlClass>> = vec![None; chars.len()];
    for (s, e, class) in syn_spans {
        for i in *s..(*e).min(chars.len()) {
            syn_cols[i] = Some(*class);
        }
    }
    let mut search_cols = vec![false; chars.len()];
    if let Some(re) = search_re {
        for m in re.find_iter(text) {
            let start_c = text[..m.start()].chars().count();
            let end_c = text[..m.end()].chars().count();
            for i in start_c..end_c.min(chars.len()) {
                search_cols[i] = true;
            }
        }
    }

    let sel_range: Option<(usize, usize)> = sel.and_then(|(a, b, kind)| {
        if line_idx < a.0 || line_idx > b.0 {
            return None;
        }
        match kind {
            VisualKind::Line => Some((0, chars.len())),
            VisualKind::Char => {
                let start = if line_idx == a.0 { a.1 } else { 0 };
                let end = if line_idx == b.0 { (b.1 + 1).min(chars.len()) } else { chars.len() };
                Some((start, end))
            }
        }
    });

    if chars.is_empty() {
        queue!(out, Print(""))?;
        return Ok(());
    }

    #[derive(PartialEq, Clone, Copy)]
    enum Style {
        Plain,
        Selected,
        Searched,
        Syntax(crate::syntax::HlClass),
    }
    let style_at = |i: usize| -> Style {
        if sel_range.map(|(s, e)| i >= s && i < e).unwrap_or(false) {
            Style::Selected
        } else if search_cols[i] {
            Style::Searched
        } else if let Some(class) = syn_cols[i] {
            Style::Syntax(class)
        } else {
            Style::Plain
        }
    };

    // Batch consecutive same-styled characters into one escape sequence +
    // one Print, instead of one per character -- cuts output bytes
    // dramatically on syntax-highlighted lines (matters over SSH).
    let mut i = 0;
    while i < chars.len() {
        let style = style_at(i);
        let mut j = i + 1;
        while j < chars.len() && style_at(j) == style {
            j += 1;
        }
        let run: String = chars[i..j].iter().collect();
        match style {
            Style::Plain => {
                queue!(out, Print(run))?;
            }
            Style::Selected => {
                queue!(out, SetAttribute(Attribute::Reverse), Print(run), SetAttribute(Attribute::Reset))?;
            }
            Style::Searched => {
                queue!(
                    out,
                    SetBackgroundColor(Color::DarkYellow),
                    SetForegroundColor(Color::Black),
                    Print(run),
                    ResetColor
                )?;
            }
            Style::Syntax(class) => {
                queue!(out, SetForegroundColor(syntax_color(class)), Print(run), ResetColor)?;
            }
        }
        i = j;
    }
    Ok(())
}

fn syntax_color(class: crate::syntax::HlClass) -> Color {
    use crate::syntax::HlClass;
    match class {
        HlClass::Comment => Color::DarkGrey,
        HlClass::String => Color::Green,
        HlClass::Number => Color::Magenta,
        HlClass::Keyword => Color::Cyan,
    }
}

fn syntax_spans_for_line(
    ed: &Editor,
    line_text: &str,
    line_idx: usize,
) -> Vec<(usize, usize, crate::syntax::HlClass)> {
    let Some(syn) = &ed.syntax else {
        return Vec::new();
    };
    let (line_start, line_end) = ed.buf().line_byte_range(line_idx);
    syn.spans_in(line_start, line_end)
        .map(|(s, e, class)| {
            let start_c = line_text[..(s - line_start).min(line_text.len())].chars().count();
            let end_c = line_text[..(e - line_start).min(line_text.len())].chars().count();
            (start_c, end_c, class)
        })
        .collect()
}

fn draw_completion_popup<W: Write>(out: &mut W, ed: &Editor, gutter_w: usize, text_rows: usize, term_cols: u16) -> io::Result<()> {
    let Some(comp) = &ed.completion else { return Ok(()) };
    if comp.items.is_empty() {
        return Ok(());
    }
    let (word_line, word_col) = comp.start;
    if word_line < ed.buf().top_line {
        return Ok(());
    }
    let word_row = word_line - ed.buf().top_line;
    if word_row >= text_rows {
        return Ok(());
    }
    let screen_col = (gutter_w + word_col).min((term_cols as usize).saturating_sub(1));

    let max_items = 8usize;
    let visible = comp.items.len().min(max_items);
    let width = comp
        .items
        .iter()
        .take(max_items)
        .map(|i| i.label.chars().count())
        .max()
        .unwrap_or(4)
        .clamp(6, 40)
        + 2;
    let width = width.min((term_cols as usize).saturating_sub(screen_col).max(4));

    let below_space = text_rows.saturating_sub(word_row + 1);
    let draw_below = below_space >= visible.min(3);
    let start_row = if draw_below { word_row + 1 } else { word_row.saturating_sub(visible) };

    for (i, item) in comp.items.iter().take(max_items).enumerate() {
        let row = start_row + i;
        if row >= text_rows {
            break;
        }
        queue!(out, MoveTo(screen_col as u16, row as u16))?;
        let mut label: String = item.label.chars().take(width.saturating_sub(2)).collect();
        while label.chars().count() < width.saturating_sub(1) {
            label.push(' ');
        }
        let text = format!(" {}", label);
        if i == comp.selected {
            queue!(out, SetAttribute(Attribute::Reverse), Print(&text), SetAttribute(Attribute::Reset))?;
        } else {
            queue!(out, SetBackgroundColor(Color::DarkBlue), SetForegroundColor(Color::White), Print(&text), ResetColor)?;
        }
    }
    Ok(())
}

fn draw_statusline<W: Write>(out: &mut W, ed: &Editor, cols: u16, row: u16) -> io::Result<()> {
    let mode_label = ed.mode.label();
    let name = ed.buf().name();
    let modified = if ed.buf().modified { " [+]" } else { "" };
    let (line, col) = (ed.buf().cursor_line + 1, ed.buf().cursor_col + 1);
    let total = ed.buf().line_count();
    let pct = if total > 0 { (line * 100 / total).min(100) } else { 100 };
    let recording = match &ed.macro_recording {
        Some((r, _)) => format!(" REC@{}", r),
        None => String::new(),
    };

    let left = format!(" {} | {}{}{} ", mode_label, name, modified, recording);
    let right = format!(" {}:{}  {}% ", line, col, pct);
    let mid = (cols as usize).saturating_sub(UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(right.as_str()));

    queue!(out, MoveTo(0, row))?;
    queue!(out, SetBackgroundColor(Color::DarkBlue), SetForegroundColor(Color::White))?;
    queue!(out, Print(&left), Print(" ".repeat(mid)), Print(&right))?;
    queue!(out, ResetColor)?;
    Ok(())
}

fn draw_messageline<W: Write>(out: &mut W, ed: &Editor, row: u16) -> io::Result<()> {
    queue!(out, MoveTo(0, row))?;
    let text = match ed.mode {
        Mode::Command(CommandKind::Ex) => format!(":{}", ed.cmdline),
        Mode::Command(CommandKind::SearchFwd) => format!("/{}", ed.cmdline),
        Mode::Command(CommandKind::SearchBack) => format!("?{}", ed.cmdline),
        _ => ed.message.clone(),
    };
    queue!(out, Print(&text))?;
    Ok(())
}

fn draw_picker<W: Write>(out: &mut W, ed: &Editor, term_cols: u16, term_rows: u16) -> io::Result<()> {
    queue!(out, Clear(ClearType::All))?;
    let Some(picker) = &ed.file_picker else {
        return out.flush();
    };

    let cols = term_cols as usize;
    let rows = term_rows as usize;

    queue!(out, MoveTo(0, 0))?;
    queue!(out, SetForegroundColor(Color::Yellow), Print("> "), ResetColor, Print(&picker.query))?;

    let list_rows = rows.saturating_sub(3);
    for (i, (_, path)) in picker.matches.iter().take(list_rows).enumerate() {
        queue!(out, MoveTo(0, (i + 1) as u16))?;
        let line: String = path.chars().take(cols).collect();
        if i == picker.selected {
            queue!(out, SetAttribute(Attribute::Reverse), Print(&line), SetAttribute(Attribute::Reset))?;
        } else {
            queue!(out, Print(&line))?;
        }
    }

    queue!(out, MoveTo(0, term_rows.saturating_sub(1)))?;
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print(format!(
            "{} / {} files  --  type to filter, ^n/^p or arrows to move, Enter to open, Esc to cancel",
            picker.matches.len(),
            ed.all_files.len()
        )),
        ResetColor
    )?;

    let cursor_col = 2 + picker.query.chars().count();
    queue!(out, MoveTo(cursor_col.min(cols.saturating_sub(1)) as u16, 0))?;
    queue!(out, SetCursorStyle::SteadyBar)?;
    out.flush()
}

pub fn setup_terminal() -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen, crossterm::cursor::Hide)
}

pub fn teardown_terminal() -> io::Result<()> {
    execute!(io::stdout(), crossterm::cursor::Show, crossterm::terminal::LeaveAlternateScreen)?;
    crossterm::terminal::disable_raw_mode()
}
