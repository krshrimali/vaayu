//! Translates Vim's ("magic") regex dialect into the PCRE-style syntax the
//! `regex` crate expects, so muscle-memory patterns like `:s/\(foo\)/[\1]/`
//! and `/\<word\>` behave the way they do in Neovim instead of silently
//! failing to match.
//!
//! Vim's default 'magic' mode inverts PCRE's escaping convention for the
//! grouping/alternation metacharacters: `( ) { } + ? |` are *literal* unless
//! backslash-escaped, and `\( \) \{ \} \+ \? \|` are the special forms. This
//! only handles that common case, plus `\<`/`\>` word boundaries -- it is not
//! a full Vim-regex engine (no `\v`/`\V`/`\%(`/collections like `\d`, which
//! already coincide with PCRE and pass through untouched).

pub fn translate_pattern(pat: &str) -> String {
    let mut out = String::new();
    let mut chars = pat.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some(n @ ('(' | ')' | '{' | '}' | '+' | '?' | '|')) => {
                    out.push(n);
                    chars.next();
                }
                Some('<') | Some('>') => {
                    out.push_str("\\b");
                    chars.next();
                }
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                    chars.next();
                }
                None => out.push('\\'),
            }
        } else if matches!(c, '(' | ')' | '{' | '}' | '+' | '?' | '|') {
            out.push('\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

/// Translates a Vim-style `:s` replacement (`\1`..`\9`, `\0`/`&` for the
/// whole match, `\\` for a literal backslash) into the `regex` crate's
/// `$1`/`${1}` replacement syntax, and escapes any literal `$` so it isn't
/// misread as a capture reference.
pub fn translate_replacement(rep: &str) -> String {
    let mut out = String::new();
    let mut chars = rep.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek().copied() {
                Some(d) if d.is_ascii_digit() => {
                    out.push_str(&format!("${{{}}}", d));
                    chars.next();
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                Some('&') => {
                    out.push('&');
                    chars.next();
                }
                Some(other) => {
                    out.push(other);
                    chars.next();
                }
                None => out.push('\\'),
            },
            '&' => out.push_str("${0}"),
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_and_backref() {
        assert_eq!(translate_pattern(r"\(hello\)"), "(hello)");
        assert_eq!(translate_replacement(r"[\1]"), "[${1}]");
    }

    #[test]
    fn literal_parens_get_escaped() {
        assert_eq!(translate_pattern("foo(bar)"), r"foo\(bar\)");
    }

    #[test]
    fn word_boundaries() {
        assert_eq!(translate_pattern(r"\<word\>"), r"\bword\b");
    }

    #[test]
    fn already_pcre_classes_pass_through() {
        assert_eq!(translate_pattern(r"\d+"), r"\d\+");
        assert_eq!(translate_pattern(r"\+"), "+");
    }
}
