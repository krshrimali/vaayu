//! Per-buffer indentation detection: a Vim modeline overrides an
//! `.editorconfig` entry, which overrides heuristic detection from the
//! file's own content, which falls back to the global config default.
//! Deterministic given the same file content and directory: no network,
//! no watching, computed once when the buffer is opened.
use crate::config::Config;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentSource {
    Modeline,
    EditorConfig,
    Detected,
    Default,
}

impl IndentSource {
    pub fn label(self) -> &'static str {
        match self {
            IndentSource::Modeline => "modeline",
            IndentSource::EditorConfig => ".editorconfig",
            IndentSource::Detected => "detected",
            IndentSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndentSettings {
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub expandtab: bool,
    pub source: IndentSource,
}

#[derive(Default)]
struct Partial {
    tabstop: Option<usize>,
    shiftwidth: Option<usize>,
    expandtab: Option<bool>,
}

impl Partial {
    fn is_empty(&self) -> bool {
        self.tabstop.is_none() && self.shiftwidth.is_none() && self.expandtab.is_none()
    }
}

fn merge(base: IndentSettings, p: Partial, source: IndentSource) -> IndentSettings {
    IndentSettings {
        tabstop: p.tabstop.unwrap_or(base.tabstop),
        shiftwidth: p.shiftwidth.unwrap_or(base.shiftwidth),
        expandtab: p.expandtab.unwrap_or(base.expandtab),
        source,
    }
}

pub fn resolve(path: Option<&Path>, text: &str, cfg: &Config) -> IndentSettings {
    let fallback = IndentSettings {
        tabstop: cfg.tabstop,
        shiftwidth: cfg.shiftwidth,
        expandtab: cfg.expandtab,
        source: IndentSource::Default,
    };
    if let Some(p) = modeline(text) {
        return merge(fallback, p, IndentSource::Modeline);
    }
    if let Some(path) = path {
        if let Some(p) = editorconfig(path) {
            return merge(fallback, p, IndentSource::EditorConfig);
        }
    }
    if let Some(p) = detect(text) {
        return merge(fallback, p, IndentSource::Detected);
    }
    fallback
}

/// Vim modelines: a `vim:`/`vi:` marker within the first or last 5 lines,
/// optionally followed by `set`, then space-separated `key=value`/bare
/// options up to a trailing `:` or end of line. Supports `sw`/`shiftwidth`,
/// `ts`/`tabstop`, `et`/`expandtab`, `noet`/`noexpandtab`.
fn modeline(text: &str) -> Option<Partial> {
    let lines: Vec<&str> = text.lines().collect();
    let candidates = lines
        .iter()
        .take(5)
        .chain(lines.iter().rev().take(5))
        .copied();
    for line in candidates {
        let Some(rest) = find_modeline_marker(line) else {
            continue;
        };
        let rest = rest.strip_prefix("set ").unwrap_or(rest);
        let rest = rest.split(':').next().unwrap_or(rest);
        let mut p = Partial::default();
        for token in rest.split_whitespace() {
            match token.split_once('=') {
                Some(("sw" | "shiftwidth", v)) => p.shiftwidth = v.parse().ok(),
                Some(("ts" | "tabstop", v)) => p.tabstop = v.parse().ok(),
                None if token == "et" || token == "expandtab" => p.expandtab = Some(true),
                None if token == "noet" || token == "noexpandtab" => p.expandtab = Some(false),
                _ => {}
            }
        }
        if !p.is_empty() {
            return Some(p);
        }
    }
    None
}

fn find_modeline_marker(line: &str) -> Option<&str> {
    for marker in ["vim:", "vi:"] {
        if let Some(idx) = line.find(marker) {
            return Some(&line[idx + marker.len()..]);
        }
    }
    None
}

/// A minimal `.editorconfig` reader: walks from the file's directory up to
/// the first `root = true` (or the filesystem root), and within each file,
/// the first section whose glob matches the filename wins (closest file to
/// the edited buffer takes precedence, matching the real tool). Supports
/// `*` and `*.ext` globs and `indent_style`/`indent_size`/`tab_width` --
/// brace-expansion globs (`*.{js,ts}`) and other keys are not implemented.
fn editorconfig(path: &Path) -> Option<Partial> {
    let path = path.canonicalize().ok()?;
    let name = path.file_name()?.to_str()?;
    let mut dir = path.parent()?.to_path_buf();
    loop {
        let candidate = dir.join(".editorconfig");
        if candidate.is_file() {
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                if let Some(p) = parse_editorconfig(&text, name) {
                    return Some(p);
                }
                if is_root(&text) {
                    return None;
                }
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return None,
        }
    }
}

/// Read the `.editorconfig` `end_of_line` setting for `path` (walking parent
/// directories up to `root = true`, closest file wins), mapping `lf`/`crlf`/`cr`
/// to a `FileFormat`. `None` when unset -- callers keep the detected ending.
pub fn editorconfig_eol(path: &Path) -> Option<crate::buffer::FileFormat> {
    let path = path.canonicalize().ok()?;
    let name = path.file_name()?.to_str()?;
    let mut dir = path.parent()?.to_path_buf();
    loop {
        let candidate = dir.join(".editorconfig");
        if candidate.is_file() {
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                if let Some(eol) = parse_editorconfig_eol(&text, name) {
                    return Some(eol);
                }
                if is_root(&text) {
                    return None;
                }
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return None,
        }
    }
}

/// Non-indent per-file `.editorconfig` settings that vaayu applies on load.
#[derive(Default, Clone, Copy)]
pub struct EcExtras {
    pub trim_trailing: Option<bool>,
    pub final_newline: Option<bool>,
    pub max_line_length: Option<usize>,
}

/// Read `trim_trailing_whitespace`, `insert_final_newline`, and
/// `max_line_length` for `path` from `.editorconfig` (same walk/glob/last-
/// section rules as the indent reader). Unset keys stay `None`.
pub fn editorconfig_extras(path: &Path) -> EcExtras {
    let Ok(path) = path.canonicalize() else {
        return EcExtras::default();
    };
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return EcExtras::default();
    };
    let mut dir = match path.parent() {
        Some(p) => p.to_path_buf(),
        None => return EcExtras::default(),
    };
    let mut out = EcExtras::default();
    loop {
        let candidate = dir.join(".editorconfig");
        if candidate.is_file() {
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                // Closest file wins: only fill fields not already set by a nearer
                // `.editorconfig` (each file resolved last-section-wins).
                let file = parse_file_extras(&text, name);
                out.trim_trailing = out.trim_trailing.or(file.trim_trailing);
                out.final_newline = out.final_newline.or(file.final_newline);
                out.max_line_length = out.max_line_length.or(file.max_line_length);
                if is_root(&text) {
                    break;
                }
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break,
        }
    }
    out
}

/// Resolve one `.editorconfig` file's extras for `name`, later matching
/// sections overriding earlier ones (real EditorConfig within-file semantics).
fn parse_file_extras(text: &str, name: &str) -> EcExtras {
    let mut r = EcExtras::default();
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_section = glob_matches(section, name);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            match key.trim() {
                "trim_trailing_whitespace" => {
                    if let Some(b) = parse_bool(value) {
                        r.trim_trailing = Some(b);
                    }
                }
                "insert_final_newline" => {
                    if let Some(b) = parse_bool(value) {
                        r.final_newline = Some(b);
                    }
                }
                "max_line_length" => {
                    if let Ok(n) = value.parse() {
                        r.max_line_length = Some(n);
                    }
                }
                _ => {}
            }
        }
    }
    r
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_editorconfig_eol(text: &str, name: &str) -> Option<crate::buffer::FileFormat> {
    use crate::buffer::FileFormat;
    let mut in_section = false;
    // Last matching section wins (like the indent parser), so a more specific
    // section listed later overrides an earlier `[*]`.
    let mut result = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_section = glob_matches(section, name);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case("end_of_line") {
                match value.trim().to_ascii_lowercase().as_str() {
                    "lf" => result = Some(FileFormat::Unix),
                    "crlf" => result = Some(FileFormat::Dos),
                    "cr" => result = Some(FileFormat::Mac),
                    _ => {} // ignore an invalid value, keep any prior valid one
                }
            }
        }
    }
    result
}

fn is_root(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l.eq_ignore_ascii_case("root = true") || l.eq_ignore_ascii_case("root=true")
    })
}

fn glob_matches(glob: &str, name: &str) -> bool {
    match glob {
        "*" => true,
        g if g.starts_with("*.") => name.ends_with(&g[1..]),
        g => g == name,
    }
}

fn parse_editorconfig(text: &str, name: &str) -> Option<Partial> {
    let mut in_matching_section = false;
    let mut p = Partial::default();
    let mut found = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_matching_section = glob_matches(section, name);
            continue;
        }
        if !in_matching_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "indent_style" => {
                p.expandtab = Some(value.eq_ignore_ascii_case("space"));
                found = true;
            }
            "indent_size" => {
                if let Ok(n) = value.parse() {
                    p.shiftwidth = Some(n);
                    found = true;
                }
            }
            "tab_width" => {
                if let Ok(n) = value.parse() {
                    p.tabstop = Some(n);
                    found = true;
                }
            }
            _ => {}
        }
    }
    // `tab_width` defaults to `indent_size` when only the latter is given.
    if p.tabstop.is_none() {
        p.tabstop = p.shiftwidth;
    }
    found.then_some(p)
}

/// vim-sleuth-style heuristic: scan up to 2000 lines, tallying tab- vs
/// space-indented lines. Tabs winning leaves shiftwidth/tabstop at the
/// config default (tab display width is a display setting, not something
/// to guess); spaces winning guesses shiftwidth from the smallest nonzero
/// leading-space run seen, which is right for consistently-indented files
/// and a reasonable, deterministic guess otherwise.
fn detect(text: &str) -> Option<Partial> {
    let mut tab_lines = 0usize;
    let mut space_counts: Vec<usize> = Vec::new();
    for line in text.lines().take(2000) {
        if line.starts_with('\t') {
            tab_lines += 1;
        } else if line.starts_with(' ') {
            let n = line.chars().take_while(|c| *c == ' ').count();
            if n > 0 && line.chars().nth(n).is_some_and(|c| !c.is_whitespace()) {
                space_counts.push(n);
            }
        }
    }
    if tab_lines == 0 && space_counts.is_empty() {
        return None;
    }
    if tab_lines > space_counts.len() {
        return Some(Partial {
            expandtab: Some(false),
            ..Default::default()
        });
    }
    let min = *space_counts.iter().min()?;
    Some(Partial {
        expandtab: Some(true),
        shiftwidth: Some(min),
        tabstop: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn falls_back_to_config_default() {
        let cfg = Config {
            tabstop: 4,
            shiftwidth: 4,
            expandtab: true,
            ..Config::default()
        };
        let s = resolve(None, "plain text\nno hints\n", &cfg);
        assert_eq!(s.source, IndentSource::Default);
        assert_eq!(s.shiftwidth, 4);
        assert!(s.expandtab);
    }

    #[test]
    fn detects_two_space_indent() {
        let text = "fn f() {\n  a();\n  if x {\n    b();\n  }\n}\n";
        let s = resolve(None, text, &Config::default());
        assert_eq!(s.source, IndentSource::Detected);
        assert!(s.expandtab);
        assert_eq!(s.shiftwidth, 2);
    }

    #[test]
    fn detects_tabs() {
        let text = "fn f() {\n\ta();\n\tif x {\n\t\tb();\n\t}\n}\n";
        let s = resolve(None, text, &Config::default());
        assert_eq!(s.source, IndentSource::Detected);
        assert!(!s.expandtab);
    }

    #[test]
    fn modeline_overrides_detection() {
        let text = "a\nb\n// vim: set sw=2 ts=2 et:\n";
        let s = resolve(None, text, &Config::default());
        assert_eq!(s.source, IndentSource::Modeline);
        assert_eq!(s.shiftwidth, 2);
        assert_eq!(s.tabstop, 2);
        assert!(s.expandtab);
    }

    #[test]
    fn modeline_noexpandtab() {
        let text = "// vi: noet sw=8\n".to_string() + &"x\n".repeat(10);
        let s = resolve(None, &text, &Config::default());
        assert_eq!(s.source, IndentSource::Modeline);
        assert!(!s.expandtab);
        assert_eq!(s.shiftwidth, 8);
    }

    #[test]
    fn editorconfig_extras_reads_trim_final_and_maxlen() {
        let dir = std::env::temp_dir().join(format!("vaayu-ecx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".editorconfig"),
            "root = true\n[*]\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n\
             max_line_length = 100\n[*.md]\ntrim_trailing_whitespace = false\nmax_line_length = 0\n",
        )
        .unwrap();
        // A `.rs` file: only the `[*]` section applies.
        let rs = dir.join("a.rs");
        std::fs::write(&rs, "x\n").unwrap();
        let e = editorconfig_extras(&rs);
        assert_eq!(e.trim_trailing, Some(true));
        assert_eq!(e.final_newline, Some(true));
        assert_eq!(e.max_line_length, Some(100));
        // A `.md` file: the later `[*.md]` section overrides trim + max_line_length.
        let md = dir.join("b.md");
        std::fs::write(&md, "x\n").unwrap();
        let e = editorconfig_extras(&md);
        assert_eq!(e.trim_trailing, Some(false), "md keeps trailing ws");
        assert_eq!(e.final_newline, Some(true), "final newline inherited from [*]");
        assert_eq!(e.max_line_length, Some(0));
        std::fs::remove_dir_all(dir).ok();
    }
    #[test]
    fn editorconfig_end_of_line_maps_to_fileformat() {
        use crate::buffer::FileFormat;
        let dir = std::env::temp_dir().join(format!("vaayu-ecfg-eol-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".editorconfig"),
            "root = true\n[*]\nend_of_line = crlf\n[*.lf]\nend_of_line = lf\n",
        )
        .unwrap();
        let crlf = dir.join("a.txt");
        std::fs::write(&crlf, "x\n").unwrap();
        assert_eq!(editorconfig_eol(&crlf), Some(FileFormat::Dos));
        // A more specific section still resolves (closest matching section).
        let lf = dir.join("b.lf");
        std::fs::write(&lf, "x\n").unwrap();
        assert_eq!(editorconfig_eol(&lf), Some(FileFormat::Unix));
        // No .editorconfig -> None.
        assert_eq!(editorconfig_eol(Path::new("/nonexistent/zzz.txt")), None);
        std::fs::remove_dir_all(dir).ok();
    }
    #[test]
    fn editorconfig_overrides_detection() {
        let dir = std::env::temp_dir().join(format!("vaayu-ecfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".editorconfig"),
            "root = true\n[*.rs]\nindent_style = space\nindent_size = 3\n",
        )
        .unwrap();
        let file = dir.join("f.rs");
        std::fs::write(&file, "fn f() {\n\ta();\n}\n").unwrap();
        let s = resolve(Some(&file), "fn f() {\n\ta();\n}\n", &Config::default());
        assert_eq!(s.source, IndentSource::EditorConfig);
        assert!(s.expandtab);
        assert_eq!(s.shiftwidth, 3);
        assert_eq!(s.tabstop, 3);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn editorconfig_glob_does_not_match_other_extensions() {
        let dir = std::env::temp_dir().join(format!("vaayu-ecfg2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".editorconfig"),
            "root = true\n[*.py]\nindent_style = space\nindent_size = 4\n",
        )
        .unwrap();
        let file = dir.join("f.rs");
        std::fs::write(&file, "fn f() {\n\ta();\n}\n").unwrap();
        let s = resolve(Some(&file), "fn f() {\n\ta();\n}\n", &Config::default());
        assert_eq!(s.source, IndentSource::Detected);
        std::fs::remove_dir_all(dir).ok();
    }
}
