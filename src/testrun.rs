//! Test runner (`:test`, `,T…`): detect the project's runner (cargo,
//! pytest, `go test`, jest / vitest / `npm test`), run the nearest test,
//! the current file, the whole suite or the last run again, stream the
//! output live into a Results pane, then parse it into a quickfix list of
//! failures and pass/fail gutter marks (`✓`/`✗`) on test declarations.
//!
//! The command runs through the shell in the project root on background
//! reader threads (stdout and stderr merged line by line), in its own
//! process group so `:teststop` can stop the runner and everything it
//! spawned. Parsing is per runner and only needs the plain-text output, so
//! colors are turned off via the usual environment variables and any ANSI
//! escapes that slip through are stripped.
use crate::{
    editor::Editor,
    mode::Mode,
    results::{Entry, Results},
};
use regex::Regex;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, OnceLock},
};

/// `:test <arg>` values (also its command-line completion).
pub const SUBCOMMANDS: &[&str] = &[
    "nearest", "file", "suite", "last", "output", "stop", "clear",
];

/// Output lines kept per run (for the pane and the parsers); a runaway test
/// printing forever can't grow memory without bound past this.
const MAX_OUTPUT_LINES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    Cargo,
    Pytest,
    Go,
    Jest,
    Vitest,
    /// A `package.json` with neither jest nor vitest: `npm test -- …`.
    Npm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Nearest,
    File,
    Suite,
}

/// One test, as the runner reported it: `name` is the short name the
/// gutter marks match against a declaration (`foo` for `tests::foo`,
/// `test_b` for `t.py::TestC::test_b[1]`, a jest title's last segment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestId {
    pub name: String,
    pub file: Option<PathBuf>,
}

/// A source location: (path, 0-based line, 0-based col).
pub type Loc = (PathBuf, usize, usize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub id: TestId,
    /// The full name as printed (`tests::foo`, `TestB/sub`, ...).
    pub full: String,
    /// Where it failed.
    pub loc: Option<Loc>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub passed: Vec<TestId>,
    pub failed: Vec<Failure>,
}

/// What a run executes: kept for `:testlast`.
#[derive(Clone, Debug)]
pub struct Spec {
    pub runner: Runner,
    pub command: String,
    /// The file a nearest/file run targeted (resolves Go's bare
    /// `foo_test.go:12:` locations against its package directory).
    pub file: Option<PathBuf>,
}

enum Msg {
    Line(String),
    Eof,
}

struct Running {
    spec: Spec,
    rx: mpsc::Receiver<Msg>,
    eofs: u8,
    child: Child,
    done: bool,
}

impl Drop for Running {
    /// Quitting (or replacing a run) mid-run stops the whole process group
    /// rather than leaving test binaries running in the background.
    fn drop(&mut self) {
        if !self.done {
            kill_group(&mut self.child);
        }
    }
}

#[cfg(unix)]
fn kill_group(child: &mut Child) {
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{}", child.id())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(unix))]
fn kill_group(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The editor's test-runner state (`Editor::tests`).
#[derive(Default)]
pub struct TestRunner {
    run: Option<Running>,
    pub last: Option<Spec>,
    /// The latest run's output, for `:testoutput`.
    pub output: Vec<String>,
    pub last_title: String,
    /// Pass (`true`) / fail by short test name, merged across runs.
    pub outcomes: HashMap<String, bool>,
    /// The same keyed by (file, name) when the runner names the file, which
    /// wins over the bare name (two files can have a `test_parse`).
    pub file_outcomes: HashMap<(PathBuf, String), bool>,
}

impl TestRunner {
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// The gutter mark for a source line of `path`: `✓`/`✗` when the line
    /// declares a test whose last outcome is known.
    pub fn mark(&self, path: Option<&Path>, text: &str) -> Option<char> {
        if self.outcomes.is_empty() {
            return None;
        }
        let path = path?;
        let name = decl_name(path.extension()?.to_str()?, text)?;
        let ok = self
            .file_outcomes
            .get(&(path.to_path_buf(), name.clone()))
            .or_else(|| self.outcomes.get(&name))?;
        Some(if *ok { '✓' } else { '✗' })
    }

    /// Merges a run's outcomes. Failures are applied after passes, so a
    /// failing case of a name (a parametrized case, a subtest) marks the
    /// declaration failed even when another case of it passed.
    fn record(&mut self, report: &Report) {
        let ids = report
            .passed
            .iter()
            .map(|id| (id, true))
            .chain(report.failed.iter().map(|f| (&f.id, false)));
        for (id, ok) in ids {
            self.outcomes.insert(id.name.clone(), ok);
            if let Some(f) = &id.file {
                self.file_outcomes.insert((f.clone(), id.name.clone()), ok);
            }
        }
    }
}

/// The runner for a file extension (`None`: a project-wide suite run),
/// falling back to the project's marker files.
pub fn detect(root: &Path, ext: Option<&str>) -> Option<Runner> {
    let js = || {
        let pkg = std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
        if pkg.contains("\"vitest\"") {
            Runner::Vitest
        } else if pkg.contains("\"jest\"") {
            Runner::Jest
        } else {
            Runner::Npm
        }
    };
    match ext {
        Some("rs") => Some(Runner::Cargo),
        Some("py" | "pyi") => Some(Runner::Pytest),
        Some("go") => Some(Runner::Go),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts") => Some(js()),
        Some(_) => None,
        None => {
            if root.join("Cargo.toml").exists() {
                Some(Runner::Cargo)
            } else if root.join("go.mod").exists() {
                Some(Runner::Go)
            } else if root.join("package.json").exists() {
                Some(js())
            } else if [
                "pytest.ini",
                "pyproject.toml",
                "setup.cfg",
                "setup.py",
                "tox.ini",
                "conftest.py",
            ]
            .iter()
            .any(|m| root.join(m).exists())
            {
                Some(Runner::Pytest)
            } else {
                None
            }
        }
    }
}

/// Quote `s` for `sh -c` unless it's made only of characters that never
/// need it.
fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=@+,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// `[package] name` from a Cargo.toml (a workspace member's `-p` value).
fn cargo_package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    Some(value.get("package")?.get("name")?.as_str()?.to_string())
}

/// `cargo test` selectors for a Rust file: (`-p`/target args, a module-path
/// filter for a file-scoped run of a `src/` module).
fn cargo_selectors(root: &Path, file: &Path) -> (Vec<String>, Option<String>) {
    let mut args = Vec::new();
    let crate_dir = file
        .ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(root))
        .find(|d| d.join("Cargo.toml").exists())
        .unwrap_or(root);
    if crate_dir != root {
        if let Some(name) = cargo_package_name(&crate_dir.join("Cargo.toml")) {
            args.extend(["-p".to_string(), name]);
        }
    }
    let rel = file.strip_prefix(crate_dir).unwrap_or(file);
    let parts: Vec<&str> = rel.iter().filter_map(|p| p.to_str()).collect();
    let stem = |s: &str| s.trim_end_matches(".rs").to_string();
    let mut filter = None;
    match parts.as_slice() {
        ["tests", f] => args.extend(["--test".to_string(), stem(f)]),
        ["tests", dir, ..] => args.extend(["--test".to_string(), dir.to_string()]),
        ["src", "lib.rs"] => args.push("--lib".into()),
        ["src", "main.rs"] => args.push("--bins".into()),
        ["src", "bin", f] => args.extend(["--bin".to_string(), stem(f)]),
        ["src", rest @ ..] if !rest.is_empty() => {
            let mut path: Vec<String> = rest.iter().map(|s| stem(s)).collect();
            if path.last().is_some_and(|l| l == "mod") {
                path.pop();
            }
            if !path.is_empty() {
                filter = Some(format!("{}::", path.join("::")));
            }
        }
        _ => {}
    }
    (args, filter)
}

/// The shell command for one run. `file` is absolute (under `root`);
/// `name` is the nearest test's name (required for `Scope::Nearest`).
pub fn build_command(
    runner: Runner,
    root: &Path,
    file: Option<&Path>,
    scope: Scope,
    name: Option<&str>,
) -> Option<String> {
    let rel = |f: &Path| {
        let r = f.strip_prefix(root).unwrap_or(f);
        shell_quote(&r.to_string_lossy())
    };
    let file = if scope == Scope::Suite { None } else { file };
    let name = if scope == Scope::Nearest {
        Some(name?)
    } else {
        None
    };
    if scope != Scope::Suite && file.is_none() {
        return None;
    }
    let mut cmd = match runner {
        Runner::Cargo => {
            let mut cmd = "cargo test".to_string();
            if let Some(f) = file {
                let (args, filter) = cargo_selectors(root, f);
                for a in &args {
                    cmd.push(' ');
                    cmd.push_str(&shell_quote(a));
                }
                // The test name replaces the module filter (cargo takes a
                // single substring filter).
                if let Some(filter) = filter.filter(|_| name.is_none()) {
                    cmd.push(' ');
                    cmd.push_str(&shell_quote(&filter));
                }
            }
            if let Some(n) = name {
                cmd.push(' ');
                cmd.push_str(&shell_quote(n));
            }
            return Some(cmd);
        }
        Runner::Pytest => {
            let mut cmd = "pytest -v".to_string();
            if let Some(f) = file {
                cmd.push(' ');
                cmd.push_str(&rel(f));
            }
            if let Some(n) = name {
                cmd.push_str(" -k ");
                cmd.push_str(&shell_quote(n));
            }
            return Some(cmd);
        }
        Runner::Go => {
            let pkg = match file.and_then(|f| f.parent()) {
                Some(dir) => {
                    let r = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy();
                    if r.is_empty() {
                        "./".to_string()
                    } else {
                        shell_quote(&format!("./{r}"))
                    }
                }
                None => "./...".to_string(),
            };
            return Some(match name {
                Some(n) => format!("go test -v -run {} {pkg}", shell_quote(&format!("^{n}$"))),
                None => format!("go test -v {pkg}"),
            });
        }
        Runner::Jest => "npx jest".to_string(),
        Runner::Vitest => "npx vitest run".to_string(),
        Runner::Npm => "npm test --".to_string(),
    };
    if let Some(f) = file {
        cmd.push(' ');
        cmd.push_str(&rel(f));
    }
    if let Some(n) = name {
        cmd.push_str(" -t ");
        cmd.push_str(&shell_quote(n));
    }
    if runner == Runner::Npm && file.is_none() {
        cmd.truncate("npm test".len());
    }
    Some(cmd)
}

fn ident_after<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = text.split_once(keyword)?.1;
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty() && rest[end..].starts_with(['(', '<'])).then_some(name)
}

/// The test (or function) a source line declares, by extension: `fn x(`,
/// `def x(`, `func X(`, or a jest/vitest/mocha `it('title'` / `test("…"`.
pub fn decl_name(ext: &str, text: &str) -> Option<String> {
    let t = text.trim_start();
    let name = match ext {
        "rs" => {
            let t = t.strip_prefix("pub ").unwrap_or(t);
            if !(t.starts_with("fn ") || t.starts_with("async fn ") || t.contains(" fn ")) {
                return None;
            }
            ident_after(t, "fn ")?
        }
        "py" | "pyi" => {
            if !(t.starts_with("def ") || t.starts_with("async def ")) {
                return None;
            }
            ident_after(t, "def ")?
        }
        "go" => ident_after(t.strip_prefix("func ")?, "")?,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => {
            static RE: OnceLock<Regex> = OnceLock::new();
            let re = RE.get_or_init(|| {
                Regex::new(r#"^(?:it|test)(?:\.\w+)*\(\s*(['"`])(.+?)['"`]"#).unwrap()
            });
            return re.captures(t).map(|c| c[2].to_string());
        }
        _ => return None,
    };
    Some(name.to_string())
}

/// Strips ANSI escapes and keeps only the text after the last carriage
/// return (a progress line redrawn in place).
fn clean_line(raw: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[()][0-9A-Za-z]").unwrap());
    let line = raw.rsplit('\r').find(|s| !s.is_empty()).unwrap_or("");
    re.replace_all(line, "").into_owned()
}

/// The short name gutter marks match: the last `::`/`.`/`/`-free segment
/// with any `[param]` suffix dropped.
fn short_name(full: &str, seps: &[&str]) -> String {
    let mut s = full;
    if let Some(i) = s.find('[') {
        s = &s[..i];
    }
    for sep in seps {
        if let Some((_, tail)) = s.rsplit_once(sep) {
            s = tail;
        }
    }
    s.to_string()
}

/// Parses one run's output. `resolve` turns a path as printed into a real
/// path (Go prints bare file names relative to the package directory).
pub fn parse(runner: Runner, lines: &[String], resolve: &dyn Fn(&str) -> PathBuf) -> Report {
    match runner {
        Runner::Cargo => parse_cargo(lines, resolve),
        Runner::Pytest => parse_pytest(lines, resolve),
        Runner::Go => parse_go(lines, resolve),
        Runner::Jest | Runner::Vitest | Runner::Npm => parse_js(lines, resolve),
    }
}

fn loc(resolve: &dyn Fn(&str) -> PathBuf, path: &str, line: &str, col: Option<&str>) -> Loc {
    let l: usize = line.parse().unwrap_or(1);
    let c: usize = col.and_then(|c| c.parse().ok()).unwrap_or(1);
    (resolve(path), l.saturating_sub(1), c.saturating_sub(1))
}

fn parse_cargo(lines: &[String], resolve: &dyn Fn(&str) -> PathBuf) -> Report {
    static RES: OnceLock<[Regex; 4]> = OnceLock::new();
    let [result, block, panic_new, panic_old] = RES.get_or_init(|| {
        [
            Regex::new(r"^test (.+?) \.\.\. (ok|FAILED)\b").unwrap(),
            Regex::new(r"^---- (.+?) stdout ----$").unwrap(),
            Regex::new(r"panicked at ([^\s:][^:]*):(\d+):(\d+):?\s*$").unwrap(),
            Regex::new(r"panicked at '(.*)', ([^\s:][^:]*):(\d+):(\d+)").unwrap(),
        ]
    });
    let mut report = Report::default();
    let mut order = Vec::new();
    let mut detail: HashMap<String, (Option<Loc>, String)> = HashMap::new();
    let mut current: Option<String> = None;
    let mut want_message = false;
    for line in lines {
        if let Some(c) = result.captures(line) {
            let full = c[1].to_string();
            let id = TestId {
                name: short_name(&full, &["::"]),
                file: None,
            };
            if &c[2] == "ok" {
                report.passed.push(id);
            } else {
                order.push((full, id));
            }
            continue;
        }
        if let Some(c) = block.captures(line) {
            current = Some(c[1].to_string());
            want_message = false;
            continue;
        }
        if line.starts_with("failures:") || line.starts_with("test result:") {
            current = None;
            continue;
        }
        let Some(cur) = &current else { continue };
        let entry = detail.entry(cur.clone()).or_default();
        if let Some(c) = panic_old.captures(line) {
            entry.0 = Some(loc(resolve, &c[2], &c[3], Some(&c[4])));
            entry.1 = c[1].to_string();
        } else if let Some(c) = panic_new.captures(line) {
            entry.0 = Some(loc(resolve, &c[1], &c[2], Some(&c[3])));
            want_message = true;
        } else if want_message && !line.trim().is_empty() {
            entry.1 = line.trim().to_string();
            want_message = false;
        }
    }
    for (full, id) in order {
        let (loc, message) = detail.remove(&full).unwrap_or_default();
        report.failed.push(Failure {
            id,
            full,
            loc,
            message,
        });
    }
    report
}

fn parse_pytest(lines: &[String], resolve: &dyn Fn(&str) -> PathBuf) -> Report {
    static RES: OnceLock<[Regex; 4]> = OnceLock::new();
    let [progress, summary, section, location] = RES.get_or_init(|| {
        [
            Regex::new(r"^(\S+?\.py)::(\S+) (PASSED|FAILED|ERROR|XPASS)\b").unwrap(),
            Regex::new(r"^(FAILED|ERROR) (\S+?\.py)(?:::(\S+?))?(?: - (.*))?$").unwrap(),
            Regex::new(r"^_{2,} (.+?) _{2,}$").unwrap(),
            Regex::new(r"^(\S+\.py):(\d+): (.*)$").unwrap(),
        ]
    });
    let mut report = Report::default();
    // Failing test full ids in first-seen order, with their summary message.
    let mut failing: Vec<(String, String, String)> = Vec::new();
    let mut sections: HashMap<String, Loc> = HashMap::new();
    let mut current: Option<String> = None;
    let short = |full: &str| short_name(full, &["::", "."]);
    for line in lines {
        if let Some(c) = progress.captures(line) {
            let id = TestId {
                name: short(&c[2]),
                file: Some(resolve(&c[1])),
            };
            match &c[3] {
                "PASSED" | "XPASS" => report.passed.push(id),
                _ => {
                    if !failing.iter().any(|(f, t, _)| f == &c[1] && t == &c[2]) {
                        failing.push((c[1].to_string(), c[2].to_string(), String::new()));
                    }
                }
            }
            continue;
        }
        if let Some(c) = summary.captures(line) {
            let test = c.get(3).map_or("", |m| m.as_str()).to_string();
            let msg = c.get(4).map_or("", |m| m.as_str()).to_string();
            match failing
                .iter_mut()
                .find(|(f, t, _)| f == &c[2] && *t == test)
            {
                Some(slot) => slot.2 = msg,
                None => failing.push((c[2].to_string(), test, msg)),
            }
            continue;
        }
        if let Some(c) = section.captures(line) {
            current = Some(short(&c[1]));
            continue;
        }
        if let (Some(cur), Some(c)) = (&current, location.captures(line)) {
            // The last location in a failure's traceback is the line that
            // raised.
            sections.insert(cur.clone(), loc(resolve, &c[1], &c[2], None));
        }
    }
    for (file, test, message) in failing {
        let path = resolve(&file);
        let name = if test.is_empty() {
            file.clone()
        } else {
            short(&test)
        };
        let loc = sections
            .get(&name)
            .cloned()
            .or_else(|| Some((path.clone(), 0, 0)));
        report.failed.push(Failure {
            id: TestId {
                name,
                file: Some(path),
            },
            full: if test.is_empty() {
                file
            } else {
                format!("{file}::{test}")
            },
            loc,
            message,
        });
    }
    report
}

fn parse_go(lines: &[String], resolve: &dyn Fn(&str) -> PathBuf) -> Report {
    static RES: OnceLock<[Regex; 3]> = OnceLock::new();
    let [run, result, location] = RES.get_or_init(|| {
        [
            Regex::new(r"^=== (?:RUN|CONT|PAUSE|NAME)\s+(\S+)").unwrap(),
            Regex::new(r"^\s*--- (PASS|FAIL): (\S+) \(").unwrap(),
            Regex::new(r"^\s+(\S+\.go):(\d+): (.*)$").unwrap(),
        ]
    });
    let mut report = Report::default();
    let mut current: Option<String> = None;
    let mut first_loc: HashMap<String, (Loc, String)> = HashMap::new();
    for line in lines {
        if let Some(c) = run.captures(line) {
            current = Some(c[1].to_string());
        } else if let Some(c) = result.captures(line) {
            let full = c[2].to_string();
            let id = TestId {
                name: full.split('/').next().unwrap_or(&full).to_string(),
                file: None,
            };
            if &c[1] == "PASS" {
                report.passed.push(id);
            } else {
                // `-v` prints a parent's FAIL before its subtests' and
                // complaints land under the subtest, so fall back to one.
                let prefix = format!("{full}/");
                let found = first_loc.get(&full).or_else(|| {
                    first_loc
                        .iter()
                        .filter(|(k, _)| k.starts_with(&prefix))
                        .min_by_key(|(k, _)| k.as_str())
                        .map(|(_, v)| v)
                });
                let (loc, message) = match found {
                    Some((l, m)) => (Some(l.clone()), m.clone()),
                    None => (None, String::new()),
                };
                report.failed.push(Failure {
                    id,
                    full,
                    loc,
                    message,
                });
            }
        } else if let (Some(cur), Some(c)) = (&current, location.captures(line)) {
            first_loc
                .entry(cur.clone())
                .or_insert_with(|| (loc(resolve, &c[1], &c[2], None), c[3].trim().to_string()));
        }
    }
    report
}

fn parse_js(lines: &[String], resolve: &dyn Fn(&str) -> PathBuf) -> Report {
    static RES: OnceLock<[Regex; 4]> = OnceLock::new();
    let [mark, header, location, timing] = RES.get_or_init(|| {
        [
            Regex::new(r"^\s*([✓√✔]|[✕×✗✘])\s+(.+?)\s*$").unwrap(),
            Regex::new(r"^\s*(?:●|FAIL)\s+(.+?)\s*$").unwrap(),
            Regex::new(r"\(?((?:\.{0,2}/)?[^\s()]+\.[cm]?[jt]sx?):(\d+):(\d+)\)?").unwrap(),
            Regex::new(r"\s*(?:\(\d+(?:\.\d+)?\s*m?s\)|\d+(?:\.\d+)?\s*m?s)$").unwrap(),
        ]
    });
    let title = |s: &str| -> String {
        let s = timing.replace(s, "");
        let last = s.rsplit(['>', '›']).next().unwrap_or(&s).trim();
        last.to_string()
    };
    let mut report = Report::default();
    let mut current: Option<String> = None;
    // Per failing title: its first non-dependency location and the first
    // line of its error message.
    let mut locs: HashMap<String, Loc> = HashMap::new();
    let mut messages: HashMap<String, String> = HashMap::new();
    let mut want_message = false;
    for line in lines {
        if let Some(c) = mark.captures(line) {
            let id = TestId {
                name: title(&c[2]),
                file: None,
            };
            if "✓√✔".contains(&c[1]) {
                report.passed.push(id);
            } else if !report.failed.iter().any(|f| f.id == id) {
                report.failed.push(Failure {
                    full: c[2].to_string(),
                    id,
                    loc: None,
                    message: String::new(),
                });
            }
            continue;
        }
        if let Some(c) = header.captures(line) {
            // A `FAIL path` line (no test title) is a file header, not a test.
            if c[1].contains(['>', '›']) || line.trim_start().starts_with('●') {
                current = Some(title(&c[1]));
                want_message = true;
            }
            continue;
        }
        let Some(cur) = &current else { continue };
        if let Some(c) = location.captures(line) {
            if !c[1].contains("node_modules") && !locs.contains_key(cur) {
                locs.insert(cur.clone(), loc(resolve, &c[1], &c[2], Some(&c[3])));
            }
        } else if want_message && !line.trim().is_empty() {
            messages.insert(cur.clone(), line.trim().to_string());
            want_message = false;
        }
    }
    for f in &mut report.failed {
        f.loc = locs.remove(&f.id.name);
        f.message = messages.remove(&f.id.name).unwrap_or_default();
    }
    report
}

/// The Results title summarizing a finished run.
fn summary(command: &str, report: &Report, success: bool, code: Option<i32>) -> String {
    let (p, f) = (report.passed.len(), report.failed.len());
    if f > 0 {
        format!("✗ {command} — {f} failed, {p} passed")
    } else if p > 0 && success {
        format!("✓ {command} — {p} passed")
    } else if success {
        format!("✓ {command} — ok")
    } else {
        match code {
            Some(c) => format!("✗ {command} — exit {c}"),
            None => format!("✗ {command} — stopped"),
        }
    }
}

impl Editor {
    /// `:test{nearest,file,suite}` / `,Tn` `,Tf` `,Ts`.
    pub fn test_run(&mut self, scope: Scope) {
        let path = self.buf().path.clone();
        let ext = path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(str::to_string);
        if scope != Scope::Suite && path.is_none() {
            self.set_message("Test: the buffer has no file");
            return;
        }
        let runner_ext = if scope == Scope::Suite {
            // A suite run follows the current file's language when it has a
            // runner, else the project's markers.
            ext.as_deref()
                .filter(|e| detect(&self.project_root, Some(e)).is_some())
        } else {
            Some(ext.as_deref().unwrap_or(""))
        };
        let Some(runner) = detect(&self.project_root, runner_ext) else {
            self.set_message(match runner_ext {
                Some(e) => format!("No test runner for .{e} files"),
                None => {
                    "No test runner detected (Cargo.toml / go.mod / package.json / pytest config)"
                        .to_string()
                }
            });
            return;
        };
        let name = if scope == Scope::Nearest {
            match self.nearest_test_name(ext.as_deref().unwrap_or("")) {
                Some(n) => Some(n),
                None => {
                    self.set_message("No test under the cursor");
                    return;
                }
            }
        } else {
            None
        };
        let Some(command) = build_command(
            runner,
            &self.project_root,
            path.as_deref(),
            scope,
            name.as_deref(),
        ) else {
            self.set_message("Test: nothing to run");
            return;
        };
        self.start_test(Spec {
            runner,
            command,
            file: path.filter(|_| scope != Scope::Suite),
        });
    }

    /// The test enclosing the cursor: tree-sitter's enclosing function for
    /// languages whose tests are functions, else (and as a fallback) the
    /// nearest test declaration at or above the cursor.
    fn nearest_test_name(&self, ext: &str) -> Option<String> {
        let js = matches!(
            ext,
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts"
        );
        if !js {
            if let Some(n) = self.enclosing_function_name() {
                return Some(n);
            }
        }
        let b = self.buf();
        (0..=self.cursor().0)
            .rev()
            .find_map(|l| decl_name(ext, &b.line_text(l)))
    }

    /// `:testlast` / `,Tl`: run the previous test command again.
    pub fn test_last(&mut self) {
        match self.tests.last.clone() {
            Some(spec) => self.start_test(spec),
            None => self.set_message("No previous test run"),
        }
    }

    /// `:teststop` / `,Tx`.
    pub fn test_stop(&mut self) {
        match self.tests.run.take() {
            // Dropping it kills the process group.
            Some(run) => {
                self.tests.last_title = summary(&run.spec.command, &Report::default(), false, None);
                drop(run);
                if let Some(r) = self.results.as_mut().filter(|r| r.test_output) {
                    r.title = self.tests.last_title.clone();
                    r.busy = false;
                }
                self.progress.finish("test", false);
                self.set_message("Test run stopped");
            }
            None => self.set_message("No test run in progress"),
        }
    }

    /// `:testclear` / `,Tc`: forget the pass/fail gutter marks.
    pub fn test_clear(&mut self) {
        self.tests.outcomes.clear();
        self.tests.file_outcomes.clear();
        self.set_message("Test marks cleared");
    }

    /// `:testoutput` / `,To`: the latest run's full output (live while it
    /// is still running).
    pub fn test_output(&mut self) {
        if self.tests.last.is_none() {
            self.set_message("No test run yet");
            return;
        }
        let title = self.tests.last_title.clone();
        let mut r = Results::new(title, self.tests.output.iter().map(Entry::text).collect());
        r.test_output = true;
        r.busy = self.tests.running();
        r.cursor = r.entries.len().saturating_sub(1);
        self.show_results(r);
    }

    fn start_test(&mut self, spec: Spec) {
        // A new run replaces one still going (dropping it stops it).
        self.tests.run = None;
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&spec.command)
            .current_dir(&self.project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .env("FORCE_COLOR", "0")
            .env("CARGO_TERM_COLOR", "never")
            .env("PYTHONUNBUFFERED", "1")
            .env("CI", "1");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.set_message(format!("Test: failed to start `{}`: {e}", spec.command));
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let readers: [Option<Box<dyn Read + Send>>; 2] = [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ];
        for reader in readers {
            let tx = tx.clone();
            std::thread::spawn(move || {
                if let Some(r) = reader {
                    let mut r = BufReader::new(r);
                    let mut buf = Vec::new();
                    while r.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
                        let text = String::from_utf8_lossy(&buf);
                        let line = clean_line(text.trim_end_matches(['\n', '\r']));
                        if tx.send(Msg::Line(line)).is_err() {
                            return;
                        }
                        buf.clear();
                    }
                }
                let _ = tx.send(Msg::Eof);
            });
        }
        self.progress.begin("test", "test", &spec.command, None);
        self.set_message(format!("Running: {}…", spec.command));
        self.tests.output.clear();
        self.tests.last_title = format!("▶ {} — running", spec.command);
        self.tests.last = Some(spec.clone());
        self.tests.run = Some(Running {
            spec,
            rx,
            eofs: 0,
            child,
            done: false,
        });
        // The live job pane.
        self.test_output();
    }

    /// Drains new output into the live pane and finishes a completed run.
    pub fn poll_tests(&mut self) -> bool {
        let Some(run) = self.tests.run.as_mut() else {
            return false;
        };
        let mut lines = Vec::new();
        let mut finished = false;
        while let Ok(msg) = run.rx.try_recv() {
            match msg {
                Msg::Line(l) => lines.push(l),
                Msg::Eof => {
                    run.eofs += 1;
                    finished = run.eofs == 2;
                }
            }
        }
        if lines.is_empty() && !finished {
            return false;
        }
        let room = MAX_OUTPUT_LINES.saturating_sub(self.tests.output.len());
        lines.truncate(room);
        if let Some(r) = self.results.as_mut().filter(|r| r.test_output) {
            let follow = r.cursor + 1 >= r.entries.len();
            for l in &lines {
                let e = Entry::text(l.clone());
                if r.filter.is_empty() {
                    r.entries.push(e.clone());
                }
                r.all_entries.push(e);
            }
            if !r.filter.is_empty() {
                r.apply_filter();
            }
            if follow {
                r.cursor = r.entries.len().saturating_sub(1);
            }
        }
        self.tests.output.extend(lines);
        if finished {
            self.finish_test();
        }
        true
    }

    fn finish_test(&mut self) {
        let Some(mut run) = self.tests.run.take() else {
            return;
        };
        run.done = true;
        let status = run.child.wait().ok();
        let success = status.is_some_and(|s| s.success());
        let code = status.and_then(|s| s.code());
        let spec = run.spec.clone();
        drop(run);
        let root = self.project_root.clone();
        let pkg_dir = spec
            .file
            .as_ref()
            .and_then(|f| f.parent())
            .map(Path::to_path_buf);
        let all_files = &self.all_files;
        let resolve = |p: &str| -> PathBuf {
            let direct = root.join(p);
            if direct.exists() {
                return direct;
            }
            if let Some(found) = pkg_dir.as_ref().map(|d| d.join(p)).filter(|f| f.exists()) {
                return found;
            }
            let suffix = format!("/{}", p.trim_start_matches("./"));
            all_files
                .iter()
                .find(|f| f.ends_with(&suffix) || *f == p)
                .map(|f| root.join(f))
                .unwrap_or(direct)
        };
        let report = parse(spec.runner, &self.tests.output, &resolve);
        self.tests.record(&report);
        let title = summary(&spec.command, &report, success, code);
        self.tests.last_title = title.clone();
        let ok = report.failed.is_empty() && success;
        self.progress.finish("test", ok);
        let showing = self.results.as_ref().is_some_and(|r| r.test_output);
        if let Some(r) = self.results.as_mut().filter(|r| r.test_output) {
            r.title = title.clone();
            r.busy = false;
        }
        let mut entries: Vec<Entry> = report
            .failed
            .iter()
            .map(|f| {
                let text = if f.message.is_empty() {
                    format!("✗ {}", f.full)
                } else {
                    format!("✗ {}  {}", f.full, f.message)
                };
                let mut e = match &f.loc {
                    Some((p, l, c)) => Entry::location(p.clone(), *l, *c, text),
                    None => Entry::text(text),
                };
                e.detail = f.message.clone();
                e
            })
            .collect();
        if entries.is_empty() && !success {
            // No test results at all: most likely a build error, which the
            // generic `file:line:col` parser understands.
            entries = crate::task::parse_errorformat(&self.tests.output.join("\n"), &root);
        }
        if entries.is_empty() {
            self.set_message(&title);
            return;
        }
        let mut r = Results::new(title.clone(), entries);
        r.quickfix = true;
        self.set_quickfix_list(r.clone());
        if self.mode == Mode::Insert {
            self.results = Some(r);
            self.set_message(format!("{title} — :copen / Ctrl-Q to view"));
        } else if showing || self.mode == Mode::Normal {
            self.show_results(r);
        } else {
            self.set_message(format!("{title} — :copen to view"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    fn rooted(p: &str) -> PathBuf {
        Path::new("/proj").join(p)
    }

    #[test]
    fn detects_runner_from_extension_and_markers() {
        let dir = std::env::temp_dir().join(format!("vaayu-testrun-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect(&dir, None), None);
        assert_eq!(detect(&dir, Some("rs")), Some(Runner::Cargo));
        assert_eq!(detect(&dir, Some("py")), Some(Runner::Pytest));
        assert_eq!(detect(&dir, Some("go")), Some(Runner::Go));
        assert_eq!(detect(&dir, Some("txt")), None);
        assert_eq!(detect(&dir, Some("ts")), Some(Runner::Npm));
        std::fs::write(
            dir.join("package.json"),
            r#"{"devDependencies":{"vitest":"1"}}"#,
        )
        .unwrap();
        assert_eq!(detect(&dir, Some("ts")), Some(Runner::Vitest));
        assert_eq!(detect(&dir, None), Some(Runner::Vitest));
        std::fs::write(
            dir.join("package.json"),
            r#"{"devDependencies":{"jest":"29"}}"#,
        )
        .unwrap();
        assert_eq!(detect(&dir, Some("js")), Some(Runner::Jest));
        std::fs::remove_file(dir.join("package.json")).unwrap();
        std::fs::write(dir.join("pyproject.toml"), "").unwrap();
        assert_eq!(detect(&dir, None), Some(Runner::Pytest));
        std::fs::write(dir.join("go.mod"), "module x\n").unwrap();
        assert_eq!(detect(&dir, None), Some(Runner::Go));
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        assert_eq!(detect(&dir, None), Some(Runner::Cargo));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builds_commands_per_runner_and_scope() {
        let root = Path::new("/nonexistent-root");
        let f = |p: &str| root.join(p);
        let b = |r, file: Option<&str>, s, n: Option<&str>| {
            build_command(r, root, file.map(f).as_deref(), s, n)
        };
        use Runner::*;
        use Scope::*;
        assert_eq!(b(Cargo, None, Suite, None).unwrap(), "cargo test");
        assert_eq!(
            b(Cargo, Some("src/lib.rs"), File, None).unwrap(),
            "cargo test --lib"
        );
        assert_eq!(
            b(Cargo, Some("src/editor/mod.rs"), File, None).unwrap(),
            "cargo test editor::"
        );
        assert_eq!(
            b(Cargo, Some("src/a/b.rs"), File, None).unwrap(),
            "cargo test a::b::"
        );
        assert_eq!(
            b(Cargo, Some("tests/cli.rs"), Nearest, Some("parses")).unwrap(),
            "cargo test --test cli parses"
        );
        assert_eq!(
            b(Cargo, Some("src/a.rs"), Nearest, Some("parses")).unwrap(),
            "cargo test parses",
            "the test name replaces the module filter"
        );
        assert_eq!(
            b(Cargo, Some("src/bin/vy.rs"), File, None).unwrap(),
            "cargo test --bin vy"
        );
        assert_eq!(b(Pytest, None, Suite, None).unwrap(), "pytest -v");
        assert_eq!(
            b(Pytest, Some("tests/test x.py"), Nearest, Some("test_a")).unwrap(),
            "pytest -v 'tests/test x.py' -k test_a"
        );
        assert_eq!(b(Go, None, Suite, None).unwrap(), "go test -v ./...");
        assert_eq!(
            b(Go, Some("pkg/a_test.go"), File, None).unwrap(),
            "go test -v ./pkg"
        );
        assert_eq!(
            b(Go, Some("a_test.go"), Nearest, Some("TestA")).unwrap(),
            "go test -v -run '^TestA$' ./"
        );
        assert_eq!(
            b(Jest, Some("src/a.test.ts"), Nearest, Some("adds one")).unwrap(),
            "npx jest src/a.test.ts -t 'adds one'"
        );
        assert_eq!(b(Vitest, None, Suite, None).unwrap(), "npx vitest run");
        assert_eq!(b(Npm, None, Suite, None).unwrap(), "npm test");
        assert_eq!(
            b(Npm, Some("a.test.js"), File, None).unwrap(),
            "npm test -- a.test.js"
        );
        // Nearest needs a name, file scopes need a file.
        assert_eq!(b(Cargo, Some("src/a.rs"), Nearest, None), None);
        assert_eq!(b(Pytest, None, File, None), None);
    }

    #[test]
    fn cargo_workspace_member_gets_a_package_flag() {
        let root = std::env::temp_dir().join(format!("vaayu-testrun-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("crates/core/src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/core\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/core/Cargo.toml"),
            "[package]\nname = \"my-core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let file = root.join("crates/core/src/lib.rs");
        assert_eq!(
            build_command(Runner::Cargo, &root, Some(&file), Scope::File, None).unwrap(),
            "cargo test -p my-core --lib"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shell_quote_leaves_safe_words_alone() {
        assert_eq!(shell_quote("src/a_b.rs"), "src/a_b.rs");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("^T$"), "'^T$'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn decl_name_finds_test_declarations() {
        assert_eq!(
            decl_name("rs", "    fn parses_input() {").as_deref(),
            Some("parses_input")
        );
        assert_eq!(
            decl_name("rs", "pub async fn go<T>(x: T) {").as_deref(),
            Some("go")
        );
        assert_eq!(decl_name("rs", "let fn_x = 1;"), None);
        assert_eq!(
            decl_name("py", "    def test_a(self):").as_deref(),
            Some("test_a")
        );
        assert_eq!(
            decl_name("py", "async def test_b():").as_deref(),
            Some("test_b")
        );
        assert_eq!(decl_name("py", "x = define(1)"), None);
        assert_eq!(
            decl_name("go", "func TestA(t *testing.T) {").as_deref(),
            Some("TestA")
        );
        assert_eq!(decl_name("go", "func (s *S) TestM() {"), None);
        assert_eq!(
            decl_name("ts", "  it('adds one', () => {").as_deref(),
            Some("adds one")
        );
        assert_eq!(
            decl_name("js", "test.only(\"x y\", async () => {").as_deref(),
            Some("x y")
        );
        assert_eq!(decl_name("js", "describe('suite', () => {"), None);
        assert_eq!(decl_name("txt", "fn a() {"), None);
    }

    #[test]
    fn clean_line_strips_ansi_and_carriage_returns() {
        assert_eq!(clean_line("\x1b[32mok\x1b[0m"), "ok");
        assert_eq!(clean_line("progress 10%\rprogress 100%"), "progress 100%");
        assert_eq!(clean_line("trailing\r"), "trailing");
    }

    #[test]
    fn parses_cargo_output() {
        let out = lines(
            "running 3 tests
test tests::adds ... ok
test tests::subtracts ... FAILED
test tests::old_style ... FAILED
test tests::skipped ... ignored

failures:

---- tests::subtracts stdout ----

thread 'tests::subtracts' (7350) panicked at src/lib.rs:12:9:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- tests::old_style stdout ----
thread 'tests::old_style' panicked at 'boom', src/math/mod.rs:3:5

failures:
    tests::old_style
    tests::subtracts

test result: FAILED. 1 passed; 2 failed; 1 ignored; 0 measured",
        );
        let r = parse(Runner::Cargo, &out, &|p| rooted(p));
        assert_eq!(r.passed.len(), 1);
        assert_eq!(r.passed[0].name, "adds");
        assert_eq!(r.failed.len(), 2);
        let f = &r.failed[0];
        assert_eq!(
            (f.full.as_str(), f.id.name.as_str()),
            ("tests::subtracts", "subtracts")
        );
        assert_eq!(f.loc, Some((rooted("src/lib.rs"), 11, 8)));
        assert_eq!(f.message, "assertion `left == right` failed");
        let f = &r.failed[1];
        assert_eq!(f.loc, Some((rooted("src/math/mod.rs"), 2, 4)));
        assert_eq!(f.message, "boom");
    }

    #[test]
    fn parses_pytest_verbose_output() {
        let out = lines(
            "tests/test_m.py::test_ok PASSED                     [ 25%]
tests/test_m.py::test_bad FAILED                    [ 50%]
tests/test_m.py::TestK::test_meth[2-3] FAILED       [ 75%]
tests/test_m.py::TestK::test_meth[1-1] PASSED       [100%]

=================================== FAILURES ===================================
___________________________________ test_bad ___________________________________

    def test_bad():
>       assert 1 == 2
E       assert 1 == 2

tests/test_m.py:7: AssertionError
_____________________________ TestK.test_meth[2-3] _____________________________
tests/helpers.py:3: in check
    raise ValueError
tests/test_m.py:12: ValueError
=========================== short test summary info ============================
FAILED tests/test_m.py::test_bad - assert 1 == 2
FAILED tests/test_m.py::TestK::test_meth[2-3] - ValueError
ERROR tests/test_broken.py - ImportError: nope
========================= 2 failed, 2 passed in 0.05s ==========================",
        );
        let r = parse(Runner::Pytest, &out, &|p| rooted(p));
        assert_eq!(r.passed.len(), 2);
        assert_eq!(r.failed.len(), 3);
        let f = &r.failed[0];
        assert_eq!(f.id.name, "test_bad");
        assert_eq!(f.id.file, Some(rooted("tests/test_m.py")));
        assert_eq!(f.loc, Some((rooted("tests/test_m.py"), 6, 0)));
        assert_eq!(f.message, "assert 1 == 2");
        let f = &r.failed[1];
        assert_eq!(f.id.name, "test_meth");
        assert_eq!(
            f.loc,
            Some((rooted("tests/test_m.py"), 11, 0)),
            "the raising line wins"
        );
        let f = &r.failed[2];
        assert_eq!(f.full, "tests/test_broken.py");
        assert_eq!(f.loc, Some((rooted("tests/test_broken.py"), 0, 0)));
        assert!(f.message.starts_with("ImportError"));
    }

    #[test]
    fn parses_go_verbose_output() {
        let out = lines(
            "=== RUN   TestAdd
--- PASS: TestAdd (0.00s)
=== RUN   TestSub
    calc_test.go:14: got 1, want 2
    calc_test.go:15: second complaint
--- FAIL: TestSub (0.00s)
=== RUN   TestTable
=== RUN   TestTable/neg
    calc_test.go:30: neg failed
--- FAIL: TestTable (0.00s)
    --- FAIL: TestTable/neg (0.00s)
FAIL
FAIL\texample.com/calc\t0.002s",
        );
        let r = parse(Runner::Go, &out, &|p| rooted(&format!("pkg/{p}")));
        assert_eq!(r.passed.len(), 1);
        let names: Vec<_> = r.failed.iter().map(|f| f.full.as_str()).collect();
        assert_eq!(names, ["TestSub", "TestTable", "TestTable/neg"]);
        assert_eq!(r.failed[0].loc, Some((rooted("pkg/calc_test.go"), 13, 0)));
        assert_eq!(r.failed[0].message, "got 1, want 2");
        assert_eq!(
            r.failed[1].loc,
            Some((rooted("pkg/calc_test.go"), 29, 0)),
            "a parent with no complaint of its own takes its subtest's location"
        );
        assert_eq!(
            r.failed[2].id.name, "TestTable",
            "subtests mark their parent"
        );
        assert_eq!(r.failed[2].loc, Some((rooted("pkg/calc_test.go"), 29, 0)));
    }

    #[test]
    fn parses_jest_output() {
        let out = lines(
            "FAIL src/sum.test.js
  math
    ✓ adds (3 ms)
    ✕ subtracts (2 ms)

  ● math › subtracts

    expect(received).toBe(expected) // Object.is equality

      at Object.<anonymous> (node_modules/expect/build/index.js:1:1)
      at Object.<anonymous> (src/sum.test.js:7:17)

Tests:       1 failed, 1 passed, 2 total",
        );
        let r = parse(Runner::Jest, &out, &|p| rooted(p));
        assert_eq!(r.passed.len(), 1);
        assert_eq!(r.passed[0].name, "adds");
        assert_eq!(r.failed.len(), 1);
        let f = &r.failed[0];
        assert_eq!(f.id.name, "subtracts");
        assert_eq!(f.loc, Some((rooted("src/sum.test.js"), 6, 16)));
        assert!(f.message.starts_with("expect(received)"));
    }

    #[test]
    fn parses_vitest_output() {
        let out = lines(
            " ✓ src/a.test.ts > suite > passes 1ms
 × src/a.test.ts > suite > fails 2ms

 FAIL  src/a.test.ts > suite > fails
AssertionError: expected 1 to be 2
 ❯ src/a.test.ts:9:15",
        );
        let r = parse(Runner::Vitest, &out, &|p| rooted(p));
        assert_eq!(r.passed[0].name, "passes");
        let f = &r.failed[0];
        assert_eq!(f.id.name, "fails");
        assert_eq!(f.loc, Some((rooted("src/a.test.ts"), 8, 14)));
        assert_eq!(f.message, "AssertionError: expected 1 to be 2");
    }

    #[test]
    fn outcomes_merge_and_failures_win_over_same_named_passes() {
        let mut t = TestRunner::default();
        let id = |n: &str| TestId {
            name: n.into(),
            file: None,
        };
        let fail = |n: &str| Failure {
            id: id(n),
            full: n.into(),
            loc: None,
            message: String::new(),
        };
        t.record(&Report {
            passed: vec![id("a"), id("b")],
            failed: vec![fail("c")],
        });
        assert_eq!(t.mark(Some(Path::new("/p/x.rs")), "fn a() {"), Some('✓'));
        assert_eq!(
            t.mark(Some(Path::new("/p/x.rs")), "    fn c() {"),
            Some('✗')
        );
        assert_eq!(t.mark(Some(Path::new("/p/x.rs")), "fn d() {"), None);
        assert_eq!(t.mark(None, "fn a() {"), None);
        // A later run of just `c` passing updates it; `a` keeps its mark.
        t.record(&Report {
            passed: vec![id("c")],
            failed: vec![],
        });
        assert_eq!(t.mark(Some(Path::new("/p/x.rs")), "fn c() {"), Some('✓'));
        assert_eq!(t.mark(Some(Path::new("/p/x.rs")), "fn a() {"), Some('✓'));
        // A parametrized case failing marks the declaration failed even if
        // another case of it passed in the same run.
        t.record(&Report {
            passed: vec![id("b")],
            failed: vec![fail("b")],
        });
        assert_eq!(t.mark(Some(Path::new("/p/x.rs")), "fn b() {"), Some('✗'));
    }

    #[test]
    fn file_qualified_outcomes_win_over_bare_names() {
        let mut t = TestRunner::default();
        let a = PathBuf::from("/p/test_a.py");
        let b = PathBuf::from("/p/test_b.py");
        t.record(&Report {
            passed: vec![TestId {
                name: "test_x".into(),
                file: Some(a.clone()),
            }],
            failed: vec![Failure {
                id: TestId {
                    name: "test_x".into(),
                    file: Some(b.clone()),
                },
                full: "test_b.py::test_x".into(),
                loc: None,
                message: String::new(),
            }],
        });
        assert_eq!(t.mark(Some(&a), "def test_x():"), Some('✓'));
        assert_eq!(t.mark(Some(&b), "def test_x():"), Some('✗'));
    }

    fn run_to_completion(e: &mut Editor) {
        let start = std::time::Instant::now();
        while e.tests.running() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "run never finished"
            );
            e.poll_tests();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_run_streams_into_the_pane_then_lists_failures_and_marks() {
        let mut e = Editor::new(crate::config::Config::default());
        e.project_root = std::env::temp_dir();
        // A stand-in for `cargo test`: cargo-format output on both streams.
        let script = "printf 'test tests::good ... ok\\ntest tests::bad ... FAILED\\n'; \
            printf -- '---- tests::bad stdout ----\\nthread x panicked at src/lib.rs:4:5:\\nboom\\n' >&2; \
            exit 101";
        e.start_test(Spec {
            runner: Runner::Cargo,
            command: script.into(),
            file: None,
        });
        assert!(e.tests.running());
        assert_eq!(e.mode, Mode::Results, "the live output pane opens at once");
        assert!(e.results.as_ref().unwrap().test_output);
        run_to_completion(&mut e);
        assert_eq!(e.tests.output.len(), 5, "{:?}", e.tests.output);
        let r = e.results.as_ref().unwrap();
        assert!(r.quickfix && !r.test_output, "failures replace the pane");
        assert!(r.title.contains("1 failed, 1 passed"), "{}", r.title);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].line, 3);
        assert!(
            r.entries[0].text.contains("tests::bad  boom"),
            "{}",
            r.entries[0].text
        );
        assert!(e.quickfix.is_some());
        let p = Path::new("/x/lib.rs");
        assert_eq!(e.tests.mark(Some(p), "fn good() {"), Some('✓'));
        assert_eq!(e.tests.mark(Some(p), "fn bad() {"), Some('✗'));
        // `:testlast` re-runs the same command.
        e.test_last();
        assert!(e.tests.running());
        run_to_completion(&mut e);
        // `:testoutput` reopens the raw output.
        e.test_output();
        let r = e.results.as_ref().unwrap();
        assert!(r.test_output && r.entries.len() == 5);
    }

    #[test]
    fn a_passing_run_keeps_the_pane_and_reports_the_count() {
        let mut e = Editor::new(crate::config::Config::default());
        e.project_root = std::env::temp_dir();
        e.start_test(Spec {
            runner: Runner::Go,
            command: "echo '--- PASS: TestA (0.00s)'".into(),
            file: None,
        });
        run_to_completion(&mut e);
        let r = e.results.as_ref().unwrap();
        assert!(r.test_output && !r.busy);
        assert_eq!(r.title, "✓ echo '--- PASS: TestA (0.00s)' — 1 passed");
        assert!(
            e.quickfix.is_none(),
            "a green run leaves the quickfix alone"
        );
    }

    #[test]
    fn stop_kills_a_running_test() {
        let mut e = Editor::new(crate::config::Config::default());
        e.project_root = std::env::temp_dir();
        e.start_test(Spec {
            runner: Runner::Cargo,
            command: "echo started; sleep 30".into(),
            file: None,
        });
        let t = std::time::Instant::now();
        e.test_stop();
        assert!(!e.tests.running());
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(e.message, "Test run stopped");
        let r = e.results.as_ref().unwrap();
        assert_eq!(r.title, "✗ echo started; sleep 30 — stopped");
        assert!(!r.busy);
    }

    #[test]
    fn summary_titles() {
        let pass = Report {
            passed: vec![TestId {
                name: "a".into(),
                file: None,
            }],
            failed: vec![],
        };
        assert_eq!(
            summary("cargo test", &pass, true, Some(0)),
            "✓ cargo test — 1 passed"
        );
        assert_eq!(
            summary("make", &Report::default(), true, Some(0)),
            "✓ make — ok"
        );
        assert_eq!(
            summary("make", &Report::default(), false, Some(2)),
            "✗ make — exit 2"
        );
        assert_eq!(
            summary("make", &Report::default(), false, None),
            "✗ make — stopped"
        );
    }
}
