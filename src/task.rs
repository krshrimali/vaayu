//! `:make` / `:task` — run a build/test command in the project root on a
//! background thread and parse its `file:line:col: message` output into the
//! quickfix list. The command is either given explicitly or defaulted from the
//! project's marker files. This is a user-invoked build runner (like Vim's
//! `:make`), so it runs the given command through the shell.

use crate::editor::Editor;
use crate::results::{Entry, Results};
use std::path::Path;
use std::sync::mpsc;

/// Build a per-language "run this one test" command from a file extension and a
/// test-function name. `None` for languages without a configured runner.
pub(crate) fn test_command_for(ext: &str, name: &str) -> Option<String> {
    Some(match ext {
        "rs" => format!("cargo test {name}"),
        "py" | "pyi" => format!("pytest -k {name}"),
        "go" => format!("go test -run {name} ./..."),
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => format!("npm test -- -t {name}"),
        _ => return None,
    })
}

impl Editor {
    /// `:make [cmd]` / `:task [cmd]`: run `cmd` (or a detected default) and
    /// route its output into the quickfix list.
    /// The name of the function enclosing the cursor (tree-sitter), for
    /// test-under-cursor. Extracts the identifier before the first `(` on the
    /// declaration line, which works for `fn`/`def`/`func`/method forms.
    pub(crate) fn enclosing_function_name(&self) -> Option<String> {
        let syn = self.syntax.as_ref()?;
        let b = self.buf();
        let (line, col) = self.cursor();
        let ci = b.char_idx(line, col);
        // Use the cursor's own byte (mid-line) so a function whose declaration
        // starts at the line head still contains it.
        let cursor_byte = b.rope.char_to_byte(ci);
        let start = syn
            .context_starts(cursor_byte, crate::editor::FUNCTION_KINDS)
            .last()
            .copied()?;
        let dl = b.pos_from_char_idx(b.rope.byte_to_char(start)).0;
        let decl = b.line_text(dl);
        let paren = decl.find('(')?;
        let name: String = decl[..paren]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        (!name.is_empty()).then_some(name)
    }

    /// `:testnearest` -- run the test function enclosing the cursor via the task
    /// runner, choosing a per-language command from the buffer's extension.
    pub fn test_nearest(&mut self) {
        let Some(name) = self.enclosing_function_name() else {
            self.set_message("No enclosing function to test");
            return;
        };
        let ext = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("");
        match test_command_for(ext, &name) {
            Some(cmd) => self.run_task(&cmd),
            None => self.set_message(format!("No test runner configured for .{ext}")),
        }
    }

    pub fn run_task(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        let command = if cmd.is_empty() {
            match self.default_task_command() {
                Some(c) => c,
                None => {
                    self.set_message(
                        "No default build command (no Cargo.toml/go.mod/package.json/Makefile) — use :make <cmd>",
                    );
                    return;
                }
            }
        } else {
            cmd.to_string()
        };
        let root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.make_task = Some(rx);
        self.set_message(format!("Running: {command}…"));
        let title = command.clone();
        std::thread::spawn(move || {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(&root)
                .output();
            let result = match out {
                Ok(o) => {
                    let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                    text.push_str(&String::from_utf8_lossy(&o.stderr));
                    let parsed = parse_errorformat(&text, &root);
                    let heading = if parsed.is_empty() {
                        format!(
                            "{title} — {}",
                            if o.status.success() {
                                "ok"
                            } else {
                                "failed"
                            }
                        )
                    } else {
                        format!("{title} — {} location(s)", parsed.len())
                    };
                    // When nothing parsed, still surface the raw output so the
                    // user can read compiler messages / test failures.
                    let entries = if parsed.is_empty() {
                        text.lines().map(Entry::text).collect()
                    } else {
                        parsed
                    };
                    Ok(Results::new(heading, entries))
                }
                Err(e) => Err(format!("failed to run command: {e}")),
            };
            let _ = tx.send(result);
        });
    }

    fn default_task_command(&self) -> Option<String> {
        let root = &self.project_root;
        if root.join("Cargo.toml").exists() {
            Some("cargo build".into())
        } else if root.join("go.mod").exists() {
            Some("go build ./...".into())
        } else if root.join("package.json").exists() {
            Some("npm run build".into())
        } else if root.join("Makefile").exists() {
            Some("make".into())
        } else {
            None
        }
    }

    /// Drains a finished `:make`/`:task` run into the quickfix list.
    pub fn poll_make_task(&mut self) -> bool {
        let Some(result) = self.make_task.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.make_task = None;
        match result {
            Ok(mut r) => {
                r.quickfix = true;
                r.live = false;
                self.quickfix = Some(r.clone());
                if !self.quickfix_history.is_empty() {
                    self.quickfix_history.truncate(self.quickfix_history_pos + 1);
                }
                self.quickfix_history.push(r.clone());
                self.quickfix_history_pos = self.quickfix_history.len() - 1;
                let title = r.title.clone();
                if self.mode == crate::mode::Mode::Insert {
                    self.results = Some(r);
                    self.set_message(format!("{title} — :copen / Ctrl-Q to view"));
                } else {
                    self.show_results(r);
                }
            }
            Err(e) => self.set_message(e),
        }
        true
    }
}

/// Parses common `file:line[:col][:] message` compiler/linter output into
/// quickfix entries. Also accepts Rust's `--> file:line:col` location lines.
/// A "path" must contain a `.` or `/` to avoid matching bare `12:34` pairs.
fn parse_errorformat(text: &str, root: &Path) -> Vec<Entry> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(?:-->\s*)?([^\s:][^:]*):(\d+)(?::(\d+))?:?\s*(.*)$").unwrap()
    });
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(c) = re.captures(line) else {
            continue;
        };
        let path = &c[1];
        if !path.contains('.') && !path.contains('/') {
            continue; // not a filename/path — skip bare "10:30" style matches
        }
        let lno: usize = c[2].parse().unwrap_or(1);
        let col: usize = c.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        let msg = c.get(4).map(|m| m.as_str().trim()).unwrap_or("");
        let display = format!("{path}:{lno}:{col}  {msg}");
        out.push(Entry::location(
            root.join(path),
            lno.saturating_sub(1),
            col.saturating_sub(1),
            display,
        ));
    }
    out
}
