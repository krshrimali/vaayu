//! Translates Vim's ("magic") regex dialect into the PCRE-style syntax the
//! `regex` crate expects, so muscle-memory patterns like `:s/\(foo\)/[\1]/`
//! and `/\<word\>` behave the way they do in Neovim instead of silently
//! failing to match.
//!
//! Vim's default 'magic' mode inverts PCRE's escaping convention for the
//! grouping/alternation metacharacters: `( ) { } + ? |` are *literal* unless
//! backslash-escaped, and `\( \) \{ \} \+ \? \|` are the special forms. This
//! supports common magic/case switches, word boundaries, noncapturing groups,
//! newline classes and postfix lookaround. Bounded fancy-regex supplies pattern
//! backreferences and lookaround execution; this is not the entire Vim dialect.

pub fn translate_pattern(pat: &str) -> String {
    let mut out = String::new();
    let mut chars = pat.chars().peekable();
    let mut mode = 'm';
    let mut class = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            let Some(n) = chars.next() else {
                out.push('\\');
                break;
            };
            if !class && matches!(n, 'v' | 'V' | 'm' | 'M') {
                mode = n;
                continue;
            }
            if !class && matches!(n, 'c' | 'C') {
                out.push_str(if n == 'c' { "(?i)" } else { "(?-i)" });
                continue;
            }
            if !class && n == '%' && chars.peek() == Some(&'(') {
                chars.next();
                out.push_str("(?:");
                continue;
            }
            if !class && n == '_' {
                if let Some(next) = chars.next() {
                    if next == '.' {
                        out.push_str("(?s:.)");
                    } else {
                        out.push_str(&format!("(?:\\{next}|\\n)"));
                    }
                }
                continue;
            }
            if !class && n == '@' {
                let mut suffix = String::new();
                while chars.peek().is_some_and(|c| matches!(c, '<' | '=' | '!')) {
                    suffix.push(chars.next().unwrap());
                }
                let prefix = match suffix.as_str() {
                    "=" => Some("?="),
                    "!" => Some("?!"),
                    "<=" => Some("?<="),
                    "<!" => Some("?<!"),
                    _ => None,
                };
                if let Some(prefix) = prefix {
                    let mut start = out.char_indices().last().map(|(i, _)| i).unwrap_or(0);
                    if out.ends_with(')') {
                        let mut depth = 0;
                        for (i, c) in out.char_indices().rev() {
                            if c == ')' {
                                depth += 1;
                            }
                            if c == '(' {
                                depth -= 1;
                                if depth == 0 {
                                    start = i;
                                    break;
                                }
                            }
                        }
                    }
                    let atom = out.split_off(start);
                    out.push_str(&format!("({prefix}{atom})"));
                    continue;
                }
                out.push_str("\\@");
                out.push_str(&suffix);
                continue;
            }
            if !class && mode != 'v' && matches!(n, '(' | ')' | '{' | '}' | '+' | '?' | '|') {
                out.push(n);
            } else if !class && matches!(mode, 'M' | 'V') && matches!(n, '.' | '*' | '[') {
                out.push(n);
                if n == '[' {
                    class = true;
                }
            } else {
                out.push('\\');
                out.push(n);
            }
        } else if class {
            out.push(c);
            if c == ']' {
                class = false;
            }
        } else if mode == 'V'
            || (mode == 'M' && matches!(c, '.' | '*' | '['))
            || (mode != 'v' && matches!(c, '(' | ')' | '{' | '}' | '+' | '?' | '|'))
        {
            out.push_str(&regex::escape(&c.to_string()));
        } else {
            out.push(c);
            if c == '[' {
                class = true;
            }
        }
    }
    out.replace("{-}", "*?")
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
        assert_eq!(translate_pattern(r"\<word\>"), r"\<word\>");
    }

    #[test]
    fn already_pcre_classes_pass_through() {
        assert_eq!(translate_pattern(r"\d+"), r"\d\+");
        assert_eq!(translate_pattern(r"\+"), "+");
    }
}
