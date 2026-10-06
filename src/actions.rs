//! Central action registry: every leader-key command has one id, title,
//! default key sequence and handler here. `run_leader` in `normal.rs`, the
//! `:keymaps` command picker and the which-key prefix popup are all
//! generated from this single table instead of keeping separate lists that
//! can drift out of sync.
use crate::editor::Editor;
use crate::mode::{CommandKind, Mode, VisualKind};
use crate::normal::begin_operator;
use crate::operator::OperatorKind;
use crate::textobject::{self, ObjectKind};

pub struct Action {
    pub id: &'static str,
    pub title: &'static str,
    pub keys: &'static str,
    pub handler: fn(&mut Editor),
}

pub enum Lookup {
    Ran,
    Prefix,
    NoMatch,
}

/// Dispatches a completed or partial leader sequence (without the leader
/// key itself, e.g. `"rc"` for `,rc`). Exact match wins even if it is also
/// a prefix of a longer binding, matching Vim's own tie-break.
pub fn dispatch(ed: &mut Editor, seq: &str) -> Lookup {
    if let Some(a) = ACTIONS.iter().find(|a| a.keys == seq) {
        // Started from the file tree: anything but a tree action runs
        // against the editing pane, not the tree's placeholder buffer.
        if ed.active_file_tree() && !a.id.starts_with("explorer.") {
            ed.focus_tree_edit_window();
        }
        (a.handler)(ed);
        return Lookup::Ran;
    }
    if ACTIONS.iter().any(|a| a.keys.starts_with(seq)) {
        Lookup::Prefix
    } else {
        Lookup::NoMatch
    }
}

pub fn find(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// Actions whose default key sequence continues the given prefix, sorted
/// for stable which-key display.
pub fn matching(prefix: &str) -> Vec<&'static Action> {
    let mut v: Vec<&'static Action> = ACTIONS
        .iter()
        .filter(|a| a.keys.starts_with(prefix) && a.keys != prefix)
        .collect();
    v.sort_by_key(|a| a.keys);
    v
}

fn save_notes(ed: &mut Editor) {
    let result = ed.save_notes();
    ed.set_message(match result {
        Ok(()) => "Comments saved".into(),
        Err(e) => e.to_string(),
    });
}

fn write_current(ed: &mut Editor) {
    match ed.save_current_formatted() {
        Ok(()) => ed.set_message("written"),
        Err(e) => ed.set_message(format!("save failed: {}", e)),
    }
}

fn delete_blackhole(ed: &mut Editor) {
    ed.pending.register = Some('_');
    begin_operator(ed, OperatorKind::Delete);
}

fn toggle_wrap(ed: &mut Editor) {
    ed.config.wrap = !ed.config.wrap;
    ed.set_message(format!("wrap: {}", ed.config.wrap));
}

fn toggle_relnum(ed: &mut Editor) {
    ed.config.relativenumber = !ed.config.relativenumber;
    ed.set_message(format!("relativenumber: {}", ed.config.relativenumber));
}

fn reload_config(ed: &mut Editor) {
    ed.config = crate::config::Config::load();
    ed.restart_lsp();
}

fn recent_files(ed: &mut Editor) {
    let entries = ed
        .recent_files
        .iter()
        .map(|p| {
            crate::results::Entry::location(
                p.clone(),
                0,
                0,
                p.file_name().unwrap_or_default().to_string_lossy(),
            )
        })
        .collect();
    ed.show_results(crate::results::Results::new("Recent files", entries));
}

fn word_under_cursor(ed: &Editor) -> Option<String> {
    let (line, col) = ed.cursor();
    let (sl, sc, el, ec) = textobject::resolve(ed.buf(), line, col, ObjectKind::Word(false), true)?;
    if sl != el {
        return None;
    }
    let chars: Vec<char> = ed.buf().line_text(sl).chars().collect();
    Some(chars.get(sc..=ec)?.iter().collect())
}

/// `,gw`: live grep for the word under the cursor in Normal mode, or the
/// selected text in Visual (Char/Line only -- Visual-block selects a
/// column, not a contiguous string, so it falls back to the word under
/// the cursor instead of guessing which row's text to use).
fn grep_word_or_selection(ed: &mut Editor) {
    match word_or_selection(ed) {
        Some(q) => ed.open_grep(&q),
        None => ed.set_message("Nothing to grep"),
    }
}

/// `,sw`: the replace screen with the word under the cursor / the Visual
/// selection as a literal search, focused on the replacement field.
fn replace_word_or_selection(ed: &mut Editor) {
    match word_or_selection(ed) {
        Some(q) => ed.open_far(Some(q)),
        None => ed.set_message("Nothing to replace"),
    }
}

/// The Visual (Char/Line) selection, or else the word under the cursor,
/// trimmed -- leaving Visual mode. `None` when that's empty.
fn word_or_selection(ed: &mut Editor) -> Option<String> {
    let query = match ed.mode {
        Mode::Visual(kind @ (VisualKind::Char | VisualKind::Line)) => {
            ed.visual_anchor.map(|anchor| {
                let cursor = ed.cursor();
                let span = if kind == VisualKind::Line {
                    crate::motion::Span::Linewise
                } else {
                    crate::motion::Span::Inclusive
                };
                let (start, end, _) = crate::normal::span_to_range(ed, anchor, cursor, span);
                ed.buf().text_range(start, end)
            })
        }
        _ => word_under_cursor(ed),
    };
    if matches!(ed.mode, Mode::Visual(_)) {
        ed.visual_anchor = None;
        ed.enter_normal();
    }
    query
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty())
}

/// `,lf`: formats the Visual selection's line range if one is active
/// (via `textDocument/rangeFormatting`), otherwise the whole buffer --
/// same "operate on the selection if there is one" convention `,gw`
/// already uses for grep.
fn format_buffer_or_selection(ed: &mut Editor) {
    if matches!(ed.mode, Mode::Visual(_)) {
        let anchor = ed.visual_anchor;
        let cursor = ed.cursor();
        ed.visual_anchor = None;
        ed.enter_normal();
        if let Some(anchor) = anchor {
            let (l1, l2) = (anchor.0.min(cursor.0), anchor.0.max(cursor.0));
            ed.request_range_format(l1, l2);
            return;
        }
    }
    ed.request_language("format", None);
}

/// `,gp`: GitHub permalink for the Visual selection's line range if one
/// is active, otherwise just the cursor's line -- same convention as
/// `,gw`/`,lf`. Always pinned to HEAD; a specific commit (e.g. from a
/// `:gitblame` entry) goes through `Editor::permalink_from_results_entry`
/// instead, since that's a Results-list action, not a buffer one.
fn permalink_for_cursor_or_selection(ed: &mut Editor) {
    let Some(path) = ed.buf().path.clone() else {
        ed.set_message("This buffer has no file on disk");
        return;
    };
    let (start, end) = if matches!(ed.mode, Mode::Visual(_)) {
        let anchor = ed.visual_anchor;
        let cursor = ed.cursor();
        ed.visual_anchor = None;
        ed.enter_normal();
        (anchor.map(|a| a.0).unwrap_or(cursor.0), cursor.0)
    } else {
        let line = ed.cursor().0;
        (line, line)
    };
    ed.generate_permalink(&path, start, end, None);
}

/// A Visual selection's line range, `.min()`/`.max()`'d against the
/// cursor -- shared by the hunk stage/reset actions so a selection drawn
/// upward (cursor above the anchor) still yields `start <= end`. Clears
/// the selection and drops back to Normal mode as a side effect, same
/// as `,gp`/`,lf`'s own selection-consuming actions.
fn selection_line_range(ed: &mut Editor) -> (usize, usize) {
    let anchor = ed.visual_anchor;
    let cursor = ed.cursor();
    ed.visual_anchor = None;
    ed.enter_normal();
    let a = anchor.map(|a| a.0).unwrap_or(cursor.0);
    (a.min(cursor.0), a.max(cursor.0))
}

/// `,gx`: resets the saved hunk under the cursor to HEAD, or -- with a
/// Visual selection active -- only the selected lines within whichever
/// hunk(s) it overlaps, the same "selection scopes it, otherwise the
/// cursor's single item does" convention as `,gp`/`,lf`/`,gw`.
fn hunk_reset_for_cursor_or_selection(ed: &mut Editor) {
    if matches!(ed.mode, Mode::Visual(_)) {
        let (start, end) = selection_line_range(ed);
        ed.reset_range_prompt(start, end);
        return;
    }
    ed.reset_current_hunk_prompt();
}

/// `,gs`: stages the saved hunk under the cursor, or -- with a Visual
/// selection active -- only the selected lines, mirroring `,gx`'s own
/// cursor-vs-selection split.
fn hunk_stage_for_cursor_or_selection(ed: &mut Editor) {
    if matches!(ed.mode, Mode::Visual(_)) {
        let (start, end) = selection_line_range(ed);
        ed.stage_range(start, end);
        return;
    }
    ed.stage_current_hunk();
}

fn select_all(ed: &mut Editor) {
    let last = ed.buf().line_count().saturating_sub(1);
    ed.visual_anchor = Some((0, 0));
    ed.set_cursor(last, 0);
    ed.mode = Mode::Visual(VisualKind::Line);
}

fn toggle_zen(ed: &mut Editor) {
    ed.config.number = !ed.config.number;
    ed.set_message(if ed.config.number {
        "Zen off"
    } else {
        "Zen on — line numbers hidden"
    });
}

fn rename_prompt(ed: &mut Editor) {
    ed.enter_command(CommandKind::Ex);
    ed.set_cmdline("rename ");
}

fn workspace_symbols_prompt(ed: &mut Editor) {
    ed.enter_command(CommandKind::Ex);
    ed.set_cmdline("workspacesymbols ");
}

fn diagnostics(ed: &mut Editor) {
    let r = ed.diagnostic_results();
    ed.show_results(r);
}

pub static ACTIONS: &[Action] = &[
    Action {
        id: "review.note",
        title: "New line/range review note",
        keys: "rc",
        handler: |ed| ed.new_note(false),
    },
    Action {
        id: "review.note_file",
        title: "New file review note",
        keys: "rf",
        handler: |ed| ed.new_note(true),
    },
    Action {
        id: "review.list",
        title: "List review notes",
        keys: "rl",
        handler: |ed| ed.comments_results(),
    },
    Action {
        id: "review.save",
        title: "Save review notes",
        keys: "rw",
        handler: save_notes,
    },
    Action {
        id: "quickfix.open",
        title: "Open quickfix",
        keys: "cq",
        handler: |ed| ed.open_quickfix(),
    },
    Action {
        id: "lsp.diagnostics",
        title: "Diagnostics list",
        keys: "ld",
        handler: diagnostics,
    },
    Action {
        id: "lsp.format",
        title: "Format buffer (or Visual selection)",
        keys: "lf",
        handler: format_buffer_or_selection,
    },
    Action {
        id: "lsp.document_highlight",
        title: "Highlight other occurrences of the symbol under the cursor",
        keys: "lh",
        handler: |ed| ed.request_language("documentHighlight", None),
    },
    Action {
        id: "lsp.document_links",
        title: "Document links; Enter opens or copies one",
        keys: "ll",
        handler: |ed| ed.request_language("documentLinks", None),
    },
    Action {
        id: "lsp.document_color",
        title: "Colorize color literals in the buffer (documentColor)",
        keys: "lC",
        handler: |ed| ed.request_language("documentColor", None),
    },
    Action {
        id: "lsp.code_lens",
        title: "Code lenses; Enter runs one (also shown as virtual text)",
        keys: "lc",
        handler: |ed| ed.request_language("codeLens", None),
    },
    Action {
        id: "lsp.inlay_hints",
        title: "Show inlay hints inline; Esc clears them",
        keys: "li",
        handler: |ed| ed.request_language("inlayHints", None),
    },
    Action {
        id: "lsp.rename",
        title: "Rename symbol",
        keys: "lr",
        handler: rename_prompt,
    },
    Action {
        id: "lsp.actions",
        title: "Code actions",
        keys: "la",
        handler: |ed| ed.request_language("actions", None),
    },
    Action {
        id: "lsp.organize_imports",
        title: "Organize imports (applies directly, no picker)",
        keys: "lI",
        handler: |ed| ed.request_language("organizeImports", None),
    },
    Action {
        id: "lsp.outline",
        title: "Document outline sidebar",
        keys: "lo",
        handler: |ed| ed.toggle_outline(),
    },
    Action {
        id: "lsp.outline_sidebar",
        title: "Toggle outline sidebar",
        keys: "lO",
        handler: |ed| ed.toggle_outline(),
    },
    Action {
        id: "lsp.references",
        title: "References",
        keys: "lR",
        handler: |ed| ed.request_language("references", None),
    },
    Action {
        id: "lsp.signature",
        title: "Signature help",
        keys: "ls",
        handler: |ed| ed.request_language("signature", None),
    },
    Action {
        id: "lsp.workspace_symbols",
        title: "Workspace symbols",
        keys: "lw",
        handler: workspace_symbols_prompt,
    },
    Action {
        id: "peek.definition",
        title: "Peek definition in a floating window",
        keys: "pd",
        handler: |ed| ed.request_language("peekDefinition", None),
    },
    Action {
        id: "peek.type_definition",
        title: "Peek type definition in a floating window",
        keys: "pt",
        handler: |ed| ed.request_language("peekTypeDefinition", None),
    },
    Action {
        id: "peek.implementation",
        title: "Peek implementation in a floating window",
        keys: "pi",
        handler: |ed| ed.request_language("peekImplementation", None),
    },
    Action {
        id: "peek.references",
        title: "Peek references (list + preview) in a floating window",
        keys: "pr",
        handler: |ed| ed.request_language("peekReferences", None),
    },
    Action {
        id: "peek.hover",
        title: "Hover documentation in a floating window",
        keys: "pk",
        handler: |ed| ed.request_language("peekHover", None),
    },
    Action {
        id: "peek.focus",
        title: "Focus the open floating window",
        keys: "pf",
        handler: |ed| ed.focus_float(),
    },
    Action {
        id: "peek.close",
        title: "Close the floating window",
        keys: "pc",
        handler: |ed| ed.close_float(),
    },
    Action {
        id: "window.vsplit_preview",
        title: "Vertical split + Markdown preview",
        keys: "ms",
        handler: |ed| ed.split_window(true, true),
    },
    Action {
        id: "window.vsplit",
        title: "Open a vertical split",
        keys: "kv",
        handler: |ed| ed.split_window(true, false),
    },
    Action {
        id: "window.split",
        title: "Open a horizontal split",
        keys: "ks",
        handler: |ed| ed.split_window(false, false),
    },
    Action {
        id: "file.write",
        title: "Write current buffer",
        keys: "w",
        handler: write_current,
    },
    Action {
        id: "file.quit",
        title: "Close buffer (quit if it is the last one)",
        keys: "q",
        handler: crate::command::close_current_buffer_keep_window,
    },
    Action {
        id: "file.quit_force",
        title: "Quit without saving",
        keys: "Q",
        handler: |ed| ed.should_quit = true,
    },
    Action {
        id: "search.clear_highlight",
        title: "Clear search highlight",
        keys: "h",
        handler: |ed| ed.hl_search = false,
    },
    Action {
        id: "edit.delete_blackhole",
        title: "Delete to black-hole register",
        keys: "d",
        handler: delete_blackhole,
    },
    Action {
        id: "option.toggle_wrap",
        title: "Toggle line wrap",
        keys: "ow",
        handler: toggle_wrap,
    },
    Action {
        id: "option.toggle_relativenumber",
        title: "Toggle relative line numbers",
        keys: "or",
        handler: toggle_relnum,
    },
    Action {
        id: "option.cursorline_info",
        title: "Show cursorline hint",
        keys: "ol",
        handler: |ed| ed.set_message("Cursor line is indicated by the highlighted line number"),
    },
    Action {
        id: "config.reload",
        title: "Reload config + restart LSP",
        keys: "R",
        handler: reload_config,
    },
    Action {
        id: "explorer.toggle",
        title: "Toggle file tree sidebar",
        keys: "ft",
        handler: |ed| ed.toggle_file_tree(),
    },
    Action {
        id: "explorer.focus",
        title: "File tree: open / focus / close",
        keys: "e",
        handler: |ed| ed.focus_or_toggle_file_tree(),
    },
    Action {
        id: "explorer.find",
        title: "Reveal current file in the file tree",
        keys: "fe",
        handler: |ed| ed.tree_find_current(),
    },
    Action {
        id: "file.recent",
        title: "Recent files",
        keys: "fr",
        handler: recent_files,
    },
    Action {
        id: "file.picker_alt",
        title: "Project file picker",
        keys: "ff",
        handler: |ed| ed.open_picker(),
    },
    Action {
        id: "cursors.add_all",
        title: "Multiple cursors: every occurrence of the word",
        keys: "ma",
        handler: crate::multicursor::add_all,
    },
    Action {
        id: "cursors.add_below",
        title: "Multiple cursors: add a cursor below",
        keys: "mj",
        handler: |ed| crate::multicursor::add_vertical(ed, true),
    },
    Action {
        id: "cursors.add_above",
        title: "Multiple cursors: add a cursor above",
        keys: "mk",
        handler: |ed| crate::multicursor::add_vertical(ed, false),
    },
    Action {
        id: "cursors.clear",
        title: "Multiple cursors: collapse to one",
        keys: "mc",
        handler: |ed| {
            crate::multicursor::clear(ed);
            ed.set_message("multiple cursors cleared");
        },
    },
    Action {
        id: "markdown.preview_toggle",
        title: "Toggle Markdown preview",
        keys: "mp",
        handler: |ed| ed.toggle_markdown_preview(),
    },
    Action {
        id: "buffer.list",
        title: "Buffer list",
        keys: "b",
        handler: |ed| ed.show_buffers(),
    },
    Action {
        id: "search.grep",
        title: "Live grep",
        keys: "/",
        handler: |ed| ed.open_grep(""),
    },
    Action {
        id: "search.replace",
        title: "Search & replace across the project (reviewed)",
        keys: "sr",
        handler: |ed| ed.open_far(None),
    },
    Action {
        id: "search.replace_word",
        title: "Replace word under cursor / selection across the project",
        keys: "sw",
        handler: replace_word_or_selection,
    },
    Action {
        id: "git.permalink",
        title: "Copy GitHub permalink (cursor line / Visual selection)",
        keys: "gp",
        handler: permalink_for_cursor_or_selection,
    },
    Action {
        id: "git.hunk_preview",
        title: "Preview the saved hunk under the cursor",
        keys: "gh",
        handler: |ed| ed.preview_current_hunk(),
    },
    Action {
        id: "git.blame_toggle",
        title: "Toggle line-blame virtual text",
        keys: "gB",
        handler: |ed| ed.toggle_line_blame(),
    },
    Action {
        id: "git.diff_overlay_toggle",
        title: "Toggle the diff overlay (deleted lines, changed-word highlight)",
        keys: "gd",
        handler: |ed| ed.toggle_diff_overlay(),
    },
    Action {
        id: "git.status",
        title: "Open the Git workspace (staged/unstaged/untracked/conflicts)",
        keys: "gS",
        handler: |ed| ed.open_git_status(),
    },
    Action {
        id: "git.lazygit",
        title: "Open lazygit in an embedded terminal",
        keys: "gl",
        handler: |ed| ed.open_lazygit(),
    },
    Action {
        id: "github.prs",
        title: "GitHub: pull requests",
        keys: "Gp",
        handler: |ed| ed.gh_pr_list(""),
    },
    Action {
        id: "github.pr",
        title: "GitHub: current branch's PR overview",
        keys: "Gv",
        handler: |ed| ed.gh_pr_view(None),
    },
    Action {
        id: "github.diff",
        title: "GitHub: current branch's PR diff",
        keys: "Gd",
        handler: |ed| ed.gh_pr_diff(None),
    },
    Action {
        id: "github.threads",
        title: "GitHub: current branch's PR review threads",
        keys: "Gr",
        handler: |ed| ed.gh_pr_threads(None),
    },
    Action {
        id: "github.checks",
        title: "GitHub: current branch's PR CI checks",
        keys: "Gc",
        handler: |ed| ed.gh_pr_checks(None),
    },
    Action {
        id: "github.issues",
        title: "GitHub: issues",
        keys: "Gi",
        handler: |ed| ed.gh_issue_list(""),
    },
    Action {
        id: "git.diff_ignore_whitespace_toggle",
        title: "Toggle ignoring whitespace in :gitdiff's view",
        keys: "gW",
        handler: |ed| ed.toggle_diff_ignore_whitespace(),
    },
    Action {
        id: "git.hunk_reset",
        title: "Reset the saved hunk under the cursor to HEAD (or the Visual selection's lines)",
        keys: "gx",
        handler: hunk_reset_for_cursor_or_selection,
    },
    Action {
        id: "git.hunk_stage",
        title: "Stage the saved hunk under the cursor (or the Visual selection's lines)",
        keys: "gs",
        handler: hunk_stage_for_cursor_or_selection,
    },
    Action {
        id: "search.grep_word",
        title: "Live grep word under cursor / selection",
        keys: "gw",
        handler: grep_word_or_selection,
    },
    Action {
        id: "search.find_word",
        title: "Find word under cursor / selection (live grep)",
        keys: "fw",
        handler: grep_word_or_selection,
    },
    Action {
        id: "search.repeat_last",
        title: "Resume the last buffer search",
        keys: "f.",
        handler: |ed| crate::normal::repeat_last_search(ed),
    },
    Action {
        id: "search.grep_current_buffer",
        title: "Live grep scoped to the current buffer",
        keys: "fb",
        handler: |ed| ed.open_grep_current_buffer(""),
    },
    Action {
        id: "search.grep_open_buffers",
        title: "Live grep scoped to every open buffer",
        keys: "fB",
        handler: |ed| ed.open_grep_open_buffers(""),
    },
    Action {
        id: "edit.select_all",
        title: "Select entire buffer",
        keys: "a",
        handler: select_all,
    },
    Action {
        id: "ui.zen_toggle",
        title: "Toggle zen (line numbers)",
        keys: "z",
        handler: toggle_zen,
    },
    Action {
        id: "agent.context_send",
        title: "Send context to an agent (file/selection/clipboard/symbol/diagnostics)",
        keys: "cx",
        handler: |ed| ed.open_context_picker(),
    },
    Action {
        id: "ai.prompt",
        title: "AI prompt to the Claude sidebar (code/selection + cursor + diagnostics)",
        keys: "ca",
        handler: |ed| ed.open_ai_prompt_picker(),
    },
    Action {
        id: "tasks.view",
        title: "Tasks: open today's editable task log (:taskview)",
        keys: "td",
        handler: |ed| ed.task_today(),
    },
    Action {
        id: "tabs.new",
        title: "Tabs: open a new tab (:tabnew)",
        keys: "tt",
        handler: |ed| ed.new_tab(),
    },
    Action {
        id: "tabs.next",
        title: "Tabs: move to the next tab (:tabnext / gt)",
        keys: "t]",
        handler: |ed| ed.next_tab(),
    },
    Action {
        id: "tabs.previous",
        title: "Tabs: move to the previous tab (:tabprevious / gT)",
        keys: "t[",
        handler: |ed| ed.prev_tab(),
    },
    Action {
        id: "tabs.close",
        title: "Tabs: close the current tab (:tabclose)",
        keys: "tq",
        handler: |ed| ed.close_tab(),
    },
    Action {
        id: "tasks.list",
        title: "Tasks: browse all saved entries, including future dates (:tasklist)",
        keys: "tL",
        handler: |ed| ed.task_list(),
    },
    Action {
        id: "tasks.add",
        title: "Tasks: write a new task in a buffer; :wq saves it (:taskadd)",
        keys: "tl",
        handler: |ed| ed.task_draft_open(chrono::Local::now().date_naive(), None),
    },
    Action {
        id: "tasks.add_alias",
        title: "Tasks: write a new task in a buffer; :wq saves it (:taskadd)",
        keys: "ta",
        handler: |ed| ed.task_draft_open(chrono::Local::now().date_naive(), None),
    },
    Action {
        id: "tasks.note",
        title: "Tasks: log an activity note (:tasknote)",
        keys: "tN",
        handler: |ed| {
            ed.enter_command(crate::mode::CommandKind::Ex);
            ed.set_cmdline("tasknote ");
        },
    },
    Action {
        id: "tasks.close",
        title: "Tasks: mark the task under the cursor done (:taskdone)",
        keys: "tD",
        handler: |ed| ed.task_done(),
    },
    Action {
        id: "tasks.yesterday",
        title: "Tasks: open yesterday's task log (:ystd)",
        keys: "ty",
        handler: |ed| ed.task_yesterday(),
    },
    Action {
        id: "tasks.week",
        title: "Tasks: browse this week's entries (:week)",
        keys: "tw",
        handler: |ed| ed.task_week(),
    },
    Action {
        id: "tasks.on_date",
        title: "Tasks: open a date (:on YYYY-MM-DD)",
        keys: "tO",
        handler: |ed| {
            ed.enter_command(crate::mode::CommandKind::Ex);
            ed.set_cmdline("on ");
        },
    },
    Action {
        id: "tour.start",
        title: "Code tours: pick one to start (:tours)",
        keys: "ts",
        handler: |ed| ed.list_tours(),
    },
    Action {
        id: "tour.next",
        title: "Code tour: next step (:tournext)",
        keys: "tn",
        handler: |ed| ed.tour_step(true),
    },
    Action {
        id: "tour.prev",
        title: "Code tour: previous step (:tourprev)",
        keys: "tp",
        handler: |ed| ed.tour_step(false),
    },
    Action {
        id: "tour.end",
        title: "Code tour: end (:tourend)",
        keys: "te",
        handler: |ed| ed.tour_end(),
    },
    Action {
        id: "tour.steps",
        title: "Code tour: jump to a step (:toursteps)",
        keys: "to",
        handler: |ed| ed.list_tour_steps(),
    },
    Action {
        id: "tour.explain",
        title: "Code tour: explain this step with Claude (:tourexplain)",
        keys: "tx",
        handler: |ed| ed.tour_explain(),
    },
    Action {
        id: "tour.new",
        title: "Code tour: describe a new one for Claude to generate (:tournew)",
        keys: "tc",
        handler: |ed| ed.tour_new(""),
    },
    Action {
        id: "test.nearest",
        title: "Test: run the test under the cursor (:testnearest)",
        keys: "Tn",
        handler: |ed| ed.test_run(crate::testrun::Scope::Nearest),
    },
    Action {
        id: "test.file",
        title: "Test: run this file's tests (:testfile)",
        keys: "Tf",
        handler: |ed| ed.test_run(crate::testrun::Scope::File),
    },
    Action {
        id: "test.suite",
        title: "Test: run the whole suite (:testsuite)",
        keys: "Ts",
        handler: |ed| ed.test_run(crate::testrun::Scope::Suite),
    },
    Action {
        id: "test.last",
        title: "Test: re-run the last test command (:testlast)",
        keys: "Tl",
        handler: |ed| ed.test_last(),
    },
    Action {
        id: "test.output",
        title: "Test: show the last run's output (:testoutput)",
        keys: "To",
        handler: |ed| ed.test_output(),
    },
    Action {
        id: "test.stop",
        title: "Test: stop the running tests (:teststop)",
        keys: "Tx",
        handler: |ed| ed.test_stop(),
    },
    Action {
        id: "test.clear",
        title: "Test: clear the pass/fail gutter marks (:testclear)",
        keys: "Tc",
        handler: |ed| ed.test_clear(),
    },
    Action {
        id: "select.expand",
        title: "Expand selection to the enclosing syntax node",
        keys: "=",
        handler: |ed| ed.expand_selection(),
    },
    Action {
        id: "select.shrink",
        title: "Shrink incremental selection",
        keys: "-",
        handler: |ed| ed.shrink_selection(),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_default_keys() {
        let mut seen = HashSet::new();
        for a in ACTIONS {
            assert!(
                seen.insert(a.keys),
                "duplicate leader binding ,{} (action {})",
                a.keys,
                a.id
            );
        }
    }

    #[test]
    fn no_duplicate_ids() {
        let mut seen = HashSet::new();
        for a in ACTIONS {
            assert!(seen.insert(a.id), "duplicate action id {}", a.id);
        }
    }

    #[test]
    fn matching_excludes_exact_and_unrelated() {
        let m = matching("f");
        let keys: Vec<_> = m.iter().map(|a| a.keys).collect();
        assert!(keys.contains(&"fr"));
        assert!(keys.contains(&"ff"));
        assert!(!keys.contains(&"w"));
    }

    #[test]
    fn dispatch_exact_and_prefix_and_nomatch() {
        let mut ed = Editor::new(crate::config::Config::default());
        assert!(matches!(dispatch(&mut ed, "h"), Lookup::Ran));
        assert!(matches!(dispatch(&mut ed, "r"), Lookup::Prefix));
        assert!(matches!(dispatch(&mut ed, "xyz"), Lookup::NoMatch));
    }
}
