//! Persistent document outline/symbol sidebar (`,lO`), reusing the same
//! special-pane pattern as the file tree and terminal (`Window::outline`).
//! Populated from the same `textDocument/documentSymbol` response
//! `,lo`/`:outline` already requests (see `language.rs`), but read as a
//! hierarchy (via `children`) instead of flattened into a plain list, so
//! nesting depth survives into the sidebar's indentation.
//!
//! Live follow-cursor (highlighting the enclosing symbol as the cursor
//! moves) is done -- see `Editor::ensure_outline_follow`/
//! `Outline::sync_to_line`. Hover preview (`K`, without navigating) is
//! done -- see `Editor::hover_outline_symbol`. Collapse/expand (`h`
//! collapses, `l` expands a collapsed node or jumps if it has no children)
//! and symbol-kind filtering (`f`) are both done: both re-derive the
//! displayed `nodes` from `all_nodes` (`SymbolNode` has no explicit
//! parent/child links, so "descendant of a collapsed node" is inferred
//! from `depth` while walking the flat, depth-sorted list), so they stay a
//! pure view over the last response rather than re-requesting.
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
    /// Identifies a collapsed node by (name, line) -- `SymbolNode` has no
    /// stable id, and this survives a same-content refresh well enough.
    pub collapsed: std::collections::BTreeSet<(String, usize)>,
}

impl Outline {
    /// Replaces the symbol list after a fresh response, keeping the
    /// current kind filter and collapsed set applied.
    pub fn set_nodes(&mut self, nodes: Vec<SymbolNode>) {
        self.all_nodes = nodes;
        self.apply_filter();
    }

    fn key(n: &SymbolNode) -> (String, usize) {
        (n.name.clone(), n.line)
    }

    /// `all_nodes` is a flat, depth-sorted (pre-order) list with no
    /// explicit child links, so "is this node inside a collapsed one" is
    /// inferred by skipping any run of nodes deeper than the nearest
    /// preceding collapsed node, until depth returns to that level or above.
    fn visible_after_collapse(&self) -> Vec<SymbolNode> {
        let mut out = Vec::new();
        let mut skip_below: Option<usize> = None;
        for n in &self.all_nodes {
            if let Some(d) = skip_below {
                if n.depth > d {
                    continue;
                }
                skip_below = None;
            }
            if self.collapsed.contains(&Self::key(n)) {
                skip_below = Some(n.depth);
            }
            out.push(n.clone());
        }
        out
    }

    pub fn has_children(&self, n: &SymbolNode) -> bool {
        self.all_nodes
            .iter()
            .position(|x| Self::key(x) == Self::key(n))
            .and_then(|i| self.all_nodes.get(i + 1))
            .is_some_and(|next| next.depth > n.depth)
    }

    fn apply_filter(&mut self) {
        let base = self.visible_after_collapse();
        self.nodes = match self.kind_filter {
            Some(k) => base.into_iter().filter(|n| n.kind == k).collect(),
            None => base,
        };
        self.cursor = self.cursor.min(self.nodes.len().saturating_sub(1));
    }

    /// `h`: collapses the node under the cursor, if it has children and
    /// isn't already collapsed. A no-op otherwise (no jump-to-parent --
    /// unlike the file tree, every symbol is always in view already).
    pub fn collapse(&mut self) {
        let Some(cur) = self.nodes.get(self.cursor).cloned() else {
            return;
        };
        if !self.has_children(&cur) {
            return;
        }
        self.collapsed.insert(Self::key(&cur));
        self.apply_filter();
        if let Some(i) = self
            .nodes
            .iter()
            .position(|n| Self::key(n) == Self::key(&cur))
        {
            self.cursor = i;
        }
    }

    /// `l`: expands the node under the cursor if it's collapsed; returns
    /// `false` (so the caller falls back to jump-to-symbol) otherwise.
    pub fn expand(&mut self) -> bool {
        let Some(cur) = self.nodes.get(self.cursor).cloned() else {
            return false;
        };
        let key = Self::key(&cur);
        if !self.collapsed.remove(&key) {
            return false;
        }
        self.apply_filter();
        if let Some(i) = self.nodes.iter().position(|n| Self::key(n) == key) {
            self.cursor = i;
        }
        true
    }

    /// Follow-cursor: moves the sidebar's cursor to the symbol that
    /// encloses `line`, without re-requesting anything. `nodes` has no
    /// end-line/range (only a start position), so "encloses" is
    /// approximated the same way aerial.nvim's simple heuristic does: the
    /// nearest symbol whose start line is `<= line`. That's exact for a
    /// pre-order, depth-sorted symbol list -- a nested function's own
    /// start line is always the closest preceding one for any line inside
    /// it, since its parent's next sibling (if any) only starts after all
    /// of the parent's descendants. A no-op if no symbol starts at or
    /// before `line` (cursor above the first symbol) or the list is empty.
    pub fn sync_to_line(&mut self, line: usize) {
        if let Some(i) = self.nodes.iter().rposition(|n| n.line <= line) {
            self.cursor = i;
        }
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

    /// Per-frame follow-cursor: while the outline sidebar is open but not
    /// itself focused (the user is editing/moving in the buffer pane),
    /// keeps the sidebar's cursor on the symbol enclosing the buffer's
    /// cursor line. Skipped while the outline pane itself has focus, so
    /// manual `j`/`k`/collapse navigation there is never clobbered.
    /// Cheap no-op whenever there's no outline sidebar open at all (the
    /// overwhelmingly common case), so this costs nothing on the hot
    /// per-frame path for buffers that never opened one.
    pub fn ensure_outline_follow(&mut self) {
        if self.outline.is_none() || self.active_outline() {
            return;
        }
        if self.buf().path.as_ref() != self.outline.as_ref().unwrap().buffer_path.as_ref() {
            return;
        }
        let line = self.cursor().0;
        self.outline.as_mut().unwrap().sync_to_line(line);
    }

    /// `K`: hover for the symbol under the outline cursor -- a "preview
    /// without navigating" (Phase 2 item 7's outline "preview" gap),
    /// distinct from `Enter`/`l`/`o`'s actual jump. `request_language`
    /// always reads the position from `self.cursor()`, so this briefly
    /// moves the buffer's real cursor there, fires the request (which
    /// embeds that position in the outgoing JSON synchronously, before
    /// this function returns), and restores it immediately -- the
    /// response arrives later and doesn't depend on where the cursor
    /// ends up, so this never disturbs the user's actual editing
    /// position. A no-op if the outline is showing a different
    /// document than the one currently open (stale after a buffer
    /// switch), so it never hovers the wrong file's position.
    pub fn hover_outline_symbol(&mut self) {
        let Some(o) = &self.outline else { return };
        if o.buffer_path.as_ref() != self.buf().path.as_ref() {
            return;
        }
        let Some((line, col)) = o.nodes.get(o.cursor).map(|n| (n.line, n.col)) else {
            return;
        };
        let saved = self.cursor();
        self.set_cursor(line, col);
        self.request_hover();
        self.set_cursor(saved.0, saved.1);
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
        Key::Char('l') => {
            let expanded = ed.outline.as_mut().is_some_and(Outline::expand);
            if expanded {
                return;
            }
            let target = ed
                .outline
                .as_ref()
                .and_then(|o| o.nodes.get(o.cursor))
                .map(|n| (n.line, n.col));
            if let Some((line, col)) = target {
                ed.jump_from_outline(line, col);
            }
        }
        Key::Char('h') => {
            if let Some(o) = &mut ed.outline {
                o.collapse();
            }
        }
        Key::Enter | Key::Char('o') => {
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
        Key::Char('K') => ed.hover_outline_symbol(),
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
        node_at(name, kind, line, 0)
    }

    fn node_at(name: &str, kind: &'static str, line: usize, depth: usize) -> SymbolNode {
        SymbolNode {
            name: name.into(),
            kind,
            line,
            col: 0,
            depth,
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

    #[test]
    fn collapse_hides_descendants_and_expand_restores_them() {
        let mut o = Outline::default();
        // Foo (depth 0)
        //   bar (depth 1)
        //     baz (depth 2)
        // Sibling (depth 0)
        o.set_nodes(vec![
            node_at("Foo", "class", 0, 0),
            node_at("bar", "method", 1, 1),
            node_at("baz", "method", 2, 2),
            node_at("Sibling", "class", 3, 0),
        ]);
        assert_eq!(o.nodes.len(), 4);
        o.cursor = 0; // on Foo
        o.collapse();
        let names: Vec<_> = o.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Foo", "Sibling"],
            "collapsing Foo should hide bar and baz but not Sibling"
        );
        assert_eq!(o.cursor, 0, "cursor should stay on Foo after collapsing");

        o.cursor = 1; // Sibling, a leaf with no children
        assert!(
            !o.expand(),
            "l on a leaf (Sibling) with no children is not an expand"
        );
        o.cursor = 0; // back on Foo, which is now collapsed
        assert!(o.expand(), "l on a collapsed node should expand it");
        let names: Vec<_> = o.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["Foo", "bar", "baz", "Sibling"]);
    }

    #[test]
    fn sync_to_line_selects_the_nearest_preceding_symbol() {
        let mut o = Outline::default();
        // Foo (line 0)
        //   bar (line 2)
        // top_fn (line 10)
        o.set_nodes(vec![
            node_at("Foo", "class", 0, 0),
            node_at("bar", "method", 2, 1),
            node_at("top_fn", "fn", 10, 0),
        ]);
        o.sync_to_line(3); // inside bar's body
        assert_eq!(o.nodes[o.cursor].name, "bar");
        o.sync_to_line(1); // inside Foo but above bar
        assert_eq!(o.nodes[o.cursor].name, "Foo");
        o.sync_to_line(20); // past every symbol, still inside top_fn
        assert_eq!(o.nodes[o.cursor].name, "top_fn");
    }

    #[test]
    fn sync_to_line_is_a_noop_above_every_symbol() {
        let mut o = Outline::default();
        o.set_nodes(vec![node_at("Foo", "class", 5, 0)]);
        o.cursor = 0;
        o.sync_to_line(0); // above the first symbol's start line
        assert_eq!(
            o.cursor, 0,
            "nothing precedes line 0, so cursor is untouched"
        );
    }

    #[test]
    fn collapse_is_a_noop_on_a_leaf_node() {
        let mut o = Outline::default();
        o.set_nodes(vec![node("leaf", "fn", 0)]);
        o.collapse();
        assert_eq!(
            o.nodes.len(),
            1,
            "collapsing a childless node changes nothing"
        );
        assert!(o.collapsed.is_empty());
    }
}
