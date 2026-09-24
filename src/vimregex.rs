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
                    } else if next == '[' {
                        // `\_[...]` is a newline-inclusive character class: open
                        // `[\n` and let the normal class scanner consume the
                        // rest through the closing `]`.
                        out.push_str("[\\n");
                        class = true;
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
                    let start = last_atom_start(&out);
                    let atom = out.split_off(start);
                    out.push_str(&format!("({prefix}{atom})"));
                    continue;
                }
                out.push_str("\\@");
                out.push_str(&suffix);
                continue;
            }
            if !class && mode != 'v' && n == '{' {
                // Vim bounded/non-greedy quantifier opened with `\{`. Parse
                // the whole `\{...}` (whether the closing brace is bare or
                // escaped as `\}`) and emit a valid Rust-regex quantifier.
                // Otherwise a bare `}` would be `regex::escape`d to `\}`
                // (yielding an invalid `{2,3\}`), and `\{-}` never became
                // the intended non-greedy `*?`.
                out.push_str(&parse_quantifier(&mut chars));
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
        } else if mode == 'v' && c == '{' {
            // Very-magic mode: a bare `{` opens a quantifier (its closing
            // `}` is bare too), the same translation as magic mode's `\{`.
            out.push_str(&parse_quantifier(&mut chars));
        } else if mode == 'v' && c == '<' {
            // Very-magic: bare `<`/`>` are word boundaries (like `\<`/`\>`).
            out.push_str("\\<");
        } else if mode == 'v' && c == '>' {
            out.push_str("\\>");
        } else if mode == 'v' && c == '%' && chars.peek() == Some(&'(') {
            // Very-magic: `%(` is a non-capturing group.
            chars.next();
            out.push_str("(?:");
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
    out
}

/// Byte index in `out` where the last regex "atom" begins, for postfix
/// lookaround (`\@=`/`\@!`/`\@<=`/`\@<!`) extraction. Handles a trailing group
/// `(...)`, an escaped char `\x` (including an escaped literal paren `\)`), and
/// a bare char -- so `)\@=` (lookahead on a literal `)`) and `\d\@=` don't
/// miscount the atom.
fn last_atom_start(out: &str) -> usize {
    let chars: Vec<(usize, char)> = out.char_indices().collect();
    if chars.is_empty() {
        return 0;
    }
    // Whether the char at index `k` is escaped (preceded by an odd run of `\`).
    let escaped = |k: usize| -> bool {
        let mut b = 0;
        let mut j = k;
        while j > 0 && chars[j - 1].1 == '\\' {
            b += 1;
            j -= 1;
        }
        b % 2 == 1
    };
    let last = chars.len() - 1;
    if escaped(last) {
        // `\x`: the atom is the backslash plus its escaped char.
        return chars[last - 1].0;
    }
    if chars[last].1 == ')' {
        // A real group close: find its matching unescaped `(`.
        let mut depth = 0;
        let mut i = last;
        loop {
            if !escaped(i) {
                match chars[i].1 {
                    ')' => depth += 1,
                    '(' => {
                        depth -= 1;
                        if depth == 0 {
                            return chars[i].0;
                        }
                    }
                    _ => {}
                }
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }
    chars[last].0
}

/// Consumes a Vim quantifier body after its opening brace (the `\{` / `{`
/// has already been consumed) up to and including the closing brace --
/// either a bare `}` or an escaped `\}` -- and returns the equivalent
/// Rust-regex quantifier.
fn parse_quantifier(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut body = String::new();
    while let Some(c) = chars.next() {
        match c {
            '}' => break,
            '\\' => match chars.next() {
                // The escaped-close form `\{...\}`: swallow the backslash
                // and stop at the `}` it protects.
                Some('}') => break,
                Some(other) => body.push(other),
                None => break,
            },
            _ => body.push(c),
        }
    }
    translate_quantifier_body(&body)
}

/// Translates the inside of a Vim `\{...}` quantifier into Rust-regex
/// syntax. Handles the greedy forms `\{n}`, `\{n,}`, `\{n,m}`, `\{,m}`,
/// `\{}` and each of their non-greedy `\{-...}` variants (`\{-}` -> `*?`,
/// `\{-n,m}` -> `{n,m}?`, ...). A missing lower bound becomes `0` since
/// the `regex` crate requires one.
fn translate_quantifier_body(body: &str) -> String {
    let (non_greedy, spec) = match body.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, body),
    };
    let quant = if spec.is_empty() {
        // `\{}` / `\{-}`: zero or more.
        "*".to_string()
    } else if let Some((lo, hi)) = spec.split_once(',') {
        let lo = if lo.is_empty() { "0" } else { lo };
        if hi.is_empty() {
            format!("{{{lo},}}")
        } else {
            format!("{{{lo},{hi}}}")
        }
    } else {
        format!("{{{spec}}}")
    };
    if non_greedy {
        format!("{quant}?")
    } else {
        quant
    }
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
                // Vim replacement escapes: `\r` inserts a newline, `\t` a
                // tab, `\n` a NUL. The catch-all used to drop the backslash
                // and push the bare letter (`\r`->"r"), which was wrong.
                Some('r') => {
                    out.push('\n');
                    chars.next();
                }
                Some('t') => {
                    out.push('\t');
                    chars.next();
                }
                Some('n') => {
                    out.push('\0');
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
    fn newline_inclusive_class() {
        // `\_[...]` must open a newline-inclusive character class, not treat
        // `[` as a literal atom followed by loose class text.
        assert_eq!(translate_pattern(r"\_[a-z]"), r"[\na-z]");
        assert_eq!(translate_pattern(r"\_[abc]x"), r"[\nabc]x");
        // `\_.` (any char incl. newline) still works, and `\_x` for a plain
        // atom stays the alternation form.
        assert_eq!(translate_pattern(r"\_."), "(?s:.)");
        assert_eq!(translate_pattern(r"\_s"), r"(?:\s|\n)");
    }

    #[test]
    fn already_pcre_classes_pass_through() {
        assert_eq!(translate_pattern(r"\d+"), r"\d\+");
        assert_eq!(translate_pattern(r"\+"), "+");
    }

    #[test]
    fn bounded_quantifier_unescaped_close() {
        assert_eq!(translate_pattern(r"a\{2,3}"), "a{2,3}");
        assert_eq!(translate_pattern(r"a\{2,3\}"), "a{2,3}");
        assert_eq!(translate_pattern(r"a\{2}"), "a{2}");
        assert_eq!(translate_pattern(r"a\{2,}"), "a{2,}");
        assert_eq!(translate_pattern(r"a\{,3}"), "a{0,3}");
        assert_eq!(translate_pattern(r"a\{}"), "a*");
    }

    #[test]
    fn non_greedy_quantifier() {
        assert_eq!(translate_pattern(r"a.\{-}b"), "a.*?b");
        assert_eq!(translate_pattern(r"a\{-2,3}"), "a{2,3}?");
        assert_eq!(translate_pattern(r"a\{-2,}"), "a{2,}?");
        assert_eq!(translate_pattern(r"a\{-,3}"), "a{0,3}?");
    }

    #[test]
    fn very_magic_bare_quantifier() {
        assert_eq!(translate_pattern(r"\va{2,3}"), "a{2,3}");
        assert_eq!(translate_pattern(r"\va.{-}b"), "a.*?b");
    }

    #[test]
    fn very_magic_word_boundaries_and_noncapturing_group() {
        // Bare `<`/`>` are word boundaries in very-magic; `%(` opens a
        // non-capturing group.
        assert_eq!(translate_pattern(r"\v<word>"), r"\<word\>");
        assert_eq!(translate_pattern(r"\v%(ab)+"), "(?:ab)+");
    }

    #[test]
    fn lookaround_atom_extraction_handles_escapes_and_groups() {
        // Lookahead on a literal `)` (a bare Vim `)` escapes to `\)`).
        assert_eq!(translate_pattern(r")\@="), r"(?=\))");
        // Lookahead on an escaped class atom `\d`.
        assert_eq!(translate_pattern(r"\d\@="), r"(?=\d)");
        // A real group is still taken as the whole atom.
        assert_eq!(translate_pattern(r"\(ab\)\@="), "(?=(ab))");
    }

    #[test]
    fn replacement_escapes() {
        assert_eq!(translate_replacement(r"\r"), "\n");
        assert_eq!(translate_replacement(r"\t"), "\t");
        assert_eq!(translate_replacement(r"\n"), "\0");
        assert_eq!(translate_replacement(r"a\rb"), "a\nb");
    }
}
