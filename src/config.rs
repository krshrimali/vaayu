use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub review_command: Vec<String>,
    pub review_timeout_secs: u64,
    pub lsp: std::collections::BTreeMap<String, LspServer>,
    pub leader: String,
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub expandtab: bool,
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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            review_command: Vec::new(),
            review_timeout_secs: 300,
            lsp: Default::default(),
            leader: ",".to_string(),
            tabstop: 4,
            shiftwidth: 4,
            expandtab: true,
            number: true,
            relativenumber: false,
            scrolloff: 8,
            timeoutlen_ms: 300,
            whichkey_delay_ms: 500,
            wrap: true,
            swap_0_and_caret: true,
            jk_escape: true,
            ignorecase: true,
            smartcase: true,
            clipboard_unnamedplus: true,
            autopairs: true,
            completion_enabled: true,
            completion_delay_ms: 0,
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
