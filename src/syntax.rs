use tree_sitter::{Language, Parser, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlClass {
    Comment,
    String,
    Number,
    Keyword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    Go,
    C,
    Bash,
    Json,
    Toml,
    Yaml,
    Lua,
}

pub fn lang_for_extension(ext: &str) -> Option<Lang> {
    Some(match ext {
        "rs" => Lang::Rust,
        "py" | "pyi" => Lang::Python,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => Lang::JavaScript,
        "go" => Lang::Go,
        "c" | "h" => Lang::C,
        "sh" | "bash" | "zsh" => Lang::Bash,
        "json" | "jsonc" => Lang::Json,
        "toml" => Lang::Toml,
        "yaml" | "yml" => Lang::Yaml,
        "lua" => Lang::Lua,
        _ => return None,
    })
}

fn ts_language(lang: Lang) -> Language {
    match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::C => tree_sitter_c::LANGUAGE.into(),
        Lang::Bash => tree_sitter_bash::LANGUAGE.into(),
        Lang::Json => tree_sitter_json::LANGUAGE.into(),
        Lang::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Lang::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        Lang::Lua => tree_sitter_lua::LANGUAGE.into(),
    }
}

/// Per-language literal keywords, matched against anonymous (token) node
/// text -- robust across grammar versions since it doesn't depend on exact
/// named node-type identifiers, only on the literal source spelling.
fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "for", "while", "loop", "if", "else",
            "match", "return", "use", "mod", "self", "Self", "async", "await", "move", "ref", "const", "static",
            "where", "unsafe", "as", "in", "break", "continue", "dyn", "type", "crate", "super", "extern", "true",
            "false", "yield",
        ],
        Lang::Python => &[
            "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as", "with", "try",
            "except", "finally", "raise", "lambda", "yield", "global", "nonlocal", "pass", "break", "continue",
            "and", "or", "not", "in", "is", "None", "True", "False", "async", "await", "del", "assert",
        ],
        Lang::JavaScript => &[
            "function", "const", "let", "var", "if", "else", "for", "while", "return", "import", "export", "from",
            "as", "class", "extends", "new", "this", "try", "catch", "finally", "throw", "typeof", "instanceof",
            "in", "of", "async", "await", "yield", "true", "false", "null", "undefined", "switch", "case", "default",
            "break", "continue", "do", "delete", "void", "interface", "type", "enum", "implements", "public",
            "private", "protected", "readonly", "static",
        ],
        Lang::Go => &[
            "func", "package", "import", "var", "const", "type", "struct", "interface", "map", "chan", "if", "else",
            "for", "range", "return", "switch", "case", "default", "break", "continue", "go", "defer", "select",
            "fallthrough", "goto", "true", "false", "nil",
        ],
        Lang::C => &[
            "int", "char", "float", "double", "void", "long", "short", "unsigned", "signed", "struct", "union",
            "enum", "typedef", "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue",
            "return", "goto", "static", "const", "extern", "sizeof", "volatile", "inline", "NULL",
        ],
        Lang::Bash => &[
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "function",
            "in", "return", "local", "export", "readonly", "break", "continue", "select",
        ],
        Lang::Json | Lang::Toml | Lang::Yaml => &["true", "false", "null"],
        Lang::Lua => &[
            "function", "local", "end", "if", "then", "else", "elseif", "for", "while", "do", "repeat", "until",
            "return", "break", "nil", "true", "false", "and", "or", "not", "in",
        ],
    }
}

pub struct Syntax {
    lang: Lang,
    parser: Parser,
    tree: Option<Tree>,
    source: String,
    spans: Vec<(usize, usize, HlClass)>,
}

impl Syntax {
    pub fn new(lang: Lang) -> Option<Syntax> {
        let mut parser = Parser::new();
        parser.set_language(&ts_language(lang)).ok()?;
        Some(Syntax { lang, parser, tree: None, source: String::new(), spans: Vec::new() })
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    pub fn reparse(&mut self, text: &str) {
        self.tree = self.parser.parse(text, self.tree.as_ref());
        self.source = text.to_string();
        self.spans.clear();
        if let Some(tree) = self.tree.clone() {
            let kws = keywords(self.lang);
            let mut cursor = tree.walk();
            walk(&mut cursor, self.source.as_bytes(), kws, &mut self.spans);
        }
        self.spans.sort_by_key(|s| s.0);
    }

    /// Highlight spans (byte ranges into the last-parsed source) intersecting
    /// [start_byte, end_byte).
    pub fn spans_in(&self, start_byte: usize, end_byte: usize) -> impl Iterator<Item = (usize, usize, HlClass)> + '_ {
        self.spans
            .iter()
            .filter(move |(s, e, _)| *s < end_byte && *e > start_byte)
            .map(move |(s, e, c)| ((*s).max(start_byte), (*e).min(end_byte), *c))
    }
}

fn walk(cursor: &mut tree_sitter::TreeCursor, source: &[u8], kws: &[&str], out: &mut Vec<(usize, usize, HlClass)>) {
    loop {
        let node = cursor.node();
        let kind = node.kind();
        let class = if kind.contains("comment") {
            Some(HlClass::Comment)
        } else if kind.contains("string") || kind.contains("char_literal") {
            Some(HlClass::String)
        } else if kind.contains("number") || kind.contains("integer") || kind.contains("float") {
            Some(HlClass::Number)
        } else if !node.is_named() {
            node.utf8_text(source).ok().filter(|t| kws.contains(t)).map(|_| HlClass::Keyword)
        } else {
            None
        };

        if let Some(class) = class {
            out.push((node.start_byte(), node.end_byte(), class));
        } else if cursor.goto_first_child() {
            walk(cursor, source, kws, out);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}
