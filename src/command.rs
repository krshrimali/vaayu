use std::path::PathBuf;

use crate::editor::Editor;
use crate::key::Key;
use crate::mode::CommandKind;

pub fn handle(ed: &mut Editor, key: Key) {
    let kind = match ed.mode {
        crate::mode::Mode::Command(k) => k,
        _ => return,
    };

    match key {
        Key::Esc => {
            ed.cmdline.clear();
            ed.enter_normal();
        }
        Key::Enter => {
            let line = ed.cmdline.clone();
            ed.cmdline.clear();
            ed.enter_normal();
            match kind {
                CommandKind::Ex => run_ex(ed, &line),
                CommandKind::SearchFwd => run_search(ed, &line, true),
                CommandKind::SearchBack => run_search(ed, &line, false),
            }
        }
        Key::Backspace => {
            if ed.cmdline.pop().is_none() {
                ed.enter_normal();
            }
        }
        Key::Char(c) => ed.cmdline.push(c),
        _ => {}
    }
}

fn run_search(ed: &mut Editor, pattern: &str, forward: bool) {
    if pattern.is_empty() {
        return;
    }
    ed.last_search = Some((pattern.to_string(), forward));
    ed.hl_search = true;
    let (line, col) = ed.cursor();
    let from = ed.buf().char_idx(line, col);
    match crate::search::find(ed.buf(), from, pattern, forward, ed.config.ignorecase, ed.config.smartcase) {
        Some(idx) => {
            let (l, c) = ed.buf().pos_from_char_idx(idx);
            ed.set_cursor(l, c);
        }
        None => ed.set_message(format!("pattern not found: {}", pattern)),
    }
}

fn run_ex(ed: &mut Editor, raw: &str) {
    let cmd = raw.trim();
    if cmd.is_empty() {
        return;
    }

    if let Ok(n) = cmd.parse::<usize>() {
        let line = n.saturating_sub(1).min(ed.buf().line_count().saturating_sub(1));
        let col = ed.buf().first_non_blank(line);
        ed.set_cursor(line, col);
        return;
    }

    let (name, rest) = split_command(cmd);
    match name {
        "w" | "write" => {
            let target = rest.trim();
            let result = if target.is_empty() {
                ed.buf_mut().save()
            } else {
                ed.buf_mut().save_as(PathBuf::from(target))
            };
            match result {
                Ok(()) => ed.set_message(format!("\"{}\" written", ed.buf().name())),
                Err(e) => ed.set_message(format!("save failed: {}", e)),
            }
        }
        "q" | "quit" => {
            if ed.buf().modified {
                ed.set_message("unsaved changes -- use :q! to discard");
            } else {
                close_current_or_quit(ed);
            }
        }
        "q!" | "quit!" => close_current_or_quit(ed),
        "wq" | "x" => match ed.buf_mut().save() {
            Ok(()) => close_current_or_quit(ed),
            Err(e) => ed.set_message(format!("save failed: {}", e)),
        },
        "qa" | "qall" | "quitall" => ed.should_quit = true,
        "qa!" | "qall!" => ed.should_quit = true,
        "wqa" | "wqall" | "xa" => {
            for i in 0..ed.buffers.len() {
                let _ = ed.buffers[i].save();
            }
            ed.should_quit = true;
        }
        "noh" | "nohlsearch" => ed.hl_search = false,
        "e" | "edit" => {
            let target = rest.trim();
            if target.is_empty() {
                ed.set_message("E32: no file name");
            } else if let Err(e) = ed.open_file(PathBuf::from(target)) {
                ed.set_message(format!("could not open {}: {}", target, e));
            }
        }
        "ls" | "buffers" => {
            let names: Vec<String> = ed
                .buffers
                .iter()
                .enumerate()
                .map(|(i, b)| format!("{}:{}", i + 1, b.name()))
                .collect();
            ed.set_message(names.join("  "));
        }
        "bn" | "bnext" => {
            ed.cur = (ed.cur + 1) % ed.buffers.len();
        }
        "bp" | "bprev" | "bprevious" => {
            ed.cur = (ed.cur + ed.buffers.len() - 1) % ed.buffers.len();
        }
        _ if name.starts_with('b') && name[1..].parse::<usize>().is_ok() => {
            let n: usize = name[1..].parse().unwrap();
            if n >= 1 && n <= ed.buffers.len() {
                ed.cur = n - 1;
            }
        }
        _ if name.starts_with('s') => run_substitute(ed, cmd),
        _ if cmd.starts_with('%') && cmd[1..].trim_start().starts_with('s') => run_substitute(ed, cmd),
        _ => ed.set_message(format!("E492: not an editor command: {}", cmd)),
    }
}

fn close_current_or_quit(ed: &mut Editor) {
    if ed.buffers.len() <= 1 {
        ed.should_quit = true;
    } else {
        ed.buffers.remove(ed.cur);
        if ed.cur >= ed.buffers.len() {
            ed.cur = ed.buffers.len() - 1;
        }
    }
}

fn split_command(cmd: &str) -> (&str, &str) {
    match cmd.find(|c: char| c.is_whitespace()) {
        Some(i) => (&cmd[..i], &cmd[i..]),
        None => (cmd, ""),
    }
}

/// Handles `:s/pat/repl/flags` on the current line and `:%s/pat/repl/flags` on the whole buffer.
fn run_substitute(ed: &mut Editor, cmd: &str) {
    let whole_buffer = cmd.starts_with('%');
    let body = if whole_buffer { &cmd[1..] } else { cmd };
    let body = body.trim_start();
    let body = match body.strip_prefix('s') {
        Some(b) => b,
        None => {
            ed.set_message("E492: not an editor command");
            return;
        }
    };
    let Some(delim) = body.chars().next() else {
        ed.set_message("E486: pattern required");
        return;
    };
    let parts: Vec<&str> = body[delim.len_utf8()..].split(delim).collect();
    if parts.len() < 2 {
        ed.set_message("E486: incomplete substitute");
        return;
    }
    let pattern = parts[0];
    let replacement = parts[1];
    let flags = parts.get(2).copied().unwrap_or("");
    let global = flags.contains('g');

    let re = match regex::RegexBuilder::new(pattern)
        .case_insensitive(ed.config.ignorecase && !(ed.config.smartcase && pattern.chars().any(|c| c.is_uppercase())))
        .build()
    {
        Ok(re) => re,
        Err(e) => {
            ed.set_message(format!("bad pattern: {}", e));
            return;
        }
    };

    let (start_line, end_line) = if whole_buffer {
        (0, ed.buf().line_count().saturating_sub(1))
    } else {
        (ed.cursor().0, ed.cursor().0)
    };

    ed.buf_mut().begin_edit();
    let mut replaced_any = false;
    for line in start_line..=end_line.min(ed.buf().line_count().saturating_sub(1)) {
        let text = ed.buf().line_text(line);
        let new_text = if global {
            re.replace_all(&text, replacement.replace("\\0", "$0").as_str()).to_string()
        } else {
            re.replace(&text, replacement.replace("\\0", "$0").as_str()).to_string()
        };
        if new_text != text {
            replaced_any = true;
            let start = ed.buf().char_idx(line, 0);
            let end = ed.buf().char_idx(line, ed.buf().line_len(line));
            ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut().insert_str(line, 0, &new_text);
        }
    }
    ed.buf_mut().commit_edit();
    if !replaced_any {
        ed.set_message(format!("pattern not found: {}", pattern));
    } else {
        ed.set_message("substitution complete");
    }
    let (l, _) = ed.cursor();
    let l = l.min(ed.buf().line_count().saturating_sub(1));
    let c = ed.buf().first_non_blank(l);
    ed.set_cursor(l, c);
}
