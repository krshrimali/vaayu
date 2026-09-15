use crate::buffer::Buffer;
use regex::RegexBuilder;

/// Find the next match of `pattern` relative to `from_idx` (a char index),
/// wrapping around the buffer. Returns the match start as a char index.
pub fn find(buf: &Buffer, from_idx: usize, pattern: &str, forward: bool, ignorecase: bool, smartcase: bool) -> Option<usize> {
    if pattern.is_empty() {
        return None;
    }
    let case_insensitive = ignorecase && !(smartcase && pattern.chars().any(|c| c.is_uppercase()));
    let re = match RegexBuilder::new(pattern).case_insensitive(case_insensitive).build() {
        Ok(re) => re,
        Err(_) => {
            let escaped = regex::escape(pattern);
            RegexBuilder::new(&escaped).case_insensitive(case_insensitive).build().ok()?
        }
    };

    let text = buf.rope.to_string();
    // Map byte offsets -> char indices once.
    let mut char_positions: Vec<usize> = Vec::with_capacity(text.len() + 1);
    let mut count = 0usize;
    for (byte_idx, _) in text.char_indices() {
        while char_positions.len() <= byte_idx {
            char_positions.push(count);
        }
        count += 1;
    }
    char_positions.push(count);
    let byte_to_char = |b: usize| -> usize { *char_positions.get(b).unwrap_or(&count) };

    let matches: Vec<usize> = re.find_iter(&text).map(|m| byte_to_char(m.start())).collect();
    if matches.is_empty() {
        return None;
    }

    if forward {
        matches
            .iter()
            .find(|&&pos| pos > from_idx)
            .or_else(|| matches.first())
            .copied()
    } else {
        matches
            .iter()
            .rev()
            .find(|&&pos| pos < from_idx)
            .or_else(|| matches.last())
            .copied()
    }
}
