use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub review_command: Vec<String>,
    pub review_timeout_secs: u64,
    /// `:claude`/`:codex`/`:agent <name>`: argv for each named long-lived
    /// agent terminal session. A name with no entry here falls back to
    /// running its own bare name as the command (so `:claude`/`:codex`
    /// work out of the box when those CLIs are already on `PATH`,
    /// without requiring any config at all); this map only exists for
    /// overriding that (a wrapper script, extra flags, a different
    /// binary name entirely).
    pub agent_commands: std::collections::BTreeMap<String, Vec<String>>,
    pub lsp: std::collections::BTreeMap<String, LspServer>,
    pub leader: String,
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub expandtab: bool,
    /// When on (the default), a new line opened by Enter/`o`/`O` after a line
    /// ending in an opening bracket (`{`/`(`/`[`) gains one extra indent
    /// level. When off, the new line only copies the source line's indent.
    pub smartindent: bool,
    /// Automatically highlight other occurrences of the symbol under the
    /// cursor (LSP `documentHighlight`) once it rests for `updatetime_ms`.
    /// Default on; silently does nothing without a capable language server.
    pub illuminate: bool,
    /// Idle time (ms) the cursor must rest before the `CursorHold` event
    /// fires (and, if enabled, illuminate requests document highlights).
    pub updatetime_ms: u64,
    pub number: bool,
    pub relativenumber: bool,
    pub scrolloff: usize,
    pub timeoutlen_ms: u64,
    /// Delay before the which-key prefix popup appears after a leader
    /// sequence like `,l` is typed with no continuation yet. Kept separate
    /// from `timeoutlen_ms`: unlike `jk` escape or ambiguous-motion
    /// timeouts, showing this popup never changes what a completed mapping
    /// does, so it can default independently.
    pub whichkey_delay_ms: u64,
    pub wrap: bool,
    /// The marker shown in the gutter of soft-wrapped continuation rows
    /// (Vim's `showbreak`). Empty -- the default -- shows nothing, so a
    /// wrapped line's continued rows have a blank gutter; set e.g. "↪" or
    /// "> " to mark them. Only applies when `wrap` is on.
    pub showbreak: String,
    pub swap_0_and_caret: bool,
    pub jk_escape: bool,
    pub ignorecase: bool,
    pub smartcase: bool,
    /// Mirrors `vim.opt.clipboard = "unnamedplus"`: the unnamed register
    /// reads/writes the system clipboard, same as explicit `"+`/`"*` always
    /// do. Set false to keep yanks purely internal, like plain Vim.
    pub clipboard_unnamedplus: bool,
    pub autopairs: bool,
    /// Disables the completion popup (both the buffer-word and LSP
    /// sources) entirely when false; `update_completion` becomes a no-op.
    /// Manual insertion still works fine -- this only stops the automatic
    /// as-you-type popup, matching an editor-wide "I find this
    /// distracting" preference rather than per-source tuning.
    pub completion_enabled: bool,
    /// How long the completion popup waits, after the triggering
    /// keystroke, before actually appearing -- `0` (the default) shows
    /// it instantly, matching this editor's existing behavior. The
    /// candidates are still computed immediately either way; this only
    /// gates when the popup is *painted*, the same render-time delay
    /// technique `whichkey_delay_ms` already uses, so raising it doesn't
    /// change what shows up, only how long a fast typist goes without
    /// the popup flashing in and out on every keystroke.
    pub completion_delay_ms: u64,
    /// Whether a line's real (non-gutter-marker) diagnostic message shows
    /// as virtual text after its own content, the current line only (to
    /// avoid cluttering every line with an error/warning tail). The
    /// gutter's E/W/I marker and the underline on the diagnostic's own
    /// range are unaffected by this -- only the extra text tail.
    pub diagnostics_virtual_text: bool,
    /// Whether newly published diagnostics update what's shown while in
    /// Insert mode. `false` (the default, matching Neovim's own default)
    /// means new diagnostics are still recorded but the visible set
    /// doesn't change mid-typing -- it catches up the moment Insert
    /// mode ends -- so a fast typist isn't distracted by error
    /// underlines/messages flickering on every keystroke.
    pub diagnostics_update_in_insert: bool,
    /// On `:w`, remove trailing spaces/tabs from every line (opt-in).
    pub trim_trailing_whitespace: bool,
    /// On `:w`, ensure a non-empty buffer ends with exactly one `\n` (opt-in).
    pub insert_final_newline: bool,
    /// Declarative autocommands: run an Ex `command` when `event` fires on a
    /// buffer whose name matches the glob `pattern` (default `*`). Parsed from
    /// `[[autocmd]]` tables. See `event.rs` for the supported event names.
    pub autocmd: Vec<Autocmd>,
    /// User key remaps, parsed from `[[keymap]]` tables. See `keymap.rs`.
    pub keymap: Vec<KeymapCfg>,
}

/// One `[[keymap]]` entry: remap `lhs` to `rhs` in the given `mode`(s).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KeymapCfg {
    /// Any combination of `n` (normal), `i` (insert), `v` (visual).
    pub mode: String,
    pub lhs: String,
    pub rhs: String,
}

impl Default for KeymapCfg {
    fn default() -> Self {
        KeymapCfg {
            mode: "n".into(),
            lhs: String::new(),
            rhs: String::new(),
        }
    }
}

/// One `[[autocmd]]` entry from the config.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Autocmd {
    pub event: String,
    pub pattern: String,
    pub command: String,
}

impl Default for Autocmd {
    fn default() -> Self {
        Autocmd {
            event: String::new(),
            pattern: "*".into(),
            command: String::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            review_command: Vec::new(),
            review_timeout_secs: 300,
            agent_commands: Default::default(),
            lsp: Default::default(),
            leader: ",".to_string(),
            tabstop: 4,
            shiftwidth: 4,
            expandtab: true,
            smartindent: true,
            illuminate: true,
            updatetime_ms: 250,
            number: true,
            relativenumber: false,
            scrolloff: 8,
            timeoutlen_ms: 300,
            whichkey_delay_ms: 500,
            wrap: true,
            showbreak: String::new(),
            swap_0_and_caret: true,
            jk_escape: true,
            ignorecase: true,
            smartcase: true,
            clipboard_unnamedplus: true,
            autopairs: true,
            completion_enabled: true,
            completion_delay_ms: 0,
            diagnostics_virtual_text: true,
            diagnostics_update_in_insert: false,
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            autocmd: Vec::new(),
            keymap: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Config {
        let path = Self::config_path();
        if let Some(path) = path {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match toml::from_str::<Config>(&text) {
                    Ok(mut cfg) => {
                        cfg.tabstop = cfg.tabstop.clamp(1, 32);
                        cfg.shiftwidth = cfg.shiftwidth.clamp(1, 32);
                        if cfg.leader.chars().count() != 1 {
                            cfg.leader = ",".into();
                        }
                        return cfg;
                    }
                    Err(e) => {
                        eprintln!("vaayu: failed to parse {}: {}", path.display(), e);
                    }
                }
            }
        }
        Config::default()
    }

    fn config_path() -> Option<PathBuf> {
        let base = dirs::config_dir()?;
        Some(base.join("vaayu").join("config.toml"))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LspServer {
    pub request_timeout_ms: u64,
    pub cmd: Vec<String>,
    pub filetypes: Vec<String>,
    pub root_markers: Vec<String>,
    pub settings: serde_json::Value,
    pub init_options: serde_json::Value,
    pub capabilities: serde_json::Value,
    pub env: std::collections::BTreeMap<String, String>,
    pub enabled: bool,
}
impl Default for LspServer {
    fn default() -> Self {
        Self {
            request_timeout_ms: 15000,
            cmd: Vec::new(),
            filetypes: Vec::new(),
            root_markers: vec![
                "Cargo.toml".into(),
                "pyproject.toml".into(),
                "package.json".into(),
                "go.mod".into(),
                ".git".into(),
            ],
            settings: serde_json::json!({}),
            init_options: serde_json::Value::Null,
            capabilities: serde_json::Value::Null,
            env: Default::default(),
            enabled: true,
        }
    }
}
