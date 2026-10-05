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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    pub text: String,
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
    let doc: DayDocument = toml::from_str(text)?;
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

impl Editor {
    pub fn task_open_day(&mut self, date: NaiveDate) {
        let result = (|| -> anyhow::Result<()> {
            let path = day_path(date)?;
            let doc = read_day(&path, date)?;
            if !path.exists() {
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
        self.task_open_day(Local::now().date_naive());
    }
    pub fn task_yesterday(&mut self) {
        self.task_open_day(Local::now().date_naive() - Duration::days(1));
    }

    pub fn task_week(&mut self) {
        let today = Local::now().date_naive();
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
            entries.extend(doc.entries.into_iter().map(|entry| {
                let time = entry.time.as_deref().unwrap_or("");
                let label = format!(
                    "{date} {time} [{} / {}] {}",
                    entry.kind,
                    entry.status,
                    entry.text.lines().next().unwrap_or("")
                );
                let mut result = Entry::location(path.clone(), 0, 0, label);
                result.detail = format!("Recorded {}\n{}", entry.created_at, entry.text);
                result
            }));
        }
        if entries.is_empty() {
            self.set_message(format!(
                "No tracker entries for {monday} through {}",
                monday + Duration::days(6)
            ));
        } else {
            self.show_results(Results::new(
                format!("Tasks — {monday} through {}", monday + Duration::days(6)),
                entries,
            ));
        }
    }

    pub fn task_add(&mut self, raw: &str, kind: &str) {
        let parts = raw.trim().split_whitespace().collect::<Vec<_>>();
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
        let text = parts[first_text..].join(" ");
        if text.is_empty() {
            self.set_message("Usage: :taskadd [YYYY-MM-DD] [HH:MM] <text>");
            return;
        }
        let result = self.task_add_record(date, time, &text, kind);
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
            time,
            text: text.to_string(),
        });
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
