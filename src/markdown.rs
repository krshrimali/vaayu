//! Renders Markdown as styled terminal lines -- the terminal-native
//! equivalent of a webview preview panel. Uses pulldown-cmark's streaming
//! event parser (no full-document AST) so re-rendering on every edit stays
//! cheap even for large files.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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
    /// Diagram geometry must never be word-wrapped.
    pub diagram: bool,
    pub image: Option<crate::graphics::ImageRow>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: SpanStyle,
}

pub type Line = Vec<Span>;

pub struct Preview {
    pub lines: Vec<Line>,
    pub scroll: usize,
    pub left: usize,
    pub zoom: u16,
    seq: Option<u64>,
    width: Option<usize>,
    revision: (u64, u64),
    document: Option<PathBuf>,
    context: Option<RenderContext>,
    diagrams: Vec<crate::mermaid::Key>,
    images: Vec<crate::markdown_images::Source>,
}

pub struct Services<'a> {
    pub diagrams: &'a mut crate::mermaid::Service,
    pub images: &'a mut crate::markdown_images::Service,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RenderContext {
    pub enabled: bool,
    pub graphical: bool,
    pub palette: crate::mermaid::Palette,
    pub zoom: u16,
    /// Thousandths, so viewport cache comparisons remain exact.
    pub cell_ratio: u16,
    pub cell_width: u16,
}

impl Preview {
    pub fn new() -> Preview {
        Preview {
            lines: Vec::new(),
            scroll: 0,
            left: 0,
            zoom: 100,
            seq: None,
            width: None,
            revision: (0, 0),
            document: None,
            context: None,
            diagrams: Vec::new(),
            images: Vec::new(),
        }
    }

    /// Re-renders if `seq` (the buffer's edit_seq) or the viewport `width`
    /// changed since the last render (width matters here because wrapping
    /// is baked into `lines` at render time, not recomputed per frame); a
    /// cheap no-op otherwise.
    /// Whether `refresh` would actually do anything for this (seq, width)
    /// -- lets a caller skip materializing the buffer text when the answer
    /// is no, rather than building it only to have `refresh` discard it.
    pub fn needs_refresh(
        &self,
        seq: u64,
        width: usize,
        revision: (u64, u64),
        context: RenderContext,
        document: Option<&Path>,
    ) -> bool {
        self.seq != Some(seq)
            || self.width != Some(width)
            || self.revision != revision
            || self.context != Some(context)
            || self.document.as_deref() != document
    }

    pub fn refresh(
        &mut self,
        text: &str,
        seq: u64,
        width: usize,
        services: Services<'_>,
        context: RenderContext,
        document: Option<&Path>,
    ) {
        let Services {
            diagrams: service,
            images,
        } = services;
        if !self.needs_refresh(
            seq,
            width,
            (service.revision, images.revision),
            context,
            document,
        ) {
            self.touch(service, images);
            return;
        }
        let anchor = self
            .lines
            .get(self.scroll)
            .filter(|l| !l.is_empty() && l.iter().all(|s| s.style.image.is_none()))
            .cloned();
        self.seq = Some(seq);
        self.width = Some(width);
        self.context = Some(context);
        self.document = document.map(Path::to_path_buf);
        self.diagrams.clear();
        self.images.clear();
        let lines = render_with(
            text,
            |source| {
                if !context.enabled {
                    return source_diagram(source, None);
                }
                let key = crate::mermaid::Key {
                    source: source.into(),
                    palette: context.palette,
                    graphical: context.graphical,
                };
                let artifact = service.request(&key);
                self.diagrams.push(key);
                diagram_lines(source, artifact.as_deref(), width, context)
            },
            |destination, alt| {
                let source = match crate::markdown_images::Source::resolve(destination, document) {
                    Ok(source) => source,
                    Err(error) => return source_image(destination, alt, Some(&error)),
                };
                // Touch even in text mode so an images-only document negotiates
                // graphics capability on its first frame.
                self.images.push(source.clone());
                images.touch(&source);
                if !context.graphical {
                    return source_image(destination, alt, None);
                }
                match images.request(&source) {
                    None => source_image(destination, alt, Some("loading…")),
                    Some(Err(error)) => source_image(destination, alt, Some(&error)),
                    Some(Ok(image)) => image_lines(&image, width, context, 1.0),
                }
            },
        );
        self.lines = wrap_lines(lines, width);
        self.revision = (service.revision, images.revision);
        if self.scroll > 0 {
            if let Some(anchor) = anchor {
                if let Some(row) = self
                    .lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| **line == anchor)
                    .min_by_key(|(i, _)| i.abs_diff(self.scroll))
                    .map(|(i, _)| i)
                {
                    self.scroll = row;
                }
            }
        }
        self.scroll = self.scroll.min(self.lines.len().saturating_sub(1));
        self.left = self.left.min(self.max_left(width));
    }

    pub fn touch(
        &self,
        service: &mut crate::mermaid::Service,
        images: &mut crate::markdown_images::Service,
    ) {
        for key in &self.diagrams {
            service.touch(key);
        }
        for source in &self.images {
            images.touch(source);
        }
    }

    pub fn max_left(&self, width: usize) -> usize {
        self.lines
            .iter()
            .filter(|line| line.iter().any(|span| span.style.diagram))
            .map(|line| {
                line.iter()
                    .map(|span| UnicodeWidthStr::width(span.text.as_str()))
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
            .saturating_sub(width)
    }

    pub fn max_scroll(&self, height: usize) -> usize {
        self.lines.len().saturating_sub(height.max(1))
    }
}

fn source_diagram(source: &str, status: Option<&str>) -> Vec<Line> {
    let mut rows = Vec::new();
    let heading = status
        .map(|s| format!("Mermaid · {s}"))
        .unwrap_or_else(|| "```mermaid".into());
    rows.push(vec![Span {
        text: heading,
        style: SpanStyle {
            dim: true,
            ..Default::default()
        },
    }]);
    for row in crate::mermaid::clean_text(source).lines() {
        rows.push(vec![Span {
            text: row.replace('\t', "    "),
            style: SpanStyle {
                code_block: true,
                ..Default::default()
            },
        }]);
    }
    if status.is_none() {
        rows.push(vec![Span {
            text: "```".into(),
            style: SpanStyle {
                dim: true,
                ..Default::default()
            },
        }]);
    }
    rows
}

fn diagram_lines(
    source: &str,
    artifact: Option<&crate::mermaid::Artifact>,
    width: usize,
    context: RenderContext,
) -> Vec<Line> {
    use crate::mermaid::Artifact;
    match artifact {
        None => source_diagram(source, Some("rendering…")),
        Some(Artifact::Error(error)) => source_diagram(source, Some(error)),
        Some(Artifact::Text(rows)) => rows
            .iter()
            .map(|row| {
                vec![Span {
                    text: row.clone(),
                    style: SpanStyle {
                        diagram: true,
                        ..Default::default()
                    },
                }]
            })
            .collect(),
        Some(Artifact::Image(image)) => image_lines(image, width, context, 1.5),
    }
}

fn source_image(destination: &str, alt: &str, status: Option<&str>) -> Vec<Line> {
    let label = if alt.trim().is_empty() {
        "Image".into()
    } else {
        format!("Image: {alt}")
    };
    let mut text = format!("[{label}] ({destination})");
    if let Some(status) = status {
        text.push_str(&format!(" · {status}"));
    }
    vec![vec![Span {
        text: crate::mermaid::clean_text(&text).replace(['\n', '\t'], " "),
        style: SpanStyle {
            dim: true,
            ..Default::default()
        },
    }]]
}

fn image_lines(
    image: &crate::graphics::Image,
    width: usize,
    context: RenderContext,
    scale: f64,
) -> Vec<Line> {
    let ratio = f64::from(context.cell_ratio) / 1000.0;
    let aspect = f64::from(image.height) / f64::from(image.width.max(1));
    let natural_columns = (f64::from(image.width) / scale / f64::from(context.cell_width.max(1)))
        .ceil()
        .max(1.0) as usize;
    let columns = (width.max(1).min(natural_columns) * usize::from(context.zoom) / 100)
        .clamp(1, 256)
        .min((256.0 / (ratio * aspect)).floor().max(1.0) as usize);
    let rows = (columns as f64 * ratio * aspect).ceil().clamp(1.0, 256.0) as u16;
    let padding = width.saturating_sub(columns) / 2;
    (0..rows)
        .map(|row| {
            vec![
                Span {
                    text: " ".repeat(padding),
                    style: SpanStyle {
                        diagram: true,
                        ..Default::default()
                    },
                },
                Span {
                    text: " ".repeat(columns),
                    style: SpanStyle {
                        diagram: true,
                        image: Some(crate::graphics::ImageRow {
                            id: image.id,
                            row,
                            columns: columns as u16,
                            rows,
                        }),
                        ..Default::default()
                    },
                },
            ]
        })
        .collect()
}

/// Word-wraps rendered lines to `width` columns, preserving per-character
/// styling across the break. Breaks at the last space before the limit when
/// one exists; hard-breaks a single unbroken run (e.g. a long code line or
/// URL) that alone exceeds the width.
fn wrap_lines(lines: Vec<Line>, width: usize) -> Vec<Line> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        if line.iter().any(|span| span.style.diagram) {
            out.push(line);
            continue;
        }
        if line.is_empty() {
            out.push(Vec::new());
            continue;
        }
        // Wrap by DISPLAY width (matching the rest of the preview, which
        // measures with UnicodeWidthStr), over whole graphemes so a wide
        // CJK cell or a base+combining cluster is never split.
        let units: Vec<(String, SpanStyle)> = line
            .iter()
            .flat_map(|s| s.text.graphemes(true).map(|g| (g.to_string(), s.style)))
            .collect();
        let total: usize = units
            .iter()
            .map(|(g, _)| UnicodeWidthStr::width(g.as_str()))
            .sum();
        if total <= width {
            out.push(line);
            continue;
        }
        let count = units.len();
        let mut start = 0;
        while start < count {
            // Take graphemes until the next one would overflow the width,
            // but always at least one (so a lone glyph wider than the pane
            // still hard-breaks onto its own row instead of looping).
            let mut end = start;
            let mut cells = 0;
            while end < count {
                let gw = UnicodeWidthStr::width(units[end].0.as_str());
                if cells + gw > width && end > start {
                    break;
                }
                cells += gw;
                end += 1;
            }
            if end < count {
                if let Some(space_rel) = units[start..end].iter().rposition(|(g, _)| g == " ") {
                    if space_rel > 0 {
                        end = start + space_rel;
                    }
                }
            }
            out.push(merge_run(&units[start..end]));
            start = end;
            while start < count && units[start].0 == " " {
                start += 1;
            }
        }
    }
    out
}

fn merge_run(units: &[(String, SpanStyle)]) -> Line {
    let mut out: Line = Vec::new();
    let mut text = String::new();
    let mut style = units.first().map(|(_, s)| *s).unwrap_or_default();
    for (g, s) in units {
        if *s == style {
            text.push_str(g);
        } else {
            out.push(Span {
                text: std::mem::take(&mut text),
                style,
            });
            text.push_str(g);
            style = *s;
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

#[cfg(test)]
pub fn render(source: &str) -> Vec<Line> {
    render_with(
        source,
        |source| source_diagram(source, None),
        |destination, alt| source_image(destination, alt, None),
    )
}

fn render_with(
    source: &str,
    mut resolve: impl FnMut(&str) -> Vec<Line>,
    mut resolve_image: impl FnMut(&str, &str) -> Vec<Line>,
) -> Vec<Line> {
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

    let mut mermaid: Option<String> = None;
    let mut image: Option<(String, String, usize)> = None;
    for event in Parser::new_ext(source, opts) {
        if let Some((_, alt, depth)) = &mut image {
            match event {
                Event::Start(Tag::Image { .. }) => *depth += 1,
                Event::End(TagEnd::Image) if *depth > 0 => *depth -= 1,
                Event::End(TagEnd::Image) => {
                    let (destination, alt, _) = image.take().unwrap();
                    if b.table.is_some() {
                        for span in source_image(&destination, &alt, None).into_iter().flatten() {
                            b.push(span.text, span.style);
                        }
                    } else {
                        if !b.current.is_empty() {
                            b.newline();
                        }
                        b.lines.extend(resolve_image(&destination, &alt));
                    }
                }
                Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                Event::SoftBreak | Event::HardBreak => alt.push(' '),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                image = Some((dest_url.to_string(), String::new(), 0));
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info
                    .split_whitespace()
                    .next()
                    .is_some_and(|s| s.eq_ignore_ascii_case("mermaid")) =>
            {
                b.blank_if_needed();
                mermaid = Some(String::new());
            }
            Event::Text(t) if mermaid.is_some() => mermaid.as_mut().unwrap().push_str(&t),
            Event::End(TagEnd::CodeBlock) if mermaid.is_some() => {
                b.lines.extend(resolve(&mermaid.take().unwrap()));
                b.lines.push(Vec::new());
            }
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
    fn images_collect_alt_text_references_and_linked_images_without_leaking_styles() {
        let source = "before ![**bold** and `code`](a.png) after\n\n![Reference][photo]\n\n[![Linked](b.jpg)](https://example.com)\n\n[photo]: images/photo.png \"Title\"\n";
        let mut captured = Vec::new();
        let lines = render_with(
            source,
            |_| Vec::new(),
            |destination, alt| {
                captured.push((destination.to_string(), alt.to_string()));
                source_image(destination, alt, None)
            },
        );
        assert_eq!(
            captured,
            [
                ("a.png".into(), "bold and code".into()),
                ("images/photo.png".into(), "Reference".into()),
                ("b.jpg".into(), "Linked".into())
            ]
        );
        let after = lines
            .iter()
            .flatten()
            .find(|s| s.text.contains("after"))
            .unwrap();
        assert!(!after.style.bold && !after.style.inline_code);
        let text: String = lines.iter().flatten().map(|s| s.text.as_str()).collect();
        assert!(text.contains("before") && text.contains("https://example.com"));
    }

    #[test]
    fn table_images_use_readable_fallback_and_fenced_image_syntax_stays_literal() {
        let source =
            "| Picture |\n| --- |\n| ![Example](x.png) |\n\n```md\n![literal](y.png)\n```\n";
        let lines = render_with(
            source,
            |_| Vec::new(),
            |_, _| panic!("table and code images must stay text"),
        );
        let text: String = lines.iter().flatten().map(|s| s.text.as_str()).collect();
        assert!(text.contains("[Image: Example] (x.png)"));
        assert!(text.contains("![literal](y.png)"));
        let fallback = source_image("missing.png", "", Some("not found"));
        assert_eq!(fallback[0][0].text, "[Image] (missing.png) · not found");
    }

    #[test]
    fn image_geometry_preserves_aspect_ratio_zoom_and_crop_coordinates() {
        let image = crate::graphics::Image::new(Vec::new(), 800, 400).unwrap();
        let context = RenderContext {
            enabled: true,
            graphical: true,
            palette: crate::mermaid::Palette::for_theme(crate::theme::Theme::default()),
            zoom: 100,
            cell_ratio: 500,
            cell_width: 8,
        };
        let rows = wrap_lines(image_lines(&image, 40, context, 1.0), 10);
        assert_eq!(rows.len(), 10);
        for (i, row) in rows.iter().enumerate() {
            let cell = row.iter().find_map(|s| s.style.image).unwrap();
            assert_eq!((cell.row, cell.columns, cell.rows), (i as u16, 40, 10));
        }
        let rows = image_lines(
            &image,
            40,
            RenderContext {
                zoom: 200,
                ..context
            },
            1.0,
        );
        assert_eq!(rows.len(), 20);
        assert_eq!(rows[0][1].style.image.unwrap().columns, 80);
    }

    #[test]
    fn diagram_fences_are_collected_in_quotes_lists_and_with_info_attributes() {
        let source = "> ~~~mermaid title\n> flowchart TD\n> A --> B\n> ~~~\n\n- diagram:\n\n  ```MERMAID\n  sequenceDiagram\n  A->>B: Hello\n  ```\n\n```rust\nfn main() {}\n```\n";
        let mut captured = Vec::new();
        let lines = render_with(
            source,
            |s| {
                captured.push(s.to_string());
                Vec::new()
            },
            |destination, alt| source_image(destination, alt, None),
        );
        assert_eq!(
            captured,
            ["flowchart TD\nA --> B\n", "sequenceDiagram\nA->>B: Hello\n"]
        );
        assert!(
            lines.iter().flatten().any(|s| s.text.contains("main")),
            "ordinary code fences must remain visible"
        );
    }

    #[test]
    fn narrow_viewports_preserve_diagram_rows_and_fallback_source() {
        let original = "┌───────────┐\n│ 界 hello  │\n└───────────┘";
        let lines = render_with(
            "```mermaid\nflowchart LR\nA --> B\n```\n\nprose wraps here",
            |_| {
                original
                    .lines()
                    .map(|text| {
                        vec![Span {
                            text: text.into(),
                            style: SpanStyle {
                                diagram: true,
                                ..Default::default()
                            },
                        }]
                    })
                    .collect()
            },
            |destination, alt| source_image(destination, alt, None),
        );
        let wrapped = wrap_lines(lines, 5);
        let diagram = wrapped
            .iter()
            .filter(|l| l.iter().any(|s| s.style.diagram))
            .map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect::<Vec<_>>();
        assert_eq!(diagram.join("\n"), original);
        let fallback = source_diagram("flowchart TD\nA[", Some("invalid diagram"));
        let text = fallback
            .iter()
            .flatten()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("invalid diagram") && text.contains("A["));
    }

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
