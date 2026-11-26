//! Clickable links in terminal panes: file references, diff rows and URLs in
//! an agent's (or any program's) output.
//!
//! What counts as a link, in the order it is tried for the clicked cell:
//!
//! 1. A URL (`https://…`), opened with the system opener.
//! 2. A file reference that exists: `src/a.rs`, `./a.rs:12`, `src/a.rs:12:5`,
//!    `a.rs#L12`, `Update(src/a.rs)`, `File "a.py", line 3`, or a range of
//!    lines: `a.rs:12-20`, `a.rs#L12-L20`, `a.rs (lines 12-20)`,
//!    `lines 12-20 of a.rs`. A directory (`src/lsp/`) is a link too. How a
//!    token becomes a path (project root, `a/`/`b/` prefixes, bare file
//!    names) is the caller's `resolve`.
//! 3. A numbered row of a diff or file listing, as agents print them under a
//!    header naming the file (Claude Code's `⎿ Updated src/a.rs …` and the
//!    rows below it, Codex's `• Edited src/a.rs (+1 -1)`): `  12 -  old`,
//!    `  12 +  new`, `  12    same`. The file is the nearest path above.
//! 4. A body row of a unified diff (`git diff`): its new-file line number is
//!    counted from the `@@ -a,b +c,d @@` header, the file is the `+++ b/…`
//!    line above that.
//! 5. Failing all of those, a name defined in the workspace ([`symbol_at`]):
//!    a function, class, type, constant, field or variable, looked up in
//!    `crate::symbols`. Only names that read as code count (`open_file`,
//!    `FilePicker`, `render(`, `Editor::open`, or a type name like `Editor`),
//!    so ordinary words in prose are not links.
//!
//! The terminal grid is read as [`Row`]s of cells; rows the program
//! soft-wrapped are joined into one [`Line`] so a path split across rows is
//! still one link, and each character remembers the cell it came from so the
//! link can be underlined cell by cell.
use regex::Regex;
use std::path::PathBuf;
use std::sync::OnceLock;

/// One terminal row: each non-empty cell's text, column and width, and
/// whether the program soft-wrapped it into the next row.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub cells: Vec<(String, u16, u8)>,
    pub wrapped: bool,
}

impl Row {
    /// A row from plain text, one cell per character.
    #[cfg(test)]
    pub fn from_text(text: &str) -> Self {
        use unicode_width::UnicodeWidthChar;
        let mut col = 0u16;
        let cells = text
            .chars()
            .map(|c| {
                let w = c.width().unwrap_or(1).max(1) as u8;
                let cell = (c.to_string(), col, w);
                col += u16::from(w);
                cell
            })
            .collect();
        Row {
            cells,
            wrapped: false,
        }
    }
}

/// A logical line: soft-wrapped rows joined. `spans[i]` is the
/// `(row, col, width)` the `i`-th character of `text` was drawn at; `row` is
/// relative to the first row of the pane's view (history above it is
/// negative).
#[derive(Clone, Debug, Default)]
pub struct Line {
    pub text: String,
    pub spans: Vec<(isize, u16, u8)>,
}

impl Line {
    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }
}

/// Joins `rows` (the first one at view row `first_row`) into logical lines.
pub fn join_rows(rows: &[Row], first_row: isize) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut continuing = false;
    for (i, row) in rows.iter().enumerate() {
        if !continuing {
            out.push(Line::default());
        }
        let line = out.last_mut().unwrap();
        let r = first_row + i as isize;
        let mut next_col = 0u16;
        for (text, col, width) in &row.cells {
            // Blank cells between drawn ones are spaces in the text.
            while next_col < *col {
                line.text.push(' ');
                line.spans.push((r, next_col, 1));
                next_col += 1;
            }
            for c in text.chars() {
                line.text.push(c);
                line.spans.push((r, *col, *width));
            }
            next_col = col + u16::from(*width);
        }
        continuing = row.wrapped;
    }
    out
}

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A file, with a 1-based line and column when the reference had them.
    /// `end` is the last line of a range (`a.rs:12-20`).
    File {
        path: PathBuf,
        line: Option<usize>,
        col: Option<usize>,
        end: Option<usize>,
    },
    Url(String),
    /// A name defined in the workspace: where (best candidates first).
    Symbol {
        name: String,
        defs: Vec<crate::symbols::Def>,
    },
}

/// A run of cells on one view row, `start..end` columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub row: isize,
    pub start: u16,
    pub end: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: Target,
    pub segments: Vec<Segment>,
}

/// The result of looking for a link at a cell.
#[derive(Debug, PartialEq, Eq)]
pub enum Found {
    Link(Link),
    None,
    /// The clicked row is a diff row whose file is named further up than the
    /// lines given; the caller should retry with more history.
    NeedsHistory,
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\b(?:https?|file)://[^\s<>"'`│]+"#).unwrap())
}

/// A path-like token and an optional location suffix: `:12`, `:12:5`,
/// `#L12`, `(12)`, `(12,5)`, Python's `", line 12`, or a range of lines:
/// `:12-20`, `:12–20`, `:12..20`, `#L12-L20`, ` (lines 12-20)`, `, line 4`.
fn path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?x)
            (?P<path>
                (?:~|\.{1,2})?/?
                [A-Za-z0-9_@+\-][A-Za-z0-9_.@+\-~/]*
                [A-Za-z0-9_]
            )
            (?:
                :L?(?P<l1>\d+)(?::(?P<c1>\d+))?(?:(?:-|–|—|\.\.)L?(?P<e1>\d+)(?::\d+)?)?
              | \#L(?P<l2>\d+)(?:C\d+)?(?:-L?(?P<e2>\d+)(?:C\d+)?)?
              | \((?P<l3>\d+)(?:,\s*(?P<c3>\d+))?\)
              | "?,\ line\ (?P<l4>\d+)
              | ,?[\ \t]+\(?lines?[\ \t]+(?P<l5>\d+)
                (?:[\ \t]*(?:-|–|—|\.\.|to)[\ \t]*(?P<e5>\d+))?\)?
            )?"#,
        )
        .unwrap()
    })
}

/// A numbered row: optional indent/box-drawing, a line number, then
/// whitespace and an optional `+`/`-` marker.
fn numbered_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[\s│┃|⎿└├]*(?P<n>\d{1,7})(?:[ \t]+[+-]?|[ \t]*[+-])(?:[ \t]|$)").unwrap()
    })
}

/// `lines 12-20 of ` (or `in`/`from`) just before a path.
fn range_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\blines?[ \t]+(?P<l>\d+)(?:[ \t]*(?:-|–|—|\.\.|to)[ \t]*(?P<e>\d+))?[ \t]+(?:of|in|from)[ \t]+["'`]?$"#,
        )
        .unwrap()
    })
}

fn hunk_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^@@ -\d+(?:,\d+)? \+(?P<c>\d+)(?:,\d+)? @@").unwrap())
}

/// Whether a token is worth asking `resolve` about: it has a directory part
/// or a file extension, and is not a bare number or version string.
fn plausible_path(token: &str) -> bool {
    if token.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return false;
    }
    if token.contains('/') {
        return token.chars().any(|c| c.is_ascii_alphabetic());
    }
    // `name.ext`: an extension with a letter in it.
    token.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && !ext.is_empty() && ext.chars().any(|c| c.is_ascii_alphabetic())
    })
}

fn char_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    (text[..start].chars().count(), text[..end].chars().count())
}

/// A file reference found on a line: its character range, path, and the
/// 1-based line, column and range end it named.
#[derive(Debug)]
struct FileRef {
    start: usize,
    end: usize,
    path: PathBuf,
    line: Option<usize>,
    col: Option<usize>,
    last: Option<usize>,
}

/// `file:///abs/path[#L12]` as a file target, when the file exists.
/// Percent escapes (`%20`) are decoded.
fn file_url_target(url: &str, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Target> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let (path, frag) = match rest.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (rest, None),
    };
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&path[i + 1..i + 3], 16) {
                decoded.push(b);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8(decoded).ok()?;
    let resolved = resolve(&path)?;
    let line = frag
        .and_then(|f| f.strip_prefix('L'))
        .and_then(|f| f.split(['-', 'C']).next())
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|&n| n > 0);
    Some(Target::File {
        path: resolved,
        line,
        col: None,
        end: None,
    })
}

/// Quoted paths that contain spaces (`'/tmp/my file.txt'`, `"a b.rs":3`),
/// which the path pattern cannot match on its own.
fn quoted_refs(line: &Line, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<FileRef> {
    let mut out = Vec::new();
    let text = &line.text;
    for q in ['\'', '"', '`'] {
        let mut from = 0;
        while let Some(open) = text[from..].find(q).map(|i| from + i) {
            let Some(close) = text[open + 1..].find(q).map(|i| open + 1 + i) else {
                break;
            };
            let inner = &text[open + 1..close];
            from = close + 1;
            if !inner.contains(' ') || !inner.contains('/') || inner.contains('\n') {
                continue;
            }
            let Some(path) = resolve(inner) else {
                continue;
            };
            // An optional `:12` right after the closing quote.
            let after = &text[close + 1..];
            let digits: String = after
                .strip_prefix(':')
                .map(|a| a.chars().take_while(|c| c.is_ascii_digit()).collect())
                .unwrap_or_default();
            let line_no = digits.parse::<usize>().ok().filter(|&n| n > 0);
            let end = if line_no.is_some() {
                close + 2 + digits.len()
            } else {
                close + 1
            };
            let (s, e) = char_range(text, open, end);
            out.push(FileRef {
                start: s,
                end: e,
                path,
                line: line_no,
                col: None,
                last: None,
            });
        }
    }
    out
}

/// File references on `line`.
fn file_refs(line: &Line, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<FileRef> {
    let mut out = quoted_refs(line, resolve);
    let quoted: Vec<(usize, usize)> = out.iter().map(|r| (r.start, r.end)).collect();
    let urls: Vec<(usize, usize)> = url_re()
        .find_iter(&line.text)
        .map(|m| (m.start(), m.end()))
        .collect();
    for caps in path_re().captures_iter(&line.text) {
        let whole = caps.get(0).unwrap();
        let path = caps.name("path").unwrap();
        if urls
            .iter()
            .any(|&(s, e)| whole.start() < e && s < whole.end())
        {
            continue;
        }
        let token = path.as_str().trim_end_matches('.');
        if !plausible_path(token) {
            continue;
        }
        let Some(resolved) = resolve(token) else {
            continue;
        };
        let num = |a: &str, b: &str| {
            caps.name(a)
                .or_else(|| caps.name(b))
                .and_then(|m| m.as_str().parse::<usize>().ok())
        };
        let mut lnum = num("l1", "l2")
            .or_else(|| num("l3", "l4"))
            .or_else(|| num("l5", "l5"))
            .filter(|&n| n > 0);
        let col = num("c1", "c3").filter(|&n| n > 0);
        let mut last = num("e1", "e2").or_else(|| num("e5", "e5"));
        // The suffix is part of the link only when it parsed.
        let end = if lnum.is_some() {
            whole.end()
        } else {
            path.start() + token.len()
        };
        let mut start = path.start();
        // `lines 12-20 of src/a.rs`: the range comes first.
        if lnum.is_none() {
            if let Some(p) = range_prefix_re().captures(&line.text[..path.start()]) {
                lnum = p["l"].parse().ok().filter(|&n| n > 0);
                last = p.name("e").and_then(|m| m.as_str().parse().ok());
                start = p.get(0).unwrap().start();
            }
        }
        let last = last.filter(|&e| lnum.is_some_and(|l| e > l));
        let (s, e) = char_range(&line.text, start, end);
        if quoted.iter().any(|&(qs, qe)| s < qe && qs < e) {
            continue;
        }
        out.push(FileRef {
            start: s,
            end: e,
            path: resolved,
            line: lnum,
            col,
            last,
        });
    }
    out
}

/// Turns the character range `s..e` of `line` into per-row cell segments.
fn segments(line: &Line, s: usize, e: usize) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for &(row, col, width) in line.spans.get(s..e).unwrap_or(&[]) {
        let end = col + u16::from(width);
        match out.last_mut() {
            Some(seg) if seg.row == row && seg.end == col => seg.end = end,
            Some(seg) if seg.row == row && seg.start == col => {}
            _ => out.push(Segment {
                row,
                start: col,
                end,
            }),
        }
    }
    out
}

/// The non-blank extent of `line` after any leading box-drawing, as a
/// character range.
fn row_extent(line: &Line) -> Option<(usize, usize)> {
    let chars = line.chars();
    let start = chars
        .iter()
        .position(|c| !c.is_whitespace() && !"│┃|⎿└├".contains(*c))?;
    let end = chars.len()
        - chars
            .iter()
            .rev()
            .position(|c| !c.is_whitespace() && !"│┃|".contains(*c))
            .unwrap_or(0);
    (start < end).then_some((start, end))
}

/// Whether `text` is blank or only box-drawing.
fn is_blank(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_whitespace() || "│┃|╭╮╰╯─━┌┐└┘⎿".contains(c))
}

/// A line starting a new message or tool call in an agent's transcript
/// (`⏺ …`, `● …`, `• …`, `> …`) or a shell prompt: where a search upward for
/// a diff's file stops.
fn is_bullet(text: &str) -> bool {
    let t = text.trim_start_matches(|c: char| c.is_whitespace() || c == '│');
    t.starts_with(['⏺', '●', '•', '◆', '✻', '✶', '❯', '>', '$', '#', '›', '▌'])
}

/// The link at character `ch` of `lines[at]`.
pub fn link_at(
    lines: &[Line],
    at: usize,
    ch: usize,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> Found {
    let Some(line) = lines.get(at) else {
        return Found::None;
    };
    // 1. URLs. A `file://` URL naming a local file opens in the editor.
    for m in url_re().find_iter(&line.text) {
        let url = m
            .as_str()
            .trim_end_matches(|c: char| ".,;:!?)]}".contains(c));
        let (s, e) = char_range(&line.text, m.start(), m.start() + url.len());
        if (s..e).contains(&ch) {
            let target = file_url_target(url, resolve).unwrap_or(Target::Url(url.to_string()));
            return Found::Link(Link {
                target,
                segments: segments(line, s, e),
            });
        }
    }
    // 2. File references.
    for r in file_refs(line, resolve) {
        if (r.start..r.end).contains(&ch) {
            return Found::Link(Link {
                target: Target::File {
                    path: r.path,
                    line: r.line,
                    col: r.col,
                    end: r.last,
                },
                segments: segments(line, r.start, r.end),
            });
        }
    }
    let Some((s, e)) = row_extent(line) else {
        return Found::None;
    };
    if !(s..e).contains(&ch) {
        return Found::None;
    }
    // 3. A numbered diff/listing row under a header naming its file.
    if let Some(caps) = numbered_re().captures(&line.text) {
        let number: usize = caps["n"].parse().unwrap_or(0);
        for above in lines[..at].iter().rev() {
            if numbered_re().is_match(&above.text) || is_blank(&above.text) {
                continue;
            }
            if let Some(r) = file_refs(above, resolve)
                .into_iter()
                .find(|r| !r.path.is_dir())
            {
                return Found::Link(Link {
                    target: Target::File {
                        path: r.path,
                        line: (number > 0).then_some(number),
                        col: None,
                        end: None,
                    },
                    segments: segments(line, s, e),
                });
            }
            if is_bullet(&above.text) {
                return Found::None;
            }
        }
        return Found::NeedsHistory;
    }
    // 4. A unified diff body row.
    let first = line.text.chars().next();
    if matches!(first, Some('+' | '-' | ' '))
        && !line.text.starts_with("+++")
        && !line.text.starts_with("---")
    {
        let mut offset = 0usize;
        for (i, above) in lines[..at].iter().enumerate().rev() {
            if let Some(h) = hunk_re().captures(&above.text) {
                let start: usize = h["c"].parse().unwrap_or(1);
                let number = start + offset;
                return match diff_file(&lines[..i], resolve) {
                    Some(path) => Found::Link(Link {
                        target: Target::File {
                            path,
                            line: Some(number.max(1)),
                            col: None,
                            end: None,
                        },
                        segments: segments(line, s, e),
                    }),
                    None if i == 0 => Found::NeedsHistory,
                    None => Found::None,
                };
            }
            match above.text.chars().next() {
                Some('-') => {}
                Some('+' | ' ') => offset += 1,
                Some('\\') => {}
                None => offset += 1,
                _ => return Found::None,
            }
        }
        return Found::NeedsHistory;
    }
    Found::None
}

/// The name defined in the workspace at character `ch` of `lines[at]`, as a
/// link to its definitions (see the module docs, step 5). Several
/// definitions are narrowed by a qualifier (`Editor::open` prefers `open`s in
/// the files defining `Editor`) and then by the files the output mentions in
/// the 40 lines up to this one.
pub fn symbol_at(
    lines: &[Line],
    at: usize,
    ch: usize,
    index: &crate::symbols::Index,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<Link> {
    use crate::symbols::Kind;
    let line = lines.get(at)?;
    let chars = line.chars();
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    if !chars.get(ch).is_some_and(|&c| ident(c)) {
        return None;
    }
    let mut s = ch;
    while s > 0 && ident(chars[s - 1]) {
        s -= 1;
    }
    let mut e = ch + 1;
    while e < chars.len() && ident(chars[e]) {
        e += 1;
    }
    let name: String = chars[s..e].iter().collect();
    if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$') {
        return None;
    }
    let before: String = chars[..s].iter().collect();
    let after: String = chars[e..].iter().collect();
    // Part of a path or file name (`src/foo_bar`, `foo_bar.rs`): not a name.
    if before.ends_with(['/', '\\', '-']) || after.starts_with(['/', '\\']) {
        return None;
    }
    let qualifier = ["::", ".", "->"].iter().find_map(|sep| {
        let head = before.strip_suffix(sep)?;
        let q: String = head
            .chars()
            .rev()
            .take_while(|&c| ident(c))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        (!q.is_empty()).then_some(q)
    });
    let continues = |sep: &str| {
        after
            .strip_prefix(sep)
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
    };
    let qualified_right = continues("::") || continues(".");
    let call = after.starts_with('(') || after.starts_with("!(");
    let backticked = before.matches('`').count() % 2 == 1;
    let upper = |c: char| c.is_ascii_uppercase();
    let lower = |c: char| c.is_ascii_lowercase();
    let snake = name.trim_matches('_').contains('_');
    let camel = chars[s..e].windows(2).any(|w| lower(w[0]) && upper(w[1]));
    let code_like = snake || camel || call || qualifier.is_some() || qualified_right || backticked;
    // `Editor`: a capitalised word counts when it names a type or module.
    let cap_word =
        name.starts_with(upper) && name.chars().skip(1).all(|c| lower(c) || c.is_ascii_digit());
    // `MAX_LEN`, `TIMEOUT`: constants.
    let all_caps = name.len() >= 3
        && name
            .chars()
            .all(|c| upper(c) || c.is_ascii_digit() || c == '_');
    let q = qualifier.as_deref();
    let mut defs = index.lookup(&name, q, &[]);
    let kind = defs.first()?.kind;
    let eligible = code_like
        || (cap_word && matches!(kind, Kind::Type | Kind::Module))
        || (all_caps && matches!(kind, Kind::Constant | Kind::Variable | Kind::Type));
    if !eligible {
        return None;
    }
    if defs.len() > 1 {
        let mut context: Vec<PathBuf> = Vec::new();
        for l in lines[at.saturating_sub(40)..=at].iter().rev() {
            for r in file_refs(l, resolve) {
                if !r.path.is_dir() && !context.contains(&r.path) {
                    context.push(r.path);
                }
            }
        }
        if !context.is_empty() {
            defs = index.lookup(&name, q, &context);
        }
    }
    Some(Link {
        target: Target::Symbol { name, defs },
        segments: segments(line, s, e),
    })
}

/// The new-side file of the diff whose hunk header follows `lines`.
fn diff_file(lines: &[Line], resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let mut old: Option<String> = None;
    for above in lines.iter().rev() {
        let t = above.text.trim_end();
        if let Some(p) = t.strip_prefix("+++ ") {
            if p != "/dev/null" {
                return resolve(p.trim());
            }
            continue;
        }
        if let Some(p) = t.strip_prefix("--- ") {
            old = Some(p.trim().to_string());
            continue;
        }
        if let Some(rest) = t.strip_prefix("diff --git ") {
            if let Some((_, b)) = rest.rsplit_once(" b/") {
                return resolve(&format!("b/{b}"));
            }
            return old.and_then(|o| resolve(&o));
        }
    }
    old.and_then(|o| resolve(&o))
}

/// Rows of history read above the view when a diff row's file is named
/// further up: a little for hover (it runs on every pointer move), a lot for
/// a click.
const HOVER_HISTORY: usize = 200;
pub(crate) const CLICK_HISTORY: usize = 3000;

/// The link under the mouse in a terminal pane, underlined while hovered.
/// Segments are rows of the view as it was when found (`offset` lines into
/// history); a scroll or new output finds it again from `pointer`.
#[derive(Clone, Debug)]
pub struct Hover {
    pub terminal: u64,
    pub offset: usize,
    pub segments: Vec<Segment>,
    pub pointer: (usize, usize, crossterm::event::KeyModifiers),
}

impl crate::editor::Editor {
    /// The link at screen cell `(x, y)` in a terminal pane, as
    /// `(pane, link)`: `None` off a terminal's program area or off a link.
    /// `history` bounds how far up a diff row's file is looked for.
    pub(crate) fn terminal_link_at(
        &self,
        cols: usize,
        rows: usize,
        x: usize,
        y: usize,
        history: usize,
    ) -> Option<(usize, Link)> {
        let rects = self.pane_rects(cols, rows);
        let pane = rects
            .iter()
            .position(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)?;
        let id = self.windows.get(pane)?.terminal?;
        let body = crate::render::terminal_body_rect(self, rects[pane]);
        if x < body.x || x >= body.x + body.width || y < body.y || y >= body.y + body.height {
            return None;
        }
        let (cx, cy) = ((x - body.x) as u16, (y - body.y) as isize);
        let pty = self.terminals.iter().find(|p| p.id == id)?;
        let base = pty.cwd.clone();
        let resolve = |t: &str| self.resolve_link_path_from(t, Some(&base));
        let mut above = 0;
        loop {
            let (cells, n) = pty.link_rows(above);
            let lines = join_rows(&cells, -(n as isize));
            let (at, ch) = lines.iter().enumerate().find_map(|(i, l)| {
                l.spans
                    .iter()
                    .position(|&(r, c, w)| r == cy && cx >= c && cx < c + u16::from(w))
                    .map(|ch| (i, ch))
            })?;
            match link_at(&lines, at, ch, &resolve) {
                Found::Link(link) => return Some((pane, link)),
                Found::NeedsHistory if above < history && n == above => above = history,
                _ => {
                    return symbol_at(&lines, at, ch, &self.symbols, &resolve).map(|l| (pane, l));
                }
            }
        }
    }

    /// Turns a token from terminal output into an existing file: absolute,
    /// `~/`, or relative to the project root (where agents run), with git's
    /// `a/`/`b/` prefixes dropped; failing that, a bare name or partial path
    /// that ends exactly one project file (`pty.rs` for `src/pty.rs`), or
    /// one that is open in a buffer. A token with a directory part may also
    /// name a directory (`src/lsp`).
    ///
    /// `base` (the folder the program printing the token started in) is
    /// tried before the project root for a relative token.
    pub(crate) fn resolve_link_path_from(
        &self,
        token: &str,
        base: Option<&std::path::Path>,
    ) -> Option<PathBuf> {
        use std::path::Path;
        let token = token.trim_matches('"');
        let bases: Vec<&Path> = base
            .filter(|b| *b != self.project_root.as_path())
            .into_iter()
            .chain(std::iter::once(self.project_root.as_path()))
            .collect();
        let expand_in = |t: &str, root: &Path| -> Option<PathBuf> {
            if let Some(rest) = t.strip_prefix("~/") {
                dirs::home_dir().map(|h| h.join(rest))
            } else if Path::new(t).is_absolute() {
                Some(PathBuf::from(t))
            } else {
                Some(root.join(t.strip_prefix("./").unwrap_or(t)))
            }
        };
        let expand = |t: &str| expand_in(t, &self.project_root);
        let stripped = token
            .strip_prefix("a/")
            .or_else(|| token.strip_prefix("b/"));
        for root in &bases {
            for t in std::iter::once(token).chain(stripped) {
                if let Some(p) = expand_in(t, root).filter(|p| p.is_file()) {
                    return Some(p);
                }
            }
        }
        if token.contains('/') {
            if let Some(p) = expand(token).filter(|p| p.is_dir() && *p != self.project_root) {
                return Some(p);
            }
        }
        if token.starts_with(['/', '~', '.']) {
            return None;
        }
        let suffix = format!("/{token}");
        let hits: Vec<&String> = self
            .all_files
            .iter()
            .filter(|f| f.as_str() == token || f.ends_with(&suffix))
            .collect();
        let pick = match hits.as_slice() {
            [one] => Some(self.project_root.join(one)),
            [] => None,
            many => many
                .iter()
                .map(|f| self.project_root.join(f))
                .find(|p| self.buffers.iter().any(|b| b.path.as_deref() == Some(p))),
        };
        pick.or_else(|| {
            // Not scanned yet, or outside the project: an open buffer.
            self.buffers
                .iter()
                .filter_map(|b| b.path.clone())
                .find(|p| p.ends_with(token))
        })
        .filter(|p| p.is_file())
    }

    /// Whether a plain click in terminal pane `pane` may open a link. Agent
    /// panes always take link clicks; another program that asked for mouse
    /// reports (lazygit, htop) gets plain clicks, so it takes Ctrl-click.
    pub(crate) fn terminal_link_clickable(
        &self,
        pane: usize,
        modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        let Some(pty) = self
            .windows
            .get(pane)
            .and_then(|w| w.terminal)
            .and_then(|id| self.terminals.iter().find(|p| p.id == id))
        else {
            return false;
        };
        pty.agent_kind.is_some()
            || modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            || !pty.mouse_state().0
    }

    /// The editing pane a link from terminal pane `from` opens in: the
    /// nearest plain buffer pane (sharing an edge with it if any), preferring
    /// the pane focused before the terminal, then the larger.
    pub(crate) fn link_target_window(
        &self,
        from: usize,
        cols: usize,
        rows: usize,
    ) -> Option<usize> {
        let rects = self.pane_rects(cols, rows);
        let origin = *rects.get(from)?;
        let gap = |a0: usize, a1: usize, b0: usize, b1: usize| {
            if b0 >= a1 {
                b0 - a1
            } else {
                a0.saturating_sub(b1)
            }
        };
        self.windows
            .iter()
            .enumerate()
            .filter(|(i, w)| {
                *i != from
                    && w.terminal.is_none()
                    && !w.file_tree
                    && !w.outline
                    && !w.review_toc
                    && !w.preview
            })
            .filter_map(|(i, _)| rects.get(i).map(|r| (i, *r)))
            .min_by_key(|(i, r)| {
                let distance = gap(origin.x, origin.x + origin.width, r.x, r.x + r.width)
                    + gap(origin.y, origin.y + origin.height, r.y, r.y + r.height);
                (
                    distance,
                    self.prev_window != Some(*i),
                    std::cmp::Reverse(r.width * r.height),
                )
            })
            .map(|(i, _)| i)
    }

    /// Opens `link` from terminal pane `from`: a URL with the system
    /// opener; a file in the adjacent editing pane (one is split off the
    /// terminal when there is none), cursor on the line, which is centred.
    /// `Ctrl-W p` goes back to the terminal.
    pub(crate) fn open_terminal_link(&mut self, from: usize, link: Link, cols: usize, rows: usize) {
        let (path, line, col, end) = match link.target {
            Target::Url(url) => {
                crate::normal::open_with_system(self, &url);
                return;
            }
            Target::File {
                path,
                line,
                col,
                end,
            } => (path, line, col, end),
            Target::Symbol { name, defs } => match defs.as_slice() {
                [one] => (one.path.clone(), Some(one.line), Some(one.col), None),
                _ => {
                    self.show_symbol_definitions(from, &name, &defs, cols, rows);
                    return;
                }
            },
        };
        let rel = path
            .strip_prefix(&self.project_root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if path.is_dir() {
            if self.mode == crate::mode::Mode::Terminal {
                self.mode = crate::mode::Mode::Normal;
            }
            self.tree_reveal(&path);
            self.set_message(format!("Revealed {rel}/ in the file tree"));
            return;
        }
        let from_name = self
            .windows
            .get(from)
            .and_then(|w| w.terminal)
            .map(|id| self.terminal_name(id))
            .unwrap_or_default();
        if !self.focus_link_target(from, cols, rows) {
            return;
        }
        let l = line.unwrap_or(1).saturating_sub(1);
        let c = col.unwrap_or(1).saturating_sub(1);
        self.jump_to(path.clone(), l, c);
        let mut selected = false;
        if line.is_some() {
            let last = self.buf().line_count().saturating_sub(1);
            let l = l.min(last);
            let c = if col.is_some() {
                c
            } else {
                self.buf().first_non_blank(l)
            };
            self.set_cursor(l, c);
            let height = self
                .pane_rects(cols, rows)
                .get(self.active_window)
                .map(|r| r.height.saturating_sub(1))
                .unwrap_or(self.screen_rows)
                .max(1);
            // A range is selected linewise (ready for `y`, `:'<,'>ai`), and
            // centred when it fits, else shown from its first line.
            let span = end
                .map(|e| e.saturating_sub(1).min(last))
                .filter(|&e| e > l);
            let top = match span {
                Some(e) if e - l < height => l.saturating_sub((height - (e - l)) / 2),
                Some(_) => l.saturating_sub(2),
                None => l.saturating_sub(height / 2),
            };
            if let Some(e) = span {
                self.visual_anchor = Some((e, 0));
                self.mode = crate::mode::Mode::Visual(crate::mode::VisualKind::Line);
                selected = true;
            }
            self.buf_mut().top_line = top;
            self.buf_mut().top_wrap = 0;
        }
        let at = match (line, end) {
            (Some(l), Some(e)) if selected => format!(":{l}-{e}"),
            (Some(l), _) => format!(":{l}"),
            _ => String::new(),
        };
        self.set_message(if selected {
            format!("Opened {rel}{at} from {from_name}, lines selected (Esc clears; Ctrl-W p goes back)")
        } else {
            format!("Opened {rel}{at} from {from_name} (Ctrl-W p goes back)")
        });
    }

    /// Focuses the editing pane a link from terminal pane `from` opens in
    /// (`link_target_window`), splitting one off the terminal when there is
    /// none, in Normal mode with the terminal as the pane `Ctrl-W p` returns
    /// to. Returns false when no pane could be had (the pane limit).
    fn focus_link_target(&mut self, from: usize, cols: usize, rows: usize) -> bool {
        if matches!(
            self.mode,
            crate::mode::Mode::Insert | crate::mode::Mode::Visual(_)
        ) {
            self.visual_anchor = None;
            self.enter_normal();
        }
        match self.link_target_window(from, cols, rows) {
            Some(idx) => {
                // Make the terminal the pane `Ctrl-W p` returns to.
                if self.active_window != from {
                    self.focus_pane_buffer(from);
                }
                self.focus_window(idx);
            }
            None => {
                self.focus_pane_buffer(from);
                let before = self.windows.len();
                self.split_window(true, false);
                if self.windows.len() <= before {
                    return false;
                }
                if let Some(w) = self.windows.get_mut(self.active_window) {
                    w.terminal = None;
                }
                self.prev_window = Some(from);
            }
        }
        self.mode = crate::mode::Mode::Normal;
        true
    }

    /// A name with several definitions: lists them (kind, location and
    /// source line) for the editing pane next to terminal pane `from`;
    /// Enter opens one there.
    fn show_symbol_definitions(
        &mut self,
        from: usize,
        name: &str,
        defs: &[crate::symbols::Def],
        cols: usize,
        rows: usize,
    ) {
        if !self.focus_link_target(from, cols, rows) {
            return;
        }
        let mut cache: std::collections::HashMap<&std::path::Path, Vec<String>> =
            Default::default();
        let entries = defs
            .iter()
            .map(|d| {
                let source = cache.entry(&d.path).or_insert_with(|| {
                    std::fs::read_to_string(&d.path)
                        .map(|t| t.lines().map(str::to_string).collect())
                        .unwrap_or_default()
                });
                let text = source
                    .get(d.line.saturating_sub(1))
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default();
                crate::results::Entry::location(
                    d.path.clone(),
                    d.line.saturating_sub(1),
                    d.col.saturating_sub(1),
                    format!("{} · {text}", d.kind.label()),
                )
            })
            .collect();
        self.show_results(crate::results::Results::new(
            format!("Definitions of {name} — Enter opens"),
            entries,
        ));
    }

    /// Records the pointer at `(x, y)` and underlines the terminal link
    /// under it, if any (and if a click there would open it).
    pub(crate) fn hover_terminal_link(
        &mut self,
        cols: usize,
        rows: usize,
        x: usize,
        y: usize,
        modifiers: crossterm::event::KeyModifiers,
    ) {
        let found = self
            .terminal_link_at(cols, rows, x, y, HOVER_HISTORY)
            .filter(|(pane, _)| self.terminal_link_clickable(*pane, modifiers));
        self.terminal_hover = found.and_then(|(pane, link)| {
            let id = self.windows.get(pane)?.terminal?;
            let offset = self.terminals.iter().find(|p| p.id == id)?.scroll_offset();
            Some(Hover {
                terminal: id,
                offset,
                segments: link.segments,
                pointer: (x, y, modifiers),
            })
        });
    }

    /// Finds the hovered link again after the view changed under a still
    /// pointer (new output, a scroll).
    pub(crate) fn refresh_terminal_hover(&mut self) {
        if let Some(Hover {
            pointer: (x, y, m), ..
        }) = self.terminal_hover
        {
            let (cols, rows) = (self.screen_cols, self.terminal_rows);
            self.hover_terminal_link(cols, rows, x, y, m);
        }
    }

    /// A click at `(x, y)`: opens the terminal link there when there is one
    /// a click may open. Returns whether it did.
    pub(crate) fn click_terminal_link(
        &mut self,
        cols: usize,
        rows: usize,
        x: usize,
        y: usize,
        modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        let Some((pane, link)) = self.terminal_link_at(cols, rows, x, y, CLICK_HISTORY) else {
            return false;
        };
        if !self.terminal_link_clickable(pane, modifiers) {
            return false;
        }
        self.terminal_hover = None;
        self.open_terminal_link(pane, link, cols, rows);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Resolves the tokens of a fake project: `src/a.rs`, `src/b.rs`,
    /// `x.py`; `a/`/`b/` prefixes are stripped as git prints them.
    fn resolve(token: &str) -> Option<PathBuf> {
        let t = token
            .strip_prefix("a/")
            .or_else(|| token.strip_prefix("b/"))
            .unwrap_or(token);
        let t = t.strip_prefix("./").unwrap_or(t);
        let t = match t {
            "a.rs" => "src/a.rs",
            other => other,
        };
        ["src/a.rs", "src/b.rs", "x.py"]
            .contains(&t)
            .then(|| Path::new("/p").join(t))
    }

    fn lines(text: &str) -> Vec<Line> {
        let rows: Vec<Row> = text.lines().map(Row::from_text).collect();
        join_rows(&rows, 0)
    }

    /// The link at the first occurrence of `needle` on the line containing it.
    fn at(text: &str, needle: &str) -> Found {
        let ls = lines(text);
        let (i, l) = ls
            .iter()
            .enumerate()
            .find(|(_, l)| l.text.contains(needle))
            .expect("needle present");
        let ch = l.text[..l.text.find(needle).unwrap()].chars().count();
        link_at(&ls, i, ch, &resolve)
    }

    fn file(found: Found) -> (String, Option<usize>, Option<usize>) {
        match found {
            Found::Link(Link {
                target: Target::File {
                    path, line, col, ..
                },
                ..
            }) => (path.display().to_string(), line, col),
            other => panic!("expected a file link, got {other:?}"),
        }
    }

    #[test]
    fn file_references_with_locations() {
        assert_eq!(
            file(at("see src/a.rs for it", "src/a")),
            ("/p/src/a.rs".into(), None, None)
        );
        assert_eq!(
            file(at("error at src/a.rs:12:5: boom", "src/a")),
            ("/p/src/a.rs".into(), Some(12), Some(5))
        );
        assert_eq!(
            file(at("`src/b.rs:40`.", "src/b")),
            ("/p/src/b.rs".into(), Some(40), None)
        );
        assert_eq!(
            file(at("src/a.rs#L7", "src/a")),
            ("/p/src/a.rs".into(), Some(7), None)
        );
        assert_eq!(
            file(at("in a.rs(3,9)", "a.rs")),
            ("/p/src/a.rs".into(), Some(3), Some(9))
        );
        assert_eq!(
            file(at(r#"  File "x.py", line 3, in main"#, "x.py")),
            ("/p/x.py".into(), Some(3), None)
        );
        assert_eq!(
            file(at("⏺ Update(src/a.rs)", "src/a")),
            ("/p/src/a.rs".into(), None, None)
        );
        // Clicking the `:12` suffix is still the link.
        assert_eq!(file(at("src/a.rs:12 here", ":12")).1, Some(12));
    }

    #[test]
    fn only_existing_paths_and_not_numbers_are_links() {
        assert_eq!(at("see src/zzz.rs here", "src/zzz"), Found::None);
        assert_eq!(at("version 1.2.3 out", "1.2.3"), Found::None);
        assert_eq!(at("plain words here", "words"), Found::None);
    }

    #[test]
    fn link_segments_cover_exactly_the_reference() {
        let ls = lines("ab src/a.rs:4 cd");
        let Found::Link(l) = link_at(&ls, 0, 5, &resolve) else {
            panic!()
        };
        assert_eq!(
            l.segments,
            vec![Segment {
                row: 0,
                start: 3,
                end: 13
            }]
        );
    }

    #[test]
    fn a_path_soft_wrapped_across_rows_is_one_link() {
        let mut r1 = Row::from_text("open src/a");
        r1.wrapped = true;
        let r2 = Row::from_text(".rs:9 now");
        let ls = join_rows(&[r1, r2], 3);
        assert_eq!(ls.len(), 1);
        let Found::Link(l) = link_at(&ls, 0, 13, &resolve) else {
            panic!()
        };
        assert_eq!(
            l.target,
            Target::File {
                path: "/p/src/a.rs".into(),
                line: Some(9),
                col: None,
                end: None,
            }
        );
        assert_eq!(
            l.segments,
            vec![
                Segment {
                    row: 3,
                    start: 5,
                    end: 10
                },
                Segment {
                    row: 4,
                    start: 0,
                    end: 5
                }
            ]
        );
    }

    #[test]
    fn urls_open_as_urls() {
        let ls = lines("PR: https://github.com/o/r/pull/12.");
        let Found::Link(l) = link_at(&ls, 0, 10, &resolve) else {
            panic!()
        };
        assert_eq!(
            l.target,
            Target::Url("https://github.com/o/r/pull/12".into())
        );
    }

    const CLAUDE: &str = "\
⏺ Update(src/a.rs)
  ⎿  Updated src/a.rs with 1 addition and 1 removal
       10    fn main() {
       11 -      println!(\"hi\");
       11 +      println!(\"hello\");
       12    }
";

    #[test]
    fn claude_code_diff_rows_open_their_file_at_the_line() {
        assert_eq!(
            file(at(CLAUDE, "11 +")),
            ("/p/src/a.rs".into(), Some(11), None)
        );
        assert_eq!(
            file(at(CLAUDE, "10 ")),
            ("/p/src/a.rs".into(), Some(10), None)
        );
        assert_eq!(
            file(at(CLAUDE, "hello")),
            ("/p/src/a.rs".into(), Some(11), None)
        );
        let ls = lines(CLAUDE);
        let Found::Link(l) = link_at(&ls, 4, 20, &resolve) else {
            panic!()
        };
        // The whole row (less its indent) is the link.
        assert_eq!(
            l.segments,
            vec![Segment {
                row: 4,
                start: 7,
                end: 35
            }]
        );
    }

    #[test]
    fn codex_edit_blocks_and_permission_boxes() {
        let codex = "• Edited src/b.rs (+1 -1)\n    3   a\n    4 - b\n    4 + c\n";
        assert_eq!(
            file(at(codex, "4 + c")),
            ("/p/src/b.rs".into(), Some(4), None)
        );
        let boxed = "│ │ src/a.rs                │ │\n│ │                         │ │\n│ │  7 -  x                 │ │\n";
        assert_eq!(
            file(at(boxed, "7 -")),
            ("/p/src/a.rs".into(), Some(7), None)
        );
    }

    #[test]
    fn a_numbered_row_with_no_file_above_is_not_a_link() {
        let text = "⏺ Here is a list\n  3   things\n";
        assert_eq!(at(text, "3   things"), Found::None);
        // No header within the given lines at all: ask for more history.
        assert_eq!(at("  12 + x\n", "12"), Found::NeedsHistory);
    }

    const GIT: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -20,4 +20,5 @@ fn f() {
 ctx20
-gone
+new21
+new22
 ctx23
";

    #[test]
    fn unified_diff_rows_count_lines_from_the_hunk_header() {
        assert_eq!(
            file(at(GIT, "ctx20")),
            ("/p/src/a.rs".into(), Some(20), None)
        );
        assert_eq!(
            file(at(GIT, "new21")),
            ("/p/src/a.rs".into(), Some(21), None)
        );
        assert_eq!(
            file(at(GIT, "new22")),
            ("/p/src/a.rs".into(), Some(22), None)
        );
        assert_eq!(
            file(at(GIT, "ctx23")),
            ("/p/src/a.rs".into(), Some(23), None)
        );
        // The `-` row points at where it was removed.
        assert_eq!(
            file(at(GIT, "gone")),
            ("/p/src/a.rs".into(), Some(21), None)
        );
        // The headers themselves are file links.
        let ls = lines(GIT);
        assert_eq!(file(link_at(&ls, 3, 6, &resolve)).0, "/p/src/a.rs");
    }

    #[test]
    fn a_hunk_cut_off_at_the_top_needs_history() {
        let cut = "@@ -1 +1 @@\n+x\n";
        assert_eq!(at(cut, "+x"), Found::NeedsHistory);
        let no_hunk = "+x\n";
        assert_eq!(at(no_hunk, "+x"), Found::NeedsHistory);
    }

    #[test]
    fn wide_characters_keep_their_cells() {
        let ls = lines("日本 src/a.rs");
        let Found::Link(l) = link_at(&ls, 0, 4, &resolve) else {
            panic!()
        };
        assert_eq!(
            l.segments,
            vec![Segment {
                row: 0,
                start: 5,
                end: 13
            }]
        );
    }

    /// The range end of the file link at `needle`.
    fn range(text: &str, needle: &str) -> (Option<usize>, Option<usize>) {
        match at(text, needle) {
            Found::Link(Link {
                target: Target::File { line, end, .. },
                ..
            }) => (line, end),
            other => panic!("expected a file link, got {other:?}"),
        }
    }

    #[test]
    fn line_ranges_in_every_spelling() {
        assert_eq!(
            range("see src/a.rs:12-20 now", "src/a.rs"),
            (Some(12), Some(20))
        );
        assert_eq!(
            range("see src/a.rs:12–20", "src/a.rs"),
            (Some(12), Some(20))
        );
        assert_eq!(
            range("see src/a.rs:12..20", "src/a.rs"),
            (Some(12), Some(20))
        );
        assert_eq!(
            range("see src/a.rs:L12-L20", "src/a.rs"),
            (Some(12), Some(20))
        );
        assert_eq!(
            range("src/a.rs:3:5-7:1 changed", "src/a.rs"),
            (Some(3), Some(7))
        );
        assert_eq!(range("see src/a.rs#L3-L7", "src/a.rs"), (Some(3), Some(7)));
        assert_eq!(range("src/a.rs#L3C2-L7C9", "src/a.rs"), (Some(3), Some(7)));
        assert_eq!(
            range("Read src/a.rs (lines 4-9)", "src/a.rs"),
            (Some(4), Some(9))
        );
        assert_eq!(
            range("Read src/a.rs, lines 4 to 9", "src/a.rs"),
            (Some(4), Some(9))
        );
        assert_eq!(range("Read src/a.rs line 4", "src/a.rs"), (Some(4), None));
        // The range may come first; clicking the phrase opens it too.
        assert_eq!(
            range("see lines 2-3 of src/a.rs here", "src/a.rs"),
            (Some(2), Some(3))
        );
        assert_eq!(
            range("see lines 2-3 of src/a.rs here", "lines 2"),
            (Some(2), Some(3))
        );
        assert_eq!(
            range("see line 5 in `src/a.rs`", "src/a.rs"),
            (Some(5), None)
        );
        // A single line, or an end before the start, is no range.
        assert_eq!(range("src/a.rs:12", "src/a.rs"), (Some(12), None));
        assert_eq!(range("src/a.rs:20-12", "src/a.rs"), (Some(20), None));
        // "- 3 more" after a location is prose, not a range.
        assert_eq!(range("src/a.rs:12 - 3 more", "src/a.rs"), (Some(12), None));
    }

    /// An index over a fake project: `Editor`/`open_file`/`ai_split` in
    /// `e.rs`, `twice` in both `t1.rs` and `t2.rs`, `MAX_LEN` in `e.rs`.
    fn symbol_project() -> (PathBuf, crate::symbols::Index) {
        let dir = std::env::temp_dir().join(format!(
            "vaayu-symlinks-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("e.rs"),
            "pub struct Editor {\n    pub ai_split: String,\n}\nimpl Editor {\n    pub fn open_file(&self) {}\n    fn render(&self) {}\n}\nconst MAX_LEN: usize = 3;\n",
        )
        .unwrap();
        std::fs::write(dir.join("t1.rs"), "fn twice() {}\n").unwrap();
        std::fs::write(dir.join("t2.rs"), "fn twice() {}\n").unwrap();
        let index =
            crate::symbols::Index::build(&dir, &["e.rs".into(), "t1.rs".into(), "t2.rs".into()]);
        (dir, index)
    }

    /// The definitions a click at `needle` (offset `dx`) resolves to, as
    /// `(file name, line)`.
    fn symbol(
        index: &crate::symbols::Index,
        dir: &Path,
        text: &str,
        needle: &str,
        dx: usize,
    ) -> Option<Vec<(String, usize)>> {
        let ls = lines(text);
        let (i, l) = ls
            .iter()
            .enumerate()
            .find(|(_, l)| l.text.contains(needle))
            .expect("needle present");
        let ch = l.text[..l.text.find(needle).unwrap()].chars().count() + dx;
        let resolve = |t: &str| Some(dir.join(t)).filter(|p| p.is_file());
        let link = symbol_at(&ls, i, ch, index, &resolve)?;
        let Target::Symbol { defs, .. } = link.target else {
            panic!("expected a symbol link")
        };
        Some(
            defs.iter()
                .map(|d| {
                    (
                        d.path.file_name().unwrap().to_string_lossy().into_owned(),
                        d.line,
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn code_names_link_to_their_definitions_and_prose_does_not() {
        let (dir, index) = symbol_project();
        let one = |file: &str, line| Some(vec![(file.to_string(), line)]);
        let s = |text: &str, needle: &str, dx| symbol(&index, &dir, text, needle, dx);
        assert_eq!(s("Call open_file to load", "open_file", 2), one("e.rs", 5));
        assert_eq!(s("the Editor keeps state", "Editor", 0), one("e.rs", 1));
        assert_eq!(s("it reads ai_split.", "ai_split", 0), one("e.rs", 2));
        assert_eq!(s("set MAX_LEN higher", "MAX_LEN", 1), one("e.rs", 8));
        // A plain lowercase word counts only as a call or qualified name.
        assert_eq!(s("we render it", "render", 0), None);
        assert_eq!(s("then render() it", "render", 0), one("e.rs", 6));
        assert_eq!(s("Editor::render does it", "render", 0), one("e.rs", 6));
        assert_eq!(s("in `render` now", "render", 0), one("e.rs", 6));
        // Undefined names, numbers and parts of paths are not links.
        assert_eq!(s("call missing_fn now", "missing_fn", 0), None);
        assert_eq!(s("at 1234 now", "1234", 0), None);
        assert_eq!(s("see src/open_file here", "open_file", 0), None);
        // Several definitions: all of them, unless a file named nearby
        // narrows it.
        assert_eq!(
            s("both twice() here", "twice", 0),
            Some(vec![("t1.rs".into(), 1), ("t2.rs".into(), 1)])
        );
        assert_eq!(
            s("Updated t2.rs\n  calls twice() now", "twice", 0),
            one("t2.rs", 1)
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn file_urls_and_quoted_paths_with_spaces_open_in_the_editor() {
        let resolve = |t: &str| {
            ["/abs/far.rs", "/abs/with space.txt"]
                .contains(&t)
                .then(|| PathBuf::from(t))
        };
        let ls = lines("see file:///abs/far.rs#L4 and '/abs/with space.txt':3 now");
        let at = |needle: &str| ls[0].text.find(needle).unwrap() + 2;
        match link_at(&ls, 0, at("file://"), &resolve) {
            Found::Link(Link {
                target: Target::File { path, line, .. },
                ..
            }) => assert_eq!((path, line), (PathBuf::from("/abs/far.rs"), Some(4))),
            other => panic!("{other:?}"),
        }
        match link_at(&ls, 0, at("with"), &resolve) {
            Found::Link(Link {
                target: Target::File { path, line, .. },
                ..
            }) => assert_eq!(
                (path, line),
                (PathBuf::from("/abs/with space.txt"), Some(3))
            ),
            other => panic!("{other:?}"),
        }
        // Percent escapes decode; a file that is not there stays a URL.
        let ls = lines("file:///abs/with%20space.txt file:///abs/gone.rs");
        assert!(matches!(
            link_at(&ls, 0, 10, &resolve),
            Found::Link(Link {
                target: Target::File { .. },
                ..
            })
        ));
        let gone = ls[0].text.find("gone").unwrap();
        assert!(matches!(
            link_at(&ls, 0, gone, &resolve),
            Found::Link(Link {
                target: Target::Url(_),
                ..
            })
        ));
    }
}
