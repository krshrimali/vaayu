use std::path::PathBuf;

use crate::editor::Editor;
use crate::key::Key;
use crate::mode::CommandKind;

/// Command-line history is capped so a very long session doesn't grow it
/// without bound; it lives in memory only for this slice, not persisted
/// across restarts (unlike notes/recovery/undo, which are).
const MAX_HISTORY: usize = 200;

fn push_history(hist: &mut Vec<String>, line: &str) {
    if line.is_empty() || hist.last().map(String::as_str) == Some(line) {
        return;
    }
    hist.push(line.to_string());
    if hist.len() > MAX_HISTORY {
        hist.remove(0);
    }
}

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
                CommandKind::Ex => {
                    push_history(&mut ed.command_history, &line);
                    run_ex(ed, &line);
                }
                CommandKind::SearchFwd => {
                    push_history(&mut ed.search_history, &line);
                    run_search(ed, &line, true);
                }
                CommandKind::SearchBack => {
                    push_history(&mut ed.search_history, &line);
                    run_search(ed, &line, false);
                }
            }
        }
        Key::Backspace => {
            if ed.cmdline.pop().is_none() {
                ed.enter_normal();
            }
        }
        Key::Up | Key::Ctrl('p') => history_step(ed, kind, true),
        Key::Down | Key::Ctrl('n') => history_step(ed, kind, false),
        Key::Char(c) => ed.cmdline.push(c),
        _ => {}
    }
}

/// Cycles through command/search history, matching Vim's Up/Down (and
/// Ctrl-P/Ctrl-N) in the command line: `older` moves toward earlier
/// entries, saving the in-progress line on the first press so it can be
/// restored when cycling back past the newest entry.
fn history_step(ed: &mut Editor, kind: CommandKind, older: bool) {
    let len = if kind == CommandKind::Ex {
        ed.command_history.len()
    } else {
        ed.search_history.len()
    };
    let next = if older {
        if len == 0 {
            return;
        }
        match ed.history_browse {
            None => {
                ed.history_draft = ed.cmdline.clone();
                Some(len - 1)
            }
            Some(0) => Some(0),
            Some(i) => Some(i - 1),
        }
    } else {
        match ed.history_browse {
            None => return,
            Some(i) if i + 1 < len => Some(i + 1),
            Some(_) => None,
        }
    };
    ed.history_browse = next;
    ed.cmdline = match next {
        Some(i) if kind == CommandKind::Ex => ed.command_history[i].clone(),
        Some(i) => ed.search_history[i].clone(),
        None => std::mem::take(&mut ed.history_draft),
    };
}

pub(crate) fn run_search(ed: &mut Editor, pattern: &str, forward: bool) {
    if pattern.is_empty() {
        return;
    }
    let pattern = crate::vimregex::translate_pattern(pattern);
    let pattern = pattern.as_str();
    if let Err(e) = crate::search::compile(pattern, ed.config.ignorecase, ed.config.smartcase) {
        ed.set_message(format!("Invalid search: {e}"));
        return;
    }
    ed.last_search = Some((pattern.to_string(), forward));
    ed.hl_search = true;
    let (line, col) = ed.cursor();
    let from = ed.buf().char_idx(line, col);
    match ed.find_search(pattern, from, forward) {
        Ok(Some(idx)) => {
            let (l, c) = ed.buf().pos_from_char_idx(idx);
            ed.set_cursor(l, c);
            crate::normal::recenter_viewport(ed);
        }
        Ok(None) => ed.set_message(format!("pattern not found: {}", pattern)),
        Err(e) => ed.set_message(format!("Search failed: {e}")),
    }
}

pub fn run_ex(ed: &mut Editor, raw: &str) {
    let cmd = raw.trim();
    if cmd.is_empty() {
        return;
    }

    if let Ok(n) = cmd.parse::<usize>() {
        let line = n
            .saturating_sub(1)
            .min(ed.buf().line_count().saturating_sub(1));
        let col = ed.buf().first_non_blank(line);
        ed.set_cursor(line, col);
        return;
    }

    let (name, rest) = split_command(cmd);
    match name {
        "gitdiff" => ed.git_results("diff"),
        "gitstage" => ed.git_results("stage"),
        "gitunstage" => ed.git_results("unstage"),
        "gitblame" => ed.git_results("blame"),
        "gitstash" => ed.show_git_stash(),
        "recover" => ed.show_recovery(),
        "reviewrun" => ed.run_review(),
        "reviewcancel" => ed.cancel_review(),
        "lspcancel" => ed.cancel_language_requests(),
        "reviewresolve" => ed.resolve_review(),
        "reviewresults" => {
            if let Some(r) = ed.review_results.clone() {
                ed.show_results(r);
            }
        }
        "reviewexport" => {
            let result = ed.export_review();
            ed.set_message(match result {
                Ok(p) => format!("Review packet: {}", p.display()),
                Err(e) => e.to_string(),
            });
        }
        "sessionsave" => {
            let result = ed.save_session();
            ed.set_message(
                result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "Session saved".into()),
            );
        }
        "sessionload" => {
            let result = ed.load_session();
            ed.set_message(
                result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "Session restored".into()),
            );
        }
        "help" => ed.show_results(crate::results::Results::new(
            "Help",
            include_str!("../HELP.md")
                .lines()
                .map(crate::results::Entry::text)
                .collect(),
        )),
        "keymaps" => {
            let mut entries: Vec<_> = crate::actions::ACTIONS
                .iter()
                .map(|a| {
                    let mut e = crate::results::Entry::text(format!(
                        "{}{:<6} {}",
                        ed.config.leader, a.keys, a.title
                    ));
                    e.action = Some(serde_json::json!({"_vaayu_action_id": a.id}));
                    e
                })
                .collect();
            entries.sort_by(|a, b| a.text.cmp(&b.text));
            ed.show_results(crate::results::Results::new("Keymaps", entries));
        }
        "comments" | "review" => ed.comments_results(),
        "comment" => ed.new_note(false),
        "commentfile" => ed.new_note(true),
        "commentswrite" => {
            let result = ed.save_notes();
            ed.set_message(match result {
                Ok(()) => "Comments saved".into(),
                Err(e) => e.to_string(),
            });
        }
        "copen" => ed.open_quickfix(),
        "cclose" => ed.enter_normal(),
        "cnext" | "cn" => ed.quickfix_step(true),
        "cprev" | "cp" => ed.quickfix_step(false),
        "colder" | "col" => ed.quickfix_older(),
        "cnewer" | "cnew" => ed.quickfix_newer(),
        "grep" => ed.open_grep(rest.trim()),
        "diagnostics" => {
            let r = ed.diagnostic_results();
            ed.show_results(r);
        }
        "outline" => ed.request_language("outline", None),
        "references" => ed.request_language("references", None),
        "typedefinition" => ed.request_type_definition(),
        "implementation" => ed.request_implementation(),
        "declaration" => ed.request_declaration(),
        "workspacesymbols" => ed.request_workspace_symbols(rest.trim()),
        "format" => ed.request_language("format", None),
        "rename" => ed.request_language("rename", Some(rest.trim())),
        "codeactions" => ed.request_language("actions", None),
        "signature" => ed.request_language("signature", None),
        "lsprestart" => ed.restart_lsp(),
        "lspinfo" => {
            let entries = ed
                .lsp_clients
                .iter()
                .map(|(key, c)| crate::results::Entry::text(format!("{key}: {}", c.server_cmd)))
                .collect();
            ed.show_results(crate::results::Results::new("Language servers", entries));
        }
        "spellcheck" => {
            if !ed.ensure_dictionary().available() {
                ed.set_message("No dictionary found (looked in /usr/share/dict/words and similar)");
                return;
            }
            let buf_id = ed.buf().id;
            let line_count = ed.buf().line_count().min(20_000);
            let mut entries = Vec::new();
            for line in 0..line_count {
                let text = ed.buf().line_text(line);
                for (start, _end, word) in ed.ensure_dictionary().misspelled_in(&text) {
                    let mut e = crate::results::Entry::text(format!(
                        "{}:{}  {}",
                        line + 1,
                        start + 1,
                        word
                    ));
                    e.buffer_id = Some(buf_id);
                    e.line = line;
                    e.col = start;
                    entries.push(e);
                }
            }
            if entries.is_empty() {
                ed.set_message("No misspelled words found");
            } else {
                ed.show_results(crate::results::Results::new("Spelling", entries));
            }
        }
        "indentinfo" => {
            let b = ed.buf();
            ed.set_message(format!(
                "indent: {} ts={} sw={} {}",
                b.indent_source.label(),
                b.tabstop,
                b.shiftwidth,
                if b.expandtab { "space" } else { "tab" }
            ));
        }
        "terminal" | "term" => ed.open_terminal(),
        "treenew" => ed.tree_new(rest.trim()),
        "treerename" => ed.tree_rename(rest.trim()),
        "tabnew" => ed.new_tab(),
        "tabclose" | "tabc" => ed.close_tab(),
        "tabonly" | "tabo" => ed.tab_only(),
        "tabnext" | "tabn" => ed.next_tab(),
        "tabprev" | "tabp" | "tabprevious" => ed.prev_tab(),
        "chistory" | "history" => {
            let entries = ed
                .command_history
                .iter()
                .rev()
                .map(|c| {
                    let mut e = crate::results::Entry::text(format!(":{c}"));
                    e.action = Some(serde_json::json!({"_vaayu_rerun_ex": c}));
                    e
                })
                .collect();
            ed.show_results(crate::results::Results::new("Command history", entries));
        }
        "shistory" => {
            let entries = ed
                .search_history
                .iter()
                .rev()
                .map(|c| {
                    let mut e = crate::results::Entry::text(format!("/{c}"));
                    e.action = Some(serde_json::json!({"_vaayu_rerun_search": c}));
                    e
                })
                .collect();
            ed.show_results(crate::results::Results::new("Search history", entries));
        }
        "jumps" => {
            let entries = ed
                .jumps
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let name = l
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "[scratch]".to_string());
                    let mut e = crate::results::Entry::text(format!(
                        "{}{} {}:{}:{}",
                        if i == ed.jump_index { "> " } else { "  " },
                        i,
                        name,
                        l.line + 1,
                        l.col + 1
                    ));
                    e.buffer_id = Some(l.buffer);
                    e.path = l.path.clone();
                    e.line = l.line;
                    e.col = l.col;
                    e
                })
                .collect();
            ed.show_results(crate::results::Results::new("Jumps", entries));
        }
        "resume" => ed.resume(),
        "treebookmarks" => ed.show_tree_bookmarks(),
        "tabs" => {
            let entries = (0..ed.tabs.len())
                .map(|i| {
                    crate::results::Entry::text(format!(
                        "{}{}",
                        i + 1,
                        if i == ed.active_tab { " (current)" } else { "" }
                    ))
                })
                .collect();
            ed.show_results(crate::results::Results::new("Tabs", entries));
        }
        "vsplit" | "split" => {
            ed.split_window(name == "vsplit", false);
            if !rest.trim().is_empty() {
                if let Err(e) = ed.open_file(PathBuf::from(rest.trim())) {
                    ed.set_message(e.to_string());
                }
            }
        }
        "vpreview" => ed.split_window(true, true),
        "close" => ed.close_window(),
        "only" => {
            let survivor_window = ed.windows.get(ed.active_window).cloned();
            let survivor_terminal = survivor_window.as_ref().and_then(|w| w.terminal);
            let to_kill: Vec<u64> = ed
                .windows
                .iter()
                .filter_map(|w| w.terminal)
                .filter(|id| Some(*id) != survivor_terminal)
                .collect();
            for id in to_kill {
                ed.shutdown_terminal(id);
            }
            // A surviving terminal pane has no buffer/cursor to fall back
            // to the way a plain buffer pane does once `windows` is empty
            // (see draw()'s `ed.windows.is_empty()` case), so it must stay
            // a real (single-entry) window rather than being dropped too.
            match survivor_window.filter(|_| survivor_terminal.is_some()) {
                Some(w) => {
                    ed.windows = vec![w];
                    ed.window_layout = Some(crate::windows::Layout::Leaf(0));
                    ed.active_window = 0;
                }
                None => {
                    ed.windows.clear();
                    ed.window_layout = None;
                    ed.active_window = 0;
                }
            }
        }
        "set" => match rest.trim() {
            "wrap" => ed.config.wrap = true,
            "nowrap" => ed.config.wrap = false,
            "number" => ed.config.number = true,
            "nonumber" => ed.config.number = false,
            _ => ed.set_message("Supported: wrap nowrap number nonumber"),
        },
        "configreload" => {
            ed.config = crate::config::Config::load();
            ed.restart_lsp();
            ed.set_message("Config reloaded");
        }
        "b" | "buffer" => {
            if let Ok(n) = rest.trim().parse::<usize>() {
                if n > 0 && n <= ed.buffers.len() {
                    ed.cur = n - 1;
                }
            } else {
                ed.show_buffers();
            }
        }
        "w!" => {
            let result = if ed.buf().note_id.is_some() {
                ed.save_current()
            } else {
                ed.buf_mut().save_force()
            };
            ed.set_message(match result {
                Ok(()) => "Written".into(),
                Err(e) => e.to_string(),
            });
        }
        "w" | "write" => {
            let target = rest.trim();
            let result = if target.is_empty() {
                ed.save_current()
            } else {
                ed.buf_mut().save_as(PathBuf::from(target))
            };
            match result {
                Ok(()) => ed.set_message(format!("\"{}\" written", ed.buf().name())),
                Err(e) => ed.set_message(format!("save failed: {}", e)),
            }
        }
        "q" | "quit" => {
            if let Some(msg) = modified_buffers_message(ed) {
                ed.set_message(msg);
            } else {
                close_current_or_quit(ed);
            }
        }
        "q!" | "quit!" => close_current_or_quit(ed),
        "wq" | "x" => match ed.save_current() {
            Ok(()) => {
                if let Some(msg) = modified_buffers_message(ed) {
                    ed.set_message(msg);
                } else {
                    close_current_or_quit(ed);
                }
            }
            Err(e) => ed.set_message(format!("save failed: {}", e)),
        },
        "qa" | "qall" | "quitall" => {
            if let Some(msg) = modified_buffers_message(ed) {
                ed.set_message(msg);
            } else {
                ed.should_quit = true;
            }
        }
        "qa!" | "qall!" => ed.should_quit = true,
        "wqa" | "wqall" | "xa" => {
            let mut failed: Vec<String> = Vec::new();
            let current = ed.cur;
            for i in 0..ed.buffers.len() {
                ed.cur = i;
                if ed.buf().is_modified() || ed.buf().path.is_some() {
                    if let Err(e) = ed.save_current() {
                        failed.push(format!("{}: {}", ed.buf().name(), e));
                    }
                }
            }
            ed.cur = current;
            if ed.notes.dirty {
                if let Err(e) = ed.notes.save() {
                    failed.push(e.to_string());
                }
            }
            if failed.is_empty() {
                ed.should_quit = true;
            } else {
                ed.set_message(format!(
                    "save failed, not quitting -- {}",
                    failed.join("; ")
                ));
            }
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
        "ls" | "buffers" => ed.show_buffers(),
        "blines" => ed.show_buffer_lines(),
        "b#" => ed.switch_to_alternate(),
        "bn" | "bnext" => {
            if ed.buffers.len() > 1 {
                ed.note_alternate_buffer();
                ed.cur = (ed.cur + 1) % ed.buffers.len();
                ed.touch_buffer_mru(ed.buffers[ed.cur].id);
            }
        }
        "bp" | "bprev" | "bprevious" => {
            if ed.buffers.len() > 1 {
                ed.note_alternate_buffer();
                ed.cur = (ed.cur + ed.buffers.len() - 1) % ed.buffers.len();
                ed.touch_buffer_mru(ed.buffers[ed.cur].id);
            }
        }
        "bd" | "bdelete" => {
            if ed.buf().is_modified() {
                ed.set_message("unsaved changes -- use :bd! to discard");
            } else {
                remove_current_buffer(ed);
            }
        }
        "bd!" | "bdelete!" => remove_current_buffer(ed),
        _ if name.starts_with('b') && name[1..].parse::<usize>().is_ok() => {
            let n: usize = name[1..].parse().unwrap();
            if n >= 1 && n <= ed.buffers.len() && n - 1 != ed.cur {
                ed.note_alternate_buffer();
                ed.cur = n - 1;
                ed.touch_buffer_mru(ed.buffers[ed.cur].id);
            }
        }
        _ if name.starts_with('s') => run_substitute(ed, cmd),
        _ if cmd.starts_with('%') && cmd[1..].trim_start().starts_with('s') => {
            run_substitute(ed, cmd)
        }
        _ => ed.set_message(format!("E492: not an editor command: {}", cmd)),
    }
}

/// There is no split-window concept in Vaayu -- one viewport, N buffers --
/// `None` if no buffer has unsaved changes; otherwise a message naming them,
/// for a non-forced quit to refuse on. Every quit path checks *all* buffers,
/// not just the current one -- since Vaayu has no window-split concept, a
/// plain `:q` used to quit the whole process while silently discarding any
/// other modified buffer that happened to be loaded in the background (e.g.
/// edit buffer A, switch to clean buffer B, `:q`).
fn modified_buffers_message(ed: &Editor) -> Option<String> {
    let mut names: Vec<String> = ed
        .buffers
        .iter()
        .filter(|b| b.is_modified())
        .map(|b| b.name())
        .collect();
    if ed.notes.dirty {
        names.push("private comments".into());
    }
    if names.is_empty() {
        None
    } else {
        Some(format!(
            "unsaved changes in {} -- use :qa! to discard or :wqa to save all",
            names.join(", ")
        ))
    }
}

/// so `:q`/`:wq` always quit the process (after the modified-check the
/// caller already did), the way real Vim's `:q` quits when it's the last
/// window regardless of how many other buffers are loaded in the background.
/// Closing just the current buffer without quitting is `:bd`, handled
/// separately.
fn close_current_or_quit(ed: &mut Editor) {
    if ed.windows.len() > 1 {
        ed.close_window();
        return;
    }
    ed.should_quit = true;
}

fn remove_current_buffer(ed: &mut Editor) {
    ed.buffers.remove(ed.cur);
    if ed.buffers.is_empty() {
        ed.buffers.push(crate::buffer::Buffer::empty());
        ed.cur = 0;
    } else if ed.cur >= ed.buffers.len() {
        ed.cur = ed.buffers.len() - 1;
    }
    let ids: Vec<_> = ed.buffers.iter().map(|b| b.id).collect();
    ed.windows.retain(|w| ids.contains(&w.buffer));
    ed.buffer_mru.retain(|id| ids.contains(id));
    ed.active_window = ed.active_window.min(ed.windows.len().saturating_sub(1));
    ed.invalidate_index_caches();
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
    let mut parts = vec![String::new()];
    let mut chars = body[delim.len_utf8()..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&delim) {
            parts.last_mut().unwrap().push(chars.next().unwrap());
        } else if c == '\\' {
            parts.last_mut().unwrap().push(c);
            if let Some(next) = chars.next() {
                parts.last_mut().unwrap().push(next);
            }
        } else if c == delim {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    if parts.len() < 2 {
        ed.set_message("E486: incomplete substitute");
        return;
    }
    let pattern = crate::vimregex::translate_pattern(&parts[0]);
    let replacement = crate::vimregex::translate_replacement(&parts[1]);
    let flags = parts.get(2).map(String::as_str).unwrap_or("");
    if flags.chars().any(|c| !matches!(c, 'g' | 'i' | 'I')) {
        ed.set_message("unsupported substitute flag (supported: g i I)");
        return;
    }
    let global = flags.contains('g');

    let re = match fancy_regex::RegexBuilder::new(&pattern)
        .backtrack_limit(100_000)
        .case_insensitive(
            flags.contains('i')
                || (!flags.contains('I')
                    && ed.config.ignorecase
                    && !(ed.config.smartcase && pattern.chars().any(|c| c.is_uppercase()))),
        )
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

    let mut plan = Vec::new();
    for line in start_line..=end_line.min(ed.buf().line_count().saturating_sub(1)) {
        let text = ed.buf().line_text(line);
        let new_text =
            match re.try_replacen(&text, if global { 0 } else { 1 }, replacement.as_str()) {
                Ok(s) => s.into_owned(),
                Err(e) => {
                    ed.set_message(format!("Substitution failed: {e}"));
                    return;
                }
            };
        plan.push((line, text, new_text));
    }
    ed.buf_mut().begin_edit();
    let mut replaced_any = false;
    for (line, text, new_text) in plan.into_iter().rev() {
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
