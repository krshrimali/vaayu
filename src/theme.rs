//! Minimal colorscheme support: a `Theme` maps the syntax highlight classes to
//! terminal colors. Built-in themes are swappable at runtime with
//! `:colorscheme`. (A fuller theme engine — UI colors, undercurl, transparent
//! backgrounds — is a follow-up; this centralizes the syntax palette first.)

use crossterm::style::Color;

#[derive(Clone, Copy)]
pub struct Theme {
    pub comment: Color,
    pub string: Color,
    pub number: Color,
    pub keyword: Color,
    /// UI colors. Defaults match the previously-hardcoded constants, so the
    /// default scheme looks unchanged; other schemes may override them.
    pub cursorline_bg: Color,
    pub statusline_active_bg: Color,
    pub statusline_inactive_bg: Color,
    /// Background for search matches (`/`, `?`, `n`/`N`, incsearch). Black text
    /// is drawn on it, so every scheme picks a light/bright tint.
    pub search_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // The original hardcoded palette.
        Theme {
            comment: Color::DarkGrey,
            string: Color::Green,
            number: Color::Magenta,
            keyword: Color::Cyan,
            cursorline_bg: Color::AnsiValue(236),
            statusline_active_bg: Color::DarkBlue,
            statusline_inactive_bg: Color::DarkGrey,
            search_bg: Color::DarkYellow,
        }
    }
}

impl Theme {
    pub fn syntax(&self, class: crate::syntax::HlClass) -> Color {
        match class {
            crate::syntax::HlClass::Comment => self.comment,
            crate::syntax::HlClass::String => self.string,
            crate::syntax::HlClass::Number => self.number,
            crate::syntax::HlClass::Keyword => self.keyword,
        }
    }
}

/// The built-in colorscheme names, for `:colorscheme` with no argument.
pub const NAMES: &[&str] = &["default", "mono", "warm", "cool"];

/// Resolves a colorscheme name to its `Theme`, or `None` if unknown.
pub fn builtin(name: &str) -> Option<Theme> {
    let t = match name {
        "default" => Theme::default(),
        // Monochrome: greys only, structure by shade.
        "mono" => Theme {
            comment: Color::AnsiValue(240),
            string: Color::AnsiValue(250),
            number: Color::AnsiValue(250),
            keyword: Color::AnsiValue(255),
            cursorline_bg: Color::AnsiValue(236),
            statusline_active_bg: Color::AnsiValue(240),
            statusline_inactive_bg: Color::AnsiValue(236),
            search_bg: Color::AnsiValue(250),
        },
        // Warm true-color palette.
        "warm" => Theme {
            comment: Color::Rgb { r: 130, g: 110, b: 90 },
            string: Color::Rgb { r: 190, g: 160, b: 90 },
            number: Color::Rgb { r: 210, g: 120, b: 70 },
            keyword: Color::Rgb { r: 200, g: 90, b: 90 },
            cursorline_bg: Color::Rgb { r: 60, g: 45, b: 35 },
            statusline_active_bg: Color::Rgb { r: 120, g: 70, b: 50 },
            statusline_inactive_bg: Color::Rgb { r: 60, g: 45, b: 35 },
            search_bg: Color::Rgb { r: 230, g: 180, b: 90 },
        },
        // Cool true-color palette.
        "cool" => Theme {
            comment: Color::Rgb { r: 90, g: 110, b: 130 },
            string: Color::Rgb { r: 120, g: 190, b: 160 },
            number: Color::Rgb { r: 150, g: 140, b: 210 },
            keyword: Color::Rgb { r: 90, g: 160, b: 210 },
            cursorline_bg: Color::Rgb { r: 35, g: 45, b: 60 },
            statusline_active_bg: Color::Rgb { r: 50, g: 80, b: 120 },
            statusline_inactive_bg: Color::Rgb { r: 35, g: 45, b: 60 },
            search_bg: Color::Rgb { r: 120, g: 200, b: 220 },
        },
        _ => return None,
    };
    Some(t)
}
