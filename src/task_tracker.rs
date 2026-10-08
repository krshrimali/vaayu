//! Personal date based task and activity tracker.
//!
//! Each day is a private, structured TOML document under the user's data
//! directory. Opening one as a normal buffer keeps editing familiar while
//! `:today`, `:ystd`, and `:week` provide date based navigation.
use crate::{
    editor::Editor,
    results::{Entry, Results},
};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DayDocument {
    pub version: u32,
    pub date: String,
    #[serde(default)]
    pub entries: Vec<DayEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DayEntry {
    pub id: u64,
    /// When this entry was captured; independent of its scheduled date/time.
    pub created_at: String,
    pub kind: String,
    pub status: String,
    /// Higher values are shown first. Legacy documents without this field
    /// are read as priority 1 and gain the explicit field on their next save.
    #[serde(default = "default_priority")]
    pub priority: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Stable source identity used to finish a rollover safely after a crash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollover_id: Option<String>,
}

fn default_priority() -> i64 {
    1
}

fn store_dir() -> anyhow::Result<PathBuf> {
    Ok(dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not locate the user data directory"))?
        .join("vaayu")
        .join("tasks"))
}

pub fn is_task_path(path: &Path) -> bool {
    store_dir()
        .is_ok_and(|root| path.starts_with(root) && path.extension().is_some_and(|e| e == "toml"))
}

fn day_path(date: NaiveDate) -> anyhow::Result<PathBuf> {
    Ok(store_dir()?.join(format!("{date}.toml")))
}

fn empty_day(date: NaiveDate) -> DayDocument {
    DayDocument {
        version: 1,
        date: date.to_string(),
        entries: Vec::new(),
    }
}

fn parse_document(text: &str, expected: NaiveDate) -> anyhow::Result<DayDocument> {
    let mut doc: DayDocument = toml::from_str(text)?;
    anyhow::ensure!(doc.version == 1, "unsupported task document version");
    anyhow::ensure!(
        doc.date == expected.to_string(),
        "task document date does not match its filename"
    );
    let mut ids = std::collections::HashSet::new();
    for entry in &doc.entries {
        anyhow::ensure!(ids.insert(entry.id), "duplicate task entry id {}", entry.id);
        DateTime::parse_from_rfc3339(&entry.created_at)
            .map_err(|e| anyhow::anyhow!("entry {} has invalid created_at: {e}", entry.id))?;
        anyhow::ensure!(
            matches!(entry.kind.as_str(), "task" | "note"),
            "entry {} has invalid kind",
            entry.id
        );
        anyhow::ensure!(
            matches!(entry.status.as_str(), "open" | "done" | "logged"),
            "entry {} has invalid status",
            entry.id
        );
        if let Some(time) = &entry.time {
            NaiveTime::parse_from_str(time, "%H:%M")
                .map_err(|e| anyhow::anyhow!("entry {} has invalid time: {e}", entry.id))?;
        }
    }
    doc.entries
        .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    Ok(doc)
}

fn encode(doc: &DayDocument) -> anyhow::Result<String> {
    let text = toml::to_string_pretty(doc)?;
    Ok(if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    })
}

fn parse_task_draft(source: &str) -> anyhow::Result<(i64, String, Option<String>)> {
    let mut lines = source.lines();
    let priority = lines
        .next()
        .and_then(|line| line.trim().strip_prefix("Priority:"))
        .map(str::trim)
        .ok_or_else(|| anyhow::anyhow!("draft must start with Priority: <integer>"))?
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("priority must be an integer"))?;
    let mut task_lines = Vec::new();
    let mut notes_lines = Vec::new();
    let mut in_notes = false;
    let first_task = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("draft is missing Task:"))?;
    let first_task = first_task
        .trim_start()
        .strip_prefix("Task:")
        .map(str::trim_start)
        .ok_or_else(|| anyhow::anyhow!("draft must have a Task: line"))?;
    task_lines.push(first_task.to_string());
    for line in lines {
        if !in_notes && line.trim_start().starts_with("Notes:") {
            in_notes = true;
            let inline_notes = line
                .trim_start()
                .strip_prefix("Notes:")
                .expect("starts_with checked above")
                .trim_start();
            if !inline_notes.is_empty() {
                notes_lines.push(inline_notes);
            }
            continue;
        }
        if in_notes {
            notes_lines.push(line);
        } else {
            task_lines.push(line.to_string());
        }
    }
    // Notes are optional. If the user removes the template separator while
    // editing, keep the remaining lines as part of the task instead of
    // making the draft impossible to save.
    let task = task_lines.join("\n").trim_end().to_string();
    if in_notes {
        let notes = notes_lines.join("\n");
        Ok((priority, task, (!notes.trim().is_empty()).then_some(notes)))
    } else {
        Ok((priority, task, None))
    }
}

fn sort_result_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        let priority = |text: &str| {
            text.find("[P")
                .and_then(|i| text[i + 2..].split_once(" / "))
                .and_then(|(n, _)| n.parse::<i64>().ok())
                .unwrap_or(i64::MIN)
        };
        priority(&b.text)
            .cmp(&priority(&a.text))
            .then_with(|| a.text.cmp(&b.text))
    });
}

/// Move each overdue open task to today's document. The task is removed from
/// its old day, so repeated runtime checks cannot duplicate it.
pub(crate) fn rollover_overdue(today: NaiveDate) -> anyhow::Result<()> {
    let root = store_dir()?;
    let mut days = Vec::new();
    match std::fs::read_dir(&root) {
        Ok(items) => {
            for item in items {
                let path = item?.path();
                let Some(date) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                else {
                    continue;
                };
                if date < today && path.extension().is_some_and(|e| e == "toml") {
                    days.push((date, path));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    let today_path = day_path(today)?;
    for (date, path) in days {
        let old_text = std::fs::read_to_string(&path)?;
        let mut old = parse_document(&old_text, date)?;
        let moving = old
            .entries
            .iter()
            .filter(|e| e.kind == "task" && e.status == "open")
            .cloned()
            .collect::<Vec<_>>();
        if moving.is_empty() {
            continue;
        }
        let today_text = std::fs::read_to_string(&today_path).ok();
        let mut current = match today_text.as_deref() {
            Some(s) => parse_document(s, today)?,
            None => empty_day(today),
        };
        for mut task in moving {
            let rollover_id = format!("{date}:{}", task.id);
            let duplicate = current
                .entries
                .iter()
                .any(|e| e.rollover_id.as_deref() == Some(rollover_id.as_str()));
            old.entries
                .retain(|e| !(e.id == task.id && e.created_at == task.created_at));
            if duplicate {
                continue;
            }
            task.rollover_id = Some(rollover_id);
            if current.entries.iter().any(|e| e.id == task.id) {
                task.id = current.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
            }
            current.entries.push(task);
        }
        current
            .entries
            .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        write_day(&today_path, &current, today_text.as_deref())?;
        write_day(&path, &old, Some(&old_text))?;
    }
    Ok(())
}

fn read_day(path: &Path, date: NaiveDate) -> anyhow::Result<DayDocument> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        anyhow::bail!("task document is a symlink");
    }
    match std::fs::read_to_string(path) {
        Ok(text) => parse_document(&text, date),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(empty_day(date)),
        Err(e) => Err(e.into()),
    }
}

fn write_day(path: &Path, doc: &DayDocument, expected: Option<&str>) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid task path"))?;
    if let Some(app_dir) = dir.parent() {
        std::fs::create_dir_all(app_dir)?;
        anyhow::ensure!(
            !std::fs::symlink_metadata(app_dir)?.file_type().is_symlink(),
            "task data directory is a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(app_dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let _lock = crate::files::private_lock(dir, "tasks.lock")?;
    anyhow::ensure!(
        !std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
        "task document is a symlink"
    );
    let actual = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(
        actual.as_deref() == expected,
        "task document changed on disk; reload before editing"
    );
    crate::files::atomic_write(path, encode(doc)?.as_bytes(), true)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTarget {
    path: PathBuf,
    id: u64,
    created_at: String,
}
impl TaskTarget {
    pub fn from_result(entry: &Entry) -> Option<Self> {
        let mut detail = entry.detail.lines();
        Some(Self {
            path: entry.path.clone()?,
            id: detail.next()?.strip_prefix("task-id=")?.parse().ok()?,
            created_at: detail.next()?.strip_prefix("Recorded ")?.to_string(),
        })
    }
    fn date(&self) -> anyhow::Result<NaiveDate> {
        self.path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .ok_or_else(|| anyhow::anyhow!("selected row is not a task entry"))
    }
    fn matches(&self, entry: &DayEntry) -> bool {
        entry.id == self.id && entry.created_at == self.created_at
    }
}

#[derive(Clone, Debug)]
pub struct TaskDeletePrompt {
    target: TaskTarget,
    snapshot: String,
    pub text: String,
}

fn result_entry(path: &Path, date: NaiveDate, entry: &DayEntry) -> Entry {
    let time = entry.time.as_deref().unwrap_or("");
    let label = format!(
        "{date} {time} [P{} / {} / {}] {}",
        entry.priority,
        entry.kind,
        entry.status,
        entry.text.lines().next().unwrap_or("")
    );
    let mut result = Entry::location(path.to_path_buf(), 0, 0, label);
    result.no_path_prefix = true;
    result.detail = format!(
        "task-id={}\nRecorded {}\n{}{}",
        entry.id,
        entry.created_at,
        entry.text,
        entry
            .notes
            .as_deref()
            .map(|n| format!("\n\nNotes:\n{n}"))
            .unwrap_or_default()
    );
    result
}

impl Editor {
    /// Open a clean text buffer for composing one task. Saving the draft turns
    /// the plain text into a validated record in its date's structured file.
    pub fn task_draft_open(&mut self, date: NaiveDate, time: Option<String>) {
        let mut buffer = crate::buffer::Buffer::empty();
        buffer.rope = ropey::Rope::new();
        buffer.mark_saved();
        self.buffers.push(buffer);
        self.cur = self.buffers.len() - 1;
        let id = self.buf().id;
        self.task_draft = Some((id, date, time));
        self.invalidate_index_caches();
        self.set_cursor(0, 0);
        let template = "Priority: 1\nTask: \nNotes:\n";
        self.buf_mut().insert_str_at(0, template);
        self.buf_mut().mark_saved();
        self.enter_insert();
        self.set_cursor_insert(1, 6);
        self.set_message(
            "Task draft · edit priority, task and optional multiline notes; :wq saves",
        );
    }

    pub fn is_task_draft_buffer(&self) -> bool {
        self.task_draft
            .as_ref()
            .is_some_and(|(id, _, _)| *id == self.buf().id)
    }

    /// Save the active task draft, remove its scratch buffer, and open the
    /// saved day's task view. On a write error, the draft remains intact.
    pub fn save_task_draft(&mut self) -> anyhow::Result<()> {
        let (id, date, time) = self
            .task_draft
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no task draft is active"))?;
        anyhow::ensure!(self.buf().id == id, "task draft is not the active buffer");
        let text = self.buf().rope.to_string();
        let (priority, task, notes) = parse_task_draft(&text)?;
        anyhow::ensure!(
            !task.trim().is_empty(),
            "task text is empty; draft was kept"
        );
        self.task_add_record(date, time, task.trim(), "task", priority, notes.as_deref())?;
        // If the day's buffer already had edits, task_add_record preserves
        // them and deliberately leaves the combined document dirty. `:wq`
        // on this composer must still persist the new task, so finish that
        // same validated/conflict-checked save here.
        if self.buf().is_modified() && self.buf().path.as_deref().is_some_and(is_task_path) {
            self.save_current()?;
        }

        if let Some(index) = self.buffers.iter().position(|buffer| buffer.id == id) {
            self.buffers.remove(index);
            if index < self.cur {
                self.cur -= 1;
            }
        }
        let replacement = self.buf().id;
        for window in &mut self.windows {
            if window.buffer == id {
                window.buffer = replacement;
                window.cursor = (0, 0);
                window.top = 0;
                window.wrap_row = 0;
                window.left = 0;
            }
        }
        self.buffer_mru.retain(|buffer| *buffer != id);
        self.task_draft = None;
        self.invalidate_index_caches();
        self.enter_normal();
        Ok(())
    }

    /// Mark the task block containing the cursor as done and persist it.
    /// The cursor must be inside the desired `[[entries]]` block in a day file.
    pub fn task_done(&mut self) {
        if self.mode == crate::mode::Mode::Results {
            self.task_done_from_results();
            return;
        }
        let Some(path) = self.buf().path.clone().filter(|p| is_task_path(p)) else {
            self.set_message(":taskdone works in a daily task document");
            return;
        };
        let Some(date) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        else {
            self.set_message("task tracker: invalid daily task filename");
            return;
        };
        let source = self.buf().rope.to_string();
        let cursor_line = self.cursor().0;
        let lines = source.lines().collect::<Vec<_>>();
        let sections = lines
            .iter()
            .enumerate()
            .filter_map(|(line_no, line)| (line.trim() == "[[entries]]").then_some(line_no))
            .collect::<Vec<_>>();
        let Some(section_index) = sections.iter().rposition(|line_no| *line_no <= cursor_line)
        else {
            self.set_message("Place the cursor inside a task entry first");
            return;
        };
        let section_start = sections[section_index];
        let section_end = sections
            .get(section_index + 1)
            .copied()
            .unwrap_or(lines.len());
        let id = lines[section_start + 1..section_end]
            .iter()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("id = ")
                    .and_then(|value| value.parse::<u64>().ok())
            });
        let Some(id) = id else {
            self.set_message("Place the cursor inside a task entry first");
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            let mut doc = parse_document(&source, date)?;
            let entry = doc
                .entries
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| anyhow::anyhow!("task entry {id} no longer exists"))?;
            anyhow::ensure!(
                entry.kind == "task",
                "activity notes cannot be closed as tasks"
            );
            anyhow::ensure!(entry.status == "open", "task is already {}", entry.status);
            entry.status = "done".into();
            let encoded = encode(&doc)?;
            self.buf_mut().begin_edit();
            let len = self.buf().rope.len_chars();
            self.buf_mut().delete_char_range(0, len);
            self.buf_mut().insert_str_at(0, &encoded);
            self.buf_mut().commit_edit();
            self.save_current()?;
            Ok(())
        })();
        match result {
            Ok(()) => self.set_message("Task marked done"),
            Err(e) => self.set_message(format!("task tracker: {e}")),
        }
    }

    fn task_done_from_results(&mut self) {
        self.task_status_from_results("done");
    }

    pub fn task_open_from_results(&mut self) {
        self.task_status_from_results("open");
    }

    fn current_task_target(&self) -> anyhow::Result<TaskTarget> {
        self.results
            .as_ref()
            .and_then(|r| r.entries.get(r.cursor))
            .and_then(TaskTarget::from_result)
            .ok_or_else(|| anyhow::anyhow!("select a task row first"))
    }

    fn ensure_task_buffer_clean(&self, path: &Path) -> anyhow::Result<()> {
        let identity = crate::files::identity(path);
        anyhow::ensure!(
            !self
                .buffers
                .iter()
                .any(|b| b.path.as_ref() == Some(&identity) && b.is_modified()),
            "save or discard the open task document's edits first"
        );
        Ok(())
    }

    fn refresh_task_buffer(&mut self, path: &Path) {
        let identity = crate::files::identity(path);
        for buffer in &mut self.buffers {
            if buffer.path.as_ref() == Some(&identity) && buffer.reload().is_err() {
                buffer.disk_changed = true;
            }
        }
    }

    // Update the producer's complete list before filtering again, so toggling
    // a task or clearing a filter cannot resurrect an old status/deleted row.
    fn update_task_result(&mut self, target: &TaskTarget, replacement: Option<Entry>) {
        let Some(r) = self.results.as_mut() else {
            return;
        };
        let cursor_target = r.entries.get(r.cursor).and_then(TaskTarget::from_result);
        let selected_targets = r
            .selected
            .iter()
            .filter_map(|i| r.entries.get(*i).and_then(TaskTarget::from_result))
            .collect::<Vec<_>>();
        r.all_entries.retain_mut(|entry| {
            if TaskTarget::from_result(entry).as_ref() != Some(target) {
                return true;
            }
            if let Some(replacement) = &replacement {
                *entry = replacement.clone();
                true
            } else {
                false
            }
        });
        r.apply_filter();
        r.selected = r
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                TaskTarget::from_result(entry)
                    .is_some_and(|target| selected_targets.contains(&target))
                    .then_some(i)
            })
            .collect();
        if let Some(cursor) = r
            .entries
            .iter()
            .position(|e| TaskTarget::from_result(e) == cursor_target)
        {
            r.move_cursor(cursor);
        }
    }

    fn task_status_from_results(&mut self, status: &str) {
        let result = (|| -> anyhow::Result<()> {
            let target = self.current_task_target()?;
            self.ensure_task_buffer_clean(&target.path)?;
            let date = target.date()?;
            let raw = std::fs::read_to_string(&target.path)?;
            let mut doc = parse_document(&raw, date)?;
            let entry = doc
                .entries
                .iter_mut()
                .find(|entry| target.matches(entry))
                .ok_or_else(|| anyhow::anyhow!("task no longer exists"))?;
            anyhow::ensure!(entry.kind == "task", "selected entry is not a task");
            entry.status = status.into();
            let replacement = result_entry(&target.path, date, entry);
            write_day(&target.path, &doc, Some(&raw))?;
            self.refresh_task_buffer(&target.path);
            self.update_task_result(&target, Some(replacement));
            Ok(())
        })();
        match result {
            Ok(()) => self.set_message(format!("Task marked {status}")),
            Err(e) => self.set_message(format!("task tracker: {e}")),
        }
    }

    pub fn task_delete_prompt(&mut self) {
        let result = (|| -> anyhow::Result<TaskDeletePrompt> {
            let target = self.current_task_target()?;
            self.ensure_task_buffer_clean(&target.path)?;
            let snapshot = std::fs::read_to_string(&target.path)?;
            let doc = parse_document(&snapshot, target.date()?)?;
            let entry = doc
                .entries
                .iter()
                .find(|entry| target.matches(entry))
                .ok_or_else(|| anyhow::anyhow!("task entry no longer exists"))?;
            let text = format!(
                "Delete {} \"{}\"? [y]es / [n]o",
                entry.kind,
                entry.text.lines().next().unwrap_or("")
            );
            Ok(TaskDeletePrompt {
                target,
                snapshot,
                text,
            })
        })();
        match result {
            Ok(prompt) => self.results.as_mut().unwrap().delete_prompt = Some(prompt),
            Err(e) => self.set_message(format!("task tracker: {e}")),
        }
    }

    pub fn task_delete_confirm(&mut self) {
        let Some(prompt) = self.results.as_mut().and_then(|r| r.delete_prompt.take()) else {
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            self.ensure_task_buffer_clean(&prompt.target.path)?;
            let mut doc = parse_document(&prompt.snapshot, prompt.target.date()?)?;
            doc.entries.retain(|entry| !prompt.target.matches(entry));
            write_day(&prompt.target.path, &doc, Some(&prompt.snapshot))?;
            self.refresh_task_buffer(&prompt.target.path);
            self.update_task_result(&prompt.target, None);
            Ok(())
        })();
        match result {
            Ok(()) => self.set_message("Task entry deleted"),
            Err(e) => self.set_message(format!("task tracker: {e}")),
        }
    }

    pub fn task_open_day(&mut self, date: NaiveDate) {
        let result = (|| -> anyhow::Result<()> {
            let path = day_path(date)?;
            let doc = read_day(&path, date)?;
            if path.exists() {
                let already_open = self
                    .buffers
                    .iter()
                    .any(|buffer| buffer.path.as_ref() == Some(&crate::files::identity(&path)));
                if !already_open {
                    let raw = std::fs::read_to_string(&path)?;
                    let normalized = encode(&doc)?;
                    if raw != normalized {
                        write_day(&path, &doc, Some(&raw))?;
                    }
                }
            } else {
                write_day(&path, &doc, None)?;
            }
            self.open_file(path)
        })();
        match result {
            Ok(()) => self.set_message(format!("Tasks for {date}")),
            Err(e) => self.set_message(format!("task tracker: {e}")),
        }
    }

    pub fn task_today(&mut self) {
        let today = Local::now().date_naive();
        if let Err(e) = rollover_overdue(today) {
            self.set_message(format!("task tracker: {e}"));
            return;
        }
        self.task_open_day(today);
    }
    pub fn task_yesterday(&mut self) {
        self.task_open_day(Local::now().date_naive() - Duration::days(1));
    }

    pub fn task_week(&mut self) {
        let today = Local::now().date_naive();
        if let Err(e) = rollover_overdue(today) {
            self.set_message(format!("task tracker: {e}"));
            return;
        }
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let mut entries = Vec::new();
        for offset in 0..7 {
            let date = monday + Duration::days(offset);
            let path = match day_path(date) {
                Ok(path) => path,
                Err(e) => {
                    self.set_message(format!("task tracker: {e}"));
                    return;
                }
            };
            let doc = match read_day(&path, date) {
                Ok(doc) => doc,
                Err(e) => {
                    self.set_message(format!("task tracker: {e}"));
                    return;
                }
            };
            entries.extend(
                doc.entries
                    .iter()
                    .map(|entry| result_entry(&path, date, entry)),
            );
        }
        sort_result_entries(&mut entries);
        if entries.is_empty() {
            self.set_message(format!(
                "No tracker entries for {monday} through {}",
                monday + Duration::days(6)
            ));
        } else {
            let mut results = Results::new(
                format!("Tasks — {monday} through {}", monday + Duration::days(6)),
                entries,
            );
            results.task_list = true;
            self.show_results(results);
        }
    }

    /// Browse every saved day, including dates in the future. `:week` is a
    /// deliberately narrow Monday–Sunday view; this is the unbounded list.
    pub fn task_list(&mut self) {
        self.task_list_filtered(false);
    }

    pub fn task_notes(&mut self) {
        self.task_list_filtered(true);
    }

    fn task_list_filtered(&mut self, notes_only: bool) {
        if let Err(e) = rollover_overdue(Local::now().date_naive()) {
            self.set_message(format!("task tracker: {e}"));
            return;
        }
        let result = (|| -> anyhow::Result<Vec<Entry>> {
            let root = store_dir()?;
            let mut days = Vec::new();
            match std::fs::read_dir(&root) {
                Ok(items) => {
                    for item in items {
                        let path = item?.path();
                        if !path.extension().is_some_and(|e| e == "toml") {
                            continue;
                        }
                        let Some(date) = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                        else {
                            continue;
                        };
                        days.push((date, path));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            days.sort_by_key(|(date, _)| *date);
            let mut entries = Vec::new();
            for (date, path) in days {
                let doc = read_day(&path, date)?;
                for entry in doc.entries {
                    if !notes_only || entry.kind == "note" {
                        entries.push(result_entry(&path, date, &entry));
                    }
                }
            }
            sort_result_entries(&mut entries);
            Ok(entries)
        })();
        match result {
            Ok(entries) if entries.is_empty() && !notes_only => {
                self.set_message("No saved task entries yet — use :taskadd <text>");
            }
            Ok(entries) => {
                let mut results = Results::new(
                    if notes_only {
                        "Activity notes"
                    } else {
                        "All tasks and activity"
                    },
                    entries,
                );
                results.task_list = true;
                self.show_results(results);
                if notes_only && self.results.as_ref().is_some_and(|r| r.entries.is_empty()) {
                    self.set_message("No activity notes yet — use :tasknote <text>");
                }
            }
            Err(e) => self.set_message(format!("task tracker: {e}")),
        }
    }

    pub fn task_add(&mut self, raw: &str, kind: &str) {
        if kind == "task" {
            if let Err(e) = rollover_overdue(Local::now().date_naive()) {
                self.set_message(format!("task tracker: {e}"));
                return;
            }
        }
        let parts = raw.split_whitespace().collect::<Vec<_>>();
        let mut date = Local::now().date_naive();
        let mut time = None;
        let mut first_text = 0;
        if parts
            .first()
            .is_some_and(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
        {
            date = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d").unwrap();
            first_text += 1;
        }
        if parts
            .get(first_text)
            .is_some_and(|s| NaiveTime::parse_from_str(s, "%H:%M").is_ok())
        {
            time = Some(parts[first_text].to_string());
            first_text += 1;
        }
        let mut priority = default_priority();
        if parts
            .get(first_text)
            .is_some_and(|s| s.starts_with("--priority="))
        {
            let Some(value) = parts[first_text]
                .strip_prefix("--priority=")
                .and_then(|v| v.parse::<i64>().ok())
            else {
                self.set_message("Priority must be an integer");
                return;
            };
            priority = value;
            first_text += 1;
        }
        let text = parts[first_text..].join(" ");
        if text.is_empty() && kind == "task" {
            self.task_draft_open(date, time);
            return;
        }
        if text.is_empty() {
            self.set_message("Usage: :taskadd [YYYY-MM-DD] [HH:MM] <text>");
            return;
        }
        let result = self.task_add_record(date, time, &text, kind, priority, None);
        match result {
            Ok(()) => self.set_message(format!("Added {kind} for {date}")),
            Err(e) => self.set_message(format!("task tracker: {e}")),
        }
    }

    fn task_add_record(
        &mut self,
        date: NaiveDate,
        time: Option<String>,
        text: &str,
        kind: &str,
        priority: i64,
        notes: Option<&str>,
    ) -> anyhow::Result<()> {
        let path = day_path(date)?;
        let existing = self
            .buffers
            .iter()
            .position(|b| b.path.as_ref() == Some(&crate::files::identity(&path)));
        let disk_snapshot = if existing.is_none() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        };
        let mut doc = if let Some(index) = existing {
            parse_document(&self.buffers[index].rope.to_string(), date)?
        } else {
            match disk_snapshot.as_deref() {
                Some(text) => parse_document(text, date)?,
                None => empty_day(date),
            }
        };
        let id = doc.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        let created_at = Local::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        doc.entries.push(DayEntry {
            id,
            created_at,
            kind: kind.to_string(),
            status: if kind == "note" { "logged" } else { "open" }.to_string(),
            priority,
            time,
            text: text.to_string(),
            notes: notes.map(str::to_string).filter(|s| !s.trim().is_empty()),
            rollover_id: None,
        });
        doc.entries
            .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        let encoded = encode(&doc)?;
        if let Some(index) = existing {
            let current = self.cur;
            let was_modified = self.buffers[index].is_modified();
            self.cur = index;
            self.buf_mut().begin_edit();
            let len = self.buf().rope.len_chars();
            self.buf_mut().delete_char_range(0, len);
            self.buf_mut().insert_str_at(0, &encoded);
            self.buf_mut().commit_edit();
            let last_line = self.buf().rope.len_lines().saturating_sub(1);
            self.set_cursor(last_line, 0);
            let saved = if !was_modified {
                // Clean buffers can be refreshed and persisted immediately.
                self.save_current()
            } else {
                // The new entry is present in the editable buffer alongside
                // the user's edits; saving remains explicit.
                Ok(())
            };
            self.cur = current;
            saved?;
            self.open_file(path)?;
        } else {
            write_day(&path, &doc, disk_snapshot.as_deref())?;
            self.open_file(path)?;
        }
        Ok(())
    }

    pub fn validate_task_buffer(&self, path: &Path, text: &str) -> anyhow::Result<()> {
        let date = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .ok_or_else(|| anyhow::anyhow!("invalid task date in filename"))?;
        parse_document(text, date)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{buffer::Buffer, config::Config, key::Key};
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

    struct TaskFixture {
        editor: Editor,
        root: PathBuf,
        path: PathBuf,
        date: NaiveDate,
    }
    impl TaskFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "vaayu-task-list-{}-{}",
                std::process::id(),
                FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let store = root.join("tasks");
            std::fs::create_dir_all(&store).unwrap();
            let date = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
            let path = store.join(format!("{date}.toml"));
            let entries = [
                (1, "task", "open", "Ship / open] feature"),
                (2, "task", "done", "Already done"),
                (3, "note", "logged", "Activity note"),
            ]
            .map(|(id, kind, status, text)| DayEntry {
                id,
                kind: kind.into(),
                status: status.into(),
                text: text.into(),
                created_at: format!("2026-10-07T10:00:0{id}Z"),
                priority: 1,
                time: None,
                notes: (id == 1).then(|| "FIRST_ATTACHED_NOTE\nSECOND_ATTACHED_NOTE".into()),
                rollover_id: None,
            })
            .to_vec();
            std::fs::write(
                &path,
                encode(&DayDocument {
                    version: 1,
                    date: date.to_string(),
                    entries: entries.clone(),
                })
                .unwrap(),
            )
            .unwrap();
            let mut editor = Editor::new(Config {
                clipboard_unnamedplus: false,
                ..Config::default()
            });
            let mut results = Results::new(
                "Tasks",
                entries
                    .iter()
                    .map(|entry| result_entry(&path, date, entry))
                    .collect(),
            );
            results.task_list = true;
            editor.show_results(results);
            Self {
                editor,
                root,
                path,
                date,
            }
        }
        fn document(&self) -> DayDocument {
            read_day(&self.path, self.date).unwrap()
        }
        fn keys(&mut self, text: &str) {
            for c in text.chars() {
                self.editor.feed_key(Key::Char(c));
            }
        }
    }
    impl Drop for TaskFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn list_done_and_open_persist_through_filters_without_changing_task_text() {
        let mut fixture = TaskFixture::new();
        fixture.editor.results.as_mut().unwrap().filter = "Ship".into();
        fixture.editor.results.as_mut().unwrap().apply_filter();
        fixture.editor.results.as_mut().unwrap().selected.insert(0);
        fixture.keys("D");
        assert_eq!(fixture.document().entries[0].status, "done");
        assert_eq!(fixture.document().entries[0].text, "Ship / open] feature");
        assert!(fixture
            .editor
            .results
            .as_ref()
            .unwrap()
            .selected
            .contains(&0));
        let r = fixture.editor.results.as_mut().unwrap();
        r.filter.clear();
        r.apply_filter();
        assert!(r.entries[0].text.contains("/ done]"));
        fixture.keys("O");
        assert_eq!(fixture.document().entries[0].status, "open");
        fixture.editor.results.as_mut().unwrap().filter = "/ open]".into();
        fixture.editor.results.as_mut().unwrap().apply_filter();
        fixture.keys("D");
        // Filter by the status field rather than the same phrase in task text.
        let r = fixture.editor.results.as_mut().unwrap();
        r.filter = "/ task / open]".into();
        r.apply_filter();
        assert!(r.entries.is_empty());
        r.filter.clear();
        r.apply_filter();
        assert!(r.entries[0].text.contains("/ done]"));
    }

    #[test]
    fn list_delete_requires_dd_and_yes_and_removes_filtered_source_rows() {
        let mut fixture = TaskFixture::new();
        let original = std::fs::read_to_string(&fixture.path).unwrap();
        fixture.keys("d");
        assert!(fixture
            .editor
            .results
            .as_ref()
            .unwrap()
            .delete_prompt
            .is_none());
        fixture.keys("jdd");
        assert!(fixture
            .editor
            .results
            .as_ref()
            .unwrap()
            .delete_prompt
            .is_some());
        fixture.editor.feed_key(Key::Ctrl('q'));
        fixture.keys("D?");
        assert!(!fixture.editor.results.as_ref().unwrap().quickfix);
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), original);
        fixture.keys("n");
        assert!(fixture
            .editor
            .results
            .as_ref()
            .unwrap()
            .delete_prompt
            .is_none());
        fixture.keys("dd");
        fixture.editor.feed_key(Key::Esc);
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), original);
        fixture.editor.results.as_mut().unwrap().filter = "Already".into();
        fixture.editor.results.as_mut().unwrap().apply_filter();
        fixture.keys("ddy");
        assert!(!fixture.document().entries.iter().any(|entry| entry.id == 2));
        let r = fixture.editor.results.as_mut().unwrap();
        assert!(r.entries.is_empty());
        r.filter.clear();
        r.apply_filter();
        assert_eq!(r.entries.len(), 2);
        assert!(r.cursor < r.entries.len());
        assert!(!r.entries.iter().any(|e| e.text.contains("Already done")));
    }

    #[test]
    fn list_delete_rejects_documents_changed_after_confirmation_prompt() {
        let mut fixture = TaskFixture::new();
        fixture.keys("dd");
        let mut changed = fixture.document();
        changed.entries[0].text = "Changed by another process".into();
        let changed = encode(&changed).unwrap();
        std::fs::write(&fixture.path, &changed).unwrap();
        fixture.keys("y");
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), changed);
        assert!(fixture.editor.message.contains("changed on disk"));
        assert_eq!(fixture.editor.results.as_ref().unwrap().entries.len(), 3);
    }

    #[test]
    fn list_status_rejects_activity_notes_and_reused_task_ids() {
        let mut fixture = TaskFixture::new();
        fixture.editor.results.as_mut().unwrap().cursor = 2;
        let original = std::fs::read_to_string(&fixture.path).unwrap();
        fixture.keys("DO");
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), original);
        assert!(fixture.editor.message.contains("not a task"));
        fixture.editor.results.as_mut().unwrap().cursor = 0;
        let mut changed = fixture.document();
        changed.entries[0].created_at = "2026-10-07T11:00:00Z".into();
        let changed = encode(&changed).unwrap();
        std::fs::write(&fixture.path, &changed).unwrap();
        fixture.keys("Ddd");
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), changed);
        assert!(fixture.editor.message.contains("no longer exists"));
        assert!(fixture
            .editor
            .results
            .as_ref()
            .unwrap()
            .delete_prompt
            .is_none());
    }

    #[test]
    fn list_actions_refresh_clean_buffers_and_protect_unsaved_edits() {
        let mut fixture = TaskFixture::new();
        fixture
            .editor
            .buffers
            .push(Buffer::from_path(fixture.path.clone()).unwrap());
        fixture.keys("D");
        let buffer = fixture.editor.buffers.last_mut().unwrap();
        assert!(buffer.rope.to_string().contains("status = \"done\""));
        assert!(!buffer.is_modified());
        assert!(!buffer.changed_on_disk());
        buffer.begin_edit();
        buffer.insert_str_at(0, "# Unsaved edit\n");
        buffer.commit_edit();
        let original = std::fs::read_to_string(&fixture.path).unwrap();
        fixture.keys("Odd");
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), original);
        assert!(fixture.editor.message.contains("edits first"));
        assert!(fixture
            .editor
            .results
            .as_ref()
            .unwrap()
            .delete_prompt
            .is_none());
        assert!(fixture
            .editor
            .buffers
            .last()
            .unwrap()
            .rope
            .to_string()
            .contains("Unsaved edit"));
        fixture.editor.buffers.last_mut().unwrap().reload().unwrap();
        fixture.keys("ddy");
        assert!(!fixture
            .editor
            .buffers
            .last()
            .unwrap()
            .rope
            .to_string()
            .contains("Ship"));
        assert!(!fixture.editor.buffers.last().unwrap().is_modified());
    }

    #[test]
    fn selected_task_quickfix_actions_keep_only_exported_rows() {
        let mut fixture = TaskFixture::new();
        fixture.editor.results.as_mut().unwrap().selected.insert(0);
        fixture.editor.feed_key(Key::Ctrl('q'));
        fixture.keys("DO");
        assert_eq!(fixture.document().entries[0].status, "open");
        let r = fixture.editor.results.as_ref().unwrap();
        assert!(r.quickfix);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.all_entries.len(), 1);
        fixture.keys("ddy");
        assert_eq!(fixture.document().entries.len(), 2);
        assert!(fixture.editor.results.as_ref().unwrap().entries.is_empty());
        fixture.keys("?");
        assert!(fixture.editor.results.as_ref().unwrap().show_help);
        fixture.editor.feed_key(Key::Esc);
        fixture.keys("DOdd");
        assert_eq!(fixture.document().entries.len(), 2);
        assert!(fixture.editor.message.contains("select a task row"));
    }

    #[test]
    fn list_details_show_attached_notes_and_help_displays_task_keys() {
        let mut fixture = TaskFixture::new();
        let mut cache = crate::render::FrameCache::new();
        let mut output = Vec::new();
        crate::render::draw(&mut output, &fixture.editor, 100, 24, &mut cache).unwrap();
        let screen = String::from_utf8(output).unwrap();
        assert!(screen.contains("FIRST_ATTACHED_NOTE"));
        assert!(screen.contains("SECOND_ATTACHED_NOTE"));
        assert!(!screen.contains("task-id="));
        fixture.keys("?");
        let mut output = Vec::new();
        crate::render::draw(&mut output, &fixture.editor, 100, 24, &mut cache).unwrap();
        let screen = String::from_utf8(output).unwrap();
        assert!(screen.contains("Tasks — keymaps"));
        assert!(screen.contains("Mark the current task done"));
        assert!(screen.contains("Mark the current task open"));
        assert!(screen.contains("yes/no confirmation"));
    }

    #[test]
    fn draft_requires_integer_priority_and_preserves_multiline_notes() {
        let (priority, task, notes) =
            parse_task_draft("Priority: 8\nTask: Ship feature\nNotes:\nfirst line\nsecond line\n")
                .unwrap();
        assert_eq!(priority, 8);
        assert_eq!(task, "Ship feature");
        assert_eq!(notes.as_deref(), Some("first line\nsecond line"));
        assert!(parse_task_draft("Priority: high\nTask: x\nNotes:\n").is_err());
        assert!(parse_task_draft("Task: x\nNotes:\n").is_err());
    }

    #[test]
    fn legacy_tasks_get_priority_one_and_documents_sort_highest_first() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        let raw = format!(
            "version = 1\ndate = \"{date}\"\n\n[[entries]]\nid = 1\ncreated_at = \"2026-10-07T10:00:00Z\"\nkind = \"task\"\nstatus = \"open\"\ntext = \"low\"\n\n[[entries]]\nid = 2\ncreated_at = \"2026-10-07T11:00:00Z\"\nkind = \"task\"\nstatus = \"open\"\npriority = 9\ntext = \"high\"\n"
        );
        let doc = parse_document(&raw, date).unwrap();
        assert_eq!(doc.entries[0].text, "high");
        assert_eq!(doc.entries[0].priority, 9);
        assert_eq!(doc.entries[1].priority, 1);
    }
}
