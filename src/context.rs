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

/// Built-in AI prompt templates for the `:ai` / `,ca` agent sidebar:
/// label shown in the picker, the instruction sent ahead of the context, and
/// which context goes with it (see `AiContext`). `ai_prompts` in the config
/// adds more or replaces one by label.
const PROMPTS: &[(&str, &str, &str)] = &[
    ("Explain", "Explain what the following code does.", "auto"),
    (
        "Fix bugs",
        "Find and fix any bugs in the following code.",
        "auto",
    ),
    (
        "Fix diagnostics",
        "Fix the diagnostics reported for the following code.",
        "auto",
    ),
    ("Write tests", "Write tests for the following code.", "auto"),
    (
        "Review",
        "Review the following code for bugs, edge cases, and improvements.",
        "auto",
    ),
    (
        "Review changes",
        "Review my uncommitted changes to this file for bugs, edge cases, and improvements.",
        "diff",
    ),
    (
        "Add docs",
        "Add documentation comments to the following code.",
        "auto",
    ),
    (
        "Optimize",
        "Improve the performance and clarity of the following code.",
        "auto",
    ),
];

/// What code `:ai` sends along with an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiContext {
    /// The selection if there is one, else the whole file.
    Auto,
    File,
    Selection,
    /// Changes to this file since `HEAD`, unsaved edits included.
    Diff,
    /// Every open file buffer.
    Buffers,
    /// The instruction alone (plus the cursor position).
    None,
}

impl AiContext {
    pub fn parse(name: &str) -> Option<AiContext> {
        Some(match name.trim() {
            "" | "auto" => AiContext::Auto,
            "file" => AiContext::File,
            "selection" => AiContext::Selection,
            "diff" => AiContext::Diff,
            "buffers" => AiContext::Buffers,
            "none" => AiContext::None,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            AiContext::Auto => "auto",
            AiContext::File => "file",
            AiContext::Selection => "selection",
            AiContext::Diff => "diff",
            AiContext::Buffers => "buffers",
            AiContext::None => "none",
        }
    }
}

/// The code a prompt is about: whole lines `lines` (inclusive), and for a
/// characterwise Visual selection, its exact start and end (inclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiSelection {
    pub lines: (usize, usize),
    pub chars: Option<((usize, usize), (usize, usize))>,
}

impl AiSelection {
    pub fn lines(s: usize, z: usize) -> AiSelection {
        AiSelection {
            lines: (s.min(z), s.max(z)),
            chars: None,
        }
    }

    /// A characterwise selection between two (line, col) ends, in any order.
    pub fn chars(a: (usize, usize), b: (usize, usize)) -> AiSelection {
        let (from, to) = if a <= b { (a, b) } else { (b, a) };
        AiSelection {
            lines: (from.0, to.0),
            chars: Some((from, to)),
        }
    }

    fn write(self, action: &mut serde_json::Value) {
        action["selection"] = serde_json::json!([self.lines.0, self.lines.1]);
        if let Some(((l0, c0), (l1, c1))) = self.chars {
            action["chars"] = serde_json::json!([l0, c0, l1, c1]);
        }
    }

    fn read(action: &serde_json::Value) -> Option<AiSelection> {
        let a = action.get("selection")?.as_array()?;
        let s = a.first()?.as_u64()? as usize;
        let z = a.get(1)?.as_u64()? as usize;
        let chars = action.get("chars").and_then(|c| {
            let c: Vec<usize> = c
                .as_array()?
                .iter()
                .map(|v| v.as_u64().map(|v| v as usize))
                .collect::<Option<_>>()?;
            (c.len() == 4).then(|| ((c[0], c[1]), (c[2], c[3])))
        });
        Some(AiSelection {
            lines: (s, z),
            chars,
        })
    }
}

/// The Markdown fence language for a file, from its extension.
fn fence_lang(path: Option<&std::path::Path>) -> String {
    let Some(ext) = path
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return String::new();
    };
    match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "rb" => "ruby",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "sh" | "bash" | "zsh" => "bash",
        "fish" => "fish",
        "md" | "markdown" => "markdown",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "json" | "jsonc" => "json",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "lua" => "lua",
        "php" => "php",
        "sql" => "sql",
        "zig" => "zig",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "ml" | "mli" => "ocaml",
        "nix" => "nix",
        other => other,
    }
    .to_string()
}

/// `text` inside a fenced block, ending with exactly one newline before the
/// closing fence.
fn fenced(lang: &str, text: &str) -> String {
    let body = text.trim_end_matches('\n');
    format!("```{lang}\n{body}\n```")
}

impl Editor {
    pub fn open_colorscheme_picker(&mut self) {
        let original = self.config.colorscheme.clone();
        self.colorscheme_picker_original = Some(original.clone());
        let entries = crate::theme::NAMES
            .iter()
            .map(|name| {
                let mut e = Entry::text(*name);
                e.action = Some(serde_json::json!({"_vaayu_colorscheme": name}));
                e
            })
            .collect();
        self.show_results(Results::new(
            "Colorschemes — Enter applies, Esc cancels",
            entries,
        ));
        self.preview_colorscheme_picker();
    }

    pub fn preview_colorscheme_picker(&mut self) {
        let name = self
            .results
            .as_ref()
            .filter(|r| {
                self.colorscheme_picker_original.is_some() && r.title.starts_with("Colorschemes —")
            })
            .and_then(|r| r.entries.get(r.cursor))
            .map(|e| e.text.clone());
        if let Some(name) = name {
            if let Some(theme) = crate::theme::builtin(&name) {
                self.theme = theme;
                self.syntax_stamp = self.syntax_stamp.wrapping_add(1);
            }
        }
    }

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
        let mut results = Results::new(
            "Send context — Enter copies it (and sends it if an agent session is attached)",
            entries,
        );
        results.menu = true;
        self.show_results(results);
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
            "file" => {
                let lang = fence_lang(path.as_deref());
                Ok(format!(
                    "File: {rel}\n{}",
                    fenced(&lang, &self.buf().rope.to_string())
                ))
            }
            "selection" => {
                let (s, z) = selection.ok_or("No Visual selection was active")?;
                let lang = fence_lang(path.as_deref());
                let mut text = String::new();
                for l in s..=z {
                    text.push_str(&self.buf().line_text(l));
                    text.push('\n');
                }
                Ok(format!(
                    "File: {rel}:{}-{}\n{}",
                    s + 1,
                    z + 1,
                    fenced(&lang, &text)
                ))
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
                Ok(format!("Clipboard:\n{}", fenced("", &text)))
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
                    // `d.col` is a UTF-16 code-unit offset (LSP convention);
                    // convert to a char column so it matches the buffer and the
                    // rest of vaayu's diagnostic surfaces on astral-plane text.
                    let col = crate::language::utf16_to_col(&self.buf().line_text(d.line), d.col);
                    text.push_str(&format!(
                        "{}:{}: {:?}: {}\n",
                        d.line + 1,
                        col + 1,
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
        let last = self
            .last_agent
            .filter(|&id| self.windows.iter().any(|w| w.terminal == Some(id)));
        let id = last.or_else(|| {
            self.windows.iter().find_map(|w| {
                let id = w.terminal?;
                self.terminals
                    .iter()
                    .any(|p| p.id == id && p.agent_kind.is_some())
                    .then_some(id)
            })
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
            let kind = pty.display_name();
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

    /// The prompt templates the picker offers: the built-ins, with any
    /// `ai_prompts` entry of the same label replacing one, then the rest of
    /// `ai_prompts` in order. Entries with a blank label or instruction or an
    /// unknown context are skipped.
    pub fn ai_templates(&self) -> Vec<(String, String, AiContext)> {
        let custom: Vec<(String, String, AiContext)> = self
            .config
            .ai_prompts
            .iter()
            .filter(|t| !t.label.trim().is_empty() && !t.instruction.trim().is_empty())
            .filter_map(|t| {
                Some((
                    t.label.trim().to_string(),
                    t.instruction.trim().to_string(),
                    AiContext::parse(&t.context)?,
                ))
            })
            .collect();
        let mut out: Vec<(String, String, AiContext)> = PROMPTS
            .iter()
            .map(|(label, instruction, context)| {
                custom
                    .iter()
                    .find(|c| c.0 == *label)
                    .cloned()
                    .unwrap_or_else(|| {
                        (
                            label.to_string(),
                            instruction.to_string(),
                            AiContext::parse(context).unwrap_or(AiContext::Auto),
                        )
                    })
            })
            .collect();
        for c in custom {
            if !out.iter().any(|o| o.0 == c.0) {
                out.push(c);
            }
        }
        out
    }

    /// The live Visual selection, if any, as an `AiSelection`
    /// (characterwise when the selection is), leaving Visual mode.
    fn take_visual_ai_selection(&mut self) -> Option<AiSelection> {
        let crate::mode::Mode::Visual(kind) = self.mode else {
            return None;
        };
        let cursor = self.cursor();
        let anchor = self.visual_anchor.take().unwrap_or(cursor);
        Some(if kind == crate::mode::VisualKind::Char {
            AiSelection::chars(anchor, cursor)
        } else {
            AiSelection::lines(anchor.0, cursor.0)
        })
    }

    /// `,ca`: the AI picker, for the live Visual selection if there is one.
    pub fn open_ai_prompt_picker(&mut self) {
        let selection = self.take_visual_ai_selection();
        self.open_ai_prompt_picker_for(selection);
    }

    /// `:ai` with no instruction: opens a picker to start (or focus) the
    /// agent, paste a prompt template with `selection` (else the whole file)
    /// as context, or restart/stop a running agent.
    pub fn open_ai_prompt_picker_for(&mut self, selection: Option<AiSelection>) {
        self.enter_normal();
        let kind = self.config.ai_agent().to_string();
        self.reap_exited_agent(&kind);
        let current = self.current_agent(&kind);
        let running = current.is_some();
        let sessions = self
            .terminals
            .iter()
            .filter(|p| p.agent_kind.as_deref() == Some(kind.as_str()))
            .count();
        // With several sessions, say which one the templates go to.
        let label = match current {
            Some(id) if sessions > 1 => self.ai_session_label(id),
            _ => self.config.ai_agent_label().to_string(),
        };
        let mut start = Entry::text(if running { "Focus" } else { "Start" });
        start.detail = format!("Open {label} in the sidebar without sending anything");
        start.action = Some(serde_json::json!({ "_vaayu_ai_start": true }));
        let mut entries = vec![start];
        if running {
            let mut new = Entry::text("New session");
            new.detail = format!(
                "Start another {} session next to the running one{}; :ai then sends to it",
                self.config.ai_agent_label(),
                if sessions > 1 { "s" } else { "" }
            );
            new.action = Some(serde_json::json!({ "_vaayu_ai_new": true }));
            entries.push(new);
        }
        // `:vert ai` / `:hor ai`: the pane opens only once an entry is
        // chosen, so the entries carry the forced split direction.
        let split = self.split_override;
        for (name, instruction, context) in self.ai_templates() {
            let mut action = serde_json::json!({
                "instruction": instruction,
                "context": context.name(),
            });
            if let Some(sel) = selection {
                sel.write(&mut action);
            }
            let mut e = Entry::text(name);
            e.detail = format!("{instruction}\n\nContext: {}", context.name());
            e.action = Some(serde_json::json!({ "_vaayu_ai_prompt": action }));
            entries.push(e);
        }
        if running {
            let mut restart = Entry::text(format!("Restart {label}"));
            restart.detail = "Stop the running session and start a fresh one".into();
            restart.action = Some(serde_json::json!({ "_vaayu_ai_restart": true }));
            let mut stop = Entry::text(format!("Stop {label}"));
            stop.detail = "Stop the running session and close its pane".into();
            stop.action = Some(serde_json::json!({ "_vaayu_ai_stop": true }));
            entries.push(restart);
            entries.push(stop);
        }
        if let Some(vertical) = split {
            for e in &mut entries {
                if let Some(serde_json::Value::Object(map)) = e.action.as_mut() {
                    map.insert("_vaayu_split".into(), vertical.into());
                }
            }
        }
        let scope = match selection {
            Some(s) if s.lines.0 == s.lines.1 => format!("line {}", s.lines.0 + 1),
            Some(s) => format!("lines {}-{}", s.lines.0 + 1, s.lines.1 + 1),
            None => "this file".to_string(),
        };
        let mut results = Results::new(
            format!("AI ({label}) — templates paste a prompt about {scope}"),
            entries,
        );
        results.menu = true;
        self.show_results(results);
    }

    /// `:ai [--file|--selection|--diff|--buffers|--none] <instruction>`:
    /// free-form prompt, no picker. `selection` is the Ex range or Visual
    /// selection, if any.
    pub fn ai_prompt(&mut self, args: &str, selection: Option<AiSelection>, new_session: bool) {
        let mut context = AiContext::Auto;
        let mut rest = args.trim();
        while let Some(flag) = rest.strip_prefix("--") {
            let (name, tail) = flag.split_once(char::is_whitespace).unwrap_or((flag, ""));
            match AiContext::parse(name).filter(|_| !name.is_empty()) {
                Some(c) => context = c,
                None => {
                    self.set_message(format!(
                        "Unknown :ai option --{name} (use --file, --selection, --diff, --buffers or --none)"
                    ));
                    return;
                }
            }
            rest = tail.trim_start();
        }
        if rest.is_empty() {
            self.set_message(
                "Usage: :ai [--file|--selection|--diff|--buffers|--none] <instruction>",
            );
            return;
        }
        let mut action = serde_json::json!({ "instruction": rest, "context": context.name() });
        if let Some(sel) = selection {
            sel.write(&mut action);
        }
        if new_session {
            action["new_session"] = true.into();
        }
        self.send_ai_prompt(&action);
    }

    /// The text of a characterwise selection, ends inclusive.
    fn char_span_text(&self, from: (usize, usize), to: (usize, usize)) -> String {
        let rope = &self.buf().rope;
        let last = rope.len_lines().saturating_sub(1);
        let index = |(l, c): (usize, usize)| {
            let l = l.min(last);
            let start = rope.line_to_char(l);
            let len = rope.line(l).len_chars();
            start + c.min(len)
        };
        let a = index(from);
        let b = (index(to) + 1).min(rope.len_chars()).max(a);
        rope.slice(a..b).to_string()
    }

    /// The code block for an `AiSelection`.
    fn ai_selection_context(&mut self, sel: AiSelection) -> Result<String, String> {
        let Some((from, to)) = sel.chars else {
            return self.build_context("selection", Some(sel.lines));
        };
        let text = self.char_span_text(from, to);
        let lang = fence_lang(self.buf().path.as_deref());
        Ok(format!(
            "File: {}:{}:{}-{}:{}\n{}",
            self.context_rel_path(),
            from.0 + 1,
            from.1 + 1,
            to.0 + 1,
            to.1 + 1,
            fenced(&lang, &text)
        ))
    }

    /// This buffer's changes since `HEAD` (unsaved edits included) as a
    /// unified diff.
    fn ai_diff_context(&self) -> Result<String, String> {
        let path = self
            .buf()
            .path
            .clone()
            .ok_or("This buffer has no file on disk to diff")?;
        let dir = path.parent().unwrap_or(&self.project_root).to_path_buf();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("This file's name is not valid UTF-8")?;
        let inside = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&dir)
            .output()
            .map_err(|e| format!("Could not run git: {e}"))?;
        if !inside.status.success() {
            return Err("This file is not in a Git repository".into());
        }
        let head = std::process::Command::new("git")
            .args(["show", &format!("HEAD:./{name}")])
            .current_dir(&dir)
            .output()
            .map_err(|e| format!("Could not run git: {e}"))?;
        let (old, note) = if head.status.success() {
            (String::from_utf8_lossy(&head.stdout).into_owned(), "")
        } else {
            (String::new(), " (new file, not in HEAD)")
        };
        let new = self.buf().rope.to_string();
        let rel = self.context_rel_path();
        let diff = similar::TextDiff::from_lines(&old, &new)
            .unified_diff()
            .context_radius(3)
            .header(&format!("a/{rel}"), &format!("b/{rel}"))
            .to_string();
        if diff.trim().is_empty() {
            return Err(format!("No changes to {rel} since HEAD"));
        }
        let unsaved = if self.buf().is_modified() {
            ", unsaved edits included"
        } else {
            ""
        };
        Ok(format!(
            "Changes to {rel} since HEAD{note}{unsaved}:\n{}",
            fenced("diff", &diff)
        ))
    }

    /// Every open file buffer, each in its own block.
    fn ai_buffers_context(&self) -> Result<String, String> {
        let mut blocks = Vec::new();
        for b in &self.buffers {
            let Some(path) = b.path.as_deref() else {
                continue;
            };
            if b.note.is_some() {
                continue;
            }
            let rel = path
                .strip_prefix(&self.project_root)
                .unwrap_or(path)
                .display()
                .to_string();
            blocks.push(format!(
                "File: {rel}\n{}",
                fenced(&fence_lang(Some(path)), &b.rope.to_string())
            ));
        }
        if blocks.is_empty() {
            return Err("No open file buffers to send".into());
        }
        Ok(blocks.join("\n\n"))
    }

    /// Assembles `instruction` + the `context` block, a cursor-position
    /// line, and the file's diagnostics when present.
    pub(crate) fn build_ai_prompt(
        &mut self,
        instruction: &str,
        context: AiContext,
        selection: Option<AiSelection>,
    ) -> Result<String, String> {
        let code = match (context, selection) {
            (AiContext::Auto | AiContext::Selection, Some(sel)) => {
                Some(self.ai_selection_context(sel)?)
            }
            (AiContext::Selection, None) => {
                return Err("This prompt needs a Visual selection or a line range".into())
            }
            (AiContext::Auto | AiContext::File, _) => Some(self.build_context("file", None)?),
            (AiContext::Diff, _) => Some(self.ai_diff_context()?),
            (AiContext::Buffers, _) => Some(self.ai_buffers_context()?),
            (AiContext::None, _) => None,
        };
        let (line, col) = self.cursor();
        let rel = self.context_rel_path();
        let mut out = instruction.to_string();
        if let Some(code) = code {
            out.push_str("\n\n");
            out.push_str(&code);
        }
        out.push_str(&format!("\n\nCursor: {rel}:{}:{}", line + 1, col + 1));
        // Diagnostics are optional context: `build_context` errors when there
        // are none, which we treat as "nothing to append".
        if context != AiContext::None {
            if let Ok(diags) = self.build_context("diagnostics", None) {
                out.push_str("\n\n");
                out.push_str(&diags);
            }
        }
        Ok(out)
    }

    /// Ensure the configured AI sidebar is open and paste `text` into it (not
    /// auto-submitted). Returns false if the CLI couldn't be started (a
    /// message is already set). Used by the code tours.
    pub(crate) fn send_to_ai_sidebar(&mut self, text: &str) -> bool {
        self.paste_to_ai_sidebar(text, false, false).is_some()
    }

    /// The configured agent's label for session `id`: `Claude`, or
    /// `Claude #2` from the second session on.
    pub(crate) fn ai_session_label(&self, id: u64) -> String {
        let label = self.config.ai_agent_label();
        match self.terminals.iter().find(|p| p.id == id) {
            Some(p) if p.agent_slot > 1 => format!("{label} #{}", p.agent_slot),
            _ => label.to_string(),
        }
    }

    /// Ensure the configured AI sidebar is open and paste `text` into it,
    /// pressing Enter afterwards when `submit`. Returns the session's id, or
    /// `None` if the CLI couldn't be started (a message is already set).
    fn paste_to_ai_sidebar(&mut self, text: &str, submit: bool, new: bool) -> Option<u64> {
        let state = if new {
            match self.new_ai_session() {
                Some(id) => crate::pty::SidebarState::Spawned(id),
                None => crate::pty::SidebarState::Missing,
            }
        } else {
            self.ensure_ai_sidebar()
        };
        match state {
            crate::pty::SidebarState::Missing => None,
            crate::pty::SidebarState::Reused(id) => {
                // If a deferred send to this same (still-starting) session is
                // already queued, append so both deliver in order once it's
                // ready rather than racing its init.
                if let Some(pending) = self.pending_agent_send.iter_mut().find(|p| p.id == id) {
                    pending.text.push('\n');
                    pending.text.push_str(text);
                    pending.submit |= submit;
                    return Some(id);
                }
                // Target the configured session by id, since another agent
                // could have a pane in an earlier window.
                if let Some(pty) = self.terminals.iter_mut().find(|p| p.id == id) {
                    pty.write_pasted_input(text);
                }
                if submit {
                    self.pending_agent_submit
                        .push((id, std::time::Instant::now()));
                }
                Some(id)
            }
            crate::pty::SidebarState::Spawned(id) => {
                // The CLI's TUI is still starting; pasting now races its init
                // and the text lands garbled. Defer until it's drawn its prompt
                // (flush_pending_agent_send).
                let rev = self
                    .terminals
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.output_revision())
                    .unwrap_or(0);
                let now = std::time::Instant::now();
                self.pending_agent_send.retain(|p| p.id != id);
                self.pending_agent_send.push(crate::pty::PendingAgentSend {
                    id,
                    text: text.to_string(),
                    submit,
                    spawned_at: now,
                    last_rev: rev,
                    last_change_at: now,
                });
                Some(id)
            }
        }
    }

    /// Dispatches a `_vaayu_ai_prompt` entry (or a `:ai` invocation): builds
    /// the prompt, opens/reuses the configured AI sidebar and pastes it in;
    /// then focuses the sidebar (`ai_focus_on_send`) and presses Enter
    /// (`ai_auto_submit`) as configured.
    pub fn send_ai_prompt(&mut self, value: &serde_json::Value) {
        let instruction = value["instruction"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let context = value["context"]
            .as_str()
            .and_then(AiContext::parse)
            .unwrap_or(AiContext::Auto);
        let selection = AiSelection::read(value);
        let text = match self.build_ai_prompt(&instruction, context, selection) {
            Ok(t) => t,
            Err(e) => {
                self.set_message(e);
                return;
            }
        };
        let submit = self.config.ai_auto_submit;
        let new = value
            .get("new_session")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let Some(id) = self.paste_to_ai_sidebar(&text, submit, new) else {
            // Only clobber the clipboard as the fallback, not on every success.
            self.registers.set(Some('+'), text, false);
            self.set_message(format!("Copied prompt to + register ({})", self.message));
            return;
        };
        let focus = self.config.ai_focus_on_send;
        if focus {
            if let Some(idx) = self.windows.iter().position(|w| w.terminal == Some(id)) {
                self.focus_pane_buffer(idx);
                self.mode = crate::mode::Mode::Terminal;
            }
        } else if self.mode == crate::mode::Mode::Terminal && self.active_terminal_id() == Some(id)
        {
            // A freshly spawned sidebar takes focus; give it back.
            if let Some(idx) = self.windows.iter().position(|w| w.terminal.is_none()) {
                self.focus_pane_buffer(idx);
                self.mode = crate::mode::Mode::Normal;
            }
        }
        let label = self.ai_session_label(id);
        let queued = self.pending_agent_send.iter().any(|p| p.id == id);
        self.set_message(match (submit, queued, focus) {
            (true, true, _) => format!("Prompt queued; {label} will receive and submit it once it has started"),
            (true, false, _) => format!("Sent and submitted the prompt to {label}"),
            (false, true, _) => format!("Prompt queued; it is pasted into {label} once it has started — press Enter there to submit"),
            (false, false, true) => format!("Sent prompt to {label} — press Enter to submit"),
            (false, false, false) => format!("Sent prompt to {label} (press Enter in the sidebar to submit)"),
        });
    }
}
