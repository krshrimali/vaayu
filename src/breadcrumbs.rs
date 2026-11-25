//! Breadcrumbs: the winbar's `src › render.rs › impl Pane › fn draw` path.
//!
//! Each segment is clickable. A folder opens a dropdown of its entries, the
//! file name a dropdown of the file's top-level declarations, and a symbol
//! a dropdown of its siblings (the declarations in its parent), with the
//! current one selected. Choosing an entry opens the file or jumps to it.
//!
//! The symbol path is cached per buffer line and is only recomputed when the
//! cursor moves to another line or tree-sitter reparses the buffer. While a
//! deferred parse is pending (typing in a large file), the cached path for
//! the line stays on screen rather than being computed from a stale tree.
//! Panes that are not focused show the path their buffer last had on their
//! cursor line, as long as that buffer has not changed since.
use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::menu::{Cmd, Item, Menu};
use crate::syntax::Scope;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

/// Declaration kinds that become breadcrumbs (the sticky-scroll set).
pub(crate) use crate::render::STICKY_KINDS as KINDS;

/// Entries kept before the cache is dropped and rebuilt on demand.
const CACHE_LIMIT: usize = 512;

/// The buffer's `edit_seq` when computed, and the enclosing declarations.
type Entry = (u64, Rc<Vec<Scope>>);

#[derive(Default)]
pub struct Cache {
    /// `(buffer, line)` -> the buffer's `edit_seq` when computed, and the
    /// enclosing declarations.
    map: HashMap<(u64, usize), Entry>,
}

/// The declarations enclosing line `line` of `b`, outermost first, or
/// `None` when nothing is known (no parser for the file, or a pane on a
/// buffer that is not current and has not been seen on that line).
pub fn scopes(ed: &Editor, b: &Buffer, line: usize) -> Option<Rc<Vec<Scope>>> {
    let mut cache = ed.breadcrumbs.borrow_mut();
    if let Some((seq, v)) = cache.map.get(&(b.id, line)) {
        if *seq == b.edit_seq {
            return Some(v.clone());
        }
    }
    let fresh = ed.syntax_seq == Some((b.id, b.edit_seq));
    let syn = ed.syntax.as_ref().filter(|_| fresh);
    let Some(syn) = syn else {
        // Reparse pending: keep what this line showed until it lands.
        return cache.map.get(&(b.id, line)).map(|(_, v)| v.clone());
    };
    let total = b.rope.len_bytes();
    let byte = b.line_byte_range(line).0.min(total);
    let v = Rc::new(syn.scopes_at(byte, KINDS));
    if cache.map.len() >= CACHE_LIMIT {
        cache.map.clear();
    }
    cache.map.insert((b.id, line), (b.edit_seq, v.clone()));
    Some(v)
}

/// What clicking a breadcrumb segment opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The rest of the bar: clicking only focuses the pane.
    Bar,
    /// A folder: a dropdown of its entries.
    Dir(PathBuf),
    /// The declarations inside `parent` (the top level when `None`), with
    /// `current` selected.
    Symbols {
        parent: Option<usize>,
        current: Option<usize>,
    },
}

/// A clickable span of a winbar, recorded by `render::draw_winbar`.
#[derive(Clone, Debug)]
pub struct Hit {
    pub pane: usize,
    pub y: usize,
    pub x0: usize,
    pub x1: usize,
    pub target: Target,
}

/// The winbar segment under `(x, y)`, from the last frame.
pub fn hit_at(ed: &Editor, x: usize, y: usize) -> Option<Hit> {
    let hits = ed.winbar_hits.borrow();
    // Segments are recorded after the whole-bar span: the last wins.
    hits.iter()
        .rfind(|h| h.y == y && x >= h.x0 && x < h.x1)
        .cloned()
}

/// Opens the dropdown for `hit` (its pane is already focused).
pub fn open(ed: &mut Editor, hit: &Hit) {
    let at = (hit.x0, hit.y);
    match &hit.target {
        Target::Bar => {}
        Target::Dir(dir) => {
            let mut entries: Vec<(bool, String, PathBuf)> = match std::fs::read_dir(dir) {
                Ok(rd) => rd
                    .flatten()
                    .map(|e| {
                        let p = e.path();
                        (!p.is_dir(), e.file_name().to_string_lossy().into_owned(), p)
                    })
                    .collect(),
                Err(e) => return ed.set_message(format!("{}: {e}", dir.display())),
            };
            entries.sort_by_key(|e| (e.0, e.1.to_lowercase()));
            let current = ed.buf().path.clone();
            let mut selected = None;
            let items: Vec<Item> = entries
                .into_iter()
                .enumerate()
                .map(|(i, (file, name, path))| {
                    if current.as_ref().is_some_and(|c| c.starts_with(&path)) {
                        selected = Some(i);
                    }
                    if file {
                        Item::new(name, Cmd::Open(path))
                    } else {
                        Item::new(format!("{name}/"), Cmd::Reveal(path))
                    }
                })
                .collect();
            if items.is_empty() {
                return ed.set_message("Empty folder");
            }
            let name = dir.file_name().map_or_else(
                || dir.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            show(ed, Menu::new(items, at).titled(name), selected);
        }
        Target::Symbols { parent, current } => {
            ed.ensure_syntax();
            let Some(syn) = ed.syntax.as_ref() else {
                return ed.set_message("No syntax tree for this file");
            };
            let scopes = syn.child_scopes(*parent, KINDS);
            if scopes.is_empty() {
                return ed.set_message("No symbols");
            }
            let b = ed.buf();
            let total = b.rope.len_bytes();
            let mut selected = None;
            let items: Vec<Item> = scopes
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    if Some(s.start) == *current {
                        selected = Some(i);
                    }
                    let ci = b.rope.byte_to_char(s.start.min(total));
                    let (line, col) = b.pos_from_char_idx(ci);
                    Item::new(s.label.clone(), Cmd::Jump(line, col)).hint((line + 1).to_string())
                })
                .collect();
            show(ed, Menu::new(items, at), selected);
        }
    }
}

fn show(ed: &mut Editor, mut menu: Menu, selected: Option<usize>) {
    menu.selected = selected.or(Some(0));
    // Scroll the current entry into view.
    let rows = ed.terminal_rows.max(1);
    if let Some(i) = menu.selected {
        menu.top = i.saturating_sub(4);
        let r = menu.rect(ed.screen_cols.max(1), rows);
        let vis = r.height.saturating_sub(2).max(1);
        menu.top = menu.top.min(menu.items.len().saturating_sub(vis));
    }
    crate::menu::open(ed, menu);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn rust_editor(text: &str) -> Editor {
        let mut e = Editor::new(Config::default());
        e.buf_mut().path = Some(std::env::temp_dir().join("crumbs.rs"));
        e.buf_mut().rope = ropey::Rope::from_str(text);
        e.ensure_syntax();
        e
    }

    const SRC: &str = "struct A;\n\
impl Widget for Pane {\n\
    fn render(&self) {\n\
        let x = 1;\n\
    }\n\
    fn size(&self) {}\n\
}\n\
fn main() {}\n";

    #[test]
    fn scopes_name_each_enclosing_declaration() {
        let e = rust_editor(SRC);
        let v = scopes(&e, e.buf(), 3).expect("parsed");
        let labels: Vec<_> = v.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["impl Widget for Pane", "fn render"]);
        assert_eq!(v[1].line, 2);
    }

    #[test]
    fn cache_survives_until_the_buffer_changes() {
        let mut e = rust_editor(SRC);
        let a = scopes(&e, e.buf(), 3).unwrap();
        let b = scopes(&e, e.buf(), 3).unwrap();
        assert!(Rc::ptr_eq(&a, &b), "second lookup is a cache hit");
        // An edit with the parse still pending keeps the old answer...
        e.buf_mut().edit_seq += 1;
        let c = scopes(&e, e.buf(), 3).unwrap();
        assert!(Rc::ptr_eq(&a, &c));
        // ...and the reparse replaces it.
        e.ensure_syntax();
        let d = scopes(&e, e.buf(), 3).unwrap();
        assert!(!Rc::ptr_eq(&a, &d));
        assert_eq!(*a, *d);
    }

    #[test]
    fn child_scopes_list_one_level() {
        let e = rust_editor(SRC);
        let syn = e.syntax.as_ref().unwrap();
        let top: Vec<_> = syn
            .child_scopes(None, KINDS)
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert_eq!(top, ["struct A", "impl Widget for Pane", "fn main"]);
        let imp = syn.child_scopes(None, KINDS)[1].start;
        let inner: Vec<_> = syn
            .child_scopes(Some(imp), KINDS)
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert_eq!(inner, ["fn render", "fn size"]);
    }
}
