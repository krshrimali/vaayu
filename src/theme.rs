//! Colorschemes: a `Theme` maps the syntax highlight classes and every UI
//! surface (statusline, tree, picker, results, diagnostics, git, popups) to
//! terminal colors. Built-in themes are swappable at runtime with
//! `:colorscheme`; `:set transparent` drops a scheme's own background.

use crossterm::style::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub comment: Color,
    pub string: Color,
    pub number: Color,
    pub keyword: Color,
    /// Semantic-token types / classes and functions / methods.
    pub type_: Color,
    pub function: Color,
    /// Editor background and default text. `Reset` (the built-in classics)
    /// leaves the terminal's own colors; a scheme that sets them gets every
    /// default-colored cell painted (unless `transparent` is on, which keeps
    /// the terminal background but still applies `fg`).
    pub bg: Color,
    pub fg: Color,
    /// UI colors. Defaults match the previously-hardcoded constants, so the
    /// default scheme looks unchanged; other schemes may override them.
    pub cursorline_bg: Color,
    pub statusline_active_bg: Color,
    pub statusline_inactive_bg: Color,
    /// Background for search matches (`/`, `?`, `n`/`N`, incsearch). Black text
    /// is drawn on it, so every scheme picks a light/bright tint.
    pub search_bg: Color,
    /// Dim text: split rules, borders, guides, ghost text, blame, `~` rows.
    pub muted: Color,
    /// Headings and focused accents (tree root, focused float border).
    pub accent: Color,
    /// Header/footer bars: picker, results, wildmenu, completion menu, tour.
    /// White text is drawn on it (as on every non-`Reset` row background
    /// below), so light schemes keep it dark enough to read.
    pub bar_bg: Color,
    /// The selected row in lists, picker, results, completion and toasts.
    pub selection_bg: Color,
    /// Secondary panel rows: split separators, preview rules, which-key
    /// items, completion docs, the tabline.
    pub panel_bg: Color,
    /// Which-key popup title row.
    pub title_bg: Color,
    pub float_bg: Color,
    pub winbar_bg: Color,
    pub sticky_bg: Color,
    pub fold_bg: Color,
    pub minimap_fg: Color,
    pub minimap_view_bg: Color,
    pub colorcolumn_bg: Color,
    pub inccommand_bg: Color,
    pub tour_bg: Color,
    /// LSP document-highlight and `,gd` changed-word backgrounds (white text).
    pub doc_highlight_bg: Color,
    pub word_diff_bg: Color,
    pub line_nr: Color,
    pub line_nr_current: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub hint: Color,
    /// Spelling underline, tree marks, NOTE keywords.
    pub special: Color,
    pub success: Color,
    pub git_add: Color,
    pub git_change: Color,
    pub git_delete: Color,
    pub diff_add_bg: Color,
    pub diff_del_bg: Color,
    pub tree_dir: Color,
    /// The file tree's cursor row while the tree has focus (unfocused, it
    /// uses `cursorline_bg`).
    pub tree_cursor_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // The original hardcoded palette.
        Theme {
            comment: Color::DarkGrey,
            string: Color::Green,
            number: Color::Magenta,
            keyword: Color::Cyan,
            type_: Color::Yellow,
            function: Color::Blue,
            bg: Color::Reset,
            fg: Color::Reset,
            cursorline_bg: Color::AnsiValue(236),
            statusline_active_bg: Color::DarkBlue,
            statusline_inactive_bg: Color::DarkGrey,
            search_bg: Color::DarkYellow,
            muted: Color::DarkGrey,
            accent: Color::Cyan,
            bar_bg: Color::DarkBlue,
            selection_bg: Color::DarkCyan,
            panel_bg: Color::DarkGrey,
            title_bg: Color::DarkYellow,
            float_bg: Color::AnsiValue(235),
            winbar_bg: Color::AnsiValue(237),
            sticky_bg: Color::AnsiValue(238),
            fold_bg: Color::AnsiValue(238),
            minimap_fg: Color::Grey,
            minimap_view_bg: Color::AnsiValue(238),
            colorcolumn_bg: Color::AnsiValue(52),
            inccommand_bg: Color::AnsiValue(23),
            tour_bg: Color::AnsiValue(23),
            doc_highlight_bg: Color::DarkBlue,
            word_diff_bg: Color::DarkMagenta,
            line_nr: Color::DarkGrey,
            line_nr_current: Color::Yellow,
            error: Color::Red,
            warning: Color::Yellow,
            info: Color::Blue,
            hint: Color::DarkGrey,
            special: Color::Magenta,
            success: Color::Green,
            git_add: Color::Green,
            git_change: Color::Yellow,
            git_delete: Color::Red,
            diff_add_bg: Color::AnsiValue(22),
            diff_del_bg: Color::AnsiValue(52),
            tree_dir: Color::Blue,
            tree_cursor_bg: Color::AnsiValue(238),
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

    pub fn severity(&self, sev: crate::lsp::Severity) -> Color {
        match sev {
            crate::lsp::Severity::Error => self.error,
            crate::lsp::Severity::Warning => self.warning,
            crate::lsp::Severity::Info => self.info,
            crate::lsp::Severity::Hint => self.hint,
        }
    }

    /// Color for a one-letter git status (`M`, `A`, `?`, `D`, `U`, ...).
    pub fn git(&self, status: char) -> Color {
        match status {
            'M' => self.git_change,
            'A' | '?' => self.git_add,
            'D' | 'U' => self.git_delete,
            _ => self.special,
        }
    }

    /// The `(bg, fg)` the renderer paints default-colored cells with, or
    /// `None` when both are the terminal's own (nothing to paint).
    /// `transparent` keeps the terminal background.
    pub fn base(&self, transparent: bool) -> Option<(Color, Color)> {
        let bg = if transparent { Color::Reset } else { self.bg };
        (bg != Color::Reset || self.fg != Color::Reset).then_some((bg, self.fg))
    }
}

/// The built-in colorscheme names, for `:colorscheme` with no argument and
/// its Tab completion.
pub const NAMES: &[&str] = &[
    "default",
    "mono",
    "warm",
    "cool",
    "gruvbox",
    "gruvbox-light",
    "flexoki",
    "flexoki-light",
    "tokyonight",
    "tokyonight-day",
];

/// `0xRRGGBB` as a true color.
const fn hex(c: u32) -> Color {
    Color::Rgb {
        r: (c >> 16) as u8,
        g: (c >> 8) as u8,
        b: c as u8,
    }
}

/// A full-palette scheme's colors by role; `Palette::theme` spreads them
/// over every token so the true-color schemes stay consistent.
struct Palette {
    bg: u32,
    fg: u32,
    /// Raised surfaces, lightest-contrast first: cursorline / floats,
    /// sticky / fold rows, panels.
    bg1: u32,
    bg2: u32,
    /// The focused tree's cursor row: a step above `bg1`.
    cursor_row: u32,
    panel: u32,
    muted: u32,
    line_nr: u32,
    comment: u32,
    keyword: u32,
    string: u32,
    number: u32,
    function: u32,
    type_: u32,
    red: u32,
    yellow: u32,
    green: u32,
    blue: u32,
    cyan: u32,
    purple: u32,
    /// Saturated backgrounds white text reads on.
    bar: u32,
    selection: u32,
    title: u32,
    doc_highlight: u32,
    word_diff: u32,
    /// Light tint black text reads on.
    search: u32,
    /// Subtle row tints.
    colorcolumn: u32,
    tint: u32,
    diff_add: u32,
    diff_del: u32,
}

impl Palette {
    fn theme(&self) -> Theme {
        Theme {
            comment: hex(self.comment),
            string: hex(self.string),
            number: hex(self.number),
            keyword: hex(self.keyword),
            type_: hex(self.type_),
            function: hex(self.function),
            bg: hex(self.bg),
            fg: hex(self.fg),
            cursorline_bg: hex(self.bg1),
            statusline_active_bg: hex(self.bar),
            statusline_inactive_bg: hex(self.panel),
            search_bg: hex(self.search),
            muted: hex(self.muted),
            accent: hex(self.cyan),
            bar_bg: hex(self.bar),
            selection_bg: hex(self.selection),
            panel_bg: hex(self.panel),
            title_bg: hex(self.title),
            float_bg: hex(self.bg1),
            winbar_bg: hex(self.bg1),
            sticky_bg: hex(self.bg2),
            fold_bg: hex(self.bg2),
            minimap_fg: hex(self.muted),
            minimap_view_bg: hex(self.bg2),
            colorcolumn_bg: hex(self.colorcolumn),
            inccommand_bg: hex(self.tint),
            tour_bg: hex(self.tint),
            doc_highlight_bg: hex(self.doc_highlight),
            word_diff_bg: hex(self.word_diff),
            line_nr: hex(self.line_nr),
            line_nr_current: hex(self.yellow),
            error: hex(self.red),
            warning: hex(self.yellow),
            info: hex(self.blue),
            hint: hex(self.cyan),
            special: hex(self.purple),
            success: hex(self.green),
            git_add: hex(self.green),
            git_change: hex(self.yellow),
            git_delete: hex(self.red),
            diff_add_bg: hex(self.diff_add),
            diff_del_bg: hex(self.diff_del),
            tree_dir: hex(self.blue),
            tree_cursor_bg: hex(self.cursor_row),
        }
    }
}

const GRUVBOX: Palette = Palette {
    bg: 0x282828,
    fg: 0xebdbb2,
    bg1: 0x3c3836,
    bg2: 0x32302f,
    cursor_row: 0x504945,
    panel: 0x504945,
    muted: 0x928374,
    line_nr: 0x7c6f64,
    comment: 0x928374,
    keyword: 0xfb4934,
    string: 0xb8bb26,
    number: 0xd3869b,
    function: 0x8ec07c,
    type_: 0xfabd2f,
    red: 0xfb4934,
    yellow: 0xfabd2f,
    green: 0xb8bb26,
    blue: 0x83a598,
    cyan: 0x8ec07c,
    purple: 0xd3869b,
    bar: 0x076678,
    selection: 0x665c54,
    title: 0xb57614,
    doc_highlight: 0x504945,
    word_diff: 0x8f3f71,
    search: 0xfabd2f,
    colorcolumn: 0x3c2a28,
    tint: 0x2c3a35,
    diff_add: 0x34381b,
    diff_del: 0x402120,
};

const GRUVBOX_LIGHT: Palette = Palette {
    bg: 0xfbf1c7,
    fg: 0x3c3836,
    bg1: 0xebdbb2,
    bg2: 0xf2e5bc,
    cursor_row: 0xd5c4a1,
    panel: 0x7c6f64,
    muted: 0x928374,
    line_nr: 0xa89984,
    comment: 0x928374,
    keyword: 0x9d0006,
    string: 0x79740e,
    number: 0x8f3f71,
    function: 0x427b58,
    type_: 0xb57614,
    red: 0x9d0006,
    yellow: 0xb57614,
    green: 0x79740e,
    blue: 0x076678,
    cyan: 0x427b58,
    purple: 0x8f3f71,
    bar: 0x076678,
    selection: 0x458588,
    title: 0xaf3a03,
    doc_highlight: 0x7c6f64,
    word_diff: 0xb16286,
    search: 0xfabd2f,
    colorcolumn: 0xf9e0c7,
    tint: 0xe3ecc4,
    diff_add: 0xe6eab5,
    diff_del: 0xf9d3c4,
};

const FLEXOKI: Palette = Palette {
    bg: 0x100f0f,
    fg: 0xcecdc3,
    bg1: 0x1c1b1a,
    bg2: 0x282726,
    cursor_row: 0x343331,
    panel: 0x403e3c,
    muted: 0x878580,
    line_nr: 0x575653,
    comment: 0x878580,
    keyword: 0x879a39,
    string: 0x3aa99f,
    number: 0x8b7ec8,
    function: 0xda702c,
    type_: 0xd0a215,
    red: 0xd14d41,
    yellow: 0xd0a215,
    green: 0x879a39,
    blue: 0x4385be,
    cyan: 0x3aa99f,
    purple: 0xce5d97,
    bar: 0x205ea6,
    selection: 0x24837b,
    title: 0xad8301,
    doc_highlight: 0x343331,
    word_diff: 0x5e409d,
    search: 0xd0a215,
    colorcolumn: 0x2a1a18,
    tint: 0x13302d,
    diff_add: 0x1e2a10,
    diff_del: 0x2e1513,
};

const FLEXOKI_LIGHT: Palette = Palette {
    bg: 0xfffcf0,
    fg: 0x100f0f,
    bg1: 0xf2f0e5,
    bg2: 0xe6e4d9,
    cursor_row: 0xdad8ce,
    panel: 0x6f6e69,
    muted: 0x6f6e69,
    line_nr: 0xb7b5ac,
    comment: 0x6f6e69,
    keyword: 0x66800b,
    string: 0x24837b,
    number: 0x5e409d,
    function: 0xbc5215,
    type_: 0xad8301,
    red: 0xaf3029,
    yellow: 0xad8301,
    green: 0x66800b,
    blue: 0x205ea6,
    cyan: 0x24837b,
    purple: 0xa02f6f,
    bar: 0x205ea6,
    selection: 0x24837b,
    title: 0xbc5215,
    doc_highlight: 0x6f6e69,
    word_diff: 0xa02f6f,
    search: 0xeccb60,
    colorcolumn: 0xf7e2d8,
    tint: 0xddf1e4,
    diff_add: 0xedeecf,
    diff_del: 0xffe1d5,
};

const TOKYONIGHT: Palette = Palette {
    bg: 0x1a1b26,
    fg: 0xc0caf5,
    bg1: 0x292e42,
    bg2: 0x1f2335,
    cursor_row: 0x3b4261,
    panel: 0x3b4261,
    muted: 0x565f89,
    line_nr: 0x3b4261,
    comment: 0x565f89,
    keyword: 0xbb9af7,
    string: 0x9ece6a,
    number: 0xff9e64,
    function: 0x7aa2f7,
    type_: 0x2ac3de,
    red: 0xf7768e,
    yellow: 0xe0af68,
    green: 0x9ece6a,
    blue: 0x7aa2f7,
    cyan: 0x7dcfff,
    purple: 0xbb9af7,
    bar: 0x3d59a1,
    selection: 0x364a82,
    title: 0x9d7cd8,
    doc_highlight: 0x3b4261,
    word_diff: 0x5a3e7a,
    search: 0xe0af68,
    colorcolumn: 0x2d2033,
    tint: 0x1f3a3f,
    diff_add: 0x20303b,
    diff_del: 0x37222c,
};

const TOKYONIGHT_DAY: Palette = Palette {
    bg: 0xe1e2e7,
    fg: 0x3760bf,
    bg1: 0xc4c8da,
    bg2: 0xd0d5e3,
    cursor_row: 0xb4b9d0,
    panel: 0x8990b3,
    muted: 0x848cb5,
    line_nr: 0xa8aecb,
    comment: 0x848cb5,
    keyword: 0x9854f1,
    string: 0x587539,
    number: 0xb15c00,
    function: 0x2e7de9,
    type_: 0x188092,
    red: 0xf52a65,
    yellow: 0x8c6c3e,
    green: 0x587539,
    blue: 0x2e7de9,
    cyan: 0x007197,
    purple: 0x9854f1,
    bar: 0x2e7de9,
    selection: 0x4a6fd0,
    title: 0xb15c00,
    doc_highlight: 0x6172b0,
    word_diff: 0x9854f1,
    search: 0xe9c46a,
    colorcolumn: 0xe9d0d6,
    tint: 0xc8e0dc,
    diff_add: 0xc8dcc0,
    diff_del: 0xecc8cf,
};

/// Resolves a colorscheme name to its `Theme`, or `None` if unknown.
pub fn builtin(name: &str) -> Option<Theme> {
    let t = match name {
        "default" => Theme::default(),
        // Monochrome: greys only, structure by shade.
        "mono" => Theme {
            accent: Color::AnsiValue(255),
            muted: Color::AnsiValue(240),
            float_bg: Color::AnsiValue(236),
            comment: Color::AnsiValue(240),
            string: Color::AnsiValue(250),
            number: Color::AnsiValue(250),
            keyword: Color::AnsiValue(255),
            cursorline_bg: Color::AnsiValue(236),
            statusline_active_bg: Color::AnsiValue(240),
            statusline_inactive_bg: Color::AnsiValue(236),
            search_bg: Color::AnsiValue(250),
            ..Theme::default()
        },
        // Warm true-color palette.
        "warm" => Theme {
            accent: hex(0xc85a5a),
            muted: hex(0x826e5a),
            float_bg: hex(0x3c2d23),
            comment: hex(0x826e5a),
            string: hex(0xbea05a),
            number: hex(0xd27846),
            keyword: hex(0xc85a5a),
            cursorline_bg: hex(0x3c2d23),
            statusline_active_bg: hex(0x784632),
            statusline_inactive_bg: hex(0x3c2d23),
            search_bg: hex(0xe6b45a),
            ..Theme::default()
        },
        // Cool true-color palette.
        "cool" => Theme {
            accent: hex(0x5aa0d2),
            muted: hex(0x5a6e82),
            float_bg: hex(0x232d3c),
            comment: hex(0x5a6e82),
            string: hex(0x78bea0),
            number: hex(0x968cd2),
            keyword: hex(0x5aa0d2),
            cursorline_bg: hex(0x232d3c),
            statusline_active_bg: hex(0x325078),
            statusline_inactive_bg: hex(0x232d3c),
            search_bg: hex(0x78c8dc),
            ..Theme::default()
        },
        "gruvbox" => GRUVBOX.theme(),
        "gruvbox-light" => GRUVBOX_LIGHT.theme(),
        "flexoki" => FLEXOKI.theme(),
        "flexoki-light" => FLEXOKI_LIGHT.theme(),
        "tokyonight" => TOKYONIGHT.theme(),
        "tokyonight-day" => TOKYONIGHT_DAY.theme(),
        _ => return None,
    };
    Some(t)
}

/// Built-in scheme names starting with `prefix`, for `:colorscheme <Tab>`.
pub fn complete(prefix: &str) -> Vec<&'static str> {
    NAMES
        .iter()
        .copied()
        .filter(|n| n.starts_with(prefix))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance (0..=1) of a true color, for contrast checks.
    fn luminance(c: Color) -> f64 {
        let Color::Rgb { r, g, b } = c else {
            panic!("expected a true color, got {c:?}");
        };
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn every_listed_name_resolves() {
        for name in NAMES {
            assert!(builtin(name).is_some(), "{name} should be a builtin");
        }
        assert!(builtin("nope").is_none());
    }

    #[test]
    fn classic_schemes_keep_terminal_base_and_existing_highlights() {
        // These schemes leave the terminal base alone and keep their existing
        // shared highlights; floating UI roles follow each scheme's palette.
        for name in ["default", "mono", "warm", "cool"] {
            let t = builtin(name).unwrap();
            assert_eq!(t.bg, Color::Reset, "{name}");
            assert_eq!(t.fg, Color::Reset, "{name}");
            assert_eq!(t.base(false), None, "{name} paints nothing extra");
            assert_eq!(t.bar_bg, Color::DarkBlue, "{name}");
            assert_eq!(t.selection_bg, Color::DarkCyan, "{name}");
            assert_eq!(t.diff_add_bg, Color::AnsiValue(22), "{name}");
            assert_eq!(t.colorcolumn_bg, Color::AnsiValue(52), "{name}");
            assert_eq!(t.error, Color::Red, "{name}");
        }
        let d = Theme::default();
        assert_eq!(d.keyword, Color::Cyan);
        assert_eq!(d.type_, Color::Yellow);
        assert_eq!(d.function, Color::Blue);
        assert_eq!(builtin("warm").unwrap().keyword, hex(0xc85a5a));
    }

    #[test]
    fn full_palette_schemes_paint_a_base_and_honor_transparent() {
        for name in [
            "gruvbox",
            "gruvbox-light",
            "flexoki",
            "flexoki-light",
            "tokyonight",
            "tokyonight-day",
        ] {
            let t = builtin(name).unwrap();
            assert_eq!(t.base(false), Some((t.bg, t.fg)), "{name}");
            assert_eq!(t.base(true), Some((Color::Reset, t.fg)), "{name}");
        }
    }

    #[test]
    fn full_palette_schemes_are_readable() {
        let white = hex(0xffffff);
        let black = hex(0x000000);
        for name in [
            "gruvbox",
            "gruvbox-light",
            "flexoki",
            "flexoki-light",
            "tokyonight",
            "tokyonight-day",
        ] {
            let t = builtin(name).unwrap();
            for (what, c) in [
                ("keyword", t.keyword),
                ("string", t.string),
                ("function", t.function),
                ("fg", t.fg),
            ] {
                assert!(contrast(c, t.bg) >= 3.0, "{name}: {what} on bg");
            }
            assert!(contrast(t.fg, t.cursorline_bg) >= 3.0, "{name}: cursorline");
            // White text is drawn on bars and selections, black on search.
            for (what, c) in [
                ("bar", t.bar_bg),
                ("selection", t.selection_bg),
                ("statusline", t.statusline_active_bg),
            ] {
                assert!(contrast(white, c) >= 3.0, "{name}: white on {what}");
            }
            assert!(contrast(black, t.search_bg) >= 4.5, "{name}: search");
        }
    }

    #[test]
    fn complete_filters_by_prefix() {
        assert_eq!(complete("gru"), vec!["gruvbox", "gruvbox-light"]);
        assert_eq!(complete("tokyonight-"), vec!["tokyonight-day"]);
        assert_eq!(complete("").len(), NAMES.len());
        assert!(complete("zz").is_empty());
    }

    #[test]
    fn git_status_letters_map_to_git_tokens() {
        let t = Theme::default();
        assert_eq!(t.git('M'), Color::Yellow);
        assert_eq!(t.git('A'), Color::Green);
        assert_eq!(t.git('?'), Color::Green);
        assert_eq!(t.git('D'), Color::Red);
        assert_eq!(t.git('R'), Color::Magenta);
    }
}
