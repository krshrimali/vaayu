use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct RegisterEntry {
    pub text: String,
    pub linewise: bool,
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
        Registers { map: HashMap::new(), unnamedplus }
    }

    fn is_clipboard_register(&self, reg: Option<char>) -> bool {
        matches!(reg, Some('+') | Some('*')) || (reg.is_none() && self.unnamedplus)
    }

    pub fn set(&mut self, reg: Option<char>, text: String, linewise: bool) {
        let entry = RegisterEntry { text: text.clone(), linewise };
        // Named register, if given.
        if let Some(r) = reg {
            self.map.insert(r, entry.clone());
        }
        // Unnamed register always receives the most recent yank/delete.
        self.map.insert('"', entry);
        if self.is_clipboard_register(reg) {
            crate::clipboard::copy(&text);
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
                let linewise = !text.is_empty() && text.ends_with('\n');
                let entry = RegisterEntry { text, linewise };
                if let Some(r) = reg {
                    self.map.insert(r, entry.clone());
                }
                self.map.insert('"', entry);
            }
        }
        self.map.get(&reg.unwrap_or('"'))
    }
}
