//! Structured context for talking to an agent (Phase 6 item 2): the
//! current file, a Visual selection, the clipboard, an enclosing
//! symbol's body/signature, or the current buffer's diagnostics -- each
//! rendered as labeled, fenced text so an agent session receives
//! unambiguous, self-describing input instead of bare pasted text.
//! `,cx` opens a picker over these kinds; Enter copies the built text
//! to the `+` register and, if an agent session (`:claude`/`:codex`/
//! `:agent`) is currently attached to a window in this tab, also types
//! it directly into that session's input -- no separate "copy" vs
//! "send" keybinding needed.
use crate::{
    editor::Editor,
    results::{Entry, Results},
};

struct ContextKind {
    label: &'static str,
    tag: &'static str,
}
const KINDS: &[ContextKind] = &[
    ContextKind {
        label: "Current file",
        tag: "file",
    },
    ContextKind {
        label: "Visual selection",
        tag: "selection",
    },
    ContextKind {
        label: "Clipboard (+ register)",
        tag: "clipboard",
    },
    ContextKind {
        label: "Enclosing symbol body",
        tag: "symbol_body",
    },
    ContextKind {
        label: "Enclosing symbol signature",
        tag: "symbol_signature",
    },
    ContextKind {
        label: "Buffer diagnostics",
        tag: "diagnostics",
    },
];

/// Built-in AI prompt templates (label shown in the picker → instruction sent
/// ahead of the code/diagnostics context) for the `:ai` / `,ai` Claude sidebar.
const PROMPTS: &[(&str, &str)] = &[
    ("Explain", "Explain what the following code does."),
    ("Fix bugs", "Find and fix any bugs in the following code."),
    (
        "Fix diagnostics",
        "Fix the diagnostics reported for the following code.",
    ),
    ("Write tests", "Write tests for the following code."),
    (
        "Review",
        "Review the following code for bugs, edge cases, and improvements.",
    ),
    ("Add docs", "Add documentation comments to the following code."),
    (
        "Optimize",
        "Improve the performance and clarity of the following code.",
    ),
];

impl Editor {
    /// `,cx`: opens the context-kind picker. A Visual selection's line
    /// range is captured up front (the same capture-then-leave-the-mode
    /// order `,gp`/`,lf` already use) so "Visual selection" can still
    /// build from it even though picking an entry means leaving Visual
    /// mode first; the kind itself is only offered when one was active.
    pub fn open_context_picker(&mut self) {
        let selection = if matches!(self.mode, crate::mode::Mode::Visual(_)) {
            let anchor = self.visual_anchor;
            let cursor = self.cursor();
            self.visual_anchor = None;
            let a = anchor.map(|a| a.0).unwrap_or(cursor.0);
            Some((a.min(cursor.0), a.max(cursor.0)))
        } else {
            None
        };
        self.enter_normal();
        let entries: Vec<Entry> = KINDS
            .iter()
            .filter(|k| k.tag != "selection" || selection.is_some())
            .map(|k| {
                let mut action = serde_json::json!({"tag": k.tag});
                if let Some((s, z)) = selection {
                    action["selection"] = serde_json::json!([s, z]);
                }
                let mut e = Entry::text(k.label);
                e.action = Some(serde_json::json!({"_vaayu_context_send": action}));
                e
            })
            .collect();
        self.show_results(Results::new(
            "Send context — Enter copies it (and sends it if an agent session is attached)",
            entries,
        ));
    }

    /// Builds the labeled, fenced text for `tag`. Never writes a
    /// register or a terminal (independently testable from the actual
    /// copy/send step in `send_context`); `&mut self` only because
    /// reading the clipboard kind refreshes from the system clipboard
    /// first, the same as any other register read already does.
    fn build_context(
        &mut self,
        tag: &str,
        selection: Option<(usize, usize)>,
    ) -> Result<String, String> {
        let path = self.buf().path.clone();
        let rel = path
            .as_ref()
            .map(|p| {
                p.strip_prefix(&self.project_root)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| "(unsaved buffer)".into());
        match tag {
            "file" => Ok(format!("File: {rel}\n```\n{}\n```", self.buf().rope)),
            "selection" => {
                let (s, z) = selection.ok_or("No Visual selection was active")?;
                let mut text = String::new();
                for l in s..=z {
                    text.push_str(&self.buf().line_text(l));
                    text.push('\n');
                }
                Ok(format!("File: {rel}:{}-{}\n```\n{text}```", s + 1, z + 1))
            }
            "clipboard" => {
                let text = self
                    .registers
                    .get(Some('+'))
                    .map(|r| r.text.clone())
                    .unwrap_or_default();
                if text.is_empty() {
                    return Err("Clipboard (+ register) is empty".into());
                }
                Ok(format!("Clipboard:\n```\n{text}\n```"))
            }
            "symbol_body" | "symbol_signature" => {
                let outline = self
                    .outline
                    .as_ref()
                    .ok_or("Open the outline first (,lO or :outline) so a symbol is available")?;
                if outline.buffer_path != path {
                    return Err(
                        "The outline is showing a different file -- open :outline again for this buffer"
                            .into(),
                    );
                }
                let line = self.cursor().0;
                let sym = outline
                    .all_nodes
                    .iter()
                    .filter(|n| n.line <= line && line <= n.end_line)
                    .max_by_key(|n| n.depth)
                    .ok_or("No symbol encloses the cursor")?;
                if tag == "symbol_signature" {
                    Ok(format!(
                        "Symbol: {} ({}) at {rel}:{}\n```\n{}\n```",
                        sym.name,
                        sym.kind,
                        sym.line + 1,
                        self.buf().line_text(sym.line)
                    ))
                } else {
                    let mut text = String::new();
                    for l in sym.line..=sym.end_line {
                        text.push_str(&self.buf().line_text(l));
                        text.push('\n');
                    }
                    Ok(format!(
                        "Symbol: {} ({}) at {rel}:{}-{}\n```\n{text}```",
                        sym.name,
                        sym.kind,
                        sym.line + 1,
                        sym.end_line + 1
                    ))
                }
            }
            "diagnostics" => {
                let path = path.ok_or("This buffer has no file on disk")?;
                let ds = self
                    .diagnostics
                    .get(&path)
                    .filter(|d| !d.is_empty())
                    .ok_or("No diagnostics for this file")?;
                let mut text = format!("Diagnostics: {rel}\n");
                for d in ds {
                    text.push_str(&format!(
                        "{}:{}: {:?}: {}\n",
                        d.line + 1,
                        d.col + 1,
                        d.severity,
                        d.message
                    ));
                }
                Ok(text)
            }
            _ => Err(format!("Unknown context kind: {tag}")),
        }
    }

    /// The agent-session terminal attached to some window in this tab,
    /// if any -- not necessarily the *active* one: the realistic
    /// workflow is picking a context kind while focused on a source
    /// file in one pane, to send into an agent session sitting in
    /// another pane, not while focused on the terminal itself (leader
    /// keys don't even reach Terminal mode, which forwards every key as
    /// raw input instead).
    fn attached_agent_terminal(&mut self) -> Option<&mut crate::pty::PtySession> {
        let id = self.windows.iter().find_map(|w| {
            let id = w.terminal?;
            self.terminals
                .iter()
                .any(|p| p.id == id && p.agent_kind.is_some())
                .then_some(id)
        })?;
        self.terminals.iter_mut().find(|p| p.id == id)
    }

    /// Dispatches a `_vaayu_context_send`-tagged entry: builds the
    /// context, copies it to the clipboard/`+` register, and -- if an
    /// agent session is attached somewhere in this tab -- also types it
    /// directly into that session's input (not auto-submitted: the
    /// same "paste, then the human decides when to send" shape as
    /// pasting into any chat by hand).
    pub fn send_context(&mut self, value: &serde_json::Value) {
        let Some(tag) = value["tag"].as_str().map(str::to_string) else {
            return;
        };
        let selection = value
            .get("selection")
            .and_then(|v| v.as_array())
            .and_then(|a| {
                let s = a.first()?.as_u64()? as usize;
                let z = a.get(1)?.as_u64()? as usize;
                Some((s, z))
            });
        let text = match self.build_context(&tag, selection) {
            Ok(t) => t,
            Err(e) => {
                self.set_message(e);
                return;
            }
        };
        self.registers.set(Some('+'), text.clone(), false);
        if let Some(pty) = self.attached_agent_terminal() {
            let kind = pty.agent_kind.clone().unwrap_or_default();
            pty.write_pasted_input(&text);
            self.set_message(format!("Copied to + register and sent to {kind}"));
        } else {
            self.set_message("Copied to + register");
        }
    }

    /// The current buffer's path relative to the project root (or a placeholder
    /// for an unsaved buffer) -- the same rendering `build_context` uses.
    fn context_rel_path(&self) -> String {
        self.buf()
            .path
            .as_ref()
            .map(|p| {
                p.strip_prefix(&self.project_root)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| "(unsaved buffer)".into())
    }

    /// `,ai` / `:ai`: opens a picker of prompt templates. A Visual selection's
    /// line range is captured up front (like `,cx`) so a template can build
    /// from it even though picking leaves Visual mode first.
    pub fn open_ai_prompt_picker(&mut self) {
        let selection = if matches!(self.mode, crate::mode::Mode::Visual(_)) {
            let anchor = self.visual_anchor;
            let cursor = self.cursor();
            self.visual_anchor = None;
            let a = anchor.map(|a| a.0).unwrap_or(cursor.0);
            Some((a.min(cursor.0), a.max(cursor.0)))
        } else {
            None
        };
        self.enter_normal();
        let entries: Vec<Entry> = PROMPTS
            .iter()
            .map(|(label, instruction)| {
                let mut action = serde_json::json!({ "instruction": instruction });
                if let Some((s, z)) = selection {
                    action["selection"] = serde_json::json!([s, z]);
                }
                let mut e = Entry::text(*label);
                e.action = Some(serde_json::json!({ "_vaayu_ai_prompt": action }));
                e
            })
            .collect();
        self.show_results(Results::new(
            "AI prompt — Enter opens the Claude sidebar and sends the prompt + context",
            entries,
        ));
    }

    /// `:ai <instruction>`: free-form prompt, no picker. `selection` is the Ex
    /// range (e.g. `'<,'>` from Visual mode), if any.
    pub fn ai_prompt(&mut self, instruction: &str, selection: Option<(usize, usize)>) {
        let mut action = serde_json::json!({ "instruction": instruction });
        if let Some((s, z)) = selection {
            action["selection"] = serde_json::json!([s, z]);
        }
        self.send_ai_prompt(&action);
    }

    /// Assembles `instruction` + a context block (the selected lines if a
    /// selection is given, else the whole file), a cursor-position line, and the
    /// file's diagnostics when present. Reuses `build_context` for each piece.
    fn build_ai_prompt(
        &mut self,
        instruction: &str,
        selection: Option<(usize, usize)>,
    ) -> Result<String, String> {
        let code = if selection.is_some() {
            self.build_context("selection", selection)?
        } else {
            self.build_context("file", None)?
        };
        let (line, col) = self.cursor();
        let rel = self.context_rel_path();
        let mut out = format!("{instruction}\n\n{code}\n\nCursor: {rel}:{}:{}", line + 1, col + 1);
        // Diagnostics are optional context: `build_context` errors when there
        // are none, which we treat as "nothing to append".
        if let Ok(diags) = self.build_context("diagnostics", None) {
            out.push_str("\n\n");
            out.push_str(&diags);
        }
        Ok(out)
    }

    /// Dispatches a `_vaayu_ai_prompt` entry (or a `:ai` invocation): builds the
    /// prompt, copies it to the `+` register, opens/reuses the `claude` sidebar,
    /// and pastes it in (not auto-submitted -- the human presses Enter, matching
    /// `send_context`).
    pub fn send_ai_prompt(&mut self, value: &serde_json::Value) {
        let instruction = value["instruction"].as_str().unwrap_or_default().to_string();
        let selection = value
            .get("selection")
            .and_then(|v| v.as_array())
            .and_then(|a| {
                let s = a.first()?.as_u64()? as usize;
                let z = a.get(1)?.as_u64()? as usize;
                Some((s, z))
            });
        let text = match self.build_ai_prompt(&instruction, selection) {
            Ok(t) => t,
            Err(e) => {
                self.set_message(e);
                return;
            }
        };
        self.registers.set(Some('+'), text.clone(), false);
        if !self.ensure_ai_sidebar() {
            // The CLI could not be started; ensure_ai_sidebar set the message.
            return;
        }
        if let Some(pty) = self.attached_agent_terminal() {
            pty.write_pasted_input(&text);
            self.set_message("Sent prompt to Claude (press Enter in the sidebar to submit)");
        } else {
            self.set_message("Copied prompt to + register");
        }
    }
}
