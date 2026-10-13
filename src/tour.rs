//! `.tours/*.tour` code tours (VS Code CodeTour-compatible JSON). `:tours`
//! lists the project's tours; `:tour [name]` starts one; `:tournext`/
//! `:tourprev` step through, jumping to each step and showing its description.

use crate::editor::Editor;
use crate::results::{Entry, Results};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub struct ExplanationFocus {
    pub source_buffer: u64,
    pub source_window: usize,
    pub explanation_buffer: u64,
}

fn one() -> usize {
    1
}

fn encode_github_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[derive(Deserialize, Serialize, Clone)]
pub struct TourStep {
    #[serde(default)]
    pub file: String,
    #[serde(default = "one")]
    pub line: usize,
    /// Optional 1-based end line: the step highlights `line..=end_line`.
    #[serde(
        default,
        rename = "endLine",
        alias = "end_line",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_line: Option<usize>,
    /// Optional anchor text (VS Code CodeTour-compatible): if present, the step
    /// re-locates to a matching line (nearest the authored line if ambiguous),
    /// so the tour survives edits that shift line numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default)]
    pub description: String,
    /// Optional Vaayu visuals; ordinary CodeTour steps need none of these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vaayu: Option<crate::tour_visuals::TourVisuals>,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct Tour {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub steps: Vec<TourStep>,
}

/// A short, filesystem-safe file stem from a user-supplied tour name or prompt.
fn slugify(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    // Collapse runs of '-' and trim the ends for a clean stem.
    let mut out = String::new();
    for c in s.chars() {
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "tour".to_string()
    } else {
        out
    }
}

fn tour_github_permalink(
    root: &std::path::Path,
    path: &std::path::Path,
    relative: &str,
    start: usize,
    end: usize,
) -> Option<String> {
    if !path.is_file() || relative.starts_with('/') || relative.split('/').any(|part| part == "..")
    {
        return None;
    }
    let remote = crate::git_tools::remote_url(root).ok()?;
    let (host, owner, repo) = crate::git_tools::parse_github_remote(&remote)?;
    let commit = crate::git_tools::head_commit(root).ok()?;
    let object = format!("HEAD:{relative}");
    let tracked = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "-e", object.as_str()])
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?
        .success();
    if !tracked {
        return None;
    }
    let unchanged = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--quiet", "HEAD", "--", relative])
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?
        .success();
    if !unchanged {
        return None;
    }
    let (lo, hi) = (start.min(end), start.max(end));
    let range = if lo == hi {
        format!("L{}", lo + 1)
    } else {
        format!("L{}-L{}", lo + 1, hi + 1)
    };
    Some(format!(
        "https://{host}/{owner}/{repo}/blob/{commit}/{}#{range}",
        encode_github_path(relative)
    ))
}

impl Editor {
    fn tours_dir(&self) -> PathBuf {
        self.project_root.join(".tours")
    }

    /// `:tours`: list `.tours/*.tour` files; Enter starts the selected tour.
    pub fn list_tours(&mut self) {
        self.tour_catalog_start();
    }

    /// `K` in the `:tours` list starts the selected tour and focuses its
    /// explanation buffer immediately.
    pub fn start_selected_tour_explanation(&mut self) {
        let name = self
            .results
            .as_ref()
            .filter(|results| results.title.starts_with("Tours — Enter starts"))
            .and_then(|results| results.entries.get(results.cursor))
            .and_then(|entry| entry.action.as_ref())
            .and_then(|action| action.get("_vaayu_rerun_ex"))
            .and_then(serde_json::Value::as_str)
            .and_then(|command| {
                command
                    .strip_prefix("tour ")
                    .or_else(|| command.strip_prefix("tourresume "))
            })
            .map(str::to_string);
        let Some(name) = name else {
            self.set_message("Select a tour first");
            return;
        };
        self.tour_resume(&name);
        if self.active_tour.is_some() {
            self.toggle_tour_explanation();
        }
    }

    /// `:tournew [name]`: open a scratch buffer for a plain-English description
    /// of the tour you want. `:toursave` then hands that prompt to the AI
    /// sidebar, which explores the repo and writes the concrete `.tour` file --
    /// the user supplies only the prompt.
    pub fn tour_new(&mut self, name: &str) {
        if self.active_tour.is_some() {
            self.tour_end();
        }
        let agent = self.config.ai_agent_label();
        let template = format!(
            "# Describe the code tour you want in plain English, then run :toursave\n\
             # to have {agent} explore the repo and generate it into .tours/*.tour.\n\
             #\n\
             # e.g. \"Walk a new contributor through how a keypress becomes a screen\n\
             #       update: input handling, the editor state update, then rendering.\"\n\
             #\n\
             # Lines starting with # are ignored.\n\n"
        );
        let mut b = crate::buffer::Buffer::empty();
        b.rope = ropey::Rope::from_str(&template);
        b.mark_saved();
        self.buffers.push(b);
        self.cur = self.buffers.len() - 1;
        let id = self.buf().id;
        self.tour_draft = Some((name.trim().to_string(), id));
        self.invalidate_index_caches();
        let last = self.buf().line_count().saturating_sub(1);
        self.set_cursor(last, 0);
        self.enter_normal();
        self.set_message(format!(
            "Describe the tour (i to edit), then :toursave to generate it with {}",
            self.config.ai_agent_label()
        ));
    }

    /// `:toursave`: send the `:tournew` prompt to the configured AI sidebar with
    /// instructions to write a concrete `.tours/<slug>.tour` (deterministic
    /// JSON) that `:tour` can then run.
    pub fn tour_save(&mut self, name_override: &str) {
        let Some((draft_name, id)) = self.tour_draft.clone() else {
            self.set_message("No tour draft — run :tournew first");
            return;
        };
        // A name given on `:toursave <name>` (e.g. from the `:w`/`:wq` prompt)
        // wins and is remembered on the draft.
        let name = if name_override.trim().is_empty() {
            draft_name
        } else {
            let n = name_override.trim().to_string();
            self.tour_draft = Some((n.clone(), id));
            n
        };
        // Read the draft buffer by id -- never fall back to the current buffer,
        // which would ship an unrelated (possibly sensitive) file to the AI.
        let Some(text) = self
            .buffers
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.rope.to_string())
        else {
            self.tour_draft = None;
            self.set_message("Tour draft buffer was closed — run :tournew again");
            return;
        };
        // The prompt is the buffer minus comment/blank lines.
        let prompt = text
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = prompt.trim();
        if prompt.is_empty() {
            self.set_message("Write a description of the tour first, then :toursave");
            return;
        }
        let slug = if name.is_empty() {
            slugify(prompt)
        } else {
            slugify(&name)
        };
        let instruction = format!(
            "Generate a vaayu code tour. Explore this repository and design a guided, \
             narrative tour for the following request:\n\n{prompt}\n\n\
             Write it to the file `.tours/{slug}.tour` (create the .tours directory if \
             needed) as JSON conforming to this schema:\n{}\n\n\
             Use real files, 1-based line numbers, and exact source anchors. Prefer a \
             unique pattern; for repeated lines supply vaayu.context (text on the \
             immediately preceding line). Use the smallest useful range and \
             vaayu.endPattern when its ending needs to survive edits. Never use a \
             generic anchor such as 'loop {{' or '}}' without disambiguation.\n\
             Design a coherent 5-12 step narrative, one idea per step. Explain what \
             the code does, why it matters, and how it connects to the next step. \
             Optional vaayu visuals are useful ONLY when they clarify the lesson. \
             Omit vaayu or unused fields for plain steps; do not force visuals into \
             every step. Prefer 1-3 focused annotations with short labels. Connections \
             reference annotation IDs in the same step and must describe relationships \
             verified in the code; annotation.file supports cross-file targets. Add a \
             diff only for a meaningful comparison: kind=git uses real revisions \
             (prefer full commit hashes; target=buffer includes unsaved edits), while \
             kind=snippets is always labeled an illustrative example. Explain the \
             purpose of a comparison in its label. Never invent historical code.\n\
             Plain step example: {{\"file\":\"src/main.rs\",\"line\":1,\"description\":\"Explain this location.\"}}.\n\
             Enhanced shape (replace ALL sample locations and text with verified repo \
             content): {{\"file\":\"src/main.rs\",\"line\":1,\"description\":\"Explain a relationship.\",\"vaayu\":{{\"annotations\":[{{\"id\":\"input\",\"line\":1,\"label\":\"Receive input\"}},{{\"id\":\"output\",\"line\":2,\"label\":\"Pass the result\"}}],\"connections\":[{{\"from\":\"input\",\"to\":\"output\",\"label\":\"Passes the decoded value\"}}]}}}}.\n\
             Before finishing, validate JSON, all file paths and ranges, both anchors, \
             unique annotation IDs, connection endpoints, and Git revisions. Create \
             only that one tour file and print nothing else.",
            crate::tour_visuals::TOUR_SCHEMA
        );
        if !self.send_to_ai_sidebar(&instruction) {
            return; // the CLI couldn't be started; message already set
        }
        self.set_message(format!(
            "Sent to {} — press Enter in the sidebar; when it finishes: :tour {slug}",
            self.config.ai_agent_label()
        ));
    }

    /// `:tourend`: stop the active tour (dismisses the step panel).
    pub fn tour_end(&mut self) {
        self.restore_tour_source_view();
        self.leave_tour_explanation();
        self.tour_visuals.clear_step();
        self.tour_visuals.clear_anchors();
        self.tour_ui.source_error = None;
        self.tour_ui.completed = false;
        self.tour_highlight = None;
        self.tour_markers = None;
        if self.active_tour.take().is_some() {
            self.active_tour_name = None;
            self.set_message("Tour ended");
        } else {
            self.set_message("No active tour");
        }
    }

    /// Restart at step one while keeping the current tour active.
    pub fn tour_restart(&mut self) {
        let Some(name) = self
            .active_tour_name
            .clone()
            .or_else(|| self.last_tour.as_ref().map(|(name, _)| name.clone()))
        else {
            self.start_tour("");
            return;
        };
        self.start_tour(&name);
    }

    /// Move focus between the source pane and the tour's existing bottom split.
    pub fn toggle_tour_explanation(&mut self) {
        if self.active_tour.is_none() {
            self.request_hover();
            return;
        }
        if let Some(focus) = self.tour_explanation {
            if self.buf().id == focus.explanation_buffer {
                self.focus_tour_source(focus);
                self.set_message("Tour source focused · K returns to the explanation");
                return;
            }
            self.store_window();
            if let Some(pane) = self.tour_buffer_pane(focus.explanation_buffer, true) {
                self.focus_window(pane);
                self.set_message("Tour explanation focused · K returns to the source");
                return;
            }
            self.focus_tour_source(focus);
        }
        if let Some(pane) = self.ensure_tour_explanation() {
            self.focus_window(pane);
            self.set_message(
                "Tour explanation focused · Vim motions and editing are available · K returns",
            );
        }
    }

    fn tour_buffer_pane(&self, buffer: u64, allow_preview: bool) -> Option<usize> {
        self.windows.iter().position(|w| {
            w.buffer == buffer
                && (allow_preview || !w.preview)
                && w.terminal.is_none()
                && !w.file_tree
                && !w.outline
        })
    }

    pub(crate) fn is_tour_description(&self, buffer: u64) -> bool {
        self.active_tour.is_some()
            && self
                .tour_explanation
                .is_some_and(|focus| focus.explanation_buffer == buffer)
    }

    /// Switch how the same scratch text is displayed without replacing edits,
    /// moving focus, or changing the source/description cursors and viewports.
    pub fn tour_markdown(&mut self, mode: &str) {
        if self.active_tour.is_none() {
            self.set_message("No active tour");
            return;
        }
        let enabled = match mode.trim() {
            "" | "toggle" => !self.tour_visuals.markdown,
            "on" => true,
            "off" => false,
            _ => {
                self.set_message("Usage: :tourmarkdown [on|off|toggle]");
                return;
            }
        };
        self.store_window();
        self.tour_visuals.markdown = enabled;
        if let Some(focus) = self.tour_explanation {
            for window in self
                .windows
                .iter_mut()
                .filter(|w| w.buffer == focus.explanation_buffer)
            {
                window.preview = enabled;
            }
        }
        self.set_message(if enabled {
            "Tour Markdown on · K description · ,vm / Esc for raw text"
        } else {
            "Tour Markdown off · raw text is editable · ,vm renders it"
        });
    }

    pub(crate) fn focus_tour_source(&mut self, focus: ExplanationFocus) {
        self.store_window();
        let pane = self
            .windows
            .get(focus.source_window)
            .filter(|w| {
                (w.buffer == focus.source_buffer
                    || self.tour_visuals.preview_buffer == Some(w.buffer))
                    && (!w.preview || self.tour_ui.source_error.is_some())
                    && w.terminal.is_none()
                    && !w.file_tree
                    && !w.outline
            })
            .map(|_| focus.source_window)
            .or_else(|| self.tour_buffer_pane(focus.source_buffer, false));
        if let Some(pane) = pane {
            self.focus_window(pane);
        } else {
            self.switch_buffer_id(focus.source_buffer);
        }
    }

    /// Create the explanation pane at tour startup, reusing its buffer and
    /// split on subsequent steps. A normal split supplies mouse hit-testing,
    /// divider dragging, scrolling and independent source/explanation cursors.
    fn ensure_tour_explanation(&mut self) -> Option<usize> {
        self.store_window();
        let source_buffer = self.buf().id;
        let source_window = self.active_window;
        if let Some(mut focus) = self.tour_explanation {
            focus.source_buffer = source_buffer;
            focus.source_window = source_window;
            self.tour_explanation = Some(focus);
            if let Some(pane) = self.tour_buffer_pane(focus.explanation_buffer, true) {
                return Some(pane);
            }
        }
        if self.windows.len() >= 32 {
            self.set_message("At most 32 panes are supported");
            return None;
        }
        let explanation_buffer = self
            .tour_explanation
            .map(|focus| focus.explanation_buffer)
            .filter(|id| self.buffers.iter().any(|b| b.id == *id))
            .unwrap_or_else(|| {
                let mut buffer = crate::buffer::Buffer::empty();
                buffer.rope = ropey::Rope::from_str(&self.tour_explanation_buffer_text());
                buffer.mark_saved();
                let id = buffer.id;
                self.buffers.push(buffer);
                self.invalidate_index_caches();
                id
            });
        self.split_window(false, false);
        let pane = self.active_window;
        self.switch_buffer_id(explanation_buffer);
        self.store_window();
        if let Some(layout) = &mut self.window_layout {
            layout.resize_active(source_window, false, 0.05);
        }
        self.tour_explanation = Some(ExplanationFocus {
            source_buffer,
            source_window,
            explanation_buffer,
        });
        self.windows[pane].preview = self.tour_visuals.markdown;
        self.focus_window(source_window);
        Some(pane)
    }

    /// Copy the active step's explanation and source metadata to `+`.
    pub fn copy_tour_step(&mut self) {
        // Export revalidates the permalink against the current disk/HEAD;
        // ordinary view switches can reuse the cached step metadata.
        self.tour_visuals.metadata.borrow_mut().take();
        let Some(text) = self.tour_explanation_text_with_link() else {
            self.set_message("No active tour step to copy");
            return;
        };
        self.registers.set(Some('+'), text, false);
        self.set_message("Copied tour step explanation and source metadata");
    }

    /// Copy the full tour as formatted CodeTour-compatible JSON to `+`.
    pub fn copy_tour(&mut self) {
        let Some((tour, _)) = &self.active_tour else {
            self.set_message("No active tour to copy");
            return;
        };
        match serde_json::to_string_pretty(tour) {
            Ok(text) => {
                self.registers.set(Some('+'), text, false);
                self.set_message("Copied the full tour as JSON");
            }
            Err(e) => self.set_message(format!("Could not serialize tour: {e}")),
        }
    }

    pub(crate) fn switch_buffer_id(&mut self, id: u64) {
        let Some(index) = self.buffers.iter().position(|buffer| buffer.id == id) else {
            self.tour_explanation = None;
            return;
        };
        if self.buf().id != id {
            self.note_alternate_buffer();
            self.cur = index;
            self.touch_buffer_mru(id);
            self.invalidate_index_caches();
            self.fire_event(crate::events::Event::BufEnter);
        }
        self.enter_normal();
    }

    fn leave_tour_explanation(&mut self) {
        if let Some(focus) = self.tour_explanation.take() {
            if self.buf().id == focus.explanation_buffer {
                self.focus_tour_source(focus);
            }
            self.store_window();
            let return_buffer = self.buf().id;
            let mut return_window = self.active_window;
            while self.windows.len() > 1 {
                let Some(pane) = self.tour_buffer_pane(focus.explanation_buffer, true) else {
                    break;
                };
                self.focus_window(pane);
                self.close_window();
                if pane < return_window {
                    return_window -= 1;
                }
            }
            if self.windows.is_empty() {
                self.switch_buffer_id(return_buffer);
            } else {
                self.focus_window(return_window);
            }
        }
    }

    pub(crate) fn replace_tour_explanation(&mut self, id: u64, text: &str) {
        if let Some(buffer) = self.buffers.iter_mut().find(|buffer| buffer.id == id) {
            buffer.replace_scratch_text(text);
        }
        for window in self.windows.iter_mut().filter(|w| w.buffer == id) {
            window.cursor = (0, 0);
            window.top = 0;
            window.wrap_row = 0;
            window.left = 0;
            window.preview_scroll = 0;
            window.preview_left = 0;
        }
        self.invalidate_index_caches();
    }

    pub(crate) fn tour_explanation_buffer_text(&self) -> String {
        let Some((tour, idx)) = &self.active_tour else {
            return "No active tour step".into();
        };
        let Some(step) = tour.steps.get(*idx) else {
            return "No active tour step".into();
        };
        format!("{}{}\n", step.description, self.tour_visuals_text())
    }

    fn tour_explanation_text_with_link(&self) -> Option<String> {
        let (tour, idx) = self.active_tour.as_ref()?;
        let step = tour.steps.get(*idx)?;
        Some(format!(
            "Tour: {}\nStep: {}/{}\n{}\n{}{}\n",
            if tour.title.is_empty() {
                "Tour"
            } else {
                &tour.title
            },
            idx + 1,
            tour.steps.len(),
            self.tour_step_metadata()?,
            step.description,
            self.tour_visuals_text(),
        ))
    }

    fn tour_step_metadata(&self) -> Option<String> {
        let (tour, idx) = self.active_tour.as_ref()?;
        let step = tour.steps.get(*idx)?;
        if let Some(error) = &self.tour_ui.source_error {
            return Some(format!(
                "File: {}:{}-{}\nSource unavailable: {error}\n",
                step.file,
                step.line,
                step.end_line.unwrap_or(step.line)
            ));
        }
        let root = &self.project_root;
        let source_id = self
            .tour_highlight
            .map(|(id, _, _)| id)
            .or_else(|| self.tour_explanation.map(|focus| focus.source_buffer))
            .unwrap_or(self.buf().id);
        let source = self.buffers.iter().find(|buffer| buffer.id == source_id);
        let seq = source.map(|buffer| buffer.edit_seq).unwrap_or(0);
        if let Some((id, cached_seq, text)) = self.tour_visuals.metadata.borrow().as_ref() {
            if (*id, *cached_seq) == (source_id, seq) {
                return Some(text.clone());
            }
        }
        let path = source
            .and_then(|buffer| buffer.path.clone())
            .unwrap_or_else(|| root.join(&step.file));
        let (start, end) = match self.tour_highlight {
            Some((buffer, start, end)) if buffer == source_id => (start, end),
            _ => (
                step.line.saturating_sub(1),
                step.end_line.unwrap_or(step.line).saturating_sub(1),
            ),
        };
        let relative = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
        let mut text = format!("File: {}:{}-{}\n", relative, start + 1, end + 1);
        if source.is_none_or(|buffer| !buffer.is_modified()) {
            if let Some(url) = tour_github_permalink(root, &path, &relative, start, end) {
                text.push_str(&format!("GitHub: {url}\n"));
            }
        }
        *self.tour_visuals.metadata.borrow_mut() = Some((source_id, seq, text.clone()));
        Some(text)
    }

    /// Jump to a specific step of the active tour (from the `:toursteps` picker).
    pub fn tour_goto(&mut self, idx: usize) {
        let ok = match &mut self.active_tour {
            Some((tour, i)) if idx < tour.steps.len() => {
                *i = idx;
                true
            }
            Some(_) => false,
            None => {
                self.set_message("No active tour");
                false
            }
        };
        if ok {
            self.goto_tour_step(true);
        }
    }

    /// `:toursteps`: a picker of the active tour's steps; Enter jumps to one.
    pub fn list_tour_steps(&mut self) {
        let Some((tour, cur)) = &self.active_tour else {
            self.set_message("No active tour — :tour to start one");
            return;
        };
        let cur = *cur;
        let root = self.project_root.clone();
        let title = format!(
            "Tour: {} — Enter jumps to a step",
            if tour.title.is_empty() {
                "steps"
            } else {
                tour.title.as_str()
            }
        );
        let entries: Vec<Entry> = tour
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let marker = if i == cur { "▶" } else { " " };
                let text = format!(
                    "{marker} [{}] {}  · {}:{}–{}",
                    i + 1,
                    crate::tour_ui::summary(s),
                    s.file,
                    s.line,
                    s.end_line.unwrap_or(s.line)
                );
                // `Entry::location` (not `::text`) so the Results preview
                // pane (`p`) can show the step's source; `no_path_prefix`
                // keeps the display text as-is instead of a `path:line:col`
                // prefix. `.action` still wins on Enter (open_result checks
                // it before ever touching path/line), same as `hunks()`.
                let mut e = Entry::location(root.join(&s.file), s.line.saturating_sub(1), 0, text);
                e.no_path_prefix = true;
                e.detail = s.description.clone();
                e.action = Some(serde_json::json!({ "_vaayu_tour_goto": i }));
                e
            })
            .collect();
        let mut results = Results::new(title, entries);
        results.cursor = cur;
        self.show_results(results);
    }

    /// `:tourexplain` / `,tx`: send the current step's highlighted code (plus its
    /// note) to the configured AI sidebar for a deeper walkthrough on demand.
    pub fn tour_explain(&mut self) {
        if self.tour_ui.source_error.is_some() {
            self.set_message("Source unavailable · :tourretry or :touredit before explaining");
            return;
        }
        let desc = match &self.active_tour {
            Some((tour, idx)) => tour
                .steps
                .get(*idx)
                .map(|s| s.description.clone())
                .unwrap_or_default(),
            None => {
                self.set_message("No active tour — :tour to start one");
                return;
            }
        };
        let (bid, start, end) =
            self.tour_highlight
                .unwrap_or((self.buf().id, self.cursor().0, self.cursor().0));
        let Some(source) = self.buffers.iter().find(|buffer| buffer.id == bid) else {
            self.set_message("Tour source buffer is unavailable");
            return;
        };
        let end = end.min(source.line_count().saturating_sub(1));
        let rel = source
            .path
            .as_ref()
            .map(|p| {
                p.strip_prefix(&self.project_root)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| "(unsaved buffer)".into());
        let mut code = String::new();
        for l in start..=end {
            code.push_str(&source.line_text(l));
            code.push('\n');
        }
        let prompt = format!(
            "Explain this code from a guided tour step.\n\nTour note: {desc}\n\n\
             File: {rel}:{}-{}\n```\n{code}```\n\nStructured tour indicators:\n{}\n\n\
             Walk me through what it does and why. Use the indicators as context; \
             verify relationships against the code. Illustrative diffs are examples, not historical changes.",
            start + 1,
            end + 1,
            self.active_tour.as_ref().and_then(|(tour, idx)| tour.steps.get(*idx))
                .and_then(|s| s.vaayu.as_ref()).and_then(|v| serde_json::to_string_pretty(v).ok())
                .unwrap_or_else(|| "None".into())
        );
        if self.send_to_ai_sidebar(&prompt) {
            self.set_message(format!(
                "Sent this tour step to {} (press Enter in the sidebar)",
                self.config.ai_agent_label()
            ));
        }
    }

    /// `:tour [name]`: start the named tour. A bare `:tour` resumes the last
    /// tour+step visited (`last_tour`, restored from shada across restarts)
    /// if that tour file still exists, else falls back to the alphabetically
    /// first `.tours/*.tour`.
    pub fn start_tour(&mut self, name: &str) {
        self.start_tour_at(name, 0);
    }

    pub(crate) fn start_tour_at(&mut self, name: &str, requested_index: usize) {
        let name = name.trim();
        let resuming = name.is_empty();
        let path = if name.is_empty() {
            self.last_tour
                .as_ref()
                .map(|(stem, _)| self.tours_dir().join(format!("{stem}.tour")))
                .filter(|p| p.is_file())
                .or_else(|| {
                    std::fs::read_dir(self.tours_dir()).ok().and_then(|rd| {
                        let mut files: Vec<_> = rd
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| p.extension().is_some_and(|e| e == "tour"))
                            .collect();
                        files.sort();
                        files.into_iter().next()
                    })
                })
        } else if name.contains('/') || name.contains('\\') || name.contains("..") {
            // Keep the name inside `.tours/`; don't let it escape the directory.
            self.set_message("Invalid tour name");
            return;
        } else {
            Some(self.tours_dir().join(format!("{name}.tour")))
        };
        let Some(path) = path else {
            self.set_message("No tours found in .tours/*.tour");
            return;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.set_message(format!(
                    "Tour file unavailable: {}: {error} · :tours to choose / create",
                    path.display()
                ));
                return;
            }
        };
        let tour = match serde_json::from_str::<Tour>(&text) {
            Ok(tour) if !tour.steps.is_empty() => tour,
            Ok(_) => {
                self.set_message(format!(
                    "Tour has no steps: {} · :tours, then e to add steps",
                    path.display()
                ));
                return;
            }
            Err(error) => {
                self.set_message(format!(
                    "Invalid tour JSON at {}:{}:{}: {error} · :tours, then e to repair",
                    path.display(),
                    error.line(),
                    error.column()
                ));
                return;
            }
        };
        let start_idx = match &self.last_tour {
            Some((last_stem, idx)) if resuming && *last_stem == stem => {
                (*idx).min(tour.steps.len() - 1)
            }
            _ => requested_index.min(tour.steps.len() - 1),
        };
        self.active_tour_name = Some(stem);
        self.tour_visuals.clear_anchors();
        self.active_tour = Some((tour, start_idx));
        self.goto_tour_step(true);
    }

    /// `:tournext` / `:tourprev`: move to the next/previous step.
    pub fn tour_step(&mut self, forward: bool) {
        let Some((tour, idx)) = &mut self.active_tour else {
            self.set_message("No active tour — :tour to start one");
            return;
        };
        let last = tour.steps.len().saturating_sub(1);
        if forward && *idx < last {
            *idx += 1;
        } else if !forward && *idx > 0 {
            *idx -= 1;
        } else {
            self.tour_ui.completed = forward;
            self.set_message(if forward {
                "Tour complete · ,ve Finish · ,vr Restart · ,vs Choose tour"
            } else {
                "Start of tour · ]t Next · ,vo Choose step"
            });
            return;
        }
        self.goto_tour_step(true);
    }

    pub(crate) fn goto_tour_step(&mut self, replace_lesson: bool) {
        self.restore_tour_source_view();
        self.tour_visuals.clear_step();
        self.tour_ui.source_error = None;
        self.tour_ui.completed = false;
        let Some((tour, idx)) = &self.active_tour else {
            return;
        };
        let (idx, total) = (*idx, tour.steps.len());
        let location = tour.steps[idx].location();
        // Copy only lightweight anchors. Other steps may contain large
        // illustrative comparisons that navigation never needs to clone.
        let others: Vec<_> = tour
            .steps
            .iter()
            .enumerate()
            .filter(|(i, other)| *i != idx && other.file == location.file)
            .map(|(_, other)| other.location())
            .collect();
        let focused_explanation = self.tour_explanation.and_then(|focus| {
            (self.buf().id == focus.explanation_buffer).then_some(focus.explanation_buffer)
        });
        if let Some(focus) = self.tour_explanation {
            self.focus_tour_source(focus);
        }
        if !location.file.is_empty() {
            let result = crate::tour_visuals::project_path(&self.project_root, &location.file)
                .and_then(|path| {
                    if !path.is_file() {
                        return Err(format!("Source file not found: {}", location.file));
                    }
                    self.open_file(path)
                        .map_err(|e| format!("Could not load {}: {e}", location.file))
                });
            if let Err(error) = result {
                self.tour_ui.source_error = Some(error.clone());
                self.tour_highlight = None;
                self.tour_markers = None;
                if let Some(pane) = self.ensure_tour_explanation() {
                    let explanation = self.tour_explanation_buffer_text();
                    let focus = self.tour_explanation.unwrap();
                    if replace_lesson {
                        self.replace_tour_explanation(focus.explanation_buffer, &explanation);
                    }
                    self.show_tour_preview(&format!("# Source unavailable\n\n{error}\n\nRetry with `:tourretry`, skip with `]t`, or repair with `:touredit`.\n"), vec![]);
                    if let Some(window) = self.windows.get_mut(focus.source_window) {
                        window.preview = true;
                    }
                    if focused_explanation.is_some() {
                        self.focus_window(pane);
                    }
                }
                if let Some(name) = &self.active_tour_name {
                    self.last_tour = Some((name.clone(), idx));
                }
                self.set_message(error);
                return;
            }
        }
        let last = self.buf().line_count().saturating_sub(1);
        let bid = self.buf().id;
        let locations: Vec<_> = std::iter::once(location.clone()).chain(others).collect();
        let resolved_locations = self.resolve_tour_locations(bid, &locations);
        let resolved = &resolved_locations[0];
        let (start, end) = (resolved.start.min(last), resolved.end.min(last));
        self.set_cursor(start, 0);
        // Bias the viewport so the step's line sits near the top, not at the
        // bottom where the tour panel would cover it.
        self.buf_mut().top_line = start.saturating_sub(3);
        self.buf_mut().top_wrap = 0;
        self.tour_highlight = Some((bid, start, end));
        // Other steps of this tour that resolve into the same file (matched
        // by the raw `file` string, same as how this step's own file opened
        // the buffer -- good enough since a tour author writes it the same
        // way for every step in one file), anchored the same pattern-or-line
        // way, for the gutter marker.
        let mut other_lines: Vec<usize> = resolved_locations
            .iter()
            .skip(1)
            .map(|other| other.start.min(last))
            .collect();
        other_lines.sort_unstable();
        other_lines.dedup();
        self.tour_markers = Some((bid, other_lines));
        self.resolve_tour_decorations();
        if let Some(pane) = self.ensure_tour_explanation() {
            let focus = self.tour_explanation.unwrap();
            let explanation = self.tour_explanation_buffer_text();
            if replace_lesson {
                self.replace_tour_explanation(focus.explanation_buffer, &explanation);
            }
            if focused_explanation.is_some() {
                self.focus_window(pane);
            }
        }
        if let Some(name) = &self.active_tour_name {
            self.last_tour = Some((name.clone(), idx));
        }
        let anchor_note = resolved
            .warning
            .as_ref()
            .map(|w| format!("  ({w})"))
            .unwrap_or_default();
        self.set_message(format!(
            "[{}/{}] {}{}  (]t/[t · ]q end · [q restart · K explanation)",
            idx + 1,
            total,
            crate::tour_ui::summary(&self.active_tour.as_ref().unwrap().0.steps[idx]),
            anchor_note
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        let mut ed = Editor::new(crate::config::Config {
            clipboard_unnamedplus: false,
            watch: false,
            ..crate::config::Config::default()
        });
        ed.tour_visuals.markdown = false;
        ed.buf_mut().rope = ropey::Rope::from_str(&"source line\n".repeat(200));
        ed.buf_mut().mark_saved();
        ed.active_tour = Some((
            Tour {
                title: "Focus".into(),
                steps: vec![
                    TourStep {
                        file: String::new(),
                        line: 40,
                        end_line: None,
                        pattern: None,
                        description: "explanation line\n".repeat(100),
                        vaayu: None,
                    },
                    TourStep {
                        file: String::new(),
                        line: 150,
                        end_line: None,
                        pattern: None,
                        description: "Next explanation".into(),
                        vaayu: None,
                    },
                ],
            },
            0,
        ));
        ed.tour_goto(0);
        ed
    }

    #[test]
    fn k_reuses_the_bottom_split_and_preserves_both_views_and_edits() {
        let mut ed = editor();
        let source = ed.buf().id;
        let focus = ed.tour_explanation.unwrap();
        let buffers = ed.buffers.len();
        let source_text = ed.buf().rope.to_string();
        ed.set_cursor(45, 3);
        ed.buf_mut().top_line = 42;
        ed.toggle_tour_explanation();
        assert_eq!(ed.active_window, 1);
        assert_eq!(ed.windows[0].buffer, source);
        assert_eq!(ed.buf().id, focus.explanation_buffer);
        ed.buf_mut()
            .replace_scratch_text("edited explanation\nsecond line\n");
        ed.set_cursor(1, 2);
        ed.buf_mut().top_line = 1;
        ed.toggle_tour_explanation();
        assert_eq!(ed.active_window, focus.source_window);
        assert_eq!(ed.cursor(), (45, 3));
        assert_eq!(ed.buf().top_line, 42);
        assert_eq!(ed.buf().rope.to_string(), source_text);
        ed.toggle_tour_explanation();
        assert_eq!(ed.cursor(), (1, 2));
        assert_eq!(ed.buf().top_line, 1);
        assert!(ed.buf().rope.to_string().contains("edited explanation"));
        assert_eq!(ed.buffers.len(), buffers);
        assert_eq!(ed.windows.len(), 2);
    }

    #[test]
    fn step_changes_keep_explanation_focus_and_the_dragged_split_size() {
        let mut ed = editor();
        let focus = ed.tour_explanation.unwrap();
        let buffers = ed.buffers.len();
        assert!(ed.drag_divider_to(&[], 120, 40, 10, 18));
        let divider = ed.pane_rects(120, 40)[1].y;
        ed.toggle_tour_explanation();
        ed.set_cursor(90, 0);
        ed.buf_mut().top_line = 88;
        ed.tour_step(true);
        assert_eq!(ed.buf().id, focus.explanation_buffer);
        assert_eq!(ed.cursor(), (0, 0));
        assert_eq!(ed.buf().top_line, 0);
        assert!(ed.buf().rope.to_string().contains("Next explanation"));
        assert_eq!(ed.windows[focus.source_window].cursor, (149, 0));
        assert_eq!(ed.pane_rects(120, 40)[1].y, divider);
        assert_eq!(ed.buffers.len(), buffers);
        assert_eq!(ed.windows.len(), 2);
        ed.tour_end();
        assert_eq!(ed.buf().id, focus.source_buffer);
        assert_eq!(ed.cursor(), (149, 0));
        assert!(ed.windows.is_empty());
        assert!(ed.window_layout.is_none());
    }

    #[test]
    fn closed_explanation_pane_reopens_with_the_same_buffer() {
        let mut ed = editor();
        let focus = ed.tour_explanation.unwrap();
        let buffers = ed.buffers.len();
        ed.toggle_tour_explanation();
        ed.close_window();
        ed.toggle_tour_explanation();
        assert_eq!(ed.buf().id, focus.explanation_buffer);
        assert_eq!(ed.buffers.len(), buffers);
        assert_eq!(ed.windows.len(), 2);
    }

    #[test]
    fn ending_a_tour_restores_existing_splits_and_the_original_source_pane() {
        let mut ed = editor();
        let tour = ed.active_tour.clone();
        ed.tour_end();
        ed.split_window(true, false);
        ed.split_window(false, false);
        let source_window = ed.active_window;
        let original_layout = serde_json::to_string(&ed.window_layout).unwrap();
        let original_windows = ed.windows.len();
        ed.active_tour = tour;
        ed.tour_goto(0);
        let focus = ed.tour_explanation.unwrap();
        assert_eq!(focus.source_window, source_window);
        ed.toggle_tour_explanation();
        ed.toggle_tour_explanation();
        assert_eq!(
            ed.active_window, source_window,
            "K returns to the same pane even when the source has multiple splits"
        );
        ed.toggle_tour_explanation();
        ed.tour_end();
        assert_eq!(ed.active_window, source_window);
        assert_eq!(ed.windows.len(), original_windows);
        assert_eq!(
            serde_json::to_string(&ed.window_layout).unwrap(),
            original_layout
        );
    }

    #[test]
    fn tour_markdown_toggle_preserves_raw_scratch_edits_and_both_views() {
        use crate::key::Key;
        let mut ed = editor();
        let source = ed.buf().id;
        ed.set_cursor(45, 3);
        ed.buf_mut().top_line = 42;
        ed.toggle_tour_explanation();
        let explanation = ed.buf().id;
        let markdown = "# Heading\n\n**bold** and `inline_token`\n\n```rust\nlet x = 1;\n```\n";
        ed.buf_mut().replace_scratch_text(markdown);
        ed.set_cursor(2, 5);
        ed.buf_mut().top_line = 1;
        let seq = ed.buf().edit_seq;
        for key in ",vm".chars() {
            ed.feed_key(Key::Char(key));
        }
        assert!(ed.windows[ed.active_window].preview);
        assert_eq!(ed.buf().id, explanation);
        assert_eq!(ed.buf().edit_seq, seq);
        assert_eq!(ed.cursor(), (2, 5));
        assert_eq!(ed.buf().top_line, 1);
        ed.feed_key(Key::Char('K'));
        assert_eq!(ed.buf().id, source);
        assert_eq!(ed.cursor(), (45, 3));
        assert_eq!(ed.buf().top_line, 42);
        ed.feed_key(Key::Char('K'));
        assert_eq!(ed.buf().id, explanation);
        ed.feed_key(Key::Esc);
        assert!(!ed.windows[ed.active_window].preview);
        assert_eq!(ed.cursor(), (2, 5));
        assert_eq!(ed.buf().rope.to_string(), markdown);
        // The existing Markdown shortcut works in the pathless description.
        for key in ",mp,mp".chars() {
            ed.feed_key(Key::Char(key));
        }
        assert!(!ed.tour_visuals.markdown);
        assert_eq!(ed.buf().edit_seq, seq);
        crate::command::run_ex(&mut ed, "tourmarkdown on");
        crate::command::run_ex(&mut ed, "tourmarkdown on");
        assert!(ed.tour_visuals.markdown);
        crate::command::run_ex(&mut ed, "tourmarkdown off");
        assert_eq!(ed.buf().rope.to_string(), markdown);
        assert_eq!(ed.windows.len(), 2);
    }

    #[test]
    fn rendered_tour_description_keeps_navigation_reuse_and_cleanup() {
        use crate::key::Key;
        let mut ed = editor();
        let source = ed.buf().id;
        let buffers = ed.buffers.len();
        ed.tour_markdown("on");
        assert_eq!(ed.buf().id, source);
        ed.toggle_tour_explanation();
        let explanation = ed.buf().id;
        ed.windows[ed.active_window].preview_scroll = 17;
        for key in "]t".chars() {
            ed.feed_key(Key::Char(key));
        }
        assert_eq!(ed.active_tour.as_ref().unwrap().1, 1);
        assert_eq!(ed.buf().id, explanation);
        assert!(ed.windows[ed.active_window].preview);
        assert_eq!(ed.windows[ed.active_window].preview_scroll, 0);
        assert!(ed.buf().rope.to_string().contains("Next explanation"));
        assert_eq!(ed.buffers.len(), buffers);
        ed.feed_key(Key::Char('q'));
        ed.toggle_tour_explanation();
        assert_eq!(ed.buf().id, explanation);
        assert!(ed.windows[ed.active_window].preview);
        assert_eq!(ed.buffers.len(), buffers);
        for key in "]q".chars() {
            ed.feed_key(Key::Char(key));
        }
        assert!(ed.active_tour.is_none());
        assert_eq!(ed.buf().id, source);
        assert!(ed.windows.is_empty());
        ed.tour_markdown("on");
        assert_eq!(ed.message, "No active tour");
    }

    #[test]
    fn tour_description_keeps_original_markdown_for_rendering_and_copy() {
        let mut ed = editor();
        let description = "# Heading\n\n**bold** and `inline_token`\n\n```rust\nlet x = 1;\n```";
        ed.active_tour.as_mut().unwrap().0.steps[0].description = description.into();
        ed.tour_goto(0);
        let explanation = ed.tour_explanation.unwrap().explanation_buffer;
        let buffer = ed.buffers.iter().find(|b| b.id == explanation).unwrap();
        assert!(buffer.rope.to_string().contains(description));
        assert!(buffer.path.is_none());
        ed.tour_markdown("on");
        ed.copy_tour_step();
        assert!(ed
            .registers
            .get(Some('+'))
            .unwrap()
            .text
            .contains(description));
        ed.tour_markdown("off");
        assert_eq!(
            ed.active_tour.as_ref().unwrap().0.steps[0].description,
            description
        );
    }
    #[test]
    fn tour_markdown_reuses_styled_preview_until_edits_or_resize() {
        let mut ed = editor();
        ed.active_tour.as_mut().unwrap().0.steps[0].description =
            "# Heading\n\n**bold_token** and `inline_token`\n\n```rust\nlet value = 1;\n```".into();
        ed.tour_goto(0);
        ed.tour_markdown("on");
        let explanation = ed.tour_explanation.unwrap().explanation_buffer;
        let key = (explanation, 98, 100);
        let mut frame = crate::render::FrameCache::new();
        let mut out = Vec::new();
        crate::render::prepare_view(&mut ed, 100, 30);
        crate::render::draw(&mut out, &ed, 100, 30, &mut frame).unwrap();
        let pointer = {
            let previews = ed.preview_panes.borrow();
            let preview = &previews[&key];
            let spans: Vec<_> = preview.lines.iter().flatten().collect();
            assert!(spans
                .iter()
                .any(|s| s.text.contains("Heading") && s.style.heading == 1));
            assert!(spans
                .iter()
                .any(|s| s.text.contains("bold_token") && s.style.bold));
            assert!(spans
                .iter()
                .any(|s| s.text.contains("inline_token") && s.style.inline_code));
            assert!(spans
                .iter()
                .any(|s| s.style.code_block && s.style.syntax.is_some()));
            preview.lines.as_ptr()
        };
        for _ in 0..10 {
            ed.feed_key(crate::key::Key::Char('j'));
            crate::render::prepare_view(&mut ed, 100, 30);
            out.clear();
            crate::render::draw(&mut out, &ed, 100, 30, &mut frame).unwrap();
            assert_eq!(ed.preview_panes.borrow()[&key].lines.as_ptr(), pointer);
        }
        crate::render::prepare_view(&mut ed, 60, 30);
        crate::render::draw(&mut out, &ed, 60, 30, &mut frame).unwrap();
        assert!(ed
            .preview_panes
            .borrow()
            .contains_key(&(explanation, 58, 100)));
        ed.buffers
            .iter_mut()
            .find(|b| b.id == explanation)
            .unwrap()
            .insert_str(0, 0, "# Changed heading\n\n");
        crate::render::draw(&mut out, &ed, 60, 30, &mut frame).unwrap();
        assert!(ed.preview_panes.borrow()[&(explanation, 58, 100)]
            .lines
            .iter()
            .flatten()
            .any(|span| span.text.contains("Changed heading") && span.style.heading == 1));
    }

    #[test]
    fn github_permalink_paths_escape_url_special_characters() {
        assert_eq!(encode_github_path("src/a b#é.rs"), "src/a%20b%23%C3%A9.rs");
    }
}
