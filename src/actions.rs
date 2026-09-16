//! Central action registry: every leader-key command has one id, title,
//! default key sequence and handler here. `run_leader` in `normal.rs`, the
//! `:keymaps` command picker and the which-key prefix popup are all
//! generated from this single table instead of keeping separate lists that
//! can drift out of sync.
use crate::editor::Editor;
use crate::mode::CommandKind;
use crate::normal::begin_operator;
use crate::operator::OperatorKind;

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
        title: "Format buffer",
        keys: "lf",
        handler: |ed| ed.request_language("format", None),
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
        id: "lsp.outline",
        title: "Document outline",
        keys: "lo",
        handler: |ed| ed.request_language("outline", None),
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
