//! `.tours/*.tour` code tours (VS Code CodeTour-compatible JSON). `:tours`
//! lists the project's tours; `:tour [name]` starts one; `:tournext`/
//! `:tourprev` step through, jumping to each step and showing its description.

use crate::editor::Editor;
use crate::results::{Entry, Results};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn one() -> usize {
    1
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
        self.show_results(Results::new("Tours — Enter starts one", entries));
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
        self.set_message("Describe the tour (i to edit), then :toursave to generate it with Claude");
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
        self.tour_highlight = None;
        if self.active_tour.take().is_some() {
            self.set_message("Tour ended");
        } else {
            self.set_message("No active tour");
        }
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
                let mut e = Entry::text(format!("{marker} [{}] {}", i + 1, s.description));
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

    /// `:tour [name]`: start the named tour (or the first one) at step 1.
    pub fn start_tour(&mut self, name: &str) {
        let name = name.trim();
        let path = if name.is_empty() {
            std::fs::read_dir(self.tours_dir())
                .ok()
                .and_then(|rd| {
                    let mut files: Vec<_> = rd
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "tour"))
                        .collect();
                    files.sort();
                    files.into_iter().next()
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
        self.active_tour = Some((tour, 0));
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
        let Some((tour, idx)) = &self.active_tour else {
            return;
        };
        let (idx, total) = (*idx, tour.steps.len());
        let step = tour.steps[idx].clone();
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
        if let Some(pat) = step.pattern.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            for l in 0..=last {
                if self.buf().line_text(l).contains(pat) {
                    anchored = Some(l);
                    break;
                }
            }
        }
        let start = anchored.unwrap_or_else(|| step.line.saturating_sub(1)).min(last);
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
        let anchor_note = if step.pattern.is_some() && anchored.is_none() {
            "  (anchor not found)"
        } else {
            ""
        };
        self.set_message(format!(
            "[{}/{}] {}{}  (]t/[t · :tourend)",
            idx + 1,
            total,
            step.description,
            anchor_note
        ));
    }
}
