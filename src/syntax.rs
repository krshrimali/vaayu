use std::rc::Rc;
use tree_sitter::{InputEdit, Language, Parser, Point, Tree};

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
    source: Rc<str>,
    spans: Vec<(usize, usize, HlClass)>,
    /// Longest (end - start) among `spans`, as of the last reparse. Lets
    /// `spans_in` binary-search to the first span that could *possibly*
    /// reach into a queried range instead of linear-scanning every span in
    /// the file -- the dominant per-frame cost before this fix, since a
    /// visible row's highlights were being looked up by scanning the whole
    /// file's span list, once per row, every single keystroke.
    max_span_len: usize,
    /// True when `tree` has moved on since `spans` was last rebuilt from
    /// it. Rebuilding is a full re-walk of the tree (see `rebuild_spans`),
    /// not incremental like parsing itself -- doing it on every keystroke
    /// during a fast typing burst measured at ~8ms/keystroke on a modest
    /// file, entirely dominating per-frame cost that was otherwise under
    /// 1ms. Throttled instead of eliminated: the buffer's edit_seq still
    /// bumps every keystroke (LSP sync and undo/dirty-tracking need that),
    /// and a render against momentarily-stale spans is safe -- the
    /// defensive UTF-8 boundary clamp in render.rs's syntax_spans_for_line
    /// exists for exactly this gap -- so a few tens of milliseconds of
    /// highlighting lag during rapid typing is the trade, not a
    /// correctness or crash risk.
    spans_dirty: bool,
    last_span_rebuild: Option<std::time::Instant>,
}

/// Minimum gap between full span rebuilds. Small enough that highlighting
/// visibly keeps up during normal typing speed and catches up within one
/// frame once typing pauses; large enough to collapse a fast burst (paste,
/// holding a key, `.` repeat) into one rebuild instead of one per
/// keystroke.
const SPAN_REBUILD_THROTTLE: std::time::Duration = std::time::Duration::from_millis(30);

impl Syntax {
    pub fn new(lang: Lang) -> Option<Syntax> {
        let mut parser = Parser::new();
        parser.set_language(&ts_language(lang)).ok()?;
        Some(Syntax {
            lang,
            parser,
            tree: None,
            source: Rc::from(""),
            spans: Vec::new(),
            max_span_len: 0,
            spans_dirty: false,
            last_span_rebuild: None,
        })
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Reparses for `new_text`. When a previous tree exists, computes the
    /// changed byte range against the previous source (a cheap prefix/
    /// suffix byte comparison, contained entirely here -- no edit-tracking
    /// threaded through the wide buffer-mutation call surface elsewhere)
    /// and feeds it to tree-sitter as an `InputEdit` before reparsing, so
    /// parsing only redoes the affected subtree instead of the whole file.
    /// The (expensive) span rebuild from the new tree is throttled -- see
    /// `spans_dirty`'s docs -- so this call itself stays fast even when
    /// called on every keystroke.
    pub fn reparse(&mut self, new_text: Rc<str>) {
        let t0 = std::time::Instant::now();
        if let Some(tree) = &mut self.tree {
            if let Some(edit) = compute_edit(&self.source, &new_text) {
                tree.edit(&edit);
            }
        }
        crate::profile::note("syn_compute_edit", t0.elapsed());
        let t1 = std::time::Instant::now();
        self.tree = self.parser.parse(new_text.as_bytes(), self.tree.as_ref());
        crate::profile::note("syn_parse", t1.elapsed());
        self.source = new_text;
        self.spans_dirty = true;
        let t2 = std::time::Instant::now();
        self.maybe_rebuild_spans();
        crate::profile::note("syn_maybe_rebuild", t2.elapsed());
    }

    /// Catches up a deferred rebuild once its throttle window has passed,
    /// even with no new edit to trigger `reparse`. Returns whether it
    /// actually rebuilt anything, so a caller on the idle path (no key, no
    /// LSP event -- nothing else that would trigger a redraw on its own)
    /// knows to redraw. Without this, highlighting could stay stale/blank
    /// indefinitely once the user stops typing: edit_seq stops changing
    /// then, so `reparse` never gets called again to finish the deferred
    /// work, and nothing else would wake the render loop to show the
    /// result even if the work does happen to get finished.
    pub fn catch_up(&mut self) -> bool {
        self.maybe_rebuild_spans()
    }

    /// Whether a rebuild is pending and its throttle window has elapsed --
    /// cheap enough to poll every idle tick.
    pub fn rebuild_due(&self) -> bool {
        self.spans_dirty && self.last_span_rebuild.map(|t| t.elapsed() >= SPAN_REBUILD_THROTTLE).unwrap_or(true)
    }

    fn maybe_rebuild_spans(&mut self) -> bool {
        if !self.spans_dirty {
            return false;
        }
        let due = self.last_span_rebuild.map(|t| t.elapsed() >= SPAN_REBUILD_THROTTLE).unwrap_or(true);
        if due {
            self.rebuild_spans();
        }
        due
    }

    fn rebuild_spans(&mut self) {
        self.spans.clear();
        if let Some(tree) = self.tree.clone() {
            let kws = keywords(self.lang);
            let mut cursor = tree.walk();
            walk(&mut cursor, self.source.as_bytes(), kws, &mut self.spans);
        }
        self.spans.sort_by_key(|s| s.0);
        self.max_span_len = self.spans.iter().map(|(s, e, _)| e.saturating_sub(*s)).max().unwrap_or(0);
        self.spans_dirty = false;
        self.last_span_rebuild = Some(std::time::Instant::now());
    }

    /// Highlight spans (byte ranges into the last-parsed source) intersecting
    /// [start_byte, end_byte).
    pub fn spans_in(&self, start_byte: usize, end_byte: usize) -> impl Iterator<Item = (usize, usize, HlClass)> + '_ {
        // `spans` is sorted by start. A span can only overlap [start_byte,
        // end_byte) if its own start is >= start_byte - max_span_len (any
        // earlier and even the longest span in the file couldn't reach this
        // far) -- binary search straight to that point instead of scanning
        // every span in the file for every visible row, every frame.
        let lo = self.spans.partition_point(|(s, _, _)| *s < start_byte.saturating_sub(self.max_span_len));
        self.spans[lo..]
            .iter()
            .take_while(move |(s, _, _)| *s < end_byte)
            .filter(move |(s, e, _)| *s < end_byte && *e > start_byte)
            .map(move |(s, e, c)| ((*s).max(start_byte), (*e).min(end_byte), *c))
    }
}

/// Finds the smallest byte range that differs between `old` and `new` via
/// matching prefix/suffix byte runs, and turns it into the `InputEdit`
/// tree-sitter needs to reuse unaffected subtrees. `None` means identical
/// text (nothing to edit).
fn compute_edit(old: &str, new: &str) -> Option<InputEdit> {
    let (ob, nb) = (old.as_bytes(), new.as_bytes());
    let min_len = ob.len().min(nb.len());
    let mut prefix = 0;
    while prefix < min_len && ob[prefix] == nb[prefix] {
        prefix += 1;
    }
    let max_suffix = min_len - prefix;
    let mut suffix = 0;
    while suffix < max_suffix && ob[ob.len() - 1 - suffix] == nb[nb.len() - 1 - suffix] {
        suffix += 1;
    }
    if prefix == ob.len() && prefix == nb.len() {
        return None; // identical text, nothing changed
    }
    let old_end_byte = ob.len() - suffix;
    let new_end_byte = nb.len() - suffix;
    Some(InputEdit {
        start_byte: prefix,
        old_end_byte,
        new_end_byte,
        start_position: point_at(ob, prefix),
        old_end_position: point_at(ob, old_end_byte),
        new_end_position: point_at(nb, new_end_byte),
    })
}

fn point_at(bytes: &[u8], offset: usize) -> Point {
    let mut row = 0;
    let mut line_start = 0;
    for (i, &b) in bytes[..offset].iter().enumerate() {
        if b == b'\n' {
            row += 1;
            line_start = i + 1;
        }
    }
    Point { row, column: offset - line_start }
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
