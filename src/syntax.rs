use std::rc::Rc;
use tree_sitter::{InputEdit, Language, Node, Parser, Point, Tree};

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
    TypeScript,
    Tsx,
    Go,
    C,
    Bash,
    Json,
    Toml,
    Yaml,
    Lua,
    Vim,
    Css,
    Html,
    Solidity,
}

pub fn lang_for_extension(ext: &str) -> Option<Lang> {
    Some(match ext {
        "rs" => Lang::Rust,
        "py" | "pyi" => Lang::Python,
        "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
        "ts" => Lang::TypeScript,
        "tsx" => Lang::Tsx,
        "go" => Lang::Go,
        "c" | "h" => Lang::C,
        "sh" | "bash" | "zsh" => Lang::Bash,
        "json" | "jsonc" => Lang::Json,
        "toml" => Lang::Toml,
        "yaml" | "yml" => Lang::Yaml,
        "lua" => Lang::Lua,
        "vim" => Lang::Vim,
        "css" => Lang::Css,
        "html" | "htm" => Lang::Html,
        "sol" => Lang::Solidity,
        _ => return None,
    })
}

fn ts_language(lang: Lang) -> Language {
    match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::C => tree_sitter_c::LANGUAGE.into(),
        Lang::Bash => tree_sitter_bash::LANGUAGE.into(),
        Lang::Json => tree_sitter_json::LANGUAGE.into(),
        Lang::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Lang::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        Lang::Lua => tree_sitter_lua::LANGUAGE.into(),
        // tree_sitter_vim's binding predates the `LanguageFn` convention
        // the other grammars here use -- `language()` already returns a
        // plain `Language`, so no `.into()` is needed (or possible).
        Lang::Vim => tree_sitter_vim::language(),
        Lang::Css => tree_sitter_css::LANGUAGE.into(),
        Lang::Html => tree_sitter_html::LANGUAGE.into(),
        Lang::Solidity => tree_sitter_solidity::LANGUAGE.into(),
    }
}

/// Per-language literal keywords, matched against anonymous (token) node
/// text -- robust across grammar versions since it doesn't depend on exact
/// named node-type identifiers, only on the literal source spelling.
fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "for", "while", "loop",
            "if", "else", "match", "return", "use", "mod", "self", "Self", "async", "await",
            "move", "ref", "const", "static", "where", "unsafe", "as", "in", "break", "continue",
            "dyn", "type", "crate", "super", "extern", "true", "false", "yield",
        ],
        Lang::Python => &[
            "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as",
            "with", "try", "except", "finally", "raise", "lambda", "yield", "global", "nonlocal",
            "pass", "break", "continue", "and", "or", "not", "in", "is", "None", "True", "False",
            "async", "await", "del", "assert",
        ],
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &[
            "interface",
            "type",
            "namespace",
            "declare",
            "readonly",
            "public",
            "private",
            "implements",
            "function",
            "const",
            "let",
            "var",
            "if",
            "else",
            "for",
            "while",
            "return",
            "import",
            "export",
            "from",
            "as",
            "class",
            "extends",
            "new",
            "this",
            "try",
            "catch",
            "finally",
            "throw",
            "typeof",
            "instanceof",
            "in",
            "of",
            "async",
            "await",
            "yield",
            "true",
            "false",
            "null",
            "undefined",
            "switch",
            "case",
            "default",
            "break",
            "continue",
            "do",
            "delete",
            "void",
            "interface",
            "type",
            "enum",
            "implements",
            "public",
            "private",
            "protected",
            "readonly",
            "static",
        ],
        Lang::Go => &[
            "func",
            "package",
            "import",
            "var",
            "const",
            "type",
            "struct",
            "interface",
            "map",
            "chan",
            "if",
            "else",
            "for",
            "range",
            "return",
            "switch",
            "case",
            "default",
            "break",
            "continue",
            "go",
            "defer",
            "select",
            "fallthrough",
            "goto",
            "true",
            "false",
            "nil",
        ],
        Lang::C => &[
            "int", "char", "float", "double", "void", "long", "short", "unsigned", "signed",
            "struct", "union", "enum", "typedef", "if", "else", "for", "while", "do", "switch",
            "case", "default", "break", "continue", "return", "goto", "static", "const", "extern",
            "sizeof", "volatile", "inline", "NULL",
        ],
        Lang::Bash => &[
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case",
            "esac", "function", "in", "return", "local", "export", "readonly", "break", "continue",
            "select",
        ],
        Lang::Json | Lang::Toml | Lang::Yaml => &["true", "false", "null"],
        Lang::Lua => &[
            "function", "local", "end", "if", "then", "else", "elseif", "for", "while", "do",
            "repeat", "until", "return", "break", "nil", "true", "false", "and", "or", "not", "in",
        ],
        Lang::Vim => &[
            "function",
            "endfunction",
            "if",
            "endif",
            "else",
            "elseif",
            "while",
            "endwhile",
            "for",
            "endfor",
            "let",
            "call",
            "return",
            "break",
            "continue",
            "try",
            "endtry",
            "catch",
            "finally",
            "throw",
            "autocmd",
            "augroup",
            "command",
            "set",
            "unlet",
            "echo",
            "execute",
            "normal",
        ],
        // CSS has no real keyword vocabulary (properties/values are
        // identifiers, not reserved words) -- just the handful of at-rule
        // names and `!important`, which only highlight when they appear
        // as their own anonymous token, the same as every other
        // language's list here.
        Lang::Css => &[
            "important",
            "media",
            "import",
            "keyframes",
            "supports",
            "charset",
            "font-face",
            "from",
            "to",
        ],
        // HTML likewise has no keyword vocabulary -- tag/attribute names
        // are identifiers, not reserved words -- so comment highlighting
        // (already generic via `classify`) is the only span this
        // grammar contributes; an empty list here is deliberate, not a
        // placeholder for one that got skipped.
        Lang::Html => &[],
        Lang::Solidity => &[
            "pragma",
            "solidity",
            "contract",
            "interface",
            "library",
            "function",
            "modifier",
            "event",
            "struct",
            "enum",
            "mapping",
            "public",
            "private",
            "internal",
            "external",
            "view",
            "pure",
            "payable",
            "memory",
            "storage",
            "calldata",
            "returns",
            "return",
            "if",
            "else",
            "for",
            "while",
            "do",
            "break",
            "continue",
            "emit",
            "require",
            "revert",
            "assert",
            "import",
            "is",
            "using",
            "override",
            "virtual",
            "constructor",
            "true",
            "false",
            "address",
            "uint",
            "int",
            "bool",
            "string",
            "bytes",
        ],
    }
}

pub struct Syntax {
    lang: Lang,
    parser: Parser,
    tree: Option<Tree>,
    source: Rc<str>,
    spans: Vec<(usize, usize, HlClass)>,
    /// Longest (end - start) among `spans`, as of the last update. Lets
    /// `spans_in` binary-search to the first span that could *possibly*
    /// reach into a queried range instead of linear-scanning every span in
    /// the file.
    max_span_len: usize,
    /// True when `compute_incremental_spans` bailed out (the affected
    /// region ballooned past its cap -- typically a run of syntactically
    /// invalid text, e.g. mid-edit with an unmatched bracket, that makes
    /// tree-sitter's error recovery reparent a large swath of the tree) and
    /// the resulting full rebuild was itself deferred by
    /// `FULL_REBUILD_THROTTLE` rather than paid for immediately. This is
    /// *not* a general throttle: a normal incremental update always runs
    /// synchronously and is never deferred. It only guards the rare full
    /// re-walk-from-root fallback, so a burst of edits that keeps landing
    /// in invalid-syntax territory can't force that expensive path on
    /// every single keystroke.
    full_rebuild_pending: bool,
    last_full_rebuild: Option<std::time::Instant>,
}

/// Above this file size, the "is the affected region most of the file
/// anyway" bailout in `compute_incremental_spans` kicks in -- below it a
/// full rebuild is already cheap enough that the extra bookkeeping isn't
/// worth it.
const INCREMENTAL_MIN_FILE_LEN: usize = 4096;

/// Minimum gap between full-tree rebuilds triggered by the incremental
/// bailout. Bounds worst-case cost during a run of invalid-syntax edits
/// without adding any latency to the normal (incremental) path.
const FULL_REBUILD_THROTTLE: std::time::Duration = std::time::Duration::from_millis(50);

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
            full_rebuild_pending: false,
            last_full_rebuild: None,
        })
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Text object: the byte range of the nearest enclosing node whose kind is
    /// in `kinds` (e.g. a function or class). With `inner`, returns the body
    /// block's content (the span of its named children, i.e. between the
    /// braces) instead of the whole node.
    pub fn object_range(&self, byte: usize, kinds: &[&str], inner: bool) -> Option<(usize, usize)> {
        let tree = self.tree.as_ref()?;
        let root = tree.root_node();
        let mut node = root.descendant_for_byte_range(byte, byte)?;
        loop {
            if kinds.contains(&node.kind()) {
                break;
            }
            node = node.parent()?;
        }
        if !inner {
            return Some((node.start_byte(), node.end_byte()));
        }
        const BLOCK_KINDS: &[&str] = &[
            "block",
            "declaration_list",
            "field_declaration_list",
            "statement_block",
            "class_body",
            "enum_variant_list",
            "body",
            "match_block",
        ];
        let mut cur = node.walk();
        let body = node.children(&mut cur).find(|c| BLOCK_KINDS.contains(&c.kind()))?;
        let mut bc = body.walk();
        let named: Vec<_> = body.named_children(&mut bc).collect();
        if let (Some(first), Some(last)) = (named.first(), named.last()) {
            Some((first.start_byte(), last.end_byte()))
        } else {
            // Empty body: the span just inside the delimiters.
            let s = (body.start_byte() + 1).min(body.end_byte());
            let e = body.end_byte().saturating_sub(1).max(s);
            Some((s, e))
        }
    }

    /// Incremental selection: the byte range of the smallest syntax node that
    /// strictly contains the byte range `[lo, hi)` -- i.e. the next node to
    /// expand a selection to. Climbs to a parent when the current selection is
    /// already exactly a node. Returns None if there is no parsed tree.
    pub fn expand_range(&self, lo: usize, hi: usize) -> Option<(usize, usize)> {
        let tree = self.tree.as_ref()?;
        let root = tree.root_node();
        let mut node = root.descendant_for_byte_range(lo, hi)?;
        // `descendant_for_byte_range` already contains [lo, hi); climb while the
        // node is exactly the selection so expansion always grows.
        while node.start_byte() >= lo && node.end_byte() <= hi {
            match node.parent() {
                Some(p) => node = p,
                None => break,
            }
        }
        Some((node.start_byte(), node.end_byte()))
    }

    /// Reparses for `new_text` and brings `spans` fully up to date,
    /// synchronously, every call -- no deferred/throttled work left over
    /// for a caller to catch up on later.
    ///
    /// When a previous tree exists, computes the changed byte range against
    /// the previous source (a cheap prefix/suffix byte comparison) and
    /// feeds it to tree-sitter as an `InputEdit`, so parsing only redoes
    /// the affected subtree instead of the whole file -- this part was
    /// already incremental. What wasn't: rebuilding the highlight span
    /// list from the tree used to re-walk the entire tree from its root on
    /// every call, dominating per-keystroke cost during insert-mode typing
    /// (~8ms on a modest file) even though only a small part of the tree
    /// actually changed. `compute_incremental_spans` fixes that: it uses
    /// `Tree::changed_ranges` to find what tree-sitter itself says changed,
    /// re-walks only the smallest enclosing node covering that, and
    /// shifts/reuses every span outside it instead of recomputing them.
    pub fn reparse(&mut self, new_text: Rc<str>) {
        let t0 = std::time::Instant::now();
        let edit = compute_edit(&self.source, &new_text);
        crate::profile::note("syn_compute_edit", t0.elapsed());

        let Some(edit) = edit else {
            // Identical text -- nothing changed.
            self.source = new_text;
            return;
        };

        if let Some(tree) = &mut self.tree {
            tree.edit(&edit);
        }

        let t1 = std::time::Instant::now();
        let new_tree = self.parser.parse(new_text.as_bytes(), self.tree.as_ref());
        crate::profile::note("syn_parse", t1.elapsed());

        let t2 = std::time::Instant::now();
        let incremental = match (&self.tree, &new_tree) {
            (Some(old), Some(new)) if !self.full_rebuild_pending => {
                compute_incremental_spans(&self.spans, self.lang, old, new, edit, &new_text)
            }
            _ => None,
        };
        self.tree = new_tree;
        self.source = new_text;
        match incremental {
            Some(spans) => {
                self.max_span_len = spans
                    .iter()
                    .map(|(s, e, _)| e.saturating_sub(*s))
                    .max()
                    .unwrap_or(0);
                self.spans = spans;
                self.full_rebuild_pending = false;
            }
            None => {
                let due = self
                    .last_full_rebuild
                    .map(|t| t.elapsed() >= FULL_REBUILD_THROTTLE)
                    .unwrap_or(true);
                if due {
                    self.rebuild_spans_full();
                } else {
                    // Spans stay stale until `catch_up` or the next call
                    // here finds the throttle window elapsed -- see
                    // `full_rebuild_pending`'s docs.
                    self.full_rebuild_pending = true;
                }
            }
        }
        crate::profile::note("syn_rebuild_spans", t2.elapsed());
    }

    /// Finishes a full rebuild deferred by the incremental-bailout throttle,
    /// once its window has passed, even with no new edit to trigger
    /// `reparse`. Returns whether it actually rebuilt anything, so a caller
    /// on the idle path knows to redraw. Without this, highlighting could
    /// stay stale past the throttle window if the user stops editing right
    /// as a rebuild gets deferred.
    pub fn catch_up(&mut self) -> bool {
        if !self.full_rebuild_pending {
            return false;
        }
        let due = self
            .last_full_rebuild
            .map(|t| t.elapsed() >= FULL_REBUILD_THROTTLE)
            .unwrap_or(true);
        if due {
            self.rebuild_spans_full();
        }
        due
    }

    /// Whether a deferred full rebuild is pending and its throttle window
    /// has elapsed -- cheap enough to poll every idle tick.
    pub fn rebuild_due(&self) -> bool {
        self.full_rebuild_pending
            && self
                .last_full_rebuild
                .map(|t| t.elapsed() >= FULL_REBUILD_THROTTLE)
                .unwrap_or(true)
    }

    fn rebuild_spans_full(&mut self) {
        self.spans.clear();
        if let Some(tree) = &self.tree {
            let kws = keywords(self.lang);
            walk_subtree(
                tree.root_node(),
                self.source.as_bytes(),
                kws,
                &mut self.spans,
            );
        }
        self.spans.sort_by_key(|s| s.0);
        self.max_span_len = self
            .spans
            .iter()
            .map(|(s, e, _)| e.saturating_sub(*s))
            .max()
            .unwrap_or(0);
        self.full_rebuild_pending = false;
        self.last_full_rebuild = Some(std::time::Instant::now());
    }

    /// Highlight spans (byte ranges into the last-parsed source) intersecting
    /// [start_byte, end_byte).
    pub fn spans_in(
        &self,
        start_byte: usize,
        end_byte: usize,
    ) -> impl Iterator<Item = (usize, usize, HlClass)> + '_ {
        // `spans` is sorted by start. A span can only overlap [start_byte,
        // end_byte) if its own start is >= start_byte - max_span_len (any
        // earlier and even the longest span in the file couldn't reach this
        // far) -- binary search straight to that point instead of scanning
        // every span in the file for every visible row, every frame.
        let lo = self
            .spans
            .partition_point(|(s, _, _)| *s < start_byte.saturating_sub(self.max_span_len));
        self.spans[lo..]
            .iter()
            .take_while(move |(s, _, _)| *s < end_byte)
            .filter(move |(s, e, _)| *s < end_byte && *e > start_byte)
            .map(move |(s, e, c)| ((*s).max(start_byte), (*e).min(end_byte), *c))
    }
}

/// Computes an updated, fully sorted span list without re-walking the whole
/// tree: spans entirely before the edit are kept as-is, spans entirely
/// after are kept with their byte offsets shifted by the edit's length
/// delta, and only the region tree-sitter's own `changed_ranges` says
/// actually changed (expanded outward to the smallest enclosing node, so no
/// span gets cut off mid-node) is re-walked and reclassified. Returns
/// `None` to signal "give up, do a full rebuild instead" when the affected
/// region is large enough (relative to file size) that the bookkeeping
/// isn't worth it -- e.g. an edit that leaves the tree in a temporarily
/// unbalanced state (an unmatched bracket while typing a new block), which
/// tree-sitter may report as changing everything up to EOF.
fn compute_incremental_spans(
    old_spans: &[(usize, usize, HlClass)],
    lang: Lang,
    old_tree: &Tree,
    new_tree: &Tree,
    edit: InputEdit,
    new_text: &str,
) -> Option<Vec<(usize, usize, HlClass)>> {
    let new_len = new_text.len();
    let mut lo = edit.start_byte;
    let mut hi = edit.new_end_byte.min(new_len);
    for r in old_tree.changed_ranges(new_tree) {
        lo = lo.min(r.start_byte);
        hi = hi.max(r.end_byte.min(new_len));
    }
    if lo > hi {
        lo = hi;
    }

    let root = new_tree.root_node();
    let mut enclosing = root.descendant_for_byte_range(lo, hi).unwrap_or(root);
    // A string leaf can be contained in a classified string node. Rebuild
    // the same atomic highlight unit used by the full traversal.
    let mut ancestor = enclosing.parent();
    while let Some(node) = ancestor {
        if classify(&node, new_text.as_bytes(), keywords(lang)).is_some() {
            enclosing = node;
        }
        ancestor = node.parent();
    }
    // Include both sides of a zero-width deletion to avoid keeping a
    // token whose classification changed at the edit boundary.
    if lo == hi {
        enclosing = enclosing.parent().unwrap_or(enclosing);
    }
    let re_start = enclosing.start_byte();
    let re_end = enclosing.end_byte();

    if new_len > INCREMENTAL_MIN_FILE_LEN && re_end - re_start > new_len / 2 {
        return None;
    }

    let delta = edit.new_end_byte as isize - edit.old_end_byte as isize;
    let mut spans = Vec::with_capacity(old_spans.len());
    let mut after = Vec::new();
    for &(s, e, c) in old_spans {
        if e <= re_start && e <= edit.start_byte {
            spans.push((s, e, c));
        } else if s >= edit.old_end_byte {
            let shifted = (
                (s as isize + delta) as usize,
                (e as isize + delta) as usize,
                c,
            );
            if shifted.0 >= re_end {
                after.push(shifted);
            }
            // else: shifted into the re-walked region -- regenerated below.
        }
        // else: overlapped the edited region -- regenerated below.
    }

    // `re_start <= edit.start_byte <= edit.old_end_byte`, and every
    // post-edit-region byte offset shifts by `delta`, so
    // `edit.new_end_byte <= re_end` always holds too: `spans` so far (all
    // ending at or before re_start) comes before the freshly walked region,
    // which comes before `after` (all starting at or after re_end, already
    // shifted to new-text coordinates) -- a plain concatenation stays fully
    // sorted, no merge against the kept spans needed.
    // If an old atomic span contains the new region, a leaf walk cannot
    // reconstruct its prefix. Fall back rather than dropping that prefix.
    if old_spans
        .iter()
        .any(|(s, e, _)| *s < re_start && *e > edit.start_byte)
    {
        return None;
    }
    let before_len = spans.len();
    let kws = keywords(lang);
    walk_subtree(enclosing, new_text.as_bytes(), kws, &mut spans);
    spans[before_len..].sort_by_key(|s| s.0);
    spans.extend(after);

    Some(spans)
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
    Point {
        row,
        column: offset - line_start,
    }
}

fn classify(node: &Node, source: &[u8], kws: &[&str]) -> Option<HlClass> {
    let kind = node.kind();
    if kind.contains("comment") {
        Some(HlClass::Comment)
    } else if kind.contains("string")
        || kind.contains("char_literal")
        || kind.contains("attribute_value")
    {
        // `attribute_value`/`quoted_attribute_value` (HTML) are the
        // closest thing to a string literal that grammar has -- still a
        // generic substring match, not a per-language special case, so
        // it doesn't need its own branch in `ts_language`/`keywords`.
        Some(HlClass::String)
    } else if kind.contains("number") || kind.contains("integer") || kind.contains("float") {
        Some(HlClass::Number)
    } else if !node.is_named() {
        node.utf8_text(source)
            .ok()
            .filter(|t| kws.contains(t))
            .map(|_| HlClass::Keyword)
    } else {
        None
    }
}

/// Walks `node` and its descendants, classifying each and appending
/// highlight spans to `out`. Recurses on `Node` directly (each level
/// iterates its own children via a cursor scoped to that node) rather than
/// threading a single shared `TreeCursor` through -- a shared cursor's
/// `goto_parent`/`goto_next_sibling` can walk back out past the node it was
/// created from, up to the real tree root, which would silently widen an
/// incremental re-walk meant to stay confined to `node`'s own subtree.
fn walk_subtree(node: Node, source: &[u8], kws: &[&str], out: &mut Vec<(usize, usize, HlClass)>) {
    if let Some(class) = classify(&node, source, kws) {
        out.push((node.start_byte(), node.end_byte(), class));
        return;
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_subtree(cursor.node(), source, kws, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    #[test]
    fn incremental_matches_fresh_parse() {
        let mut text = (0..200)
            .map(|i| format!("fn f{i}() {{ let value = \"hello\"; /* comment */ }}\n"))
            .collect::<String>();
        let mut syn = Syntax::new(Lang::Rust).unwrap();
        syn.reparse(text.clone().into());
        for n in 0..120 {
            let needle = if n % 2 == 0 { "hello" } else { "world" };
            if let Some(i) = text.find(needle) {
                text.replace_range(
                    i..i + needle.len(),
                    if n % 2 == 0 { "world" } else { "hello" },
                );
            }
            if n % 7 == 0 {
                text.insert_str(0, "// prefix 界\n");
            }
            if n % 11 == 0 {
                text.insert_str(0, "/*");
            }
            if n % 11 == 1 && text.starts_with("/*") {
                text.replace_range(..2, "");
            }
            syn.reparse(text.clone().into());
            if syn.full_rebuild_pending {
                syn.rebuild_spans_full();
            }
            let mut fresh = Syntax::new(Lang::Rust).unwrap();
            fresh.reparse(text.clone().into());
            assert_eq!(syn.spans, fresh.spans, "edit {n}");
        }
    }
    #[test]
    fn deferred_spans_never_reused_as_current_revision() {
        let mut text = "fn a() { let value = 123; }\n".repeat(500);
        let mut s = Syntax::new(Lang::Rust).unwrap();
        s.reparse(text.clone().into());
        text.insert_str(0, "/*");
        s.reparse(text.clone().into());
        assert!(s.full_rebuild_pending);
        text.insert(4, 'x');
        s.reparse(text.clone().into());
        if s.full_rebuild_pending {
            s.rebuild_spans_full();
        }
        let mut fresh = Syntax::new(Lang::Rust).unwrap();
        fresh.reparse(text.into());
        assert_eq!(s.spans, fresh.spans);
    }
}

#[cfg(test)]
mod added_language_tests {
    use super::*;
    fn classes(lang: Lang, text: &str) -> Vec<HlClass> {
        let mut syn = Syntax::new(lang).unwrap();
        syn.reparse(Rc::from(text));
        syn.spans_in(0, text.len()).map(|(_, _, c)| c).collect()
    }
    #[test]
    fn vim_highlights_comment_keyword_and_number() {
        let text = "\" a comment\nlet x = 1\nfunction Foo()\nendfunction\n";
        let classes = classes(Lang::Vim, text);
        assert!(classes.contains(&HlClass::Comment), "{classes:?}");
        assert!(classes.contains(&HlClass::Keyword), "{classes:?}");
        assert!(classes.contains(&HlClass::Number), "{classes:?}");
    }
    #[test]
    fn css_highlights_comment_number_and_string() {
        let text = "/* c */\n.a { width: 1px; content: \"hi\"; }\n";
        let classes = classes(Lang::Css, text);
        assert!(classes.contains(&HlClass::Comment), "{classes:?}");
        assert!(classes.contains(&HlClass::Number), "{classes:?}");
        assert!(classes.contains(&HlClass::String), "{classes:?}");
    }
    #[test]
    fn html_highlights_comment_and_attribute_value_as_string() {
        let text = "<!-- c -->\n<div class=\"a\">text</div>\n";
        let classes = classes(Lang::Html, text);
        assert!(classes.contains(&HlClass::Comment), "{classes:?}");
        assert!(
            classes.contains(&HlClass::String),
            "a quoted attribute value should classify as String, got {classes:?}"
        );
    }
    #[test]
    fn solidity_highlights_comment_keyword_number_and_string() {
        let text = "// c\ncontract Foo { uint x = 1; string s = \"hi\"; }\n";
        let classes = classes(Lang::Solidity, text);
        assert!(classes.contains(&HlClass::Comment), "{classes:?}");
        assert!(classes.contains(&HlClass::Keyword), "{classes:?}");
        assert!(classes.contains(&HlClass::Number), "{classes:?}");
        assert!(classes.contains(&HlClass::String), "{classes:?}");
    }
    #[test]
    fn new_languages_are_reachable_by_their_common_file_extensions() {
        assert_eq!(lang_for_extension("vim"), Some(Lang::Vim));
        assert_eq!(lang_for_extension("css"), Some(Lang::Css));
        assert_eq!(lang_for_extension("html"), Some(Lang::Html));
        assert_eq!(lang_for_extension("htm"), Some(Lang::Html));
        assert_eq!(lang_for_extension("sol"), Some(Lang::Solidity));
    }
}
