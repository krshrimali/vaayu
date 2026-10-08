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
    #[serde(default, alias = "endLine", skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    /// Optional anchor text (VS Code CodeTour-compatible): if present, the step
    /// re-locates to the first line containing it, so the tour survives edits
    /// that shift line numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default)]
    pub description: String,
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
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.tours_dir()) {
            let mut files: Vec<_> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "tour"))
                .collect();
            files.sort();
            for p in files {
                let stem = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let title = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Tour>(&t).ok())
                    .map(|tour| tour.title)
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| stem.clone());
                let mut e = Entry::text(format!("{title}  ({stem})"));
                // `_vaayu_rerun_ex` is passed straight to `run_ex`, which does
                // not strip a leading `:`, so store the bare command.
                e.action = Some(serde_json::json!({ "_vaayu_rerun_ex": format!("tour {stem}") }));
                entries.push(e);
            }
        }
        if entries.is_empty() {
            self.set_message("No tours found in .tours/*.tour");
            return;
        }
        self.show_results(Results::new(
            "Tours — Enter starts · K explanation",
            entries,
        ));
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
            .and_then(|command| command.strip_prefix("tour "))
            .map(str::to_string);
        let Some(name) = name else {
            self.set_message("Select a tour first");
            return;
        };
        self.start_tour(&name);
        if self.active_tour.is_some() {
            self.toggle_tour_explanation();
        }
    }

    /// `:tournew [name]`: open a scratch buffer for a plain-English description
    /// of the tour you want. `:toursave` then hands that prompt to the Claude
    /// sidebar, which explores the repo and writes the concrete `.tour` file --
    /// the user supplies only the prompt.
    pub fn tour_new(&mut self, name: &str) {
        let template = "# Describe the code tour you want in plain English, then run :toursave\n\
             # to have Claude explore the repo and generate it into .tours/*.tour.\n\
             #\n\
             # e.g. \"Walk a new contributor through how a keypress becomes a screen\n\
             #       update: input handling, the editor state update, then rendering.\"\n\
             #\n\
             # Lines starting with # are ignored.\n\n";
        let mut b = crate::buffer::Buffer::empty();
        b.rope = ropey::Rope::from_str(template);
        b.mark_saved();
        self.buffers.push(b);
        self.cur = self.buffers.len() - 1;
        let id = self.buf().id;
        self.tour_draft = Some((name.trim().to_string(), id));
        self.invalidate_index_caches();
        let last = self.buf().line_count().saturating_sub(1);
        self.set_cursor(last, 0);
        self.enter_normal();
        self.set_message(
            "Describe the tour (i to edit), then :toursave to generate it with Claude",
        );
    }

    /// `:toursave`: send the `:tournew` prompt to the Claude sidebar with
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
             needed) as JSON with EXACTLY this schema:\n\
             {{\"title\": \"<short title>\", \"steps\": [{{\
             \"file\": \"<repo-relative path>\", \
             \"line\": <1-based start line>, \
             \"endLine\": <1-based end line of the region to highlight>, \
             \"pattern\": \"<a short, unique substring from the start line so the step \
             survives edits>\", \
             \"description\": \"<clear explanation of this step>\"}}]}}\n\
             Use real files and line numbers from this repo. Set endLine to cover the \
             whole relevant construct (function/block), not just one line, and make \
             pattern a short exact substring of the start line. Order as a 5-12 step \
             narrative. Create only that one file and print nothing else."
        );
        if !self.send_to_ai_sidebar(&instruction) {
            return; // the CLI couldn't be started; message already set
        }
        self.set_message(format!(
            "Sent to Claude — press Enter in the sidebar; when it finishes: :tour {slug}"
        ));
    }

    /// `:tourend`: stop the active tour (dismisses the step panel).
    pub fn tour_end(&mut self) {
        self.leave_tour_explanation();
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

    /// Toggle between the current source buffer and a normal, navigable
    /// scratch buffer containing the current tour step's explanation.
    pub fn toggle_tour_explanation(&mut self) {
        if self.active_tour.is_none() {
            self.request_hover();
            return;
        }
        let current = self.buf().id;
        if let Some(focus) = self.tour_explanation {
            if current == focus.explanation_buffer {
                self.switch_buffer_id(focus.source_buffer);
                self.set_message("Tour source focused · K returns to the explanation");
                return;
            }
            if self
                .buffers
                .iter()
                .any(|buffer| buffer.id == focus.explanation_buffer)
            {
                let text = self.tour_explanation_text();
                self.replace_tour_explanation(focus.explanation_buffer, &text);
                self.tour_explanation = Some(ExplanationFocus {
                    source_buffer: current,
                    explanation_buffer: focus.explanation_buffer,
                });
                self.switch_buffer_id(focus.explanation_buffer);
                self.set_message("Tour explanation focused · K returns to the source");
                return;
            }
        }
        let text = self.tour_explanation_text();
        let mut buffer = crate::buffer::Buffer::empty();
        buffer.rope = ropey::Rope::from_str(&text);
        buffer.mark_saved();
        let explanation_buffer = buffer.id;
        self.note_alternate_buffer();
        self.buffers.push(buffer);
        self.invalidate_index_caches();
        self.tour_explanation = Some(ExplanationFocus {
            source_buffer: current,
            explanation_buffer,
        });
        self.switch_buffer_id(explanation_buffer);
        self.set_message(
            "Tour explanation focused · Vim motions and editing are available · K returns",
        );
    }

    /// Copy the active step's explanation and source metadata to `+`.
    pub fn copy_tour_step(&mut self) {
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

    fn switch_buffer_id(&mut self, id: u64) {
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
                self.switch_buffer_id(focus.source_buffer);
            }
        }
    }

    fn replace_tour_explanation(&mut self, id: u64, text: &str) {
        if let Some(buffer) = self.buffers.iter_mut().find(|buffer| buffer.id == id) {
            buffer.replace_scratch_text(text);
        }
        self.invalidate_index_caches();
    }

    fn tour_explanation_text(&self) -> String {
        self.tour_explanation_text_with_link()
            .unwrap_or_else(|| "No active tour step".into())
    }

    fn tour_explanation_text_with_link(&self) -> Option<String> {
        let (tour, idx) = self.active_tour.as_ref()?;
        let step = tour.steps.get(*idx)?;
        let root = &self.project_root;
        let source_id = self
            .tour_explanation
            .filter(|focus| self.buf().id == focus.explanation_buffer)
            .map(|focus| focus.source_buffer)
            .unwrap_or(self.buf().id);
        let source = self.buffers.iter().find(|buffer| buffer.id == source_id);
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
        let mut text = format!(
            "Tour: {}\nStep: {}/{}\nFile: {}:{}-{}\n",
            if tour.title.is_empty() {
                "Tour"
            } else {
                &tour.title
            },
            idx + 1,
            tour.steps.len(),
            relative,
            start + 1,
            end + 1
        );
        if let Some(url) = tour_github_permalink(root, &path, &relative, start, end) {
            text.push_str(&format!("GitHub: {url}\n"));
        }
        text.push_str(&format!("\n{}\n", step.description));
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
            self.goto_tour_step();
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
                let text = format!("{marker} [{}] {}", i + 1, s.description);
                // `Entry::location` (not `::text`) so the Results preview
                // pane (`p`) can show the step's source; `no_path_prefix`
                // keeps the display text as-is instead of a `path:line:col`
                // prefix. `.action` still wins on Enter (open_result checks
                // it before ever touching path/line), same as `hunks()`.
                let mut e = Entry::location(root.join(&s.file), s.line.saturating_sub(1), 0, text);
                e.no_path_prefix = true;
                e.action = Some(serde_json::json!({ "_vaayu_tour_goto": i }));
                e
            })
            .collect();
        self.show_results(Results::new(title, entries));
    }

    /// `:tourexplain` / `,tx`: send the current step's highlighted code (plus its
    /// note) to the Claude sidebar for a deeper walkthrough on demand.
    pub fn tour_explain(&mut self) {
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
        let bid = self.buf().id;
        let (start, end) = match self.tour_highlight {
            Some((b, s, e)) if b == bid => (s, e),
            _ => {
                let l = self.cursor().0;
                (l, l)
            }
        };
        let end = end.min(self.buf().line_count().saturating_sub(1));
        let rel = self
            .buf()
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
            code.push_str(&self.buf().line_text(l));
            code.push('\n');
        }
        let prompt = format!(
            "Explain this code from a guided tour step.\n\nTour note: {desc}\n\n\
             File: {rel}:{}-{}\n```\n{code}```\n\nWalk me through what it does and why.",
            start + 1,
            end + 1
        );
        if self.send_to_ai_sidebar(&prompt) {
            self.set_message("Sent this tour step to Claude (press Enter in the sidebar)");
        } else {
            self.set_message("Could not start Claude");
        }
    }

    /// `:tour [name]`: start the named tour. A bare `:tour` resumes the last
    /// tour+step visited (`last_tour`, restored from shada across restarts)
    /// if that tour file still exists, else falls back to the alphabetically
    /// first `.tours/*.tour`.
    pub fn start_tour(&mut self, name: &str) {
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
        let tour = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Tour>(&t).ok())
        {
            Some(t) if !t.steps.is_empty() => t,
            Some(_) => {
                self.set_message("Tour has no steps");
                return;
            }
            None => {
                self.set_message(format!("Could not read tour: {}", path.display()));
                return;
            }
        };
        let start_idx = match &self.last_tour {
            Some((last_stem, idx)) if resuming && *last_stem == stem => {
                (*idx).min(tour.steps.len() - 1)
            }
            _ => 0,
        };
        self.active_tour_name = Some(stem);
        self.active_tour = Some((tour, start_idx));
        self.goto_tour_step();
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
            self.set_message(if forward {
                "End of tour"
            } else {
                "Start of tour"
            });
            return;
        }
        self.goto_tour_step();
    }

    fn goto_tour_step(&mut self) {
        let Some((tour, idx)) = self.active_tour.clone() else {
            return;
        };
        let focused_explanation = self.tour_explanation.and_then(|focus| {
            (self.buf().id == focus.explanation_buffer).then_some(focus.explanation_buffer)
        });
        if focused_explanation.is_some() {
            if let Some(source) = self.tour_explanation.map(|focus| focus.source_buffer) {
                self.switch_buffer_id(source);
            }
        }
        let (idx, total) = (idx, tour.steps.len());
        let steps = tour.steps.clone();
        let step = steps[idx].clone();
        if !step.file.is_empty() {
            let path = self.project_root.join(&step.file);
            if let Err(e) = self.open_file(path) {
                self.set_message(format!("Tour step file: {e}"));
                return;
            }
        }
        let last = self.buf().line_count().saturating_sub(1);
        // Re-anchor to `pattern` if given: the first line containing it wins, so
        // the step follows its code across edits that shift line numbers.
        let mut anchored = None;
        if let Some(pat) = step
            .pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            for l in 0..=last {
                if self.buf().line_text(l).contains(pat) {
                    anchored = Some(l);
                    break;
                }
            }
        }
        let start = anchored
            .unwrap_or_else(|| step.line.saturating_sub(1))
            .min(last);
        // Highlight range: an explicit end_line, else through the anchored line.
        let end = step
            .end_line
            .map(|e| e.saturating_sub(1))
            .unwrap_or(start)
            .clamp(start, last);
        self.set_cursor(start, 0);
        // Bias the viewport so the step's line sits near the top, not at the
        // bottom where the tour panel would cover it.
        self.buf_mut().top_line = start.saturating_sub(3);
        self.buf_mut().top_wrap = 0;
        let bid = self.buf().id;
        self.tour_highlight = Some((bid, start, end));
        // Other steps of this tour that resolve into the same file (matched
        // by the raw `file` string, same as how this step's own file opened
        // the buffer -- good enough since a tour author writes it the same
        // way for every step in one file), anchored the same pattern-or-line
        // way, for the gutter marker.
        let mut other_lines: Vec<usize> = steps
            .iter()
            .enumerate()
            .filter(|(i, other)| *i != idx && other.file == step.file)
            .map(|(_, other)| {
                let pat = other
                    .pattern
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty());
                let anchored =
                    pat.and_then(|p| (0..=last).find(|&l| self.buf().line_text(l).contains(p)));
                anchored
                    .unwrap_or_else(|| other.line.saturating_sub(1))
                    .min(last)
            })
            .collect();
        other_lines.sort_unstable();
        other_lines.dedup();
        self.tour_markers = Some((bid, other_lines));
        if let Some(mut focus) = self.tour_explanation {
            focus.source_buffer = bid;
            self.tour_explanation = Some(focus);
            let explanation = self.tour_explanation_text();
            self.replace_tour_explanation(focus.explanation_buffer, &explanation);
            if focused_explanation.is_some() {
                self.switch_buffer_id(focus.explanation_buffer);
            }
        }
        if let Some(name) = &self.active_tour_name {
            self.last_tour = Some((name.clone(), idx));
        }
        let anchor_note = if step.pattern.is_some() && anchored.is_none() {
            "  (anchor not found)"
        } else {
            ""
        };
        self.set_message(format!(
            "[{}/{}] {}{}  (]t/[t · ]q end · [q restart · K explanation)",
            idx + 1,
            total,
            step.description,
            anchor_note
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::encode_github_path;

    #[test]
    fn github_permalink_paths_escape_url_special_characters() {
        assert_eq!(encode_github_path("src/a b#é.rs"), "src/a%20b%23%C3%A9.rs");
    }
}
