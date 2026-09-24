use std::collections::HashMap;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
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

    /// Directly set a register entry (used when restoring persisted registers
    /// from shada; bypasses the uppercase-append and unnamed-mirror rules).
    pub fn restore(&mut self, name: char, entry: RegisterEntry) {
        self.map.insert(name, entry);
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
                    // Append: keep `linewise` sticky (Vim keeps an append to a
                    // linewise register linewise) and separate a linewise base
                    // from the appended text with a newline.
                    let mut combined = old.text.clone();
                    if old.linewise && !combined.ends_with('\n') {
                        combined.push('\n');
                    }
                    combined.push_str(&entry.text);
                    entry.text = combined;
                    entry.linewise = old.linewise || linewise;
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

    /// Record a yank: fills register `0` (the yank register) when no register
    /// was named, in addition to the named/unnamed registers Vim always sets.
    pub fn yank(&mut self, reg: Option<char>, text: String, linewise: bool) {
        if reg.is_none() {
            self.map.insert(
                '0',
                RegisterEntry {
                    text: text.clone(),
                    linewise,
                    block_width: None,
                },
            );
        }
        self.set(reg, text, linewise);
    }

    /// Record a delete/change: when no register was named, a delete of one or
    /// more lines shifts registers `1`..`8` into `2`..`9` and stores into `1`,
    /// while a delete of less than a line goes to the small-delete register `-`
    /// (Vim's numbered/small-delete semantics).
    pub fn delete(&mut self, reg: Option<char>, text: String, linewise: bool) {
        if reg.is_none() {
            let entry = RegisterEntry {
                text: text.clone(),
                linewise,
                block_width: None,
            };
            if !linewise && !text.contains('\n') {
                self.map.insert('-', entry);
            } else {
                for n in (1..=8u8).rev() {
                    let from = (b'0' + n) as char;
                    let to = (b'0' + n + 1) as char;
                    if let Some(e) = self.map.get(&from).cloned() {
                        self.map.insert(to, e);
                    }
                }
                self.map.insert('1', entry);
            }
        }
        self.set(reg, text, linewise);
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
