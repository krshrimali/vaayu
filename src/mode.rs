#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Results,
    Normal,
    Insert,
    Visual(VisualKind),
    Command(CommandKind),
    Picker,
    MarkdownPreview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualKind {
    Block,
    Char,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Ex,
    SearchFwd,
    SearchBack,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Results => "RESULTS",
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual(VisualKind::Char) => "VISUAL",
            Mode::Visual(VisualKind::Line) => "V-LINE",
            Mode::Visual(VisualKind::Block) => "V-BLOCK",
            Mode::Command(CommandKind::Ex) => "COMMAND",
            Mode::Command(CommandKind::SearchFwd) => "SEARCH",
            Mode::Command(CommandKind::SearchBack) => "SEARCH",
            Mode::Picker => "FILES",
            Mode::MarkdownPreview => "PREVIEW",
        }
    }
}
