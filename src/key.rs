use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Enter,
    Esc,
    Backspace,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
}

impl Key {
    pub fn from_event(ev: KeyEvent) -> Option<Key> {
        match ev.code {
            KeyCode::Char(c) => {
                if ev.modifiers.contains(KeyModifiers::CONTROL) {
                    Some(Key::Ctrl(c.to_ascii_lowercase()))
                } else {
                    Some(Key::Char(c))
                }
            }
            KeyCode::Enter => Some(Key::Enter),
            KeyCode::Esc => Some(Key::Esc),
            KeyCode::Backspace => Some(Key::Backspace),
            KeyCode::Tab => Some(Key::Tab),
            KeyCode::Left => Some(Key::Left),
            KeyCode::Right => Some(Key::Right),
            KeyCode::Up => Some(Key::Up),
            KeyCode::Down => Some(Key::Down),
            KeyCode::Home => Some(Key::Home),
            KeyCode::End => Some(Key::End),
            KeyCode::Delete => Some(Key::Delete),
            KeyCode::PageUp => Some(Key::PageUp),
            KeyCode::PageDown => Some(Key::PageDown),
            _ => None,
        }
    }

    pub fn as_char(&self) -> Option<char> {
        match self {
            Key::Char(c) => Some(*c),
            _ => None,
        }
    }

    /// Render as a Neovim-style token, e.g. "<Esc>", "<C-r>", "a".
    pub fn token(&self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Ctrl(c) => format!("<C-{}>", c),
            Key::Enter => "<CR>".to_string(),
            Key::Esc => "<Esc>".to_string(),
            Key::Backspace => "<BS>".to_string(),
            Key::Tab => "<Tab>".to_string(),
            Key::Left => "<Left>".to_string(),
            Key::Right => "<Right>".to_string(),
            Key::Up => "<Up>".to_string(),
            Key::Down => "<Down>".to_string(),
            Key::Home => "<Home>".to_string(),
            Key::End => "<End>".to_string(),
            Key::Delete => "<Del>".to_string(),
            Key::PageUp => "<PageUp>".to_string(),
            Key::PageDown => "<PageDown>".to_string(),
        }
    }
}

pub fn keys_to_string(keys: &[Key]) -> String {
    keys.iter().map(|k| k.token()).collect::<Vec<_>>().join("")
}
