//! Common LSP snippet forms: numbered stops, nested defaults, choices and
//! variables. Deliberately infallible: a transform (`${1/regex/fmt/flags}`,
//! not implemented -- see below) or malformed syntax (an unclosed brace,
//! pathological nesting) degrades to the closest reasonable plain-text
//! reading instead of rejecting the whole completion outright, since a
//! completion item that silently inserts nothing on Tab/Enter is a worse
//! outcome than one that inserts something slightly imperfect.
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Session {
    pub stops: Vec<(usize, usize)>,
    pub mirrors: Vec<Vec<(usize, usize)>>,
    /// Choice lists (`${n|a,b,c|}`) for stops that had one, keyed by the
    /// same index as `stops`/`mirrors`. `,` (while `selected`) cycles the
    /// current stop's text through this list -- see `snippet_cycle_choice`.
    pub choices: BTreeMap<usize, Vec<String>>,
    pub current: usize,
    pub selected: bool,
}
pub fn expand(input: &str, variables: &BTreeMap<String, String>) -> Expansion {
    fn parse(
        input: &str,
        vars: &BTreeMap<String, String>,
        values: &mut BTreeMap<u32, String>,
        stops: &mut BTreeMap<u32, Vec<(usize, usize)>>,
        choices: &mut BTreeMap<u32, Vec<String>>,
        out: &mut String,
        depth: usize,
    ) {
        // Pathological/malicious nesting: stop expanding and pass
        // whatever's left through literally rather than recursing
        // forever or erroring the whole snippet out.
        if depth >= 32 {
            out.push_str(input);
            return;
        }
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
                let mut closed = false;
                while i < chars.len() {
                    let c = chars[i];
                    i += 1;
                    if c == '{' {
                        nesting += 1;
                    }
                    if c == '}' {
                        nesting -= 1;
                        if nesting == 0 {
                            closed = true;
                            break;
                        }
                    }
                    body.push(c);
                }
                if !closed {
                    // No matching `}` anywhere in the rest of the input --
                    // show it as the literal text it must have been meant
                    // to be, rather than erroring the whole snippet out.
                    out.push_str("${");
                    out.push_str(&body);
                    continue;
                }
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
            // Transforms (`${1/regex/format/flags}`) aren't implemented --
            // treated as a plain, empty-default numbered stop (tail is
            // simply ignored) rather than rejecting the snippet: the
            // fields to type still exist, just without regex-derived
            // pre-filled text.
            let tail = if tail.starts_with('/') { "" } else { tail };
            let start = out.chars().count();
            if let Ok(n) = name.parse::<u32>() {
                // An occurrence carrying a default/choice must capture its
                // text even when a bare occurrence of the same stop (e.g. the
                // `$1` in `$1 ... ${1:default}`) was parsed first and recorded
                // an empty value -- otherwise that empty value wins and the
                // default text is lost from every occurrence.
                let has_default = tail.starts_with(':') || tail.starts_with('|');
                let should_expand = match values.get(&n) {
                    None => true,
                    Some(existing) => existing.is_empty() && has_default,
                };
                if should_expand {
                    if let Some(default) = tail.strip_prefix(':') {
                        parse(default, vars, values, stops, choices, out, depth + 1);
                    } else if let Some(choice_list) =
                        tail.strip_prefix('|').and_then(|s| s.strip_suffix('|'))
                    {
                        let list: Vec<String> =
                            choice_list.split(',').map(str::to_string).collect();
                        out.push_str(list.first().map(String::as_str).unwrap_or(""));
                        choices.insert(n, list);
                    }
                    let value = out.chars().skip(start).collect();
                    values.insert(n, value);
                } else {
                    out.push_str(values.get(&n).map(String::as_str).unwrap_or(""));
                }
                stops
                    .entry(n)
                    .or_default()
                    .push((start, out.chars().count()));
            } else if let Some(value) = vars.get(name) {
                out.push_str(value);
            } else if let Some(default) = tail.strip_prefix(':') {
                parse(default, vars, values, stops, choices, out, depth + 1);
            } else {
                out.push_str(name);
            }
        }
    }
    let mut out = String::new();
    let mut stops = BTreeMap::new();
    let mut choice_lists = BTreeMap::new();
    parse(
        input,
        variables,
        &mut BTreeMap::new(),
        &mut stops,
        &mut choice_lists,
        &mut out,
        0,
    );
    let end = stops
        .remove(&0)
        .unwrap_or_else(|| vec![(out.chars().count(), out.chars().count())]);
    // `stops` (a BTreeMap<u32, _>) iterates in ascending stop-number order
    // -- record each surviving number's position in that order so
    // `choice_lists` (still keyed by the original stop number) can be
    // rekeyed to match the final `stops`/`mirrors` index space below.
    let order: Vec<u32> = stops.keys().copied().collect();
    let mut groups: Vec<_> = stops.into_values().collect();
    groups.push(end);
    let stops = groups.iter().map(|g| g[0]).collect();
    let mirrors = groups
        .into_iter()
        .map(|g| g.into_iter().skip(1).collect())
        .collect();
    let choices = order
        .into_iter()
        .enumerate()
        .filter_map(|(idx, n)| choice_lists.remove(&n).map(|list| (idx, list)))
        .collect();
    Expansion {
        text: out,
        stops,
        mirrors,
        choices,
    }
}
pub struct Expansion {
    pub text: String,
    pub stops: Vec<(usize, usize)>,
    pub mirrors: Vec<Vec<(usize, usize)>>,
    pub choices: BTreeMap<usize, Vec<String>>,
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

    /// `${n|a,b,c|}`'s choices UI: while the placeholder is still
    /// `selected` (not yet typed over), cycles its text through the
    /// snippet's own choice list for that stop instead of the usual
    /// "any keystroke replaces the selection" behavior -- stays
    /// `selected` afterward so repeated presses keep cycling, and typing
    /// anything else still replaces whichever choice is showing, same as
    /// it always did for a plain default.
    pub fn snippet_cycle_choice(&mut self, forward: bool) {
        let Some(s) = &self.snippet else { return };
        if !s.selected {
            return;
        }
        let Some(list) = s.choices.get(&s.current) else {
            return;
        };
        let (a, b) = s.stops[s.current];
        let current_text = self.buf().text_range(a, b);
        let idx = list.iter().position(|c| *c == current_text).unwrap_or(0);
        let next_idx = if forward {
            (idx + 1) % list.len()
        } else {
            (idx + list.len() - 1) % list.len()
        };
        let next = list[next_idx].clone();
        self.buf_mut().delete_char_range(a, b);
        self.buf_mut().insert_str_at(a, &next);
        let mut s = self.snippet.take().unwrap();
        let current = s.current;
        s.shift(a, b, next.chars().count(), Some(current));
        // Propagate the new choice text to this stop's mirror occurrences
        // right away; otherwise they keep showing the stale choice until the
        // next Tab/Esc triggers a mirror sync.
        let mut ranges = s.mirrors[current].clone();
        ranges.sort_by_key(|(a, _)| std::cmp::Reverse(*a));
        for (start, end) in ranges {
            self.buf_mut().delete_char_range(start, end);
            self.buf_mut().insert_str_at(start, &next);
            s.shift(start, end, next.chars().count(), None);
        }
        // Recompute against the (possibly shifted) active stop, since syncing
        // a mirror before it moves its position.
        let (l, c) = self
            .buf()
            .pos_from_char_idx(s.stops[current].0 + next.chars().count());
        self.snippet = Some(s);
        self.set_cursor_insert(l, c);
    }
}
