use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub leader: String,
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub expandtab: bool,
    pub number: bool,
    pub relativenumber: bool,
    pub scrolloff: usize,
    pub timeoutlen_ms: u64,
    pub wrap: bool,
    pub swap_0_and_caret: bool,
    pub jk_escape: bool,
    pub ignorecase: bool,
    pub smartcase: bool,
    /// Mirrors `vim.opt.clipboard = "unnamedplus"`: the unnamed register
    /// reads/writes the system clipboard, same as explicit `"+`/`"*` always
    /// do. Set false to keep yanks purely internal, like plain Vim.
    pub clipboard_unnamedplus: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            leader: ",".to_string(),
            tabstop: 4,
            shiftwidth: 4,
            expandtab: true,
            number: true,
            relativenumber: false,
            scrolloff: 8,
            timeoutlen_ms: 300,
            wrap: true,
            swap_0_and_caret: true,
            jk_escape: true,
            ignorecase: true,
            smartcase: true,
            clipboard_unnamedplus: true,
        }
    }
}

impl Config {
    pub fn load() -> Config {
        let path = Self::config_path();
        if let Some(path) = path {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match toml::from_str::<Config>(&text) {
                    Ok(cfg) => return cfg,
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
