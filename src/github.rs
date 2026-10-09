//! GitHub workspace (read-first), built on the `gh` CLI: PR list, a PR's
//! overview / checkout / diff / review threads, CI checks and their logs,
//! and the issue list. Every view is an ordinary `Results` list (the same
//! choice `:gitstatus` made), each `gh` call runs on a background thread
//! (they all hit the network), and a missing `gh`, a logged-out `gh`, or a
//! project without a GitHub remote each degrade to one clear message
//! rather than a raw stderr dump. Nothing here writes to GitHub; the one
//! local side effect is `gh pr checkout`.
use crate::{
    editor::Editor,
    results::{Entry, Results},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
};

/// What a background `gh` job hands back to the main thread: a list to
/// show, or the outcome of `gh pr checkout` (which needs the open buffers
/// reloaded afterwards, so it can't just be another list).
pub enum GhDone {
    Show(Result<Box<Results>, String>),
    CheckedOut(Result<String, String>),
}
pub struct GhTask {
    pub(crate) receiver: mpsc::Receiver<GhDone>,
    pub(crate) checkout: bool,
    pub(crate) ready: Option<GhDone>,
}

fn show(r: Result<Results, String>) -> GhDone {
    GhDone::Show(r.map(Box::new))
}

pub const NO_GH: &str =
    "GitHub: the `gh` CLI isn't installed (https://cli.github.com) -- the GitHub workspace needs it";
pub const NO_AUTH: &str = "GitHub: `gh` isn't logged in -- run `gh auth login` in a terminal";
pub const NO_REMOTE: &str = "GitHub: this project has no GitHub remote";
pub const NO_PR: &str = "GitHub: no pull request for the current branch (pass a number)";

/// Maps a failed `gh` run's stderr to the message shown: the three
/// expected "can't use GitHub here" cases get a fixed, actionable line;
/// anything else keeps `gh`'s own first line.
fn classify_error(sub: &str, stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("gh auth login")
        || lower.contains("not logged in")
        || lower.contains("authentication required")
        || lower.contains("bad credentials")
        || lower.contains("http 401")
    {
        return NO_AUTH.to_string();
    }
    if lower.contains("no git remotes")
        || lower.contains("none of the git remotes")
        || lower.contains("not a git repository")
        || lower.contains("unable to determine base repository")
    {
        return NO_REMOTE.to_string();
    }
    if lower.contains("no pull requests found for branch") {
        return NO_PR.to_string();
    }
    let first = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("failed");
    format!("gh {sub}: {first}")
}

/// Runs `gh <args>` in `root` with prompts, pagers, colors and update
/// checks off. `Ok(stdout)` on success; with `lenient`, a non-zero exit
/// that still printed something on stdout counts as success too (`gh pr
/// checks` exits 1/8 for failing/pending checks but prints the JSON).
fn gh(root: &Path, args: &[&str], lenient: bool) -> Result<String, String> {
    let out = Command::new("gh")
        .args(args)
        .current_dir(root)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PAGER", "cat")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                NO_GH.to_string()
            } else {
                format!("gh: {e}")
            }
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() || (lenient && !stdout.trim().is_empty()) {
        Ok(stdout)
    } else {
        Err(classify_error(
            args.first().copied().unwrap_or(""),
            &String::from_utf8_lossy(&out.stderr),
        ))
    }
}

fn gh_json(root: &Path, args: &[&str], lenient: bool) -> Result<Value, String> {
    let text = gh(root, args, lenient)?;
    serde_json::from_str(&text).map_err(|e| format!("gh {}: unexpected output ({e})", args[0]))
}

/// Only an omitted argument means the current branch's PR. Invalid explicit
/// numbers must not silently select another pull request.
pub fn parse_pr_arg(arg: &str) -> Result<Option<u64>, &'static str> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Ok(None);
    }
    arg.strip_prefix('#')
        .unwrap_or(arg)
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .map(Some)
        .ok_or("expected a positive pull request number")
}

/// The PR to act on: `number` if given, else the one `gh` associates with
/// the checked-out branch.
fn resolve_pr(root: &Path, number: Option<u64>) -> Result<u64, String> {
    if let Some(n) = number {
        return Ok(n);
    }
    let v = gh_json(root, &["pr", "view", "--json", "number"], false)?;
    v["number"]
        .as_u64()
        .ok_or_else(|| "GitHub: no pull request for the current branch".to_string())
}

fn login(v: &Value) -> &str {
    v["author"]["login"]
        .as_str()
        .or_else(|| v["user"]["login"].as_str())
        .unwrap_or("?")
}

/// `gh pr list --json` rows → one entry per PR; Enter opens its overview.
pub fn pr_list_entries(list: &Value) -> Vec<Entry> {
    let rows = list.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return vec![Entry::text("No pull requests")];
    }
    rows.iter()
        .filter_map(|pr| {
            let n = pr["number"].as_u64()?;
            let mut tags = String::new();
            if pr["isDraft"].as_bool() == Some(true) {
                tags.push_str(" [draft]");
            }
            match pr["reviewDecision"].as_str() {
                Some("APPROVED") => tags.push_str(" [approved]"),
                Some("CHANGES_REQUESTED") => tags.push_str(" [changes requested]"),
                _ => {}
            }
            if let Some(state) = pr["state"].as_str().filter(|s| *s != "OPEN") {
                tags.push_str(&format!(" [{}]", state.to_lowercase()));
            }
            let mut e = Entry::text(format!(
                "#{n}  {}  · {} · {}{tags}",
                pr["title"].as_str().unwrap_or(""),
                login(pr),
                pr["headRefName"].as_str().unwrap_or(""),
            ));
            e.action = Some(serde_json::json!({ "_vaayu_gh_pr": n }));
            Some(e)
        })
        .collect()
}

/// `(passed, failed, pending)` over a `statusCheckRollup`: check runs
/// report `conclusion`, legacy commit statuses report `state`.
fn rollup_counts(rollup: &Value) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    for c in rollup.as_array().map(Vec::as_slice).unwrap_or_default() {
        let verdict = c["conclusion"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| c["state"].as_str())
            .unwrap_or("");
        match verdict {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => counts.0 += 1,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE" => counts.1 += 1,
            _ => counts.2 += 1,
        }
    }
    counts
}

/// `gh pr view --json` → the overview: a header, the action rows
/// (checkout / diff / threads / checks), then the description.
pub fn pr_view_entries(pr: &Value) -> Vec<Entry> {
    let n = pr["number"].as_u64().unwrap_or(0);
    let mut entries = vec![Entry::text(format!(
        "#{n} {}",
        pr["title"].as_str().unwrap_or("")
    ))];
    let mut state = pr["state"].as_str().unwrap_or("").to_lowercase();
    if pr["isDraft"].as_bool() == Some(true) {
        state.push_str(" draft");
    }
    let review = match pr["reviewDecision"].as_str() {
        Some("APPROVED") => " · approved",
        Some("CHANGES_REQUESTED") => " · changes requested",
        Some("REVIEW_REQUIRED") => " · review required",
        _ => "",
    };
    entries.push(Entry::text(format!(
        "{state} · {} · {} → {} · +{} −{} in {} file(s){review}",
        login(pr),
        pr["headRefName"].as_str().unwrap_or(""),
        pr["baseRefName"].as_str().unwrap_or(""),
        pr["additions"].as_u64().unwrap_or(0),
        pr["deletions"].as_u64().unwrap_or(0),
        pr["changedFiles"].as_u64().unwrap_or(0),
    )));
    if let Some(url) = pr["url"].as_str() {
        entries.push(Entry::text(url));
    }
    let (pass, fail, pending) = rollup_counts(&pr["statusCheckRollup"]);
    if pass + fail + pending > 0 {
        entries.push(Entry::text(format!(
            "checks: {pass} passed, {fail} failed, {pending} pending"
        )));
    }
    entries.push(Entry::text("── Actions ──"));
    for (label, key) in [
        ("Checkout this PR locally", "_vaayu_gh_checkout"),
        ("Diff", "_vaayu_gh_diff"),
        ("Review threads", "_vaayu_gh_threads"),
        ("CI checks", "_vaayu_gh_checks"),
    ] {
        let mut e = Entry::text(format!("  ▸ {label}"));
        e.action = Some(serde_json::json!({ key: n }));
        entries.push(e);
    }
    let body = pr["body"].as_str().unwrap_or("").trim();
    if !body.is_empty() {
        entries.push(Entry::text("── Description ──"));
        entries.extend(body.lines().map(|l| Entry::text(l.trim_end())));
    }
    entries
}

/// A unified diff covering many files → entries: each `diff --git`
/// header and preamble as plain text, every hunk line a location in the
/// local copy of that file (its `+`-side line, as `:gitdiff` maps them).
pub fn diff_entries(root: &Path, text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut sections: Vec<&str> = Vec::new();
    let mut start = 0;
    for (i, _) in text.match_indices("diff --git ") {
        if i == 0 || text.as_bytes()[i - 1] == b'\n' {
            if i > start {
                sections.push(&text[start..i]);
            }
            start = i;
        }
    }
    if start < text.len() {
        sections.push(&text[start..]);
    }
    for section in sections {
        let mut path: Option<PathBuf> = None;
        let mut in_hunk = false;
        let lines = crate::git_tools::diff_line_numbers(section);
        for (line, new_line) in section.lines().zip(lines) {
            if let Some(rest) = line.strip_prefix("+++ ") {
                path = rest.strip_prefix("b/").map(|p| root.join(p));
            } else if path.is_none() && line.starts_with("diff --git ") {
                // A pure rename / mode change has no `+++` line; fall
                // back to the header's own `b/` side.
                path = line.rsplit_once(" b/").map(|(_, p)| root.join(p));
            }
            in_hunk |= line.starts_with("@@ ");
            match (&path, in_hunk) {
                (Some(p), true) => entries.push(Entry::location(p.clone(), new_line, 0, line)),
                _ => entries.push(Entry::text(line)),
            }
        }
    }
    if entries.is_empty() {
        entries.push(Entry::text("Empty diff"));
    }
    entries
}

/// `(threads, comments)` over `pulls/N/comments`, plus the entries: one
/// location row per thread root, its body's remaining lines and every
/// reply indented under it (all jumping to the same line). `--paginate`
/// prints one JSON array per page back to back, so `pages` is a stream.
pub fn thread_entries(root: &Path, pages: &str) -> Result<(usize, usize, Vec<Entry>), String> {
    let mut comments = Vec::new();
    for page in serde_json::Deserializer::from_str(pages).into_iter::<Value>() {
        match page.map_err(|e| format!("gh api: unexpected output ({e})"))? {
            Value::Array(items) => comments.extend(items),
            other => comments.push(other),
        }
    }
    let mut roots: Vec<(u64, Vec<&Value>)> = Vec::new();
    for c in &comments {
        let id = c["id"].as_u64().unwrap_or(0);
        match c["in_reply_to_id"].as_u64() {
            Some(parent) if roots.iter().any(|(r, _)| *r == parent) => {
                if let Some((_, replies)) = roots.iter_mut().find(|(r, _)| *r == parent) {
                    replies.push(c);
                }
            }
            _ => roots.push((id, vec![c])),
        }
    }
    let mut entries = Vec::new();
    for (_, thread) in &roots {
        let first = thread[0];
        let file = first["path"].as_str().unwrap_or("");
        let path = root.join(file);
        // `line` is null once the commented code changed ("outdated");
        // `original_line` still says where it was.
        let (line, outdated) = match first["line"].as_u64() {
            Some(l) => (l, false),
            None => (first["original_line"].as_u64().unwrap_or(1), true),
        };
        let line0 = line.saturating_sub(1) as usize;
        for (i, c) in thread.iter().enumerate() {
            let body = c["body"].as_str().unwrap_or("").trim();
            let mut body_lines = body.lines();
            let head = body_lines.next().unwrap_or("");
            let text = if i == 0 {
                format!(
                    "{}{}: {head}",
                    if outdated { "(outdated) " } else { "" },
                    login(c)
                )
            } else {
                format!("    ↳ {}: {head}", login(c))
            };
            let mut e = Entry::location(path.clone(), line0, 0, text);
            e.detail = body.to_string();
            entries.push(e);
            let indent = if i == 0 { "    " } else { "      " };
            for rest in body_lines.filter(|l| !l.trim().is_empty()) {
                let mut e = Entry::location(path.clone(), line0, 0, format!("{indent}{rest}"));
                e.no_path_prefix = true;
                entries.push(e);
            }
        }
    }
    if entries.is_empty() {
        entries.push(Entry::text("No review comments"));
    }
    Ok((roots.len(), comments.len(), entries))
}

/// Sort rank and icon for a `gh pr checks` bucket: failures first.
fn bucket(b: &str) -> (u8, &'static str) {
    match b {
        "fail" => (0, "✗"),
        "pending" => (1, "●"),
        "cancel" => (2, "⊘"),
        "pass" => (3, "✓"),
        _ => (4, "−"),
    }
}

/// `gh pr checks --json` → one row per check, failing ones first; Enter
/// on a GitHub Actions check opens its log.
pub fn check_entries(checks: &Value) -> Vec<Entry> {
    let mut rows: Vec<&Value> = checks
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    if rows.is_empty() {
        return vec![Entry::text("No checks reported")];
    }
    rows.sort_by_key(|c| bucket(c["bucket"].as_str().unwrap_or("")).0);
    rows.into_iter()
        .map(|c| {
            let b = c["bucket"].as_str().unwrap_or("");
            let name = c["name"].as_str().unwrap_or("");
            let label = match c["workflow"]
                .as_str()
                .filter(|w| !w.is_empty() && *w != name)
            {
                Some(w) => format!("{w} / {name}"),
                None => name.to_string(),
            };
            let desc = c["description"].as_str().unwrap_or("");
            let mut e = Entry::text(if desc.is_empty() {
                format!("{} {label}", bucket(b).1)
            } else {
                format!("{} {label}  ({desc})", bucket(b).1)
            });
            e.action = Some(serde_json::json!({ "_vaayu_gh_log": {
                "link": c["link"].as_str().unwrap_or(""),
                "name": label,
                "failed": b == "fail",
            }}));
            e
        })
        .collect()
}

/// The Actions job id in a check's link
/// (`https://github.com/o/r/actions/runs/<run>/job/<job>`).
pub fn job_id(link: &str) -> Option<&str> {
    let (_, rest) = link.split_once("/actions/runs/")?;
    let (_, job) = rest.split_once("/job/")?;
    let job = job.split(['/', '?', '#']).next()?;
    (!job.is_empty() && job.bytes().all(|b| b.is_ascii_digit())).then_some(job)
}

/// `gh run view --log` lines (`job<TAB>step<TAB><timestamp> text`) →
/// entries: a header row whenever the step changes, then each line with
/// its job name and timestamp dropped. Returns the index of the first
/// line mentioning an error too, for the cursor to start on.
pub fn log_entries(text: &str) -> (Vec<Entry>, Option<usize>) {
    let mut entries = Vec::new();
    let mut step = String::new();
    let mut first_error = None;
    for raw in text.lines() {
        let raw = raw.trim_start_matches('\u{feff}');
        let mut fields = raw.splitn(3, '\t');
        let (line, this_step) = match (fields.next(), fields.next(), fields.next()) {
            (Some(_job), Some(s), Some(rest)) => (strip_timestamp(rest), s),
            _ => (raw, step.as_str()),
        };
        if this_step != step {
            step = this_step.to_string();
            entries.push(Entry::text(format!("── {step} ──")));
        }
        let line = line.trim_start_matches('\u{feff}');
        if first_error.is_none() && line.to_lowercase().contains("error") {
            first_error = Some(entries.len());
        }
        entries.push(Entry::text(line));
    }
    if entries.is_empty() {
        entries.push(Entry::text("Empty log"));
    }
    (entries, first_error)
}

/// Drops a leading `2026-01-02T03:04:05.1234567Z ` timestamp.
fn strip_timestamp(s: &str) -> &str {
    match s.split_once(' ') {
        Some((ts, rest)) if ts.ends_with('Z') && ts.len() >= 20 && ts.as_bytes()[4] == b'-' => rest,
        _ => s,
    }
}

/// `gh issue list --json` rows → one entry per issue; Enter shows it.
pub fn issue_list_entries(list: &Value) -> Vec<Entry> {
    let rows = list.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return vec![Entry::text("No issues")];
    }
    rows.iter()
        .filter_map(|issue| {
            let n = issue["number"].as_u64()?;
            let labels: Vec<&str> = issue["labels"]
                .as_array()
                .map(|a| a.iter().filter_map(|l| l["name"].as_str()).collect())
                .unwrap_or_default();
            let labels = if labels.is_empty() {
                String::new()
            } else {
                format!(" [{}]", labels.join(", "))
            };
            let mut e = Entry::text(format!(
                "#{n}  {}  · {}{labels}",
                issue["title"].as_str().unwrap_or(""),
                login(issue),
            ));
            e.action = Some(serde_json::json!({ "_vaayu_gh_issue": n }));
            Some(e)
        })
        .collect()
}

/// `gh pr list` / `gh issue list` accept `open|closed|merged|all`;
/// anything else (including empty) means open.
fn list_state(arg: &str) -> &'static str {
    match arg.trim() {
        "closed" => "closed",
        "merged" => "merged",
        "all" => "all",
        _ => "open",
    }
}

impl Editor {
    /// Reads may replace earlier reads. A checkout must retain its completion
    /// receiver so buffers are refreshed after the working tree changes.
    fn gh_spawn(
        &mut self,
        busy: &str,
        job: impl FnOnce(PathBuf) -> GhDone + Send + 'static,
    ) -> bool {
        if self.gh_task.as_ref().is_some_and(|task| task.checkout) {
            self.set_message(
                "GitHub checkout in progress — wait before starting another GitHub operation",
            );
            return false;
        }
        let root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.gh_task = Some(GhTask {
            receiver: rx,
            checkout: false,
            ready: None,
        });
        self.set_message(busy.to_string());
        std::thread::spawn(move || {
            let _ = tx.send(job(root));
        });
        true
    }

    /// `:ghprs [open|closed|merged|all]`: the repository's pull requests.
    pub fn gh_pr_list(&mut self, arg: &str) {
        let state = list_state(arg);
        self.gh_spawn("Reading pull requests…", move |root| {
            show(
                gh_json(
                    &root,
                    &[
                        "pr",
                        "list",
                        "--state",
                        state,
                        "--limit",
                        "100",
                        "--json",
                        "number,title,author,headRefName,isDraft,state,reviewDecision",
                    ],
                    false,
                )
                .map(|v| {
                    let entries = pr_list_entries(&v);
                    Results::new(
                        format!("GitHub pull requests ({state}) — Enter opens one"),
                        entries,
                    )
                }),
            )
        });
    }

    /// `:ghpr [N]`: PR N's (default: the current branch's) overview.
    pub fn gh_pr_view(&mut self, number: Option<u64>) {
        self.gh_spawn("Reading pull request…", move |root| {
            show((|| -> Result<Results, String> {
                let n = resolve_pr(&root, number)?;
                let v = gh_json(
                    &root,
                    &[
                        "pr",
                        "view",
                        &n.to_string(),
                        "--json",
                        "number,title,state,isDraft,author,headRefName,baseRefName,url,\
                         additions,deletions,changedFiles,reviewDecision,body,statusCheckRollup",
                    ],
                    false,
                )?;
                Ok(Results::new(format!("GitHub PR #{n}"), pr_view_entries(&v)))
            })())
        });
    }

    /// `:ghdiff [N]`: the PR's diff, each hunk line jumping into the local
    /// copy of its file (exact once the PR is checked out).
    pub fn gh_pr_diff(&mut self, number: Option<u64>) {
        self.gh_spawn("Reading pull request diff…", move |root| {
            show((|| -> Result<Results, String> {
                let n = resolve_pr(&root, number)?;
                let text = gh(
                    &root,
                    &["pr", "diff", &n.to_string(), "--color", "never"],
                    false,
                )?;
                Ok(Results::new(
                    format!("GitHub PR #{n} diff"),
                    diff_entries(&root, &text),
                ))
            })())
        });
    }

    /// `:ghthreads [N]`: the PR's review comments grouped into threads,
    /// each jumping to the commented line.
    pub fn gh_pr_threads(&mut self, number: Option<u64>) {
        self.gh_spawn("Reading review threads…", move |root| {
            show((|| -> Result<Results, String> {
                let n = resolve_pr(&root, number)?;
                let pages = gh(
                    &root,
                    &[
                        "api",
                        "--paginate",
                        &format!("repos/{{owner}}/{{repo}}/pulls/{n}/comments?per_page=100"),
                    ],
                    false,
                )?;
                let (threads, comments, entries) = thread_entries(&root, &pages)?;
                Ok(Results::new(
                    format!("GitHub PR #{n} review threads — {threads} thread(s), {comments} comment(s)"),
                    entries,
                ))
            })())
        });
    }

    /// `:ghchecks [N]`: the PR's CI checks, failing first.
    pub fn gh_pr_checks(&mut self, number: Option<u64>) {
        self.gh_spawn("Reading CI checks…", move |root| {
            show((|| -> Result<Results, String> {
                let n = resolve_pr(&root, number)?;
                let v = gh_json(
                    &root,
                    &[
                        "pr",
                        "checks",
                        &n.to_string(),
                        "--json",
                        "name,state,bucket,link,workflow,description",
                    ],
                    true,
                )?;
                Ok(Results::new(
                    format!("GitHub PR #{n} checks — Enter opens a log"),
                    check_entries(&v),
                ))
            })())
        });
    }

    /// Enter on a check: its GitHub Actions job log (only the failed
    /// steps for a failed job, falling back to the whole log).
    pub fn gh_check_log(&mut self, v: &Value) {
        let link = v["link"].as_str().unwrap_or("").to_string();
        let name = v["name"].as_str().unwrap_or("check").to_string();
        let failed = v["failed"].as_bool().unwrap_or(false);
        let Some(job) = job_id(&link).map(str::to_string) else {
            self.set_message(if link.is_empty() {
                format!("{name}: no details link")
            } else {
                format!("{name} isn't a GitHub Actions job; details: {link}")
            });
            return;
        };
        self.gh_spawn("Reading CI log…", move |root| {
            show((|| -> Result<Results, String> {
                let mut text = String::new();
                if failed {
                    text = gh(
                        &root,
                        &["run", "view", "--job", &job, "--log-failed"],
                        false,
                    )?;
                }
                if text.trim().is_empty() {
                    text = gh(&root, &["run", "view", "--job", &job, "--log"], false)?;
                }
                let (entries, first_error) = log_entries(&text);
                let mut r = Results::new(
                    format!(
                        "CI log — {name}{}",
                        if failed { " (failed steps)" } else { "" }
                    ),
                    entries,
                );
                r.cursor = first_error.unwrap_or(0);
                Ok(r)
            })())
        });
    }

    /// `:ghissues [open|closed|all]`: the repository's issues.
    pub fn gh_issue_list(&mut self, arg: &str) {
        let state = match list_state(arg) {
            "merged" => "closed",
            s => s,
        };
        self.gh_spawn("Reading issues…", move |root| {
            show(
                gh_json(
                    &root,
                    &[
                        "issue",
                        "list",
                        "--state",
                        state,
                        "--limit",
                        "100",
                        "--json",
                        "number,title,author,labels",
                    ],
                    false,
                )
                .map(|v| {
                    Results::new(
                        format!("GitHub issues ({state}) — Enter shows one"),
                        issue_list_entries(&v),
                    )
                }),
            )
        });
    }

    /// Enter on an issue: `gh issue view N --comments` as text.
    pub fn gh_issue_view(&mut self, n: u64) {
        self.gh_spawn("Reading issue…", move |root| {
            show(
                gh(
                    &root,
                    &["issue", "view", &n.to_string(), "--comments"],
                    false,
                )
                .map(|text| {
                    Results::new(
                        format!("GitHub issue #{n}"),
                        text.lines().map(Entry::text).collect(),
                    )
                }),
            )
        });
    }

    /// `:ghcheckout N`: `gh pr checkout N`, refused while any buffer has
    /// unsaved edits (the same guard as `:gitbranch`'s checkout).
    pub fn gh_pr_checkout(&mut self, number: Option<u64>) {
        let Some(n) = number else {
            self.set_message("Usage: :ghcheckout <number>");
            return;
        };
        if self.buffers.iter().any(|b| b.is_modified()) {
            self.set_message("Save all buffers before checking out another branch");
            return;
        }
        if self.gh_spawn(&format!("Checking out PR #{n}…"), move |root| {
            GhDone::CheckedOut(
                gh(&root, &["pr", "checkout", &n.to_string()], false)
                    .map(|_| format!("Checked out PR #{n}")),
            )
        }) {
            self.gh_task.as_mut().unwrap().checkout = true;
        }
    }

    pub fn poll_gh_task(&mut self) -> bool {
        let busy = self.mode != crate::mode::Mode::Normal
            || !self.pending.is_empty()
            || self.window_prefix
            || self.macro_recording.is_some()
            || self.active_file_tree()
            || self.active_outline()
            || self.float.as_ref().is_some_and(|f| f.focused);
        let Some(task) = self.gh_task.as_mut() else {
            return false;
        };
        if task.ready.is_some() && busy {
            return false;
        }
        let Some(done) = task.ready.take().or_else(|| task.receiver.try_recv().ok()) else {
            return false;
        };
        if matches!(done, GhDone::Show(Ok(_))) && busy {
            task.ready = Some(done);
            self.progress.finish("gh", true);
            self.set_message("GitHub results ready — Ctrl-Q to view");
            return true;
        }
        self.gh_task = None;
        match done {
            GhDone::Show(Ok(r)) => {
                self.progress.finish("gh", true);
                let title = r.title.clone();
                self.show_results(*r);
                self.set_message(title);
            }
            GhDone::Show(Err(e)) => {
                self.progress.finish("gh", false);
                self.set_message(e);
            }
            GhDone::CheckedOut(result) => {
                self.progress.finish("gh", result.is_ok());
                match result {
                    Ok(label) => {
                        self.reload_after_checkout(&label);
                        self.update_git_background();
                    }
                    Err(e) => self.set_message(e),
                }
            }
        }
        true
    }

    /// Ctrl-Q explicitly opens a deferred result without disturbing other
    /// result lists while command input, selections, or overlays are active.
    pub fn open_ready_gh_results(&mut self) -> bool {
        let ready = self.gh_task.as_mut().and_then(|task| task.ready.take());
        let Some(GhDone::Show(Ok(r))) = ready else {
            return false;
        };
        self.gh_task = None;
        self.flush_pending_jk();
        let title = r.title.clone();
        self.show_results(*r);
        self.set_message(title);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_error_names_the_three_expected_failures() {
        assert_eq!(
            classify_error(
                "pr",
                "To get started with GitHub CLI, please run:  gh auth login\n"
            ),
            NO_AUTH
        );
        assert_eq!(
            classify_error(
                "pr",
                "HTTP 401: Bad credentials (https://api.github.com/graphql)"
            ),
            NO_AUTH
        );
        assert_eq!(classify_error("pr", "no git remotes found\n"), NO_REMOTE);
        assert_eq!(
            classify_error("pr", "\nGraphQL: Could not resolve to a PullRequest\nmore"),
            "gh pr: GraphQL: Could not resolve to a PullRequest"
        );
        assert_eq!(classify_error("api", ""), "gh api: failed");
        assert_eq!(
            classify_error("pr", "HTTP 502: upstream temporarily unavailable"),
            "gh pr: HTTP 502: upstream temporarily unavailable"
        );
        assert_eq!(
            classify_error("pr", "no pull requests found for branch \"main\""),
            NO_PR
        );
    }

    #[test]
    fn parse_pr_arg_accepts_bare_and_hash_numbers() {
        assert_eq!(parse_pr_arg("12"), Ok(Some(12)));
        assert_eq!(parse_pr_arg(" #7 "), Ok(Some(7)));
        assert_eq!(parse_pr_arg(""), Ok(None));
        for invalid in ["abc", "0", "-1", "##7", "7 8", "18446744073709551616"] {
            assert!(parse_pr_arg(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn pr_list_rows_carry_number_title_author_branch_and_tags() {
        let v = json!([
            {"number": 12, "title": "Add X", "author": {"login": "ann"},
             "headRefName": "add-x", "isDraft": true, "state": "OPEN",
             "reviewDecision": "APPROVED"},
            {"number": 9, "title": "Old", "author": {"login": "bob"},
             "headRefName": "old", "isDraft": false, "state": "MERGED",
             "reviewDecision": ""}
        ]);
        let e = pr_list_entries(&v);
        assert_eq!(e[0].text, "#12  Add X  · ann · add-x [draft] [approved]");
        assert_eq!(e[0].action, Some(json!({"_vaayu_gh_pr": 12})));
        assert_eq!(e[1].text, "#9  Old  · bob · old [merged]");
        assert_eq!(pr_list_entries(&json!([]))[0].text, "No pull requests");
    }

    #[test]
    fn pr_view_has_header_summary_actions_and_body() {
        let v = json!({
            "number": 5, "title": "Fix", "state": "OPEN", "isDraft": false,
            "author": {"login": "ann"}, "headRefName": "fix", "baseRefName": "main",
            "url": "https://github.com/o/r/pull/5", "additions": 3, "deletions": 1,
            "changedFiles": 2, "reviewDecision": "CHANGES_REQUESTED",
            "body": "Line one\nLine two",
            "statusCheckRollup": [
                {"conclusion": "SUCCESS"}, {"conclusion": "FAILURE"},
                {"conclusion": "", "status": "IN_PROGRESS"}, {"state": "SUCCESS"}
            ]
        });
        let e = pr_view_entries(&v);
        let texts: Vec<&str> = e.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts[0], "#5 Fix");
        assert_eq!(
            texts[1],
            "open · ann · fix → main · +3 −1 in 2 file(s) · changes requested"
        );
        assert_eq!(texts[3], "checks: 2 passed, 1 failed, 1 pending");
        let checkout = e.iter().find(|e| e.text.contains("Checkout")).unwrap();
        assert_eq!(checkout.action, Some(json!({"_vaayu_gh_checkout": 5})));
        assert!(e
            .iter()
            .any(|e| e.action == Some(json!({"_vaayu_gh_threads": 5}))));
        assert_eq!(texts.last(), Some(&"Line two"));
    }

    #[test]
    fn diff_entries_map_hunk_lines_of_each_file_to_local_locations() {
        let root = Path::new("/p");
        let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n\
                    @@ -1,2 +1,2 @@\n one\n-two\n+TWO\n\
                    diff --git a/b.rs b/b.rs\nnew file mode 100644\n--- /dev/null\n+++ b/b.rs\n\
                    @@ -0,0 +5,1 @@\n+new\n";
        let e = diff_entries(root, text);
        assert_eq!(e.len(), 14);
        assert!(e[0].path.is_none() && e[3].path.is_none());
        assert_eq!(e[4].path.as_deref(), Some(Path::new("/p/a.rs")));
        let at = |i: usize| (e[i].path.clone().unwrap(), e[i].line);
        assert_eq!(at(5), (PathBuf::from("/p/a.rs"), 0));
        assert_eq!(at(7), (PathBuf::from("/p/a.rs"), 1));
        // The second file's preamble is text again; its hunk is b.rs.
        assert!(e[8].path.is_none() && e[11].path.is_none());
        assert_eq!(at(13), (PathBuf::from("/p/b.rs"), 4));
        assert_eq!(e[13].text, "+new");
        assert_eq!(diff_entries(root, "")[0].text, "Empty diff");
    }

    #[test]
    fn thread_entries_group_replies_under_their_root_across_pages() {
        let root = Path::new("/p");
        let pages = r#"[
            {"id": 1, "path": "src/a.rs", "line": 10, "user": {"login": "ann"},
             "body": "Rename this?\nIt reads oddly."},
            {"id": 2, "path": "src/b.rs", "line": null, "original_line": 3,
             "user": {"login": "bob"}, "body": "nit"}
        ][
            {"id": 3, "in_reply_to_id": 1, "path": "src/a.rs", "line": 10,
             "user": {"login": "cy"}, "body": "Done"}
        ]"#;
        let (threads, comments, e) = thread_entries(root, pages).unwrap();
        assert_eq!((threads, comments), (2, 3));
        let texts: Vec<&str> = e.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "ann: Rename this?",
                "    It reads oddly.",
                "    ↳ cy: Done",
                "(outdated) bob: nit"
            ]
        );
        assert!(e[..3]
            .iter()
            .all(|e| e.path.as_deref() == Some(Path::new("/p/src/a.rs")) && e.line == 9));
        assert_eq!(
            (e[3].line, e[3].path.clone()),
            (2, Some(PathBuf::from("/p/src/b.rs")))
        );
        assert_eq!(e[0].detail, "Rename this?\nIt reads oddly.");
        assert_eq!(
            thread_entries(root, "[]").unwrap().2[0].text,
            "No review comments"
        );
        assert!(thread_entries(root, "{oops").is_err());
    }

    #[test]
    fn check_entries_put_failures_first_and_link_their_logs() {
        let v = json!([
            {"name": "lint", "bucket": "pass", "workflow": "CI", "link": "L1", "description": ""},
            {"name": "test", "bucket": "fail", "workflow": "CI", "link": "L2", "description": "1 failed"},
            {"name": "deploy", "bucket": "pending", "workflow": "", "link": "", "description": ""}
        ]);
        let e = check_entries(&v);
        let texts: Vec<&str> = e.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["✗ CI / test  (1 failed)", "● deploy", "✓ CI / lint"]
        );
        assert_eq!(
            e[0].action,
            Some(json!({"_vaayu_gh_log": {"link": "L2", "name": "CI / test", "failed": true}}))
        );
        assert_eq!(check_entries(&json!([]))[0].text, "No checks reported");
    }

    #[test]
    fn job_id_reads_actions_links_only() {
        assert_eq!(
            job_id("https://github.com/o/r/actions/runs/123/job/456"),
            Some("456")
        );
        assert_eq!(
            job_id("https://github.com/o/r/actions/runs/123/job/456?pr=1"),
            Some("456")
        );
        assert_eq!(job_id("https://github.com/o/r/actions/runs/123"), None);
        assert_eq!(job_id("https://ci.example.com/build/9"), None);
    }

    #[test]
    fn log_entries_drop_job_and_timestamp_and_head_each_step() {
        let text = "\u{feff}test\tSet up job\t2026-01-02T03:04:05.1234567Z Starting\n\
                    test\tRun tests\t2026-01-02T03:04:06.0000000Z running 2 tests\n\
                    test\tRun tests\t2026-01-02T03:04:07.0000000Z error[E0308]: mismatched types\n";
        let (e, first_error) = log_entries(text);
        let texts: Vec<&str> = e.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "── Set up job ──",
                "Starting",
                "── Run tests ──",
                "running 2 tests",
                "error[E0308]: mismatched types"
            ]
        );
        assert_eq!(first_error, Some(4));
        assert_eq!(log_entries("").0[0].text, "Empty log");
    }

    #[test]
    fn issue_rows_show_labels() {
        let v = json!([{"number": 3, "title": "Crash", "author": {"login": "ann"},
                        "labels": [{"name": "bug"}, {"name": "p1"}]}]);
        let e = issue_list_entries(&v);
        assert_eq!(e[0].text, "#3  Crash  · ann [bug, p1]");
        assert_eq!(e[0].action, Some(json!({"_vaayu_gh_issue": 3})));
    }
}
