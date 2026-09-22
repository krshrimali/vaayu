//! User key remapping, parsed from `[[keymap]]` config entries (config.rs
//! `KeymapCfg`). Supports single-key remaps in Normal/Insert/Visual and
//! leader-sequence remaps (`<leader>x`). The right-hand side is either a key
//! sequence to replay (`noremap` semantics -- it is not itself remapped) or an
//! Ex command when it begins with `:`. Multi-key non-leader left-hand sides are
//! out of scope for this slice.
use crate::editor::Editor;
use crate::key::Key;
use crate::mode::Mode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rhs {
    Keys(Vec<Key>),
    Ex(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Modes {
    pub normal: bool,
    pub insert: bool,
    pub visual: bool,
}

#[derive(Clone, Debug)]
pub struct Keymap {
    pub modes: Modes,
    pub lhs: Vec<Key>,
    /// For a leader mapping, the key sequence after the leader; empty otherwise.
    pub leader_seq: Option<String>,
    pub rhs: Rhs,
}

fn parse_modes(s: &str) -> Modes {
    Modes {
        normal: s.contains('n'),
        insert: s.contains('i'),
        visual: s.contains('v'),
    }
}

/// Parse a single `<...>` token (already stripped of the angle brackets).
fn parse_token(tok: &str, leader: char) -> Option<Vec<Key>> {
    let lower = tok.to_ascii_lowercase();
    let key = match lower.as_str() {
        "leader" => Key::Char(leader),
        "cr" | "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Esc,
        "tab" => Key::Tab,
        "s-tab" | "bs-tab" => Key::BackTab,
        "bs" | "backspace" => Key::Backspace,
        "del" | "delete" => Key::Delete,
        "space" => Key::Char(' '),
        "bar" => Key::Char('|'),
        "lt" => Key::Char('<'),
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        _ => {
            // <C-x> / <c-x>
            if let Some(rest) = lower.strip_prefix("c-") {
                let c = rest.chars().next()?;
                Key::Ctrl(c)
            } else {
                return None;
            }
        }
    };
    Some(vec![key])
}

/// Parse a key-notation string ("Y", "jj", "<leader>x", "<C-s>", ":w<CR>") into
/// keys. `<...>` tokens are expanded; other characters are literal.
pub fn parse_keys(notation: &str, leader: char) -> Vec<Key> {
    let mut out = Vec::new();
    let chars: Vec<char> = notation.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == '>') {
                let tok: String = chars[i + 1..i + 1 + close].iter().collect();
                if let Some(keys) = parse_token(&tok, leader) {
                    out.extend(keys);
                    i += close + 2;
                    continue;
                }
            }
            // Not a recognised token: treat '<' literally.
            out.push(Key::Char('<'));
            i += 1;
        } else {
            out.push(Key::Char(chars[i]));
            i += 1;
        }
    }
    out
}

fn parse_rhs(rhs: &str, leader: char) -> Rhs {
    if let Some(stripped) = rhs.strip_prefix(':') {
        // Ex command; drop a trailing <CR>/<Enter> the way a real mapping has.
        let body = stripped
            .strip_suffix("<CR>")
            .or_else(|| stripped.strip_suffix("<cr>"))
            .or_else(|| stripped.strip_suffix("<Enter>"))
            .unwrap_or(stripped);
        Rhs::Ex(body.to_string())
    } else {
        Rhs::Keys(parse_keys(rhs, leader))
    }
}

/// Build the parsed keymaps from config. Invalid/empty entries are skipped.
pub fn build(cfgs: &[crate::config::KeymapCfg], leader: char) -> Vec<Keymap> {
    let mut out = Vec::new();
    for c in cfgs {
        if c.lhs.is_empty() || c.rhs.is_empty() {
            continue;
        }
        let lhs = parse_keys(&c.lhs, leader);
        if lhs.is_empty() {
            continue;
        }
        // A leader mapping is one whose lhs begins with the leader key.
        let leader_seq = if lhs.first() == Some(&Key::Char(leader)) && lhs.len() > 1 {
            Some(c.lhs.chars().skip(leader_marker_len(&c.lhs)).collect())
        } else {
            None
        };
        out.push(Keymap {
            modes: parse_modes(&c.mode),
            lhs,
            leader_seq,
            rhs: parse_rhs(&c.rhs, leader),
        });
    }
    out
}

/// How many chars of the lhs string represent the leader prefix (either the
/// literal leader char, or the `<leader>` token).
fn leader_marker_len(lhs: &str) -> usize {
    let low = lhs.to_ascii_lowercase();
    if low.starts_with("<leader>") {
        "<leader>".len()
    } else {
        1
    }
}

impl Editor {
    fn mode_matches(&self, m: &Modes) -> bool {
        match self.mode {
            Mode::Normal => m.normal,
            Mode::Insert => m.insert,
            Mode::Visual(_) => m.visual,
            _ => false,
        }
    }

    /// A single-key remap for the current mode/key, applicable only in a clean
    /// pending state and not while replaying (so a mapping's own expansion is
    /// not itself remapped -- `noremap`).
    pub fn single_key_remap(&self, key: Key) -> Option<Rhs> {
        if self.replaying || !self.pending.is_empty() {
            return None;
        }
        self.keymaps
            .iter()
            .find(|k| k.leader_seq.is_none() && k.lhs.len() == 1 && k.lhs[0] == key && self.mode_matches(&k.modes))
            .map(|k| k.rhs.clone())
    }

    /// An exact leader-sequence remap (`seq` is the keys typed after leader).
    pub fn leader_remap(&self, seq: &str) -> Option<Rhs> {
        self.keymaps
            .iter()
            .find(|k| k.leader_seq.as_deref() == Some(seq) && self.mode_matches(&k.modes))
            .map(|k| k.rhs.clone())
    }

    /// True if some leader remap's sequence starts with `seq` but is longer
    /// (so the editor should keep waiting for more keys).
    pub fn leader_remap_prefix(&self, seq: &str) -> bool {
        self.keymaps.iter().any(|k| {
            k.leader_seq
                .as_deref()
                .is_some_and(|s| s.starts_with(seq) && s.len() > seq.len())
                && self.mode_matches(&k.modes)
        })
    }

    /// Apply a remap's right-hand side: replay keys (noremap), or run an Ex
    /// command.
    pub fn apply_remap(&mut self, rhs: Rhs) {
        match rhs {
            Rhs::Keys(keys) => self.replay(&keys),
            Rhs::Ex(cmd) => crate::command::run_ex(self, &cmd),
        }
    }
}
