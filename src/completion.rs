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
}

pub struct CompletionState {
    /// (line, col) of the first character of the word being completed.
    pub start: (usize, usize),
    pub prefix: String,
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
                if word.len() > prefix.len() && word.starts_with(prefix) && seen.insert(word.clone()) {
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
        .map(|(_, w)| Item { label: w.clone(), insert_text: w, detail: None, source: Source::Buffer })
        .collect()
}
