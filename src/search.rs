use crate::buffer::Buffer;
use fancy_regex::RegexBuilder;

/// Find the next match of `pattern` relative to `from_idx` (a char index),
/// wrapping around the buffer. Returns the match start as a char index.
#[cfg_attr(not(test), allow(dead_code))]
pub fn find(
    buf: &Buffer,
    from_idx: usize,
    pattern: &str,
    forward: bool,
    ignorecase: bool,
    smartcase: bool,
) -> Result<Option<usize>, String> {
    if pattern.is_empty() {
        return Ok(None);
    }
    let matches = positions(buf, pattern, ignorecase, smartcase)?;
    Ok(find_position(&matches, from_idx, forward))
}

/// Materialize every match once for a buffer revision. Callers that repeat a
/// search retain this vector and use `find_position` in O(log n), instead of
/// rebuilding the full Rope string and rescanning it on every `n`/`N`.
pub fn positions(
    buf: &Buffer,
    pattern: &str,
    ignorecase: bool,
    smartcase: bool,
) -> Result<Vec<usize>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = compile(pattern, ignorecase, smartcase)?;
    let text = buf.rope.to_string();
    re.find_iter(&text)
        .map(|m| {
            m.map(|m| buf.rope.byte_to_char(m.start()))
                .map_err(|e| e.to_string())
        })
        .collect()
}

pub fn find_position(matches: &[usize], from_idx: usize, forward: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    if forward {
        let i = matches.partition_point(|p| *p <= from_idx);
        matches.get(i).or_else(|| matches.first()).copied()
    } else {
        let i = matches.partition_point(|p| *p < from_idx);
        i.checked_sub(1)
            .and_then(|i| matches.get(i))
            .or_else(|| matches.last())
            .copied()
    }
}

thread_local! {static REGEX_CACHE:std::cell::RefCell<Option<(String,bool,bool,fancy_regex::Regex)>>=const{std::cell::RefCell::new(None)};}
pub fn compile(
    pattern: &str,
    ignorecase: bool,
    smartcase: bool,
) -> Result<fancy_regex::Regex, String> {
    REGEX_CACHE.with(|cache| {
        if let Some((p, i, s, re)) = cache.borrow().as_ref() {
            if p == pattern && *i == ignorecase && *s == smartcase {
                return Ok(re.clone());
            }
        }
        let ci = ignorecase && !(smartcase && pattern.chars().any(char::is_uppercase));
        let re = RegexBuilder::new(pattern)
            .backtrack_limit(100_000)
            .multi_line(true)
            .crlf(true)
            .case_insensitive(ci)
            .build()
            .map_err(|e| e.to_string())?;
        *cache.borrow_mut() = Some((pattern.into(), ignorecase, smartcase, re.clone()));
        Ok(re)
    })
}
