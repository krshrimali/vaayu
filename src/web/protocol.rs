//! Messages between the server and the tab. Input arrives as JSON and becomes
//! the crossterm events the terminal frontend would have produced, so dispatch
//! cannot tell them apart. Frames go out as binary messages; the rest is JSON.
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthChar;

#[derive(Clone)]
pub struct Font {
    pub family: String,
    pub size: u16,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ClientMessage {
    Size {
        cols: u16,
        rows: u16,
        cw: u16,
        ch: u16,
    },
    Key {
        key: String,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        shift: bool,
    },
    Mouse {
        kind: String,
        #[serde(default)]
        button: u8,
        col: u16,
        row: u16,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        shift: bool,
    },
    Paste {
        text: String,
    },
    Focus,
    Clipboard {
        text: String,
    },
}

fn modifiers(ctrl: bool, alt: bool, shift: bool) -> KeyModifiers {
    let mut mods = KeyModifiers::NONE;
    mods.set(KeyModifiers::CONTROL, ctrl);
    mods.set(KeyModifiers::ALT, alt);
    mods.set(KeyModifiers::SHIFT, shift);
    mods
}

/// Translates a browser `KeyboardEvent.key`. A chord a terminal encodes as a
/// control byte is reported the way crossterm decodes that byte, because that
/// is the form the keymaps were written against.
pub fn key_event(key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<Event> {
    let mut mods = modifiers(ctrl, alt, shift);
    let code = match key {
        "Enter" => KeyCode::Enter,
        "Escape" => KeyCode::Esc,
        "Backspace" => KeyCode::Backspace,
        "Tab" if shift => {
            mods.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "Tab" => KeyCode::Tab,
        "ArrowLeft" => KeyCode::Left,
        "ArrowRight" => KeyCode::Right,
        "ArrowUp" => KeyCode::Up,
        "ArrowDown" => KeyCode::Down,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "Delete" => KeyCode::Delete,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            let control_byte = ctrl.then(|| match c.to_ascii_lowercase() {
                '[' => Some(KeyCode::Esc),
                'i' => Some(KeyCode::Tab),
                'm' => Some(KeyCode::Enter),
                _ => None,
            });
            if let Some(Some(code)) = control_byte {
                mods.remove(KeyModifiers::CONTROL);
                code
            } else if ctrl {
                KeyCode::Char(match c {
                    '@' | '`' | '2' => ' ',
                    '\\' => '4',
                    ']' => '5',
                    '^' | '6' => '6',
                    '_' | '/' | '-' | '7' => '7',
                    c => c.to_ascii_lowercase(),
                })
            } else {
                KeyCode::Char(c)
            }
        }
    };
    Some(Event::Key(KeyEvent::new(code, mods)))
}

pub fn mouse_event(
    kind: &str,
    button: u8,
    column: u16,
    row: u16,
    modifiers_held: KeyModifiers,
) -> Option<Event> {
    let button = match button {
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::Left,
    };
    let kind = match kind {
        "down" => MouseEventKind::Down(button),
        "up" => MouseEventKind::Up(button),
        "drag" => MouseEventKind::Drag(button),
        "move" => MouseEventKind::Moved,
        "wheelup" => MouseEventKind::ScrollUp,
        "wheeldown" => MouseEventKind::ScrollDown,
        "wheelleft" => MouseEventKind::ScrollLeft,
        "wheelright" => MouseEventKind::ScrollRight,
        _ => return None,
    };
    Some(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: modifiers_held,
    }))
}

pub fn input_event(message: ClientMessage) -> Option<Event> {
    match message {
        ClientMessage::Key {
            key,
            ctrl,
            alt,
            shift,
        } => key_event(&key, ctrl, alt, shift),
        ClientMessage::Mouse {
            kind,
            button,
            col,
            row,
            ctrl,
            alt,
            shift,
        } => mouse_event(&kind, button, col, row, modifiers(ctrl, alt, shift)),
        ClientMessage::Paste { text } => Some(Event::Paste(text)),
        ClientMessage::Focus => Some(Event::FocusGained),
        ClientMessage::Size { cols, rows, .. } => Some(Event::Resize(cols, rows)),
        ClientMessage::Clipboard { .. } => None,
    }
}

/// Every character whose cell width is not one, as `[first, last, width]`. The
/// tab must advance exactly as the renderer counted; sharing the table means
/// the two can't disagree across `unicode-width` upgrades.
fn width_ranges() -> Vec<[u32; 3]> {
    let mut ranges: Vec<[u32; 3]> = Vec::new();
    for code in 0x80..=0x10_ffffu32 {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        let width = c.width().unwrap_or(0) as u32;
        if width == 1 {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if last[1] + 1 == code && last[2] == width => last[1] = code,
            _ => ranges.push([code, code, width]),
        }
    }
    ranges
}

pub fn hello(font: &Font) -> String {
    static TABLES: OnceLock<(Vec<[u32; 3]>, Vec<u32>)> = OnceLock::new();
    let (widths, diacritics) =
        TABLES.get_or_init(|| (width_ranges(), crate::graphics::DIACRITICS.to_vec()));
    json!({
        "t": "hello",
        "version": env!("CARGO_PKG_VERSION"),
        "widths": widths,
        "diacritics": diacritics,
        "font": { "family": font.family, "size": font.size },
    })
    .to_string()
}

pub fn clipboard(text: &str) -> String {
    json!({ "t": "clipboard", "text": text }).to_string()
}

pub fn title(title: &str) -> String {
    json!({ "t": "title", "title": title }).to_string()
}

pub fn bye() -> String {
    json!({ "t": "bye" }).to_string()
}

pub fn superseded() -> String {
    json!({ "t": "superseded" }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Key;

    fn key(event: Option<Event>) -> Option<Key> {
        match event {
            Some(Event::Key(k)) => Key::from_event(k),
            _ => None,
        }
    }

    #[test]
    fn browser_keys_become_the_keys_a_terminal_would_deliver() {
        let cases = [
            (("a", false, false), Key::Char('a')),
            (("A", false, true), Key::Char('A')),
            (("é", false, false), Key::Char('é')),
            ((" ", false, false), Key::Char(' ')),
            (("w", true, false), Key::Ctrl('w')),
            (("W", true, true), Key::Ctrl('w')),
            (("[", true, false), Key::Esc),
            (("i", true, false), Key::Tab),
            (("m", true, false), Key::Enter),
            (("\\", true, false), Key::Ctrl('4')),
            ((" ", true, false), Key::Ctrl(' ')),
            (("Enter", false, false), Key::Enter),
            (("Escape", false, false), Key::Esc),
            (("Tab", false, true), Key::BackTab),
            (("ArrowUp", false, false), Key::Up),
        ];
        for ((name, ctrl, shift), expected) in cases {
            assert_eq!(
                key(key_event(name, ctrl, false, shift)),
                Some(expected),
                "{name}"
            );
        }
        assert_eq!(key_event("F13", false, false, false), None);
        assert_eq!(key_event("Dead", false, false, false), None);
        assert_eq!(key_event("", false, false, false), None);
    }

    #[test]
    fn mouse_messages_keep_position_and_reject_unknown_kinds() {
        let message = serde_json::from_str(
            r#"{"t":"mouse","kind":"drag","button":2,"col":7,"row":9,"shift":true}"#,
        )
        .unwrap();
        let Some(Event::Mouse(mouse)) = input_event(message) else {
            panic!("not a mouse event");
        };
        assert_eq!(mouse.kind, MouseEventKind::Drag(MouseButton::Right));
        assert_eq!((mouse.column, mouse.row), (7, 9));
        assert_eq!(mouse.modifiers, KeyModifiers::SHIFT);
        assert!(mouse_event("hover", 0, 0, 0, KeyModifiers::NONE).is_none());
    }

    #[test]
    fn width_table_covers_wide_and_zero_width_characters_only() {
        let ranges = width_ranges();
        let lookup = |c: char| {
            ranges
                .iter()
                .find(|r| r[0] <= c as u32 && c as u32 <= r[1])
                .map_or(1, |r| r[2])
        };
        assert_eq!(lookup('é'), 1);
        assert_eq!(lookup('界'), 2);
        assert_eq!(lookup('😀'), 2);
        assert_eq!(lookup('\u{0301}'), 0);
        assert!(serde_json::to_string(&ranges).unwrap().len() < 32 * 1024);
    }
}
