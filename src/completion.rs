use crate::buffer::Buffer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Buffer,
    Lsp,
    Path,
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
    /// The LSP `CompletionItemKind` numeric value, when the server sent
    /// one. `None` for buffer-word candidates, which have no kind.
    pub kind: Option<u64>,
}

/// The completion item's LSP `documentation` field, if it sent one --
/// either a plain string or `{kind, value}` (`MarkupContent`, `kind` is
/// `"markdown"` or `"plaintext"`; the raw `value` is shown either way,
/// no markdown rendering, just the text). Distinct from `detail` (a
/// short one-line signature/type shown inline in the popup already) --
/// this is the longer, multi-line prose a server provides, shown in a
/// separate preview panel only when present, so most items (which have
/// no `documentation` at all) don't grow the popup for nothing.
pub fn item_documentation(item: &Item) -> Option<String> {
    let doc = item.raw.as_ref()?.get("documentation")?;
    let text = doc.as_str().or_else(|| doc["value"].as_str())?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// LSP `CompletionItemKind` numeric values (1-indexed) mapped to a short
/// label for the popup -- a distinct numbering from `outline::kind_label`'s
/// `SymbolKind` table (the two enums don't share values).
pub fn kind_label(kind: u64) -> &'static str {
    match kind {
        1 => "text",
        2 => "method",
        3 => "fn",
        4 => "ctor",
        5 => "field",
        6 => "var",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enum member",
        21 => "const",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type param",
        _ => "lsp",
    }
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

/// LSP clients are responsible for filtering completion results. Servers may
/// return the whole completion set and optionally provide a `filterText`
/// value that is better suited to matching than the displayed label.
pub fn matches(candidate: &str, prefix: &str) -> bool {
    let mut candidate = candidate.chars().flat_map(char::to_lowercase);
    prefix
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| candidate.by_ref().any(|c| c == wanted))
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

/// Like `word_prefix`, but scans backward over path-shaped characters
/// (adds `/`, `.`, `-` to the identifier class) so `src/fo` or
/// `../fixtures/te` are captured whole rather than just the trailing
/// segment. Returns `None` unless the result actually contains a `/` --
/// a plain identifier with a `-` or `.` in it (rare, but real in some
/// languages) must fall through to ordinary buffer/LSP completion
/// instead of being misread as a path.
pub fn path_prefix(buf: &Buffer, line: usize, col: usize) -> Option<(usize, String)> {
    let text: Vec<char> = buf.line_text(line).chars().collect();
    let mut start = col.min(text.len());
    let is_path_char = |c: char| is_word_char(c) || matches!(c, '.' | '-' | '/');
    while start > 0 && is_path_char(text[start - 1]) {
        start -= 1;
    }
    let prefix: String = text[start..col.min(text.len())].iter().collect();
    prefix.contains('/').then_some((start, prefix))
}

/// Filesystem entries under `prefix`'s directory portion (resolved
/// against `base_dir`, the current buffer's own directory, unless the
/// prefix is itself absolute) whose name starts with its file portion.
/// Directories get a trailing `/` so a repeated trigger can keep
/// descending. `insert_text` is the *whole* replacement (directory
/// portion included), matching `path_prefix`'s start position -- like
/// every other completion source, acceptance replaces from that start
/// to the cursor with `insert_text` verbatim, not just the trailing
/// segment.
pub fn path_candidates(prefix: &str, base_dir: &std::path::Path) -> Vec<Item> {
    let (dir_part, file_part) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    let resolved = if dir_part.starts_with('/') {
        std::path::PathBuf::from(dir_part)
    } else {
        base_dir.join(dir_part)
    };
    let Ok(read) = std::fs::read_dir(&resolved) else {
        return Vec::new();
    };
    let mut entries: Vec<(bool, String)> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(file_part) {
                return None;
            }
            if name.starts_with('.') && !file_part.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some((is_dir, name))
        })
        .collect();
    // Directories first (matching shell/editor path-completion convention),
    // then alphabetically within each group.
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    entries
        .into_iter()
        .take(50)
        .map(|(is_dir, name)| {
            let label = if is_dir {
                format!("{name}/")
            } else {
                name.clone()
            };
            Item {
                insert_text: format!("{dir_part}{label}"),
                label,
                detail: None,
                source: Source::Path,
                edit: None,
                additional: Vec::new(),
                raw: None,
                snippet: false,
                kind: None,
            }
        })
        .collect()
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
                if word.chars().count() > prefix.chars().count()
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
            kind: None,
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
                    && word.chars().count() > prefix.chars().count()
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
                kind: None,
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

#[cfg(test)]
mod tests {
    #[test]
    fn completion_match_is_case_insensitive_and_fuzzy() {
        assert!(super::matches("from_millis", "from_"));
        assert!(super::matches("from_millis", "fm"));
        assert!(super::matches("Duration", "dur"));
        assert!(!super::matches("ZERO", "from_"));
        assert!(!super::matches("new", "from_"));
    }

    #[test]
    fn kind_label_maps_known_lsp_completion_item_kinds() {
        assert_eq!(super::kind_label(3), "fn");
        assert_eq!(super::kind_label(6), "var");
        assert_eq!(super::kind_label(7), "class");
        assert_eq!(super::kind_label(14), "keyword");
    }

    #[test]
    fn kind_label_falls_back_for_an_unknown_kind() {
        assert_eq!(super::kind_label(0), "lsp");
        assert_eq!(super::kind_label(999), "lsp");
    }

    fn buf(text: &str) -> crate::buffer::Buffer {
        let mut b = crate::buffer::Buffer::empty();
        b.rope = ropey::Rope::from_str(text);
        b
    }

    #[test]
    fn path_prefix_captures_the_whole_partial_path() {
        let b = buf("src/fo\n");
        assert_eq!(
            super::path_prefix(&b, 0, 6),
            Some((0, "src/fo".to_string()))
        );
    }

    #[test]
    fn path_prefix_is_none_without_a_slash() {
        // A plain identifier, even with a `-` or `.` in it, must fall
        // through to ordinary buffer/LSP completion instead.
        let b = buf("foo-bar.baz\n");
        assert_eq!(super::path_prefix(&b, 0, 11), None);
    }

    #[test]
    fn path_prefix_stops_at_a_quote() {
        let b = buf("\"src/fo\n");
        assert_eq!(
            super::path_prefix(&b, 0, 7),
            Some((1, "src/fo".to_string()))
        );
    }

    #[test]
    fn path_candidates_lists_matching_entries_dirs_first_sorted() {
        let dir = std::env::temp_dir().join(format!(
            "vaayu-pathcomplete-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("foo_dir")).unwrap();
        std::fs::write(dir.join("foo_file.rs"), "").unwrap();
        std::fs::write(dir.join("bar.rs"), "").unwrap();
        std::fs::write(dir.join(".hidden_foo"), "").unwrap();

        let items = super::path_candidates("sub/fo", &dir);
        // "sub/" doesn't exist, so nothing should come back rather than
        // panicking or listing the wrong directory.
        assert!(items.is_empty());

        let items = super::path_candidates("fo", &dir);
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["foo_dir/", "foo_file.rs"],
            "should match both, sorted, with the directory's trailing /, \
             and exclude bar.rs and the dotfile"
        );
        assert_eq!(items[0].insert_text, "foo_dir/");
        assert_eq!(
            items[0].source,
            crate::completion::Source::Path,
            "should be tagged as a path completion, not a buffer word"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn path_candidates_insert_text_includes_the_directory_portion() {
        let dir = std::env::temp_dir().join(format!(
            "vaayu-pathcomplete-test2-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/target.txt"), "").unwrap();

        let items = super::path_candidates("nested/tar", &dir);
        assert_eq!(items.len(), 1);
        // Acceptance replaces the whole prefix span with insert_text, so
        // it must include "nested/", not just the trailing "target.txt".
        assert_eq!(items[0].insert_text, "nested/target.txt");

        std::fs::remove_dir_all(&dir).ok();
    }

    fn item_with_raw(raw: Option<serde_json::Value>) -> super::Item {
        super::Item {
            label: "x".into(),
            insert_text: "x".into(),
            detail: None,
            source: super::Source::Lsp,
            edit: None,
            raw,
            snippet: false,
            additional: Vec::new(),
            kind: None,
        }
    }

    #[test]
    fn item_documentation_reads_a_plain_string() {
        let item = item_with_raw(Some(serde_json::json!({"documentation": "some docs"})));
        assert_eq!(super::item_documentation(&item), Some("some docs".into()));
    }

    #[test]
    fn item_documentation_reads_markup_content() {
        let item = item_with_raw(Some(
            serde_json::json!({"documentation": {"kind": "markdown", "value": "**bold** docs"}}),
        ));
        assert_eq!(
            super::item_documentation(&item),
            Some("**bold** docs".into())
        );
    }

    #[test]
    fn item_documentation_is_none_when_absent_or_blank() {
        assert_eq!(super::item_documentation(&item_with_raw(None)), None);
        assert_eq!(
            super::item_documentation(&item_with_raw(Some(serde_json::json!({})))),
            None
        );
        assert_eq!(
            super::item_documentation(&item_with_raw(Some(
                serde_json::json!({"documentation": "   "})
            ))),
            None,
            "whitespace-only documentation should count as absent"
        );
    }
}
