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
//! as the cursor moves), and no hover preview. Symbol-kind filtering (`f`)
//! is done: it re-derives the displayed `nodes` from `all_nodes`, so it
//! stays a pure view over the last response rather than re-requesting.
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
    /// The full, unfiltered symbol list from the last response. `nodes` is
    /// derived from this by `apply_filter` and is what's actually
    /// rendered/navigated -- keeping both means filtering never has to
    /// re-request symbols, and clearing the filter is lossless.
    pub all_nodes: Vec<SymbolNode>,
    pub nodes: Vec<SymbolNode>,
    pub cursor: usize,
    /// `f` cycles through the kinds present in `all_nodes` (plus "all",
    /// i.e. `None`) and re-derives `nodes` to only that kind.
    pub kind_filter: Option<&'static str>,
}

impl Outline {
    /// Replaces the symbol list after a fresh response, keeping the
    /// current kind filter applied.
    pub fn set_nodes(&mut self, nodes: Vec<SymbolNode>) {
        self.all_nodes = nodes;
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        self.nodes = match self.kind_filter {
            Some(k) => self
                .all_nodes
                .iter()
                .filter(|n| n.kind == k)
                .cloned()
                .collect(),
            None => self.all_nodes.clone(),
        };
        self.cursor = self.cursor.min(self.nodes.len().saturating_sub(1));
    }

    /// Cycles the kind filter forward through the kinds actually present
    /// in `all_nodes`, in first-seen order, wrapping back to "all" (`None`).
    pub fn cycle_kind_filter(&mut self) {
        let mut kinds: Vec<&'static str> = Vec::new();
        for n in &self.all_nodes {
            if !kinds.contains(&n.kind) {
                kinds.push(n.kind);
            }
        }
        if kinds.is_empty() {
            return;
        }
        let next = match self.kind_filter {
            None => kinds.first().copied(),
            Some(k) => match kinds.iter().position(|&x| x == k) {
                Some(i) if i + 1 < kinds.len() => Some(kinds[i + 1]),
                _ => None,
            },
        };
        self.kind_filter = next;
        self.apply_filter();
    }
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
        Key::Char('f') => {
            let label = ed.outline.as_mut().map(|o| {
                o.cycle_kind_filter();
                o.kind_filter.unwrap_or("all")
            });
            if let Some(label) = label {
                ed.set_message(format!("Outline filter: {label}"));
            }
        }
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

    fn node(name: &str, kind: &'static str, line: usize) -> SymbolNode {
        SymbolNode {
            name: name.into(),
            kind,
            line,
            col: 0,
            depth: 0,
        }
    }

    #[test]
    fn cycle_kind_filter_narrows_then_wraps_back_to_all() {
        let mut o = Outline::default();
        o.set_nodes(vec![
            node("Foo", "class", 0),
            node("bar", "method", 1),
            node("baz", "method", 2),
            node("x", "var", 3),
        ]);
        assert_eq!(o.nodes.len(), 4, "no filter yet -- everything shows");

        o.cycle_kind_filter(); // first kind seen: "class"
        assert_eq!(o.kind_filter, Some("class"));
        assert_eq!(o.nodes.len(), 1);
        assert_eq!(o.nodes[0].name, "Foo");

        o.cycle_kind_filter(); // "method"
        assert_eq!(o.kind_filter, Some("method"));
        let names: Vec<_> = o.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["bar", "baz"]);

        o.cycle_kind_filter(); // "var"
        assert_eq!(o.kind_filter, Some("var"));
        assert_eq!(o.nodes.len(), 1);

        o.cycle_kind_filter(); // wraps back to "all"
        assert_eq!(o.kind_filter, None);
        assert_eq!(o.nodes.len(), 4);
    }

    #[test]
    fn set_nodes_reapplies_the_current_filter_to_a_fresh_response() {
        let mut o = Outline::default();
        o.set_nodes(vec![node("a", "fn", 0), node("b", "var", 1)]);
        o.cycle_kind_filter(); // "fn"
        assert_eq!(o.nodes.len(), 1);
        // A refresh (`R`) with a different symbol set must keep the filter.
        o.set_nodes(vec![
            node("c", "fn", 0),
            node("d", "fn", 1),
            node("e", "var", 2),
        ]);
        assert_eq!(o.kind_filter, Some("fn"));
        let names: Vec<_> = o.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["c", "d"]);
    }
}
