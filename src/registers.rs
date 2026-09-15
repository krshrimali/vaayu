use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct RegisterEntry {
    pub text: String,
    pub linewise: bool,
}

pub struct Registers {
    map: HashMap<char, RegisterEntry>,
}

impl Registers {
    pub fn new() -> Registers {
        Registers { map: HashMap::new() }
    }

    pub fn set(&mut self, reg: Option<char>, text: String, linewise: bool) {
        let entry = RegisterEntry { text, linewise };
        // Named register, if given.
        if let Some(r) = reg {
            self.map.insert(r, entry.clone());
        }
        // Unnamed register always receives the most recent yank/delete.
        self.map.insert('"', entry);
    }

    pub fn get(&self, reg: Option<char>) -> Option<&RegisterEntry> {
        self.map.get(&reg.unwrap_or('"'))
    }
}
