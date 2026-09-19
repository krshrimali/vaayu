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
}
