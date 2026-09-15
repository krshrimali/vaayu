use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Literal(char),
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
}
