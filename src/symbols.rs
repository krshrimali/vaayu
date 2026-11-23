//! A workspace index of where names are defined -- functions, types and
//! classes, modules, constants, struct/interface fields and top-level
//! variables -- so a name in an agent's output (`open_file`, `Editor`,
//! `ai_split`) can be clicked through to its definition.
//!
//! Definitions are found per line with language-agnostic patterns rather than
//! a parser or language server: it costs one pass over the project's source
//! files (on a background thread, see `Editor::start_symbol_index`), works
//! for any language with keyword-introduced definitions, and stays current
//! through the file watcher re-indexing changed files.
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What a name was defined as, best first: when a name has definitions of
/// several kinds, only the best kind is offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Type,
    Function,
    Module,
    Constant,
    Field,
    Variable,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Type => "type",
            Kind::Function => "function",
            Kind::Module => "module",
            Kind::Constant => "constant",
            Kind::Field => "field",
            Kind::Variable => "variable",
        }
    }
}

/// One definition: an absolute path, a 1-based line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub kind: Kind,
}

/// Files larger than this are not indexed (generated code, bundles).
const MAX_FILE_BYTES: u64 = 512 * 1024;
/// At most this many files are indexed.
const MAX_FILES: usize = 30_000;

#[derive(Clone, Debug, Default)]
pub struct Index {
    by_name: HashMap<String, Vec<Def>>,
    /// The names each file defines, to drop them when it is re-indexed.
    by_file: HashMap<PathBuf, Vec<String>>,
}

impl Index {
    /// Indexes `files` (relative to `root`).
    pub fn build(root: &Path, files: &[String]) -> Index {
        let mut index = Index::default();
        for f in files.iter().filter(|f| indexable(f)).take(MAX_FILES) {
            index.add_file(&root.join(f));
        }
        index
    }

    /// Re-reads one file (dropping it when it is gone or not indexable).
    pub fn reindex_file(&mut self, path: &Path) {
        self.remove_file(path);
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(indexable)
        {
            self.add_file(path);
        }
    }

    fn remove_file(&mut self, path: &Path) {
        let Some(names) = self.by_file.remove(path) else {
            return;
        };
        for name in names {
            if let Some(defs) = self.by_name.get_mut(&name) {
                defs.retain(|d| d.path != path);
                if defs.is_empty() {
                    self.by_name.remove(&name);
                }
            }
        }
    }

    fn add_file(&mut self, path: &Path) {
        if std::fs::metadata(path).map_or(true, |m| !m.is_file() || m.len() > MAX_FILE_BYTES) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let found = scan(&text, &ext);
        if found.is_empty() {
            return;
        }
        let mut names = Vec::with_capacity(found.len());
        for (name, line, col, kind) in found {
            let defs = self.by_name.entry(name.clone()).or_default();
            if !defs.iter().any(|d| d.path == path && d.line == line) {
                defs.push(Def {
                    path: path.to_path_buf(),
                    line,
                    col,
                    kind,
                });
            }
            names.push(name);
        }
        names.sort();
        names.dedup();
        self.by_file.insert(path.to_path_buf(), names);
    }

    /// Every definition of `name`.
    pub fn defs(&self, name: &str) -> &[Def] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    /// The definitions worth jumping to for `name`: those of its best kind,
    /// narrowed to the files defining `qualifier` (`Editor` in
    /// `Editor::open_file`) and then to `context` files (ones the output
    /// mentioned nearby) when either leaves any.
    pub fn lookup(&self, name: &str, qualifier: Option<&str>, context: &[PathBuf]) -> Vec<Def> {
        let defs = self.defs(name);
        let Some(best) = defs.iter().map(|d| d.kind).min() else {
            return Vec::new();
        };
        let mut out: Vec<Def> = defs.iter().filter(|d| d.kind == best).cloned().collect();
        if out.len() > 1 {
            if let Some(q) = qualifier {
                let files: Vec<&PathBuf> = self.defs(q).iter().map(|d| &d.path).collect();
                let narrowed: Vec<Def> = out
                    .iter()
                    .filter(|d| files.contains(&&d.path))
                    .cloned()
                    .collect();
                if !narrowed.is_empty() {
                    out = narrowed;
                }
            }
        }
        if out.len() > 1 && !context.is_empty() {
            let narrowed: Vec<Def> = out
                .iter()
                .filter(|d| context.contains(&d.path))
                .cloned()
                .collect();
            if !narrowed.is_empty() {
                out = narrowed;
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        out
    }
}

/// Whether a file name looks like source worth indexing.
fn indexable(name: &str) -> bool {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "rs" | "py"
            | "pyi"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "mts"
            | "cts"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "scala"
            | "swift"
            | "dart"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "cxx"
            | "hh"
            | "hpp"
            | "hxx"
            | "m"
            | "mm"
            | "cs"
            | "fs"
            | "rb"
            | "php"
            | "lua"
            | "ex"
            | "exs"
            | "erl"
            | "zig"
            | "nim"
            | "jl"
            | "r"
            | "pl"
            | "pm"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "vim"
            | "el"
            | "clj"
            | "hs"
            | "ml"
            | "mli"
            | "sql"
            | "proto"
            | "graphql"
            | "gql"
            | "sol"
            | "v"
            | "sv"
            | "vue"
            | "svelte"
    )
}

/// Keyword-introduced definitions: optional modifiers, a definition keyword,
/// then the name (`pub(crate) async fn open`, `export default class App`,
/// `func (e *Editor) Open`, `static mut COUNT`).
fn keyword_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?x)
            ^(?P<indent>[\ \t]*)
            (?:(?:pub(?:\([^)]*\))?|export|default|async|static|public|private|protected
                |internal|abstract|final|override|unsafe|extern(?:[\ \t]+"[^"]*")?|inline
                |virtual|open|data|sealed|declare|readonly|partial|local|global|inner
                |suspend|operator|infix|tailrec|lateinit|const)[\ \t]+)*
            (?P<kw>fn|func|function\*?|def|defp|defmacro|defmodule|defstruct|fun|sub|proc
                |class|struct|enum|trait|interface|type|typedef[\ \t]+(?:struct|enum|union)
                |union|mod|module|namespace|protocol|record|object|macro_rules!
                |const|static|let|var|val|message|service)
            [\ \t]+
            (?:\([^)]*\)[\ \t]*)?
            (?:mut[\ \t]+)?
            (?:self\.)?
            (?P<name>[A-Za-z_$][\w$]*[?!]?)"#,
        )
        .unwrap()
    })
}

/// A C-family function or method definition: a return type then the name and
/// an opening parenthesis, with no `;` after it on the line.
fn c_function_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^[ \t]*(?P<ty>[A-Za-z_][\w:<>,*&\[\]? ]*?[\w>*&\]?])[ \t*&]+(?P<name>[A-Za-z_]\w*)[ \t]*\([^;]*$",
        )
        .unwrap()
    })
}

/// A field: `name: Type` (Rust, TypeScript, Python dataclasses), indented.
fn field_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^[ \t]+(?:pub(?:\([^)]*\))?[ \t]+|readonly[ \t]+|private[ \t]+|public[ \t]+|protected[ \t]+)*(?P<name>[A-Za-z_]\w*)\??[ \t]*:[ \t]*(?P<ty>[^=]+?)[ \t]*[,;]?[ \t]*(?://.*)?$",
        )
        .unwrap()
    })
}

/// A top-level assignment: `NAME = …` / `name: T = …` at column 0.
fn assign_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?P<name>[A-Za-z_]\w*)[ \t]*(?::[^=]+)?=[^=]").unwrap())
}

/// A shell function: `name() {` or `function name`.
fn shell_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[ \t]*(?P<name>[A-Za-z_][\w-]*)[ \t]*\(\)").unwrap())
}

/// Words a C-family match takes for a function name but that are not.
const NOT_NAMES: &[&str] = &[
    "if", "for", "while", "switch", "return", "catch", "else", "new", "delete", "sizeof", "throw",
    "case", "do", "await", "yield", "typeof", "using", "foreach", "lock", "match",
];

/// Whether a field's type text reads like a type (not an expression in a
/// struct literal or object literal).
fn type_like(ty: &str) -> bool {
    let ty = ty.trim();
    if ty.is_empty() || ty.contains('{') || ty.contains('.') || ty.ends_with('(') {
        return false;
    }
    // `Vec::new()`, `Some(5)`, `foo(x)`: calls. Only function types put
    // a word right before a parenthesis.
    static CALL: OnceLock<Regex> = OnceLock::new();
    let call = CALL.get_or_init(|| Regex::new(r"(\w+)[ \t]*\(").unwrap());
    if call
        .captures_iter(ty)
        .any(|c| !matches!(&c[1], "fn" | "Fn" | "FnMut" | "FnOnce"))
    {
        return false;
    }
    // `Kind::Type`: an enum variant, not a type.
    static VARIANT: OnceLock<Regex> = OnceLock::new();
    let variant = VARIANT.get_or_init(|| Regex::new(r"^[A-Z]\w*::[A-Z]\w*$").unwrap());
    if variant.is_match(ty.trim_end_matches([',', ' '])) {
        return false;
    }
    let first = ty.trim_start_matches(['&', '*', '[', '(', '\'', ' ']);
    let first = first.strip_prefix("mut ").unwrap_or(first);
    let word: String = first
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    word.starts_with(|c: char| c.is_ascii_uppercase())
        || matches!(
            word.as_str(),
            "u8" | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "f32"
                | "f64"
                | "bool"
                | "char"
                | "str"
                | "dyn"
                | "impl"
                | "fn"
                | "std"
                | "crate"
                | "super"
                | "string"
                | "number"
                | "boolean"
                | "any"
                | "unknown"
                | "never"
                | "void"
                | "int"
                | "float"
                | "bytes"
                | "list"
                | "dict"
                | "tuple"
                | "set"
                | "object"
                | "bigint"
                | "symbol"
                | "undefined"
                | "null"
        )
}

/// The definitions on each line of `text` (a file with extension `ext`), as
/// `(name, line, col, kind)` with 1-based line and column.
pub fn scan(text: &str, ext: &str) -> Vec<(String, usize, usize, Kind)> {
    let c_family = matches!(
        ext,
        "c" | "h"
            | "cc"
            | "cpp"
            | "cxx"
            | "hh"
            | "hpp"
            | "hxx"
            | "java"
            | "cs"
            | "m"
            | "mm"
            | "dart"
            | "scala"
    );
    let fields = matches!(
        ext,
        "rs" | "ts" | "tsx" | "mts" | "cts" | "py" | "pyi" | "swift" | "kt" | "kts" | "scala"
    );
    let assigns = matches!(
        ext,
        "py" | "pyi" | "rb" | "sh" | "bash" | "zsh" | "r" | "jl" | "lua" | "ex" | "exs" | "nim"
    );
    let shell = matches!(ext, "sh" | "bash" | "zsh");
    let mut out = Vec::new();
    let mut in_block_comment = false;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        // Skip comments, so prose like "// this function returns…" is not a
        // definition of `returns`.
        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") && !trimmed.contains("*/") {
            in_block_comment = true;
            continue;
        }
        if trimmed.starts_with("//")
            || trimmed.starts_with('*')
            || trimmed.starts_with("--")
            || (trimmed.starts_with('#')
                && !trimmed.starts_with("#define")
                && !trimmed.starts_with("#["))
            || trimmed.starts_with(';')
            || trimmed.starts_with('"') && ext == "vim"
        {
            if let Some(rest) = trimmed.strip_prefix("#define") {
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    let col = line.find(&name).unwrap_or(0) + 1;
                    out.push((name, i + 1, col, Kind::Constant));
                }
            }
            continue;
        }
        let push = |out: &mut Vec<_>, name: &str, start: usize, kind: Kind| {
            let col = line[..start].chars().count() + 1;
            out.push((name.to_string(), i + 1, col, kind));
        };
        if let Some(c) = keyword_re().captures(line) {
            let kw = c.name("kw").unwrap().as_str();
            let name = c.name("name").unwrap();
            let top = c.name("indent").unwrap().as_str().is_empty();
            let kind = match kw {
                "class" | "struct" | "enum" | "trait" | "interface" | "type" | "union"
                | "protocol" | "record" | "object" | "message" | "service" | "defstruct" => {
                    Some(Kind::Type)
                }
                k if k.starts_with("typedef") => Some(Kind::Type),
                "mod" | "module" | "namespace" | "defmodule" => Some(Kind::Module),
                // Typed languages declare variables with a type, so there
                // `static`/`const` are modifiers (`public static void`).
                "const" | "static" | "let" | "var" | "val" if c_family => None,
                // An indented `const` is a local unless it is SCREAMING_CASE
                // (Rust associated constants, class constants).
                "const" | "static"
                    if top
                        || name
                            .as_str()
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') =>
                {
                    Some(Kind::Constant)
                }
                "const" | "static" => None,
                "let" | "var" | "val" => top.then_some(Kind::Variable),
                _ => Some(Kind::Function),
            };
            // `type` in prose-like positions (`type = "x"` in a config
            // block, `type: string`) is not a definition.
            let after = &line[name.end()..];
            let bogus = kw == "type" && after.trim_start().starts_with([':', ',']);
            if let (Some(kind), false) = (kind, bogus) {
                push(&mut out, name.as_str(), name.start(), kind);
                continue;
            }
        }
        if shell {
            if let Some(c) = shell_fn_re().captures(line) {
                let name = c.name("name").unwrap();
                push(&mut out, name.as_str(), name.start(), Kind::Function);
                continue;
            }
        }
        if c_family {
            if let Some(c) = c_function_re().captures(line) {
                let name = c.name("name").unwrap();
                let ty = c.name("ty").unwrap().as_str();
                let ty_word = ty.split_whitespace().last().unwrap_or(ty);
                if !NOT_NAMES.contains(&name.as_str()) && !NOT_NAMES.contains(&ty_word) {
                    push(&mut out, name.as_str(), name.start(), Kind::Function);
                    continue;
                }
            }
        }
        if fields {
            if let Some(c) = field_re().captures(line) {
                let name = c.name("name").unwrap();
                if type_like(c.name("ty").unwrap().as_str()) && !NOT_NAMES.contains(&name.as_str())
                {
                    push(&mut out, name.as_str(), name.start(), Kind::Field);
                    continue;
                }
            }
        }
        if assigns {
            if let Some(c) = assign_re().captures(line) {
                let name = c.name("name").unwrap();
                push(&mut out, name.as_str(), name.start(), Kind::Variable);
            }
        }
    }
    out
}

impl crate::editor::Editor {
    /// Rebuilds the symbol index from `all_files` on a background thread;
    /// `poll_symbol_index` installs it.
    pub(crate) fn start_symbol_index(&mut self) {
        let root = self.project_root.clone();
        let files = self.all_files.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.symbol_index_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send((root.clone(), Index::build(&root, &files)));
        });
    }

    /// Installs a finished index build, replaying files that changed while it
    /// ran. Returns whether the index changed.
    pub(crate) fn poll_symbol_index(&mut self) -> bool {
        let Some(rx) = &self.symbol_index_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok((root, index)) => {
                self.symbol_index_rx = None;
                if root != self.project_root {
                    return false;
                }
                self.symbols = index;
                for path in std::mem::take(&mut self.symbols_stale) {
                    self.symbols.reindex_file(&path);
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.symbol_index_rx = None;
                false
            }
        }
    }

    /// Re-indexes files the watcher saw change.
    pub(crate) fn reindex_symbols<'a>(&mut self, paths: impl IntoIterator<Item = &'a PathBuf>) {
        for path in paths {
            if self.symbol_index_rx.is_some() {
                self.symbols_stale.push(path.clone());
            } else {
                self.symbols.reindex_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str, ext: &str) -> Vec<(String, Kind)> {
        scan(text, ext)
            .into_iter()
            .map(|(n, _, _, k)| (n, k))
            .collect()
    }

    fn has(found: &[(String, Kind)], name: &str, kind: Kind) -> bool {
        found.iter().any(|(n, k)| n == name && *k == kind)
    }

    #[test]
    fn rust_items_fields_and_not_locals_or_literals() {
        let src = r#"
pub struct Editor {
    pub ai_split: String,
    pending: Option<(u64, Instant)>,
}
pub(crate) async fn open_file(&mut self) {}
impl Foo {
    pub(crate) fn link_at(&self) {
        let rows = 5;
        let x = Foo {
            rows: 5,
            list: Vec::new(),
        };
    }
}
const MAX: usize = 3;
static mut COUNT: u32 = 0;
enum Kind { A }
trait Show {}
type Alias = u8;
mod termlinks;
macro_rules! hello { () => {} }
// fn commented_out() {}
"#;
        let f = names(src, "rs");
        for (n, k) in [
            ("Editor", Kind::Type),
            ("ai_split", Kind::Field),
            ("pending", Kind::Field),
            ("open_file", Kind::Function),
            ("link_at", Kind::Function),
            ("MAX", Kind::Constant),
            ("COUNT", Kind::Constant),
            ("Kind", Kind::Type),
            ("Show", Kind::Type),
            ("Alias", Kind::Type),
            ("termlinks", Kind::Module),
            ("hello", Kind::Function),
        ] {
            assert!(has(&f, n, k), "{n} as {k:?} in {f:?}");
        }
        for n in ["rows", "x", "list", "commented_out"] {
            assert!(
                !f.iter().any(|(m, _)| m == n),
                "{n} is not a definition: {f:?}"
            );
        }
    }

    #[test]
    fn python_typescript_go_and_c_family() {
        let py = names(
            "class Model(Base):\n    name: str\n    def save(self):\n        local = 1\nasync def main():\n    pass\nTIMEOUT = 30\n",
            "py",
        );
        assert!(has(&py, "Model", Kind::Type));
        assert!(has(&py, "save", Kind::Function));
        assert!(has(&py, "main", Kind::Function));
        assert!(has(&py, "TIMEOUT", Kind::Variable));
        assert!(has(&py, "name", Kind::Field));
        assert!(!py.iter().any(|(n, _)| n == "local"));

        let ts = names(
            "export default class App {}\nexport interface Props {\n  title: string;\n}\nexport const useThing = () => {};\nfunction helper() {}\nexport type Id = string;\n",
            "ts",
        );
        assert!(has(&ts, "App", Kind::Type));
        assert!(has(&ts, "Props", Kind::Type));
        assert!(has(&ts, "title", Kind::Field));
        assert!(has(&ts, "useThing", Kind::Constant));
        assert!(has(&ts, "helper", Kind::Function));
        assert!(has(&ts, "Id", Kind::Type));

        let go = names(
            "func (e *Editor) Open(p string) error {\n}\nfunc main() {}\ntype Server struct {}\n",
            "go",
        );
        assert!(has(&go, "Open", Kind::Function));
        assert!(has(&go, "main", Kind::Function));
        assert!(has(&go, "Server", Kind::Type));

        let java = names(
            "public class Main {\n    public static void run(String[] a) {\n        if (x) {\n        return foo(1);\n    }\n}\n#define LIMIT 4\n",
            "java",
        );
        assert!(has(&java, "Main", Kind::Type));
        assert!(has(&java, "run", Kind::Function));
        assert!(!java.iter().any(|(n, _)| n == "if" || n == "foo"));
    }

    #[test]
    fn lookup_prefers_best_kind_then_qualifier_then_context() {
        let dir = std::env::temp_dir().join(format!("vaayu-sym-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.rs"),
            "struct Editor;\nimpl Editor {\n    fn open(&self) {}\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("b.rs"),
            "struct Other;\nfn open() {}\nfn open_b() {}\n",
        )
        .unwrap();
        std::fs::write(dir.join("c.rs"), "struct Thing {\n    open: u8,\n}\n").unwrap();
        let index = Index::build(&dir, &["a.rs".into(), "b.rs".into(), "c.rs".into()]);
        // Functions beat the field of the same name; two remain.
        assert_eq!(index.lookup("open", None, &[]).len(), 2);
        let q = index.lookup("open", Some("Editor"), &[]);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].path, dir.join("a.rs"));
        assert_eq!((q[0].line, q[0].col), (3, 8));
        let c = index.lookup("open", None, &[dir.join("b.rs")]);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].path, dir.join("b.rs"));

        // Re-indexing a changed file replaces its definitions.
        let mut index = index;
        std::fs::write(dir.join("b.rs"), "fn renamed() {}\n").unwrap();
        index.reindex_file(&dir.join("b.rs"));
        assert_eq!(index.lookup("open", None, &[]).len(), 1);
        assert_eq!(index.lookup("renamed", None, &[]).len(), 1);
        assert!(index.defs("open_b").is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
