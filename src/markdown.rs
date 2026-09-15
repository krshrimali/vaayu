//! Renders Markdown as styled terminal lines -- the terminal-native
//! equivalent of a webview preview panel. Uses pulldown-cmark's streaming
//! event parser (no full-document AST) so re-rendering on every edit stays
//! cheap even for large files.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub syntax: Option<crate::syntax::HlClass>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub inline_code: bool,
    pub code_block: bool,
    pub heading: u8,
    pub dim: bool,
    pub link: bool,
}

pub struct Span {
    pub text: String,
    pub style: SpanStyle,
}

pub type Line = Vec<Span>;

pub struct Preview {
    pub lines: Vec<Line>,
    pub scroll: usize,
    seq: Option<u64>,
    width: Option<usize>,
}

impl Preview {
    pub fn new() -> Preview {
        Preview {
            lines: Vec::new(),
            scroll: 0,
            seq: None,
            width: None,
        }
    }

    /// Re-renders if `seq` (the buffer's edit_seq) or the viewport `width`
    /// changed since the last render (width matters here because wrapping
    /// is baked into `lines` at render time, not recomputed per frame); a
    /// cheap no-op otherwise.
    /// Whether `refresh` would actually do anything for this (seq, width)
    /// -- lets a caller skip materializing the buffer text when the answer
    /// is no, rather than building it only to have `refresh` discard it.
    pub fn needs_refresh(&self, seq: u64, width: usize) -> bool {
        self.seq != Some(seq) || self.width != Some(width)
    }

    pub fn refresh(&mut self, text: &str, seq: u64, width: usize) {
        if self.seq == Some(seq) && self.width == Some(width) {
            return;
        }
        self.seq = Some(seq);
        self.width = Some(width);
        self.lines = wrap_lines(render(text), width);
        self.scroll = self.scroll.min(self.lines.len().saturating_sub(1));
    }
}

/// Word-wraps rendered lines to `width` columns, preserving per-character
/// styling across the break. Breaks at the last space before the limit when
/// one exists; hard-breaks a single unbroken run (e.g. a long code line or
/// URL) that alone exceeds the width.
fn wrap_lines(lines: Vec<Line>, width: usize) -> Vec<Line> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        if line.is_empty() {
            out.push(Vec::new());
            continue;
        }
        let chars: Vec<(char, SpanStyle)> = line
            .iter()
            .flat_map(|s| s.text.chars().map(|c| (c, s.style)))
            .collect();
        let total = chars.len();
        if total <= width {
            out.push(line);
            continue;
        }
        let mut start = 0;
        while start < total {
            let mut end = (start + width).min(total);
            if end < total {
                if let Some(space_rel) = chars[start..end].iter().rposition(|(c, _)| *c == ' ') {
                    if start + space_rel > start {
                        end = start + space_rel;
                    }
                }
            }
            out.push(merge_run(&chars[start..end]));
            start = end;
            while start < total && chars[start].0 == ' ' {
                start += 1;
            }
        }
    }
    out
}

fn merge_run(chars: &[(char, SpanStyle)]) -> Line {
    let mut out: Line = Vec::new();
    let mut text = String::new();
    let mut style = chars.first().map(|(_, s)| *s).unwrap_or_default();
    for &(c, s) in chars {
        if s == style {
            text.push(c);
        } else {
            out.push(Span {
                text: std::mem::take(&mut text),
                style,
            });
            text.push(c);
            style = s;
        }
    }
    if !text.is_empty() {
        out.push(Span { text, style });
    }
    out
}

struct Builder {
    lines: Vec<Line>,
    current: Line,
    bold: u32,
    italic: u32,
    strike: u32,
    heading: u8,
    list_stack: Vec<Option<u64>>, // None = bullet, Some(n) = next ordinal
    quote_depth: u32,
    in_code_block: bool,
    code_lang: Option<crate::syntax::Lang>,
    links: Vec<String>,
    table: Option<TableBuilder>,
}

#[derive(Default)]
struct TableBuilder {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    aligns: Vec<Alignment>,
    header_done: bool,
}

impl Builder {
    fn style(&self) -> SpanStyle {
        SpanStyle {
            bold: self.bold > 0 || self.heading > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            heading: self.heading,
            ..Default::default()
        }
    }

    fn push(&mut self, text: impl Into<String>, style: SpanStyle) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        if let Some(table) = &mut self.table {
            table.current_cell.push_str(&text);
        } else {
            self.current.push(Span { text, style });
        }
    }

    fn newline(&mut self) {
        let line = std::mem::take(&mut self.current);
        self.lines.push(line);
    }

    fn blank_if_needed(&mut self) {
        if !self.current.is_empty() {
            self.newline();
        }
        if !matches!(self.lines.last(), Some(l) if l.is_empty()) {
            self.lines.push(Vec::new());
        }
    }
}

pub fn render(source: &str) -> Vec<Line> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let mut b = Builder {
        lines: Vec::new(),
        current: Vec::new(),
        bold: 0,
        italic: 0,
        strike: 0,
        heading: 0,
        list_stack: Vec::new(),
        quote_depth: 0,
        in_code_block: false,
        code_lang: None,
        links: Vec::new(),
        table: None,
    };

    for event in Parser::new_ext(source, opts) {
        match event {
            Event::Start(tag) => start_tag(&mut b, tag),
            Event::End(tag) => end_tag(&mut b, tag),
            Event::Text(t) => text_event(&mut b, &t),
            Event::Code(t) => {
                let style = SpanStyle {
                    inline_code: true,
                    ..b.style()
                };
                b.push(t.to_string(), style);
            }
            Event::SoftBreak => b.push(" ", b.style()),
            Event::HardBreak => b.newline(),
            Event::Rule => {
                b.blank_if_needed();
                b.lines.push(vec![Span {
                    text: "\u{2500}".repeat(40),
                    style: SpanStyle {
                        dim: true,
                        ..Default::default()
                    },
                }]);
                b.lines.push(Vec::new());
            }
            Event::TaskListMarker(checked) => {
                let mark = if checked { "[x] " } else { "[ ] " };
                b.push(mark, b.style());
            }
            _ => {}
        }
    }
    if !b.current.is_empty() {
        b.newline();
    }
    b.lines
}

fn start_tag(b: &mut Builder, tag: Tag) {
    match tag {
        Tag::Heading { level, .. } => {
            b.blank_if_needed();
            b.heading = heading_num(level);
        }
        Tag::Paragraph => {
            if b.table.is_none() && b.list_stack.is_empty() {
                b.blank_if_needed();
            }
        }
        Tag::Emphasis => b.italic += 1,
        Tag::Strong => b.bold += 1,
        Tag::Strikethrough => b.strike += 1,
        Tag::BlockQuote(_) => {
            b.blank_if_needed();
            b.quote_depth += 1;
        }
        Tag::CodeBlock(kind) => {
            b.blank_if_needed();
            b.in_code_block = true;
            if let CodeBlockKind::Fenced(lang) = kind {
                b.code_lang = crate::syntax::lang_for_extension(match lang.as_ref() {
                    "rust" => "rs",
                    "python" => "py",
                    "javascript" => "js",
                    "typescript" => "ts",
                    "shell" => "sh",
                    s => s,
                });
                if !lang.is_empty() {
                    b.push(
                        format!("```{}", lang),
                        SpanStyle {
                            dim: true,
                            ..Default::default()
                        },
                    );
                    b.newline();
                }
            }
        }
        Tag::List(start) => {
            b.list_stack.push(start);
        }
        Tag::Item => {
            if !b.current.is_empty() {
                b.newline();
            }
            let depth = b.list_stack.len().saturating_sub(1);
            let indent = "  ".repeat(depth);
            let marker = match b.list_stack.last_mut() {
                Some(Some(n)) => {
                    let m = format!("{}. ", n);
                    *n += 1;
                    m
                }
                _ => "\u{2022} ".to_string(),
            };
            b.push(format!("{}{}", indent, marker), b.style());
        }
        Tag::Link { dest_url, .. } => {
            b.links.push(dest_url.to_string());
        }
        Tag::Table(aligns) => {
            b.table = Some(TableBuilder {
                aligns,
                ..Default::default()
            });
        }
        Tag::TableHead => {}
        Tag::TableRow => {}
        Tag::TableCell => {}
        _ => {}
    }
}

fn end_tag(b: &mut Builder, tag: TagEnd) {
    match tag {
        TagEnd::Heading(_) => {
            b.newline();
            b.heading = 0;
        }
        TagEnd::Paragraph => {
            if b.table.is_none() {
                b.newline();
            }
        }
        TagEnd::Emphasis => b.italic = b.italic.saturating_sub(1),
        TagEnd::Strong => b.bold = b.bold.saturating_sub(1),
        TagEnd::Strikethrough => b.strike = b.strike.saturating_sub(1),
        TagEnd::BlockQuote(_) => {
            b.quote_depth = b.quote_depth.saturating_sub(1);
            if b.quote_depth == 0 {
                b.blank_if_needed();
            }
        }
        TagEnd::CodeBlock => {
            b.in_code_block = false;
            b.code_lang = None;
            b.push(
                "```",
                SpanStyle {
                    dim: true,
                    ..Default::default()
                },
            );
            b.newline();
            b.lines.push(Vec::new());
        }
        TagEnd::List(_) => {
            b.list_stack.pop();
            if b.list_stack.is_empty() {
                b.blank_if_needed();
            }
        }
        TagEnd::Item => {
            if !b.current.is_empty() {
                b.newline();
            }
        }
        TagEnd::Link => {
            if let Some(url) = b.links.pop() {
                b.push(
                    format!(" ({url})"),
                    SpanStyle {
                        dim: true,
                        link: true,
                        ..Default::default()
                    },
                );
            }
        }
        TagEnd::Table => {
            if let Some(t) = b.table.take() {
                emit_table(b, t);
            }
        }
        TagEnd::TableHead => {
            if let Some(t) = &mut b.table {
                t.rows.push(std::mem::take(&mut t.current_row));
                t.header_done = true;
            }
        }
        TagEnd::TableRow => {
            if let Some(t) = &mut b.table {
                t.rows.push(std::mem::take(&mut t.current_row));
            }
        }
        TagEnd::TableCell => {
            if let Some(t) = &mut b.table {
                let cell = std::mem::take(&mut t.current_cell);
                t.current_row.push(cell);
            }
        }
        _ => {}
    }
}

fn text_event(b: &mut Builder, t: &str) {
    if let Some(table) = &mut b.table {
        table.current_cell.push_str(t);
        return;
    }
    if b.in_code_block {
        let mut parser = b.code_lang.and_then(crate::syntax::Syntax::new);
        if let Some(p) = &mut parser {
            p.reparse(t.into());
        }
        let spans: Vec<_> = parser
            .as_ref()
            .map(|p| p.spans_in(0, t.len()).collect())
            .unwrap_or_default();
        for (byte, c) in t.char_indices() {
            if c == '\n' {
                b.newline();
                continue;
            }
            let syntax = spans
                .iter()
                .find(|(a, z, _)| byte >= *a && byte < *z)
                .map(|(_, _, class)| *class);
            let style = SpanStyle {
                code_block: true,
                syntax,
                ..Default::default()
            };
            if let Some(last) = b.current.last_mut() {
                if last.style == style {
                    last.text.push(c);
                    continue;
                }
            }
            b.push(c.to_string(), style);
        }
        return;
    }
    if b.quote_depth > 0 && b.current.is_empty() {
        b.push(
            "\u{258e} ".repeat(b.quote_depth as usize),
            SpanStyle {
                dim: true,
                ..Default::default()
            },
        );
    }
    let mut style = b.style();
    style.dim = style.dim || b.quote_depth > 0;
    style.italic = style.italic || b.quote_depth > 0;
    b.push(t.to_string(), style);
}

fn emit_table(b: &mut Builder, t: TableBuilder) {
    let cols = t
        .aligns
        .len()
        .max(t.rows.iter().map(|r| r.len()).max().unwrap_or(0));
    let mut widths = vec![0usize; cols];
    for row in &t.rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(unicode_width::UnicodeWidthStr::width(cell.as_str()));
        }
    }
    for (ri, row) in t.rows.iter().enumerate() {
        let mut line: Line = Vec::new();
        for (i, w) in widths.iter().enumerate() {
            let cell = row.get(i).map(String::as_str).unwrap_or("");
            let pad = w.saturating_sub(unicode_width::UnicodeWidthStr::width(cell));
            let (left, right) = match t.aligns.get(i) {
                Some(Alignment::Right) => (pad, 0),
                Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                _ => (0, pad),
            };
            let text = format!("{}{}{} ", " ".repeat(left), cell, " ".repeat(right));
            line.push(Span {
                text,
                style: SpanStyle {
                    bold: ri == 0,
                    ..Default::default()
                },
            });
            if i + 1 < widths.len() {
                line.push(Span {
                    text: "\u{2502} ".to_string(),
                    style: SpanStyle {
                        dim: true,
                        ..Default::default()
                    },
                });
            }
        }
        b.lines.push(line);
        if ri == 0 {
            let rule: String = widths
                .iter()
                .map(|w| "\u{2500}".repeat(w + 1))
                .collect::<Vec<_>>()
                .join("\u{253c}");
            b.lines.push(vec![Span {
                text: rule,
                style: SpanStyle {
                    dim: true,
                    ..Default::default()
                },
            }]);
        }
    }
    b.lines.push(Vec::new());
}

fn heading_num(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strikethrough_and_link_preserved() {
        let src = "This is a paragraph with **bold text**, *italic text*, and `inline code`.\nIt also has ~~strikethrough~~ and a [link](https://example.com).\n";
        let lines = render(src);
        for line in &lines {
            let text: String = line.iter().map(|s| s.text.as_str()).collect();
            println!("LINE: {:?}", text);
        }
        let joined: String = lines
            .iter()
            .flat_map(|l| l.iter())
            .map(|s| s.text.as_str())
            .collect();
        assert!(
            joined.contains("strikethrough"),
            "lost strikethrough text: {:?}",
            joined
        );
        assert!(joined.contains("link"), "lost link text: {:?}", joined);
    }
}
