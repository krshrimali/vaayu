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

/// A filesystem-safe file stem from a user-supplied tour name.
fn slugify(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "tour".to_string()
    } else {
        s
    }
}

/// Parse the friendly `:tournew` draft buffer into a `Tour`. Blank and `#`
/// comment lines are ignored; a `Title:` line sets the title; every other line
/// is a step `"<path>:<line>  <description>"` (line defaults to 1, description
/// optional).
fn parse_tour_draft(text: &str, default_title: &str) -> Tour {
    let mut title = default_title.to_string();
    let mut steps = Vec::new();
    for raw in text.lines() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t
            .strip_prefix("Title:")
            .or_else(|| t.strip_prefix("title:"))
        {
            let r = rest.trim();
            if !r.is_empty() {
                title = r.to_string();
            }
            continue;
        }
        let (loc, desc) = match t.split_once(char::is_whitespace) {
            Some((a, b)) => (a, b.trim().to_string()),
            None => (t, String::new()),
        };
        let (file, line) = match loc.rsplit_once(':') {
            Some((p, n)) => (p.to_string(), n.parse::<usize>().unwrap_or(1)),
            None => (loc.to_string(), 1),
        };
        if !file.is_empty() {
            steps.push(TourStep {
                file,
                line,
                description: desc,
            });
        }
    }
    Tour { title, steps }
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
                e.action = Some(serde_json::json!({ "_vaayu_rerun_ex": format!(":tour {stem}") }));
                entries.push(e);
            }
        }
        if entries.is_empty() {
            self.set_message("No tours found in .tours/*.tour");
            return;
        }
        self.show_results(Results::new("Tours — Enter starts one", entries));
    }

    /// `:tournew [name]`: open a scratch buffer pre-filled with a friendly
    /// template; the user fills in steps and runs `:toursave` to write the
    /// `.tour` file -- no hand-written JSON. The current file/cursor seeds a
    /// first step.
    pub fn tour_new(&mut self, name: &str) {
        let name = name.trim();
        let title = if name.is_empty() { "My tour" } else { name };
        let slug = slugify(name);
        // Seed a first step from the current file/cursor (if it's a real file).
        let seed = self
            .buf()
            .path
            .as_ref()
            .map(|p| {
                let rel = p
                    .strip_prefix(&self.project_root)
                    .unwrap_or(p)
                    .display()
                    .to_string();
                format!("{}:{}  ", rel, self.cursor().0 + 1)
            })
            .unwrap_or_default();
        let template = format!(
            "# New code tour -- one step per line, then run :toursave\n\
             # Step format:   <path>:<line>  <description>\n\
             #   <path> is relative to the project root; <line> is 1-based.\n\
             #   Lines starting with # are ignored.\n\
             Title: {title}\n\
             {seed}\n"
        );
        let mut b = crate::buffer::Buffer::empty();
        b.rope = ropey::Rope::from_str(&template);
        b.mark_saved();
        self.buffers.push(b);
        self.cur = self.buffers.len() - 1;
        let id = self.buf().id;
        self.tour_draft = Some((slug, id));
        self.invalidate_index_caches();
        // Park the cursor at the end of the seeded step line, ready to type.
        let sl = self.buf().line_count().saturating_sub(1);
        let sc = self.buf().line_len(sl);
        self.set_cursor(sl, sc);
        self.enter_normal();
        self.set_message("New tour: add `path:line  description` steps, then :toursave (i to edit)");
    }

    /// `:toursave`: parse the `:tournew` draft buffer and write it to
    /// `.tours/<slug>.tour` (VS Code CodeTour-compatible JSON).
    pub fn tour_save(&mut self) {
        let Some((slug, id)) = self.tour_draft.clone() else {
            self.set_message("No tour draft — run :tournew first");
            return;
        };
        let text = self
            .buffers
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.rope.to_string())
            .unwrap_or_else(|| self.buf().rope.to_string());
        let tour = parse_tour_draft(&text, &slug);
        if tour.steps.is_empty() {
            self.set_message("Tour has no steps — add `path:line  description` lines, then :toursave");
            return;
        }
        let count = tour.steps.len();
        let dir = self.tours_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.set_message(format!("Could not create .tours: {e}"));
            return;
        }
        let json = match serde_json::to_vec_pretty(&tour) {
            Ok(j) => j,
            Err(e) => {
                self.set_message(format!("Could not serialize tour: {e}"));
                return;
            }
        };
        // Tours are meant to be committed/shared, so not a private write.
        match crate::files::atomic_write(&dir.join(format!("{slug}.tour")), &json, false) {
            Ok(()) => self.set_message(format!(
                "Saved {count} step(s) to .tours/{slug}.tour — :tour {slug} to run it"
            )),
            Err(e) => self.set_message(format!("Could not write tour: {e}")),
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
        self.set_cursor(step.line.saturating_sub(1), 0);
        self.set_message(format!(
            "[{}/{}] {}  (:tournext / :tourprev)",
            idx + 1,
            total,
            step.description
        ));
    }
}
