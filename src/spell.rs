//! Optional spell checking. Loads a system word list if one is present
//! (several common Linux/BSD/macOS paths) plus a private per-user
//! dictionary; degrades to "no dictionary available" rather than failing
//! or blocking editing when neither exists, matching this project's rule
//! that optional integrations fail visibly instead of getting in the way.
//!
//! Scope: misspelling "decoration" is a navigable results list
//! (`:spellcheck`), not a live inline underline -- adding that to the
//! render cache/RowSignature system is separate, riskier work left for
//! later. Word-splitting is alphabetic-run based with no code-aware
//! filtering beyond skipping anything containing a digit or `_`, and
//! skipping runs under 3 letters, so this is most useful on prose (docs,
//! commit messages, comments) and will flag real code identifiers too --
//! it's an explicit, opt-in command, never a background pass.
use std::collections::HashSet;
use std::path::PathBuf;

const SYSTEM_DICTIONARIES: &[&str] = &[
    "/usr/share/dict/words",
    "/usr/share/dict/american-english",
    "/usr/share/dict/british-english",
    "/usr/share/dict/words.txt",
];
const MIN_WORD_LEN: usize = 3;
const MAX_SUGGESTIONS: usize = 8;
/// Bounds the suggestion scan cost on a huge system dictionary: only words
/// within this length difference of the target are ever compared.
const SUGGEST_LEN_WINDOW: usize = 2;

pub struct Dictionary {
    words: HashSet<String>,
    user_words: HashSet<String>,
    user_path: Option<PathBuf>,
}

impl Dictionary {
    pub fn load() -> Self {
        let mut words = HashSet::new();
        for path in SYSTEM_DICTIONARIES {
            if let Ok(text) = std::fs::read_to_string(path) {
                words.extend(text.lines().map(|w| w.trim().to_lowercase()));
                break;
            }
        }
        let user_path = dirs::config_dir().map(|d| d.join("vaayu").join("dictionary.txt"));
        let mut user_words = HashSet::new();
        if let Some(p) = &user_path {
            if let Ok(text) = std::fs::read_to_string(p) {
                user_words.extend(
                    text.lines()
                        .map(|w| w.trim().to_lowercase())
                        .filter(|w| !w.is_empty()),
                );
            }
        }
        Self {
            words,
            user_words,
            user_path,
        }
    }

    pub fn available(&self) -> bool {
        !self.words.is_empty()
    }

    /// A dictionary seeded directly with words, bypassing the filesystem --
    /// for tests, so they don't depend on `/usr/share/dict/words` existing
    /// (it often doesn't, e.g. in minimal containers) or on its contents.
    #[cfg(test)]
    pub fn for_test(words: &[&str]) -> Self {
        Self {
            words: words.iter().map(|w| w.to_lowercase()).collect(),
            user_words: HashSet::new(),
            user_path: None,
        }
    }

    fn is_correct(&self, word: &str) -> bool {
        let lower = word.to_lowercase();
        self.words.contains(&lower) || self.user_words.contains(&lower)
    }

    pub fn add_word(&mut self, word: &str) -> anyhow::Result<()> {
        let lower = word.to_lowercase();
        if !self.user_words.insert(lower.clone()) {
            return Ok(());
        }
        if let Some(p) = &self.user_path {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)?;
            writeln!(f, "{lower}")?;
        }
        Ok(())
    }

    /// Alphabetic-run spans in `text` (char columns, end exclusive) that
    /// look like words worth checking: length >= 3, no digits/underscore,
    /// internal apostrophes allowed (`don't`).
    pub fn words_in(text: &str) -> Vec<(usize, usize, String)> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if !chars[i].is_alphabetic() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphabetic()
                    || (chars[i] == '\'' && i + 1 < chars.len() && chars[i + 1].is_alphabetic()))
            {
                i += 1;
            }
            if i - start >= MIN_WORD_LEN {
                out.push((start, i, chars[start..i].iter().collect()));
            }
        }
        out
    }

    /// The word span (if any) containing char column `col`, for `zg`/`z=`.
    pub fn word_at(text: &str, col: usize) -> Option<(usize, usize, String)> {
        Self::words_in(text)
            .into_iter()
            .find(|(s, e, _)| col >= *s && col < *e)
    }

    /// Misspelled word spans in `text`, in the same shape as `words_in`.
    pub fn misspelled_in(&self, text: &str) -> Vec<(usize, usize, String)> {
        Self::words_in(text)
            .into_iter()
            .filter(|(_, _, w)| !self.is_correct(w))
            .collect()
    }

    pub fn suggestions(&self, word: &str) -> Vec<String> {
        let target = word.to_lowercase();
        let mut scored: Vec<(usize, &str)> = self
            .words
            .iter()
            .chain(self.user_words.iter())
            .filter(|w| w.len().abs_diff(target.len()) <= SUGGEST_LEN_WINDOW)
            .filter_map(|w| {
                let d = levenshtein(&target, w);
                (d <= SUGGEST_LEN_WINDOW).then_some((d, w.as_str()))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        scored.dedup_by(|a, b| a.1 == b.1);
        scored
            .into_iter()
            .take(MAX_SUGGESTIONS)
            .map(|(_, w)| w.to_string())
            .collect()
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

impl crate::editor::Editor {
    /// Loads the dictionary on first call and reuses it after; see
    /// `Editor::dictionary`'s doc comment for why this isn't eager.
    pub fn ensure_dictionary(&mut self) -> &mut Dictionary {
        self.dictionary.get_or_insert_with(Dictionary::load)
    }

    /// Recomputes `spell_spans` (misspelled `(line, start, end)` char spans)
    /// for the current buffer when `spell` is on and the cache is stale. A
    /// no-op (clearing the cache) when spell is off or no dictionary exists.
    pub fn update_spell_spans(&mut self) {
        if !self.config.spell || !self.ensure_dictionary().available() {
            if !self.spell_spans.is_empty() {
                self.spell_spans.clear();
                self.spell_spans_buffer = None;
            }
            return;
        }
        let id = self.buf().id;
        let seq = self.buf().edit_seq;
        if self.spell_spans_buffer == Some(id) && self.spell_spans_edit_seq == seq {
            return;
        }
        let line_count = self.buf().line_count().min(20_000);
        let mut spans = Vec::new();
        for line in 0..line_count {
            let text = self.buf().line_text(line);
            for (start, end, _) in self.ensure_dictionary().misspelled_in(&text) {
                spans.push((line, start, end));
            }
        }
        self.spell_spans = spans;
        self.spell_spans_buffer = Some(id);
        self.spell_spans_edit_seq = seq;
    }

    /// `]s` / `[s`: move the cursor to the next / previous misspelled word.
    pub fn spell_nav(&mut self, forward: bool) {
        self.update_spell_spans();
        if self.spell_spans.is_empty() {
            self.set_message("No misspellings (or spell is off — :set spell)");
            return;
        }
        let (line, col) = self.cursor();
        let target = if forward {
            self.spell_spans
                .iter()
                .find(|&&(l, s, _)| (l, s) > (line, col))
                .or_else(|| self.spell_spans.first())
        } else {
            self.spell_spans
                .iter()
                .rev()
                .find(|&&(l, s, _)| (l, s) < (line, col))
                .or_else(|| self.spell_spans.last())
        };
        if let Some(&(l, s, _)) = target {
            self.push_jump();
            self.set_cursor(l, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(words: &[&str]) -> Dictionary {
        Dictionary {
            words: words.iter().map(|w| w.to_lowercase()).collect(),
            user_words: HashSet::new(),
            user_path: None,
        }
    }

    #[test]
    fn splits_alphabetic_runs_and_skips_short_ones() {
        let spans = Dictionary::words_in("fn foo_bar(x: i32) -> don't123 xyzzy");
        let words: Vec<_> = spans.iter().map(|(_, _, w)| w.as_str()).collect();
        // "fn"/"x"/"i" too short (digits never extend a run at all, so
        // "i32" only ever contributes the 1-letter "i"); "foo"/"bar" split
        // on '_'; "don't" keeps its apostrophe, stopping before "123".
        assert_eq!(words, vec!["foo", "bar", "don't", "xyzzy"]);
    }

    #[test]
    fn flags_words_not_in_the_dictionary() {
        let d = dict(&["hello", "world"]);
        let bad = d.misspelled_in("hello wrold");
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].2, "wrold");
    }

    #[test]
    fn add_word_makes_it_correct() {
        let mut d = dict(&["hello"]);
        assert!(!d.is_correct("vaayu"));
        d.add_word("Vaayu").unwrap(); // case-insensitive
        assert!(d.is_correct("vaayu"));
        assert!(d.is_correct("VAAYU"));
    }

    #[test]
    fn suggestions_are_close_by_edit_distance() {
        let d = dict(&["hello", "world", "help", "yellow"]);
        let s = d.suggestions("wrold");
        assert!(s.contains(&"world".to_string()), "{s:?}");
        let s2 = d.suggestions("helo");
        assert!(
            s2.contains(&"hello".to_string()) || s2.contains(&"help".to_string()),
            "{s2:?}"
        );
    }
}
