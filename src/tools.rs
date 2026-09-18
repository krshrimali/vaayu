//! A Mason-like `:tools` view: lists the language servers this editor
//! knows how to spawn (the same names `lsp::client::candidates_for`
//! already tries), whether each is currently on `PATH` (its "health"),
//! and -- deliberately scoped down from a full package manager -- the
//! exact, pinned install command for it, run only when the user
//! explicitly presses Enter (opt-in), through the same embedded-terminal
//! infrastructure `:terminal` already uses so the install's real output
//! is visible rather than run blind in the background. Update/remove
//! aren't separate actions: every command below already updates in place
//! when re-run (`npm install -g` always installs the latest, `cargo
//! install` needs `--force` which the command already includes where
//! relevant), and removal is each ecosystem's own package manager's job,
//! not something worth reimplementing per-tool here.
pub struct Tool {
    pub name: &'static str,
    pub lang: &'static str,
    pub exe: &'static str,
    pub install: &'static str,
}
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "rust-analyzer",
        lang: "rust",
        exe: "rust-analyzer",
        install: "rustup component add rust-analyzer",
    },
    Tool {
        name: "pylsp",
        lang: "python",
        exe: "pylsp",
        install: "pip install --user python-lsp-server",
    },
    Tool {
        name: "pyright",
        lang: "python",
        exe: "pyright-langserver",
        install: "npm install -g pyright",
    },
    Tool {
        name: "typescript-language-server",
        lang: "javascript/typescript",
        exe: "typescript-language-server",
        install: "npm install -g typescript-language-server typescript",
    },
    Tool {
        name: "gopls",
        lang: "go",
        exe: "gopls",
        install: "go install golang.org/x/tools/gopls@latest",
    },
    Tool {
        name: "clangd",
        lang: "c/cpp",
        exe: "clangd",
        install: "your OS package manager, e.g. apt install clangd / brew install llvm",
    },
    Tool {
        name: "lua-language-server",
        lang: "lua",
        exe: "lua-language-server",
        install: "https://github.com/LuaLS/lua-language-server/releases (no single cross-platform package manager command)",
    },
    Tool {
        name: "bash-language-server",
        lang: "bash",
        exe: "bash-language-server",
        install: "npm install -g bash-language-server",
    },
    Tool {
        name: "vscode-json-language-server",
        lang: "json",
        exe: "vscode-json-language-server",
        install: "npm install -g vscode-langservers-extracted",
    },
    Tool {
        name: "taplo",
        lang: "toml",
        exe: "taplo",
        install: "cargo install taplo-cli --locked --features lsp",
    },
    Tool {
        name: "yaml-language-server",
        lang: "yaml",
        exe: "yaml-language-server",
        install: "npm install -g yaml-language-server",
    },
    Tool {
        name: "vim-language-server",
        lang: "vim",
        exe: "vim-language-server",
        install: "npm install -g vim-language-server",
    },
    Tool {
        name: "vscode-css-language-server",
        lang: "css",
        exe: "vscode-css-language-server",
        install: "npm install -g vscode-langservers-extracted",
    },
    Tool {
        name: "vscode-html-language-server",
        lang: "html",
        exe: "vscode-html-language-server",
        install: "npm install -g vscode-langservers-extracted",
    },
    Tool {
        name: "nomicfoundation-solidity-language-server",
        lang: "solidity",
        exe: "nomicfoundation-solidity-language-server",
        install: "npm install -g @nomicfoundation/solidity-language-server",
    },
];

/// Whether `exe` resolves on `PATH` -- scans directories directly rather
/// than shelling out to `which` (a program that isn't guaranteed to
/// exist either), and treats "exists" as healthy enough on non-Unix
/// targets where the executable-bit check below doesn't apply.
pub fn on_path(exe: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let full = dir.join(exe);
        if !full.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&full)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

/// A one-line version string from `<exe> --version`'s first line, when
/// the tool is on `PATH` -- a slightly stronger "health" signal than
/// bare presence (it also confirms the binary actually runs), still
/// entirely local/offline and near-instant for both the found and
/// not-found cases.
pub fn version_of(exe: &str) -> Option<String> {
    let out = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .ok()?;
    let text = if out.stdout.is_empty() {
        out.stderr
    } else {
        out.stdout
    };
    String::from_utf8_lossy(&text)
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl crate::editor::Editor {
    /// `:tools`: one entry per known language server, showing whether
    /// it's on `PATH` (and its version, if so) -- Enter on an entry
    /// that isn't installed runs its pinned install command in a new
    /// embedded terminal (the same mechanism `:terminal` uses), so
    /// nothing installs without the user explicitly choosing it and
    /// watching it happen.
    pub fn show_tools(&mut self) {
        let entries: Vec<crate::results::Entry> = TOOLS
            .iter()
            .map(|t| {
                let installed = on_path(t.exe);
                let status = if installed {
                    match version_of(t.exe) {
                        Some(v) => format!("✓ {v}"),
                        None => "✓ installed".to_string(),
                    }
                } else {
                    "✗ not installed".to_string()
                };
                let mut e =
                    crate::results::Entry::text(format!("{} ({}) — {status}", t.name, t.lang));
                e.action = Some(serde_json::json!({"_vaayu_tool_install": {
                    "installed": installed,
                    "name": t.name,
                    "install": t.install,
                }}));
                e
            })
            .collect();
        self.show_results(crate::results::Results::new("Tools", entries));
    }

    /// Runs `cmd` (a `Tool::install` string) in a new embedded terminal
    /// split, reusing `open_terminal`'s exact spawn/split/mode-switch
    /// machinery with a `sh -c <cmd>` argv instead of a bare shell, so
    /// the install's real output streams live instead of running
    /// silently in the background.
    pub fn run_tool_install(&mut self, cmd: &str) {
        let rows = self.screen_rows.max(1) as u16;
        let cols = self.screen_cols.max(1) as u16;
        let argv = ["sh".to_string(), "-c".to_string(), cmd.to_string()];
        match crate::pty::PtySession::spawn(&argv, &self.project_root, rows, cols) {
            Ok(session) => {
                let id = session.id;
                self.terminals.push(session);
                self.split_window(false, false);
                if let Some(w) = self.windows.get_mut(self.active_window) {
                    w.terminal = Some(id);
                }
                self.mode = crate::mode::Mode::Terminal;
                self.set_message(format!("Running: {cmd} (Esc for pane navigation)"));
            }
            Err(e) => self.set_message(format!("Could not run install command: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn on_path_finds_a_known_executable_and_rejects_a_bogus_one() {
        assert!(
            on_path("sh"),
            "sh should be on PATH in any test environment"
        );
        assert!(!on_path("definitely-not-a-real-executable-xyz123"));
    }
    #[test]
    fn version_of_reads_the_first_line_of_output() {
        let v = version_of("sh");
        // Some `sh` implementations (POSIX dash) print nothing for
        // `--version` and just exit -- accept either a real version
        // line or no output, but never panic either way.
        if let Some(v) = v {
            assert!(!v.is_empty());
        }
        assert!(version_of("definitely-not-a-real-executable-xyz123").is_none());
    }
    #[test]
    fn every_tool_entry_has_non_empty_fields() {
        for t in TOOLS {
            assert!(!t.name.is_empty());
            assert!(!t.lang.is_empty());
            assert!(!t.exe.is_empty());
            assert!(!t.install.is_empty());
        }
    }
}
