//! Optional tour visuals. Anchor scans happen on step entry or after an edit
//! settles; rendering only reads cached ranges/labels. Git and diff work runs
//! off the input thread and never changes a project buffer.

use crate::{
    editor::Editor,
    results::{Entry, Results},
    tour::{Tour, TourStep},
    windows::Window,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

pub const TOUR_SCHEMA: &str = include_str!("../assets/tour.schema.json");
const MAX_TEXT: usize = 2 * 1024 * 1024;
const MAX_ANNOTATIONS: usize = 16;
const MAX_CONNECTIONS: usize = 24;

fn one() -> usize {
    1
}
fn three() -> usize {
    3
}
fn buffer_target() -> String {
    "buffer".into()
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TourVisuals {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub takeaway: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_pattern: Option<String>,
    /// Optional text on the immediately preceding line, for repeated anchors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<TourAnnotation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<TourConnection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<TourDiff>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct TourLocation {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(default = "one")]
    pub line: usize,
    #[serde(default, alias = "end_line", skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TourAnnotation {
    pub id: String,
    pub label: String,
    #[serde(flatten)]
    pub location: TourLocation,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TourConnection {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TourDiff {
    Snippets {
        before: String,
        after: String,
        #[serde(default)]
        label: String,
        #[serde(default = "three")]
        context: usize,
    },
    Git {
        base: String,
        #[serde(default = "buffer_target")]
        target: String,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        label: String,
        #[serde(default = "three")]
        context: usize,
    },
}

impl TourStep {
    pub fn location(&self) -> TourLocation {
        TourLocation {
            file: self.file.clone(),
            line: self.line,
            end_line: self.end_line,
            pattern: self.pattern.clone(),
            end_pattern: self.vaayu.as_ref().and_then(|v| v.end_pattern.clone()),
            context: self.vaayu.as_ref().and_then(|v| v.context.clone()),
        }
    }
}

#[derive(Clone)]
pub struct ResolvedLocation {
    pub start: usize,
    pub end: usize,
    pub warning: Option<String>,
}

pub fn resolve_location(lines: &[&str], location: &TourLocation) -> ResolvedLocation {
    let last = lines.len().saturating_sub(1);
    let authored = location.line.saturating_sub(1);
    let mut start = authored.min(last);
    let mut warnings: Vec<String> = Vec::new();
    if location.line == 0 || location.end_line.is_some_and(|e| e < location.line) {
        warnings.push("invalid line range".into());
    }
    if let Some(pattern) = location.pattern.as_deref().filter(|p| !p.trim().is_empty()) {
        let matches: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(i, text)| {
                text.contains(pattern)
                    && location
                        .context
                        .as_deref()
                        .is_none_or(|context| *i > 0 && lines[*i - 1].contains(context))
            })
            .map(|(i, _)| i)
            .collect();
        if let Some(nearest) = matches.iter().min_by_key(|i| i.abs_diff(authored)) {
            start = *nearest;
            if matches.len() > 1 {
                warnings.push("ambiguous anchor; nearest authored location used".into());
            }
        } else {
            warnings.push("anchor not found; authored line used".into());
        }
    }
    // Keep the authored span when the start relocates. An explicit end anchor
    // handles edits *inside* the construct, rather than only preceding edits.
    let span = location
        .end_line
        .unwrap_or(location.line)
        .saturating_sub(location.line);
    let mut end = start.saturating_add(span).min(last);
    if authored > last && start == authored.min(last) {
        warnings.push("authored line exceeds source".into());
    }
    if let Some(pattern) = location
        .end_pattern
        .as_deref()
        .filter(|p| !p.trim().is_empty())
    {
        if let Some(offset) = lines
            .iter()
            .skip(start)
            .position(|line| line.contains(pattern))
        {
            end = start + offset;
        } else {
            warnings.push("end anchor not found; original span used".into());
        }
    }
    ResolvedLocation {
        start,
        end,
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum TourView {
    #[default]
    Source,
    Diff,
    Flow,
}

impl TourView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Diff => "Diff",
            Self::Flow => "Flow",
        }
    }
}

pub struct TourVisualState {
    pub enabled: bool,
    /// Reading preference, retained across steps and tours.
    pub markdown: bool,
    pub view: TourView,
    /// JSON annotation index selected by highlight navigation, jump or follow.
    pub selected_annotation: Option<usize>,
    pub preview_buffer: Option<u64>,
    source_view: Option<Window>,
    pub decorated_buffer: Option<(u64, u64)>,
    pub ranges: Vec<(usize, usize)>,
    pub labels: HashMap<usize, (char, String)>,
    pub warnings: Vec<String>,
    pub preview_lines: Vec<i8>,
    pub metadata: std::cell::RefCell<Option<(u64, u64, String)>>,
    diff_rx: Option<mpsc::Receiver<Result<DiffPreview, String>>>,
    diff_stamp: Option<(u64, u64)>,
    diff_cache: Option<DiffPreview>,
    check_rx: Option<mpsc::Receiver<Vec<TourIssue>>>,
    pending_edit: Option<(u64, u64, Instant)>,
    anchors: HashMap<u64, AnchorCache>,
}

#[derive(Default)]
struct AnchorCache {
    seq: u64,
    locations: HashMap<TourLocation, ResolvedLocation>,
}

impl Default for TourVisualState {
    fn default() -> Self {
        Self {
            enabled: true,
            markdown: true,
            view: TourView::Source,
            selected_annotation: None,
            preview_buffer: None,
            source_view: None,
            decorated_buffer: None,
            ranges: Vec::new(),
            labels: HashMap::new(),
            warnings: Vec::new(),
            preview_lines: Vec::new(),
            metadata: Default::default(),
            diff_rx: None,
            diff_stamp: None,
            diff_cache: None,
            check_rx: None,
            pending_edit: None,
            anchors: HashMap::new(),
        }
    }
}

impl TourVisualState {
    pub fn clear_anchors(&mut self) {
        self.anchors.clear();
    }
    pub fn clear_step(&mut self) {
        self.view = TourView::Source;
        self.selected_annotation = None;
        self.decorated_buffer = None;
        self.ranges.clear();
        self.labels.clear();
        self.warnings.clear();
        self.preview_lines.clear();
        self.diff_rx = None;
        self.diff_stamp = None;
        self.diff_cache = None;
        self.pending_edit = None;
        self.metadata.borrow_mut().take();
    }
    pub fn valid_for(&self, buffer: &crate::buffer::Buffer) -> bool {
        self.enabled && self.decorated_buffer == Some((buffer.id, buffer.edit_seq))
    }
}

fn badge(index: usize) -> char {
    char::from_u32(0x2460 + index as u32).unwrap_or('→')
}

pub(crate) fn project_path(root: &Path, file: &str) -> Result<PathBuf, String> {
    let path = Path::new(file);
    if file.is_empty()
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!("invalid project path: {file}"));
    }
    Ok(root.join(path))
}

impl Editor {
    /// Reuse location resolutions across steps in an unchanged buffer. Only a
    /// cache miss makes a source snapshot or scans its lines for uniqueness.
    pub(crate) fn resolve_tour_locations(
        &mut self,
        source_id: u64,
        locations: &[TourLocation],
    ) -> Vec<ResolvedLocation> {
        let Some(source) = self.buffers.iter().find(|b| b.id == source_id) else {
            return Vec::new();
        };
        let cached = self.tour_visuals.anchors.entry(source_id).or_default();
        if cached.seq != source.edit_seq {
            cached.locations.clear();
            cached.seq = source.edit_seq;
        }
        if locations
            .iter()
            .any(|location| !cached.locations.contains_key(location))
        {
            let text = source.rope.to_string();
            let lines: Vec<_> = text.lines().collect();
            for location in locations {
                cached
                    .locations
                    .entry(location.clone())
                    .or_insert_with(|| resolve_location(&lines, location));
            }
        }
        locations
            .iter()
            .map(|location| cached.locations[location].clone())
            .collect()
    }

    pub(crate) fn resolve_tour_decorations(&mut self) {
        let source_id = if self
            .tour_explanation
            .is_some_and(|f| f.explanation_buffer == self.buf().id)
            || self.tour_visuals.preview_buffer == Some(self.buf().id)
        {
            self.tour_explanation
                .map(|f| f.source_buffer)
                .unwrap_or(self.buf().id)
        } else {
            self.buf().id
        };
        self.resolve_tour_decorations_for(source_id);
    }

    fn resolve_tour_decorations_for(&mut self, source_id: u64) {
        let Some((tour, idx)) = self.active_tour.as_ref() else {
            return;
        };
        let Some(step) = tour.steps.get(*idx) else {
            return;
        };
        let Some(buffer) = self.buffers.iter().find(|b| b.id == source_id) else {
            return;
        };
        let seq = buffer.edit_seq;
        let primary = self
            .tour_highlight
            .is_some_and(|(id, _, _)| id == source_id)
            .then(|| step.location());
        let annotations: Vec<_> = step
            .vaayu
            .as_ref()
            .into_iter()
            .flat_map(|v| v.annotations.iter())
            .take(MAX_ANNOTATIONS)
            .enumerate()
            .filter_map(|(index, annotation)| {
                let file = if annotation.location.file.is_empty() {
                    &step.file
                } else {
                    &annotation.location.file
                };
                let same_file = buffer
                    .path
                    .as_ref()
                    .is_some_and(|p| self.project_root.join(file) == *p)
                    || (file.is_empty() && primary.is_some());
                same_file.then(|| {
                    (
                        index,
                        annotation.id.clone(),
                        annotation.label.clone(),
                        annotation.location.clone(),
                    )
                })
            })
            .collect();
        let locations: Vec<_> = primary
            .iter()
            .cloned()
            .chain(annotations.iter().map(|(_, _, _, loc)| loc.clone()))
            .collect();
        let mut resolved = self
            .resolve_tour_locations(source_id, &locations)
            .into_iter();
        let mut ranges = Vec::new();
        let mut labels: HashMap<usize, (char, String)> = HashMap::new();
        let mut warnings = Vec::new();
        if let Some(primary) = primary.and_then(|_| resolved.next()) {
            self.tour_highlight = Some((source_id, primary.start, primary.end));
            if let Some(warning) = primary.warning {
                warnings.push(warning);
            }
        }
        for ((index, id, label, _), location) in annotations.into_iter().zip(resolved) {
            if let Some(warning) = location.warning {
                warnings.push(format!("{id}: {warning}"));
                // Never attach a confident arrow to an unresolved target.
                continue;
            }
            ranges.push((location.start, location.end));
            let label = format!("{} → {}", badge(index), label);
            labels
                .entry(location.start)
                .and_modify(|(_, text)| {
                    text.push_str(" · ");
                    text.push_str(&label);
                })
                .or_insert((badge(index), label));
        }
        self.tour_visuals.decorated_buffer = Some((source_id, seq));
        self.tour_visuals.ranges = ranges;
        self.tour_visuals.labels = labels;
        self.tour_visuals.warnings = warnings;
    }

    pub(crate) fn tour_visuals_text(&self) -> String {
        let Some((tour, idx)) = &self.active_tour else {
            return String::new();
        };
        let Some(step) = tour.steps.get(*idx) else {
            return String::new();
        };
        let mut text = String::new();
        if let Some(v) = &step.vaayu {
            if !v.title.is_empty() {
                text.push_str(&format!("\n\n## {}\n", v.title));
            }
            {
                for (i, a) in v.annotations.iter().take(MAX_ANNOTATIONS).enumerate() {
                    text.push_str(&format!("\n- {} → {} (`{}`)", badge(i), a.label, a.id));
                }
                for c in v.connections.iter().take(MAX_CONNECTIONS) {
                    text.push_str(&format!("\n- {} → {}: {}", c.from, c.to, c.label));
                }
            }
            if !v.takeaway.is_empty() {
                text.push_str(&format!("\n\n> **Takeaway:** {}", v.takeaway));
            }
            if v.diff.is_some() || !v.connections.is_empty() || !v.annotations.is_empty() {
                text.push_str(
                    "\n\nUse `:tourjump id` or `:tourfollow 1`; `:tourcheck` validates targets.",
                );
            }
        }
        text
    }

    pub(crate) fn tour_indicator_position(&self) -> (usize, usize) {
        let total = self
            .active_tour
            .as_ref()
            .and_then(|(tour, idx)| tour.steps.get(*idx))
            .and_then(|step| step.vaayu.as_ref())
            .map_or(0, |v| v.annotations.len().min(MAX_ANNOTATIONS));
        let current = self
            .tour_visuals
            .selected_annotation
            .filter(|idx| *idx < total)
            .map_or(0, |idx| idx + 1);
        (current, total)
    }

    /// Going back from a preview must not consume a source-history jump or
    /// record the scratch preview in the jumplist.
    pub(crate) fn tour_preview_back(&mut self) -> bool {
        if self.active_tour.is_none()
            || self.tour_visuals.view == TourView::Source
            || !self.tour_explanation.is_some_and(|focus| {
                self.buf().id == focus.explanation_buffer
                    || self.tour_visuals.preview_buffer == Some(self.buf().id)
            })
        {
            return false;
        }
        self.tour_view("source");
        true
    }

    pub fn toggle_tour_indicators(&mut self) {
        if self.active_tour.is_none() {
            self.set_message("No active tour");
            return;
        }
        self.tour_visuals.enabled = !self.tour_visuals.enabled;
        self.set_message(if self.tour_visuals.enabled {
            "Tour indicators on"
        } else {
            "Tour indicators off"
        });
    }

    pub(crate) fn restore_tour_source_view(&mut self) {
        let Some(saved) = self.tour_visuals.source_view.take() else {
            return;
        };
        self.store_window();
        if let Some(pane) = self
            .windows
            .iter()
            .position(|w| Some(w.buffer) == self.tour_visuals.preview_buffer)
            .or_else(|| {
                self.tour_explanation.and_then(|focus| {
                    self.windows
                        .get(focus.source_window)
                        .filter(|w| {
                            w.buffer != focus.explanation_buffer
                                && !w.preview
                                && w.terminal.is_none()
                                && !w.file_tree
                                && !w.outline
                        })
                        .map(|_| focus.source_window)
                })
            })
        {
            let active = self.active_window;
            // focus_window stores the outgoing pane first; load the saved
            // source *after* moving focus away from the preview.
            if active == pane {
                self.switch_buffer_id(saved.buffer);
                self.set_cursor(saved.cursor.0, saved.cursor.1);
                let buffer = self.buf_mut();
                buffer.top_line = saved.top;
                buffer.top_wrap = saved.wrap_row;
                buffer.left_col = saved.left;
            }
            self.windows[pane] = saved;
        }
        self.tour_visuals.view = TourView::Source;
    }

    pub(crate) fn show_tour_preview(&mut self, text: &str, lines: Vec<i8>) {
        let Some(focus) = self.tour_explanation else {
            return;
        };
        self.store_window();
        let active = self.active_window;
        let pane = self
            .windows
            .iter()
            .position(|w| Some(w.buffer) == self.tour_visuals.preview_buffer)
            .or_else(|| {
                self.windows
                    .get(focus.source_window)
                    .filter(|w| w.buffer == focus.source_buffer)
                    .map(|_| focus.source_window)
            })
            .or_else(|| {
                self.windows
                    .iter()
                    .position(|w| w.buffer == focus.source_buffer)
            });
        let Some(pane) = pane else {
            self.set_message("Tour source pane is closed");
            return;
        };
        if self.tour_visuals.source_view.is_none() {
            self.tour_visuals.source_view = Some(self.windows[pane].clone());
        }
        let id = self
            .tour_visuals
            .preview_buffer
            .filter(|id| self.buffers.iter().any(|b| b.id == *id))
            .unwrap_or_else(|| {
                let buffer = crate::buffer::Buffer::empty();
                let id = buffer.id;
                self.buffers.push(buffer);
                self.invalidate_index_caches();
                id
            });
        self.tour_visuals.preview_buffer = Some(id);
        self.replace_tour_explanation(id, text);
        self.tour_visuals.preview_lines = lines;
        self.focus_window(pane);
        self.switch_buffer_id(id);
        self.set_cursor(0, 0);
        self.store_window();
        if active != pane && active < self.windows.len() {
            self.focus_window(active);
        }
    }

    pub fn tour_view(&mut self, view: &str) {
        if self.tour_ui.source_error.is_some() {
            self.set_message(
                "Source unavailable · :tourretry to reload · ]t to skip · :touredit to repair",
            );
            return;
        }
        let Some((tour, idx)) = &self.active_tour else {
            self.set_message("No active tour");
            return;
        };
        let view = match view.trim() {
            "source" => TourView::Source,
            "diff" => TourView::Diff,
            "flow" => TourView::Flow,
            "" => {
                self.set_message(format!(
                    "Tour view: {} · :tourview source|diff|flow",
                    self.tour_visuals.view.label()
                ));
                return;
            }
            _ => {
                self.set_message("Usage: :tourview source|diff|flow");
                return;
            }
        };
        if view == TourView::Source {
            self.restore_tour_source_view();
            self.tour_visuals.diff_rx = None;
            self.tour_visuals.view = view;
            self.set_message("Tour Source view");
            return;
        }
        // Source/Flow navigation must not clone embedded comparison payloads.
        let step = &tour.steps[*idx];
        let Some(visuals) = step.vaayu.as_ref() else {
            self.set_message("This step has no visual views");
            return;
        };
        if view == TourView::Flow {
            if visuals.connections.is_empty() {
                self.set_message("This step has no connections");
                return;
            }
            let text = flow_text(step);
            self.tour_visuals.diff_rx = None;
            self.tour_visuals.view = view;
            self.show_tour_preview(&text, Vec::new());
            self.set_message(
                "Tour Flow · Ctrl-O / ,vS back · :tourfollow <number> jumps to target",
            );
            return;
        }
        let Some(diff) = visuals.diff.clone() else {
            self.set_message("This step has no diff");
            return;
        };
        let file = step.file.clone();
        let source_id = self
            .tour_highlight
            .map(|(id, _, _)| id)
            .or_else(|| self.tour_explanation.map(|f| f.source_buffer))
            .unwrap_or(self.buf().id);
        let source = match &diff {
            TourDiff::Git {
                file: Some(file), ..
            } => self
                .buffers
                .iter()
                .find(|b| b.path.as_ref() == Some(&self.project_root.join(file))),
            _ => self.buffers.iter().find(|b| b.id == source_id),
        };
        let stamp = match &diff {
            TourDiff::Git { target, .. } if target == "buffer" => {
                source.map(|b| (b.id, b.edit_seq))
            }
            _ => Some((0, 0)),
        };
        self.tour_visuals.view = view;
        if self.tour_visuals.diff_stamp == stamp {
            if let Some(preview) = self
                .tour_visuals
                .diff_cache
                .clone()
                .filter(|_| stamp.is_some())
            {
                self.show_tour_preview(&preview.text, preview.lines);
                self.set_message("Tour Diff view · :tourview source returns to code");
                return;
            }
            if self.tour_visuals.diff_rx.is_some() {
                return;
            }
        }
        let buffer_text = match &diff {
            TourDiff::Git { target, .. } if target == "buffer" => {
                source.map(|b| b.rope.to_string())
            }
            _ => None,
        };
        self.tour_visuals.diff_stamp = stamp;
        self.tour_visuals.diff_cache = None;
        let root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.tour_visuals.diff_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(build_diff(&root, &file, diff, buffer_text));
        });
        self.show_tour_preview(
            "Loading tour comparison…\n\n:tourview source returns to code\n",
            Vec::new(),
        );
        self.set_message("Loading tour diff in background");
    }

    pub fn tour_jump(&mut self, id: &str) {
        if self.active_tour.is_none() {
            self.set_message("No active tour");
            return;
        }
        let index = self
            .active_tour
            .as_ref()
            .and_then(|(tour, idx)| tour.steps[*idx].vaayu.as_ref())
            .and_then(|v| {
                v.annotations
                    .iter()
                    .take(MAX_ANNOTATIONS)
                    .position(|a| a.id == id.trim())
            });
        let Some(index) = index else {
            self.set_message("Unknown annotation · :tourjump <id>");
            return;
        };
        if let Err(error) = self.jump_tour_annotation(index) {
            self.set_message(error);
        }
    }

    /// Walk annotations in authored order, including cross-file targets. Stay
    /// within this step, skip broken anchors, and bound work by the schema cap.
    pub fn tour_highlight_step(&mut self, forward: bool, count: usize) {
        if self.active_tour.is_none() {
            self.set_message("No active tour");
            return;
        }
        let (_, total) = self.tour_indicator_position();
        if total == 0 {
            self.set_message("This step has no highlights · ]t/[t changes steps");
            return;
        }
        let selected = self.tour_visuals.selected_annotation;
        let mut next = match selected {
            Some(idx) if forward => idx.checked_add(1).filter(|i| *i < total),
            Some(idx) => idx.checked_sub(1),
            None if forward => Some(0),
            None => Some(total - 1),
        };
        if next.is_none() {
            self.set_message(if forward {
                "Last highlight in this step · ]t for the next step"
            } else {
                "First highlight in this step · [t for the previous step"
            });
            return;
        }
        let mut remaining = count.clamp(1, MAX_ANNOTATIONS);
        let mut error = None;
        while let Some(index) = next {
            match self.jump_tour_annotation(index) {
                Ok(()) => {
                    remaining -= 1;
                    error = None;
                    if remaining == 0 {
                        return;
                    }
                }
                Err(message) => error = Some(message),
            }
            next = if forward {
                index.checked_add(1).filter(|i| *i < total)
            } else {
                index.checked_sub(1)
            };
        }
        if let Some(error) = error {
            self.set_message(error);
        }
    }

    fn jump_tour_annotation(&mut self, index: usize) -> Result<(), String> {
        let (step_file, annotation) = self
            .active_tour
            .as_ref()
            .and_then(|(tour, idx)| tour.steps.get(*idx))
            .and_then(|step| {
                step.vaayu
                    .as_ref()?
                    .annotations
                    .get(index)
                    .map(|annotation| (step.file.clone(), annotation.clone()))
            })
            .ok_or("Unknown tour annotation")?;
        let file = if annotation.location.file.is_empty() {
            &step_file
        } else {
            &annotation.location.file
        };
        let path = project_path(&self.project_root, file)?;
        if !path.is_file() {
            return Err(format!("{}: source file not found: {file}", annotation.id));
        }
        self.restore_tour_source_view();
        self.tour_visuals.diff_rx = None;
        if let Some(focus) = self.tour_explanation {
            self.focus_tour_source(focus);
        }
        let origin = self.capture_window();
        self.open_file(path)
            .map_err(|e| format!("Tour annotation: {e}"))?;
        let bid = self.buf().id;
        let location = self
            .resolve_tour_locations(bid, &[annotation.location])
            .remove(0);
        if let Some(warning) = location.warning {
            self.switch_buffer_id(origin.buffer);
            self.set_cursor(origin.cursor.0, origin.cursor.1);
            let buffer = self.buf_mut();
            buffer.top_line = origin.top;
            buffer.top_wrap = origin.wrap_row;
            buffer.left_col = origin.left;
            self.store_window();
            return Err(format!("{}: {warning}", annotation.id));
        }
        self.set_cursor(location.start, 0);
        let buffer = self.buf_mut();
        buffer.top_line = location.start.saturating_sub(3);
        buffer.top_wrap = 0;
        buffer.left_col = 0;
        self.tour_visuals.selected_annotation = Some(index);
        if let Some(focus) = self.tour_explanation.as_mut() {
            focus.source_buffer = bid;
            focus.source_window = self.active_window;
        }
        self.store_window();
        self.resolve_tour_decorations();
        self.set_message(format!(
            "Tour indicator {}/{} · {} · {}",
            index + 1,
            self.tour_indicator_position().1,
            annotation.id,
            annotation.label
        ));
        Ok(())
    }

    pub fn tour_follow(&mut self, index: &str) {
        let target = index
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| {
                self.active_tour
                    .as_ref()
                    .and_then(|(tour, step)| tour.steps[*step].vaayu.as_ref())
                    .and_then(|v| v.connections.get(i))
            })
            .map(|c| c.to.clone());
        if let Some(target) = target {
            self.tour_jump(&target);
        } else {
            self.set_message("Usage: :tourfollow <connection number>");
        }
    }

    pub fn tour_check(&mut self, name: &str) {
        let tour = if name.trim().is_empty() {
            match &self.active_tour {
                Some((tour, _)) => tour.clone(),
                None => {
                    self.set_message("Start a tour or use :tourcheck <name>");
                    return;
                }
            }
        } else {
            if name.contains(['/', '\\']) || name.contains("..") {
                self.set_message("Invalid tour name");
                return;
            }
            let path = self
                .project_root
                .join(".tours")
                .join(format!("{}.tour", name.trim()));
            match std::fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<Tour>(&s).map_err(|e| e.to_string()))
            {
                Ok(tour) => tour,
                Err(e) => {
                    self.set_message(format!("Tour JSON: {e}"));
                    return;
                }
            }
        };
        let root = self.project_root.clone();
        let mut relevant = std::collections::HashSet::new();
        for step in &tour.steps {
            relevant.insert(root.join(&step.file));
            if let Some(v) = &step.vaayu {
                for a in &v.annotations {
                    relevant.insert(root.join(&a.location.file));
                }
                if let Some(TourDiff::Git {
                    file: Some(file), ..
                }) = &v.diff
                {
                    relevant.insert(root.join(file));
                }
            }
        }
        let buffers: HashMap<_, _> = self
            .buffers
            .iter()
            .filter_map(|b| {
                b.path
                    .as_ref()
                    .filter(|p| relevant.contains(*p))
                    .map(|p| (p.clone(), b.rope.to_string()))
            })
            .collect();
        let (tx, rx) = mpsc::channel();
        self.tour_visuals.check_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(check_tour(&root, &tour, buffers));
        });
        self.set_message("Checking tour anchors and visuals in background");
    }

    pub(crate) fn poll_tour_visuals(&mut self) -> bool {
        let mut changed = self.poll_tour_catalog();
        if let Some(result) = self
            .tour_visuals
            .diff_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.tour_visuals.diff_rx = None;
            if self.active_tour.is_some() && self.tour_visuals.view == TourView::Diff {
                let preview = result.unwrap_or_else(|e| DiffPreview {
                    text: format!(
                        "Tour diff unavailable: {e}\n\n:tourview source returns to code\n"
                    ),
                    lines: Vec::new(),
                });
                self.show_tour_preview(&preview.text, preview.lines.clone());
                self.tour_visuals.diff_cache = Some(preview);
                self.set_message(
                    "Tour Diff view · comparison snapshot · :tourview diff refreshes after edits",
                );
                changed = true;
            }
        }
        if let Some(issues) = self
            .tour_visuals
            .check_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.tour_visuals.check_rx = None;
            if issues.is_empty() {
                self.set_message("Tour check passed: paths, anchors, connections and comparisons");
            } else {
                let entries = issues
                    .into_iter()
                    .map(|issue| {
                        if let Ok(path) = project_path(&self.project_root, &issue.file) {
                            Entry::location(path, issue.line.saturating_sub(1), 0, issue.message)
                        } else {
                            Entry::text(issue.message)
                        }
                    })
                    .collect();
                self.show_results(Results::new(
                    "Tour check — fix these targets before adding visuals",
                    entries,
                ));
            }
            changed = true;
        }
        if let Some((id, seq)) = self.tour_visuals.decorated_buffer {
            if let Some(buffer) = self.buffers.iter().find(|b| b.id == id) {
                if buffer.edit_seq != seq {
                    let next = (id, buffer.edit_seq);
                    if self
                        .tour_visuals
                        .pending_edit
                        .as_ref()
                        .is_none_or(|(id, seq, _)| (*id, *seq) != next)
                    {
                        self.tour_visuals.pending_edit = Some((next.0, next.1, Instant::now()));
                    } else if self
                        .tour_visuals
                        .pending_edit
                        .as_ref()
                        .is_some_and(|(_, _, since)| since.elapsed() >= Duration::from_millis(180))
                    {
                        self.resolve_tour_decorations_for(id);
                        self.tour_visuals.pending_edit = None;
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}

fn flow_text(step: &TourStep) -> String {
    let Some(v) = &step.vaayu else {
        return String::new();
    };
    let mut text = format!(
        "Tour Flow — {}\n\n",
        if v.title.is_empty() {
            "connections"
        } else {
            &v.title
        }
    );
    for (i, c) in v.connections.iter().take(MAX_CONNECTIONS).enumerate() {
        let from = v.annotations.iter().find(|a| a.id == c.from);
        let to = v.annotations.iter().find(|a| a.id == c.to);
        if let (Some(from), Some(to)) = (from, to) {
            text.push_str(&format!(
                "{}. {} → {}\n   {}\n",
                i + 1,
                from.label,
                to.label,
                c.label
            ));
            let file = if to.location.file.is_empty() {
                &step.file
            } else {
                &to.location.file
            };
            text.push_str(&format!("   {}:{} [{}]\n\n", file, to.location.line, to.id));
        } else {
            text.push_str(&format!(
                "{}. Unresolved connection: {} → {}\n\n",
                i + 1,
                c.from,
                c.to
            ));
        }
    }
    text.push_str(":tourfollow <number> jumps to target · Ctrl-O / ,vS returns to source\n");
    text
}

#[derive(Clone)]
struct DiffPreview {
    text: String,
    lines: Vec<i8>,
}

fn read_bounded(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    file.take((MAX_TEXT + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_TEXT {
        return Err("comparison exceeds 2 MiB per side".into());
    }
    Ok(text)
}

fn git_blob(root: &Path, revision: &str, file: &str) -> Result<(String, String), String> {
    project_path(root, file)?;
    if revision.is_empty() || revision.starts_with('-') || revision.contains([':', '\n', '\0']) {
        return Err("invalid Git revision".into());
    }
    let hash = crate::git_tools::run(
        root,
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
    )
    .map_err(|e| format!("Git revision {revision}: {e}"))?;
    let hash = hash.trim().to_string();
    let text = crate::git_tools::run(
        root,
        &[
            "show",
            "--no-ext-diff",
            "--no-textconv",
            &format!("{hash}:{file}"),
        ],
    )
    .map_err(|e| format!("Git file {file} at {revision}: {e}"))?;
    if text.len() > MAX_TEXT {
        return Err("comparison exceeds 2 MiB per side".into());
    }
    Ok((hash, text))
}

fn build_diff(
    root: &Path,
    step_file: &str,
    spec: TourDiff,
    buffer: Option<String>,
) -> Result<DiffPreview, String> {
    let (before, after, before_label, after_label, label, context) = match spec {
        TourDiff::Snippets {
            before,
            after,
            label,
            context,
        } => (
            before,
            after,
            "example before".into(),
            "example after".into(),
            format!("Illustrative example — {label}"),
            context,
        ),
        TourDiff::Git {
            base,
            target,
            file,
            label,
            context,
        } => {
            let file = file.as_deref().unwrap_or(step_file);
            let (base_hash, before) = git_blob(root, &base, file)?;
            let (after_label, after) = if target == "buffer" {
                (
                    format!("{file} (buffer snapshot, includes unsaved edits)"),
                    match buffer {
                        Some(text) => text,
                        None => read_bounded(&project_path(root, file)?)?,
                    },
                )
            } else {
                let (hash, text) = git_blob(root, &target, file)?;
                (format!("{file} @ {hash}"), text)
            };
            (
                before,
                after,
                format!("{file} @ {base_hash}"),
                after_label,
                format!("Git comparison — {label}"),
                context,
            )
        }
    };
    if before.len() > MAX_TEXT || after.len() > MAX_TEXT {
        return Err("comparison exceeds 2 MiB per side".into());
    }
    let diff = similar::TextDiff::configure()
        .timeout(Duration::from_millis(500))
        .diff_lines(&before, &after);
    let unified = diff
        .unified_diff()
        .context_radius(context.min(20))
        .header(&before_label, &after_label)
        .to_string();
    let mut text = format!("{label}\nComparison snapshot · :tourview source returns to code\n\n");
    let mut lines = vec![0; 3];
    if unified.is_empty() {
        text.push_str("No changes between these sources.\n");
        lines.push(0);
    } else {
        for (i, line) in unified.lines().enumerate() {
            if i >= 4000 {
                text.push_str("… diff truncated after 4000 lines\n");
                lines.push(0);
                break;
            }
            let kind = if line.starts_with('+') && !line.starts_with("+++") {
                1
            } else if line.starts_with('-') && !line.starts_with("---") {
                -1
            } else {
                0
            };
            // Terminal control bytes in illustrative or historical text must
            // render as text rather than becoming terminal escape sequences.
            text.extend(line.chars().map(|c| {
                if c.is_control() && c != '\t' {
                    '�'
                } else {
                    c
                }
            }));
            text.push('\n');
            lines.push(kind);
        }
    }
    Ok(DiffPreview { text, lines })
}

struct TourIssue {
    file: String,
    line: usize,
    message: String,
}

fn check_tour(root: &Path, tour: &Tour, mut files: HashMap<PathBuf, String>) -> Vec<TourIssue> {
    let mut issues = Vec::new();
    if tour.steps.is_empty() {
        issues.push(TourIssue {
            file: String::new(),
            line: 1,
            message: "Tour has no steps".into(),
        });
    }
    for (index, step) in tour.steps.iter().enumerate() {
        let mut report = |file: &str, line: usize, message: String| {
            issues.push(TourIssue {
                file: file.into(),
                line,
                message: format!("Step {}: {message}", index + 1),
            })
        };
        let mut locations = vec![("step", step.location())];
        if let Some(v) = &step.vaayu {
            if v.annotations.len() > MAX_ANNOTATIONS || v.connections.len() > MAX_CONNECTIONS {
                report(
                    &step.file,
                    step.line,
                    "at most 16 annotations and 24 connections per step".into(),
                );
            }
            let mut ids = std::collections::HashSet::new();
            for a in &v.annotations {
                if a.id.is_empty()
                    || !a
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    || !ids.insert(&a.id)
                {
                    report(
                        &step.file,
                        step.line,
                        format!("empty or duplicate annotation ID: {}", a.id),
                    );
                }
                if a.label.trim().is_empty() {
                    report(
                        &step.file,
                        step.line,
                        format!("{} has an empty label", a.id),
                    );
                }
                let mut location = a.location.clone();
                if location.file.is_empty() {
                    location.file = step.file.clone();
                }
                locations.push((&a.id, location));
            }
            for c in &v.connections {
                if !ids.contains(&c.from) || !ids.contains(&c.to) {
                    report(
                        &step.file,
                        step.line,
                        format!("unknown connection target: {} → {}", c.from, c.to),
                    );
                }
            }
            if let Some(diff) = &v.diff {
                let diff_file = match diff {
                    TourDiff::Git {
                        file: Some(file), ..
                    } => file,
                    _ => &step.file,
                };
                if let Err(e) = build_diff(
                    root,
                    &step.file,
                    diff.clone(),
                    files.get(&root.join(diff_file)).cloned(),
                ) {
                    report(&step.file, step.line, e);
                }
            }
        }
        for (id, location) in locations {
            let path = match project_path(root, &location.file) {
                Ok(p) => p,
                Err(e) => {
                    report(&location.file, location.line, e);
                    continue;
                }
            };
            if !path.is_file() {
                report(
                    &location.file,
                    location.line,
                    format!("source file not found: {}", location.file),
                );
                continue;
            }
            if !files.contains_key(&path) {
                match read_bounded(&path) {
                    Ok(text) => {
                        files.insert(path.clone(), text);
                    }
                    Err(e) => {
                        report(&location.file, location.line, e);
                        continue;
                    }
                }
            }
            let lines: Vec<_> = files[&path].lines().collect();
            if let Some(warning) = resolve_location(&lines, &location).warning {
                report(&location.file, location.line, format!("{id}: {warning}"));
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        editor: Editor,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "vaayu-tour-visuals-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join(".tours")).unwrap();
        std::fs::write(
            root.join("demo.txt"),
            "intro\nread_input();\nupdate_buffer();\nrequest_redraw();\ntail\n",
        )
        .unwrap();
        std::fs::write(root.join("other.txt"), "other intro\npaint_screen();\n").unwrap();
        let tour = serde_json::json!({"title":"Visual tour","steps":[
            {"file":"demo.txt","line":2,"endLine":4,"pattern":"read_input();","description":"Follow the edit.","vaayu":{
                "title":"Decode then draw","takeaway":"Only the affected rows change.",
                "annotations":[
                    {"id":"read","line":2,"pattern":"read_input();","label":"Read input"},
                    {"id":"update","line":3,"pattern":"update_buffer();","label":"Change the buffer"},
                    {"id":"paint","file":"other.txt","line":2,"pattern":"paint_screen();","label":"Paint the screen"}],
                "connections":[{"from":"update","to":"paint","label":"Schedules painting"}],
                "diff":{"kind":"snippets","before":"old_value\nshared\n","after":"new_value\nshared\n","label":"A focused example"}}},
            {"file":"demo.txt","line":5,"description":"Plain step"}
        ]});
        std::fs::write(root.join(".tours/visual.tour"), tour.to_string()).unwrap();
        let mut editor = Editor::new(crate::config::Config {
            clipboard_unnamedplus: false,
            watch: false,
            ..Default::default()
        });
        editor.project_root = root.clone();
        editor.start_tour("visual");
        Fixture { root, editor }
    }

    fn await_diff(editor: &mut Editor) {
        let start = Instant::now();
        while editor.tour_visuals.diff_rx.is_some() {
            editor.poll_tour_visuals();
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn tour_view_controls_preserve_scratch_text_and_both_reading_positions() {
        let mut f = fixture();
        let ed = &mut f.editor;
        ed.toggle_tour_explanation();
        ed.tour_markdown("off");
        ed.buf_mut().insert_str(0, 0, "MY NOTE\n");
        ed.set_cursor(1, 3);
        ed.buf_mut().top_line = 1;
        ed.store_window();
        ed.windows[ed.active_window].preview_scroll = 4;
        let id = ed.buf().id;
        let text = ed.buf().rope.to_string();
        let seq = ed.buf().edit_seq;
        for view in ["flow", "source", "diff", "source"] {
            ed.tour_view(view);
            if view == "diff" {
                await_diff(ed);
            }
            assert_eq!(ed.buf().id, id);
            assert_eq!(ed.buf().rope.to_string(), text);
            assert_eq!(ed.buf().edit_seq, seq);
            assert!(ed.buf().is_modified());
            assert_eq!(ed.cursor(), (1, 3));
            assert_eq!(ed.buf().top_line, 1);
            assert_eq!(ed.windows[ed.active_window].preview_scroll, 4);
        }
        ed.toggle_tour_indicators();
        ed.tour_markdown("on");
        ed.tour_highlight_step(true, 1);
        ed.toggle_tour_explanation();
        assert_eq!(ed.buf().id, id);
        assert_eq!(ed.buf().rope.to_string(), text);
        assert_eq!(ed.cursor(), (1, 3));
        assert_eq!(ed.windows[ed.active_window].preview_scroll, 4);
    }

    #[test]
    fn tour_missing_source_is_read_only_recoverable_and_always_validated() {
        let mut f = fixture();
        let ed = &mut f.editor;
        let missing = f.root.join("missing.rs");
        ed.active_tour.as_mut().unwrap().0.steps[0].file = "missing.rs".into();
        ed.active_tour.as_mut().unwrap().0.steps[0].line = 1;
        let tour = ed.active_tour.as_ref().unwrap().0.clone();
        // An editor-created new-file buffer cannot mask a missing disk path.
        let mut snapshots = HashMap::new();
        snapshots.insert(missing.clone(), String::new());
        assert!(check_tour(&f.root, &tour, snapshots)
            .iter()
            .any(|i| i.file == "missing.rs" && i.message.contains("file not found")));
        ed.tour_goto(0);
        assert!(ed.tour_ui.source_error.is_some());
        assert!(ed.windows[ed.active_window].preview);
        assert!(ed.buffers.iter().all(|b| b.path.as_ref() != Some(&missing)));
        let source = std::fs::read_to_string(f.root.join("demo.txt")).unwrap();
        ed.feed_key(crate::key::Key::Char('i'));
        assert_eq!(ed.mode, crate::mode::Mode::Normal);
        ed.toggle_tour_explanation();
        ed.toggle_tour_explanation();
        assert!(ed.windows[ed.active_window].preview);
        let unavailable = ed.buf().id;
        for view in ["source", "diff", "flow"] {
            ed.tour_view(view);
            assert_eq!(ed.buf().id, unavailable);
            assert!(ed.windows[ed.active_window].preview);
        }
        std::fs::write(&missing, "recovered\n").unwrap();
        ed.tour_retry();
        assert!(ed.tour_ui.source_error.is_none());
        assert_eq!(ed.buf().path.as_ref(), Some(&missing));
        assert!(!ed.windows[ed.active_window].preview);
        assert_eq!(
            std::fs::read_to_string(f.root.join("demo.txt")).unwrap(),
            source
        );
    }

    #[test]
    fn tour_visuals_relocate_ranges_and_disambiguate_with_context() {
        let lines = [
            "prefix",
            "fn target",
            "body",
            "end",
            "section two",
            "fn target",
            "body",
            "more",
            "end",
        ];
        let mut location: TourLocation =
            serde_json::from_value(serde_json::json!({"line":5,"endLine":7,"pattern":"fn target"}))
                .unwrap();
        let resolved = resolve_location(&lines, &location);
        assert_eq!((resolved.start, resolved.end), (5, 7));
        assert!(resolved.warning.unwrap().contains("ambiguous"));
        location.context = Some("section two".into());
        location.end_pattern = Some("end".into());
        let resolved = resolve_location(&lines, &location);
        assert_eq!((resolved.start, resolved.end), (5, 8));
        assert!(resolved.warning.is_none());
        location.pattern = Some("removed".into());
        assert!(resolve_location(&lines, &location)
            .warning
            .unwrap()
            .contains("not found"));
    }

    #[test]
    fn tour_visuals_are_optional_and_survive_json_copy() {
        let mut f = fixture();
        f.editor.copy_tour();
        let text = f.editor.registers.get(Some('+')).unwrap().text.clone();
        let copied: Tour = serde_json::from_str(&text).unwrap();
        let v = copied.steps[0].vaayu.as_ref().unwrap();
        assert_eq!(v.annotations[2].location.file, "other.txt");
        assert_eq!(v.connections[0].to, "paint");
        assert!(matches!(v.diff, Some(TourDiff::Snippets { .. })));
        assert!(copied.steps[1].vaayu.is_none());
        assert!(!serde_json::to_string(&copied.steps[1])
            .unwrap()
            .contains("vaayu"));
        let schema: serde_json::Value = serde_json::from_str(TOUR_SCHEMA).unwrap();
        assert!(schema["$defs"]["visuals"]["properties"]["diff"].is_object());
    }

    #[test]
    fn tour_visuals_cache_callouts_and_disable_them_without_losing_source() {
        let mut f = fixture();
        let editor = &mut f.editor;
        assert!(editor.tour_visuals.diff_rx.is_none());
        assert_eq!(editor.tour_visuals.ranges, vec![(1, 1), (2, 2)]);
        assert!(editor.tour_visuals.labels[&2].1.contains("② → Change"));
        let label_ptr = editor.tour_visuals.labels[&2].1.as_ptr();
        for _ in 0..100 {
            editor.set_cursor(3, 0);
            assert!(!editor.poll_tour_visuals());
            assert_eq!(editor.tour_visuals.labels[&2].1.as_ptr(), label_ptr);
        }
        editor.toggle_tour_indicators();
        assert!(!editor.tour_visuals.valid_for(editor.buf()));
        assert!(editor.tour_highlight.is_some());
        editor.toggle_tour_indicators();
        assert!(editor.tour_visuals.valid_for(editor.buf()));
    }

    #[test]
    fn tour_visuals_views_restore_source_cursor_and_keep_explanation_focus() {
        let mut f = fixture();
        let editor = &mut f.editor;
        let source = editor.buf().id;
        let original = editor.buf().rope.to_string();
        editor.set_cursor(2, 3);
        editor.toggle_tour_explanation();
        let explanation = editor.buf().id;
        editor.tour_view("diff");
        await_diff(editor);
        assert_eq!(editor.buf().id, explanation);
        assert_eq!(editor.windows.len(), 2);
        let preview = editor.tour_visuals.preview_buffer.unwrap();
        assert!(editor
            .buffers
            .iter()
            .find(|b| b.id == preview)
            .unwrap()
            .rope
            .to_string()
            .contains("-old_value"));
        assert!(editor.tour_visuals.preview_lines.contains(&-1));
        assert!(editor.tour_visuals.preview_lines.contains(&1));
        editor.toggle_tour_explanation();
        assert_eq!(editor.buf().id, preview);
        editor.tour_view("flow");
        assert!(editor
            .buf()
            .rope
            .to_string()
            .contains("Change the buffer → Paint the screen"));
        let count = editor.buffers.len();
        for _ in 0..3 {
            editor.tour_view("diff");
            editor.tour_view("flow");
        }
        assert_eq!(editor.buffers.len(), count);
        editor.tour_view("source");
        assert_eq!(editor.buf().id, source);
        assert_eq!(editor.cursor(), (2, 3));
        assert_eq!(editor.buf().rope.to_string(), original);
        editor.tour_view("diff");
        editor.tour_end();
        assert_eq!(editor.buf().id, source);
        assert!(editor.windows.is_empty());
        assert_eq!(editor.buf().rope.to_string(), original);
    }

    #[test]
    fn tour_visuals_follow_cross_file_targets_and_plain_steps_clear_views() {
        let mut f = fixture();
        let editor = &mut f.editor;
        let source = editor.buf().id;
        editor.tour_view("flow");
        editor.tour_follow("1");
        assert!(editor.buf().path.as_ref().unwrap().ends_with("other.txt"));
        assert_eq!(editor.cursor().0, 1);
        assert_eq!(editor.tour_highlight.unwrap().0, source);
        assert!(editor.tour_visuals.labels[&1].1.contains("③"));
        editor.tour_step(true);
        assert!(editor.buf().path.as_ref().unwrap().ends_with("demo.txt"));
        assert!(editor.tour_visuals.labels.is_empty());
        assert!(!editor
            .tour_controls(140)
            .iter()
            .any(|c| matches!(c.command, "tourview diff" | "tourview flow")));
        editor.tour_view("diff");
        assert_eq!(editor.tour_visuals.view.label(), "Source");
    }

    #[test]
    fn rendered_description_keeps_indicator_navigation_and_source_preview_returns() {
        use crate::key::Key;
        let mut f = fixture();
        let editor = &mut f.editor;
        editor.tour_markdown("on");
        editor.toggle_tour_explanation();
        let explanation = editor.buf().id;
        let seq = editor.buf().edit_seq;
        for key in "]h".chars() {
            editor.feed_key(Key::Char(key));
        }
        assert_eq!(editor.tour_indicator_position(), (1, 3));
        assert_ne!(editor.buf().id, explanation);
        assert_eq!(
            editor
                .buffers
                .iter()
                .find(|b| b.id == explanation)
                .unwrap()
                .edit_seq,
            seq
        );
        editor.toggle_tour_explanation();
        for view in ["flow", "diff"] {
            editor.tour_view(view);
            assert_eq!(editor.buf().id, explanation);
            assert!(editor.windows[editor.active_window].preview);
            editor.feed_key(Key::Ctrl('o'));
            assert_eq!(editor.tour_visuals.view.label(), "Source");
            assert_eq!(editor.tour_indicator_position(), (1, 3));
            assert_eq!(editor.buf().id, explanation);
            assert!(editor.windows[editor.active_window].preview);
            assert!(crate::render::statusline_label(
                editor,
                editor.buf(),
                &editor.windows[editor.active_window],
                80,
                true,
            )
            .contains("Highlight 1/3"));
        }
        for key in ":tourmarkdown off".chars() {
            editor.feed_key(Key::Char(key));
        }
        editor.feed_key(Key::Enter);
        assert!(!editor.windows[editor.active_window].preview);
        assert_eq!(editor.buf().id, explanation);
    }

    #[test]
    fn tour_highlight_keys_counts_and_commands_stay_within_the_step() {
        use crate::key::Key;
        let mut f = fixture();
        let editor = &mut f.editor;
        let source = editor.buf().id;
        assert_eq!(editor.tour_indicator_position(), (0, 3));
        let keys = |editor: &mut Editor, text: &str| {
            for key in text.chars() {
                editor.feed_key(Key::Char(key));
            }
        };
        keys(editor, "]h");
        assert_eq!(editor.tour_indicator_position(), (1, 3));
        assert_eq!(editor.cursor(), (1, 0));
        keys(editor, ",v]");
        assert_eq!(editor.tour_indicator_position(), (2, 3));
        assert_eq!(editor.cursor(), (2, 0));
        let cached = editor.tour_visuals.anchors[&source].locations.len();
        keys(editor, "]h");
        assert!(editor.buf().path.as_ref().unwrap().ends_with("other.txt"));
        assert_eq!(editor.tour_indicator_position(), (3, 3));
        keys(editor, "]h");
        assert_eq!(editor.active_tour.as_ref().unwrap().1, 0);
        assert_eq!(editor.tour_indicator_position(), (3, 3));
        assert!(editor.message.contains("Last highlight"));
        keys(editor, "[h,v[");
        assert_eq!(editor.tour_indicator_position(), (1, 3));
        assert_eq!(editor.buf().id, source);
        assert_eq!(editor.tour_visuals.anchors[&source].locations.len(), cached);
        keys(editor, "[h");
        assert!(editor.message.contains("First highlight"));
        crate::command::run_ex(editor, "tourhnext");
        assert_eq!(editor.tour_indicator_position(), (2, 3));
        crate::command::run_ex(editor, "tourhprev");
        assert_eq!(editor.tour_indicator_position(), (1, 3));
        editor.tour_follow("1");
        assert_eq!(editor.tour_indicator_position(), (3, 3));
        keys(editor, "2[h");
        assert_eq!(editor.tour_indicator_position(), (1, 3));
        editor.tour_step(true);
        assert_eq!(editor.tour_indicator_position(), (0, 0));
        keys(editor, "]h");
        assert!(editor.message.contains("no highlights"));
        editor.tour_step(false);
        assert_eq!(editor.tour_indicator_position(), (0, 3));
        keys(editor, "2]h");
        assert_eq!(editor.tour_indicator_position(), (2, 3));
        editor.tour_end();
        keys(editor, "]h");
        assert!(editor.message.contains("No active tour"));
        assert_eq!(editor.tour_indicator_position(), (0, 0));
    }

    #[test]
    fn tour_highlights_skip_broken_anchors_without_changing_the_selected_source() {
        let mut f = fixture();
        let editor = &mut f.editor;
        editor.active_tour.as_mut().unwrap().0.steps[0]
            .vaayu
            .as_mut()
            .unwrap()
            .annotations[1]
            .location
            .pattern = Some("removed anchor".into());
        editor.tour_highlight_step(true, 1);
        editor.tour_highlight_step(true, 1);
        assert_eq!(editor.tour_indicator_position(), (3, 3));
        let source = editor.buf().id;
        let cursor = editor.cursor();
        editor.tour_jump("update");
        assert_eq!(editor.buf().id, source);
        assert_eq!(editor.cursor(), cursor);
        assert_eq!(editor.tour_indicator_position(), (3, 3));
        assert!(editor.message.contains("anchor not found"));
        assert!(editor.tour_visuals.valid_for(editor.buf()));
        editor.tour_highlight_step(false, 1);
        assert_eq!(editor.tour_indicator_position(), (1, 3));
        // Missing cross-file targets must not silently create empty buffers.
        std::fs::remove_file(f.root.join("other.txt")).unwrap();
        let buffers = editor.buffers.len();
        editor.tour_jump("paint");
        assert!(editor.message.contains("file not found"));
        assert_eq!(editor.buffers.len(), buffers);
        assert_eq!(editor.tour_indicator_position(), (1, 3));
    }

    #[test]
    fn tour_preview_ctrl_o_restores_position_and_preserves_real_jump_history() {
        use crate::key::Key;
        for view in ["flow", "diff"] {
            for explanation_focused in [false, true] {
                let mut f = fixture();
                let editor = &mut f.editor;
                let source = editor.buf().id;
                editor.set_cursor(0, 0);
                editor.push_jump();
                editor.tour_jump("update");
                editor.set_cursor(2, 3);
                editor.buf_mut().top_line = 1;
                editor.buf_mut().left_col = 2;
                let jumps = editor.jumps.clone();
                let jump_index = editor.jump_index;
                let explanation = editor.tour_explanation.unwrap().explanation_buffer;
                if explanation_focused {
                    editor.toggle_tour_explanation();
                }
                editor.tour_view(view);
                if view == "diff" {
                    await_diff(editor);
                }
                editor.feed_key(Key::Ctrl('o'));
                assert_eq!(editor.tour_visuals.view.label(), "Source");
                assert!(editor.tour_visuals.source_view.is_none());
                assert_eq!(editor.tour_indicator_position(), (2, 3));
                assert_eq!(editor.jumps, jumps);
                assert_eq!(editor.jump_index, jump_index);
                if explanation_focused {
                    assert_eq!(editor.buf().id, explanation);
                    editor.toggle_tour_explanation();
                }
                assert_eq!(editor.buf().id, source);
                assert_eq!(editor.cursor(), (2, 3));
                assert_eq!(editor.buf().top_line, 1);
                assert_eq!(editor.buf().left_col, 2);
                // Once back in Source, Ctrl-O still navigates real jumps.
                editor.feed_key(Key::Ctrl('o'));
                assert_eq!(editor.cursor(), (0, 0));
                let preview = editor.tour_visuals.preview_buffer.unwrap();
                assert!(editor.jumps.iter().all(|jump| jump.buffer != preview));
            }
        }
        let mut f = fixture();
        let editor = &mut f.editor;
        editor.jumps.clear();
        let source = editor.buf().id;
        editor.tour_view("flow");
        editor.feed_key(Key::Ctrl('o'));
        assert_eq!(editor.buf().id, source);
        assert_eq!(editor.tour_visuals.view.label(), "Source");
        assert!(editor.jumps.is_empty());
        // Explicit Source also recovers the saved pane if another navigation
        // action replaced the preview buffer before returning.
        editor.set_cursor(2, 3);
        editor.tour_view("flow");
        editor.open_file(f.root.join("other.txt")).unwrap();
        editor.store_window();
        editor.tour_view("source");
        assert_eq!(editor.buf().id, source);
        assert_eq!(editor.cursor(), (2, 3));
        assert_eq!(editor.windows[editor.active_window].buffer, source);
    }

    #[test]
    fn tour_indicator_status_survives_views_custom_formats_and_narrow_panes() {
        let mut f = fixture();
        let editor = &mut f.editor;
        for (id, position) in [("read", "1/3"), ("update", "2/3"), ("paint", "3/3")] {
            editor.tour_jump(id);
            for format in ["", "%M %f"] {
                editor.config.statusline = format.into();
                for view in ["source", "flow", "diff"] {
                    editor.tour_view(view);
                    for explanation_focused in [false, true] {
                        if explanation_focused {
                            editor.toggle_tour_explanation();
                        }
                        for width in [40, 80, 140] {
                            let status = crate::render::statusline_label(
                                editor,
                                editor.buf(),
                                &editor.capture_window(),
                                width,
                                true,
                            );
                            assert!(status.contains(&format!("Highlight {position}")));
                            assert_eq!(
                                unicode_width::UnicodeWidthStr::width(status.as_str()),
                                width
                            );
                        }
                        if explanation_focused {
                            editor.toggle_tour_explanation();
                        }
                    }
                }
            }
        }
        editor.tour_step(true);
        let status = crate::render::statusline_label(
            editor,
            editor.buf(),
            &editor.capture_window(),
            40,
            true,
        );
        assert!(!status.contains("Highlight"));
    }

    #[test]
    fn tour_visuals_discard_late_diffs_and_refresh_only_after_edit_settles() {
        let mut f = fixture();
        let editor = &mut f.editor;
        editor.tour_view("diff");
        editor.tour_step(true);
        std::thread::sleep(Duration::from_millis(10));
        assert!(!editor.poll_tour_visuals());
        assert_eq!(editor.tour_visuals.view.label(), "Source");
        editor.tour_step(false);
        let id = editor.buf().id;
        editor.buf_mut().insert_str(0, 0, "inserted\n");
        assert!(!editor.tour_visuals.valid_for(editor.buf()));
        editor.poll_tour_visuals();
        editor.tour_visuals.pending_edit = Some((
            id,
            editor.buf().edit_seq,
            Instant::now() - Duration::from_millis(200),
        ));
        assert!(editor.poll_tour_visuals());
        assert!(editor.tour_visuals.valid_for(editor.buf()));
        assert!(editor.tour_visuals.labels[&2].1.contains("Read input"));
        assert_eq!(editor.tour_highlight, Some((id, 2, 4)));
    }

    #[test]
    fn tour_visuals_validator_reports_missing_ambiguous_and_broken_connections() {
        let f = fixture();
        let mut tour = f.editor.active_tour.as_ref().unwrap().0.clone();
        assert!(check_tour(&f.root, &tour, HashMap::new()).is_empty());
        let v = tour.steps[0].vaayu.as_mut().unwrap();
        v.annotations[0].location.pattern = Some("missing source".into());
        v.connections[0].to = "unknown".into();
        v.annotations[1].location.file = "../outside.txt".into();
        let issues = check_tour(&f.root, &tour, HashMap::new());
        assert!(issues
            .iter()
            .any(|i| i.message.contains("anchor not found")));
        assert!(issues
            .iter()
            .any(|i| i.message.contains("unknown connection target")));
        assert!(issues
            .iter()
            .any(|i| i.message.contains("invalid project path")));
    }

    #[test]
    fn tour_visuals_git_diff_uses_real_revisions_and_unsaved_buffer_snapshot() {
        let mut f = fixture();
        crate::git_tools::run(&f.root, &["init", "-q"]).unwrap();
        crate::git_tools::run(&f.root, &["add", "demo.txt"]).unwrap();
        crate::git_tools::run(
            &f.root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "base",
            ],
        )
        .unwrap();
        let base = crate::git_tools::run(&f.root, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let original_disk = std::fs::read_to_string(f.root.join("demo.txt")).unwrap();
        f.editor.buf_mut().insert_str(0, 0, "unsaved addition\n");
        let spec = TourDiff::Git {
            base: base.clone(),
            target: "buffer".into(),
            file: None,
            label: "Actual change".into(),
            context: 3,
        };
        let preview = build_diff(
            &f.root,
            "demo.txt",
            spec.clone(),
            Some(f.editor.buf().rope.to_string()),
        )
        .unwrap();
        assert!(preview.text.contains("+unsaved addition"));
        assert!(preview.text.contains("includes unsaved edits"));
        assert_eq!(
            std::fs::read_to_string(f.root.join("demo.txt")).unwrap(),
            original_disk
        );
        std::fs::write(f.root.join("demo.txt"), "committed replacement\n").unwrap();
        crate::git_tools::run(&f.root, &["add", "demo.txt"]).unwrap();
        crate::git_tools::run(
            &f.root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "next",
            ],
        )
        .unwrap();
        let spec = TourDiff::Git {
            base,
            target: "HEAD".into(),
            file: None,
            label: "Two revisions".into(),
            context: 3,
        };
        let preview = build_diff(&f.root, "demo.txt", spec, None).unwrap();
        assert!(preview.text.contains("+committed replacement"));
        assert!(!preview.text.contains("unsaved addition"));
    }

    #[test]
    fn tour_visuals_diff_limits_and_identical_sources_are_explicit() {
        let spec = TourDiff::Snippets {
            before: "same\n".into(),
            after: "same\n".into(),
            label: String::new(),
            context: 3,
        };
        assert!(build_diff(Path::new("."), "", spec, None)
            .unwrap()
            .text
            .contains("No changes"));
        let spec = TourDiff::Snippets {
            before: "x".repeat(MAX_TEXT + 1),
            after: String::new(),
            label: String::new(),
            context: 3,
        };
        assert!(build_diff(Path::new("."), "", spec, None)
            .err()
            .unwrap()
            .contains("2 MiB"));
    }

    #[test]
    fn tour_visuals_overridden_diff_tracks_its_own_buffer_revision() {
        let mut f = fixture();
        crate::git_tools::run(&f.root, &["init", "-q"]).unwrap();
        crate::git_tools::run(&f.root, &["add", "demo.txt", "other.txt"]).unwrap();
        crate::git_tools::run(
            &f.root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "base",
            ],
        )
        .unwrap();
        let other = crate::buffer::Buffer::from_path(f.root.join("other.txt")).unwrap();
        let other_id = other.id;
        f.editor.buffers.push(other);
        f.editor.active_tour.as_mut().unwrap().0.steps[0]
            .vaayu
            .as_mut()
            .unwrap()
            .diff = Some(TourDiff::Git {
            base: "HEAD".into(),
            target: "buffer".into(),
            file: Some("other.txt".into()),
            label: "Other file".into(),
            context: 3,
        });
        f.editor.tour_view("diff");
        await_diff(&mut f.editor);
        assert!(f
            .editor
            .tour_visuals
            .diff_cache
            .as_ref()
            .unwrap()
            .text
            .contains("No changes"));
        f.editor
            .buffers
            .iter_mut()
            .find(|b| b.id == other_id)
            .unwrap()
            .insert_str(0, 0, "other unsaved\n");
        f.editor.tour_view("diff");
        await_diff(&mut f.editor);
        assert!(f
            .editor
            .tour_visuals
            .diff_cache
            .as_ref()
            .unwrap()
            .text
            .contains("+other unsaved"));
        f.editor.tour_check("");
        let start = Instant::now();
        while f.editor.tour_visuals.check_rx.is_some() {
            f.editor.poll_tour_visuals();
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(f.editor.message.contains("Tour check passed"));
        assert!(!std::fs::read_to_string(f.root.join("other.txt"))
            .unwrap()
            .contains("unsaved"));
    }
}
