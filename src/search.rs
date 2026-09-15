use crate::buffer::Buffer;
use regex::RegexBuilder;

/// Find the next match of `pattern` relative to `from_idx` (a char index),
/// wrapping around the buffer. Returns the match start as a char index.
pub fn find(
    buf: &Buffer,
    from_idx: usize,
    pattern: &str,
    forward: bool,
    ignorecase: bool,
    smartcase: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return None;
    }
    let re = compile(pattern, ignorecase, smartcase).ok()?;

    let text = buf.rope.to_string();
    let matches: Vec<usize> = re
        .find_iter(&text)
        .map(|m| buf.rope.byte_to_char(m.start()))
        .collect();
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

thread_local! {static REGEX_CACHE:std::cell::RefCell<Option<(String,bool,bool,regex::Regex)>>=const{std::cell::RefCell::new(None)};}
pub fn compile(pattern: &str, ignorecase: bool, smartcase: bool) -> Result<regex::Regex, String> {
    REGEX_CACHE.with(|cache| {
        if let Some((p, i, s, re)) = cache.borrow().as_ref() {
            if p == pattern && *i == ignorecase && *s == smartcase {
                return Ok(re.clone());
            }
        }
        let ci = ignorecase && !(smartcase && pattern.chars().any(char::is_uppercase));
        let re = RegexBuilder::new(pattern)
            .multi_line(true)
            .crlf(true)
            .case_insensitive(ci)
            .build()
            .map_err(|e| e.to_string())?;
        *cache.borrow_mut() = Some((pattern.into(), ignorecase, smartcase, re.clone()));
        Ok(re)
    })
}
