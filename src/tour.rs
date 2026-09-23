//! `.tours/*.tour` code tours (VS Code CodeTour-compatible JSON). `:tours`
//! lists the project's tours; `:tour [name]` starts one; `:tournext`/
//! `:tourprev` step through, jumping to each step and showing its description.

use crate::editor::Editor;
use crate::results::{Entry, Results};
use serde::Deserialize;
use std::path::PathBuf;

fn one() -> usize {
    1
}

#[derive(Deserialize, Clone)]
pub struct TourStep {
    #[serde(default)]
    pub file: String,
    #[serde(default = "one")]
    pub line: usize,
    #[serde(default)]
    pub description: String,
}

#[derive(Deserialize, Clone, Default)]
pub struct Tour {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub steps: Vec<TourStep>,
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
