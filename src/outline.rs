//! Persistent document outline/symbol sidebar (`,lO`), reusing the same
//! special-pane pattern as the file tree and terminal (`Window::outline`).
//! Populated from the same `textDocument/documentSymbol` response
//! `,lo`/`:outline` already requests (see `language.rs`), but read as a
//! hierarchy (via `children`) instead of flattened into a plain list, so
//! nesting depth survives into the sidebar's indentation.
//!
//! Scope for this slice: no collapse/expand (everything is always shown
//! fully expanded -- symbol trees are rarely deep enough for this to be a
//! real problem), no live follow-cursor (highlighting the enclosing symbol
//! as the cursor moves), no symbol-kind filtering, and no hover preview.
//! `flatten` itself only sees the raw LSP response, not buffer text, so it
//! stores each symbol's column as the LSP's raw UTF-16 code unit count;
//! `language.rs`'s response handler corrects it to a char index (the same
//! `utf16_to_col` the transient `:outline` results list already applies)
//! once the buffer's line text is available -- see NEOVIM_PARITY_PLAN.md's
//! progress log.
use crate::editor::Editor;
use crate::key::Key;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone)]
pub struct SymbolNode {
    pub name: String,
    pub kind: &'static str,
    pub line: usize,
    pub col: usize,
    pub depth: usize,
}

#[derive(Default)]
pub struct Outline {
    pub buffer_path: Option<PathBuf>,
    pub nodes: Vec<SymbolNode>,
    pub cursor: usize,
}

/// LSP `SymbolKind` numeric values (1-indexed) mapped to a short label.
fn kind_label(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "ctor",
        10 => "enum",
        11 => "interface",
        12 => "fn",
        13 => "var",
        14 => "const",
        15 => "string",
        16 => "number",
        17 => "bool",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type param",
        _ => "symbol",
    }
}

/// Flattens a `DocumentSymbol[]` (hierarchical, `children`) or
/// `SymbolInformation[]` (flat, `location`) response into depth-tagged
/// nodes, capping depth/count the same way `language.rs`'s `locations()`
/// already bounds its own recursion.
pub fn flatten(value: &Value, depth: usize, out: &mut Vec<SymbolNode>) {
    if depth > 64 || out.len() >= 5000 {
        return;
    }
    let Some(arr) = value.as_array() else {
        return;
    };
    for sym in arr {
        let name = sym["name"].as_str().unwrap_or("?").to_string();
        let kind = kind_label(sym["kind"].as_u64().unwrap_or(0));
        let range = sym
            .get("selectionRange")
            .or_else(|| sym.get("range"))
            .or_else(|| sym.get("location").and_then(|l| l.get("range")));
        let (line, col) = range.map_or((0, 0), |r| {
            (
                r["start"]["line"].as_u64().unwrap_or(0) as usize,
                r["start"]["character"].as_u64().unwrap_or(0) as usize,
            )
        });
        out.push(SymbolNode {
            name,
            kind,
            line,
            col,
            depth,
        });
        if let Some(children) = sym.get("children") {
            flatten(children, depth + 1, out);
        }
    }
}

impl Editor {
    pub fn active_outline(&self) -> bool {
        self.windows
            .get(self.active_window)
            .is_some_and(|w| w.outline)
    }

    /// `,lO`: opens the sidebar in a new vertical split and requests this
    /// buffer's symbols (the response is routed to the sidebar by
    /// `language.rs` checking `active_outline`-style state instead of the
    /// transient results list, since a sidebar pane is now open), or
    /// closes the sidebar if already open.
    pub fn toggle_outline(&mut self) {
        if let Some(idx) = self.windows.iter().position(|w| w.outline) {
            self.active_window = idx;
            self.close_window();
            return;
        }
        self.outline.get_or_insert_with(Outline::default);
        self.split_window(true, false);
        if let Some(w) = self.windows.get_mut(self.active_window) {
            w.outline = true;
        }
        self.request_language("outline", None);
    }

    fn jump_from_outline(&mut self, line: usize, col: usize) {
        let Some(other) = self.windows.iter().position(|w| !w.outline) else {
            return;
        };
        self.focus_window(other);
        self.push_jump();
        self.set_cursor(line, col);
        self.enter_normal();
        self.store_window();
    }
}

pub fn handle_key(ed: &mut Editor, key: Key) {
    match key {
        Key::Char('j') | Key::Down => {
            if let Some(o) = &mut ed.outline {
                if !o.nodes.is_empty() {
                    o.cursor = (o.cursor + 1).min(o.nodes.len() - 1);
                }
            }
        }
        Key::Char('k') | Key::Up => {
            if let Some(o) = &mut ed.outline {
                o.cursor = o.cursor.saturating_sub(1);
            }
        }
        Key::Home => {
            if let Some(o) = &mut ed.outline {
                o.cursor = 0;
            }
        }
        Key::End | Key::Char('G') => {
            if let Some(o) = &mut ed.outline {
                o.cursor = o.nodes.len().saturating_sub(1);
            }
        }
        Key::Enter | Key::Char('o') | Key::Char('l') => {
            let target = ed
                .outline
                .as_ref()
                .and_then(|o| o.nodes.get(o.cursor))
                .map(|n| (n.line, n.col));
            if let Some((line, col)) = target {
                ed.jump_from_outline(line, col);
            }
        }
        Key::Char('R') => ed.request_language("outline", None),
        Key::Char(':') => ed.enter_command(crate::mode::CommandKind::Ex),
        Key::Char('q') | Key::Esc => ed.toggle_outline(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_nested_document_symbols_with_depth() {
        let v = serde_json::json!([
            {"name":"Foo","kind":5,"range":{"start":{"line":0,"character":0}},
             "children":[
                {"name":"bar","kind":6,"range":{"start":{"line":1,"character":4}}}
             ]},
            {"name":"top_fn","kind":12,"range":{"start":{"line":10,"character":0}}}
        ]);
        let mut nodes = Vec::new();
        flatten(&v, 0, &mut nodes);
        assert_eq!(nodes.len(), 3);
        assert_eq!(
            (nodes[0].name.as_str(), nodes[0].kind, nodes[0].depth),
            ("Foo", "class", 0)
        );
        assert_eq!(
            (nodes[1].name.as_str(), nodes[1].kind, nodes[1].depth),
            ("bar", "method", 1)
        );
        assert_eq!(
            (nodes[2].name.as_str(), nodes[2].depth, nodes[2].line),
            ("top_fn", 0, 10)
        );
    }

    #[test]
    fn flattens_flat_symbol_information() {
        let v = serde_json::json!([
            {"name":"legacy_fn","kind":12,"location":{"range":{"start":{"line":3,"character":2}}}}
        ]);
        let mut nodes = Vec::new();
        flatten(&v, 0, &mut nodes);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].line, 3);
        assert_eq!(nodes[0].col, 2);
    }

    #[test]
    fn caps_depth_and_count() {
        let mut v =
            serde_json::json!({"name":"leaf","kind":12,"range":{"start":{"line":0,"character":0}}});
        for _ in 0..100 {
            v = serde_json::json!([{"name":"wrap","kind":5,"range":{"start":{"line":0,"character":0}},"children":[v]}]);
        }
        let mut nodes = Vec::new();
        flatten(&v, 0, &mut nodes);
        assert!(
            nodes.len() <= 65,
            "depth cap should stop recursion well short of 100 levels"
        );
    }
}
