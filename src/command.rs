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

/// The `:commands` picker source: one canonical name per ex command (the
/// full word where one exists, e.g. `quit` not `q` -- a browse/select UI
/// benefits from a clear name, unlike typing where short aliases save
/// keystrokes) paired with a one-line description, mirroring `:keymaps`'
/// existing `ACTIONS` registry for leader bindings. Kept here, next to
/// `run_ex`'s own match, as the single place both are maintained; there's
/// no compile-time link between the two (a name here could in principle
/// drift from the match below), the same soft-drift tradeoff `:keymaps`
/// already accepts for `ACTIONS`.
pub const EX_COMMANDS: &[(&str, &str)] = &[
    ("gitdiff", "Current file's saved diff as navigable results"),
    ("gitstage", "Saved unstaged hunks; Enter stages one hunk"),
    ("gitunstage", "Staged hunks; Enter unstages one hunk"),
    ("gitblame", "Current file's blame as navigable results"),
    ("gitstash", "Stash list; Enter shows a stash's diff"),
    ("gitstashpush", "Stash all current tracked changes"),
    (
        "gitstatus",
        "Git workspace: staged/unstaged/untracked/conflicts",
    ),
    ("gitcommit", "Commit staged changes with a message"),
    ("gitcommitamend", "Amend the last commit"),
    ("gitlog", "Commit log; Enter shows a commit's diff"),
    ("gitbranch", "Local branches; Enter checks one out"),
    ("gitpush", "Push the current branch"),
    ("gitpull", "Pull the current branch"),
    ("gitfetch", "Fetch from the remote"),
    ("lazygit", "Open lazygit in an embedded terminal"),
    (
        "claude",
        "Start/toggle a long-lived Claude terminal session",
    ),
    ("codex", "Start/toggle a long-lived Codex terminal session"),
    (
        "agent",
        "Start/toggle a long-lived agent terminal session by name",
    ),
    ("agents", "List running agent sessions; Enter attaches one"),
    ("permalink", "Copy a GitHub permalink for the cursor line"),
    ("recover", "Browse source drafts from interrupted sessions"),
    ("reviewrun", "Run the configured agent review command"),
    ("reviewcancel", "Cancel an in-progress agent review"),
    ("reviewresolve", "Mark the current review comment resolved"),
    ("reviewresults", "Reopen the last agent review output"),
    ("reviewexport", "Export a review packet to a file"),
    ("sessionsave", "Save the current tab/window layout"),
    ("sessionload", "Restore the last saved session"),
    ("help", "Search this editor's own help text"),
    ("keymaps", "List leader-key bindings; Enter runs one"),
    ("commands", "List ex commands; Enter fills the command line"),
    (
        "everything",
        "Keymaps, commands and recent projects combined",
    ),
    ("comments", "List private review comments"),
    ("comment", "Add a comment anchored to the cursor line"),
    ("commentfile", "Add a comment anchored to the whole file"),
    ("commentswrite", "Save comment edits and relocated anchors"),
    ("copen", "Reopen the quickfix list"),
    ("cclose", "Close the quickfix list"),
    ("cnext", "Next quickfix location"),
    ("cprev", "Previous quickfix location"),
    ("colder", "Switch to the previous quickfix list"),
    ("cnewer", "Switch to the next quickfix list"),
    ("grep", "Live grep for a pattern"),
    ("diagnostics", "Shared diagnostics list"),
    ("outline", "Document symbols as navigable results"),
    ("documentlinks", "Document links; Enter opens or copies one"),
    (
        "codelens",
        "Code lenses; Enter runs one (also shown as virtual text)",
    ),
    ("inlayhints", "Show inlay hints inline; Esc clears them"),
    (
        "projects",
        "Recently launched-from directories; Enter switches",
    ),
    ("references", "References to the symbol under the cursor"),
    (
        "typedefinition",
        "Type definition of the symbol under the cursor",
    ),
    (
        "implementation",
        "Implementation of the symbol under the cursor",
    ),
    ("declaration", "Declaration of the symbol under the cursor"),
    ("workspacesymbols", "Workspace symbol search"),
    ("format", "Format the buffer (or Visual selection)"),
    ("rename", "Rename the symbol under the cursor across files"),
    ("codeactions", "List and apply a code action"),
    (
        "organizeimports",
        "Apply the server's organize-imports action",
    ),
    ("signature", "Signature help at the cursor"),
    ("lsprestart", "Restart language servers for this buffer"),
    ("lspinfo", "Show language server status"),
    (
        "tools",
        "Known language servers: health, and Enter to install one",
    ),
    ("lspcancel", "Cancel outstanding language requests"),
    ("spellcheck", "Toggle spell-check underlines"),
    ("indentinfo", "Show detected/configured indent settings"),
    ("terminal", "Open an embedded terminal pane"),
    ("treenew", "Create a new file/directory in the file tree"),
    ("treerename", "Rename the file tree's selected path"),
    ("tabnew", "Open a new tab"),
    ("tabclose", "Close the current tab"),
    ("tabonly", "Close every tab except this one"),
    ("tabnext", "Next tab"),
    ("tabprevious", "Previous tab"),
    ("chistory", "Command-line history; Enter reruns one"),
    ("shistory", "Search history; Enter reruns one"),
    ("jumps", "Jump list as navigable results"),
    ("resume", "Reopen the last picker or Results/quickfix list"),
    ("treebookmarks", "List file tree bookmarks"),
    ("tabs", "List open tabs"),
    ("vsplit", "Split the window vertically"),
    ("vpreview", "Open a Markdown preview split"),
    ("close", "Close the current window/pane"),
    ("only", "Close every window except this one"),
    ("set", "Toggle wrap/number (nowrap/nonumber to disable)"),
    ("configreload", "Reload config.toml and restart servers"),
    ("buffer", "Switch to buffer N, or list buffers"),
    ("write", "Save the current buffer (or Save As <path>)"),
    ("quit", "Close the window, or quit if it's the last one"),
    ("wq", "Save and quit"),
    ("quitall", "Quit, refusing if any buffer is unsaved"),
    ("wqall", "Save every buffer, then quit"),
    ("nohlsearch", "Clear search-match highlighting"),
    ("edit", "Open a file by path"),
    ("buffers", "List open buffers"),
    (
        "blines",
        "Current buffer's non-blank lines as navigable results",
    ),
    ("b#", "Switch to the alternate buffer"),
    ("bnext", "Next buffer"),
    ("bprevious", "Previous buffer"),
    ("bdelete", "Close the current buffer"),
];
pub fn run_ex(ed: &mut Editor, raw: &str) {
    let cmd = raw.trim();
    if cmd.is_empty() {
        return;
    }

    // `:` entered from Visual mode leaves `visual_anchor` set; consume it
    // so a range-taking command (`:s`) with no explicit range defaults to
    // the selected line range. Taken unconditionally so a stale anchor
    // can't leak into a later command.
    let cur_line = ed.cursor().0;
    let visual_range = ed
        .visual_anchor
        .take()
        .map(|(al, _)| (al.min(cur_line), al.max(cur_line)));

    // Parse a leading Ex address/range (`.`, `$`, `N`, `+N`/`-N`, `'m`,
    // `%`, and `a,b` ranges) off the front, resolving each to a 0-based
    // line index.
    let (range, remainder) = match parse_range(ed, cmd) {
        Ok(v) => v,
        Err(e) => {
            ed.set_message(e);
            return;
        }
    };
    let remainder = remainder.trim();

    // A bare address with no command moves the cursor to its last line
    // (`:$`, `:.`, `:+3`, `:5`).
    if remainder.is_empty() {
        if let Some((_, end)) = range {
            let line = end.min(ed.buf().line_count().saturating_sub(1));
            let col = ed.buf().first_non_blank(line);
            ed.set_cursor(line, col);
        }
        return;
    }

    // An explicit range wins; otherwise a Visual selection supplies one.
    let effective_range = range.or(visual_range);

    let (name, rest) = split_command(remainder);
    match name {
        "gitdiff" => ed.git_results("diff"),
        "gitstage" => ed.git_results("stage"),
        "gitunstage" => ed.git_results("unstage"),
        "gitblame" => ed.git_results("blame"),
        "gitstash" => ed.show_git_stash(),
        "gitstashpush" => ed.git_stash_push(),
        "gitstatus" => ed.open_git_status(),
        "gitcommit" => ed.git_commit(rest.trim(), false),
        "gitcommitamend" => ed.git_commit(rest.trim(), true),
        "gitlog" => ed.git_log(),
        "gitbranch" => ed.git_branches(),
        "gitpush" => ed.git_push(),
        "gitpull" => ed.git_pull(),
        "gitfetch" => ed.git_fetch(),
        "lazygit" => ed.open_lazygit(),
        "claude" => ed.toggle_agent_session("claude"),
        "codex" => ed.toggle_agent_session("codex"),
        "agent" => {
            let name = rest.trim();
            if name.is_empty() {
                ed.set_message("Usage: :agent <name>");
            } else {
                ed.toggle_agent_session(name);
            }
        }
        "agents" => ed.list_agent_sessions(),
        "permalink" => {
            if let Some(path) = ed.buf().path.clone() {
                let line = ed.cursor().0;
                ed.generate_permalink(&path, line, line, None);
            } else {
                ed.set_message("This buffer has no file on disk");
            }
        }
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
        "commands" => {
            let mut entries: Vec<_> = EX_COMMANDS
                .iter()
                .map(|(name, desc)| {
                    let mut e =
                        crate::results::Entry::text(format!("{:<18} {}", format!(":{name}"), desc));
                    e.action = Some(serde_json::json!({"_vaayu_prefill_ex": format!("{name} ")}));
                    e
                })
                .collect();
            entries.sort_by(|a, b| a.text.cmp(&b.text));
            ed.show_results(crate::results::Results::new("Commands", entries));
        }
        "everything" => {
            // One combined picker over every built-in source that already
            // has its own action-tag handling in `results.rs::open_result`
            // (`:keymaps`' `_vaayu_action_id`, `:commands`' `_vaayu_prefill_ex`,
            // `:projects`' `_vaayu_switch_project`) -- reusing those tags
            // outright means this needed zero new dispatch logic, just
            // building one list from the same three sources those
            // commands already build separately. Grouped by category
            // (not one alphabetical sort across all of them, which would
            // just interleave unrelated things), each already in its own
            // sensible order; `f` (Results filtering, see Phase 2 item 6)
            // is what actually makes searching across all of them at once
            // useful.
            let mut entries: Vec<_> = crate::actions::ACTIONS
                .iter()
                .map(|a| {
                    let mut e = crate::results::Entry::text(format!(
                        "[keymap]  {}{:<6} {}",
                        ed.config.leader, a.keys, a.title
                    ));
                    e.action = Some(serde_json::json!({"_vaayu_action_id": a.id}));
                    e
                })
                .collect();
            entries.sort_by(|a, b| a.text.cmp(&b.text));
            let mut commands: Vec<_> = EX_COMMANDS
                .iter()
                .map(|(name, desc)| {
                    let mut e = crate::results::Entry::text(format!(
                        "[command] {:<18} {}",
                        format!(":{name}"),
                        desc
                    ));
                    e.action = Some(serde_json::json!({"_vaayu_prefill_ex": format!("{name} ")}));
                    e
                })
                .collect();
            commands.sort_by(|a, b| a.text.cmp(&b.text));
            entries.append(&mut commands);
            entries.extend(
                crate::projects::load_recent_projects()
                    .into_iter()
                    .filter(|p| p != &ed.project_root)
                    .map(|p| {
                        let mut e =
                            crate::results::Entry::text(format!("[project] {}", p.display()));
                        e.action = Some(serde_json::json!({"_vaayu_switch_project": p}));
                        e
                    }),
            );
            ed.show_results(crate::results::Results::new("Everything", entries));
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
        "documentlinks" => ed.request_language("documentLinks", None),
        "codelens" => ed.request_language("codeLens", None),
        "inlayhints" => ed.request_language("inlayHints", None),
        "projects" => ed.show_recent_projects(),
        "references" => ed.request_language("references", None),
        "typedefinition" => ed.request_type_definition(),
        "implementation" => ed.request_implementation(),
        "declaration" => ed.request_declaration(),
        "workspacesymbols" => ed.request_workspace_symbols(rest.trim()),
        "format" => ed.request_language("format", None),
        "rename" => ed.request_language("rename", Some(rest.trim())),
        "codeactions" => ed.request_language("actions", None),
        "organizeimports" => ed.request_language("organizeImports", None),
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
        "tools" => ed.show_tools(),
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
                // Same MRU/alternate bookkeeping as the spaceless `:b3`
                // form below, so `:b 3` doesn't leave the alternate buffer
                // and MRU order stale.
                if n >= 1 && n <= ed.buffers.len() && n - 1 != ed.cur {
                    ed.note_alternate_buffer();
                    ed.cur = n - 1;
                    ed.touch_buffer_mru(ed.buffers[ed.cur].id);
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
                if ed.buf().is_modified() {
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
        "e!" | "edit!" => {
            let target = rest.trim();
            if target.is_empty() {
                // Bare `:e!`: reload the current buffer from disk,
                // discarding in-memory changes -- real Vim's own
                // behavior. A target path (`:e! other.txt`) isn't given
                // the same "discard and force" treatment as a plain
                // `:e other.txt` would need, since that's a separate,
                // unrelated feature this doesn't otherwise need.
                match ed.buf_mut().reload() {
                    Ok(()) => ed.set_message("Reloaded"),
                    Err(e) => ed.set_message(format!("could not reload: {e}")),
                }
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
        // Only real substitute syntax (`:s` followed by a non-alphanumeric
        // delimiter, or a bare `:s`) routes here -- `:sort`/`:set`/`:sp`
        // and other unknown `s...` commands fall through to the error below.
        _ if is_substitute(name) => run_substitute(ed, remainder, effective_range),
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

/// True when `name` is a substitute command: `:s` on its own, or `s`
/// followed by a non-alphanumeric delimiter (`s/`, `s#`, `s|`, ...).
/// Anything else that merely starts with `s` (`:sort`, `:set`, `:sp`,
/// `:setlocal`) is not a substitute and must not be routed there.
fn is_substitute(name: &str) -> bool {
    let mut chars = name.chars();
    if chars.next() != Some('s') {
        return false;
    }
    match chars.next() {
        None => true,
        Some(c) => !c.is_alphanumeric(),
    }
}

/// Parses an optional leading Ex address or range off the front of `cmd`,
/// resolving each address to a 0-based line index. Returns the inclusive
/// `(start, end)` range (or `None` when no address was present) together
/// with the remainder of the command line after the range. `%` expands to
/// the whole buffer (`1,$`).
fn parse_range(ed: &Editor, cmd: &str) -> Result<(Option<(usize, usize)>, String), String> {
    let chars: Vec<char> = cmd.chars().collect();
    let last = ed.buf().line_count().saturating_sub(1);
    let mut i = 0;
    while matches!(chars.get(i), Some(' ') | Some('\t')) {
        i += 1;
    }
    if chars.get(i) == Some(&'%') {
        i += 1;
        return Ok((Some((0, last)), chars[i..].iter().collect()));
    }
    let cur = ed.cursor().0;
    let Some((a1, ni)) = parse_one_address(ed, &chars, i, cur)? else {
        return Ok((None, cmd.to_string()));
    };
    i = ni;
    if matches!(chars.get(i), Some(',') | Some(';')) {
        // `;` rebases the second address (its `.` and any offset) on the
        // first; `,` keeps them relative to the real current line.
        let base = if chars[i] == ';' { a1 } else { cur };
        i += 1;
        match parse_one_address(ed, &chars, i, base)? {
            Some((a2, ni2)) => {
                i = ni2;
                let (lo, hi) = if a1 <= a2 { (a1, a2) } else { (a2, a1) };
                return Ok((Some((lo, hi)), chars[i..].iter().collect()));
            }
            None => return Ok((Some((a1, a1)), chars[i..].iter().collect())),
        }
    }
    Ok((Some((a1, a1)), chars[i..].iter().collect()))
}

/// Parses one address starting at `chars[i]`, using `cur` as the value of
/// `.` and the base for a leading `+`/`-` offset. Returns the resolved
/// 0-based line and the index just past the address, `Ok(None)` when there
/// is no address here, or `Err` for a malformed one (e.g. an unset mark).
fn parse_one_address(
    ed: &Editor,
    chars: &[char],
    mut i: usize,
    cur: usize,
) -> Result<Option<(usize, usize)>, String> {
    let last = ed.buf().line_count().saturating_sub(1) as i64;
    let start = i;
    let mut line: Option<i64> = None;
    match chars.get(i) {
        Some('.') => {
            line = Some(cur as i64);
            i += 1;
        }
        Some('$') => {
            line = Some(last);
            i += 1;
        }
        Some('\'') => {
            let Some(&m) = chars.get(i + 1) else {
                return Err("E20: mark not set".into());
            };
            match ed.marks.get(&m) {
                Some(loc) => {
                    line = Some(loc.line as i64);
                    i += 2;
                }
                None => return Err(format!("E20: mark not set: {m}")),
            }
        }
        Some(c) if c.is_ascii_digit() => {
            let mut n: i64 = 0;
            while let Some(d) = chars.get(i).and_then(|c| c.to_digit(10)) {
                n = n * 10 + d as i64;
                i += 1;
            }
            line = Some(n - 1);
        }
        _ => {}
    }
    while matches!(chars.get(i), Some('+') | Some('-')) {
        let sign = if chars[i] == '+' { 1i64 } else { -1 };
        i += 1;
        let mut n: i64 = 0;
        let mut had = false;
        while let Some(d) = chars.get(i).and_then(|c| c.to_digit(10)) {
            n = n * 10 + d as i64;
            i += 1;
            had = true;
        }
        if !had {
            n = 1;
        }
        let base = line.unwrap_or(cur as i64);
        line = Some(base + sign * n);
    }
    if i == start {
        return Ok(None);
    }
    let resolved = line.unwrap_or(cur as i64).clamp(0, last) as usize;
    Ok(Some((resolved, i)))
}

/// Handles `:s/pat/repl/flags`. `range` is the resolved 0-based inclusive
/// line range to operate on (from an Ex range/`%`/Visual selection), or
/// `None` for the current line only.
fn run_substitute(ed: &mut Editor, body: &str, range: Option<(usize, usize)>) {
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
    let pattern = if parts[0].is_empty() {
        // An empty pattern reuses the last search pattern (Vim behavior),
        // not an empty regex that matches at every position. `last_search`
        // already holds the translated pattern, so use it as-is.
        let Some((p, _)) = ed.last_search.clone() else {
            ed.set_message("E35: no previous regular expression");
            return;
        };
        p
    } else {
        crate::vimregex::translate_pattern(&parts[0])
    };
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

    let (start_line, end_line) = match range {
        Some((s, e)) => (s, e),
        None => (ed.cursor().0, ed.cursor().0),
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
