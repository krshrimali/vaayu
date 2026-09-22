use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct RegisterEntry {
    pub text: String,
    pub linewise: bool,
    pub block_width: Option<usize>,
}

pub struct Registers {
    map: HashMap<char, RegisterEntry>,
    /// Mirrors `clipboard=unnamedplus`: the unnamed register (no explicit
    /// `"x` prefix) reads through to and writes through to the system
    /// clipboard, same as `+`/`*` always do regardless of this setting.
    unnamedplus: bool,
}

impl Registers {
    pub fn new(unnamedplus: bool) -> Registers {
        Registers {
            map: HashMap::new(),
            unnamedplus,
        }
    }

    fn is_clipboard_register(&self, reg: Option<char>) -> bool {
        matches!(reg, Some('+') | Some('*')) || (reg.is_none() && self.unnamedplus)
    }

    /// All currently-set registers, sorted by name, for the `:reg` viewer.
    pub fn list(&self) -> Vec<(char, RegisterEntry)> {
        let mut v: Vec<(char, RegisterEntry)> =
            self.map.iter().map(|(k, e)| (*k, e.clone())).collect();
        v.sort_by_key(|(k, _)| *k);
        v
    }

    pub fn set(&mut self, reg: Option<char>, text: String, linewise: bool) {
        if reg == Some('_') {
            return;
        }
        let mut entry = RegisterEntry {
            text: text.clone(),
            linewise,
            block_width: None,
        };
        // Named register, if given.
        if let Some(r) = reg {
            if r.is_ascii_uppercase() {
                let lower = r.to_ascii_lowercase();
                if let Some(old) = self.map.get(&lower) {
                    entry.text = format!("{}{}", old.text, entry.text);
                }
                self.map.insert(lower, entry.clone());
            } else {
                self.map.insert(r, entry.clone());
            }
        }
        // Unnamed register always receives the most recent yank/delete.
        self.map.insert('"', entry);
        if self.is_clipboard_register(reg) {
            crate::clipboard::copy(&text);
        }
    }

    pub fn set_block(&mut self, reg: Option<char>, text: String, width: usize) {
        self.set(reg, text, false);
        if reg == Some('_') {
            return;
        }
        for key in [reg.unwrap_or('"').to_ascii_lowercase(), '"'] {
            if let Some(e) = self.map.get_mut(&key) {
                e.block_width = Some(width);
            }
        }
    }
    /// Reads a register. For a clipboard-backed register on a local session,
    /// refreshes from the system clipboard first, so a copy made in another
    /// app shows up on paste -- real Vim's `unnamedplus`/`+`/`*` behavior.
    /// Over SSH (or if the clipboard tool isn't available) this is a no-op
    /// and the last known in-memory value is used, same as pasting from the
    /// plain unnamed register.
    pub fn get(&mut self, reg: Option<char>) -> Option<&RegisterEntry> {
        if self.is_clipboard_register(reg) {
            if let Some(text) = crate::clipboard::paste() {
                if self
                    .map
                    .get(&reg.unwrap_or('"'))
                    .is_some_and(|e| e.text == text)
                {
                    return self.map.get(&reg.unwrap_or('"'));
                }
                let linewise = !text.is_empty() && text.ends_with('\n');
                let entry = RegisterEntry {
                    text,
                    linewise,
                    block_width: None,
                };
                if let Some(r) = reg {
                    self.map.insert(r, entry.clone());
                }
                self.map.insert('"', entry);
            }
        }
        self.map.get(&reg.unwrap_or('"'))
    }
}
