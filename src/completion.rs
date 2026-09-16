use crate::buffer::Buffer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Buffer,
    Lsp,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub insert_text: String,
    pub detail: Option<String>,
    pub source: Source,
    pub edit: Option<serde_json::Value>,
    pub raw: Option<serde_json::Value>,
    pub snippet: bool,
    pub additional: Vec<serde_json::Value>,
}

pub struct CompletionState {
    /// (line, col) of the first character of the word being completed.
    pub start: (usize, usize),
    pub items: Vec<Item>,
    pub selected: usize,
    /// Bumped every time the popup is (re)triggered at a new position, so a
    /// slow LSP response can be dropped if the user has moved on by the
    /// time it arrives.
    pub request_id: u64,
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word-prefix ending at (line, col), i.e. the identifier characters
/// immediately before the cursor.
pub fn word_prefix(buf: &Buffer, line: usize, col: usize) -> (usize, String) {
    let text: Vec<char> = buf.line_text(line).chars().collect();
    let mut start = col.min(text.len());
    while start > 0 && is_word_char(text[start - 1]) {
        start -= 1;
    }
    let prefix: String = text[start..col.min(text.len())].iter().collect();
    (start, prefix)
}

/// Scans the whole buffer for identifier-like words matching `prefix`
/// (case-sensitive, prefix match, excluding the word currently being typed),
/// nearest-line-first so locally relevant names surface first.
pub fn buffer_word_candidates(buf: &Buffer, prefix: &str, cursor_line: usize) -> Vec<Item> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for line in 0..buf.line_count() {
        let text = buf.line_text(line);
        let mut word = String::new();
        for c in text.chars().chain(std::iter::once(' ')) {
            if is_word_char(c) {
                word.push(c);
            } else {
                if word.len() > prefix.len()
                    && word.starts_with(prefix)
                    && seen.insert(word.clone())
                {
                    let dist = line.abs_diff(cursor_line);
                    scored.push((dist, word.clone()));
                }
                word.clear();
            }
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(50)
        .map(|(_, w)| Item {
            label: w.clone(),
            insert_text: w,
            detail: None,
            source: Source::Buffer,
            edit: None,
            additional: Vec::new(),
            raw: None,
            snippet: false,
        })
        .collect()
}

/// Lazily index a bounded neighborhood; entry and large-file typing avoid
/// allocating one map per line in the entire source buffer.
pub struct WordIndex {
    id: u64,
    lines: std::collections::BTreeMap<usize, std::collections::HashSet<String>>,
    line_count: usize,
    last_line: usize,
}
impl WordIndex {
    pub fn new(buf: &Buffer) -> Self {
        Self {
            id: buf.id,
            lines: Default::default(),
            line_count: buf.line_count(),
            last_line: buf.cursor_line,
        }
    }
    pub fn candidates(&mut self, buf: &Buffer, prefix: &str, line: usize) -> Vec<Item> {
        if self.id != buf.id || self.line_count != buf.line_count() {
            *self = Self::new(buf);
        }
        self.lines.retain(|l, _| l.abs_diff(line) <= 200);
        for l in line.saturating_sub(200)..(line + 201).min(buf.line_count()) {
            self.lines
                .entry(l)
                .or_insert_with(|| words(&buf.line_text(l)));
        }
        for l in [self.last_line, line] {
            if l < buf.line_count() {
                self.lines.insert(l, words(&buf.line_text(l)));
            }
        }
        self.last_line = line;
        let (_, pre) = word_prefix(buf, line, buf.cursor_col);
        let line_text = buf.line_text(line);
        let suffix: String = line_text
            .chars()
            .skip(buf.cursor_col)
            .take_while(|c| is_word_char(*c))
            .collect();
        let current = format!("{pre}{suffix}");
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        let mut order: Vec<_> = self.lines.keys().copied().collect();
        order.sort_by_key(|l| l.abs_diff(line));
        for l in order {
            for word in &self.lines[&l] {
                if word != &current
                    && word.starts_with(prefix)
                    && word.len() > prefix.len()
                    && seen.insert(word.clone())
                {
                    items.push((l.abs_diff(line), word.clone()));
                }
            }
        }
        items.sort();
        items
            .into_iter()
            .take(50)
            .map(|(_, s)| Item {
                label: s.clone(),
                insert_text: s,
                detail: None,
                source: Source::Buffer,
                edit: None,
                additional: Vec::new(),
                raw: None,
                snippet: false,
            })
            .collect()
    }
}
fn words(text: &str) -> std::collections::HashSet<String> {
    text.split(|c| !is_word_char(c))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
