//! Common LSP snippet forms: numbered stops, nested defaults, choices and
//! variables. Deliberately infallible: a transform (`${1/regex/fmt/flags}`,
//! not implemented -- see below) or malformed syntax (an unclosed brace,
//! pathological nesting) degrades to the closest reasonable plain-text
//! reading instead of rejecting the whole completion outright, since a
//! completion item that silently inserts nothing on Tab/Enter is a worse
//! outcome than one that inserts something slightly imperfect.
use std::collections::BTreeMap;
/// One parsed occurrence of a numbered stop: `(start, end, transform)`, where
/// `transform` is `Some(spec)` for a `${n/re/fmt/flags}` mirror.
type Occurrence = (usize, usize, Option<String>);
#[derive(Clone, Debug)]
pub struct Session {
    pub stops: Vec<(usize, usize)>,
    pub mirrors: Vec<Vec<(usize, usize)>>,
    /// Per-mirror transform spec (`${n/regex/fmt/flags}`), aligned 1:1 with
    /// `mirrors` (same outer and inner indices). `None` = a plain mirror that
    /// copies the stop's text verbatim; `Some(spec)` = the stop's text run
    /// through `apply_transform` before being written, recomputed on each
    /// stop-sync so it tracks edits to the stop.
    pub mirror_transforms: Vec<Vec<Option<String>>>,
    /// Choice lists (`${n|a,b,c|}`) for stops that had one, keyed by the
    /// same index as `stops`/`mirrors`. `,` (while `selected`) cycles the
    /// current stop's text through this list -- see `snippet_cycle_choice`.
    pub choices: BTreeMap<usize, Vec<String>>,
    pub current: usize,
    pub selected: bool,
    /// `true` for an LSP linked-editing session (`:linkededit` with no name):
    /// there are no tab stops to cycle — the single stop is the range under the
    /// cursor and its mirrors are the other linked ranges, synced *live* on
    /// every keystroke (see `sync_linked_live`) so typing in one range updates
    /// the others (e.g. an open/close tag pair). Default `false` for snippets.
    pub linked: bool,
}
/// Splits a transform spec `regex/format/flags` on unescaped `/`, unescaping
/// `\/` to `/` within each part.
fn split_transform(spec: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'/') => {
                parts.last_mut().unwrap().push('/');
                chars.next();
            }
            '\\' => parts.last_mut().unwrap().push('\\'),
            '/' => parts.push(String::new()),
            _ => parts.last_mut().unwrap().push(c),
        }
    }
    parts
}

/// Applies an LSP variable transform (`regex/format/flags`) to `value`.
/// Supports capture references (`$1`, `${1}`) in the format and the `g`
/// (global) and `i` (case-insensitive) flags. On any error the input value is
/// returned unchanged (the engine is deliberately infallible).
fn apply_transform(value: &str, spec: &str) -> String {
    let parts = split_transform(spec);
    let Some(pattern) = parts.first() else {
        return value.to_string();
    };
    let format = parts.get(1).map(String::as_str).unwrap_or("");
    let flags = parts.get(2).map(String::as_str).unwrap_or("");
    let mut builder = regex::RegexBuilder::new(pattern);
    if flags.contains('i') {
        builder.case_insensitive(true);
    }
    let Ok(re) = builder.build() else {
        return value.to_string();
    };
    if flags.contains('g') {
        re.replace_all(value, format).into_owned()
    } else {
        re.replace(value, format).into_owned()
    }
}

pub fn expand(input: &str, variables: &BTreeMap<String, String>) -> Expansion {
    fn parse(
        input: &str,
        vars: &BTreeMap<String, String>,
        values: &mut BTreeMap<u32, String>,
        stops: &mut BTreeMap<u32, Vec<Occurrence>>,
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
            let full_tail = &body[split..];
            // A numbered-stop transform (`${1/regex/fmt/flags}`) records an
            // empty mirror occurrence of that stop carrying the transform spec;
            // it's applied to the stop's text on each sync (see
            // `sync_snippet_mirrors`). Variable transforms
            // (`${TM_FILENAME/.../.../}`) ARE applied below at expand time.
            let numbered_transform = full_tail
                .strip_prefix('/')
                .filter(|_| name.parse::<u32>().is_ok())
                .map(str::to_string);
            let tail = if full_tail.starts_with('/') { "" } else { full_tail };
            let start = out.chars().count();
            if let (Ok(n), Some(spec)) = (name.parse::<u32>(), &numbered_transform) {
                // A transform mirror (`${n/re/fmt/flags}`) renders the stop's
                // value run through the transform, and never defines the stop's
                // own value. Only when a source occurrence has already set the
                // value (source-first, the usual `${1:x} … ${1/…/}` order) is
                // the transform shown at expand; a lone/forward transform stays
                // empty (so a lone `${1/…/}` remains an editable empty stop).
                // Either way its live value is recomputed on each sync.
                if let Some(value) = values.get(&n) {
                    out.push_str(&apply_transform(value, spec));
                }
                stops.entry(n).or_default().push((
                    start,
                    out.chars().count(),
                    numbered_transform,
                ));
            } else if let Ok(n) = name.parse::<u32>() {
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
                stops.entry(n).or_default().push((start, out.chars().count(), None));
            } else if let Some(spec) = full_tail.strip_prefix('/') {
                // Variable transform: apply the regex to the variable's value
                // (empty when the variable is unset), computed once at expand.
                let value = vars.get(name).cloned().unwrap_or_default();
                out.push_str(&apply_transform(&value, spec));
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
        .unwrap_or_else(|| vec![(out.chars().count(), out.chars().count(), None)]);
    // `stops` (a BTreeMap<u32, _>) iterates in ascending stop-number order
    // -- record each surviving number's position in that order so
    // `choice_lists` (still keyed by the original stop number) can be
    // rekeyed to match the final `stops`/`mirrors` index space below.
    let order: Vec<u32> = stops.keys().copied().collect();
    let mut groups: Vec<Vec<Occurrence>> = stops.into_values().collect();
    groups.push(end);
    // The editable tab stop for a group is its first *non-transform*
    // occurrence (a lone `${1/.../}` with no `$1`/`${1:..}` falls back to the
    // transform occurrence, staying an empty editable stop as before); the
    // remaining occurrences become mirrors carrying their transform spec.
    let mut stops: Vec<(usize, usize)> = Vec::with_capacity(groups.len());
    let mut mirrors: Vec<Vec<(usize, usize)>> = Vec::with_capacity(groups.len());
    let mut mirror_transforms: Vec<Vec<Option<String>>> = Vec::with_capacity(groups.len());
    for g in groups {
        let primary = g.iter().position(|(_, _, t)| t.is_none()).unwrap_or(0);
        stops.push((g[primary].0, g[primary].1));
        let mut m = Vec::new();
        let mut mt = Vec::new();
        for (i, (a, b, t)) in g.into_iter().enumerate() {
            if i == primary {
                continue;
            }
            m.push((a, b));
            mt.push(t);
        }
        mirrors.push(m);
        mirror_transforms.push(mt);
    }
    let choices = order
        .into_iter()
        .enumerate()
        .filter_map(|(idx, n)| choice_lists.remove(&n).map(|list| (idx, list)))
        .collect();
    Expansion {
        text: out,
        stops,
        mirrors,
        mirror_transforms,
        choices,
    }
}
pub struct Expansion {
    pub text: String,
    pub stops: Vec<(usize, usize)>,
    pub mirrors: Vec<Vec<(usize, usize)>>,
    pub mirror_transforms: Vec<Vec<Option<String>>>,
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
        let raw = self.buf().text_range(a, b);
        // Pair each mirror with its transform spec, then edit right-to-left so
        // earlier edits don't invalidate later ranges.
        let mut items: Vec<((usize, usize), Option<String>)> = s.mirrors[s.current]
            .iter()
            .copied()
            .zip(s.mirror_transforms[s.current].iter().cloned())
            .collect();
        items.sort_by_key(|((a, _), _)| std::cmp::Reverse(*a));
        for ((start, end), spec) in items {
            let text = match &spec {
                Some(spec) => apply_transform(&raw, spec),
                None => raw.clone(),
            };
            self.buf_mut().delete_char_range(start, end);
            self.buf_mut().insert_str_at(start, &text);
            s.shift(start, end, text.chars().count(), None);
        }
        self.snippet = Some(s);
    }

    /// Live mirror for an LSP linked-editing session: after an edit inside the
    /// active range, copy its text to the sibling ranges immediately (not just
    /// on Tab/Esc like snippets), keeping the cursor at the same offset within
    /// the active range even if a preceding mirror's length changed.
    pub fn sync_linked_live(&mut self) {
        let Some(s) = self.snippet.as_ref().filter(|s| s.linked) else {
            return;
        };
        let (a, _) = s.stops[s.current];
        let cur = self.buf().char_idx(self.cursor().0, self.cursor().1);
        let offset = cur.saturating_sub(a);
        self.sync_snippet_mirrors();
        if let Some(s) = self.snippet.as_ref() {
            let start = s.stops[s.current].0;
            let (l, c) = self.buf().pos_from_char_idx(start + offset);
            self.set_cursor_insert(l, c);
        }
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
        // right away (running each through its transform, if any); otherwise
        // they keep showing the stale choice until the next Tab/Esc syncs.
        let mut items: Vec<((usize, usize), Option<String>)> = s.mirrors[current]
            .iter()
            .copied()
            .zip(s.mirror_transforms[current].iter().cloned())
            .collect();
        items.sort_by_key(|((a, _), _)| std::cmp::Reverse(*a));
        for ((start, end), spec) in items {
            let text = match &spec {
                Some(spec) => apply_transform(&next, spec),
                None => next.clone(),
            };
            self.buf_mut().delete_char_range(start, end);
            self.buf_mut().insert_str_at(start, &text);
            s.shift(start, end, text.chars().count(), None);
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
