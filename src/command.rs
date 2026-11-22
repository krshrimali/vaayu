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

    let is_search = matches!(kind, CommandKind::SearchFwd | CommandKind::SearchBack);
    // Any key other than Tab/BackTab ends a completion cycle.
    let completing = matches!(key, Key::Tab | Key::BackTab);

    // Esc always cancels immediately, matching real Vim/Neovim's own
    // command line: unlike the picker/results query bars (`crate::
    // queryline`'s Insert/Normal sub-mode), this one is deliberately
    // never modal, so Esc is intercepted here rather than reaching
    // `queryline::handle` below (whose own Esc would otherwise just drop
    // to a Normal sub-mode this prompt doesn't have).
    if key == Key::Esc {
        ed.cmdline.clear();
        ed.cancel_incsearch();
        cancel_search_operator(ed, is_search);
        ed.enter_normal();
        ed.cmdline_completions.clear();
        ed.cmdline_completion_index = None;
        return;
    }
    // Backspace on an already-empty line exits (unchanged from before);
    // a non-empty line's Backspace goes through `queryline` below, which
    // deletes at the cursor rather than always the last character.
    if key == Key::Backspace && ed.cmdline.is_empty() {
        ed.cancel_incsearch();
        cancel_search_operator(ed, is_search);
        ed.enter_normal();
        ed.cmdline_completions.clear();
        ed.cmdline_completion_index = None;
        return;
    }
    // `cmdline_qcursor.insert` never goes false here (Esc never reaches
    // it), so this only ever exercises `queryline`'s Insert-sub-mode
    // editing -- cursor motion, mid-string insert/delete, Ctrl-w/Ctrl-u --
    // giving the command line a real cursor for the first time (it used
    // to only ever append/pop the last character) without changing what
    // typing a letter like `h`/`w`/`b` does.
    if let Some(changed) = crate::queryline::handle(&mut ed.cmdline, &mut ed.cmdline_qcursor, key) {
        if changed {
            on_cmdline_changed(ed, kind);
        }
        if !completing {
            ed.cmdline_completions.clear();
            ed.cmdline_completion_index = None;
        }
        return;
    }
    match key {
        Key::Tab => {
            cmdline_complete(ed, kind, true);
            on_cmdline_changed(ed, kind);
        }
        Key::BackTab => {
            cmdline_complete(ed, kind, false);
            on_cmdline_changed(ed, kind);
        }
        Key::Enter => {
            let line = ed.cmdline.clone();
            ed.cmdline.clear();
            // A live search runs from the origin, so <CR> lands on the same
            // match incsearch previewed; then clear the preview state.
            if is_search {
                if let Some((ol, oc, _, _)) = ed.search_origin.take() {
                    ed.set_cursor(ol, oc);
                }
            }
            ed.incsearch = None; // clear incsearch/inccommand preview highlight
            ed.sub_preview.clear();
            ed.enter_normal();
            match kind {
                CommandKind::Ex => {
                    push_history(&mut ed.command_history, &line);
                    run_ex(ed, &line);
                }
                CommandKind::SearchFwd | CommandKind::SearchBack => {
                    push_history(&mut ed.search_history, &line);
                    let origin = ed.cursor();
                    run_search(ed, &line, kind == CommandKind::SearchFwd);
                    // `d/pat`, `c?pat`: the search was the operator's motion.
                    if ed.pending.operator.is_some() {
                        crate::normal::finish_search_operator(ed, origin);
                    }
                }
            }
        }
        Key::Up | Key::Ctrl('p') => {
            history_step(ed, kind, true);
            on_cmdline_changed(ed, kind);
        }
        Key::Down | Key::Ctrl('n') => {
            history_step(ed, kind, false);
            on_cmdline_changed(ed, kind);
        }
        _ => {}
    }
    if !completing {
        ed.cmdline_completions.clear();
        ed.cmdline_completion_index = None;
    }
}

/// `:checkhealth`: a diagnostic report of external tools, configured language
/// servers, and built-in tree-sitter grammars, as a results list.
/// `:Log`: every logged line (rotated files first, oldest first) in a
/// pathless scratch buffer, cursor on the newest entry.
fn open_log(ed: &mut Editor) {
    let lines = match &ed.logger {
        Some(logger) => logger.read_all(),
        None => match crate::log::log_path() {
            Some(path) => crate::log::read_all(&path, ed.config.log_keep),
            None => Vec::new(),
        },
    };
    if lines.is_empty() {
        let hint = if ed.config.persist_messages {
            "No logged messages yet"
        } else {
            "No logged messages (persist_messages is off)"
        };
        ed.set_message(hint);
        return;
    }
    let mut buffer = crate::buffer::Buffer::empty();
    buffer.rope = ropey::Rope::from_str(&(lines.join("\n") + "\n"));
    buffer.mark_saved();
    buffer.view = true;
    let id = buffer.id;
    ed.buffers.push(buffer);
    ed.note_alternate_buffer();
    ed.cur = ed.buffers.len() - 1;
    ed.touch_buffer_mru(id);
    ed.invalidate_index_caches();
    let last = lines.len() - 1;
    ed.set_cursor(last, 0);
    ed.set_message(format!("Log · {} line(s)", lines.len()));
}

fn open_health(ed: &mut Editor) {
    use crate::results::Entry;
    let ok = |b: bool| if b { "✓" } else { "✗" };
    let mut entries = vec![Entry::text("── External tools ──".to_string())];
    for (label, bins) in [
        ("ripgrep (file discovery, live grep)", &["rg"][..]),
        ("git (git features)", &["git"][..]),
        ("lazygit (,gl)", &["lazygit"][..]),
        ("gh (GitHub workspace, ,G)", &["gh"][..]),
        ("clipboard", &["wl-copy", "xclip", "xsel"][..]),
    ] {
        let present = bins.iter().any(|b| crate::tools::on_path(b));
        entries.push(Entry::text(format!(
            "{} {} [{}]",
            ok(present),
            label,
            bins.join("/")
        )));
    }
    entries.push(Entry::text(ed.watcher_health()));
    entries.push(Entry::text(String::new()));
    entries.push(Entry::text(
        "── Configured language servers (see :tools) ──".to_string(),
    ));
    if ed.config.lsp.is_empty() {
        entries.push(Entry::text("  (none configured)".to_string()));
    } else {
        for (name, srv) in &ed.config.lsp {
            let bin = srv.cmd.first().cloned().unwrap_or_default();
            entries.push(Entry::text(format!(
                "{} {} [{}]",
                ok(crate::tools::on_path(&bin)),
                name,
                bin
            )));
        }
    }
    entries.push(Entry::text(String::new()));
    entries.push(Entry::text(
        "── Tree-sitter grammars (built-in) ──".to_string(),
    ));
    entries.push(Entry::text(
        "✓ rust python javascript typescript tsx go c bash json toml yaml lua vim css html solidity"
            .to_string(),
    ));
    ed.show_results(crate::results::Results::new("Health", entries));
}

/// Called whenever the command line's text changes: drive incsearch (`/`?`) or
/// the inccommand `:s` preview (Ex).
fn on_cmdline_changed(ed: &mut Editor, kind: CommandKind) {
    match kind {
        CommandKind::SearchFwd | CommandKind::SearchBack => ed.update_incsearch(),
        CommandKind::Ex => update_inccommand(ed),
    }
}

/// inccommand: if the in-progress Ex line is a substitute, highlight its pattern
/// live (reusing the incsearch highlight); otherwise clear the preview.
fn update_inccommand(ed: &mut Editor) {
    ed.incsearch = substitute_pattern(&ed.cmdline);
    ed.sub_preview = compute_sub_preview(ed);
}

/// Split a `s<delim>pat<delim>repl<delim>flags` body (leading `s` required,
/// delimiter must be non-alphanumeric) into raw `(pattern, replacement,
/// flags)`, honoring `\`-escaped delimiters. `None` unless it has at least a
/// pattern and a replacement field.
fn parse_substitute_body(body: &str) -> Option<(String, String, String)> {
    let rest = body.trim_start().strip_prefix('s')?;
    let delim = rest.chars().next()?;
    if delim.is_alphanumeric() {
        return None;
    }
    let mut parts = vec![String::new()];
    let mut chars = rest[delim.len_utf8()..].chars().peekable();
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
        return None;
    }
    let flags = parts.get(2).cloned().unwrap_or_default();
    Some((parts[0].clone(), parts[1].clone(), flags))
}

/// inccommand: compute the live substitute preview for the in-progress command
/// line — a map of line index to the text that line would become — mirroring
/// `run_substitute`'s regex/flags/capture-group handling. Empty when the line
/// isn't a valid, complete substitute. Bounded so a `%s` on a huge file stays
/// cheap on every keystroke.
fn compute_sub_preview(ed: &Editor) -> std::collections::HashMap<usize, String> {
    let mut out = std::collections::HashMap::new();
    let Ok((range, remainder)) = parse_range(ed, &ed.cmdline) else {
        return out;
    };
    let Some((raw_pat, raw_repl, flags)) = parse_substitute_body(remainder.trim_start()) else {
        return out;
    };
    if flags.chars().any(|c| !matches!(c, 'g' | 'i' | 'I')) {
        return out;
    }
    // An empty pattern would reuse last_search in run_substitute; skip previewing
    // that ambiguous case rather than guess.
    if raw_pat.is_empty() {
        return out;
    }
    let pattern = crate::vimregex::translate_pattern(&raw_pat);
    let replacement = crate::vimregex::translate_replacement(&raw_repl);
    let global = flags.contains('g');
    let case_insensitive = flags.contains('i')
        || (!flags.contains('I')
            && ed.config.ignorecase
            && !(ed.config.smartcase && raw_pat.chars().any(|c| c.is_uppercase())));
    let Ok(re) = fancy_regex::RegexBuilder::new(&pattern)
        .backtrack_limit(100_000)
        .case_insensitive(case_insensitive)
        .build()
    else {
        return out;
    };
    let last = ed.buf().line_count().saturating_sub(1);
    let (start, end) = range.unwrap_or_else(|| {
        let c = ed.cursor().0;
        (c, c)
    });
    // Cap the scan so a `%s` on a very large file doesn't run the regex over
    // every line on each keystroke; off-screen previews aren't rendered anyway.
    const MAX_PREVIEW_LINES: usize = 4000;
    for line in start..=end.min(last) {
        if out.len() >= MAX_PREVIEW_LINES {
            break;
        }
        let text = ed.buf().line_text(line);
        if let Ok(new_text) =
            re.try_replacen(&text, if global { 0 } else { 1 }, replacement.as_str())
        {
            if new_text != text {
                out.insert(line, new_text.into_owned());
            }
        }
    }
    out
}

/// Extract the pattern from an in-progress `[range]s/pat/...` line, tolerating a
/// leading range and any single-char delimiter. Returns None if it isn't a
/// substitute or the pattern is empty.
fn substitute_pattern(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != 's' {
            continue;
        }
        // Everything before the `s` must be a plausible range.
        let prefix_ok = chars[..i].iter().all(|c| {
            c.is_ascii_digit() || matches!(c, '.' | '$' | '%' | '+' | '-' | ',' | ';' | ' ')
        });
        if !prefix_ok {
            continue;
        }
        let &delim = chars.get(i + 1)?;
        if delim.is_alphanumeric() || delim == ' ' {
            continue; // e.g. "set" -- not a substitute
        }
        let mut pat = String::new();
        let mut j = i + 2;
        while j < chars.len() {
            if chars[j] == '\\' && j + 1 < chars.len() {
                pat.push(chars[j]);
                pat.push(chars[j + 1]);
                j += 2;
                continue;
            }
            if chars[j] == delim {
                break;
            }
            pat.push(chars[j]);
            j += 1;
        }
        // Translate to PCRE so the inccommand highlight matches what the actual
        // `:s` will substitute (run_substitute translates the same way).
        return if pat.is_empty() {
            None
        } else {
            Some(crate::vimregex::translate_pattern(&pat))
        };
    }
    None
}

/// Ex command-line Tab-completion (wildmenu). First Tab computes candidates for
/// the current token (command name, or a file-path argument for path-taking
/// commands) and applies the first; subsequent Tab/BackTab cycle.
fn cmdline_complete(ed: &mut Editor, kind: CommandKind, forward: bool) {
    if kind != CommandKind::Ex {
        return;
    }
    if ed.cmdline_completion_index.is_some() && !ed.cmdline_completions.is_empty() {
        let n = ed.cmdline_completions.len();
        let i = ed.cmdline_completion_index.unwrap();
        let ni = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        ed.cmdline_completion_index = Some(ni);
        ed.cmdline = ed.cmdline_completions[ni].clone();
        ed.cmdline_qcursor = crate::queryline::QueryCursor::at_end(&ed.cmdline);
        return;
    }
    let cands = compute_cmdline_candidates(ed);
    if cands.is_empty() {
        return;
    }
    ed.cmdline = cands[0].clone();
    ed.cmdline_qcursor = crate::queryline::QueryCursor::at_end(&ed.cmdline);
    ed.cmdline_completions = cands;
    ed.cmdline_completion_index = Some(0);
}

fn is_path_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "e" | "edit"
            | "e!"
            | "edit!"
            | "w"
            | "write"
            | "sav"
            | "saveas"
            | "tabnew"
            | "sp"
            | "split"
            | "vsp"
            | "vs"
            | "vsplit"
    )
}

fn compute_cmdline_candidates(ed: &Editor) -> Vec<String> {
    let line = ed.cmdline.clone();
    match line.find(' ') {
        None => {
            let mut v: Vec<String> = EX_COMMANDS
                .iter()
                .map(|(n, _)| n.to_string())
                .filter(|n| n.starts_with(&line))
                .collect();
            v.sort();
            v.dedup();
            v
        }
        Some(_) => {
            let cmd = line.split_whitespace().next().unwrap_or("");
            if matches!(cmd, "colorscheme" | "colo") {
                let sp = line.rfind(' ').unwrap();
                return crate::theme::complete(&line[sp + 1..])
                    .into_iter()
                    .map(|n| format!("{}{n}", &line[..=sp]))
                    .collect();
            }
            if cmd == "test" {
                let sp = line.rfind(' ').unwrap();
                return crate::testrun::SUBCOMMANDS
                    .iter()
                    .filter(|s| s.starts_with(&line[sp + 1..]))
                    .map(|s| format!("{}{s}", &line[..=sp]))
                    .collect();
            }
            let dirs_only = matches!(cmd, "cd" | "chdir");
            if !dirs_only && !is_path_command(cmd) {
                return Vec::new();
            }
            let sp = line.rfind(' ').unwrap();
            let before = &line[..=sp]; // includes the trailing space
            let token = &line[sp + 1..];
            path_candidates(ed, token)
                .into_iter()
                .filter(|p| !dirs_only || p.ends_with('/'))
                .map(|p| format!("{before}{p}"))
                .collect()
        }
    }
}

fn path_candidates(ed: &Editor, token: &str) -> Vec<String> {
    let (dir_part, prefix) = match token.rfind('/') {
        Some(i) => (&token[..=i], &token[i + 1..]),
        None => ("", token),
    };
    let home = dirs::home_dir();
    let base: PathBuf = if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else if let (Some(rest), Some(home)) = (dir_part.strip_prefix("~/"), home) {
        home.join(rest)
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ed.project_root.clone());
        if dir_part.is_empty() {
            cwd
        } else {
            cwd.join(dir_part)
        }
    };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix) {
                let is_dir = entry.path().is_dir();
                out.push(format!("{dir_part}{name}{}", if is_dir { "/" } else { "" }));
            }
        }
    }
    out.sort();
    out
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
    ed.cmdline_qcursor = crate::queryline::QueryCursor::at_end(&ed.cmdline);
}

/// An operator waiting on a `/`/`?` motion is dropped with the search.
fn cancel_search_operator(ed: &mut Editor, is_search: bool) {
    if is_search && ed.pending.operator.is_some() {
        ed.abort_change_recording();
        ed.pending.reset();
    }
}

/// Records a search pattern run without the command line (`*`, `#`), so
/// `/` + Up recalls it like a typed search.
pub(crate) fn push_search_history(ed: &mut Editor, pattern: &str) {
    push_history(&mut ed.search_history, pattern);
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
    (
        "gitfilehistory",
        "Current file's commit history; Enter shows a commit",
    ),
    (
        "gitrevert",
        "Revert a commit (default HEAD) as a new commit",
    ),
    (
        "gitcherrypick",
        "Cherry-pick a commit onto the current branch",
    ),
    ("gitbranch", "Local branches; Enter checks one out"),
    ("gitpush", "Push the current branch"),
    ("gitpull", "Pull the current branch"),
    ("gitfetch", "Fetch from the remote"),
    ("lazygit", "Open lazygit in an embedded terminal"),
    (
        "web",
        "Open this project in the browser editor (optional file)",
    ),
    ("ghprs", "GitHub pull requests (gh); Enter opens one"),
    ("ghpr", "GitHub PR overview (default: current branch's PR)"),
    (
        "ghcheckout",
        "Check out a GitHub PR locally (gh pr checkout)",
    ),
    ("ghdiff", "GitHub PR diff as navigable results"),
    ("ghthreads", "GitHub PR review threads as jumpable results"),
    ("ghchecks", "GitHub PR CI checks; Enter opens a job's log"),
    ("ghissues", "GitHub issues; Enter shows one"),
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
    (
        "ai",
        "AI prompt to the configured agent sidebar (code/selection + cursor + diagnostics)",
    ),
    ("aistart", "Open or focus the configured AI agent sidebar"),
    ("aistop", "Stop the configured AI agent and close its pane"),
    (
        "airestart",
        "Restart the configured AI agent in the sidebar",
    ),
    ("agentstop", "Stop a named agent session and close its pane"),
    (
        "termcapture",
        "Open the focused terminal's output (with history) in a buffer",
    ),
    ("permalink", "Copy a GitHub permalink for the cursor line"),
    ("recover", "Browse source drafts from interrupted sessions"),
    (
        "reviewrun",
        "Send the selected (or all open) review comments to the configured agent",
    ),
    ("reviewcancel", "Cancel an in-progress agent review"),
    (
        "reviewresolve",
        "Toggle resolved on the selected review comments",
    ),
    ("reviewresults", "Show the latest agent review output"),
    ("reviewruns", "Browse past agent review runs"),
    (
        "reviewexport",
        "Write the agent review packet to a file without running",
    ),
    ("reviews", "List the reviews in this folder; Enter switches"),
    ("reviewnew", "Start a new review: [--base REV] [title]"),
    ("reviewswitch", "Switch to a review by title or id"),
    ("reviewrename", "Rename the active review"),
    ("reviewclose", "Close the active review"),
    ("reviewreopen", "Reopen the active review"),
    (
        "reviewbase",
        "Set (or clear) the Git revision the review compares against",
    ),
    (
        "reviewfiles",
        "Files changed since the review base, with comment counts",
    ),
    (
        "reviewpublish",
        "Publish open comments as a pending GitHub PR review",
    ),
    (
        "reviewimport",
        "Import a GitHub PR's review threads as a new review",
    ),
    ("sessionsave", "Save the current tab/window layout"),
    ("sessionload", "Restore the last saved session"),
    ("help", "Search this editor's own help text"),
    ("keymaps", "List leader-key bindings; Enter runs one"),
    ("commands", "List ex commands; Enter fills the command line"),
    (
        "everything",
        "Keymaps, commands and recent projects combined",
    ),
    ("comments", "List the active review's comments"),
    (
        "comment",
        "Add a comment on the cursor line or Visual lines",
    ),
    ("commentfile", "Add a comment on the whole file"),
    ("commentreply", "Reply to the comment on the cursor line"),
    ("commentview", "Show the comment thread on the cursor line"),
    (
        "commentmove",
        "Re-anchor a comment to the cursor line / Visual lines",
    ),
    (
        "commentresolve",
        "Resolve or reopen the comment on the cursor line",
    ),
    (
        "commentswrite",
        "Retry pending comment writes and reload other sessions' changes",
    ),
    (
        "commentsreload",
        "Reload comments changed by other sessions",
    ),
    ("copen", "Reopen the quickfix list"),
    ("make", "Run a build/test command; output → quickfix"),
    (
        "test",
        "Run tests: :test [nearest|file|suite|last|output|stop|clear]",
    ),
    ("testnearest", "Run the test under the cursor"),
    ("testfile", "Run the current file's tests"),
    ("testsuite", "Run the whole test suite"),
    ("testlast", "Re-run the last test command"),
    ("testoutput", "Show the last test run's output"),
    ("teststop", "Stop the running tests"),
    ("testclear", "Clear the test pass/fail gutter marks"),
    (
        "taskwatch",
        "Re-run a command into the quickfix on every save",
    ),
    ("today", "Open today's personal task log"),
    ("taskview", "Open today's editable task log"),
    ("ystd", "Open yesterday's personal task log"),
    ("week", "Browse this week's task and activity entries"),
    (
        "tasklist",
        "Browse every saved task and activity entry, including future dates",
    ),
    ("on", "Open a task log date: :on YYYY-MM-DD"),
    (
        "taskadd",
        "Add a timestamped task, or open today's log when no text is given",
    ),
    ("tasknote", "Add a timestamped activity note"),
    ("tasknotes", "Browse all saved activity notes"),
    ("taskdone", "Mark the task entry under the cursor as done"),
    (
        "termsend",
        "Send the current line (or range) to a terminal (REPL)",
    ),
    (
        "diffthis",
        "Mark this buffer for diff mode (compare two buffers)",
    ),
    ("diffoff", "Turn off diff mode"),
    (
        "difffold",
        "Collapse unchanged regions in diff mode (:difffold [N])",
    ),
    (
        "conflictours",
        "Resolve the merge conflict here keeping our side",
    ),
    (
        "conflicttheirs",
        "Resolve the merge conflict here keeping their side",
    ),
    (
        "conflictboth",
        "Resolve the merge conflict here keeping both sides",
    ),
    ("conflictnext", "Jump to the next merge conflict"),
    ("conflictprev", "Jump to the previous merge conflict"),
    ("normal", "Run normal-mode keys (:normal[!] {keys})"),
    ("global", "Run a command on matching lines (:g/pat/cmd)"),
    (
        "vglobal",
        "Run a command on non-matching lines (:v/pat/cmd)",
    ),
    ("delete", "Delete the range or current line (:[range]d)"),
    ("sort", "Sort lines (:[range]sort[!] [u][n][i])"),
    ("move", "Move lines after {addr} (:[range]m {addr})"),
    ("copy", "Copy lines after {addr} (:[range]t {addr})"),
    ("join", "Join the range lines into one (:[range]j[!])"),
    (
        "yank",
        "Yank the range/current line into a register (:[range]y [reg])",
    ),
    (
        "put",
        "Put a register's lines below the range/current line (:[range]put [reg])",
    ),
    ("colorpick", "Report the hex color under the cursor"),
    (
        "colorlighten",
        "Lighten the hex color under the cursor (:colorlighten [pct])",
    ),
    (
        "colordarken",
        "Darken the hex color under the cursor (:colordarken [pct])",
    ),
    (
        "colorscheme",
        "Switch syntax colorscheme (:colorscheme [name])",
    ),
    ("zen", "Toggle zen/focus mode (hide gutter + status line)"),
    ("tours", "List .tours/*.tour code tours; Enter starts one"),
    ("tour", "Start a code tour (:tour [name])"),
    (
        "tournew",
        "Describe a tour; the AI agent generates it (:tournew [name] + :toursave)",
    ),
    (
        "toursave",
        "Send the :tournew prompt to the AI agent to generate the .tour",
    ),
    ("tourend", "Stop the active code tour (hide the step panel)"),
    (
        "tourrestart",
        "Restart the active code tour at its first step",
    ),
    (
        "toursteps",
        "List the active tour's steps; Enter jumps to one",
    ),
    (
        "tourexplain",
        "Ask the AI agent to explain the current tour step's code",
    ),
    ("tournext", "Next code-tour step"),
    ("tourprev", "Previous code-tour step"),
    (
        "tourhnext",
        "Next highlight within the current tour step (]h)",
    ),
    (
        "tourhprev",
        "Previous highlight within the current tour step ([h)",
    ),
    ("tourview", "Switch tour view (:tourview source|diff|flow)"),
    ("tourresume", "Resume a named tour (:tourresume [name])"),
    (
        "tourhelp",
        "Tour actions, focus, layouts and scratch-edit help",
    ),
    ("tourfocus", "Switch Source / Lesson focus (K)"),
    ("tourretry", "Retry loading the current tour source"),
    ("touredit", "End the tour and edit its definition"),
    (
        "tourexample",
        "Create a minimal .tours/example.tour to edit",
    ),
    (
        "tourlayout",
        "Tour layout (:tourlayout auto|horizontal|vertical)",
    ),
    (
        "tourcopystep",
        "Copy the current tour step and source metadata",
    ),
    ("tourcopyall", "Copy the entire tour as JSON"),
    (
        "tourmarkdown",
        "Render tour Markdown or edit raw text (:tourmarkdown [on|off|toggle])",
    ),
    (
        "tourindicators",
        "Toggle tour callouts and additional highlights",
    ),
    ("tourjump", "Jump to a tour annotation (:tourjump <id>)"),
    (
        "tourfollow",
        "Follow a tour connection (:tourfollow <number>)",
    ),
    (
        "tourcheck",
        "Validate tour paths, anchors and visuals (:tourcheck [name])",
    ),
    ("lopen", "Reopen the location list"),
    ("lnext", "Next location-list entry"),
    ("lprev", "Previous location-list entry"),
    (
        "ldiagnostics",
        "Fill the location list from this buffer's diagnostics",
    ),
    (
        "lgrep",
        "Grep the project into the location list (:lgrep <pattern>)",
    ),
    ("cclose", "Close the quickfix list"),
    ("cnext", "Next quickfix location"),
    ("cprev", "Previous quickfix location"),
    ("colder", "Switch to the previous quickfix list"),
    ("cnewer", "Switch to the next quickfix list"),
    ("grep", "Live grep for a pattern"),
    (
        "fold",
        "Fold a line range (:{range}fold); za/zo/zc/zd/zR/zM manage folds",
    ),
    (
        "foldindent",
        "Auto-fold by indentation into a nested overview",
    ),
    ("foldsyntax", "Auto-fold functions/classes via tree-sitter"),
    (
        "foldlsp",
        "Auto-fold via the language server's foldingRange",
    ),
    (
        "far",
        "Reviewed project-wide search & replace screen (toggle matches, then R)",
    ),
    (
        "cfar",
        "Find/replace across every file in the results list (cfar/pat/repl/g)",
    ),
    (
        "cfarpreview",
        "Preview cfar changes across the results files without applying",
    ),
    ("todo", "Index TODO/FIXME/HACK/XXX comments"),
    ("diagnostics", "Shared diagnostics list"),
    (
        "workspacediagnostics",
        "Pull project-wide diagnostics from the language server",
    ),
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
    (
        "cd",
        "Change directory and project root (~, - and Tab work)",
    ),
    ("pwd", "Show the current working directory"),
    ("references", "References to the symbol under the cursor"),
    (
        "callers",
        "Incoming calls (callers) of the function under the cursor",
    ),
    ("callees", "Outgoing calls of the function under the cursor"),
    (
        "linkededit",
        "Rename all linked ranges to a new name (no name = live)",
    ),
    (
        "linkedlive",
        "Live linked editing: type in one range, mirror the others",
    ),
    ("supertypes", "Supertypes of the type under the cursor"),
    ("subtypes", "Subtypes of the type under the cursor"),
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
    ("renameapply", "Apply a previewed rename (refactor_preview)"),
    (
        "renamecancel",
        "Discard a previewed rename (refactor_preview)",
    ),
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
    ("tree", "Toggle the file tree sidebar"),
    ("treefind", "Reveal the current file in the file tree"),
    ("treeroot", "Re-root the file tree (default: project root)"),
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
    ("registers", "Show registers"),
    ("marks", "Show marks (Enter jumps)"),
    ("messages", "Show recent messages"),
    (
        "Log",
        "Open every logged message (all sessions) in a buffer",
    ),
    (
        "checkhealth",
        "Health: external tools, LSP servers, grammars",
    ),
    ("earlier", "Undo N changes (:earlier [N])"),
    ("later", "Redo N changes (:later [N])"),
    ("undolist", "Undo history viewer (Enter jumps to a state)"),
    ("resume", "Reopen the last picker or Results/quickfix list"),
    ("treebookmarks", "List file tree bookmarks"),
    (
        "treesort",
        "Sort the file tree: name, type, mtime, size (none: next)",
    ),
    ("treetrash", "List .vaayu/trash; Enter restores an item"),
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
    // Consume the "command line was opened over a Results panel" flag for this
    // one command, so `:q` here dismisses the panel instead of quitting.
    let over_results = std::mem::take(&mut ed.cmdline_over_results);
    let cmd = raw.trim();
    if cmd.is_empty() {
        return;
    }

    // `:` entered from Visual mode leaves `visual_anchor` set; consume it
    // so a range-taking command (`:s`) with no explicit range defaults to
    // the selected line range. Taken unconditionally so a stale anchor
    // can't leak into a later command.
    let cur_line = ed.cursor().0;
    // A characterwise Visual selection's exact ends, for commands that use
    // more than its lines (`:ai`).
    let visual_chars = ed
        .visual_anchor
        .filter(|_| matches!(ed.last_visual, Some((crate::mode::VisualKind::Char, _, _))));
    let visual_chars = visual_chars.map(|a| crate::context::AiSelection::chars(a, ed.cursor()));
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
    // A comment composer saves into the review and closes its own pane,
    // so `:wq` / `:q` never quit the editor from it.
    if ed.buf().note.is_some() && !over_results {
        let composer = ed.buf().id;
        match name {
            "w" | "write" | "w!" | "write!" | "up" | "update" => {
                match ed.save_comment_buffer() {
                    Ok(m) => ed.set_message(m),
                    Err(e) => ed.set_message(e.to_string()),
                }
                return;
            }
            "wq" | "x" | "wq!" | "x!" | "xit" | "exit" => {
                match ed.save_comment_buffer() {
                    Ok(m) => {
                        if ed.buf().id == composer {
                            ed.close_composer(false);
                        }
                        ed.set_message(m);
                    }
                    Err(e) => ed.set_message(e.to_string()),
                }
                return;
            }
            "q" | "quit" | "clo" | "close" => {
                ed.close_composer(false);
                return;
            }
            "q!" | "quit!" | "clo!" | "close!" => {
                ed.close_composer(true);
                return;
            }
            _ => {}
        }
    }
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
        "gitfilehistory" | "gitfilelog" => ed.git_file_history(),
        "gitrevert" => ed.git_revert(rest.trim()),
        "gitcherrypick" => ed.git_cherry_pick(rest.trim()),
        "gitbranch" => ed.git_branches(),
        "gitpush" => ed.git_push(),
        "gitpull" => ed.git_pull(),
        "gitfetch" => ed.git_fetch(),
        "lazygit" => ed.open_lazygit(),
        "web" => crate::web::launch::ex_command(ed, rest.trim()),
        "ghprs" => ed.gh_pr_list(rest),
        "ghpr" | "ghcheckout" | "ghdiff" | "ghthreads" | "ghchecks" => {
            match crate::github::parse_pr_arg(rest) {
                Ok(number) => match name {
                    "ghpr" => ed.gh_pr_view(number),
                    "ghcheckout" => ed.gh_pr_checkout(number),
                    "ghdiff" => ed.gh_pr_diff(number),
                    "ghthreads" => ed.gh_pr_threads(number),
                    _ => ed.gh_pr_checks(number),
                },
                Err(e) => ed.set_message(format!("Usage: :{name} <number> — {e}")),
            }
        }
        "ghissues" => ed.gh_issue_list(rest),
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
        "ai" | "sidekick" => {
            // An explicit range is whole lines; a characterwise Visual
            // selection keeps its exact ends.
            let selection = match (range, visual_chars) {
                (None, Some(chars)) => Some(chars),
                _ => effective_range.map(|(s, e)| crate::context::AiSelection::lines(s, e)),
            };
            let a = rest.trim();
            if a.is_empty() {
                ed.open_ai_prompt_picker_for(selection);
            } else {
                ed.ai_prompt(a, selection);
            }
        }
        "aistart" => ed.open_ai_sidebar(),
        "aistop" => {
            let kind = ed.config.ai_agent().to_string();
            ed.stop_agent_session(&kind);
        }
        "airestart" => ed.restart_ai_agent(),
        "agentstop" => {
            let name = rest.trim();
            if name.is_empty() {
                ed.set_message("Usage: :agentstop <name>");
            } else {
                ed.stop_agent_session(name);
            }
        }
        "termcapture" | "termcap" => {
            ed.open_terminal_capture();
        }
        "permalink" => {
            if let Some(path) = ed.buf().path.clone() {
                let line = ed.cursor().0;
                ed.generate_permalink(&path, line, line, None);
            } else {
                ed.set_message("This buffer has no file on disk");
            }
        }
        "recover" => ed.show_recovery(),
        // Typed over a review list, these act on its selection.
        "reviewrun" if over_results => {
            ed.cmdline_over_results = true;
            ed.run_review();
            ed.cmdline_over_results = false;
        }
        "reviewexport" if over_results => {
            ed.cmdline_over_results = true;
            let result = ed.export_review();
            ed.cmdline_over_results = false;
            ed.set_message(match result {
                Ok(p) => format!("Review packet written to {}", p.display()),
                Err(e) => e.to_string(),
            });
        }
        "reviewrun" => ed.run_review(),
        "reviewcancel" => ed.cancel_review(),
        "lspcancel" => ed.cancel_language_requests(),
        "reviewresolve" => ed.resolve_review(),
        "reviewresults" => ed.show_review_results(),
        "reviewruns" => ed.review_runs(),
        "reviewexport" => {
            let result = ed.export_review();
            ed.set_message(match result {
                Ok(p) => format!("Review packet written to {}", p.display()),
                Err(e) => e.to_string(),
            });
        }
        "reviews" => ed.reviews_list(),
        "reviewnew" => ed.review_new(rest),
        "reviewswitch" => ed.review_switch_named(rest),
        "reviewrename" => ed.review_rename(rest),
        "reviewclose" => ed.review_set_status(crate::review_store::ReviewStatus::Closed),
        "reviewreopen" => ed.review_set_status(crate::review_store::ReviewStatus::Open),
        "reviewbase" => ed.review_set_base(rest),
        "reviewfiles" => ed.review_files(),
        "reviewpublish" | "reviewimport" => match crate::github::parse_pr_arg(rest) {
            Ok(number) if name == "reviewpublish" => ed.review_publish(number),
            Ok(number) => ed.review_import(number),
            Err(e) => ed.set_message(format!("Usage: :{name} [number] — {e}")),
        },
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
        "commentreply" => ed.reply_at_cursor(),
        "commentview" => ed.view_comments_at_cursor(),
        "commentmove" => ed.move_comment_here(),
        "commentresolve" => ed.resolve_at_cursor(),
        "commentswrite" | "commentsreload" => ed.review_sync(),
        "copen" => ed.open_quickfix(),
        "cclose" => ed.enter_normal(),
        "cnext" | "cn" => ed.quickfix_step(true),
        "cprev" | "cp" => ed.quickfix_step(false),
        "colder" | "col" => ed.quickfix_older(),
        "cnewer" | "cnew" => ed.quickfix_newer(),
        "lopen" => ed.open_loclist(),
        "lclose" => ed.enter_normal(),
        "lnext" | "lne" => ed.loclist_step(true),
        "lprev" | "lp" => ed.loclist_step(false),
        "ldiagnostics" | "ldiag" => ed.loclist_from_diagnostics(),
        "lgrep" => ed.lgrep(rest.trim()),
        "make" | "task" => ed.run_task(rest.trim()),
        "today" => ed.task_today(),
        "taskview" => ed.task_today(),
        "ystd" | "yesterday" => ed.task_yesterday(),
        "week" => ed.task_week(),
        "tasklist" => ed.task_list(),
        "tasknotes" => ed.task_notes(),
        "on" => match chrono::NaiveDate::parse_from_str(rest.trim(), "%Y-%m-%d") {
            Ok(date) => ed.task_open_day(date),
            Err(_) => ed.set_message("Usage: :on YYYY-MM-DD"),
        },
        "taskadd" => ed.task_add(rest.trim(), "task"),
        "tasknote" => ed.task_add(rest.trim(), "note"),
        "taskdone" | "taskclose" => ed.task_done(),
        "testnearest" | "testfn" => ed.test_run(crate::testrun::Scope::Nearest),
        "testfile" => ed.test_run(crate::testrun::Scope::File),
        "testsuite" => ed.test_run(crate::testrun::Scope::Suite),
        "testlast" => ed.test_last(),
        "testoutput" => ed.test_output(),
        "teststop" => ed.test_stop(),
        "testclear" => ed.test_clear(),
        "test" => match rest.trim() {
            "" | "suite" => ed.test_run(crate::testrun::Scope::Suite),
            "nearest" => ed.test_run(crate::testrun::Scope::Nearest),
            "file" => ed.test_run(crate::testrun::Scope::File),
            "last" => ed.test_last(),
            "output" => ed.test_output(),
            "stop" => ed.test_stop(),
            "clear" => ed.test_clear(),
            other => ed.set_message(format!(
                "Unknown :test argument: {other} (nearest|file|suite|last|output|stop|clear)"
            )),
        },
        "taskwatch" | "watch" => {
            let cmd = rest.trim();
            let command = if cmd.is_empty() {
                ed.default_task_command()
            } else {
                Some(cmd.to_string())
            };
            match command {
                Some(c) => {
                    ed.watch_task = Some(c.clone());
                    ed.run_task(&c);
                    ed.set_message(format!("Watching: re-runs '{c}' on every save (:taskwatchoff to stop)"));
                }
                None => ed.set_message("No default command to watch — use :taskwatch <cmd>"),
            }
        }
        "taskwatchoff" | "watchoff" => {
            ed.watch_task = None;
            ed.set_message("Task watch off");
        }
        "termsend" | "tsend" => match effective_range {
            Some((a, b)) => ed.termsend_lines(a, b),
            None => {
                let l = ed.cursor().0;
                ed.termsend_lines(l, l);
            }
        },
        "fold" | "fo" => match effective_range {
            Some((a, b)) => ed.create_fold(a, b),
            None => ed.set_message("Usage: :{range}fold (or select lines, then :fold)"),
        },
        "foldopen" | "foldopenall" => ed.open_all_folds(),
        "foldclose" | "foldcloseall" => ed.close_all_folds(),
        "foldindent" => ed.fold_by_indent(),
        "foldsyntax" => ed.fold_by_syntax(),
        "foldlsp" => ed.request_language("foldingRange", None),
        "colorscheme" | "colo" => {
            let name = rest.trim();
            if name.is_empty() {
                ed.open_colorscheme_picker();
            } else if let Some(t) = crate::theme::builtin(name) {
                ed.theme = t;
                ed.config.colorscheme = name.to_string();
                // Force every cached row to repaint with the new palette.
                ed.syntax_stamp = ed.syntax_stamp.wrapping_add(1);
                ed.set_message(format!("colorscheme {name}"));
            } else {
                ed.set_message(format!(
                    "Unknown colorscheme: {name} (try: {})",
                    crate::theme::NAMES.join(", ")
                ));
            }
        }
        "diffthis" => ed.diff_this(),
        "diffoff" => ed.diff_off(),
        "conflictours" => ed.resolve_conflict(crate::conflict::Keep::Ours),
        "conflicttheirs" => ed.resolve_conflict(crate::conflict::Keep::Theirs),
        "conflictboth" => ed.resolve_conflict(crate::conflict::Keep::Both),
        "conflictnext" => ed.goto_conflict(true),
        "conflictprev" => ed.goto_conflict(false),
        "normal" | "norm" | "normal!" | "norm!" => {
            // Feed the literal keys as normal-mode input (one leading space is
            // the separator; further spaces are part of the keys). Reuses the
            // macro-replay path, so recursion/budget limits and the recording
            // guard all apply. `:normal` ends any pending Insert.
            let keys = rest.strip_prefix(' ').unwrap_or(rest);
            let ks: Vec<Key> = keys
                .chars()
                .map(|c| match c {
                    '\x1b' => Key::Esc,
                    '\n' | '\r' => Key::Enter,
                    '\t' => Key::Tab,
                    _ => Key::Char(c),
                })
                .collect();
            if !ks.is_empty() {
                ed.replay(&ks);
                if matches!(ed.mode, crate::mode::Mode::Insert) {
                    ed.replay(&[Key::Esc]);
                }
                ed.enter_normal();
            }
        }
        "colorpick" => match color_at_cursor(ed) {
            Some((_, _, (r, g, b))) => {
                ed.set_message(format!("#{r:02x}{g:02x}{b:02x}  rgb({r}, {g}, {b})"))
            }
            None => ed.set_message("No hex color under the cursor"),
        },
        "colorlighten" => {
            let pct = rest.trim().parse::<i32>().unwrap_or(10).clamp(0, 100);
            color_adjust(ed, pct);
        }
        "colordarken" => {
            let pct = rest.trim().parse::<i32>().unwrap_or(10).clamp(0, 100);
            color_adjust(ed, -pct);
        }
        "difffold" => {
            let ctx = rest.trim().parse::<usize>().unwrap_or(3);
            ed.fold_diff_context(ctx);
        }
        "zen" => {
            ed.zen = !ed.zen;
            ed.set_message(if ed.zen {
                "Zen mode on (:zen to exit)"
            } else {
                "Zen mode off"
            });
        }
        "tours" => ed.list_tours(),
        "tour" => ed.start_tour(rest.trim()),
        "tourresume" => ed.tour_resume(rest.trim()),
        "tourhelp" => ed.tour_help(),
        "tourfocus" => ed.toggle_tour_explanation(),
        "tourretry" => ed.tour_retry(),
        "touredit" => ed.tour_edit(),
        "tourexample" => ed.tour_example(),
        "tourlayout" => ed.tour_layout(rest.trim()),
        "tourcopystep" => ed.copy_tour_step(),
        "tourcopyall" => ed.copy_tour(),
        "tournew" => ed.tour_new(rest.trim()),
        "toursave" => ed.tour_save(rest.trim()),
        "tournext" | "tourn" => ed.tour_step(true),
        "tourprev" | "tourp" => ed.tour_step(false),
        "tourend" => ed.tour_end(),
        "tourrestart" => ed.tour_restart(),
        "toursteps" => ed.list_tour_steps(),
        "tourexplain" => ed.tour_explain(),
        "tourview" => ed.tour_view(rest.trim()),
        "tourmarkdown" => ed.tour_markdown(rest.trim()),
        "tourindicators" => ed.toggle_tour_indicators(),
        "tourjump" => ed.tour_jump(rest.trim()),
        "tourfollow" => ed.tour_follow(rest.trim()),
        "tourhnext" => ed.tour_highlight_step(true, 1),
        "tourhprev" => ed.tour_highlight_step(false, 1),
        "tourcheck" => ed.tour_check(rest.trim()),
        "grep" => ed.open_grep(rest.trim()),
        // Bare `:far` opens the reviewed replace screen; with a body it
        // stays the `:cfar` alias it always was.
        "far" if rest.trim().is_empty() => ed.open_far(None),
        "cfar" | "far" => run_far_replace(ed, rest),
        "cfarpreview" | "farpreview" => run_far_preview(ed, rest),
        "todo" => {
            // Project-wide index of TODO/FIXME/HACK/XXX comments: a fixed-pattern
            // grep, shown as a navigable (non-query-editing) results list.
            ed.open_grep(r"(TODO|FIXME|HACK|XXX)");
            if let Some(r) = &mut ed.results {
                // Leave query-editing mode so it shows as a navigable list, but
                // keep `live` so the async grep results are actually applied.
                r.search_input = None;
                r.title = "TODO / FIXME / HACK".into();
            }
        }
        "diagnostics" => {
            let r = ed.diagnostic_results();
            ed.show_results(r);
        }
        "workspacediagnostics" | "wdiagnostics" | "wdiag" => ed.request_workspace_diagnostics(),
        "outline" => ed.request_language("outline", None),
        "documentlinks" => ed.request_language("documentLinks", None),
        "codelens" => ed.request_language("codeLens", None),
        "inlayhints" => ed.request_language("inlayHints", None),
        "projects" => ed.show_recent_projects(),
        "cd" | "chdir" => ed.change_directory(rest.trim()),
        "pwd" => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| ed.project_root.clone());
            ed.set_message(cwd.display().to_string());
        }
        "references" => ed.request_language("references", None),
        "callers" | "incomingcalls" | "callhierarchy" => {
            ed.request_language("callHierarchy", None)
        }
        "callees" | "outgoingcalls" => ed.request_language("callHierarchyOut", None),
        "linkededit" => {
            let name = rest.trim();
            if name.is_empty() {
                // No name: start a live linked-editing session (type in one
                // range, the others mirror as you go).
                ed.pending_linked_live = true;
                ed.request_language("linkedEditing", None);
            } else {
                ed.pending_linked_edit = Some(name.to_string());
                ed.request_language("linkedEditing", None);
            }
        }
        "linkedlive" => {
            ed.pending_linked_live = true;
            ed.request_language("linkedEditing", None);
        }
        "supertypes" => ed.request_language("typeHierarchySuper", None),
        "subtypes" => ed.request_language("typeHierarchySub", None),
        "typedefinition" => ed.request_type_definition(),
        "implementation" => ed.request_implementation(),
        "declaration" => ed.request_declaration(),
        "workspacesymbols" => ed.request_workspace_symbols(rest.trim()),
        "format" => ed.request_language("format", None),
        "rename" => ed.request_language("rename", Some(rest.trim())),
        "renameapply" | "refactorapply" => ed.apply_pending_rename(),
        "renamecancel" | "refactorcancel" => ed.cancel_pending_rename(),
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
        "tree" => ed.toggle_file_tree(),
        "treefind" => ed.tree_find_current(),
        "treeroot" => ed.tree_set_root(rest.trim()),
        "treenew" => ed.tree_new(rest.trim()),
        "treerename" => ed.tree_rename(rest.trim()),
        "tabnew" => {
            ed.new_tab();
            if !rest.trim().is_empty() {
                if let Err(e) = ed.open_file(PathBuf::from(rest.trim())) {
                    ed.set_message(e.to_string());
                }
            }
        }
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
        "reg" | "registers" => {
            let regs = ed.registers.list();
            if regs.is_empty() {
                ed.set_message("No registers set");
            } else {
                let entries = regs
                    .iter()
                    .map(|(name, e)| {
                        let kind = if e.block_width.is_some() {
                            "b"
                        } else if e.linewise {
                            "l"
                        } else {
                            "c"
                        };
                        let preview: String =
                            e.text.replace('\n', "\\n").chars().take(200).collect();
                        crate::results::Entry::text(format!("\"{name} [{kind}]  {preview}"))
                    })
                    .collect();
                ed.show_results(crate::results::Results::new("Registers", entries));
            }
        }
        "marks" => {
            let mut marks: Vec<(char, crate::navigation::Location)> =
                ed.marks.iter().map(|(c, l)| (*c, l.clone())).collect();
            marks.sort_by_key(|(c, _)| *c);
            if marks.is_empty() {
                ed.set_message("No marks set");
            } else {
                let entries = marks
                    .iter()
                    .map(|(c, l)| {
                        let name = l
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "[scratch]".into());
                        let mut e = crate::results::Entry::text(format!(
                            "'{}  {}:{}  {}",
                            c,
                            l.line + 1,
                            l.col + 1,
                            name
                        ));
                        e.buffer_id = Some(l.buffer);
                        e.path = l.path.clone();
                        e.line = l.line;
                        e.col = l.col;
                        e
                    })
                    .collect();
                ed.show_results(crate::results::Results::new("Marks", entries));
            }
        }
        "y" | "yank" | "ya" => {
            let last = ed.buf().line_count().saturating_sub(1);
            let (s, e) = effective_range.unwrap_or((ed.cursor().0, ed.cursor().0));
            let (s, e) = (s.min(last), e.min(last));
            let reg = rest.trim().chars().next().filter(|c| c.is_alphanumeric());
            let text: String = (s..=e)
                .map(|l| format!("{}\n", ed.buf().line_text(l)))
                .collect();
            ed.registers.set(reg, text, true);
            let n = e - s + 1;
            ed.set_message(format!("{n} line{} yanked", if n == 1 { "" } else { "s" }));
        }
        "put" | "pu" => {
            let last = ed.buf().line_count().saturating_sub(1);
            let (_, e) = effective_range.unwrap_or((ed.cursor().0, ed.cursor().0));
            let target = e.min(last);
            let reg = rest.trim().chars().next().filter(|c| c.is_alphanumeric());
            let Some(entry) = ed.registers.get(reg) else {
                ed.set_message("E353: nothing in register");
                return;
            };
            let text = entry.text.clone();
            let mut lines: Vec<&str> = text.split('\n').collect();
            if text.ends_with('\n') {
                lines.pop(); // trailing newline yields an empty final element
            }
            if lines.is_empty() {
                return;
            }
            let total = ed.buf().rope.len_chars();
            let has_trailing_nl = total > 0 && ed.buf().rope.char(total - 1) == '\n';
            ed.buf_mut().begin_edit();
            let first_pasted = if target == last && !has_trailing_nl {
                // Append below a file that lacks a final newline.
                ed.buf_mut().insert_str_at(total, &format!("\n{}", lines.join("\n")));
                target + 1
            } else {
                let at = if target < last {
                    ed.buf().char_idx(target + 1, 0)
                } else {
                    total
                };
                ed.buf_mut().insert_str_at(at, &format!("{}\n", lines.join("\n")));
                target + 1
            };
            ed.buf_mut().commit_edit();
            let line = first_pasted.min(ed.buf().line_count().saturating_sub(1));
            ed.set_cursor(line, ed.buf().first_non_blank(line));
        }
        "join" | "j" | "join!" | "j!" => {
            let bang = name.ends_with('!');
            let last = ed.buf().line_count().saturating_sub(1);
            let (s, e0) = effective_range.unwrap_or((ed.cursor().0, ed.cursor().0));
            let s = s.min(last);
            let mut e = e0.min(last);
            if e <= s {
                e = (s + 1).min(last); // no range / single line: join with the next
            }
            if e > s {
                let lines: Vec<String> = (s..=e).map(|l| ed.buf().line_text(l)).collect();
                let first_len = lines[0].chars().count();
                let joined = if bang {
                    lines.concat()
                } else {
                    let mut out = lines[0].clone();
                    for l in &lines[1..] {
                        let t = l.trim_start();
                        if !out.is_empty() && !out.ends_with(' ') && !t.is_empty() {
                            out.push(' ');
                        }
                        out.push_str(t);
                    }
                    out
                };
                let start = ed.buf().char_idx(s, 0);
                let end = if e < last {
                    ed.buf().char_idx(e + 1, 0)
                } else {
                    ed.buf().rope.len_chars()
                };
                let ends_nl = end > 0 && ed.buf().rope.char(end - 1) == '\n';
                let repl = if ends_nl {
                    format!("{joined}\n")
                } else {
                    joined
                };
                ed.buf_mut().begin_edit();
                ed.buf_mut().delete_char_range(start, end);
                ed.buf_mut().insert_str_at(start, &repl);
                ed.buf_mut().commit_edit();
                ed.set_cursor(s, first_len);
            }
        }
        "sort" | "sor" | "sort!" | "sor!" => {
            let flags = rest.trim();
            let reverse = name.ends_with('!') || flags.contains('!');
            let unique = flags.contains('u');
            let numeric = flags.contains('n');
            let icase = flags.contains('i');
            let last = ed.buf().line_count().saturating_sub(1);
            let (s, e) = effective_range.unwrap_or((0, last));
            let (s, e) = (s.min(last), e.min(last));
            let mut lines: Vec<String> = (s..=e).map(|l| ed.buf().line_text(l)).collect();
            if numeric {
                lines.sort_by_key(|l| leading_number(l));
            } else if icase {
                lines.sort_by_key(|l| l.to_lowercase());
            } else {
                lines.sort();
            }
            if reverse {
                lines.reverse();
            }
            if unique {
                lines.dedup();
            }
            let start = ed.buf().char_idx(s, 0);
            let end = if e < last {
                ed.buf().char_idx(e + 1, 0)
            } else {
                ed.buf().rope.len_chars()
            };
            // Preserve whether the replaced span ended in a newline, so sorting
            // the tail of a file with no final newline doesn't add one.
            let ends_nl = end > 0 && ed.buf().rope.char(end - 1) == '\n';
            let replacement = if lines.is_empty() {
                String::new()
            } else if ends_nl {
                format!("{}\n", lines.join("\n"))
            } else {
                lines.join("\n")
            };
            ed.buf_mut().begin_edit();
            ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut().insert_str_at(start, &replacement);
            ed.buf_mut().commit_edit();
            let line = s.min(ed.buf().line_count().saturating_sub(1));
            ed.set_cursor(line, ed.buf().first_non_blank(line));
        }
        "d" | "delete" | "de" | "del" => {
            let last = ed.buf().line_count().saturating_sub(1);
            let (s, e) = effective_range.unwrap_or((ed.cursor().0, ed.cursor().0));
            let (s, e) = (s.min(last), e.min(last));
            let total = ed.buf().rope.len_chars();
            let has_trailing_nl = total > 0 && ed.buf().rope.char(total - 1) == '\n';
            // Deleting through the last line of a file that has no trailing
            // newline must also remove the newline that ended the preceding
            // line, so the file doesn't gain a trailing newline (Vim's `:$d` on
            // "a\nb\nc" leaves "a\nb"). With a trailing newline, or when
            // deleting from line 0, the plain span is correct.
            let start = if e >= last && !has_trailing_nl && s > 0 {
                ed.buf().char_idx(s - 1, ed.buf().line_len(s - 1))
            } else {
                ed.buf().char_idx(s, 0)
            };
            let end = if e < last {
                ed.buf().char_idx(e + 1, 0)
            } else {
                total
            };
            ed.buf_mut().begin_edit();
            ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut().commit_edit();
            let line = s.min(ed.buf().line_count().saturating_sub(1));
            let col = ed.buf().first_non_blank(line);
            ed.set_cursor(line, col);
        }
        "messages" => {
            if ed.messages.is_empty() {
                ed.set_message("No messages");
            } else {
                let entries = ed
                    .messages
                    .iter()
                    .map(|m| crate::results::Entry::text(m.clone()))
                    .collect();
                ed.show_results(crate::results::Results::new("Messages", entries));
            }
        }
        "Log" => open_log(ed),
        "checkhealth" | "health" => open_health(ed),
        "earlier" | "ea" => {
            let n = rest.trim().parse::<usize>().unwrap_or(1).max(1);
            let mut done = 0;
            for _ in 0..n {
                if ed.buf_mut().undo() {
                    done += 1;
                } else {
                    break;
                }
            }
            ed.set_message(format!("{done} change{} earlier", if done == 1 { "" } else { "s" }));
        }
        "later" | "lat" => {
            let n = rest.trim().parse::<usize>().unwrap_or(1).max(1);
            let mut done = 0;
            for _ in 0..n {
                if ed.buf_mut().redo() {
                    done += 1;
                } else {
                    break;
                }
            }
            ed.set_message(format!("{done} change{} later", if done == 1 { "" } else { "s" }));
        }
        "undolist" | "undotree" | "undohistory" => {
            let (states, current) = ed.buf().undo_timeline();
            let entries = states
                .iter()
                .enumerate()
                .map(|(i, (nlines, cursor, preview))| {
                    let marker = if i == current { "▶" } else { " " };
                    let tag = if i == current { "  ← current" } else { "" };
                    let body = if preview.is_empty() {
                        "(empty)"
                    } else {
                        preview.as_str()
                    };
                    let display = format!(
                        "{marker} #{i}  {nlines} line(s)  {}:{}  {body}{tag}",
                        cursor.0 + 1,
                        cursor.1 + 1
                    );
                    let mut e = crate::results::Entry::text(display);
                    // Selecting a non-current state jumps there via the
                    // existing :earlier/:later machinery (relative to now).
                    if i < current {
                        e.action = Some(serde_json::json!({
                            "_vaayu_rerun_ex": format!("earlier {}", current - i)
                        }));
                    } else if i > current {
                        e.action = Some(serde_json::json!({
                            "_vaayu_rerun_ex": format!("later {}", i - current)
                        }));
                    }
                    e
                })
                .collect();
            ed.show_results(crate::results::Results::new("Undo history", entries));
        }
        "resume" => ed.resume(),
        "treebookmarks" => ed.show_tree_bookmarks(),
        "treesort" => ed.tree_sort(rest.trim()),
        "treetrash" => ed.show_tree_trash(),
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
        "vsplit" | "split" | "sp" | "vs" | "vsp" | "spl" => {
            ed.split_window(matches!(name, "vsplit" | "vs" | "vsp"), false);
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
            "cursorline" | "cul" => ed.config.cursorline = true,
            "nocursorline" | "nocul" => ed.config.cursorline = false,
            "list" => ed.config.list = true,
            "nolist" => ed.config.list = false,
            "rainbow" => ed.config.rainbow = true,
            "norainbow" => ed.config.rainbow = false,
            "colorswatch" => ed.config.colorswatch = true,
            "nocolorswatch" => ed.config.colorswatch = false,
            "conceal" => ed.config.conceal = true,
            "noconceal" => ed.config.conceal = false,
            "spell" => ed.config.spell = true,
            "nospell" => ed.config.spell = false,
            "notifications" => ed.config.notifications = true,
            "nonotifications" => ed.config.notifications = false,
            "persist_messages" => {
                ed.config.persist_messages = true;
                ed.open_logger();
            }
            "nopersist_messages" => {
                ed.config.persist_messages = false;
                ed.disable_logger();
            }
            "progress" => ed.config.progress = true,
            "noprogress" => ed.config.progress = false,
            "watch" => ed.config.watch = true,
            "nowatch" => ed.config.watch = false,
            "autoread" | "ar" => ed.config.autoread = true,
            "noautoread" | "noar" => ed.config.autoread = false,
            "transparent" => ed.config.transparent = true,
            "notransparent" => ed.config.transparent = false,
            "stickyscroll" | "sticky" => ed.config.sticky_scroll = true,
            "nostickyscroll" | "nosticky" => ed.config.sticky_scroll = false,
            "minimap" | "mmp" => ed.config.minimap = true,
            "nominimap" | "nommp" => ed.config.minimap = false,
            "minimapcharacters" | "mmpchars" => ed.config.minimap_render_characters = true,
            "nominimapcharacters" | "nommpchars" => ed.config.minimap_render_characters = false,
            "minimapsections" | "mmpsections" => ed.config.minimap_section_headers = true,
            "nominimapsections" | "nommpsections" => ed.config.minimap_section_headers = false,
            opt if opt.starts_with("reviewtext=") => {
                match opt.split_once('=').map(|(_, v)| v.trim()).unwrap_or("") {
                    v @ ("all" | "cursor" | "off") => {
                        ed.config.review_virtual_text = v.to_string();
                        ed.review_ui.live_gen += 1;
                    }
                    _ => ed.set_message("reviewtext must be all, cursor, or off"),
                }
            }
            opt if opt.starts_with("minimapsize=") || opt.starts_with("mmpsize=") => {
                let val = opt.split_once('=').map(|(_, v)| v).unwrap_or("");
                match crate::minimap::Size::parse(val) {
                    Some(_) => ed.config.minimap_size = val.trim().to_string(),
                    None => ed.set_message("minimapsize must be proportional, fit, or fill"),
                }
            }
            opt if opt.starts_with("minimapslider=") || opt.starts_with("mmpslider=") => {
                match opt.split_once('=').map(|(_, v)| v.trim()).unwrap_or("") {
                    v @ ("always" | "mouseover") => ed.config.minimap_show_slider = v.to_string(),
                    _ => ed.set_message("minimapslider must be always or mouseover"),
                }
            }
            "refactorpreview" | "rfp" => ed.config.refactor_preview = true,
            "norefactorpreview" | "norfp" => ed.config.refactor_preview = false,
            "winbar" | "wbr" => ed.config.winbar = true,
            "nowinbar" | "nowbr" => ed.config.winbar = false,
            "globalstatusline" | "laststatus3" | "gstl" => ed.config.global_statusline = true,
            "noglobalstatusline" | "nogstl" => ed.config.global_statusline = false,
            "foldcolumn" | "fdc" => ed.config.foldcolumn = true,
            "nofoldcolumn" | "nofdc" => ed.config.foldcolumn = false,
            "formatonsave" | "fos" => ed.config.format_on_save = true,
            "noformatonsave" | "nofos" => ed.config.format_on_save = false,
            "todohighlight" | "todo" => ed.config.todo_highlight = true,
            "notodohighlight" | "notodo" => ed.config.todo_highlight = false,
            "ghosttext" | "ghost" => ed.config.ghost_text = true,
            "noghosttext" | "noghost" => ed.config.ghost_text = false,
            "semantictokens" | "semantic" => ed.config.semantic_tokens = true,
            "nosemantictokens" | "nosemantic" => {
                ed.config.semantic_tokens = false;
                ed.semantic_tokens.clear();
                ed.semantic_result = None;
                ed.semantic_raw.clear();
            }
            "relativenumber" | "rnu" => ed.config.relativenumber = true,
            "norelativenumber" | "nornu" => ed.config.relativenumber = false,
            "ignorecase" | "ic" => ed.config.ignorecase = true,
            "noignorecase" | "noic" => ed.config.ignorecase = false,
            "smartcase" | "scs" => ed.config.smartcase = true,
            "nosmartcase" | "noscs" => ed.config.smartcase = false,
            "smartindent" | "si" => ed.config.smartindent = true,
            "nosmartindent" | "nosi" => ed.config.smartindent = false,
            "autopairs" => ed.config.autopairs = true,
            "noautopairs" => ed.config.autopairs = false,
            // Per-buffer indent options also update the config default.
            "expandtab" | "et" => {
                ed.config.expandtab = true;
                ed.buf_mut().expandtab = true;
            }
            "noexpandtab" | "noet" => {
                ed.config.expandtab = false;
                ed.buf_mut().expandtab = false;
            }
            opt if opt.starts_with("colorcolumn=") || opt.starts_with("cc=") => {
                let val = opt.split_once('=').map(|(_, v)| v).unwrap_or("");
                match val.parse::<usize>() {
                    Ok(n) => ed.config.colorcolumn = n,
                    Err(_) if val.is_empty() => ed.config.colorcolumn = 0,
                    Err(_) => ed.set_message("colorcolumn must be a number (0 disables)"),
                }
            }
            "ff?" | "fileformat?" => {
                let ff = ed.buf().fileformat.name();
                ed.set_message(format!("fileformat={ff}"));
            }
            opt if opt.starts_with("ff=") || opt.starts_with("fileformat=") => {
                let val = opt.split_once('=').map(|(_, v)| v).unwrap_or("");
                match crate::buffer::FileFormat::parse(val) {
                    Some(ff) => {
                        ed.buf_mut().fileformat = ff;
                        ed.set_message(format!("fileformat={} (write to apply)", ff.name()));
                    }
                    None => ed.set_message("fileformat must be unix, dos, or mac"),
                }
            }
            // Numeric options `key=N`. Per-buffer ones (tabstop/shiftwidth)
            // also update the config default so new buffers inherit them.
            opt if opt.contains('=') => {
                let (k, v) = opt.split_once('=').unwrap();
                match (k, v.parse::<usize>()) {
                    ("tabstop" | "ts", Ok(n)) if n >= 1 => {
                        ed.config.tabstop = n;
                        ed.buf_mut().tabstop = n;
                    }
                    ("shiftwidth" | "sw", Ok(n)) if n >= 1 => {
                        ed.config.shiftwidth = n;
                        ed.buf_mut().shiftwidth = n;
                    }
                    ("scrolloff" | "so", Ok(n)) => ed.config.scrolloff = n,
                    ("textwidth" | "tw", Ok(n)) => ed.config.textwidth = n,
                    ("updatetime" | "ut", Ok(n)) => ed.config.updatetime_ms = n as u64,
                    ("largefilekb" | "largefile", Ok(n)) => ed.config.large_file_kb = n,
                    ("minimapwidth" | "mmpwidth", Ok(n))
                        if (crate::minimap::MIN_WIDTH..=crate::minimap::MAX_WIDTH).contains(&n) =>
                    {
                        ed.config.minimap_width = n
                    }
                    ("minimapmaxcolumn" | "mmpmaxcol", Ok(n)) if n >= 1 => {
                        ed.config.minimap_max_column = n
                    }
                    _ => ed.set_message(format!("Unknown or invalid :set option: {opt}")),
                }
            }
            _ => ed.set_message(
                "Supported: [no]wrap [no]number [no]relativenumber [no]cursorline [no]list \
                 [no]ignorecase [no]smartcase [no]smartindent [no]expandtab [no]autopairs \
                 tabstop=N shiftwidth=N scrolloff=N textwidth=N updatetime=N colorcolumn=N ff={unix,dos,mac}",
            ),
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
            let result = if ed.buf().note.is_some()
                || ed.buf().path.as_deref().is_some_and(crate::task_tracker::is_task_path)
            {
                ed.save_current()
            } else {
                ed.buf_mut().save_force()
            };
            ed.set_message(match result {
                Ok(()) => "Written".into(),
                Err(e) => e.to_string(),
            });
        }
        // Saving the `:tournew` prompt buffer isn't a file write -- it means
        // "generate the tour". Ask for a name (prefill the command line, like
        // rename/workspacesymbols do), then `:toursave <name>` sends it to
        // the configured AI agent. Applies to :w/:write/:wq/:x/:x!/:wq!.
        "w" | "write" | "wq" | "x" | "wq!" | "x!" | "write!"
            if ed
                .tour_draft
                .as_ref()
                .is_some_and(|(_, id)| *id == ed.buf().id) =>
        {
            let existing = ed
                .tour_draft
                .as_ref()
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            ed.enter_command(crate::mode::CommandKind::Ex);
            let mut prefill = format!("toursave {existing}");
            if !prefill.ends_with(' ') {
                prefill.push(' ');
            }
            ed.set_cmdline(prefill);
            ed.set_message(format!(
                "Name this tour, then press Enter to generate it with {}",
                ed.config.ai_agent_label()
            ));
        }
        "w" | "write" | "wq" | "x" if ed.is_task_draft_buffer() => {
            match ed.save_task_draft() {
                Ok(()) => ed.set_message("Task saved · opened its day view"),
                Err(e) => ed.set_message(format!("task draft: {e}")),
            }
        }
        "w" | "write" if rest.trim_start().starts_with('!') => {
            // `:w !cmd`: pipe the buffer (or range) to a command; nothing is
            // written to disk.
            let (l1, l2) = effective_range.unwrap_or((0, ed.buf().line_count().saturating_sub(1)));
            let text = lines_text(ed, l1, l2);
            pipe_to_command(ed, rest.trim_start()[1..].trim(), &text);
        }
        "w" | "write" if rest.trim_start().starts_with("++") => {
            ed.set_message("E474: ++opt arguments are not supported; use :set fileencoding / fileformat");
        }
        "w" | "write" if rest.trim_start().starts_with(">>") => {
            let target = rest.trim_start()[2..].trim();
            let (l1, l2) = effective_range.unwrap_or((0, ed.buf().line_count().saturating_sub(1)));
            let text = lines_text(ed, l1, l2);
            let path = if target.is_empty() {
                ed.buf().path.clone()
            } else {
                Some(PathBuf::from(target))
            };
            let result = match path {
                None => Err(std::io::Error::other("E32: No file name")),
                Some(p) => std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&p)
                    .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes())),
            };
            match result {
                Ok(()) => ed.set_message(format!("{} line(s) appended", l2 - l1 + 1)),
                Err(e) => ed.set_message(format!("append failed: {e}")),
            }
        }
        "w" | "write" => {
            let target = rest.trim();
            let result = if target.is_empty() {
                ed.save_current_formatted()
            } else if ed.buf().path.as_deref().is_some_and(crate::task_tracker::is_task_path) {
                Err(anyhow::anyhow!("task documents must be saved in place"))
            } else {
                ed.buf_mut().save_as(PathBuf::from(target))
            };
            match result {
                Ok(()) => ed.set_message(format!("\"{}\" written", ed.buf().name())),
                Err(e) => ed.set_message(format!("save failed: {}", e)),
            }
        }
        "q" | "quit" => {
            // Dismissing a Results panel never risks unsaved data, so skip the
            // modified-buffers guard in that case.
            if over_results {
                close_current_or_quit(ed, true);
            } else if is_closable_view(ed) {
                if ed.buf().is_modified() {
                    ed.set_message("unsaved changes -- use :q! to discard");
                } else {
                    close_view(ed);
                }
            } else if ed.windows.len() > 1 && !ed.only_sidebars_remain() {
                // Closing one split/pane (e.g. the AI sidebar or a terminal)
                // never discards a buffer, so skip the unsaved-changes guard --
                // it only matters when :q would actually quit the editor.
                ed.close_window();
            } else if let Some(msg) = modified_buffers_message(ed) {
                ed.set_message(msg);
            } else {
                close_current_or_quit(ed, false);
            }
        }
        "q!" | "quit!" if !over_results && is_closable_view(ed) => close_view(ed),
        "q!" | "quit!" => close_current_or_quit(ed, over_results),
        "wq" | "x" | "xit" | "exit" | "exi" => match ed.save_current_formatted() {
            Ok(()) => {
                if let Some(msg) = modified_buffers_message(ed) {
                    ed.set_message(msg);
                } else {
                    close_current_or_quit(ed, over_results);
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
            if let Some(a) = ed.reviews.active.as_mut() {
                if let Err(e) = a.flush() {
                    failed.push(format!("review comments: {e}"));
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
        "new" | "vnew" | "vne" => {
            ed.split_window(name != "new", false);
            open_empty_buffer(ed);
            if !rest.trim().is_empty() {
                if let Err(e) = ed.open_file(PathBuf::from(rest.trim())) {
                    ed.set_message(e.to_string());
                }
            }
        }
        "enew" | "ene" | "enew!" | "ene!" => open_empty_buffer(ed),
        "tabe" | "tabedit" if !rest.trim().is_empty() => {
            ed.new_tab();
            if let Err(e) = ed.open_file(PathBuf::from(rest.trim())) {
                ed.set_message(e.to_string());
            }
        }
        "tabe" | "tabedit" => ed.new_tab(),
        "wa" | "wall" => {
            let mut failed = Vec::new();
            let mut saved = 0;
            let current = ed.cur;
            for i in 0..ed.buffers.len() {
                ed.cur = i;
                if ed.buf().path.is_some() && ed.buf().is_modified() {
                    match ed.save_current() {
                        Ok(()) => saved += 1,
                        Err(e) => failed.push(format!("{}: {e}", ed.buf().name())),
                    }
                }
            }
            ed.cur = current;
            if failed.is_empty() {
                ed.set_message(format!("{saved} buffer(s) written"));
            } else {
                ed.set_message(format!("write failed -- {}", failed.join("; ")));
            }
        }
        "up" | "update" => {
            if ed.buf().is_modified() {
                run_ex(ed, "w");
            }
        }
        "sav" | "saveas" | "sav!" | "saveas!" => {
            let target = rest.trim();
            if target.is_empty() {
                ed.set_message("E471: Argument required");
            } else if !name.ends_with('!') && PathBuf::from(target).exists() {
                ed.set_message("E13: File exists (add ! to override)");
            } else {
                match ed.buf_mut().save_as(PathBuf::from(target)) {
                    Ok(()) => ed.set_message(format!("\"{}\" written", ed.buf().name())),
                    Err(e) => ed.set_message(format!("save failed: {e}")),
                }
            }
        }
        "e#" => ed.switch_to_alternate(),
        "u" | "un" | "undo" => {
            let n = rest.trim().parse::<usize>().unwrap_or(1).max(1);
            for _ in 0..n {
                if !ed.buf_mut().undo() {
                    break;
                }
            }
            ed.set_message("undo");
        }
        "red" | "redo" => {
            if ed.buf_mut().redo() {
                ed.set_message("redo");
            } else {
                ed.set_message("Already at newest change");
            }
        }
        "r" | "read" => {
            let arg = rest.trim();
            let at = effective_range.map(|(_, b)| b).unwrap_or(ed.cursor().0);
            let text = if let Some(cmd) = arg.strip_prefix('!') {
                match shell_output(ed, cmd.trim()) {
                    Ok(t) => t,
                    Err(e) => {
                        ed.set_message(e);
                        return;
                    }
                }
            } else {
                let path = if arg.is_empty() {
                    ed.buf().path.clone()
                } else {
                    Some(PathBuf::from(arg))
                };
                match path.map(std::fs::read_to_string) {
                    Some(Ok(t)) => t,
                    Some(Err(e)) => {
                        ed.set_message(format!("E484: Can't open file {arg}: {e}"));
                        return;
                    }
                    None => {
                        ed.set_message("E32: No file name");
                        return;
                    }
                }
            };
            insert_lines_below(ed, at, &text);
        }
        "retab" | "ret" | "retab!" | "ret!" => {
            if let Ok(n) = rest.trim().parse::<usize>() {
                if n > 0 {
                    ed.buf_mut().tabstop = n;
                }
            }
            let (l1, l2) = effective_range.unwrap_or((0, ed.buf().line_count().saturating_sub(1)));
            retab(ed, l1, l2, name.ends_with('!'));
        }
        "bufdo" | "bufd" => {
            let cmd = rest.trim().to_string();
            for i in 0..ed.buffers.len() {
                if i < ed.buffers.len() {
                    ed.cur = i;
                    run_ex(ed, &cmd);
                }
            }
        }
        "windo" | "wind" => {
            let cmd = rest.trim().to_string();
            if ed.windows.is_empty() {
                run_ex(ed, &cmd);
            } else {
                for i in 0..ed.windows.len() {
                    if i < ed.windows.len() {
                        ed.focus_window(i);
                        run_ex(ed, &cmd);
                    }
                }
            }
        }
        "tabdo" | "tabd" => {
            let cmd = rest.trim().to_string();
            for i in 0..ed.tabs.len().max(1) {
                if !ed.tabs.is_empty() {
                    ed.switch_tab(i);
                }
                run_ex(ed, &cmd);
            }
        }
        "changes" => {
            let list = ed.buf().changelist.clone();
            if list.is_empty() {
                ed.set_message("E664: Changelist is empty");
            } else {
                let current = ed.buf().change_idx.unwrap_or(list.len());
                let id = ed.buf().id;
                let path = ed.buf().path.clone();
                let entries = list
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(i, (l, c))| {
                        let text = ed.buf().line_text(*l);
                        let mut e = crate::results::Entry::text(format!(
                            "{}{:>3} {:>5}:{:<3} {}",
                            if i == current { ">" } else { " " },
                            list.len() - i,
                            l + 1,
                            c + 1,
                            text.trim()
                        ));
                        e.buffer_id = Some(id);
                        e.path = path.clone();
                        e.line = *l;
                        e.col = *c;
                        e
                    })
                    .collect();
                ed.show_results(crate::results::Results::new("Changes", entries));
            }
        }
        "delm" | "delmarks" => delete_marks(ed, rest.trim()),
        "delm!" | "delmarks!" => {
            let id = ed.buf().id;
            ed.marks.retain(|c, loc| !(c.is_ascii_lowercase() && loc.buffer == id));
            ed.set_message("Deleted this buffer's lowercase marks");
        }
        "ma" | "mark" | "k" => match rest.trim().chars().next() {
            Some(c) => {
                let line = effective_range.map(|(_, b)| b).unwrap_or(ed.cursor().0);
                let (l, col) = ed.cursor();
                ed.set_cursor(line, 0);
                ed.set_mark(c);
                ed.set_cursor(l, col);
            }
            None => ed.set_message("E471: Argument required"),
        },
        _ if name.len() == 2 && name.starts_with('k') && name[1..].chars().all(|c| c.is_ascii_alphabetic()) => {
            let c = name.chars().nth(1).unwrap();
            let line = effective_range.map(|(_, b)| b).unwrap_or(ed.cursor().0);
            let (l, col) = ed.cursor();
            ed.set_cursor(line, 0);
            ed.set_mark(c);
            ed.set_cursor(l, col);
        }
        "ce" | "center" | "ri" | "right" | "le" | "left" => {
            let (l1, l2) = effective_range.unwrap_or((ed.cursor().0, ed.cursor().0));
            let tw = if ed.config.textwidth > 0 { ed.config.textwidth } else { 80 };
            let arg = rest.trim().parse::<usize>().ok();
            align_lines(ed, l1, l2, name.chars().next().unwrap(), arg, tw);
        }
        "res" | "resize" => resize_command(ed, rest.trim(), false),
        "vertical" | "vert" => {
            let inner = rest.trim();
            let (sub, arg) = split_command(inner);
            match sub {
                "res" | "resize" => resize_command(ed, arg.trim(), true),
                "new" => run_ex(ed, &format!("vnew{arg}")),
                "split" | "sp" => run_ex(ed, &format!("vsplit{arg}")),
                _ => run_ex(ed, inner),
            }
        }
        "wincmd" | "winc" => match rest.trim().chars().next() {
            Some(c) => ed.window_key(Key::Char(c)),
            None => ed.set_message("E471: Argument required"),
        },
        "cq" | "cquit" | "cq!" | "cquit!" => {
            ed.exit_code = Some(rest.trim().parse::<i32>().unwrap_or(1));
            ed.should_quit = true;
        }
        "s" if rest.trim().is_empty() => repeat_substitute(ed, effective_range, false, false),
        "&" => repeat_substitute(ed, effective_range, rest.trim() == "&", false),
        "&&" => repeat_substitute(ed, effective_range, true, false),
        "~" => repeat_substitute(ed, effective_range, rest.trim() == "&", true),
        "g&" => {
            let all = Some((0, ed.buf().line_count().saturating_sub(1)));
            repeat_substitute(ed, all, true, true);
        }
        "sm" | "smagic" | "sno" | "snomagic" => {
            let magic = if name.starts_with("sm") { "\\m" } else { "\\M" };
            let body = rest.trim_start();
            let Some(delim) = body.chars().next() else {
                ed.set_message("E486: pattern required");
                return;
            };
            let after = &body[delim.len_utf8()..];
            run_substitute(ed, &format!("s{delim}{magic}{after}"), effective_range);
        }
        "f" | "fi" | "file" => {
            let arg = rest.trim();
            if !arg.is_empty() {
                ed.buf_mut().path = Some(crate::files::identity(&PathBuf::from(arg)));
            }
            crate::normal::show_file_info(ed);
        }
        "as" | "ascii" => crate::normal::char_info(ed, false),
        "di" | "display" => run_ex(ed, "registers"),
        "badd" | "bad" => {
            let arg = rest.trim();
            if arg.is_empty() {
                ed.set_message("E471: Argument required");
            } else {
                let cur = ed.cur;
                let alt = ed.alternate_buffer;
                match ed.open_file(PathBuf::from(arg)) {
                    Ok(()) => {
                        ed.cur = cur.min(ed.buffers.len() - 1);
                        ed.alternate_buffer = alt;
                        ed.set_message(format!("Added {arg} to the buffer list"));
                    }
                    Err(e) => ed.set_message(e.to_string()),
                }
            }
        }
        "bw" | "bwipeout" | "bun" | "bunload" => run_ex(ed, "bd"),
        "bw!" | "bwipeout!" | "bun!" | "bunload!" => run_ex(ed, "bd!"),
        "ar" | "args" => {
            let arg = rest.trim();
            if !arg.is_empty() {
                ed.arglist = arg.split_whitespace().map(PathBuf::from).collect();
                ed.arg_index = 0;
                let first = ed.arglist[0].clone();
                if let Err(e) = ed.open_file(first) {
                    ed.set_message(e.to_string());
                }
            } else if ed.arglist.is_empty() {
                ed.set_message("Argument list is empty");
            } else {
                let list: Vec<String> = ed
                    .arglist
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        if i == ed.arg_index {
                            format!("[{}]", p.display())
                        } else {
                            p.display().to_string()
                        }
                    })
                    .collect();
                ed.set_message(list.join(" "));
            }
        }
        "n" | "next" | "wn" | "wnext" => {
            if name.starts_with('w') {
                if let Err(e) = ed.save_current() {
                    ed.set_message(format!("save failed: {e}"));
                    return;
                }
            }
            step_arglist(ed, 1);
        }
        "N" | "Next" | "prev" | "previous" | "wN" | "wprevious" => {
            if name.starts_with('w') {
                if let Err(e) = ed.save_current() {
                    ed.set_message(format!("save failed: {e}"));
                    return;
                }
            }
            step_arglist(ed, -1);
        }
        "fir" | "first" | "rew" | "rewind" => {
            ed.arg_index = 0;
            step_arglist(ed, 0);
        }
        "la" | "last" => {
            ed.arg_index = ed.arglist.len().saturating_sub(1);
            step_arglist(ed, 0);
        }
        "vim" | "vimgrep" | "lv" | "lvimgrep" => {
            let loclist = name.starts_with('l');
            vimgrep(ed, rest.trim(), loclist);
        }
        "cnf" | "cnfile" => quickfix_file_step(ed, true),
        "cpf" | "cpfile" | "cNf" | "cNfile" => quickfix_file_step(ed, false),
        "cb" | "cbuffer" | "cgetb" | "cgetbuffer" | "cad" | "caddbuffer" => {
            let text = ed.buf().rope.to_string();
            let entries = crate::task::parse_errorformat(&text, &ed.project_root);
            set_quickfix_entries(ed, "Quickfix (buffer)", entries, name.starts_with("cb"));
        }
        "cf" | "cfile" | "cg" | "cgetfile" => {
            let arg = rest.trim();
            match std::fs::read_to_string(arg) {
                Ok(text) => {
                    let entries = crate::task::parse_errorformat(&text, &ed.project_root);
                    set_quickfix_entries(ed, format!("Quickfix ({arg})"), entries, name.starts_with("cf"));
                }
                Err(e) => ed.set_message(format!("E40: Can't open errorfile {arg}: {e}")),
            }
        }
        "checkt" | "checktime" => check_disk_changes(ed),
        "setf" | "setfiletype" => {
            let ft = rest.trim();
            if ft.is_empty() {
                ed.set_message("E471: Argument required");
            } else {
                run_ex(ed, &format!("set filetype={ft}"));
            }
        }
        "setl" | "setlocal" | "setg" | "setglobal" | "se" => run_ex(ed, &format!("set{rest}")),
        "sy" | "syntax" | "filet" | "filetype" => {
            // Highlighting comes from tree-sitter and is always on.
            ed.set_message(format!(":{name} -- syntax highlighting is always on (tree-sitter)"));
        }
        "hi" | "highlight" => {
            ed.set_message("Colors come from the theme: :colorscheme to switch, [theme] in config.toml to customize");
        }
        "go" | "goto" => {
            let n = rest.trim().parse::<usize>().unwrap_or(1).max(1);
            let bytes = ed.buf().rope.len_bytes();
            let idx = ed.buf().rope.byte_to_char((n - 1).min(bytes.saturating_sub(1)));
            let (l, c) = ed.buf().pos_from_char_idx(idx);
            ed.push_jump();
            ed.set_cursor(l, c);
        }
        "=" => {
            let n = match effective_range {
                Some((_, b)) => b + 1,
                None => ed.buf().line_count(),
            };
            ed.set_message(n.to_string());
        }
        "p" | "print" | "P" | "Print" | "nu" | "number" | "#" | "l" | "list" | "z" => {
            let (l1, l2) = match (name, effective_range) {
                ("z", _) => {
                    let l = ed.cursor().0;
                    (l, (l + ed.screen_rows.max(2) - 1).min(ed.buf().line_count().saturating_sub(1)))
                }
                (_, Some(r)) => r,
                (_, None) => (ed.cursor().0, ed.cursor().0),
            };
            let numbered = matches!(name, "nu" | "number" | "#" | "z");
            let list = matches!(name, "l" | "list");
            let id = ed.buf().id;
            let path = ed.buf().path.clone();
            let entries: Vec<_> = (l1..=l2)
                .map(|l| {
                    let mut text = ed.buf().line_text(l);
                    if list {
                        text = text.replace('\t', "^I") + "$";
                    }
                    let shown = if numbered { format!("{:>4} {text}", l + 1) } else { text };
                    let mut e = crate::results::Entry::text(shown);
                    e.buffer_id = Some(id);
                    e.path = path.clone();
                    e.line = l;
                    e
                })
                .collect();
            if entries.len() == 1 {
                ed.set_message(entries[0].text.clone());
            } else {
                ed.show_results(crate::results::Results::new("Lines", entries));
            }
            let last = l2.min(ed.buf().line_count().saturating_sub(1));
            let c = ed.buf().first_non_blank(last);
            ed.set_cursor(last, c);
        }
        "keepj" | "keepjumps" | "keepm" | "keepmarks" | "keepp" | "keeppatterns" | "keepa"
        | "keepalt" | "noa" | "noautocmd" | "uns" | "unsilent" | "verb" | "verbose" | "lockm"
        | "lockmarks" | "conf" | "confirm" | "bro" | "browse" | "to" | "topleft" | "bo"
        | "botright" | "abo" | "aboveleft" | "bel" | "belowright" | "lefta" | "leftabove"
        | "rightb" | "rightbelow" | "hid" | "hide" | "sandbox" | "san" => {
            let inner = rest.trim();
            if inner.is_empty() {
                if matches!(name, "hid" | "hide") {
                    ed.close_window();
                }
            } else {
                let saved = ed.last_search.clone();
                run_ex(ed, inner);
                if name.starts_with("keepp") {
                    ed.last_search = saved;
                }
            }
        }
        "sil" | "silent" | "sil!" | "silent!" => {
            run_ex(ed, rest.trim());
            ed.message.clear();
        }
        "tab" => {
            ed.new_tab();
            run_ex(ed, rest.trim());
        }
        "exe" | "execute" => {
            for part in split_exe_args(rest.trim()) {
                run_ex(ed, &part);
            }
        }
        "ec" | "echo" | "echom" | "echomsg" | "echon" => {
            let text = split_exe_args(rest.trim()).join(" ");
            ed.set_message(text);
        }
        "so" | "source" => {
            let arg = rest.trim();
            match std::fs::read_to_string(shellexpand_home(arg)) {
                Ok(text) => {
                    let mut ran = 0;
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('"') {
                            continue;
                        }
                        run_ex(ed, line.strip_prefix(':').unwrap_or(line));
                        ran += 1;
                    }
                    if ed.message.is_empty() {
                        ed.set_message(format!("Sourced {arg} ({ran} command(s))"));
                    }
                }
                Err(e) => ed.set_message(format!("E484: Can't open file {arg}: {e}")),
            }
        }
        "startinsert" | "star" | "startinsert!" | "star!" => {
            if name.ends_with('!') {
                let l = ed.cursor().0;
                let len = ed.buf().line_len(l);
                ed.set_cursor_insert(l, len);
            }
            ed.buf_mut().begin_edit();
            ed.enter_insert();
        }
        "startreplace" | "startr" => crate::normal::handle(ed, Key::Char('R')),
        "stopinsert" | "stopi" => {
            if ed.mode == crate::mode::Mode::Insert {
                crate::insert::leave_insert(ed);
            }
        }
        "Explore" | "Ex" | "Sexplore" | "Se" | "Vexplore" | "Ve" | "Lexplore" | "Lex" | "Texplore" => {
            ed.toggle_file_tree();
        }
        "spe" | "spellgood" => {
            let word = rest.trim();
            if word.is_empty() {
                ed.set_message("E471: Argument required");
            } else {
                match ed.ensure_dictionary().add_word(word) {
                    Ok(()) => ed.set_message(format!("Added \"{word}\" to the dictionary")),
                    Err(e) => ed.set_message(format!("Could not update dictionary: {e}")),
                }
            }
        }
        "mks" | "mksession" | "mks!" | "mksession!" => run_ex(ed, "sessionsave"),
        "ol" | "oldfiles" => {
            let entries: Vec<_> = ed
                .recent_files
                .iter()
                .enumerate()
                .map(|(i, p)| crate::results::Entry::location(p.clone(), 0, 0, format!("{}: {}", i + 1, p.display())))
                .collect();
            if entries.is_empty() {
                ed.set_message("No recent files");
            } else {
                ed.show_results(crate::results::Results::new("Old files", entries));
            }
        }
        "map" | "nm" | "nmap" | "vm" | "vmap" | "xm" | "xmap" | "im" | "imap" | "no" | "noremap"
        | "nn" | "nnoremap" | "vn" | "vnoremap" | "xn" | "xnoremap" | "ino" | "inoremap" => {
            map_command(ed, name, rest.trim());
        }
        "unm" | "unmap" | "nun" | "nunmap" | "vu" | "vunmap" | "xu" | "xunmap" | "iu" | "iunmap" => {
            unmap_command(ed, name, rest.trim());
        }
        "ab" | "abbreviate" | "ia" | "iabbrev" | "norea" | "noreabbrev" | "inorea" | "inoreabbrev" => {
            abbreviate_command(ed, rest.trim());
        }
        "una" | "unabbreviate" | "iuna" | "iunabbrev" => {
            let lhs = rest.trim();
            if ed.abbreviations.remove(lhs).is_none() {
                ed.set_message("E24: No such abbreviation");
            }
        }
        "abc" | "abclear" | "iabc" | "iabclear" => ed.abbreviations.clear(),
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
                // Bare `:e!` reloads the current buffer from disk.
                match ed.buf_mut().reload() {
                    Ok(()) => ed.set_message("Reloaded"),
                    Err(e) => ed.set_message(format!("could not reload: {e}")),
                }
            } else {
                let path = crate::files::identity(&PathBuf::from(target));
                let loaded = ed.buffers.iter().any(|b| b.path.as_ref() == Some(&path));
                match ed.open_file(path) {
                    Err(e) => ed.set_message(format!("could not open {}: {}", target, e)),
                    Ok(()) if loaded => match ed.buf_mut().reload() {
                        Ok(()) => ed.set_message("Reloaded"),
                        Err(e) => ed.set_message(format!("could not reload: {e}")),
                    },
                    Ok(()) => {},
                }
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
        // `:cfar/pat/repl/` with no space before the delimiter, mirroring how
        // `:s/pat/repl/` is accepted (the spaced `:cfar /pat/repl/` form is
        // handled by the explicit arm above).
        _ if is_far(name) => run_far_replace(ed, &remainder["cfar".len()..]),
        // Only real substitute syntax (`:s` followed by a non-alphanumeric
        // delimiter, or a bare `:s`) routes here -- `:sort`/`:set`/`:sp`
        // and other unknown `s...` commands fall through to the error below.
        _ if is_substitute(name) => run_substitute(ed, remainder, effective_range),
        // `:g/pat/cmd` / `:global` / `:v` / `:vglobal` — run `cmd` on each
        // matching (or, for v/vglobal, non-matching) line.
        // `:[range]!cmd` filters lines through a shell command; `:!cmd` runs one.
        _ if name.starts_with('!') => run_filter(ed, &remainder[1..], effective_range),
        // `:[range]>`/`:<` (repeatable `:>>`) shift lines by `shiftwidth`.
        _ if !name.is_empty()
            && (name.chars().all(|c| c == '>') || name.chars().all(|c| c == '<')) =>
        {
            let right = name.starts_with('>');
            let (l1, l2) = effective_range.unwrap_or((ed.cursor().0, ed.cursor().0));
            let sw = ed.buf().shiftwidth;
            for _ in 0..name.len() {
                crate::operator::indent_lines(ed.buf_mut(), l1, l2, right, sw);
            }
            let l = l1.min(ed.buf().line_count().saturating_sub(1));
            let col = ed.buf().first_non_blank(l);
            ed.set_cursor(l, col);
        }
        _ if parse_global(remainder).is_some() => run_global(ed, remainder, effective_range),
        // `:[range]m{addr}` / `:t{addr}` etc. — the destination address can abut
        // the command letter (`:2m4`) or follow a space (`:2m 4`).
        _ if parse_move_copy(remainder).is_some() => {
            let (copy, dest) = parse_move_copy(remainder).unwrap();
            run_move_copy(ed, dest, effective_range, copy);
        }
        _ => ed.set_message(format!("E492: not an editor command: {}", cmd)),
    }
}

/// `:[range]!cmd` — with a range, filter those lines through the shell command
/// (stdin = the lines, stdout replaces them); with no range, run the command
/// and show its output in a Results list. A user-invoked shell runner, like
/// Vim's `:!` (and `:make`), so it goes through `sh -c`.
fn run_filter(ed: &mut Editor, cmd: &str, range: Option<(usize, usize)>) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let cmd = cmd.trim();
    if cmd.is_empty() {
        ed.set_message("Usage: :[range]!cmd  (filter) or :!cmd (run)");
        return;
    }
    let root = ed.project_root.clone();
    let run = |input: Option<String>| -> std::io::Result<(bool, String)> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let (Some(mut stdin), Some(text)) = (child.stdin.take(), input) {
            // Write on a thread so a large output can't deadlock us mid-write.
            std::thread::spawn(move || {
                let _ = stdin.write_all(text.as_bytes());
            });
        }
        let out = child.wait_with_output()?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            s.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        Ok((out.status.success(), s))
    };
    match range {
        Some((s, e)) => {
            let last = ed.buf().line_count().saturating_sub(1);
            let (s, e) = (s.min(last), e.min(last));
            let input: String = (s..=e)
                .map(|l| format!("{}\n", ed.buf().line_text(l)))
                .collect();
            match run(Some(input)) {
                Ok((_, output)) => {
                    let start = ed.buf().char_idx(s, 0);
                    let end = if e < last {
                        ed.buf().char_idx(e + 1, 0)
                    } else {
                        ed.buf().rope.len_chars()
                    };
                    let ends_nl = end > 0 && ed.buf().rope.char(end - 1) == '\n';
                    // Match the replaced span's trailing-newline shape.
                    let mut repl = output;
                    if ends_nl && !repl.ends_with('\n') {
                        repl.push('\n');
                    } else if !ends_nl {
                        while repl.ends_with('\n') {
                            repl.pop();
                        }
                    }
                    ed.buf_mut().begin_edit();
                    ed.buf_mut().delete_char_range(start, end);
                    ed.buf_mut().insert_str_at(start, &repl);
                    ed.buf_mut().commit_edit();
                    let line = s.min(ed.buf().line_count().saturating_sub(1));
                    ed.set_cursor(line, ed.buf().first_non_blank(line));
                    ed.set_message(format!("Filtered through: {cmd}"));
                }
                Err(err) => ed.set_message(format!("filter failed: {err}")),
            }
        }
        None => match run(None) {
            Ok((ok, output)) => {
                let entries: Vec<crate::results::Entry> = output
                    .lines()
                    .map(|l| crate::results::Entry::text(l.to_string()))
                    .collect();
                if entries.is_empty() {
                    ed.set_message(format!(
                        "{cmd} — {}",
                        if ok { "ok (no output)" } else { "failed" }
                    ));
                } else {
                    ed.show_results(crate::results::Results::new(format!("!{cmd}"), entries));
                }
            }
            Err(err) => ed.set_message(format!("command failed: {err}")),
        },
    }
}

/// Recognize `:move`/`:m`/`:copy`/`:co`/`:t` followed by a destination address
/// (which may abut the letter, as in `:2m4`, or start with a space). Returns
/// `(is_copy, dest)`. The char after the command word must be address-like so
/// real commands (`messages`, `tabnew`, `colorscheme`, …) are never matched.
fn parse_move_copy(remainder: &str) -> Option<(bool, &str)> {
    let addr_like = |s: &str| {
        s.is_empty()
            || s.starts_with([' ', '\t', '.', '$', '\'', '+', '-', '%'])
            || s.starts_with(|c: char| c.is_ascii_digit())
    };
    for (word, copy) in [
        ("move", false),
        ("copy", true),
        ("co", true),
        ("m", false),
        ("t", true),
    ] {
        if let Some(rest) = remainder.strip_prefix(word) {
            if addr_like(rest) {
                return Some((copy, rest));
            }
        }
    }
    None
}

/// `:[range]move {addr}` / `:[range]copy {addr}` (`:m`/`:t`/`:co`). The range
/// lines are moved (or copied) to just after destination line `{addr}`; `0`
/// means before the first line. Rebuilds the affected lines in one edit.
fn run_move_copy(ed: &mut Editor, dest: &str, range: Option<(usize, usize)>, copy: bool) {
    let last = ed.buf().line_count().saturating_sub(1);
    let cur = ed.cursor().0;
    let (s, e) = range.unwrap_or((cur, cur));
    let (s, e) = (s.min(last), e.min(last));
    let arg = dest.trim();
    let dest_after: i64 = if arg == "0" {
        -1
    } else {
        let chars: Vec<char> = arg.chars().collect();
        match parse_one_address(ed, &chars, 0, cur) {
            Ok(Some((d, _))) => d as i64,
            _ => {
                ed.set_message("E14: invalid destination address");
                return;
            }
        }
    };
    // A move into itself is only invalid when the destination is *strictly
    // inside* the block (a line being removed). Destination `s-1` (just before)
    // or `e` (just after) is a valid no-op, as in Vim.
    if !copy && dest_after >= s as i64 && dest_after < e as i64 {
        ed.set_message("E134: cannot move a range into itself");
        return;
    }
    let had_trailing = ed
        .buf()
        .rope
        .len_chars()
        .checked_sub(1)
        .map(|i| ed.buf().rope.char(i) == '\n')
        .unwrap_or(false);
    let mut lines: Vec<String> = (0..=last).map(|l| ed.buf().line_text(l)).collect();
    let block: Vec<String> = lines[s..=e].to_vec();
    let at = if copy {
        (dest_after + 1).clamp(0, lines.len() as i64) as usize
    } else {
        lines.drain(s..=e);
        let removed = (e - s + 1) as i64;
        let adj = if dest_after >= e as i64 {
            dest_after - removed
        } else {
            dest_after
        };
        (adj + 1).clamp(0, lines.len() as i64) as usize
    };
    for (k, ln) in block.iter().enumerate() {
        lines.insert(at + k, ln.clone());
    }
    let mut new = lines.join("\n");
    if had_trailing {
        new.push('\n');
    }
    let total = ed.buf().rope.len_chars();
    ed.buf_mut().begin_edit();
    ed.buf_mut().delete_char_range(0, total);
    ed.buf_mut().insert_str_at(0, &new);
    ed.buf_mut().commit_edit();
    let landing = (at + block.len()).saturating_sub(1);
    let line = landing.min(ed.buf().line_count().saturating_sub(1));
    ed.set_cursor(line, ed.buf().first_non_blank(line));
}

/// Parse a `:g`/`:global`/`:v`/`:vglobal` command: `<word><delim>pat<delim>cmd`.
/// Returns `(invert, pattern, command)`; `invert` is true for `v`/`vglobal`.
/// The delimiter must be a non-alphanumeric char, so real commands like
/// `:gitdiff`/`:vsplit` (a letter follows) never match.
fn parse_global(remainder: &str) -> Option<(bool, &str, &str)> {
    let (invert, after) = ["vglobal", "global", "v", "g"].iter().find_map(|w| {
        remainder
            .strip_prefix(*w)
            .map(|rest| (w.starts_with('v'), rest))
    })?;
    let delim = after.chars().next()?;
    if delim.is_alphanumeric() || delim.is_whitespace() {
        return None;
    }
    let body = &after[delim.len_utf8()..];
    let end = body.find(delim)?;
    Some((invert, &body[..end], &body[end + delim.len_utf8()..]))
}

/// Execute a parsed `:global`/`:vglobal`. Matching lines are collected first,
/// then the command is run on each from the bottom up so line-count changes
/// (e.g. `:g/pat/d`) don't shift the indices still to process.
fn run_global(ed: &mut Editor, remainder: &str, range: Option<(usize, usize)>) {
    let Some((invert, pat, cmd)) = parse_global(remainder) else {
        return;
    };
    let pattern = if pat.is_empty() {
        match ed.last_search.clone() {
            Some((p, _)) => p,
            None => {
                ed.set_message("E35: no previous regular expression");
                return;
            }
        }
    } else {
        crate::vimregex::translate_pattern(pat)
    };
    let re = match fancy_regex::RegexBuilder::new(&pattern)
        .backtrack_limit(100_000)
        .case_insensitive(
            ed.config.ignorecase
                && !(ed.config.smartcase && pattern.chars().any(|c| c.is_uppercase())),
        )
        .build()
    {
        Ok(re) => re,
        Err(e) => {
            ed.set_message(format!("bad pattern: {e}"));
            return;
        }
    };
    // `:g/foo/` sets the last search pattern (like Vim), so a nested empty
    // `:s//repl/` reuses `foo` rather than the stale previous search.
    ed.last_search = Some((pattern.clone(), true));
    let last = ed.buf().line_count().saturating_sub(1);
    let (s, e) = range.unwrap_or((0, last));
    let (s, e) = (s.min(last), e.min(last));
    let matched: Vec<usize> = (s..=e)
        .filter(|&l| re.is_match(&ed.buf().line_text(l)).unwrap_or(false) != invert)
        .collect();
    if matched.is_empty() {
        ed.set_message("global: no matching lines");
        return;
    }
    // Only trim leading space: a trailing space can be significant to a
    // per-line `:normal` (e.g. inserted text).
    let cmd = cmd.trim_start();
    let count = matched.len();
    // Bottom-up so a command that deletes/adds lines keeps earlier indices valid.
    for &l in matched.iter().rev() {
        let line = l.min(ed.buf().line_count().saturating_sub(1));
        ed.set_cursor(line, 0);
        if cmd.is_empty() {
            continue; // Vim would `:p`; here a bare :g just leaves the cursor.
        }
        run_ex(ed, cmd);
    }
    ed.set_message(format!("global: {count} line(s)"));
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
    if let Some(e) = ed
        .reviews
        .active
        .as_ref()
        .filter(|a| a.has_pending())
        .and_then(|a| a.write_error.clone())
    {
        names.push(format!(
            "review comments not yet written ({e}; :commentswrite retries)"
        ));
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
fn close_current_or_quit(ed: &mut Editor, over_results: bool) {
    // `:q` from a Results overlay just returns to the buffer: pressing `:`
    // already left the panel (mode is Normal now), so there's nothing to quit.
    if over_results {
        return;
    }
    if ed.windows.len() > 1 && !ed.only_sidebars_remain() {
        ed.close_window();
        return;
    }
    ed.should_quit = true;
}

pub(crate) fn remove_current_buffer(ed: &mut Editor) {
    ed.buffers.remove(ed.cur);
    if ed.buffers.is_empty() {
        ed.buffers.push(crate::buffer::Buffer::empty());
        ed.cur = 0;
    } else if ed.cur >= ed.buffers.len() {
        ed.cur = ed.buffers.len() - 1;
    }
    let ids: Vec<_> = ed.buffers.iter().map(|b| b.id).collect();
    // Shut down any terminal hosted by a pane that showed the removed buffer
    // BEFORE the panes are dropped -- otherwise the child process, its PTY, and
    // its reader thread leak (a bare `windows.retain`, or `close_window`'s
    // collapse of the last split, would drop the pane without shutting it down).
    let doomed_terminals: Vec<u64> = ed
        .windows
        .iter()
        .filter(|w| !ids.contains(&w.buffer))
        .filter_map(|w| w.terminal)
        .collect();
    for id in doomed_terminals {
        ed.shutdown_terminal(id);
    }
    // Drop the doomed panes through the normal close path so `window_layout`
    // leaf indices stay consistent (a bare `retain` leaves them dangling and
    // renders a stale half-pane), then fall back to the implicit single window
    // if a collapsed single pane still shows the removed buffer.
    while ed.windows.len() > 1 {
        match ed.windows.iter().position(|w| !ids.contains(&w.buffer)) {
            Some(idx) => {
                ed.active_window = idx;
                ed.close_window();
            }
            None => break,
        }
    }
    if ed.windows.iter().any(|w| !ids.contains(&w.buffer)) {
        ed.windows.clear();
        ed.window_layout = None;
        ed.active_window = 0;
    }
    ed.buffer_mru.retain(|id| ids.contains(id));
    ed.active_window = ed.active_window.min(ed.windows.len().saturating_sub(1));
    ed.invalidate_index_caches();
}

/// `<leader>q` drops the active buffer while keeping its window when another
/// buffer exists; closing the last buffer quits after the usual dirty check.
/// Other panes that showed the removed buffer move to the replacement too.
pub fn close_current_buffer_keep_window(ed: &mut Editor) {
    if ed.buffers.len() == 1 {
        if let Some(message) = modified_buffers_message(ed) {
            ed.set_message(message);
        } else {
            ed.should_quit = true;
        }
        return;
    }
    if ed.buf().is_modified() {
        ed.set_message("unsaved changes -- use :bd! to discard");
        return;
    }
    let removed_id = ed.buf().id;
    ed.buffers.remove(ed.cur);
    ed.cur = ed.cur.min(ed.buffers.len() - 1);
    let replacement = ed.buf().id;
    for window in &mut ed.windows {
        if window.buffer == removed_id {
            window.buffer = replacement;
            window.cursor = (0, 0);
            window.top = 0;
            window.wrap_row = 0;
            window.left = 0;
        }
    }
    ed.buffer_mru.retain(|id| *id != removed_id);
    ed.invalidate_index_caches();
    ed.set_message("Closed current buffer");
}

/// Whether `:q` should close the current view buffer (see `Buffer::view`)
/// rather than its window or the editor: it is the only editing pane and
/// another buffer exists to return to.
fn is_closable_view(ed: &Editor) -> bool {
    ed.buf().view && ed.buffers.len() > 1 && (ed.windows.len() <= 1 || ed.only_sidebars_remain())
}

/// Drops the current view buffer and shows the buffer it was opened over
/// (the alternate, else the most recent, else a neighbour) in its place.
fn close_view(ed: &mut Editor) {
    let removed_id = ed.buf().id;
    let back = ed
        .alternate_buffer
        .into_iter()
        .chain(ed.buffer_mru.iter().copied())
        .find(|id| *id != removed_id && ed.buffers.iter().any(|b| b.id == *id));
    ed.buffers.remove(ed.cur);
    ed.cur = back
        .and_then(|id| ed.buffers.iter().position(|b| b.id == id))
        .unwrap_or(ed.cur.min(ed.buffers.len() - 1));
    let replacement = ed.buf().id;
    let (line, col) = (ed.buf().cursor_line, ed.buf().cursor_col);
    let top = ed.buf().top_line;
    for window in &mut ed.windows {
        if window.buffer == removed_id {
            window.buffer = replacement;
            window.cursor = (line, col);
            window.top = top;
            window.wrap_row = 0;
            window.left = 0;
        }
    }
    if ed.alternate_buffer == Some(removed_id) || ed.alternate_buffer == Some(replacement) {
        ed.alternate_buffer = None;
    }
    ed.buffer_mru.retain(|id| *id != removed_id);
    ed.touch_buffer_mru(replacement);
    if ed
        .task_draft
        .as_ref()
        .is_some_and(|(id, _, _)| *id == removed_id)
    {
        ed.task_draft = None;
    }
    if ed
        .tour_draft
        .as_ref()
        .is_some_and(|(_, id)| *id == removed_id)
    {
        ed.tour_draft = None;
    }
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

/// True when `name` is a no-space `:cfar/pat/repl/` -- `cfar` immediately
/// followed by a non-alphanumeric delimiter. The bare/spaced `cfar` forms are
/// matched by the explicit dispatch arm instead.
fn is_far(name: &str) -> bool {
    name.strip_prefix("cfar")
        .and_then(|r| r.chars().next())
        .is_some_and(|c| !c.is_alphanumeric())
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
                // Saturating so an absurdly long address can't overflow-panic.
                n = n.saturating_mul(10).saturating_add(d as i64);
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
            n = n.saturating_mul(10).saturating_add(d as i64);
            i += 1;
            had = true;
        }
        if !had {
            n = 1;
        }
        let base = line.unwrap_or(cur as i64);
        // Saturating so a huge offset (e.g. `:2+9999999999999999999`) can't
        // overflow-panic in debug / wrap in release; it clamps to the range
        // just below.
        line = Some(base.saturating_add(sign.saturating_mul(n)));
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
/// `:cfar/pat/repl/[flags]` -- project-wide find & replace across every file in
/// the current results/quickfix list (typically produced by a prior `:grep`).
/// Each file's buffer gets a whole-file substitution (reusing `run_substitute`,
/// so regex/flags/capture-group semantics match `:s`) and is saved; the
/// original buffer is refocused afterward. A no-op on files with no match.
fn run_far_replace(ed: &mut Editor, body: &str) {
    let body = body.trim();
    // Accept both `cfar/pat/repl/` and `cfar s/pat/repl/`; normalize to the
    // leading-`s` form `run_substitute` expects.
    let sub_body = match body.strip_prefix('s') {
        Some(rest) if rest.chars().next().is_some_and(|c| !c.is_alphanumeric()) => body.to_string(),
        _ => format!("s{body}"),
    };
    // Validate the pattern once up front so a typo reports a real error rather
    // than silently "replacing across 0 files".
    let after_s = &sub_body[1..];
    let Some(delim) = after_s.chars().next() else {
        ed.set_message("E486: pattern required");
        return;
    };
    let parts: Vec<&str> = after_s[delim.len_utf8()..].splitn(3, delim).collect();
    if parts.len() < 2 || parts[0].is_empty() {
        ed.set_message("E486: incomplete substitute (need cfar/pat/repl/)");
        return;
    }
    let translated = crate::vimregex::translate_pattern(parts[0]);
    if fancy_regex::Regex::new(&translated).is_err() {
        ed.set_message(format!("bad pattern: {}", parts[0]));
        return;
    }
    // Collect the unique files from the active results list (results first,
    // then a standalone quickfix list).
    let list = ed.results.as_ref().or(ed.quickfix.as_ref());
    let Some(list) = list else {
        ed.set_message("no results list -- run :grep first");
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for e in &list.entries {
        if let Some(p) = &e.path {
            if !files.contains(p) {
                files.push(p.clone());
            }
        }
    }
    if files.is_empty() {
        ed.set_message("results list has no files to replace in");
        return;
    }
    let origin = ed.buf().path.clone();
    let mut changed = 0usize;
    for path in &files {
        if ed.open_file(path.clone()).is_err() {
            continue;
        }
        let before = ed.buf().edit_seq;
        let last = ed.buf().line_count().saturating_sub(1);
        run_substitute(ed, &sub_body, Some((0, last)));
        if ed.buf().edit_seq != before {
            let _ = ed.save_current();
            changed += 1;
        }
    }
    if let Some(origin) = origin {
        let _ = ed.open_file(origin);
    }
    ed.set_message(format!(
        "cfar: replaced in {changed} of {} file(s)",
        files.len()
    ));
}

/// Parse a `cfar`/`far` body (`[s]/pat/repl/[flags]`) into a compiled regex,
/// translated replacement, and the global flag — mirroring `run_substitute`'s
/// pattern/replacement/flag handling so a preview matches the real replace.
fn far_regex(ed: &Editor, body: &str) -> Result<(fancy_regex::Regex, String, bool), String> {
    let body = body.trim();
    let sub_body = match body.strip_prefix('s') {
        Some(rest) if rest.chars().next().is_some_and(|c| !c.is_alphanumeric()) => body.to_string(),
        _ => format!("s{body}"),
    };
    let after_s = &sub_body[1..];
    let delim = after_s.chars().next().ok_or("E486: pattern required")?;
    let parts: Vec<&str> = after_s[delim.len_utf8()..].splitn(3, delim).collect();
    if parts.len() < 2 || parts[0].is_empty() {
        return Err("E486: incomplete substitute (need cfar/pat/repl/)".into());
    }
    let pattern = crate::vimregex::translate_pattern(parts[0]);
    let replacement = crate::vimregex::translate_replacement(parts[1]);
    let flags = parts.get(2).copied().unwrap_or("");
    if flags.chars().any(|c| !matches!(c, 'g' | 'i' | 'I')) {
        return Err("unsupported substitute flag (supported: g i I)".into());
    }
    let re = fancy_regex::RegexBuilder::new(&pattern)
        .backtrack_limit(100_000)
        .case_insensitive(
            flags.contains('i')
                || (!flags.contains('I')
                    && ed.config.ignorecase
                    && !(ed.config.smartcase && pattern.chars().any(|c| c.is_uppercase()))),
        )
        .build()
        .map_err(|e| format!("bad pattern: {e}"))?;
    Ok((re, replacement, flags.contains('g')))
}

/// `:cfarpreview /pat/repl/[flags]` — show every line the matching `:cfar`
/// would change across the results/quickfix files, in a Results list, WITHOUT
/// modifying anything. Open buffers are read in their current (possibly
/// unsaved) state; the rest are read from disk. Bounded so a huge result set
/// can't blow up. Run `:cfar …` to actually apply.
fn run_far_preview(ed: &mut Editor, body: &str) {
    let (re, replacement, global) = match far_regex(ed, body) {
        Ok(v) => v,
        Err(e) => {
            ed.set_message(e);
            return;
        }
    };
    let list = ed.results.as_ref().or(ed.quickfix.as_ref());
    let Some(list) = list else {
        ed.set_message("no results list -- run :grep first");
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for e in &list.entries {
        if let Some(p) = &e.path {
            if !files.contains(p) {
                files.push(p.clone());
            }
        }
    }
    if files.is_empty() {
        ed.set_message("results list has no files to preview");
        return;
    }
    let count = if global { 0 } else { 1 };
    let mut entries = Vec::new();
    let mut changed_files = 0usize;
    'files: for path in &files {
        // Prefer an open buffer's live text; otherwise read the file.
        let text = match ed.buffers.iter().find(|b| b.path.as_ref() == Some(path)) {
            Some(b) => b.rope.to_string(),
            None => match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(_) => continue,
            },
        };
        let mut file_changed = false;
        for (lineno, line) in text.lines().enumerate() {
            let Ok(new) = re.try_replacen(line, count, replacement.as_str()) else {
                continue;
            };
            if new != line {
                file_changed = true;
                entries.push(crate::results::Entry::location(
                    path.clone(),
                    lineno,
                    0,
                    new.trim_end().to_string(),
                ));
                if entries.len() >= 5000 {
                    break 'files;
                }
            }
        }
        if file_changed {
            changed_files += 1;
        }
    }
    if entries.is_empty() {
        ed.set_message("cfar preview: no matches in the results files");
        return;
    }
    let title = format!(
        "cfar preview — {} change(s) in {changed_files} file(s) (run :cfar to apply)",
        entries.len()
    );
    ed.show_results(crate::results::Results::new(title, entries));
}

/// Parse `#rgb`/`#rrggbb` from a hex string into `(r, g, b)`.
/// The first integer in a line (optional leading `-`), for `:sort n`. Lines
/// with no number sort as 0, matching Vim's "numeric sort" placing them first.
fn leading_number(line: &str) -> i64 {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let neg = i > 0 && chars[i - 1] == '-';
            let mut n: i64 = 0;
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                n = n
                    .saturating_mul(10)
                    .saturating_add((chars[j] as u8 - b'0') as i64);
                j += 1;
            }
            return if neg { -n } else { n };
        }
        i += 1;
    }
    0
}

fn parse_hex_color(h: &str) -> Option<(u8, u8, u8)> {
    let full: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    Some((
        u8::from_str_radix(&full[0..2], 16).ok()?,
        u8::from_str_radix(&full[2..4], 16).ok()?,
        u8::from_str_radix(&full[4..6], 16).ok()?,
    ))
}

/// The hex color token (`#rgb`/`#rrggbb`) covering the cursor column on the
/// current line: `(start_col, end_col_exclusive, (r, g, b))`.
fn color_at_cursor(ed: &Editor) -> Option<(usize, usize, (u8, u8, u8))> {
    let (line, col) = ed.cursor();
    let chars: Vec<char> = ed.buf().line_text(line).chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_hexdigit() {
                j += 1;
            }
            let len = j - i - 1;
            if len == 3 || len == 6 {
                let end = i + 1 + len;
                if col >= i && col < end {
                    let hex: String = chars[i + 1..end].iter().collect();
                    return parse_hex_color(&hex).map(|rgb| (i, end, rgb));
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// `:colorlighten`/`:colordarken` — scale each channel of the hex under the
/// cursor toward white (`pct > 0`) or black (`pct < 0`) and rewrite it as
/// `#rrggbb`.
fn color_adjust(ed: &mut Editor, pct: i32) {
    let Some((start, end, (r, g, b))) = color_at_cursor(ed) else {
        ed.set_message("No hex color under the cursor");
        return;
    };
    let adj = |c: u8| -> u8 {
        let c = c as i32;
        let nc = if pct >= 0 {
            c + (255 - c) * pct / 100
        } else {
            c + c * pct / 100
        };
        nc.clamp(0, 255) as u8
    };
    let (nr, ng, nb) = (adj(r), adj(g), adj(b));
    let new = format!("#{nr:02x}{ng:02x}{nb:02x}");
    let line = ed.cursor().0;
    let a = ed.buf().char_idx(line, start);
    let z = ed.buf().char_idx(line, end);
    ed.buf_mut().begin_edit();
    ed.buf_mut().delete_char_range(a, z);
    ed.buf_mut().insert_str_at(a, &new);
    ed.buf_mut().commit_edit();
    ed.set_cursor(line, start);
    ed.set_message(format!("{new}  rgb({nr}, {ng}, {nb})"));
}

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
    // Like Vim, `:s` makes its pattern the last search pattern (`n`, `g&`).
    if !parts[0].is_empty() {
        ed.last_search = Some((pattern.clone(), true));
    }
    ed.last_substitute = Some((
        parts[0].clone(),
        parts[1].clone(),
        parts.get(2).cloned().unwrap_or_default(),
    ));
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

// ---------------------------------------------------------------------
// Helpers for the Vim-parity Ex commands.
// ---------------------------------------------------------------------

/// Text of lines `l1..=l2`, each ending in a newline.
fn lines_text(ed: &Editor, l1: usize, l2: usize) -> String {
    (l1..=l2.min(ed.buf().line_count().saturating_sub(1)))
        .map(|l| ed.buf().line_text(l) + "\n")
        .collect()
}

/// Runs `cmd` through `sh -c` in the project root, returning stdout (and
/// stderr appended) or an error message.
fn shell_output(ed: &Editor, cmd: &str) -> Result<String, String> {
    if cmd.is_empty() {
        return Err("E471: Argument required".into());
    }
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(&ed.project_root)
        .output()
        .map_err(|e| format!("failed to run {cmd}: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(text)
}

/// `:w !cmd`: feeds `input` to `cmd` and shows what it prints.
fn pipe_to_command(ed: &mut Editor, cmd: &str, input: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if cmd.is_empty() {
        ed.set_message("E471: Argument required");
        return;
    }
    let child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(&ed.project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            ed.set_message(format!("failed to run {cmd}: {e}"));
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    match child.wait_with_output() {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            let lines: Vec<_> = text.lines().map(crate::results::Entry::text).collect();
            match lines.len() {
                0 => ed.set_message(format!("{cmd}: exit {}", out.status.code().unwrap_or(-1))),
                1 => ed.set_message(lines[0].text.clone()),
                _ => ed.show_results(crate::results::Results::new(format!(":w !{cmd}"), lines)),
            }
        }
        Err(e) => ed.set_message(format!("{cmd}: {e}")),
    }
}

/// Inserts `text` as whole lines below line `at` (`:r`), as one undo step.
fn insert_lines_below(ed: &mut Editor, at: usize, text: &str) {
    if text.is_empty() {
        ed.set_message("Nothing to read");
        return;
    }
    let mut body = text.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    let n = body.matches('\n').count();
    ed.buf_mut().begin_edit();
    let next = at + 1;
    let idx = if next < ed.buf().line_count() {
        ed.buf().char_idx(next, 0)
    } else {
        let end = ed.buf().rope.len_chars();
        if !ed.buf().rope.to_string().ends_with('\n') {
            ed.buf_mut().insert_str_at(end, "\n");
        }
        ed.buf().rope.len_chars()
    };
    ed.buf_mut().insert_str_at(idx, &body);
    ed.buf_mut().commit_edit();
    let (l, _) = ed.buf().pos_from_char_idx(idx);
    let c = ed.buf().first_non_blank(l);
    ed.set_cursor(l, c);
    ed.set_message(format!("{n} line(s) read"));
}

/// `:retab`: with `expandtab`, tabs become spaces; with `noexpandtab` and
/// `!`, runs of leading spaces become tabs.
fn retab(ed: &mut Editor, l1: usize, l2: usize, bang: bool) {
    let ts = ed.buf().tabstop.max(1);
    let expand = ed.buf().expandtab;
    ed.buf_mut().begin_edit();
    let mut changed = 0;
    for l in l1..=l2.min(ed.buf().line_count().saturating_sub(1)) {
        let text = ed.buf().line_text(l);
        let new = if expand {
            let mut out = String::new();
            let mut cell = 0;
            for c in text.chars() {
                if c == '\t' {
                    let n = ts - cell % ts;
                    out.push_str(&" ".repeat(n));
                    cell += n;
                } else {
                    out.push(c);
                    cell += 1;
                }
            }
            out
        } else if bang {
            let lead: String = text
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let width: usize =
                lead.chars()
                    .fold(0, |w, c| if c == '\t' { w + ts - w % ts } else { w + 1 });
            let tabs = "\t".repeat(width / ts) + &" ".repeat(width % ts);
            tabs + &text[lead.len()..]
        } else {
            continue;
        };
        if new != text {
            let start = ed.buf().char_idx(l, 0);
            let end = ed.buf().char_idx(l, ed.buf().line_len(l));
            ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut().insert_str_at(start, &new);
            changed += 1;
        }
    }
    ed.buf_mut().commit_edit();
    ed.set_message(format!("retab: {changed} line(s) changed"));
}

/// `:delmarks a b-d`: deletes the named marks (ranges allowed).
fn delete_marks(ed: &mut Editor, arg: &str) {
    if arg.is_empty() {
        ed.set_message("E471: Argument required");
        return;
    }
    let chars: Vec<char> = arg.chars().filter(|c| !c.is_whitespace()).collect();
    let mut names = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' {
            for c in chars[i]..=chars[i + 2] {
                names.push(c);
            }
            i += 3;
        } else {
            names.push(chars[i]);
            i += 1;
        }
    }
    let before = ed.marks.len();
    for c in &names {
        ed.marks.remove(c);
    }
    ed.set_message(format!("Deleted {} mark(s)", before - ed.marks.len()));
}

/// `:center` / `:right` / `:left` on lines `l1..=l2`.
fn align_lines(ed: &mut Editor, l1: usize, l2: usize, kind: char, arg: Option<usize>, tw: usize) {
    ed.buf_mut().begin_edit();
    for l in l1..=l2.min(ed.buf().line_count().saturating_sub(1)) {
        let text = ed.buf().line_text(l);
        let body = text.trim();
        let len = body.chars().count();
        let new = match kind {
            'l' => format!("{}{body}", " ".repeat(arg.unwrap_or(0))),
            'r' => format!(
                "{}{body}",
                " ".repeat(arg.unwrap_or(tw).saturating_sub(len))
            ),
            _ => format!(
                "{}{body}",
                " ".repeat(arg.unwrap_or(tw).saturating_sub(len) / 2)
            ),
        };
        let new = if body.is_empty() { String::new() } else { new };
        if new != text {
            let start = ed.buf().char_idx(l, 0);
            let end = ed.buf().char_idx(l, ed.buf().line_len(l));
            ed.buf_mut().delete_char_range(start, end);
            ed.buf_mut().insert_str_at(start, &new);
        }
    }
    ed.buf_mut().commit_edit();
}

/// `:resize [+-]N` / `:vertical resize [+-]N`: the active pane's height
/// (or width) in rows/columns.
fn resize_command(ed: &mut Editor, arg: &str, vertical: bool) {
    let (cols, rows) = crossterm::terminal::size()
        .map(|(c, r)| (c as usize, r as usize))
        .unwrap_or((ed.screen_cols.max(1), ed.screen_rows.max(1) + 2));
    if ed.windows.len() < 2 {
        ed.set_message("Only one window");
        return;
    }
    let rects = ed.pane_rects(cols, rows);
    let Some(r) = rects.get(ed.active_window) else {
        return;
    };
    let current = if vertical { r.width } else { r.height };
    let total = if vertical { cols } else { rows }.max(1);
    let target = if let Some(n) = arg.strip_prefix('+') {
        current + n.parse::<usize>().unwrap_or(1)
    } else if let Some(n) = arg.strip_prefix('-') {
        current.saturating_sub(n.parse::<usize>().unwrap_or(1))
    } else if arg.is_empty() {
        total
    } else {
        match arg.parse::<usize>() {
            Ok(n) => n,
            Err(_) => {
                ed.set_message("E475: Invalid argument");
                return;
            }
        }
    };
    let delta = (target as f32 - current as f32) / total as f32;
    let active = ed.active_window;
    if let Some(l) = &mut ed.window_layout {
        l.resize_active(active, vertical, delta);
    }
}

/// `&`, `:&&`, `:~`, `g&`: repeat the last `:s`. `keep_flags` keeps its
/// flags; `last_search` swaps in the last search pattern (`:~`, `g&`).
fn repeat_substitute(
    ed: &mut Editor,
    range: Option<(usize, usize)>,
    keep_flags: bool,
    last_search: bool,
) {
    let Some((pat, rep, flags)) = ed.last_substitute.clone() else {
        ed.set_message("E35: No previous substitute");
        return;
    };
    let esc = |s: &str| s.replace('/', "\\/");
    let pat = if last_search {
        String::new()
    } else {
        esc(&pat)
    };
    let flags = if keep_flags { flags } else { String::new() };
    let range = range.or(Some((ed.cursor().0, ed.cursor().0)));
    let remembered = ed.last_substitute.clone();
    run_substitute(ed, &format!("s/{pat}/{}/{flags}", esc(&rep)), range);
    // A repeat doesn't replace what is remembered (its flags in particular).
    ed.last_substitute = remembered;
}

/// `:enew`: a new empty, unnamed buffer in the current window.
fn open_empty_buffer(ed: &mut Editor) {
    let b = crate::buffer::Buffer::empty();
    let id = b.id;
    ed.buffers.push(b);
    ed.note_alternate_buffer();
    ed.cur = ed.buffers.len() - 1;
    ed.touch_buffer_mru(id);
    if let Some(w) = ed.windows.get_mut(ed.active_window) {
        w.buffer = id;
        w.cursor = (0, 0);
        w.top = 0;
    }
    ed.invalidate_index_caches();
    ed.set_cursor(0, 0);
}

/// `:next` / `:prev` (`delta` ±1) or reopen the current entry (`0`).
fn step_arglist(ed: &mut Editor, delta: isize) {
    if ed.arglist.is_empty() {
        ed.set_message("E163: There is only one file to edit");
        return;
    }
    let next = ed.arg_index as isize + delta;
    if next < 0 {
        ed.set_message("E164: Cannot go before first file");
        return;
    }
    if next as usize >= ed.arglist.len() {
        ed.set_message("E165: Cannot go beyond last file");
        return;
    }
    ed.arg_index = next as usize;
    let path = ed.arglist[ed.arg_index].clone();
    match ed.open_file(path) {
        Ok(()) => ed.set_message(format!(
            "\"{}\" ({} of {})",
            ed.buf().name(),
            ed.arg_index + 1,
            ed.arglist.len()
        )),
        Err(e) => ed.set_message(e.to_string()),
    }
}

/// `:vimgrep /pat/ [files]`: regex matches in the current buffer (`%` or
/// no files) or in project files matching the given globs, as a quickfix
/// (or location) list.
fn vimgrep(ed: &mut Editor, arg: &str, loclist: bool) {
    let arg = arg.trim();
    let (pattern, files) = match arg.chars().next() {
        Some(d) if !d.is_alphanumeric() && d != '\\' => match arg[d.len_utf8()..].split_once(d) {
            Some((p, rest)) => (
                p.to_string(),
                rest.trim_start_matches(['g', 'j']).trim().to_string(),
            ),
            None => (arg[d.len_utf8()..].to_string(), String::new()),
        },
        _ => match arg.split_once(char::is_whitespace) {
            Some((p, rest)) => (p.to_string(), rest.trim().to_string()),
            None => (arg.to_string(), String::new()),
        },
    };
    if pattern.is_empty() {
        ed.set_message("E683: File name missing or invalid pattern");
        return;
    }
    let translated = crate::vimregex::translate_pattern(&pattern);
    let re = match crate::search::compile(&translated, ed.config.ignorecase, ed.config.smartcase) {
        Ok(r) => r,
        Err(e) => {
            ed.set_message(format!("Invalid pattern: {e}"));
            return;
        }
    };
    let mut entries = Vec::new();
    let mut scan = |path: Option<PathBuf>, text: &str, buffer: Option<u64>| {
        for (l, line) in text.lines().enumerate() {
            if let Ok(Some(m)) = re.find(line) {
                let col = line[..m.start()].chars().count();
                let mut e = match &path {
                    Some(p) => {
                        crate::results::Entry::location(p.clone(), l, col, line.trim().to_string())
                    }
                    None => crate::results::Entry::text(line.trim().to_string()),
                };
                e.buffer_id = buffer;
                e.line = l;
                e.col = col;
                entries.push(e);
            }
        }
    };
    if files.is_empty() || files == "%" {
        let text = ed.buf().rope.to_string();
        scan(ed.buf().path.clone(), &text, Some(ed.buf().id));
    } else {
        let globs: Vec<regex::Regex> = files
            .split_whitespace()
            .filter_map(|g| {
                let mut re = String::from("^");
                let chars: Vec<char> = g.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    match chars[i] {
                        '*' if chars.get(i + 1) == Some(&'*') => {
                            re.push_str(".*");
                            i += 1;
                            if chars.get(i + 1) == Some(&'/') {
                                i += 1;
                                re.push_str("/?");
                            }
                        }
                        '*' => re.push_str("[^/]*"),
                        '?' => re.push_str("[^/]"),
                        c => re.push_str(&regex::escape(&c.to_string())),
                    }
                    i += 1;
                }
                re.push('$');
                regex::Regex::new(&re).ok()
            })
            .collect();
        let scanned;
        let all: &[String] = if ed.all_files.is_empty() {
            scanned = crate::picker::scan_files(&ed.project_root);
            &scanned
        } else {
            &ed.all_files
        };
        let files: Vec<String> = all
            .iter()
            .filter(|f| globs.iter().any(|g| g.is_match(f)))
            .take(5000)
            .cloned()
            .collect();
        for f in files {
            let path = ed.project_root.join(&f);
            if let Ok(text) = std::fs::read_to_string(&path) {
                scan(Some(path), &text, None);
            }
        }
    }
    if entries.is_empty() {
        ed.set_message(format!("E480: No match: {pattern}"));
        return;
    }
    let title = format!(":vimgrep {pattern} ({} match(es))", entries.len());
    if loclist {
        let mut r = crate::results::Results::new(title, entries);
        r.quickfix = true;
        r.live = false;
        ed.show_results(r);
    } else {
        set_quickfix_entries(ed, title, entries, true);
    }
}

/// Makes `entries` the current quickfix list; `jump` also goes to the first.
fn set_quickfix_entries(
    ed: &mut Editor,
    title: impl Into<String>,
    entries: Vec<crate::results::Entry>,
    jump: bool,
) {
    if entries.is_empty() {
        ed.set_message("No errors");
        return;
    }
    let n = entries.len();
    let mut r = crate::results::Results::new(title, entries);
    r.quickfix = true;
    r.live = false;
    let first = r.entries.first().cloned();
    ed.set_quickfix_list(r);
    if let (true, Some(e)) = (jump, first) {
        if let Some(p) = e.path.clone().filter(|p| ed.buf().path.as_ref() != Some(p)) {
            if let Err(err) = ed.open_file(p) {
                ed.set_message(err.to_string());
                return;
            }
        }
        ed.push_jump();
        let l = e.line.min(ed.buf().line_count().saturating_sub(1));
        ed.set_cursor(l, e.col.min(ed.buf().line_len(l)));
    }
    ed.set_message(format!(
        "{n} quickfix entr{}",
        if n == 1 { "y" } else { "ies" }
    ));
}

/// `:cnfile` / `:cpfile`: the first entry of the next / previous file in
/// the quickfix list.
fn quickfix_file_step(ed: &mut Editor, forward: bool) {
    let Some(qf) = ed.quickfix.as_ref() else {
        ed.set_message("E42: No Errors");
        return;
    };
    let entries = qf.entries.clone();
    if entries.is_empty() {
        ed.set_message("E42: No Errors");
        return;
    }
    let current = ed.buf().path.clone();
    let here = entries.iter().position(|e| e.path == current).unwrap_or(0);
    let target = if forward {
        entries[here..]
            .iter()
            .position(|e| e.path != current)
            .map(|i| here + i)
    } else {
        let prev_path = entries[..here]
            .iter()
            .rev()
            .find(|e| e.path != current)
            .and_then(|e| e.path.clone());
        prev_path.and_then(|p| entries.iter().position(|e| e.path.as_ref() == Some(&p)))
    };
    match target.and_then(|i| entries.get(i).cloned()) {
        Some(e) => {
            if let Some(p) = e.path.clone() {
                ed.push_jump();
                if let Err(err) = ed.open_file(p) {
                    ed.set_message(err.to_string());
                    return;
                }
                let l = e.line.min(ed.buf().line_count().saturating_sub(1));
                ed.set_cursor(l, e.col.min(ed.buf().line_len(l)));
            }
        }
        None => ed.set_message("E553: No more items"),
    }
}

/// `:checktime`: reload clean buffers changed on disk; flag modified ones.
fn check_disk_changes(ed: &mut Editor) {
    let mut reloaded = 0;
    let mut conflicts = Vec::new();
    for b in ed.buffers.iter_mut() {
        if b.path.is_none() || !b.changed_on_disk() {
            continue;
        }
        if b.is_modified() {
            b.disk_changed = true;
            conflicts.push(b.name());
        } else if b.reload().is_ok() {
            reloaded += 1;
        }
    }
    ed.set_message(if conflicts.is_empty() {
        format!("{reloaded} buffer(s) reloaded from disk")
    } else {
        format!(
            "{reloaded} reloaded; changed on disk and here: {} (:e! to reload)",
            conflicts.join(", ")
        )
    });
}

/// Splits `:execute` / `:echo` arguments: quoted strings lose their quotes
/// and are joined, `"a" . "b"` concatenates.
fn split_exe_args(arg: &str) -> Vec<String> {
    let mut out = String::new();
    let mut chars = arg.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                while let Some(n) = chars.next() {
                    if n == c {
                        break;
                    }
                    if n == '\\' && c == '"' {
                        match chars.next() {
                            Some('n') => out.push('\n'),
                            Some('t') => out.push('\t'),
                            Some(o) => out.push(o),
                            None => {}
                        }
                    } else {
                        out.push(n);
                    }
                }
            }
            '.' => {}
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out.split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn shellexpand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

fn map_modes(name: &str) -> &'static str {
    match name.chars().next() {
        Some('n') if name != "no" && name != "noremap" => "n",
        Some('v') | Some('x') => "v",
        Some('i') => "i",
        _ => "nv",
    }
}

/// `:map` family: add a mapping for this session (`:map` alone lists
/// them). Same engine as `[[keymap]]`: single keys and leader sequences.
fn map_command(ed: &mut Editor, name: &str, arg: &str) {
    let arg = arg
        .trim_start_matches("<silent>")
        .trim_start_matches("<buffer>")
        .trim_start_matches("<nowait>")
        .trim();
    let Some((lhs, rhs)) = arg.split_once(char::is_whitespace) else {
        let entries: Vec<_> = ed
            .keymaps
            .iter()
            .map(|k| crate::results::Entry::text(format!("{:?} -> {:?}", k.lhs, k.rhs)))
            .collect();
        if entries.is_empty() {
            ed.set_message("No mappings");
        } else {
            ed.show_results(crate::results::Results::new("Mappings", entries));
        }
        return;
    };
    let cfg = crate::config::KeymapCfg {
        mode: map_modes(name).to_string(),
        lhs: lhs.to_string(),
        rhs: rhs.trim().to_string(),
    };
    let leader = ed.config.leader.chars().next().unwrap_or(',');
    let built = crate::keymap::build(&[cfg], leader);
    let Some(map) = built.into_iter().next() else {
        ed.set_message("E474: Invalid argument");
        return;
    };
    if map.leader_seq.is_none() && map.lhs.len() > 1 {
        ed.set_message("Only single-key and <leader> mappings are supported");
        return;
    }
    ed.keymaps
        .retain(|k| !(k.lhs == map.lhs && k.modes == map.modes));
    ed.keymaps.push(map);
    ed.set_message(format!("Mapped {lhs}"));
}

/// `:unmap {lhs}` (and the mode variants).
fn unmap_command(ed: &mut Editor, name: &str, lhs: &str) {
    if lhs.is_empty() {
        ed.set_message("E471: Argument required");
        return;
    }
    let leader = ed.config.leader.chars().next().unwrap_or(',');
    let keys = crate::keymap::parse_keys(lhs, leader);
    let modes = map_modes(name.trim_start_matches("un").trim_start_matches('u'));
    let before = ed.keymaps.len();
    ed.keymaps.retain(|k| {
        let in_mode = (modes.contains('n') && k.modes.normal)
            || (modes.contains('v') && k.modes.visual)
            || (modes.contains('i') && k.modes.insert);
        !(k.lhs == keys && in_mode)
    });
    if ed.keymaps.len() == before {
        ed.set_message("E31: No such mapping");
    }
}

/// `:abbreviate {lhs} {rhs}` (or `:ab` alone to list them).
fn abbreviate_command(ed: &mut Editor, arg: &str) {
    match arg.split_once(char::is_whitespace) {
        Some((lhs, rhs)) => {
            ed.abbreviations
                .insert(lhs.to_string(), rhs.trim().to_string());
        }
        None if arg.is_empty() => {
            let mut list: Vec<_> = ed
                .abbreviations
                .iter()
                .map(|(k, v)| crate::results::Entry::text(format!("i  {k}  {v}")))
                .collect();
            list.sort_by(|a, b| a.text.cmp(&b.text));
            if list.is_empty() {
                ed.set_message("No abbreviation found");
            } else {
                ed.show_results(crate::results::Results::new("Abbreviations", list));
            }
        }
        None => match ed.abbreviations.get(arg) {
            Some(v) => ed.set_message(format!("i  {arg}  {v}")),
            None => ed.set_message("No abbreviation found"),
        },
    }
}
