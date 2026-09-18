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
    match ed.save_current() {
        Ok(()) => ed.set_message("written"),
        Err(e) => ed.set_message(format!("save failed: {}", e)),
    }
}

fn quit_checked(ed: &mut Editor) {
    let any_modified = ed.buffers.iter().any(|b| b.is_modified());
    if any_modified {
        ed.set_message("unsaved changes -- ,Q to discard, ,w to save");
    } else {
        ed.should_quit = true;
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
    match query.map(|q| q.trim().to_string()) {
        Some(q) if !q.is_empty() => ed.open_grep(&q),
        _ => ed.set_message("Nothing to grep"),
    }
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
    ed.cmdline = "rename ".into();
}

fn workspace_symbols_prompt(ed: &mut Editor) {
    ed.enter_command(CommandKind::Ex);
    ed.cmdline = "workspacesymbols ".into();
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
        title: "Document outline",
        keys: "lo",
        handler: |ed| ed.request_language("outline", None),
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
        id: "window.vsplit_preview",
        title: "Vertical split + Markdown preview",
        keys: "ms",
        handler: |ed| ed.split_window(true, true),
    },
    Action {
        id: "file.write",
        title: "Write current buffer",
        keys: "w",
        handler: write_current,
    },
    Action {
        id: "file.quit",
        title: "Quit (checked)",
        keys: "q",
        handler: quit_checked,
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
        id: "file.picker",
        title: "Project file picker",
        keys: "e",
        handler: |ed| ed.open_picker(),
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
        id: "git.hunk_reset",
        title: "Reset the saved hunk under the cursor to HEAD",
        keys: "gx",
        handler: |ed| ed.reset_current_hunk_prompt(),
    },
    Action {
        id: "search.grep_word",
        title: "Live grep word under cursor / selection",
        keys: "gw",
        handler: grep_word_or_selection,
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
