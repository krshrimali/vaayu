//! Common LSP snippet forms: numbered stops, defaults, choices and variables.
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Session {
    pub stops: Vec<(usize, usize)>,
    pub mirrors: Vec<Vec<(usize, usize)>>,
    pub current: usize,
    pub selected: bool,
}
pub fn expand(input: &str, variables: &BTreeMap<String, String>) -> anyhow::Result<Expansion> {
    fn parse(
        input: &str,
        vars: &BTreeMap<String, String>,
        values: &mut BTreeMap<u32, String>,
        stops: &mut BTreeMap<u32, Vec<(usize, usize)>>,
        out: &mut String,
        depth: usize,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(depth < 32, "Snippet nesting too deep");
        let chars: Vec<_> = input.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if chars[i] != '$' {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            i += 1;
            let mut body = String::new();
            if i < chars.len() && chars[i] == '{' {
                i += 1;
                let mut nesting = 1;
                while i < chars.len() {
                    let c = chars[i];
                    i += 1;
                    if c == '{' {
                        nesting += 1;
                    }
                    if c == '}' {
                        nesting -= 1;
                        if nesting == 0 {
                            break;
                        }
                    }
                    body.push(c);
                }
                anyhow::ensure!(nesting == 0, "Unclosed snippet placeholder");
            } else {
                let numeric = chars.get(i).is_some_and(char::is_ascii_digit);
                while i < chars.len()
                    && (if numeric {
                        chars[i].is_ascii_digit()
                    } else {
                        chars[i].is_alphanumeric() || chars[i] == '_'
                    })
                {
                    body.push(chars[i]);
                    i += 1;
                }
            }
            if body.is_empty() {
                out.push('$');
                continue;
            }
            let split = body.find([':', '|', '/']).unwrap_or(body.len());
            let name = &body[..split];
            let tail = &body[split..];
            anyhow::ensure!(
                !tail.starts_with('/'),
                "Snippet transforms are not supported"
            );
            let start = out.chars().count();
            if let Ok(n) = name.parse::<u32>() {
                if let Some(value) = values.get(&n) {
                    out.push_str(value);
                } else {
                    if let Some(default) = tail.strip_prefix(':') {
                        parse(default, vars, values, stops, out, depth + 1)?;
                    } else if let Some(choices) =
                        tail.strip_prefix('|').and_then(|s| s.strip_suffix('|'))
                    {
                        out.push_str(choices.split(',').next().unwrap_or(""));
                    }
                    let value = out.chars().skip(start).collect();
                    values.insert(n, value);
                }
                stops
                    .entry(n)
                    .or_default()
                    .push((start, out.chars().count()));
            } else if let Some(value) = vars.get(name) {
                out.push_str(value);
            } else if let Some(default) = tail.strip_prefix(':') {
                parse(default, vars, values, stops, out, depth + 1)?;
            } else {
                out.push_str(name);
            }
        }
        Ok(())
    }
    let mut out = String::new();
    let mut stops = BTreeMap::new();
    parse(
        input,
        variables,
        &mut BTreeMap::new(),
        &mut stops,
        &mut out,
        0,
    )?;
    let end = stops
        .remove(&0)
        .unwrap_or_else(|| vec![(out.chars().count(), out.chars().count())]);
    let mut groups: Vec<_> = stops.into_values().collect();
    groups.push(end);
    let stops = groups.iter().map(|g| g[0]).collect();
    let mirrors = groups
        .into_iter()
        .map(|g| g.into_iter().skip(1).collect())
        .collect();
    Ok(Expansion {
        text: out,
        stops,
        mirrors,
    })
}
pub struct Expansion {
    pub text: String,
    pub stops: Vec<(usize, usize)>,
    pub mirrors: Vec<Vec<(usize, usize)>>,
}
impl Session {
    pub fn shift(&mut self, start: usize, end: usize, new_len: usize, active: Option<usize>) {
        let delta = new_len as isize - (end - start) as isize;
        let adjust = |range: &mut (usize, usize)| {
            if end > start && range.0 >= start && range.1 <= end {
                *range = (start + new_len, start + new_len);
            } else if range.0 >= end {
                range.0 = (range.0 as isize + delta).max(0) as usize;
                range.1 = (range.1 as isize + delta).max(range.0 as isize) as usize;
            } else if range.1 >= end {
                range.1 = (range.1 as isize + delta).max(range.0 as isize) as usize;
            }
        };
        for (i, r) in self.stops.iter_mut().enumerate() {
            if active == Some(i) {
                r.1 = (r.1 as isize + delta).max(r.0 as isize) as usize;
            } else {
                adjust(r);
            }
        }
        for group in &mut self.mirrors {
            for r in group {
                if *r == (start, end) {
                    r.1 = start + new_len;
                } else {
                    adjust(r);
                }
            }
        }
    }
}

impl crate::editor::Editor {
    pub fn sync_snippet_mirrors(&mut self) {
        let Some(mut s) = self.snippet.take() else {
            return;
        };
        let (a, b) = s.stops[s.current];
        let text = self.buf().text_range(a, b);
        let mut ranges = s.mirrors[s.current].clone();
        ranges.sort_by_key(|(a, _)| std::cmp::Reverse(*a));
        for (start, end) in ranges {
            self.buf_mut().delete_char_range(start, end);
            self.buf_mut().insert_str_at(start, &text);
            s.shift(start, end, text.chars().count(), None);
        }
        self.snippet = Some(s);
    }

    pub fn snippet_next(&mut self, backwards: bool) {
        self.sync_snippet_mirrors();
        let Some(s) = &mut self.snippet else { return };
        if backwards {
            s.current = s.current.saturating_sub(1);
        } else {
            s.current += 1;
        }
        if s.current >= s.stops.len() {
            self.snippet = None;
            return;
        }
        let pos = s.stops[s.current].0;
        s.selected = true;
        let (l, c) = self.buf().pos_from_char_idx(pos);
        self.set_cursor_insert(l, c);
        self.close_completion();
    }
}
