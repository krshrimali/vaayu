//! Tour presentation state. Authored/scratch lesson text never contains UI controls.
use crate::{
    editor::Editor,
    mode::Mode,
    results::{Entry, Results},
    tour::{Tour, TourStep},
};
use std::{path::PathBuf, sync::mpsc};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum TourLayout {
    #[default]
    Auto,
    Horizontal,
    Vertical,
}

pub struct CatalogResult {
    root: PathBuf,
    entries: Vec<Entry>,
}

#[derive(Default)]
pub struct TourUiState {
    pub source_error: Option<String>,
    pub completed: bool,
    pub layout: TourLayout,
    catalog_rx: Option<mpsc::Receiver<CatalogResult>>,
}

#[derive(Clone)]
pub struct Control {
    pub label: String,
    pub command: &'static str,
}

pub fn ellipsize(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    for g in text.graphemes(true) {
        if result.width() + g.width() + 1 > width {
            break;
        }
        result.push_str(g);
    }
    result.push('…');
    result
}

pub fn summary(step: &TourStep) -> String {
    let title = step
        .vaayu
        .as_ref()
        .map(|v| v.title.trim())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            step.description
                .lines()
                .map(str::trim)
                .find(|s| !s.is_empty())
        })
        .unwrap_or("Untitled step");
    let text: String = title
        .trim_start_matches('#')
        .trim()
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '*' | '`'))
        .collect();
    ellipsize(text.trim(), 100)
}

fn command_entry(label: impl Into<String>, command: &str) -> Entry {
    let mut entry = Entry::text(label);
    entry.action = Some(serde_json::json!({"_vaayu_rerun_ex": command}));
    entry
}

impl Editor {
    pub(crate) fn tour_chrome_rows(&self, rows: usize) -> usize {
        if self.active_tour.is_some() && rows >= 7 {
            2
        } else {
            0
        }
    }

    pub(crate) fn tour_compact(&self, rows: usize) -> bool {
        self.active_tour.is_some()
            && (rows <= 10 || (self.tour_ui.layout == TourLayout::Auto && rows <= 16))
    }

    pub fn tour_layout(&mut self, value: &str) {
        self.tour_ui.layout = match value.trim() {
            "auto" => TourLayout::Auto,
            "horizontal" => TourLayout::Horizontal,
            "vertical" => TourLayout::Vertical,
            _ => {
                self.set_message("Usage: :tourlayout auto|horizontal|vertical");
                return;
            }
        };
        self.set_message(format!("Tour layout: {}", value.trim()));
    }

    pub fn tour_retry(&mut self) {
        if self.active_tour.is_some() {
            self.goto_tour_step(false);
        } else {
            self.set_message("No active tour");
        }
    }

    pub fn tour_edit(&mut self) {
        let path = self.active_tour_name.as_ref().map(|name| {
            self.project_root
                .join(".tours")
                .join(format!("{name}.tour"))
        });
        if let Some(path) = path {
            self.tour_end();
            if let Err(error) = self.open_file(path) {
                self.set_message(format!("Could not edit tour: {error}"));
            }
        } else {
            self.set_message("Select or start a tour first");
        }
    }

    pub fn tour_resume(&mut self, name: &str) {
        if name.trim().is_empty() {
            self.start_tour("");
            return;
        }
        let index = self
            .last_tour
            .as_ref()
            .filter(|(last, _)| last == name)
            .map(|(_, i)| *i)
            .unwrap_or(0);
        self.start_tour_at(name, index);
    }

    pub fn tour_help(&mut self) {
        if self.active_tour.is_none() {
            self.show_results(Results::new(
                "Tours — getting started",
                vec![
                    command_entry("Choose a project tour (:tours)", "tours"),
                    command_entry(
                        "Describe a tour for your configured AI agent (:tournew)",
                        "tournew",
                    ),
                    command_entry(
                        "Create a minimal example in .tours/example.tour",
                        "tourexample",
                    ),
                ],
            ));
            return;
        }
        let leader = self.config.leader.clone();
        let mut entries = vec![
            command_entry("Next step  ]t", "tournext"),
            command_entry("Previous step  [t", "tourprev"),
            command_entry("Choose a step", "toursteps"),
            command_entry(
                format!(
                    "Switch Source / Lesson focus  K (now {})",
                    if self.is_tour_description(self.buf().id) {
                        "Lesson"
                    } else {
                        self.tour_visuals.view.label()
                    }
                ),
                "tourfocus",
            ),
            command_entry(
                "Read Markdown / edit raw scratch text (discarded on step change)",
                "tourmarkdown",
            ),
            command_entry(
                format!(
                    "Source view  Ctrl-O returns from Diff / Flow (view: {})",
                    self.tour_visuals.view.label()
                ),
                "tourview source",
            ),
        ];
        if let Some(visuals) = self
            .active_tour
            .as_ref()
            .and_then(|(t, i)| t.steps[*i].vaayu.as_ref())
        {
            if visuals.diff.is_some() {
                entries.push(command_entry("Diff view", "tourview diff"));
            }
            if !visuals.connections.is_empty() {
                entries.push(command_entry("Flow view", "tourview flow"));
            }
            for annotation in &visuals.annotations {
                entries.push(command_entry(
                    format!(
                        "Highlight: {}  (:tourjump {})",
                        annotation.label, annotation.id
                    ),
                    &format!("tourjump {}", annotation.id),
                ));
            }
            for (i, connection) in visuals.connections.iter().enumerate() {
                entries.push(command_entry(
                    format!("Follow: {}  (:tourfollow {})", connection.label, i + 1),
                    &format!("tourfollow {}", i + 1),
                ));
            }
        }
        entries.extend([
            command_entry(
                "Automatic layout (compact / split / side by side)",
                "tourlayout auto",
            ),
            command_entry("Horizontal layout", "tourlayout horizontal"),
            command_entry("Side by side layout", "tourlayout vertical"),
            command_entry("Check source paths and anchors", "tourcheck"),
            command_entry("Retry the current source", "tourretry"),
            command_entry("Edit tour definition", "touredit"),
            command_entry("Copy step and source metadata", "tourcopystep"),
            command_entry("Copy full tour JSON", "tourcopyall"),
            command_entry("Finish tour", "tourend"),
            command_entry("Restart from the first step", "tourrestart"),
            command_entry("Choose another tour", "tours"),
        ]);
        self.show_results(Results::new(
            format!("Tour help — Enter acts · {leader}v? help · q returns"),
            entries,
        ));
    }

    pub fn tour_example(&mut self) {
        if self.active_tour.is_some() {
            self.tour_end();
        }
        let path = self.project_root.join(".tours/example.tour");
        if !path.exists() {
            let file = self
                .buf()
                .path
                .as_ref()
                .and_then(|p| p.strip_prefix(&self.project_root).ok())
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let tour = serde_json::json!({"title":"My first tour","steps":[{"file":file,"line":1,"description":"# Start here\n\nExplain this part of the project."}]});
            let result = std::fs::create_dir_all(path.parent().unwrap()).and_then(|()| {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                file.write_all(serde_json::to_string_pretty(&tour).unwrap().as_bytes())
            });
            if let Err(error) = result {
                self.set_message(format!("Could not create example: {error}"));
                return;
            }
        }
        if let Err(error) = self.open_file(path) {
            self.set_message(error.to_string());
        } else {
            self.set_message("Edit the example, :w to save, then :tour example to start");
        }
    }

    pub(crate) fn tour_header(&self, width: usize) -> String {
        let Some((tour, idx)) = &self.active_tour else {
            return String::new();
        };
        let focus = if self.is_tour_description(self.buf().id) {
            if self.tour_visuals.markdown {
                "Lesson"
            } else {
                "Raw scratch"
            }
        } else {
            self.tour_visuals.view.label()
        };
        let progress = format!("step {}/{} · {focus}", idx + 1, tour.steps.len());
        let warning = self
            .tour_ui
            .source_error
            .as_deref()
            .or_else(|| self.tour_visuals.warnings.first().map(String::as_str));
        let detail = if let Some(warning) = warning {
            format!("! {warning}")
        } else if self.tour_ui.completed {
            "Complete · Finish / Restart / Tours".into()
        } else {
            let step = &tour.steps[*idx];
            let (start, end) = self
                .tour_highlight
                .map(|(_, a, b)| (a + 1, b + 1))
                .unwrap_or((step.line, step.end_line.unwrap_or(step.line)));
            format!("{}:{}–{} · {}", step.file, start, end, tour.title)
        };
        let room = width.saturating_sub(progress.width() + 3);
        format!(
            "{progress} · {}",
            ellipsize(&detail.replace(['\n', '\r', '\t'], " "), room)
        )
    }

    pub(crate) fn tour_controls(&self, width: usize) -> Vec<Control> {
        let Some((tour, idx)) = &self.active_tour else {
            return vec![];
        };
        let leader = &self.config.leader;
        let mut controls = if self.tour_ui.source_error.is_some() && !self.tour_ui.completed {
            vec![
                (format!("{leader}vR Retry"), "tourretry"),
                ("]t Skip".into(), "tournext"),
                (format!("{leader}vE Edit"), "touredit"),
            ]
        } else if self.tour_ui.completed || idx + 1 == tour.steps.len() {
            vec![
                (
                    format!("{leader}ve {}", if width < 80 { "End" } else { "Finish" }),
                    "tourend",
                ),
                (
                    format!(
                        "{leader}vr {}",
                        if width < 80 { "Reset" } else { "Restart" }
                    ),
                    "tourrestart",
                ),
                (format!("{leader}vs Tours"), "tours"),
            ]
        } else {
            vec![
                ("[t Prev".into(), "tourprev"),
                ("]t Next".into(), "tournext"),
                ("K Focus".into(), "tourfocus"),
            ]
        };
        controls.push((format!("{leader}v? Help"), "tourhelp"));
        if width >= 80 {
            controls.push((format!("{leader}vo Steps"), "toursteps"));
            controls.push((
                format!(
                    "{leader}vm {}",
                    if self.tour_visuals.markdown {
                        "Raw/Edit"
                    } else {
                        "Read"
                    }
                ),
                "tourmarkdown",
            ));
        }
        if width >= 115 && self.tour_ui.source_error.is_none() {
            controls.push((
                format!(
                    "{}Source",
                    if self.tour_visuals.view == crate::tour_visuals::TourView::Source {
                        "● "
                    } else {
                        ""
                    }
                ),
                "tourview source",
            ));
            if let Some(v) = &tour.steps[*idx].vaayu {
                if v.diff.is_some() {
                    controls.push((
                        format!(
                            "{}Diff",
                            if self.tour_visuals.view == crate::tour_visuals::TourView::Diff {
                                "● "
                            } else {
                                ""
                            }
                        ),
                        "tourview diff",
                    ));
                }
                if !v.connections.is_empty() {
                    controls.push((
                        format!(
                            "{}Flow",
                            if self.tour_visuals.view == crate::tour_visuals::TourView::Flow {
                                "● "
                            } else {
                                ""
                            }
                        ),
                        "tourview flow",
                    ));
                }
            }
        }
        controls
            .into_iter()
            .map(|(label, command)| Control { label, command })
            .collect()
    }

    pub(crate) fn tour_catalog_start(&mut self) {
        let root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.tour_ui.catalog_rx = Some(rx);
        let mut results = Results::new(
            "Tours — Enter starts · K explanation · n create · e edit · r restart",
            vec![],
        );
        results.tour_list = true;
        results.busy = true;
        self.show_results(results);
        std::thread::spawn(move || {
            let mut paths: Vec<_> = std::fs::read_dir(root.join(".tours"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "tour"))
                .collect();
            paths.sort();
            let mut entries = Vec::new();
            for path in paths {
                let entry = catalog_entry(&root, &path);
                entries.push(entry);
            }
            let _ = tx.send(CatalogResult { root, entries });
        });
    }

    pub(crate) fn poll_tour_catalog(&mut self) -> bool {
        let Some(result) = self
            .tour_ui
            .catalog_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        else {
            return false;
        };
        self.tour_ui.catalog_rx = None;
        if result.root != self.project_root {
            return false;
        }
        if !matches!(self.mode, Mode::Results)
            || !self.results.as_ref().is_some_and(|r| r.tour_list)
        {
            return false;
        }
        let mut entries = result.entries;
        for entry in &mut entries {
            if let Some(command) = entry
                .action
                .as_ref()
                .and_then(|a| a.get("_vaayu_rerun_ex"))
                .and_then(|s| s.as_str())
            {
                if let Some((name, idx)) = self
                    .last_tour
                    .as_ref()
                    .filter(|(name, _)| command == format!("tour {name}"))
                {
                    let total = entry
                        .action
                        .as_ref()
                        .and_then(|a| a.get("_vaayu_tour_steps"))
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    let resume = (*idx as u64).min(total.saturating_sub(1)) + 1;
                    entry.text = format!("Resume step {resume}/{total} · {}", entry.text);
                    entry.action =
                        Some(serde_json::json!({"_vaayu_rerun_ex": format!("tourresume {name}")}));
                }
            }
        }
        if entries.is_empty() {
            let mut create = command_entry("No tours yet — Create with AI (:tournew)", "tournew");
            create.detail = "Tours live in .tours/*.tour. Describe a walkthrough, then :toursave sends it to your configured agent.".into();
            entries.push(create);
            entries.push(command_entry(
                "Create a minimal example to edit",
                "tourexample",
            ));
        }
        let results = self.results.as_mut().unwrap();
        results.busy = false;
        results.all_entries = entries.clone();
        results.entries = entries;
        results.apply_filter();
        true
    }
}

fn catalog_entry(root: &std::path::Path, path: &std::path::Path) -> Entry {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let parsed = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|text| {
            serde_json::from_str::<Tour>(&text)
                .map_err(|e| format!("Invalid JSON at {}:{}: {e}", e.line(), e.column()))
        });
    let mut entry = Entry::location(path.to_path_buf(), 0, 0, String::new());
    entry.no_path_prefix = true;
    match parsed {
        Ok(tour) if !tour.steps.is_empty() => {
            let missing = tour
                .steps
                .iter()
                .filter(|s| {
                    !s.file.is_empty()
                        && !crate::tour_visuals::project_path(root, &s.file)
                            .is_ok_and(|p| p.is_file())
                })
                .count();
            entry.text = format!(
                "{} steps{} · {} ({stem})",
                tour.steps.len(),
                if missing > 0 {
                    format!(" · ! {missing} missing source")
                } else {
                    String::new()
                },
                if tour.title.is_empty() {
                    stem.as_ref()
                } else {
                    tour.title.as_str()
                }
            );
            entry.detail = format!(
                "{}\n{}\nEnter starts / resumes · r starts over · e edits definition",
                summary(&tour.steps[0]),
                tour.steps[0].file
            );
            entry.action = Some(
                serde_json::json!({"_vaayu_rerun_ex": format!("tour {stem}"), "_vaayu_tour_steps": tour.steps.len()}),
            );
        }
        Ok(tour) => {
            entry.text = format!("! Empty tour · {} ({stem})", tour.title);
            entry.detail =
                "Add a step with file, line, and description. Enter / e edits the definition."
                    .into();
        }
        Err(error) => {
            entry.text = format!("! Invalid tour · {stem}");
            entry.detail =
                format!("{error}\nEnter / e edits the definition; R refreshes the list.");
        }
    }
    if entry.action.is_none() {
        entry.action = Some(serde_json::json!({"_vaayu_tour_edit_path": path}));
    }
    entry.text = entry.text.replace(['\n', '\r', '\t'], " ");
    entry
}
