use crate::{buffer::Buffer, config::Config, editor::Editor, key::Key, mode::Mode};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(1);
fn temp() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "vaayu-test-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn editor(text: &str) -> Editor {
    let cfg = Config {
        clipboard_unnamedplus: false,
        jk_escape: false,
        ..Config::default()
    };
    let mut e = Editor::new(cfg);
    e.buf_mut().rope = ropey::Rope::from_str(text);
    e.buf_mut().mark_saved();
    e
}
fn keys(e: &mut Editor, s: &str) {
    for c in s.chars() {
        e.feed_key(match c {
            '\x1b' => Key::Esc,
            '\n' => Key::Enter,
            _ => Key::Char(c),
        });
    }
}
#[test]
fn exit_protects_hidden_and_notes() {
    let mut e = editor("abc\n");
    keys(&mut e, "x");
    e.buffers.push(Buffer::empty());
    e.cur = 1;
    keys(&mut e, ":q\n");
    assert!(!e.should_quit);
    keys(&mut e, ":qa\n");
    assert!(!e.should_quit);
    e.buffers.remove(0);
    e.cur = 0;
    e.notes.dirty = true;
    keys(&mut e, ":qa\n");
    assert!(!e.should_quit);
}
#[test]
fn failed_save_all_stays_open() {
    let mut e = editor("abc\n");
    keys(&mut e, "x:wqa\n");
    assert!(!e.should_quit);
}
#[test]
fn undo_saved_state_and_revision() {
    let mut e = editor("abc\n");
    keys(&mut e, "x");
    e.buf_mut().mark_saved();
    let seq = e.buf().edit_seq;
    keys(&mut e, "u");
    assert!(e.buf().is_modified());
    assert!(e.buf().edit_seq > seq);
    e.feed_key(Key::Ctrl('r'));
    assert!(!e.buf().is_modified());
}
#[test]
fn insert_revision_noop_and_redo() {
    let mut e = editor("abc\n");
    keys(&mut e, "xui\x1b");
    assert!(!e.buf().is_modified());
    e.feed_key(Key::Ctrl('r'));
    assert_eq!(e.buf().line_text(0), "bc");
    let seq = e.buf().edit_seq;
    keys(&mut e, "iZ");
    assert!(e.buf().edit_seq > seq);
    let typed_seq = e.buf().edit_seq;
    keys(&mut e, "\x1b");
    assert_eq!(e.buf().edit_seq, typed_seq);
}
#[test]
fn blackhole_and_append() {
    let mut e = editor("abc\n");
    e.registers.set(None, "keep".into(), false);
    keys(&mut e, "\"_x");
    assert_eq!(e.registers.get(None).unwrap().text, "keep");
    e.registers.set(Some('a'), "one".into(), false);
    e.registers.set(Some('A'), "two".into(), false);
    assert_eq!(e.registers.get(Some('a')).unwrap().text, "onetwo");
}
#[test]
fn leader_delete() {
    let mut e = editor("abc\n");
    keys(&mut e, ",dw");
    assert!(e.buf().line_text(0).is_empty());
}
#[test]
fn command_history_cycles_with_up_down_and_restores_draft() {
    let mut e = editor("a\n");
    keys(&mut e, ":set wrap\n");
    keys(&mut e, ":set number\n");
    keys(&mut e, ":partial");
    e.feed_key(Key::Up);
    assert_eq!(e.cmdline, "set number");
    e.feed_key(Key::Up);
    assert_eq!(e.cmdline, "set wrap");
    e.feed_key(Key::Up); // already at the oldest entry: stays put
    assert_eq!(e.cmdline, "set wrap");
    e.feed_key(Key::Down);
    assert_eq!(e.cmdline, "set number");
    e.feed_key(Key::Down); // past the newest entry: restores the draft
    assert_eq!(e.cmdline, "partial");
}
#[test]
fn search_history_is_separate_from_command_history() {
    let mut e = editor("needle haystack\n");
    keys(&mut e, "/needle\n");
    keys(&mut e, ":set wrap\n");
    keys(&mut e, "/");
    e.feed_key(Key::Up);
    assert_eq!(e.cmdline, "needle");
    e.feed_key(Key::Esc);
    keys(&mut e, ":");
    e.feed_key(Key::Up);
    assert_eq!(e.cmdline, "set wrap");
}
#[test]
fn repeated_identical_commands_do_not_duplicate_in_history() {
    let mut e = editor("a\n");
    keys(&mut e, ":set wrap\n");
    keys(&mut e, ":set wrap\n");
    assert_eq!(e.command_history, vec!["set wrap".to_string()]);
}
#[test]
fn jumps_command_lists_jumplist_and_navigates_to_entries() {
    let root = temp();
    let a = root.join("a.txt");
    std::fs::write(&a, "one\ntwo\nthree\nfour\nfive\n").unwrap();
    let mut e = editor("");
    e.open_file(a).unwrap();
    e.set_cursor(0, 0);
    e.push_jump();
    e.set_cursor(3, 0);
    e.push_jump();
    keys(&mut e, ":jumps\n");
    let r = e
        .results
        .as_ref()
        .expect("jumps should open a results list");
    assert_eq!(r.entries.len(), 2);
    assert!(r.entries[0].text.contains(":1:1"));
    assert!(r.entries[1].text.contains(":4:1"));
    e.results.as_mut().unwrap().cursor = 0;
    e.open_result();
    assert_eq!(e.cursor(), (0, 0));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn history_command_lists_and_reruns_a_past_command() {
    let mut e = editor("a\n");
    keys(&mut e, ":set wrap\n");
    keys(&mut e, ":set nowrap\n");
    assert!(!e.config.wrap);
    keys(&mut e, ":chistory\n");
    let r = e.results.as_ref().unwrap();
    // Most recent first -- :chistory's own invocation is now the newest
    // entry, same as a shell's `history` command showing itself.
    assert_eq!(r.entries[0].text, ":chistory");
    assert_eq!(r.entries[1].text, ":set nowrap");
    assert_eq!(r.entries[2].text, ":set wrap");
    let idx = r
        .entries
        .iter()
        .position(|e| e.text == ":set wrap")
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    assert!(e.config.wrap, "selecting a history entry must rerun it");
}
#[test]
fn shistory_command_lists_and_reruns_a_past_search() {
    let mut e = editor("alpha\nneedle\nbeta\n");
    keys(&mut e, "/needle\n");
    keys(&mut e, "gg"); // back to the top before rerunning
    keys(&mut e, ":shistory\n");
    let r = e.results.as_ref().unwrap();
    assert_eq!(r.entries[0].text, "/needle");
    e.results.as_mut().unwrap().cursor = 0;
    e.open_result();
    assert_eq!(e.cursor().0, 1, "rerunning the search must jump to needle");
}
#[test]
fn search_and_search_next_recenter_the_viewport_on_the_match() {
    let mut text = String::new();
    for i in 0..60 {
        text.push_str(if i == 49 { "needle\n" } else { "line\n" });
    }
    let mut e = editor(&text);
    let rows = e.screen_rows;
    let last = e.buf().line_count() - 1;
    let max_top = last.saturating_sub(rows.saturating_sub(1));
    let expected_top = 49usize.saturating_sub(rows / 2).min(max_top);
    keys(&mut e, "/needle\n");
    assert_eq!(e.cursor().0, 49);
    assert_eq!(
        e.buf().top_line,
        expected_top,
        "/ search should center the match in the viewport"
    );
    e.buf_mut().top_line = 0; // disturb it, then prove `n` recenters again
    keys(&mut e, "gg");
    keys(&mut e, "/needle\n");
    e.buf_mut().top_line = 0;
    keys(&mut e, "n");
    assert_eq!(e.cursor().0, 49);
    assert_eq!(
        e.buf().top_line,
        expected_top,
        "n should recenter the match in the viewport"
    );
}
#[test]
fn ctrl_6_toggles_to_the_alternate_buffer() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "file a\n").unwrap();
    std::fs::write(&b, "file b\n").unwrap();
    let mut e = editor("");
    e.open_file(a.clone()).unwrap();
    e.open_file(b.clone()).unwrap();
    assert_eq!(e.buf().path.as_deref(), Some(b.as_path()));
    e.feed_key(Key::Ctrl('6'));
    assert_eq!(
        e.buf().path.as_deref(),
        Some(a.as_path()),
        "Ctrl-6 should switch to the alternate buffer"
    );
    e.feed_key(Key::Ctrl('6'));
    assert_eq!(
        e.buf().path.as_deref(),
        Some(b.as_path()),
        "Ctrl-6 again should toggle back"
    );
    keys(&mut e, ":b#\n");
    assert_eq!(
        e.buf().path.as_deref(),
        Some(a.as_path()),
        ":b# should also toggle to the alternate buffer"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn buffers_list_orders_by_most_recently_activated_first() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    let c = root.join("c.txt");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    std::fs::write(&c, "c\n").unwrap();
    let mut e = editor("");
    e.open_file(a.clone()).unwrap();
    e.open_file(b.clone()).unwrap();
    e.open_file(c.clone()).unwrap();
    // Opened in order a, b, c (c now current) -- switch back to a, making
    // the activation order c, a (most recent first: a, c, b).
    e.open_file(a.clone()).unwrap();
    e.show_buffers();
    let names: Vec<_> = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(
        names,
        vec![Some(a.clone()), Some(c.clone()), Some(b.clone())],
        "buffer list should be most-recently-activated first, not insertion order"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn closing_a_buffer_removes_it_from_the_mru_list() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let mut e = editor("");
    e.open_file(a.clone()).unwrap();
    e.open_file(b.clone()).unwrap();
    let a_id = e
        .buffers
        .iter()
        .find(|buf| buf.path == Some(a.clone()))
        .unwrap()
        .id;
    assert!(e.buffer_mru.contains(&a_id));
    keys(&mut e, ":bp\n"); // switch to a.txt so it isn't "current" when deleted... actually delete b (current)
    keys(&mut e, ":bn\n"); // back to b.txt
    keys(&mut e, ":bd\n"); // delete current buffer (b.txt)
    assert!(
        !e.buffer_mru
            .iter()
            .any(|id| e.buffers.iter().all(|buf| buf.id != *id)),
        "buffer_mru must not retain ids for buffers that no longer exist"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn blines_lists_nonblank_lines_and_enter_jumps_to_one() {
    let mut e = editor("first\n\nsecond needle\n   \nthird\n");
    keys(&mut e, ":blines\n");
    let r = e
        .results
        .as_ref()
        .expect("blines should open a results list");
    // Two blank/whitespace-only lines (indices 1 and 3) are skipped.
    assert_eq!(r.entries.len(), 3);
    let idx = r
        .entries
        .iter()
        .position(|e| e.text.contains("second needle"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    assert_eq!(e.cursor().0, 2, "selecting a line entry must jump there");
}
#[test]
fn ctrl_v_opens_a_result_location_into_a_vertical_split() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "current\n").unwrap();
    std::fs::write(&b, "target\n").unwrap();
    let mut e = editor("");
    e.open_file(a.clone()).unwrap();
    let entries = vec![crate::results::Entry::location(b.clone(), 0, 0, "target")];
    e.show_results(crate::results::Results::new("Test", entries));
    e.feed_key(Key::Ctrl('v'));
    assert_eq!(e.windows.len(), 2, "Ctrl-v must open a new split");
    assert_eq!(e.buf().path, Some(b));
    assert_eq!(crate::mode::Mode::Normal, e.mode);
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn ctrl_v_in_file_picker_opens_into_a_split() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "current\n").unwrap();
    std::fs::write(&b, "target\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(a.clone()).unwrap();
    e.all_files = vec![b.display().to_string()];
    e.open_picker();
    keys(&mut e, "b");
    e.feed_key(Key::Ctrl('v'));
    assert_eq!(e.windows.len(), 2, "Ctrl-v in the picker must open a split");
    assert_eq!(e.buf().path, Some(b));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn ctrl_t_opens_a_result_location_into_a_new_tab() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "current\n").unwrap();
    std::fs::write(&b, "target\n").unwrap();
    let mut e = editor("");
    e.open_file(a.clone()).unwrap();
    let entries = vec![crate::results::Entry::location(b.clone(), 0, 0, "target")];
    e.show_results(crate::results::Results::new("Test", entries));
    e.feed_key(Key::Ctrl('t'));
    assert_eq!(e.tabs.len(), 2, "Ctrl-t must open a new tab");
    assert!(
        e.windows.is_empty(),
        "the new tab should not also split (no split_window call)"
    );
    assert_eq!(e.buf().path, Some(b));
    assert_eq!(crate::mode::Mode::Normal, e.mode);
    keys(&mut e, "gT");
    assert_eq!(e.buf().path, Some(a), "the original tab must keep a.txt");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn ctrl_t_in_file_picker_opens_into_a_new_tab() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "current\n").unwrap();
    std::fs::write(&b, "target\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(a.clone()).unwrap();
    e.all_files = vec![b.display().to_string()];
    e.open_picker();
    keys(&mut e, "b");
    e.feed_key(Key::Ctrl('t'));
    assert_eq!(e.tabs.len(), 2, "Ctrl-t in the picker must open a new tab");
    assert_eq!(e.buf().path, Some(b));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn resume_reopens_the_last_dismissed_file_picker_with_its_state_intact() {
    let root = temp();
    let a = root.join("a.txt");
    std::fs::write(&a, "x\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(a).unwrap();
    e.all_files = vec!["foo.rs".into(), "bar.rs".into()];
    e.open_picker();
    keys(&mut e, "foo");
    assert_eq!(e.file_picker.as_ref().unwrap().query, "foo");
    e.feed_key(Key::Esc);
    assert!(e.file_picker.is_none(), "Esc should dismiss the picker");
    assert!(!matches!(e.mode, Mode::Picker));
    keys(&mut e, ":resume\n");
    assert!(
        matches!(e.mode, Mode::Picker),
        ":resume should reopen the picker"
    );
    assert_eq!(
        e.file_picker.as_ref().unwrap().query,
        "foo",
        "resumed picker should keep its query"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn file_picker_preview_toggle_renders_the_selected_files_content() {
    let root = temp();
    let a = root.join("a.txt");
    std::fs::write(&a, "alpha\n").unwrap();
    let b = root.join("b.txt");
    std::fs::write(&b, "beta content here\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.all_files = vec!["a.txt".into(), "b.txt".into()];
    e.open_picker();
    assert!(
        !e.file_picker.as_ref().unwrap().preview,
        "preview starts off"
    );

    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 60, 20);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 20, &mut cache).unwrap();
    assert!(
        !String::from_utf8_lossy(&out).contains("alpha"),
        "no preview content before toggling it on"
    );

    e.feed_key(Key::Ctrl('r'));
    assert!(e.file_picker.as_ref().unwrap().preview);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 20, &mut cache).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("alpha"),
        "the first (selected) match's content should render. Got: {text:?}"
    );
    assert!(!text.contains("beta content here"));

    // Ctrl-r must not be swallowed as a query character -- it's a
    // control key, not a printable one, so the query is untouched.
    assert_eq!(e.file_picker.as_ref().unwrap().query, "");

    e.feed_key(Key::Down);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 20, &mut cache).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("beta content here"),
        "moving the selection should preview the newly-selected file. Got: {text:?}"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn preview_source_lines_refuses_a_file_larger_than_the_cap() {
    let root = temp();
    let big = root.join("big.log");
    // A sparse file: `set_len` reports the target size in metadata
    // without actually writing that many bytes to disk, so this stays
    // fast regardless of the cap's exact size.
    std::fs::File::create(&big)
        .unwrap()
        .set_len(16 * 1024 * 1024)
        .unwrap();
    let e = editor("");
    let source = e.preview_source_lines(&big);
    assert_eq!(source, vec!["(file too large to preview)".to_string()]);
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn resume_reopens_the_last_dismissed_results_list_with_its_state_intact() {
    let mut e = editor("a\nb\nc\n");
    let entries = vec![
        crate::results::Entry::text("one"),
        crate::results::Entry::text("two"),
    ];
    e.show_results(crate::results::Results::new("Test", entries));
    e.results.as_mut().unwrap().cursor = 1;
    keys(&mut e, "q"); // dismiss
    assert!(!matches!(e.mode, Mode::Results));
    keys(&mut e, ":resume\n");
    assert!(
        matches!(e.mode, Mode::Results),
        ":resume should reopen the results list"
    );
    assert_eq!(
        e.results.as_ref().unwrap().cursor,
        1,
        "resumed results should keep the cursor position"
    );
}
#[test]
fn resume_prefers_whichever_of_picker_or_results_was_dismissed_more_recently() {
    let mut e = editor("a\n");
    e.all_files = vec!["f.rs".into()];
    e.open_picker();
    e.feed_key(Key::Esc); // dismiss the picker first
    let entries = vec![crate::results::Entry::text("one")];
    e.show_results(crate::results::Results::new("Test", entries));
    keys(&mut e, "q"); // dismiss results -- the more recent dismissal
    keys(&mut e, ":resume\n");
    assert!(
        matches!(e.mode, Mode::Results),
        "resume should prefer the more recently dismissed session"
    );
}
#[test]
fn colder_and_cnewer_navigate_quickfix_history() {
    let mut e = editor("a\n");
    e.show_results(crate::results::Results::new(
        "First",
        vec![crate::results::Entry::text("one")],
    ));
    e.export_quickfix(); // history: [First]
    e.show_results(crate::results::Results::new(
        "Second",
        vec![
            crate::results::Entry::text("two-a"),
            crate::results::Entry::text("two-b"),
        ],
    ));
    e.export_quickfix(); // history: [First, Second], pos=1

    assert_eq!(e.quickfix.as_ref().unwrap().title, "Second");
    keys(&mut e, ":colder\n");
    assert_eq!(
        e.quickfix.as_ref().unwrap().title,
        "First",
        ":colder should switch to the older list"
    );
    keys(&mut e, ":colder\n");
    assert_eq!(
        e.quickfix.as_ref().unwrap().title,
        "First",
        "already at the oldest -- :colder again must be a no-op, not panic"
    );
    keys(&mut e, ":cnewer\n");
    assert_eq!(
        e.quickfix.as_ref().unwrap().title,
        "Second",
        ":cnewer should switch back to the newer list"
    );
    keys(&mut e, ":cnewer\n");
    assert_eq!(
        e.quickfix.as_ref().unwrap().title,
        "Second",
        "already at the newest -- :cnewer again must be a no-op, not panic"
    );
}
#[test]
fn a_new_quickfix_list_discards_forward_history_from_the_current_point() {
    let mut e = editor("a\n");
    e.show_results(crate::results::Results::new("A", vec![]));
    e.export_quickfix();
    e.show_results(crate::results::Results::new("B", vec![]));
    e.export_quickfix();
    e.show_results(crate::results::Results::new("C", vec![]));
    e.export_quickfix(); // history: [A, B, C], pos=2
    keys(&mut e, ":colder\n"); // pos=1 (B)
    keys(&mut e, ":colder\n"); // pos=0 (A)
    e.show_results(crate::results::Results::new("D", vec![]));
    e.export_quickfix(); // history should become [A, D], discarding B and C
    assert_eq!(e.quickfix.as_ref().unwrap().title, "D");
    keys(&mut e, ":colder\n");
    assert_eq!(
        e.quickfix.as_ref().unwrap().title,
        "A",
        "B and C should have been discarded once a new list was created from an older point"
    );
}
#[test]
fn dismissing_the_current_quickfix_list_updates_history_in_place_not_a_new_entry() {
    let mut e = editor("a\n");
    let mut r = crate::results::Results::new("Only", vec![crate::results::Entry::text("x")]);
    r.quickfix = true;
    e.show_results(r);
    e.export_quickfix();
    // Move the cursor and dismiss -- this must update the existing
    // history slot, not grow history (remember_results, not export).
    e.results.as_mut().unwrap().cursor = 0;
    keys(&mut e, "q");
    assert_eq!(
        e.quickfix_history.len(),
        1,
        "dismissing the current list must not grow history"
    );
    keys(&mut e, ":colder\n");
    assert_eq!(
        e.quickfix.as_ref().unwrap().title,
        "Only",
        ":colder should have nothing older -- dismissing must not have grown history"
    );
}
#[test]
fn file_tree_diagnostic_marker_reflects_worst_severity_and_reaches_unexpanded_dirs() {
    let root = temp();
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let clean = root.join("clean.txt");
    let warned = root.join("warned.txt");
    let errored = sub.join("errored.txt");
    std::fs::write(&clean, "x").unwrap();
    std::fs::write(&warned, "x").unwrap();
    std::fs::write(&errored, "x").unwrap();
    let e = editor("");
    let diag = |severity| crate::lsp::Diagnostic {
        line: 0,
        col: 0,
        end_line: 0,
        end_col: 1,
        severity,
        message: "fixture".into(),
        raw: serde_json::json!({}),
    };
    let mut e = e;
    e.diagnostics
        .insert(warned.clone(), vec![diag(crate::lsp::Severity::Warning)]);
    e.diagnostics
        .insert(errored.clone(), vec![diag(crate::lsp::Severity::Error)]);

    assert_eq!(
        crate::render::tree_diagnostic_marker(&e, &clean, false),
        None,
        "a file with no diagnostics gets no marker"
    );
    assert_eq!(
        crate::render::tree_diagnostic_marker(&e, &warned, false),
        Some('W')
    );
    assert_eq!(
        crate::render::tree_diagnostic_marker(&e, &errored, false),
        Some('E')
    );
    // sub/ was never expanded (diagnostics are keyed by full path
    // regardless of what the lazily-built tree has loaded), but its
    // descendant's error should still surface on the directory itself.
    assert_eq!(
        crate::render::tree_diagnostic_marker(&e, &sub, true),
        Some('E')
    );
    // The project root contains both a warning and an error -- Error wins.
    assert_eq!(
        crate::render::tree_diagnostic_marker(&e, &root, true),
        Some('E')
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn results_filter_narrows_the_list_via_real_keys_and_f_clears_it() {
    let mut e = editor("");
    keys(&mut e, ":commands\n");
    let total = e.results.as_ref().unwrap().entries.len();

    keys(&mut e, "f");
    assert!(e.results.as_ref().unwrap().filter_input);
    keys(&mut e, "gitblame");
    let r = e.results.as_ref().unwrap();
    assert_eq!(r.filter, "gitblame");
    assert!(
        r.entries.iter().all(|en| en.text.contains("gitblame")),
        "every remaining entry should match the filter"
    );
    assert!(r.entries.len() < total, "the list should have narrowed");
    let narrowed = r.entries.len();

    keys(&mut e, "\n"); // Enter leaves filter-input, keeping the filter
    assert!(!e.results.as_ref().unwrap().filter_input);
    assert_eq!(e.results.as_ref().unwrap().entries.len(), narrowed);

    // A fresh 'f' clears the previous filter before editing the new one.
    keys(&mut e, "f");
    assert_eq!(e.results.as_ref().unwrap().filter, "");
    assert_eq!(e.results.as_ref().unwrap().entries.len(), total);
    keys(&mut e, "\n");
}
#[test]
fn commands_lists_every_entry_sorted_and_tagged_for_prefill() {
    let mut e = editor("");
    keys(&mut e, ":commands\n");
    let r = e
        .results
        .as_ref()
        .expect(":commands should open a results list");
    assert_eq!(r.entries.len(), crate::command::EX_COMMANDS.len());
    let texts: Vec<&str> = r.entries.iter().map(|en| en.text.as_str()).collect();
    let mut sorted = texts.clone();
    sorted.sort();
    assert_eq!(texts, sorted, "entries should be sorted alphabetically");
    assert!(texts.iter().any(|t| t.starts_with(":grep")));
    let grep_entry = r
        .entries
        .iter()
        .find(|en| en.text.starts_with(":grep "))
        .unwrap();
    assert_eq!(
        grep_entry.action.as_ref().unwrap()["_vaayu_prefill_ex"],
        "grep "
    );
}
#[test]
fn everything_combines_keymaps_commands_and_projects() {
    let mut e = editor("");
    keys(&mut e, ":everything\n");
    let r = e
        .results
        .as_ref()
        .expect(":everything should open a results list");
    let keymap_count = r
        .entries
        .iter()
        .filter(|en| en.text.starts_with("[keymap]"))
        .count();
    let command_count = r
        .entries
        .iter()
        .filter(|en| en.text.starts_with("[command]"))
        .count();
    assert_eq!(keymap_count, crate::actions::ACTIONS.len());
    assert_eq!(command_count, crate::command::EX_COMMANDS.len());
    // Reuses the exact same action tags :keymaps/:commands already use,
    // so dispatch itself needed no new code -- spot-check one of each.
    let grep_entry = r
        .entries
        .iter()
        .find(|en| en.text.contains(":grep "))
        .unwrap();
    assert_eq!(
        grep_entry.action.as_ref().unwrap()["_vaayu_prefill_ex"],
        "grep "
    );
    let keymap_entry = r
        .entries
        .iter()
        .find(|en| en.text.starts_with("[keymap]"))
        .unwrap();
    assert!(keymap_entry.action.as_ref().unwrap()["_vaayu_action_id"].is_string());
    // Every project entry (whatever the real recent-projects file
    // happens to contain on this machine) must be tagged correctly and
    // must never include the current project_root.
    for en in r
        .entries
        .iter()
        .filter(|en| en.text.starts_with("[project]"))
    {
        let tagged = en.action.as_ref().unwrap()["_vaayu_switch_project"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(std::path::PathBuf::from(tagged), e.project_root);
    }
}
#[test]
fn commands_enter_prefills_the_command_line_without_running_it() {
    let mut e = editor("needle\n");
    keys(&mut e, ":commands\n");
    // Move the cursor to the ":grep" entry (order is alphabetical, so its
    // exact position could shift if the list changes -- find it by text
    // instead of hardcoding an index).
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.starts_with(":grep "))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    assert!(
        matches!(
            e.mode,
            crate::mode::Mode::Command(crate::mode::CommandKind::Ex)
        ),
        "selecting a command should open the command line, not run it"
    );
    assert_eq!(e.cmdline, "grep ");
    assert!(
        e.results.is_none() || !e.results.as_ref().unwrap().live,
        "the command must not have actually run yet"
    );
    // Finish it like a real user would: add the pattern and press Enter.
    keys(&mut e, "needle\n");
    let r = e.results.as_ref().expect("grep should now have run");
    assert!(r.live);
    assert_eq!(r.query, "needle");
}
#[test]
fn grep_word_under_cursor_opens_live_grep_with_that_word() {
    let mut e = editor("needle in a haystack\n");
    keys(&mut e, ",gw");
    let r = e.results.as_ref().expect("grep should open a results list");
    assert!(r.live);
    assert_eq!(r.query, "needle");
}
#[test]
fn grep_visual_selection_opens_live_grep_with_the_selected_text() {
    let mut e = editor("hello world\n");
    keys(&mut e, "vlll"); // select "hell" (v on 'h', 3x l -> through 'l')
    keys(&mut e, ",gw");
    let r = e.results.as_ref().expect("grep should open a results list");
    assert_eq!(r.query, "hell");
    assert!(matches!(e.mode, crate::mode::Mode::Results));
}
#[test]
fn outline_sidebar_receives_real_lsp_document_symbol_response() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "abc\n").unwrap();
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Opening the sidebar (unlike request_language("outline", None) alone)
    // must route the response into ed.outline, not the transient results
    // list -- this is the real end-to-end request/response path, not a
    // fabricated JSON payload.
    e.toggle_outline();
    assert!(e.active_outline());
    let start = std::time::Instant::now();
    while e.outline.as_ref().is_none_or(|o| o.nodes.is_empty()) {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(e.outline.as_ref().unwrap().nodes[0].name, "symbol");
    assert!(
        e.results.is_none(),
        "the response must not also open the transient results list"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn outline_hover_shows_docs_for_the_outline_symbol_without_moving_the_cursor() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "abc\ndef\nghi\n").unwrap();
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.toggle_outline();
    let start = std::time::Instant::now();
    while e.outline.as_ref().is_none_or(|o| o.nodes.is_empty()) {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "outline LSP timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Move the buffer's real cursor away from the symbol first, so a bug
    // that actually navigates (instead of just peeking) would be caught.
    e.set_cursor(2, 1);
    let before = e.cursor();
    e.hover_outline_symbol();
    let start = std::time::Instant::now();
    while e.results.is_none() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "hover LSP timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        e.cursor(),
        before,
        "hovering from the outline must not move the buffer's real cursor"
    );
    let r = e.results.as_ref().unwrap();
    assert_eq!(r.title, "Hover");
    assert!(r.entries.iter().any(|en| en.text.contains("fixture hover")));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn outline_hover_is_a_noop_when_the_outline_is_stale_for_a_different_buffer() {
    let mut e = editor("abc\n");
    e.outline = Some(crate::outline::Outline::default());
    e.outline.as_mut().unwrap().buffer_path = Some(PathBuf::from("/some/other/file.rs"));
    // No path on this buffer at all, so buffer_path can never match --
    // confirms the mismatch guard, not a crash from an absent path.
    e.hover_outline_symbol();
    assert!(
        e.results.is_none(),
        "a stale outline (different/no document) must not fire a hover request"
    );
}
#[test]
fn type_definition_implementation_and_declaration_jump_via_a_real_lsp_round_trip() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\nsix\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let lsp_cfg = crate::config::LspServer {
        cmd: vec![
            "python3".into(),
            fixture.display().to_string(),
            log.display().to_string(),
        ],
        filetypes: vec!["rust".into()],
        ..Default::default()
    };
    fn wait(e: &mut Editor, predicate: impl Fn(&Editor) -> bool) {
        let start = std::time::Instant::now();
        while !predicate(e) {
            e.poll_lsp_events();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "LSP timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    for (request, method) in [
        (
            Editor::request_type_definition as fn(&mut Editor),
            "textDocument/typeDefinition",
        ),
        (
            Editor::request_implementation,
            "textDocument/implementation",
        ),
        (Editor::request_declaration, "textDocument/declaration"),
    ] {
        let mut e = editor("");
        e.config.lsp.insert("fixture".into(), lsp_cfg.clone());
        e.open_file(file.clone()).unwrap();
        e.sync_lsp();
        wait(&mut e, |e| !e.diagnostics.is_empty());
        request(&mut e);
        wait(&mut e, |e| e.cursor().0 == 4);
        assert_eq!(
            e.cursor(),
            (4, 2),
            "{method} should auto-jump to the single returned location"
        );
    }
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn workspace_symbols_sends_the_query_and_shows_a_list_without_auto_jumping() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one\ntwo needle\nthree\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.request_workspace_symbols("needle_query");
    let start = std::time::Instant::now();
    while e.results.is_none() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let r = e.results.as_ref().unwrap();
    assert_eq!(
        r.entries[0].text, "match_for_needle_query",
        "the query text must actually reach the server, not just any list appear"
    );
    assert_eq!(
        e.mode,
        Mode::Results,
        "workspace symbols must show the picker, not auto-jump like gd on a single result"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn visual_format_sends_a_range_format_request_for_the_selected_lines() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Select lines 2..=3 (0-indexed) via Visual line mode, not the start
    // of the buffer -- proving the requested range, not just line 0, is
    // what actually reaches the server.
    e.set_cursor(2, 0);
    keys(&mut e, "Vj"); // Visual line mode, extend down one line
    keys(&mut e, ",lf");
    assert!(
        !matches!(e.mode, Mode::Visual(_)),
        "formatting the selection must leave Visual mode"
    );
    let start = std::time::Instant::now();
    while e.buf().line_text(2) == "three" {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(e.buf().line_text(0), "one", "line 0 must be untouched");
    assert_eq!(e.buf().line_text(1), "two", "line 1 must be untouched");
    assert!(
        e.buf().line_text(2).starts_with("RANGEFMT"),
        "line 2 (the selection's start) should receive the range-format edit"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn lsp_progress_notifications_surface_in_the_message_line() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
                "--progress".into(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    fn wait(e: &mut Editor, predicate: impl Fn(&Editor) -> bool) {
        let start = std::time::Instant::now();
        while !predicate(e) {
            e.poll_lsp_events();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "LSP timeout: {}",
                e.message
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    // "begin" (on initialized) then "report" (on didOpen, alongside the
    // diagnostic didOpen already sends) should both have landed by now.
    wait(&mut e, |e| e.message.contains("halfway"));
    assert!(
        e.message.contains("Indexing") && e.message.contains("50%"),
        "progress message should show the title and percentage too: {}",
        e.message
    );
    assert!(
        e.lsp_progress.values().any(|p| p.percentage == Some(50)),
        "progress state should be tracked, not just transiently printed"
    );
    // "end" (sent on hover) should remove it from the tracked state.
    e.request_language("hover", None);
    wait(&mut e, |e| e.lsp_progress.is_empty());
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn lsp_progress_shows_in_the_status_line_even_when_the_message_line_says_something_else() {
    let mut e = editor("one\n");
    e.lsp_progress.insert(
        ("fixture".into(), "tok".into()),
        crate::lsp::LspProgress {
            title: Some("Indexing".into()),
            message: None,
            percentage: Some(42),
        },
    );
    // An unrelated action's own message must not hide the persistent
    // status-line indicator, unlike the message-line surfacing this
    // same state already had before this slice.
    e.set_message("Saved");
    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 60, 10);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 10, &mut cache).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("Indexing") && text.contains("42%"),
        "the status line should show active LSP progress. Got: {text:?}"
    );
    assert!(
        text.contains("Saved"),
        "the message line's own text should still be shown separately. Got: {text:?}"
    );

    e.lsp_progress.clear();
    let mut out2 = Vec::new();
    crate::render::draw(&mut out2, &e, 60, 10, &mut cache).unwrap();
    let text2 = String::from_utf8_lossy(&out2);
    assert!(
        !text2.contains("Indexing"),
        "no active progress should mean no indicator. Got: {text2:?}"
    );
}
#[test]
fn completion_enabled_false_suppresses_the_popup_entirely() {
    let mut e = editor("needle\nneed\n");
    e.config.completion_enabled = false;
    e.buf_mut().begin_edit();
    e.enter_insert();
    e.set_cursor_insert(1, 4);
    e.update_completion();
    assert!(
        e.completion.is_none(),
        "completion_enabled = false must suppress the popup even with a real match available"
    );
    e.config.completion_enabled = true;
    e.update_completion();
    assert!(
        e.completion.is_some(),
        "re-enabling it should let the same prefix show the popup again"
    );
}
#[test]
fn completion_delay_ms_hides_the_popup_until_it_elapses() {
    let mut e = editor("needle\nneed\n");
    e.config.completion_delay_ms = 10_000;
    e.buf_mut().begin_edit();
    e.enter_insert();
    e.set_cursor_insert(1, 4);
    e.update_completion();
    assert!(
        e.completion.is_some(),
        "candidates should still be computed immediately regardless of the delay"
    );

    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 40, 10);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 40, 10, &mut cache).unwrap();
    let before = String::from_utf8_lossy(&out).into_owned();
    assert!(
        !before.contains(" buf "),
        "the popup must not be painted before the delay has elapsed. Got: {before:?}"
    );

    // Backdate the trigger time to simulate the delay having elapsed,
    // rather than a real sleep in the test.
    e.completion_since = Some(std::time::Instant::now() - std::time::Duration::from_millis(20_000));
    let mut out2 = Vec::new();
    crate::render::draw(&mut out2, &e, 40, 10, &mut cache).unwrap();
    let after = String::from_utf8_lossy(&out2).into_owned();
    assert!(
        after.contains(" buf "),
        "the popup should be painted once the delay has elapsed. Got: {after:?}"
    );
}
#[test]
fn path_completion_lists_real_directory_entries_relative_to_the_buffer() {
    let root = temp();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/logo.png"), "").unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, "\n").unwrap();

    let mut e = editor("");
    e.open_file(file).unwrap();
    e.enter_insert();
    keys(&mut e, "assets/lo");
    let comp = e
        .completion
        .as_ref()
        .expect("a path-shaped prefix should trigger the completion popup");
    assert!(
        comp.items
            .iter()
            .any(|i| i.insert_text == "assets/logo.png"
                && i.source == crate::completion::Source::Path),
        "should offer the real file relative to the buffer's own directory, \
         got: {:?}",
        comp.items
            .iter()
            .map(|i| &i.insert_text)
            .collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn path_completion_does_not_fire_for_a_plain_identifier() {
    let mut e = editor("");
    e.enter_insert();
    keys(&mut e, "foo_bar");
    // No slash anywhere -- ordinary identifier completion (buffer/LSP)
    // territory, not a path; with no other buffer content to match
    // against, no popup should appear at all.
    assert!(
        e.completion.as_ref().is_none_or(|c| c
            .items
            .iter()
            .all(|i| i.source != crate::completion::Source::Path)),
        "a plain identifier must never trigger path completion"
    );
}
#[test]
fn completion_popup_item_carries_the_lsp_kind_label_from_a_real_round_trip() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    // "fi" fuzzy-matches mock_lsp.py's completion reply, whose
    // filterText is "FIX" (matches is a case-insensitive, in-order
    // subsequence check -- f then i, both present).
    std::fs::write(&file, "fi\n").unwrap();
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.enter_insert();
    e.set_cursor_insert(0, 2); // end of "fi"
    e.update_completion();
    let start = std::time::Instant::now();
    while !e.completion.as_ref().is_some_and(|c| {
        c.items
            .iter()
            .any(|i| i.source == crate::completion::Source::Lsp)
    }) {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "completion LSP timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let item = e
        .completion
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|i| i.source == crate::completion::Source::Lsp)
        .unwrap();
    // mock_lsp.py's completion reply sets "kind": 3 (LSP Function).
    assert_eq!(item.kind, Some(3));
    assert_eq!(crate::completion::kind_label(item.kind.unwrap()), "fn");
    assert_eq!(
        crate::completion::item_documentation(item),
        Some("fixture docs for FIX".to_string()),
        "should extract the server's MarkupContent documentation, not \
         just the one-line detail"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn document_highlight_round_trip_populates_ranges_and_esc_clears_them() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one two three\nfour five six\nseven eight nine\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.request_language("documentHighlight", None);
    let start = std::time::Instant::now();
    while e.document_highlights.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "documentHighlight timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // mock_lsp.py's fixture reply: line 0 chars 4-10, line 2 chars 0-6.
    assert_eq!(e.document_highlights, vec![(0, 4, 0, 10), (2, 0, 2, 6)]);
    assert_eq!(e.document_highlights_buffer, Some(e.buf().id));
    keys(&mut e, "\x1b"); // plain Esc in Normal mode clears them
    assert!(
        e.document_highlights.is_empty(),
        "Esc should clear document highlights"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn document_links_round_trip_lists_a_file_link_and_a_web_link() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one two three\nfour five six\n").unwrap();
    // The mock's file link points at this exact sibling path.
    std::fs::write(root.join("other.txt"), "sibling content\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.project_root = root.clone();
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.request_language("documentLinks", None);
    let start = std::time::Instant::now();
    while e.results.is_none() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "documentLinks timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let r = e.results.as_ref().unwrap();
    assert_eq!(r.title, "Document links");
    assert_eq!(r.entries.len(), 2);
    assert_eq!(r.entries[0].text, "Open other.txt");
    assert_eq!(
        r.entries[1].action.as_ref().unwrap()["_vaayu_open_link"],
        "https://example.com/docs"
    );

    // Opening entry 0 (the file link) should actually open other.txt.
    e.open_result();
    assert_eq!(
        e.buf()
            .path
            .as_ref()
            .map(|p| p.file_name().unwrap().to_str().unwrap()),
        Some("other.txt"),
        "opening a file:// link should switch to that buffer"
    );

    // Re-request and open the web link this time -- it must not touch
    // the current buffer, only copy the URL.
    e.request_language("documentLinks", None);
    let start = std::time::Instant::now();
    while e.results.is_none() {
        e.poll_lsp_events();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.results.as_mut().unwrap().cursor = 1;
    e.open_result();
    assert_eq!(
        e.buf()
            .path
            .as_ref()
            .map(|p| p.file_name().unwrap().to_str().unwrap()),
        Some("other.txt"),
        "a web link must not switch buffers"
    );
    assert_eq!(
        e.registers.get(Some('+')).unwrap().text,
        "https://example.com/docs"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn code_lens_round_trip_shows_only_the_runnable_lens_and_runs_it() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one two three\nfour five six\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.project_root = root.clone();
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.request_language("codeLens", None);
    let start = std::time::Instant::now();
    while e.results.is_none() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "codeLens timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let r = e.results.as_ref().unwrap();
    assert_eq!(r.title, "Code lenses");
    assert_eq!(
        r.entries.len(),
        1,
        "the resolve-only lens (no command) should be skipped"
    );
    assert_eq!(r.entries[0].text, "\u{25b6} Run fixture");
    assert_eq!(
        e.code_lenses.len(),
        1,
        "the runnable lens should also be cached for virtual-text rendering"
    );
    assert_eq!(e.code_lenses[0].0, 0, "lens is on line 0");

    e.open_result();
    let start = std::time::Instant::now();
    while e.message != "Code action completed" {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "running the lens's command timed out: {}",
            e.message
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let sent = std::fs::read_to_string(&log).unwrap();
    assert!(
        sent.contains("workspace/executeCommand") && sent.contains("fixture.run"),
        "running a code lens should send workspace/executeCommand with its own \
         command, reusing apply_code_action's existing dispatch:\n{sent}"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn inlay_hints_round_trip_positions_both_label_shapes_and_clears_on_esc() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one two three\nfour five six\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.project_root = root.clone();
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.request_language("inlayHints", None);
    let start = std::time::Instant::now();
    while e.inlay_hints.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "inlayHints timeout: {}",
            e.message
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(e.inlay_hints.len(), 2);
    assert_eq!(
        e.inlay_hints[0],
        (0, 3, " : Type".to_string()),
        "a plain-string label with paddingLeft should get a leading space"
    );
    assert_eq!(
        e.inlay_hints[1],
        (1, 0, "param: ".to_string()),
        "a label given as parts should concatenate their values"
    );
    assert!(e.message.contains("2 inlay hint"));

    keys(&mut e, "\u{1b}"); // Esc in Normal mode clears them
    assert!(
        e.inlay_hints.is_empty(),
        "Esc should clear inlay hints the same way it clears document highlights"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn location_results_get_a_working_preview_pane_and_jump_list_entry() {
    // Phase 3 item 2 ("preview panes for definition/implementation/type
    // definition/references with jump-list integration") turned out to
    // already be satisfied: every one of those kinds builds an ordinary
    // location-carrying `Results` list, and `Results::preview_rows`/
    // `Editor::jump_to` are already fully generic over any such list --
    // not something specific to search/grep. This test is the concrete
    // evidence for that claim, not new production code.
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    // typeDefinition's mock reply always points at line 4 (0-indexed) --
    // needs to actually exist, unlike the 1-2 line fixtures other tests
    // in this file use for features that don't care.
    std::fs::write(&file, "one\ntwo needle\nthree\nfour\nfive\nsix\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.project_root = root.clone();
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // typeDefinition is single-result, so it auto-jumps -- proving
    // jump-list integration: the pre-jump location is pushed first.
    let jumps_before = e.jumps.len();
    e.request_language("typeDefinition", None);
    let start = std::time::Instant::now();
    while e.cursor().0 != 4 {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "typeDefinition timeout: {}",
            e.message
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        e.cursor(),
        (4, 2),
        "auto-jump should land exactly where the mock server pointed"
    );
    assert!(
        e.jumps.len() > jumps_before,
        "auto-jump should push the pre-jump location onto the jump list"
    );

    // workspaceSymbols never auto-jumps (a search list, not "go here"),
    // so it stays in Results mode where a preview pane can be checked.
    // `self.results` is already `Some` (typeDefinition's, never cleared
    // by its own auto-jump above) so the wait has to check for *this*
    // request's own list by title, not just any list showing up.
    e.request_language("workspaceSymbols", Some("thing"));
    let start = std::time::Instant::now();
    while e.results.as_ref().map(|r| r.title.as_str()) != Some("workspaceSymbols") {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "workspaceSymbols timeout: {}",
            e.message
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(e.mode, crate::mode::Mode::Results);
    let entry_path = e.results.as_ref().unwrap().entries[0]
        .path
        .clone()
        .expect("a workspace symbol result should carry a location path");
    let source = e.preview_source_lines(&entry_path);
    e.results.as_mut().unwrap().preview = true;
    let rows = e
        .results
        .as_ref()
        .unwrap()
        .preview_rows(&source, 5, 78, 1)
        .expect("preview should be available for a location entry");
    assert!(
        rows.iter().any(|r| r.is_match && r.text.contains("needle")),
        "the preview pane should show the real source line the symbol points at, got: {:?}",
        rows.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn code_actions_show_disabled_reason_and_sort_preferred_first() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "abc\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.project_root = root.clone();
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.request_language("actions", None);
    let start = std::time::Instant::now();
    while e.results.is_none() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "actions timeout: {}",
            e.message
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let r = e.results.as_ref().unwrap();
    assert_eq!(
        r.entries.len(),
        2,
        "a disabled action must still be shown, not silently dropped"
    );
    assert!(
        r.entries[0].text.starts_with("* Fix fixture"),
        "the isPreferred action should sort first and be marked, got: {}",
        r.entries[0].text
    );
    assert!(
        r.entries[1].text.contains("Disabled fixture")
            && r.entries[1].text.contains("not applicable here"),
        "the disabled action should show its reason, got: {}",
        r.entries[1].text
    );

    // Selecting the disabled one must refuse to run it.
    e.results.as_mut().unwrap().cursor = 1;
    e.open_result();
    assert_eq!(
        e.message, "This action is disabled: not applicable here",
        "opening a disabled action should explain why, not silently no-op"
    );
    assert_eq!(
        e.buf().line_text(0),
        "abc",
        "a disabled action must never be applied"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn organize_imports_applies_directly_without_a_picker() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "abc\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.project_root = root.clone();
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.request_language("organizeImports", None);
    let start = std::time::Instant::now();
    while e.buf().line_text(0) == "abc" {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "organizeImports timeout: {}",
            e.message
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(e.buf().line_text(0), "ORGANIZED");
    assert!(
        e.results.as_ref().is_none_or(|r| r.title != "Code actions"),
        "organizeImports should apply directly, never open a picker"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn diagnostics_defer_visible_updates_in_insert_mode_and_flush_on_esc() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let mut e = editor("");
    e.project_root = root.clone();
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
                "--diag-on-change".into(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(e.diagnostic_results().entries[0]
        .text
        .contains("fixture warning"));

    // Enter Insert mode and dirty the buffer -- didChange fires, and the
    // mock (only with --diag-on-change) replies with a completely
    // different diagnostic (an error, with source/code/relatedInformation).
    keys(&mut e, "iX");
    assert_eq!(e.mode, crate::mode::Mode::Insert);
    e.sync_lsp();
    let start = std::time::Instant::now();
    while !e
        .server_diagnostics
        .values()
        .any(|ds| ds.iter().any(|d| d.message == "fixture error"))
    {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "didChange diagnostics timeout: {}",
            e.message
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // The new diagnostic has definitely arrived (recorded in
    // server_diagnostics above) but the *visible* set must still be the
    // old one -- diagnostics_update_in_insert defaults to false.
    assert!(
        e.diagnostic_results().entries[0]
            .text
            .contains("fixture warning"),
        "diagnostics must not update visibly while still in Insert mode, got: {}",
        e.diagnostic_results().entries[0].text
    );

    e.close_completion(); // typing "X" may have opened a completion popup,
                          // whose own Esc would just close it instead of
                          // leaving Insert -- close it directly so a
                          // single Esc below is unambiguous.
    keys(&mut e, "\u{1b}"); // Esc leaves Insert mode -> flush
    assert_eq!(e.mode, crate::mode::Mode::Normal);
    let r = e.diagnostic_results();
    assert!(
        r.entries[0].text.contains("fixture error")
            && r.entries[0].text.contains("[eslint(no-unused-vars)]"),
        "leaving Insert mode should flush the deferred update, with its \
         source/code label, got: {}",
        r.entries[0].text
    );
    assert!(
        r.entries[1].text.contains("↳") && r.entries[1].text.contains("declared here"),
        "relatedInformation should show as its own jumpable entry right \
         after its parent, got: {:?}",
        r.entries.iter().map(|e| &e.text).collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn buffer_reload_discards_in_memory_changes_and_undo_history() {
    let root = temp();
    let file = root.join("f.txt");
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    let mut e = editor("");
    e.open_file(file.clone()).unwrap();
    keys(&mut e, "dd"); // dirty the in-memory buffer, building undo history
    assert!(e.buf().is_modified());
    assert_eq!(e.buf().rope.to_string(), "two\nthree\n");

    // The file changes on disk out from under the buffer (as a git hunk
    // reset would do via `git apply --reverse`), then the buffer reloads.
    std::fs::write(&file, "one\ntwo\nthree\nfour\n").unwrap();
    e.buf_mut().reload().unwrap();

    assert_eq!(e.buf().rope.to_string(), "one\ntwo\nthree\nfour\n");
    assert!(
        !e.buf().is_modified(),
        "the reloaded content should count as saved, not dirty"
    );
    // Undo history must be cleared -- there's nothing coherent left for
    // `u` to reconstruct once the rope it was built against is gone.
    keys(&mut e, "u");
    assert_eq!(
        e.buf().rope.to_string(),
        "one\ntwo\nthree\nfour\n",
        "undo after a reload should be a no-op, not resurrect pre-reload content"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn buffer_reload_clamps_the_cursor_into_a_shorter_file() {
    let root = temp();
    let file = root.join("f.txt");
    std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\n").unwrap();
    let mut e = editor("");
    e.open_file(file.clone()).unwrap();
    e.set_cursor(4, 0); // last line

    std::fs::write(&file, "one\n").unwrap();
    e.buf_mut().reload().unwrap();

    let max_line = e.buf().rope.len_lines().saturating_sub(1);
    assert_eq!(
        e.cursor().0,
        max_line,
        "the cursor must clamp into the now-shorter file, not point past its end"
    );
    assert!(
        max_line <= 1,
        "sanity check: the reloaded one-line file shouldn't have more \
         than ropey's usual trailing-newline extra empty line"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn ex_command_e_force_reloads_the_current_buffer() {
    let root = temp();
    let file = root.join("f.txt");
    std::fs::write(&file, "one\n").unwrap();
    let mut e = editor("");
    e.open_file(file.clone()).unwrap();
    keys(&mut e, "dd"); // dirty it
    assert!(e.buf().is_modified());

    std::fs::write(&file, "reloaded content\n").unwrap();
    keys(&mut e, ":e!\n");

    assert_eq!(e.buf().rope.to_string(), "reloaded content\n");
    assert!(!e.buf().is_modified());
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn switch_project_updates_root_and_drops_any_open_file_tree() {
    let root = temp();
    let other = temp();
    let mut e = editor("");
    e.project_root = root.clone();
    e.toggle_file_tree();
    assert!(e.file_tree.is_some());
    assert!(e.windows.iter().any(|w| w.file_tree));

    e.switch_project(other.clone());
    assert_eq!(e.project_root, other);
    assert!(
        e.file_tree.is_none(),
        "an open file tree must be dropped, not left stale at the old root"
    );
    assert!(
        !e.windows.iter().any(|w| w.file_tree),
        "the tree's window should be closed too, not just forgotten"
    );
    assert!(e.message.contains(&other.display().to_string()));
    std::fs::remove_dir_all(root).ok();
    std::fs::remove_dir_all(other).ok();
}
#[test]
fn results_entry_tagged_switch_project_calls_through_open_result() {
    let mut e = editor("");
    let target = temp();
    e.results = Some(crate::results::Results::new(
        "Recent projects",
        vec![{
            let mut entry = crate::results::Entry::text(target.display().to_string());
            entry.action = Some(serde_json::json!({"_vaayu_switch_project": target}));
            entry
        }],
    ));
    e.open_result();
    assert_eq!(e.project_root, target);
    std::fs::remove_dir_all(target).ok();
}
#[test]
fn hunk_preview_shows_the_hunk_under_the_cursor_not_a_different_one() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..20).map(|i| format!("line {i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    // Far enough apart (13 lines) that --unified=3's context windows
    // don't overlap and merge into a single hunk.
    let new = old
        .replace("line 2\n", "CHANGED_A\n")
        .replace("line 15\n", "CHANGED_B\n");
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();

    e.set_cursor(2, 0); // inside the first hunk
    e.preview_current_hunk();
    let r = e
        .results
        .as_ref()
        .expect("hunk preview should open a results list");
    assert!(r.entries.iter().any(|en| en.text.contains("CHANGED_A")));
    assert!(
        !r.entries.iter().any(|en| en.text.contains("CHANGED_B")),
        "should show only the hunk under the cursor, not the other one"
    );

    e.set_cursor(15, 0); // inside the second hunk
    e.preview_current_hunk();
    let r = e.results.as_ref().unwrap();
    assert!(r.entries.iter().any(|en| en.text.contains("CHANGED_B")));
    assert!(!r.entries.iter().any(|en| en.text.contains("CHANGED_A")));

    std::fs::remove_dir_all(root).ok();
}
#[test]
fn hunk_preview_refuses_on_an_unsaved_buffer_and_reports_no_hunks() {
    let root = temp();
    std::process::Command::new("git")
        .current_dir(&root)
        .args(["init", "-q"])
        .output()
        .unwrap();
    let file = root.join("sample.txt");
    std::fs::write(&file, "one\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    keys(&mut e, "x"); // dirty the buffer without saving
    e.preview_current_hunk();
    assert!(
        e.results.is_none() && e.message.contains("Save"),
        "an unsaved buffer should refuse with a clear message, got: {}",
        e.message
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn hunk_reset_restores_the_working_tree_and_open_buffer_to_head() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..20).map(|i| format!("line {i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    let new = old
        .replace("line 2\n", "CHANGED_A\n")
        .replace("line 15\n", "CHANGED_B\n");
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.set_cursor(2, 0); // inside the first hunk only

    e.reset_current_hunk_prompt();
    let r = e
        .results
        .as_ref()
        .expect("resetting a hunk should show a confirmation prompt");
    assert!(r.entries.iter().any(|en| en.text.contains("CHANGED_A")));
    let value = r.entries[0]
        .action
        .as_ref()
        .and_then(|a| a.get("_vaayu_git_hunk_reset"))
        .cloned()
        .expect("prompt entries should carry the hunk-reset action");

    e.apply_hunk_reset(&value);
    assert!(
        e.message.to_lowercase().contains("reset"),
        "got: {}",
        e.message
    );
    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert!(
        on_disk.contains("line 2\n"),
        "the reset hunk should be back to HEAD on disk"
    );
    assert!(
        on_disk.contains("CHANGED_B\n"),
        "the other, untouched hunk should be left alone"
    );
    assert_eq!(
        e.buf().rope.to_string(),
        on_disk,
        "the open buffer should reload to match the file on disk"
    );

    std::fs::remove_dir_all(root).ok();
}
#[test]
fn hunk_reset_refuses_on_an_unsaved_buffer() {
    let root = temp();
    std::process::Command::new("git")
        .current_dir(&root)
        .args(["init", "-q"])
        .output()
        .unwrap();
    let file = root.join("sample.txt");
    std::fs::write(&file, "one\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    keys(&mut e, "x"); // dirty the buffer without saving
    e.reset_current_hunk_prompt();
    assert!(
        e.results.is_none() && e.message.contains("Save"),
        "an unsaved buffer should refuse with a clear message, got: {}",
        e.message
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn line_blame_toggle_populates_and_renders_metadata_for_the_current_line() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let file = root.join("sample.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.set_cursor(1, 0); // "beta"

    assert!(!e.blame_toggle);
    e.toggle_line_blame();
    assert!(e.blame_toggle);
    let start = std::time::Instant::now();
    while e.line_blame.is_none() {
        e.poll_blame_task();
        assert!(e.blame_toggle, "blame task errored out: {}", e.message);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "blame task timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let lines = e.line_blame.as_ref().unwrap();
    assert_eq!(lines.len(), 3);
    assert!(
        lines[1].contains("Vaayu test"),
        "line 1's blame should carry the committing author, got: {}",
        lines[1]
    );

    // Rendering paints it only on the current line (line 1), not others.
    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 60, 10);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 10, &mut cache).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("Vaayu test"),
        "blame text should be painted for the current line. Got: {text:?}"
    );

    e.toggle_line_blame();
    assert!(!e.blame_toggle);
    assert!(e.line_blame.is_none(), "toggling off should clear the data");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn line_blame_toggle_refuses_without_a_file_on_disk() {
    let mut e = editor("abc\n");
    e.toggle_line_blame();
    assert!(!e.blame_toggle);
    assert!(e.message.contains("Open a repository file"));
}
#[test]
fn document_highlight_becomes_stale_after_an_edit_and_is_not_painted() {
    let mut e = editor("one two three\n");
    let id = e.buf().id;
    e.document_highlights = vec![(0, 4, 0, 10)];
    e.document_highlights_buffer = Some(id);
    e.document_highlights_edit_seq = e.buf().edit_seq;
    let fresh_seq = e.document_highlights_edit_seq;
    // A real edit bumps edit_seq, making the captured snapshot stale --
    // render.rs's `doc_highlighted` gate must then skip painting rather
    // than highlighting whatever now sits at those old positions.
    keys(&mut e, "x");
    assert_ne!(
        e.buf().edit_seq,
        fresh_seq,
        "editing the buffer should change edit_seq"
    );
    assert_eq!(
        e.document_highlights_edit_seq, fresh_seq,
        "the stored snapshot itself is untouched by the edit"
    );
}
#[test]
fn outline_sidebar_corrects_utf16_columns_for_surrogate_pairs() {
    // mock_lsp.py's documentSymbol reply always reports character=3 (UTF-16
    // code units) on line 0, regardless of file content. A leading
    // surrogate-pair character (like this emoji, 2 UTF-16 units but 1
    // char) makes the raw unit count land on a different character than
    // the corrected char index does -- exactly the bug this fixes.
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "\u{1F600}xyz\n").unwrap();
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while e.diagnostics.is_empty() {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP init timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    e.toggle_outline();
    let start = std::time::Instant::now();
    while e.outline.as_ref().is_none_or(|o| o.nodes.is_empty()) {
        e.poll_lsp_events();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "LSP timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Raw UTF-16 units (3) would land on 'z'; the corrected char index (2)
    // lands on 'y', right after the 1-char, 2-unit emoji.
    assert_eq!(
        e.outline.as_ref().unwrap().nodes[0].col,
        2,
        "outline symbol column must be UTF-16-corrected, not a raw code-unit count"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn tabs_keep_independent_pane_state() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let mut e = editor("");
    e.open_file(a.clone()).unwrap();
    e.set_cursor(0, 2);
    e.new_tab();
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active_tab, 1);
    e.open_file(b.clone()).unwrap();
    e.set_cursor(0, 1);
    // Switching back to tab 1 must restore its own buffer and cursor,
    // independent of what happened in tab 2.
    e.prev_tab();
    assert_eq!(e.active_tab, 0);
    assert_eq!(e.buf().path, Some(a.clone()));
    assert_eq!(e.cursor(), (0, 2));
    e.next_tab();
    assert_eq!(e.active_tab, 1);
    assert_eq!(e.buf().path, Some(b));
    assert_eq!(e.cursor(), (0, 1));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn gt_and_counted_gt_navigate_tabs() {
    let mut e = editor("a\n");
    e.new_tab();
    e.new_tab();
    assert_eq!(e.active_tab, 2);
    keys(&mut e, "gT");
    assert_eq!(e.active_tab, 1);
    keys(&mut e, "gt");
    assert_eq!(e.active_tab, 2);
    keys(&mut e, "1gt");
    assert_eq!(e.active_tab, 0);
}
#[test]
fn counted_gt_works_while_focused_on_a_terminal_pane() {
    // Regression: the terminal guard used to swallow the completing key of
    // a multi-key sequence (the 't' of `{n}gt`) whenever it wasn't itself
    // 'g' or a digit, leaving Awaiting::GPrefix stuck forever and quietly
    // eating every keystroke after -- including the ':' that should have
    // opened the command line.
    let mut e = editor("a\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.new_tab();
    e.open_terminal(); // active pane in tab 2 is now a terminal
    e.feed_key(Key::Esc);
    keys(&mut e, "1gt");
    assert_eq!(
        e.active_tab, 0,
        "{{n}}gt must switch tabs from a terminal pane"
    );
    assert!(
        e.pending.awaiting.is_none(),
        "no awaiting state should be left stuck"
    );
    // And the very next keystroke, ':', must still open the command line.
    keys(&mut e, ":tabs\n");
    assert!(e.results.is_some(), "':' must not have been swallowed");
}
#[test]
fn tabclose_kills_its_terminals_and_refuses_to_close_the_last_tab() {
    let mut e = editor("a\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.new_tab();
    e.open_terminal();
    assert_eq!(e.terminals.len(), 1);
    e.close_tab();
    assert_eq!(e.tabs.len(), 1);
    assert!(
        e.terminals.is_empty(),
        "closing a tab must shut down its terminals"
    );
    e.close_tab();
    assert_eq!(e.tabs.len(), 1, "the last tab must never close");
}
#[test]
fn tabonly_kills_terminals_in_discarded_tabs_only() {
    let mut e = editor("a\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.open_terminal(); // terminal in tab 1 (the eventual survivor)
    e.feed_key(Key::Esc);
    e.new_tab();
    e.open_terminal(); // terminal in tab 2 (discarded)
    e.feed_key(Key::Esc);
    e.prev_tab(); // back to tab 1, the survivor
    assert_eq!(e.terminals.len(), 2);
    e.tab_only();
    assert_eq!(e.tabs.len(), 1);
    assert_eq!(
        e.terminals.len(),
        1,
        "the survivor's own terminal must not be killed"
    );
}
#[test]
fn terminal_opens_runs_shell_and_shuts_down_on_close() {
    let mut e = editor("x\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.open_terminal();
    assert_eq!(e.mode, Mode::Terminal);
    assert_eq!(e.windows.len(), 2);
    let id = e
        .active_terminal_id()
        .expect("active pane should be a terminal");

    // Type a command and see its output land in the vt100 screen.
    keys(&mut e, "echo hi_from_test\n");
    let start = std::time::Instant::now();
    loop {
        let seen = e
            .terminals
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .with_screen(|s| s.contents().contains("hi_from_test"));
        if seen {
            break;
        }
        assert!(
            start.elapsed().as_secs() < 5,
            "terminal output never arrived"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // Esc leaves to Normal (still focused on the pane) without touching the job.
    e.feed_key(Key::Esc);
    assert_eq!(e.mode, Mode::Normal);
    assert_eq!(e.terminals.len(), 1);

    // Closing the pane must shut the job down: no leaked terminals list entry.
    e.close_window();
    assert!(
        e.terminals.is_empty(),
        "closing the pane must shut its job down"
    );
    assert!(e.active_terminal_id().is_none());
}
#[test]
fn toggle_agent_session_starts_detaches_and_reattaches() {
    let mut e = editor("x\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    // A real shell standing in for "claude"/"codex" -- this sandbox has
    // neither installed, but the toggle logic itself is identical
    // regardless of which binary it happens to run.
    e.config
        .agent_commands
        .insert("testagent".into(), vec!["/bin/sh".into()]);

    e.toggle_agent_session("testagent");
    assert_eq!(e.mode, Mode::Terminal);
    assert_eq!(e.windows.len(), 2);
    let id = e
        .terminals
        .iter()
        .find(|p| p.agent_kind.as_deref() == Some("testagent"))
        .expect("a tagged agent session should be running")
        .id;
    assert_eq!(e.active_terminal_id(), Some(id));

    // Toggling again detaches: the pane goes away, but the process
    // keeps running.
    e.toggle_agent_session("testagent");
    assert!(e.windows.is_empty() || e.windows.iter().all(|w| w.terminal != Some(id)));
    assert_eq!(e.terminals.len(), 1, "detaching must not kill the session");
    assert!(
        e.message.to_lowercase().contains("detached"),
        "got: {}",
        e.message
    );

    // Toggling a third time reattaches the same session (not a new one).
    e.toggle_agent_session("testagent");
    assert_eq!(e.mode, Mode::Terminal);
    assert_eq!(
        e.terminals.len(),
        1,
        "reattaching must reuse the existing session, not spawn another"
    );
    assert_eq!(e.active_terminal_id(), Some(id));
    assert!(e.message.to_lowercase().contains("reattached"));

    // Closing the pane now (the normal, non-toggle path) still kills it.
    e.close_window();
    assert!(e.terminals.is_empty());
}
#[test]
fn list_agent_sessions_reports_attached_and_detached_state() {
    let mut e = editor("x\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.config
        .agent_commands
        .insert("testagent".into(), vec!["/bin/sh".into()]);
    e.toggle_agent_session("testagent");

    e.list_agent_sessions();
    let r = e.results.as_ref().expect("should list the running session");
    assert!(r.entries[0].text.contains("testagent"));
    assert!(r.entries[0].text.contains("attached in this tab"));

    e.enter_normal();
    e.toggle_agent_session("testagent"); // detach
    e.list_agent_sessions();
    let r = e.results.as_ref().unwrap();
    assert!(
        r.entries[0].text.contains("detached"),
        "got: {}",
        r.entries[0].text
    );

    // Enter on the (detached) entry reattaches it here.
    e.open_result();
    assert_eq!(e.mode, Mode::Terminal);
    assert!(e.active_terminal_id().is_some());

    e.close_window();
}
#[test]
fn reattach_agent_session_focuses_instead_of_duplicating() {
    let mut e = editor("x\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.config
        .agent_commands
        .insert("testagent".into(), vec!["/bin/sh".into()]);
    e.toggle_agent_session("testagent");
    let id = e.active_terminal_id().unwrap();
    e.split_window(true, false); // a second, unrelated pane
    assert_eq!(e.windows.len(), 3);

    e.reattach_agent_session(id);
    assert_eq!(
        e.windows.len(),
        3,
        "reattaching an already-attached session should just focus it, not open a duplicate pane"
    );
    assert_eq!(e.active_terminal_id(), Some(id));
    e.close_window();
    e.close_window();
}
#[test]
fn toggle_agent_session_fails_visibly_for_a_missing_binary() {
    let mut e = editor("x\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.config.agent_commands.insert(
        "testagent".into(),
        vec!["vaayu-definitely-not-a-real-binary".into()],
    );
    e.toggle_agent_session("testagent");
    assert!(
        e.message.to_lowercase().contains("could not start"),
        "got: {}",
        e.message
    );
    assert!(e.terminals.is_empty());
    assert_eq!(e.mode, Mode::Normal);
}
#[test]
fn results_preview_scroll_stops_at_the_last_line_instead_of_scrolling_forever() {
    let root = temp();
    let file = root.join("f.txt");
    std::fs::write(&file, "needle\nsecond\nthird\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.open_grep("needle");
    let start = std::time::Instant::now();
    while e.results.as_ref().unwrap().busy {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    keys(&mut e, "\x1b"); // leave query-editing, into browsing
    keys(&mut e, "p"); // preview on

    // Scroll far past the file's own 3 lines -- this used to grow
    // preview_scroll without any bound at all.
    for _ in 0..20 {
        e.feed_key(Key::Ctrl('e'));
    }
    assert_eq!(
        e.results.as_ref().unwrap().preview_scroll,
        2,
        "scrolling should stop once the source's last real line is showing, \
         not keep growing forever"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn file_picker_preview_scroll_stops_at_the_last_line_instead_of_scrolling_forever() {
    let root = temp();
    let file = root.join("small.txt");
    std::fs::write(&file, "only one line\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.all_files = vec!["small.txt".into()];
    e.open_picker();
    e.file_picker.as_mut().unwrap().preview = true;

    for _ in 0..20 {
        e.feed_key(Key::Ctrl('e'));
    }
    assert_eq!(
        e.file_picker.as_ref().unwrap().preview_scroll,
        0,
        "a single-line file should never let the preview scroll at all"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn zg_adds_word_under_cursor_to_dictionary() {
    let mut e = editor("vaayu\n");
    e.dictionary = Some(crate::spell::Dictionary::for_test(&["hello"]));
    keys(&mut e, "zg");
    assert!(e
        .dictionary
        .as_ref()
        .unwrap()
        .misspelled_in("vaayu")
        .is_empty());
}
#[test]
fn z_equals_suggests_and_replaces_the_word_under_cursor() {
    let mut e = editor("wrold\n");
    e.dictionary = Some(crate::spell::Dictionary::for_test(&["world", "hello"]));
    keys(&mut e, "z=");
    let idx = {
        let r = e.results.as_ref().expect("suggestions should open");
        r.entries
            .iter()
            .position(|en| en.text == "world")
            .expect("world should be suggested for wrold")
    };
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    assert_eq!(e.buf().line_text(0), "world");
}
#[test]
fn spellcheck_command_lists_misspelled_words_only() {
    let mut e = editor("hello wrold\nfoo bar\n");
    e.dictionary = Some(crate::spell::Dictionary::for_test(&["hello", "foo", "bar"]));
    keys(&mut e, ":spellcheck\n");
    let r = e.results.as_ref().expect("results should open");
    assert_eq!(r.entries.len(), 1);
    assert!(r.entries[0].text.contains("wrold"));
}
#[test]
fn increment_and_decrement_numbers() {
    let mut e = editor("count: 41\n");
    e.feed_key(Key::Ctrl('a'));
    assert_eq!(e.buf().line_text(0), "count: 42");
    e.feed_key(Key::Ctrl('a'));
    e.feed_key(Key::Ctrl('a'));
    assert_eq!(e.buf().line_text(0), "count: 44");
    e.feed_key(Key::Ctrl('x'));
    assert_eq!(e.buf().line_text(0), "count: 43");
}
#[test]
fn increment_preserves_zero_padding_and_count() {
    let mut e = editor("id 007\n");
    keys(&mut e, "5");
    e.feed_key(Key::Ctrl('a'));
    assert_eq!(e.buf().line_text(0), "id 012");
}
#[test]
fn increment_searches_forward_on_current_line_only() {
    let mut e = editor("no digits here\nbut 9 on this one\n");
    e.feed_key(Key::Ctrl('a'));
    // No number on line 1 at/after the cursor: no-op, no crash, no
    // searching into line 2.
    assert_eq!(e.buf().line_text(0), "no digits here");
    assert_eq!(e.buf().line_text(1), "but 9 on this one");
}
#[test]
fn leader_a_selects_entire_buffer() {
    let mut e = editor("one\ntwo\nthree\n");
    keys(&mut e, ",a");
    assert!(matches!(
        e.mode,
        crate::mode::Mode::Visual(crate::mode::VisualKind::Line)
    ));
    assert_eq!(e.visual_anchor, Some((0, 0)));
    assert_eq!(e.cursor().0, 2);
}
#[test]
fn visual_indent_retains_selection_for_repeated_presses() {
    let mut e = editor("a\nb\nc\n");
    keys(&mut e, "VG");
    e.feed_key(Key::Char('>'));
    assert!(matches!(
        e.mode,
        crate::mode::Mode::Visual(crate::mode::VisualKind::Line)
    ));
    e.feed_key(Key::Char('>'));
    e.feed_key(Key::Esc);
    assert_eq!(e.buf().line_text(0), "        a");
    assert_eq!(e.buf().line_text(2), "        c");
}
#[test]
fn subword_motion_with_operator_and_dot_repeat() {
    let mut e = editor("myVarName rest\n");
    keys(&mut e, "dgw");
    assert_eq!(e.buf().line_text(0), "VarName rest");
    keys(&mut e, ".");
    assert_eq!(e.buf().line_text(0), "Name rest");
}
#[test]
fn subword_motion_extends_visual_selection() {
    let mut e = editor("myVarName\n");
    keys(&mut e, "vgwgwd");
    // v anchors at 'm' (col 0); gw gw moves to 'V' (2) then 'N' (5),
    // inclusive visual delete removes columns 0..=5 ("myVarN").
    assert_eq!(e.buf().line_text(0), "ame");
}
#[test]
fn empty_inner_objects() {
    for s in ["()\n", "\"\"\n"] {
        let mut e = editor(s);
        keys(&mut e, if s.starts_with('(') { "di(" } else { "di\"" });
        assert_eq!(e.buf().rope.to_string(), s);
    }
}
#[test]
fn change_empty_pair() {
    let mut e = editor("()\n");
    keys(&mut e, "ci(Z\x1b");
    assert_eq!(e.buf().line_text(0), "(Z)");
}
#[test]
fn dot_count_and_override() {
    let mut e = editor("abcdefghij\n");
    keys(&mut e, "3x.");
    assert_eq!(e.buf().line_text(0), "ghij");
    let mut e = editor("abcdefghij\n");
    keys(&mut e, "x3.");
    assert_eq!(e.buf().line_text(0), "efghij");
}
#[test]
fn visual_dot() {
    let mut e = editor("abcdefghij\n");
    keys(&mut e, "vld.");
    assert_eq!(e.buf().line_text(0), "efghij");
    assert_eq!(e.mode, Mode::Normal);
}
#[test]
fn insert_exit_cursor() {
    let mut e = editor("abc\n");
    keys(&mut e, "iZ\x1b");
    assert_eq!(e.cursor(), (0, 0));
}
#[test]
fn eof_word_delete() {
    let mut e = editor("abc\n");
    keys(&mut e, "dw");
    assert!(e.buf().line_text(0).is_empty());
}
#[test]
fn unicode_join() {
    let mut e = editor("a\n\u{2003}xyz\n");
    keys(&mut e, "J");
    assert_eq!(e.buf().rope.to_string(), "a xyz\n");
}
#[test]
fn newline_at_eof() {
    let mut e = editor("abc");
    keys(&mut e, "A\nZ\x1b");
    assert_eq!(e.buf().rope.to_string(), "abc\nZ");
}
#[test]
fn counted_paste_single_undo() {
    let mut e = editor("X\n");
    e.registers.set(None, "ab".into(), false);
    keys(&mut e, "2p");
    assert_eq!(e.buf().line_text(0), "Xabab");
    keys(&mut e, "u");
    assert_eq!(e.buf().line_text(0), "X");
}
#[test]
fn search_anchors_unicode() {
    let e = editor("界abc\ndef\n");
    assert_eq!(
        crate::search::find(e.buf(), 0, "^def", true, false, false).unwrap(),
        Some(5)
    );
}

#[test]
fn repeated_search_cache_wraps_and_invalidates_on_edit() {
    let mut e = editor("one x one\n");
    assert_eq!(e.find_search("one", 0, true).unwrap(), Some(6));
    assert_eq!(e.find_search("one", 6, true).unwrap(), Some(0));
    assert_eq!(e.find_search("one", 0, false).unwrap(), Some(6));
    e.buf_mut().insert_str(0, 0, "one ");
    assert_eq!(e.find_search("one", 0, true).unwrap(), Some(4));
}
#[test]
fn escaped_substitute() {
    let mut e = editor("a/b\n");
    keys(&mut e, ":s/a\\/b/c/\n");
    assert_eq!(e.buf().line_text(0), "c");
}
#[test]
fn unicode_picker() {
    let mut e = editor("\n");
    e.all_files = vec!["İx".into()];
    e.file_picker = Some(crate::picker::FilePicker::new(&e.all_files));
    e.mode = Mode::Picker;
    keys(&mut e, "x");
}
#[test]
fn macro_expansion_not_recorded() {
    let mut e = editor("abcdef\n");
    keys(&mut e, "qaxqqb@aq");
    assert_eq!(
        e.macros.get(&'b').unwrap(),
        &vec![Key::Char('@'), Key::Char('a')]
    );
}
#[test]
fn large_count_bounded() {
    let mut e = editor("abc\n");
    keys(&mut e, "999999999999999999999999999999");
    assert!(e.pending.total_count() <= 1_000_000);
}
#[test]
fn failed_save_as_keeps_path() {
    let mut e = editor("abc");
    e.buf_mut().path = Some("old.txt".into());
    assert!(e
        .buf_mut()
        .save_as("/nonexistent-vaayu-dir/new".into())
        .is_err());
    assert_eq!(e.buf().path, Some(PathBuf::from("old.txt")));
}
#[test]
fn save_preserves_bytes_and_detects_external_write() {
    let root = temp();
    let p = root.join("file");
    std::fs::write(&p, b"").unwrap();
    let mut b = Buffer::from_path(p.clone()).unwrap();
    b.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"");
    b.begin_edit();
    b.insert_str(0, 0, "abc");
    b.commit_edit();
    std::fs::write(&p, "external").unwrap();
    assert!(b.save().is_err());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "external");
    b.save_force().unwrap();
    assert_eq!(std::fs::read_to_string(p).unwrap(), "abc");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn identity_and_uri() {
    let root = temp();
    let p = root.join("a #界%.rs");
    let uri = crate::files::uri(&p);
    assert!(uri.contains("%23"));
    assert_eq!(crate::files::from_uri(&uri), Some(p));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn note_roundtrip_private_and_conflict() {
    let root = temp();
    let mut n = crate::notes::Notes::load(&root);
    n.items.push(crate::notes::Note {
        id: 1,
        file: "a.rs".into(),
        start: 0,
        end: 0,
        whole_file: false,
        anchor: "fn main() {}".into(),
        text: "Review this".into(),
        stale: false,
        resolved: false,
    });
    n.dirty = true;
    n.save().unwrap();
    let other = crate::notes::Notes::load(&root);
    assert_eq!(other.items[0].text, "Review this");
    assert_eq!(
        std::fs::read_to_string(root.join(".vaayu/.gitignore")).unwrap(),
        "*\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.join(".vaayu/comments.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::write(root.join(".vaayu/comments.json"), "changed").unwrap();
    assert!(n.save().is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn corrupt_notes_not_overwritten() {
    let root = temp();
    std::fs::create_dir(root.join(".vaayu")).unwrap();
    std::fs::write(root.join(".vaayu/comments.json"), "bad").unwrap();
    let mut n = crate::notes::Notes::load(&root);
    assert!(n.load_error.is_some());
    assert!(n.save().is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn comment_edit_save_and_select_export() {
    let root = temp();
    let p = root.join("a.rs");
    std::fs::write(&p, "first\nsecond\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.notes = crate::notes::Notes::load(&root);
    e.open_file(p.clone()).unwrap();
    keys(&mut e, "Vj,rc");
    assert_eq!(e.notes.items[0].end, 1);
    keys(&mut e, "iReview this\x1b");
    e.save_current().unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "first\nsecond\n");
    e.comments_results();
    let out = e.results.as_ref().unwrap().export(true, &root);
    assert!(out.contains("Review this"));
    assert!(out.contains("a.rs:1"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn note_anchor_relocation() {
    let mut n = crate::notes::Note {
        id: 1,
        file: "a".into(),
        start: 0,
        end: 0,
        whole_file: false,
        anchor: "anchor".into(),
        text: "note".into(),
        stale: false,
        resolved: false,
    };
    crate::notes::Notes::relocate(&mut n, "new\nanchor\n");
    assert_eq!(n.start, 1);
    crate::notes::Notes::relocate(&mut n, "removed\n");
    assert!(n.stale);
}
#[test]
fn results_search_and_quickfix_selection() {
    let mut e = editor("\n");
    let entries = ["Alpha", "Beta", "Gamma"]
        .into_iter()
        .map(crate::results::Entry::text)
        .collect();
    e.show_results(crate::results::Results::new("test", entries));
    keys(&mut e, "/Beta\n");
    assert_eq!(e.results.as_ref().unwrap().cursor, 1);
    e.feed_key(Key::Tab);
    e.feed_key(Key::Ctrl('q'));
    assert_eq!(e.results.as_ref().unwrap().entries.len(), 1);
    assert_eq!(e.results.as_ref().unwrap().entries[0].text, "Beta");
    assert!(e.results.as_ref().unwrap().quickfix);
}
#[test]
fn marks_and_jumps() {
    let mut e = editor("one\ntwo\nthree\n");
    keys(&mut e, "maj");
    keys(&mut e, "'a");
    assert_eq!(e.cursor().0, 0);
    e.feed_key(Key::Ctrl('o'));
    assert_eq!(e.cursor().0, 1);
    e.feed_key(Key::Ctrl('i'));
    assert_eq!(e.cursor().0, 0);
}
#[test]
fn vertical_column_restored() {
    let mut e = editor("abcdef\nx\nabcdef\n");
    keys(&mut e, "4ljj");
    assert_eq!(e.cursor(), (2, 4));
}
#[test]
fn splits_keep_independent_positions() {
    let mut e = editor("one\ntwo\nthree\n");
    e.split_window(true, false);
    keys(&mut e, "jj");
    e.focus_window(0);
    assert_eq!(e.cursor().0, 0);
    e.focus_window(1);
    assert_eq!(e.cursor().0, 2);
    e.close_window();
    assert!(e.windows.is_empty());
}
#[test]
fn utf16_boundaries_and_overlapping_edits() {
    assert_eq!(crate::language::utf16_col("a😀b", 2), 3);
    assert_eq!(crate::language::utf16_to_col("a😀b", 3), 2);
    let rope = ropey::Rope::from_str("a😀b\n");
    let edit = serde_json::json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":3}},"newText":"X"});
    let changes = crate::language::validate_edits(&rope, std::slice::from_ref(&edit)).unwrap();
    assert_eq!(changes[0], (1, 2, "X".into()));
    assert!(crate::language::validate_edits(&rope, &[edit.clone(), edit]).is_err());
}
#[test]
fn workspace_edit_is_transactional() {
    let root = temp();
    let p = root.join("a.rs");
    std::fs::write(&p, "abc\n").unwrap();
    let mut e = editor("");
    e.open_file(p.clone()).unwrap();
    let v = serde_json::json!({"changes":{crate::files::uri(&p):[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"Z"},{"range":{"start":{"line":9,"character":0},"end":{"line":9,"character":1}},"newText":"Z"}]}});
    assert!(e.apply_workspace_edit(&v, None).is_err());
    assert_eq!(e.buf().line_text(0), "abc");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn completion_honors_text_edit() {
    let v = serde_json::json!([{"label":"label","textEdit":{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"actual"}}]);
    assert_eq!(
        crate::lsp::client::extract_completion_items(&v)[0].insert_text,
        "actual"
    );
}
#[test]
fn completion_extracts_filter_text_with_label_fallback() {
    let v = serde_json::json!([
        {"label":"display label","filterText":"needle"},
        {"label":"fallback"}
    ]);
    let items = crate::lsp::client::extract_completion_items(&v);
    assert_eq!(items[0].filter_text, "needle");
    assert_eq!(items[1].filter_text, "fallback");
}
#[test]
fn config_custom_servers() {
    let s = r#"[lsp.clangd]
cmd = ["clangd", "--clang-tidy"]
filetypes = ["c", "cpp"]
root_markers = ["compile_commands.json", ".git"]
[lsp.clangd.settings.test]
enabled = true
[lsp.clangd.init_options]
key = "value"
"#;
    let c: Config = toml::from_str(s).unwrap();
    assert_eq!(c.lsp["clangd"].cmd[1], "--clang-tidy");
    assert_eq!(c.lsp["clangd"].settings["test"]["enabled"], true);
}
#[test]
fn bracketed_paste_is_literal() {
    let mut e = editor("\n");
    e.config.jk_escape = true;
    keys(&mut e, "i");
    e.insert_paste("jk\n:qa!\n");
    assert!(e.buf().rope.to_string().starts_with("jk\n:qa!"));
    assert!(!e.should_quit);
}
#[test]
fn bracketed_paste_in_command_mode_goes_to_the_command_line() {
    // Ctrl+Shift+V while typing `:e <path>` delivers the clipboard as a
    // bracketed paste; it must land in the command line, not the buffer.
    let mut e = editor("hello world\n");
    keys(&mut e, ":e ");
    assert!(matches!(e.mode, crate::mode::Mode::Command(_)));
    // A trailing newline (common when copying a path) is stripped, not run.
    e.insert_paste("/tmp/some/file.rs\n");
    assert_eq!(e.cmdline, "e /tmp/some/file.rs");
    assert!(matches!(e.mode, crate::mode::Mode::Command(_)));
    // The buffer is untouched and stays clean.
    assert_eq!(e.buf().rope.to_string(), "hello world\n");
    assert!(!e.buf().is_modified());
    // A search prompt behaves the same way.
    keys(&mut e, "\x1b/");
    e.insert_paste("needle");
    assert_eq!(e.cmdline, "needle");
    assert_eq!(e.buf().rope.to_string(), "hello world\n");
}
#[test]
fn file_tree_viewport_follows_cursor_and_wheel() {
    let root = temp();
    for i in 0..40 {
        std::fs::write(root.join(format!("f{i:02}.txt")), "x\n").unwrap();
    }
    let mut t = crate::filetree::FileTree::new(root.clone());
    let n = t.nodes.len();
    assert!(n >= 40, "expected the created files as nodes, got {n}");
    // Cursor at the bottom: a height-10 viewport must scroll so it's visible.
    t.cursor = n - 1;
    t.ensure_visible(10);
    assert!(t.top <= t.cursor && t.cursor < t.top + 10, "top={}", t.top);
    // Wheel up past the top clamps to 0 and keeps the cursor on screen.
    t.scroll(-10_000, 10);
    assert_eq!(t.top, 0);
    assert!(t.cursor < 10);
    // Wheel down past the end clamps to the last full screen.
    t.scroll(10_000, 10);
    assert_eq!(t.top, n - 10);
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn outline_viewport_follows_cursor_and_wheel() {
    let mut o = crate::outline::Outline::default();
    o.set_nodes(
        (0..40)
            .map(|i| crate::outline::SymbolNode {
                name: format!("s{i}"),
                kind: "fn",
                line: i,
                col: 0,
                depth: 0,
                end_line: i,
            })
            .collect(),
    );
    let n = o.nodes.len();
    assert_eq!(n, 40);
    o.cursor = n - 1;
    o.ensure_visible(10);
    assert!(o.top <= o.cursor && o.cursor < o.top + 10, "top={}", o.top);
    o.scroll(-10_000, 10);
    assert_eq!(o.top, 0);
    o.scroll(10_000, 10);
    assert_eq!(o.top, n - 10);
}
#[test]
fn showbreak_marks_wrapped_lines_only_when_configured() {
    let long = "abcdefghij ".repeat(6); // ~66 cols: wraps several times at width 18
    let mut e = editor(&format!("{long}\n"));
    e.config.wrap = true;
    let render = |e: &mut Editor| {
        let mut cache = crate::render::FrameCache::new();
        crate::render::prepare_view(e, 18, 10);
        let mut out = Vec::new();
        crate::render::draw(&mut out, e, 18, 10, &mut cache).unwrap();
        String::from_utf8_lossy(&out).into_owned()
    };
    // Default (showbreak empty): wrapped continuation rows show no marker.
    assert!(e.config.showbreak.is_empty());
    assert!(!render(&mut e).contains('↪'), "default should show no wrap marker");
    // Configured: the marker appears on continuation rows.
    e.config.showbreak = "↪".into();
    assert!(render(&mut e).contains('↪'), "showbreak marker should appear");
}
#[test]
fn on_save_trims_trailing_whitespace_and_adds_final_newline() {
    let root = temp();
    let p = root.join("f.txt");
    // Trailing spaces, a trailing tab (leading tab kept), and a last line with
    // trailing whitespace and NO final newline.
    std::fs::write(&p, "abc   \n\tdef\t \nno newline eof   ").unwrap();
    let mut e = editor("");
    e.config.trim_trailing_whitespace = true;
    e.config.insert_final_newline = true;
    e.open_file(p.clone()).unwrap();
    e.save_current().unwrap();
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "abc\n\tdef\nno newline eof\n"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn final_newline_is_a_noop_on_empty_and_already_terminated_buffers() {
    let root = temp();
    let empty = root.join("empty.txt");
    std::fs::write(&empty, "").unwrap();
    let mut e = editor("");
    e.config.insert_final_newline = true;
    e.open_file(empty.clone()).unwrap();
    e.save_current().unwrap();
    assert_eq!(std::fs::read_to_string(&empty).unwrap(), ""); // stays empty

    let done = root.join("done.txt");
    std::fs::write(&done, "line\n").unwrap();
    e.open_file(done.clone()).unwrap();
    e.save_current().unwrap();
    assert_eq!(std::fs::read_to_string(&done).unwrap(), "line\n"); // no double newline
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn on_save_defaults_do_not_modify_content() {
    let root = temp();
    let p = root.join("f.txt");
    let original = "abc   \nno newline   ";
    std::fs::write(&p, original).unwrap();
    let mut e = editor(""); // defaults: trim=false, final_newline=false, no autocmds
    e.open_file(p.clone()).unwrap();
    e.save_current().unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn window_split_resize_and_equalize() {
    let mut e = editor("hello\nworld\n");
    e.split_window(true, false); // vertical split: two panes side by side
    let w0 = e.pane_rects(80, 24)[0].width;
    e.feed_key(Key::Ctrl('w'));
    e.feed_key(Key::Char('>'));
    let w1 = e.pane_rects(80, 24)[0].width;
    assert_ne!(w1, w0, "Ctrl-W > should change the split widths");
    e.feed_key(Key::Ctrl('w'));
    e.feed_key(Key::Char('='));
    let w2 = e.pane_rects(80, 24)[0].width;
    assert_eq!(w2, w0, "Ctrl-W = should restore an even split");
}
#[test]
fn focus_gained_autoreloads_unmodified_but_not_dirty() {
    let root = temp();
    let p = root.join("f.txt");
    std::fs::write(&p, "original\n").unwrap();
    let mut e = editor("");
    e.open_file(p.clone()).unwrap();
    assert!(!e.buf().changed_on_disk());
    // External change while the buffer is clean -> auto-reload on focus.
    std::fs::write(&p, "changed externally\n").unwrap();
    assert!(e.buf().changed_on_disk());
    e.on_focus_gained();
    assert_eq!(e.buf().rope.to_string(), "changed externally\n");
    // Make a local edit, then change disk again -> must NOT clobber.
    e.set_cursor(0, 0);
    e.feed_key(Key::Char('i'));
    e.feed_key(Key::Char('Z'));
    e.feed_key(Key::Esc);
    std::fs::write(&p, "disk overwrite\n").unwrap();
    e.on_focus_gained();
    assert!(
        e.buf().rope.to_string().contains('Z'),
        "a dirty buffer must not be reloaded"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn todo_index_lists_todo_and_fixme() {
    let root = temp();
    std::fs::write(root.join("a.rs"), "fn f() {} // TODO: wire it up\n").unwrap();
    std::fs::write(root.join("b.py"), "# FIXME: broken\nx = 1\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    crate::command::run_ex(&mut e, "todo");
    let start = std::time::Instant::now();
    loop {
        e.poll_jobs();
        let ready = e.results.as_ref().is_some_and(|r| !r.entries.is_empty());
        if ready || start.elapsed() > std::time::Duration::from_secs(5) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let r = e.results.as_ref().expect("todo results");
    let text = r
        .entries
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("TODO"), "{text}");
    assert!(text.contains("FIXME"), "{text}");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn earlier_later_undo_redo_by_count() {
    let mut e = editor("start\n");
    // Three separate edits (each a change/undo step).
    for word in ["a", "b", "c"] {
        e.set_cursor(0, 0);
        for ch in word.chars() {
            e.feed_key(Key::Char('i'));
            e.feed_key(Key::Char(ch));
            e.feed_key(Key::Esc);
        }
    }
    let after_edits = e.buf().rope.to_string();
    assert!(after_edits.contains("cba") || after_edits.starts_with("cbastart"), "{after_edits}");
    crate::command::run_ex(&mut e, "earlier 2"); // undo two changes
    let back2 = e.buf().rope.to_string();
    crate::command::run_ex(&mut e, "later 1"); // redo one
    let fwd1 = e.buf().rope.to_string();
    assert_ne!(back2, after_edits, "earlier 2 should undo");
    assert_ne!(fwd1, back2, "later 1 should redo");
}
#[test]
fn checkhealth_reports_sections() {
    let mut e = editor("");
    crate::command::run_ex(&mut e, "checkhealth");
    let r = e.results.as_ref().expect("checkhealth opens a results list");
    assert_eq!(r.title, "Health");
    let text = r
        .entries
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("External tools"), "{text}");
    assert!(text.contains("git"), "{text}");
    assert!(text.contains("Tree-sitter grammars"), "{text}");
    assert!(text.contains("rust"), "{text}");
}
#[test]
fn move_lines_down_up_count_and_undo() {
    let mut e = editor("aaa\nbbb\nccc\n");
    e.set_cursor(0, 1);
    for k in "]e".chars() {
        e.feed_key(Key::Char(k));
    }
    assert_eq!(e.buf().rope.to_string(), "bbb\naaa\nccc\n");
    assert_eq!(e.cursor().0, 1, "cursor follows the moved line");
    for k in "[e".chars() {
        e.feed_key(Key::Char(k));
    }
    assert_eq!(e.buf().rope.to_string(), "aaa\nbbb\nccc\n");
    assert_eq!(e.cursor().0, 0);
    // Count: 2]e moves the line down twice.
    e.set_cursor(0, 0);
    for k in "2]e".chars() {
        e.feed_key(Key::Char(k));
    }
    assert_eq!(e.buf().rope.to_string(), "bbb\nccc\naaa\n");
    assert_eq!(e.cursor().0, 2);
    // No-op at the bottom boundary.
    for k in "]e".chars() {
        e.feed_key(Key::Char(k));
    }
    assert_eq!(e.buf().rope.to_string(), "bbb\nccc\naaa\n");
    // One move is one undo step.
    e.buf_mut().undo();
    assert_eq!(e.buf().rope.to_string(), "aaa\nbbb\nccc\n");
}
#[test]
fn tree_textobjects_function_and_class() {
    let src = "struct S {\n    x: i32,\n}\nfn foo() {\n    let a = 1;\n    let b = 2;\n}\n";
    let setup = |src: &str| {
        let mut e = editor(src);
        let mut syn = crate::syntax::Syntax::new(crate::syntax::Lang::Rust).unwrap();
        syn.reparse(std::rc::Rc::from(src));
        e.syntax = Some(syn);
        e
    };
    // daf inside foo deletes the whole function.
    let mut e = setup(src);
    e.set_cursor(4, 8);
    for k in "daf".chars() {
        e.feed_key(Key::Char(k));
    }
    let after = e.buf().rope.to_string();
    assert!(!after.contains("fn foo"), "daf should delete the function:\n{after}");
    assert!(after.contains("struct S"), "other items remain:\n{after}");
    // dif inside foo clears the body but keeps the signature.
    let mut e = setup(src);
    e.set_cursor(4, 8);
    for k in "dif".chars() {
        e.feed_key(Key::Char(k));
    }
    let after = e.buf().rope.to_string();
    assert!(after.contains("fn foo()"), "dif keeps the signature:\n{after}");
    assert!(!after.contains("let a = 1"), "dif clears the body:\n{after}");
    // dac on the struct deletes it.
    let mut e = setup(src);
    e.set_cursor(1, 6);
    for k in "dac".chars() {
        e.feed_key(Key::Char(k));
    }
    let after = e.buf().rope.to_string();
    assert!(!after.contains("struct S"), "dac should delete the struct:\n{after}");
    assert!(after.contains("fn foo"), "the function remains:\n{after}");
}
#[test]
fn incremental_selection_expands_and_shrinks() {
    let src = "fn main() {\n    let x = foo(1, 2);\n}\n";
    let mut e = editor(src);
    let mut syn = crate::syntax::Syntax::new(crate::syntax::Lang::Rust).expect("rust grammar");
    syn.reparse(std::rc::Rc::from(src));
    e.syntax = Some(syn);
    e.set_cursor(1, 16); // the `1` inside foo(1, 2)
    let sel_len = |e: &Editor| {
        let (cl, cc) = e.cursor();
        let cur = e.buf().char_idx(cl, cc);
        let (al, ac) = e.visual_anchor.unwrap();
        let anc = e.buf().char_idx(al, ac);
        cur.max(anc) - cur.min(anc) + 1
    };
    e.expand_selection();
    assert!(matches!(e.mode, crate::mode::Mode::Visual(_)));
    let a = sel_len(&e);
    e.expand_selection();
    let b = sel_len(&e);
    assert!(b > a, "expand should grow the selection: {a} -> {b}");
    e.expand_selection();
    let c = sel_len(&e);
    assert!(c > b, "expand should keep growing: {b} -> {c}");
    e.shrink_selection();
    assert_eq!(sel_len(&e), b, "shrink returns to the previous selection");
    e.shrink_selection();
    assert_eq!(sel_len(&e), a);
}
fn km(entries: &[(&str, &str, &str)]) -> Vec<crate::keymap::Keymap> {
    let cfgs: Vec<crate::config::KeymapCfg> = entries
        .iter()
        .map(|(m, l, r)| crate::config::KeymapCfg {
            mode: (*m).into(),
            lhs: (*l).into(),
            rhs: (*r).into(),
        })
        .collect();
    crate::keymap::build(&cfgs, ',')
}
#[test]
fn keymap_single_key_replays_keys() {
    let mut e = editor("hello world\n");
    e.keymaps = km(&[("n", "Y", "y$")]);
    e.set_cursor(0, 0);
    e.feed_key(Key::Char('Y')); // remapped to y$
    assert_eq!(
        e.registers.get(None).map(|r| r.text.clone()).as_deref(),
        Some("hello world")
    );
}
#[test]
fn keymap_ex_command_runs() {
    let mut e = editor("foo\n");
    e.keymaps = km(&[("n", "Q", ":s/foo/bar/<CR>")]);
    e.set_cursor(0, 0);
    e.feed_key(Key::Char('Q'));
    assert_eq!(e.buf().line_text(0), "bar");
}
#[test]
fn keymap_is_mode_isolated() {
    let mut e = editor("x\n");
    e.keymaps = km(&[("n", "Y", "y$")]);
    e.set_cursor(0, 0);
    e.feed_key(Key::Char('i')); // insert mode
    e.feed_key(Key::Char('Y')); // literal, not remapped
    e.feed_key(Key::Esc);
    assert!(e.buf().line_text(0).contains('Y'));
}
#[test]
fn keymap_rhs_is_not_remapped_noremap() {
    let mut e = editor("hello\n");
    e.keymaps = km(&[("n", "a", "b"), ("n", "b", "x")]);
    e.set_cursor(0, 2);
    e.feed_key(Key::Char('a')); // a -> b (word-back), NOT chained to x (delete)
    assert_eq!(e.buf().line_text(0), "hello", "rhs must not be remapped");
    assert_eq!(e.cursor(), (0, 0), "the b motion still ran");
}
#[test]
fn keymap_leader_remap_runs_command() {
    let mut e = editor("foo\n");
    e.keymaps = km(&[("n", "<leader>x", ":s/foo/bar/<CR>")]);
    e.set_cursor(0, 0);
    e.feed_key(Key::Char(',')); // leader
    e.feed_key(Key::Char('x'));
    assert_eq!(e.buf().line_text(0), "bar");
}
#[test]
fn keymap_invalid_notation_is_ignored() {
    // Empty lhs/rhs and unknown tokens don't produce a mapping or panic.
    let maps = km(&[("n", "", "y$"), ("n", "Z", "")]);
    assert!(maps.is_empty());
}
#[test]
fn inccommand_highlights_substitute_pattern_for_ranges_and_delimiters() {
    for cmd in ["s/foo/x/", "%s/foo/x/", "1,3s/foo/x/", "s#foo#x#"] {
        let mut e = editor("foo bar\nbaz foo\nfoo\n");
        e.feed_key(Key::Char(':'));
        for c in cmd.chars() {
            e.feed_key(Key::Char(c));
        }
        assert_eq!(e.incsearch.as_deref(), Some("foo"), "cmd {cmd}");
        e.feed_key(Key::Esc);
        assert!(e.incsearch.is_none(), "Esc clears the preview for {cmd}");
    }
}
#[test]
fn inccommand_ignores_non_substitute_and_empty_pattern() {
    let mut e = editor("foo\n");
    e.feed_key(Key::Char(':'));
    for c in "set number".chars() {
        e.feed_key(Key::Char(c));
    }
    assert!(e.incsearch.is_none(), "a non-substitute must not preview");
    e.feed_key(Key::Esc);
    let mut e = editor("foo\n");
    e.feed_key(Key::Char(':'));
    for c in "%s//".chars() {
        e.feed_key(Key::Char(c));
    }
    assert!(e.incsearch.is_none(), "empty pattern must not preview");
    e.feed_key(Key::Esc);
}
#[test]
fn inccommand_cleared_after_submit_and_substitute_applies() {
    let mut e = editor("foo\n");
    e.feed_key(Key::Char(':'));
    for c in "s/foo/bar/".chars() {
        e.feed_key(Key::Char(c));
    }
    assert_eq!(e.incsearch.as_deref(), Some("foo"));
    e.feed_key(Key::Enter);
    assert!(e.incsearch.is_none(), "submit clears the preview");
    assert_eq!(e.buf().line_text(0), "bar", "the substitution applied");
}
#[test]
fn cmdline_tab_completes_unique_command_name() {
    let mut e = editor("");
    e.feed_key(Key::Char(':'));
    for c in "registe".chars() {
        e.feed_key(Key::Char(c));
    }
    e.feed_key(Key::Tab);
    assert_eq!(e.cmdline, "registers");
}
#[test]
fn cmdline_tab_cycles_and_backtab_reverses() {
    let mut e = editor("");
    e.feed_key(Key::Char(':'));
    for c in "git".chars() {
        e.feed_key(Key::Char(c));
    }
    e.feed_key(Key::Tab);
    let first = e.cmdline.clone();
    e.feed_key(Key::Tab);
    let second = e.cmdline.clone();
    assert_ne!(first, second, "Tab should cycle to a different git* command");
    assert!(first.starts_with("git") && second.starts_with("git"));
    e.feed_key(Key::BackTab);
    assert_eq!(e.cmdline, first, "BackTab returns to the previous candidate");
    // A non-Tab key ends the cycle.
    e.feed_key(Key::Char('x'));
    assert!(e.cmdline_completion_index.is_none());
}
#[test]
fn cmdline_tab_completes_absolute_file_path() {
    let root = temp();
    std::fs::write(root.join("alpha.txt"), "x").unwrap();
    let mut e = editor("");
    e.feed_key(Key::Char(':'));
    for c in format!("e {}/al", root.display()).chars() {
        e.feed_key(Key::Char(c));
    }
    e.feed_key(Key::Tab);
    assert_eq!(e.cmdline, format!("e {}/alpha.txt", root.display()));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn incsearch_previews_first_match_and_esc_restores() {
    let mut e = editor("alpha\nbeta\ngamma\ndelta\n");
    e.set_cursor(0, 0);
    e.feed_key(Key::Char('/'));
    for c in "gamma".chars() {
        e.feed_key(Key::Char(c));
    }
    assert_eq!(e.cursor().0, 2, "incsearch should preview the match line");
    assert_eq!(e.incsearch.as_deref(), Some("gamma"));
    e.feed_key(Key::Esc);
    assert_eq!(e.cursor(), (0, 0), "Esc restores the origin");
    assert!(e.incsearch.is_none());
}
#[test]
fn incsearch_enter_lands_on_previewed_match() {
    let mut e = editor("alpha\nbeta\ngamma\n");
    e.set_cursor(0, 0);
    e.feed_key(Key::Char('/'));
    for c in "gamma".chars() {
        e.feed_key(Key::Char(c));
    }
    e.feed_key(Key::Enter);
    assert_eq!(e.cursor().0, 2);
    assert!(e.incsearch.is_none());
    assert_eq!(e.last_search.as_ref().unwrap().0, "gamma");
}
#[test]
fn incsearch_invalid_regex_is_a_noop() {
    let mut e = editor("a[b]c\n");
    e.set_cursor(0, 0);
    e.feed_key(Key::Char('/'));
    e.feed_key(Key::Char('[')); // unclosed char class -> invalid regex
    assert_eq!(e.cursor(), (0, 0), "invalid regex mid-typing must not move");
    assert!(e.incsearch.is_none());
    e.feed_key(Key::Esc);
    assert_eq!(e.cursor(), (0, 0));
}
#[test]
fn incsearch_empty_query_returns_to_origin() {
    let mut e = editor("alpha\nbeta\n");
    e.set_cursor(1, 2);
    e.feed_key(Key::Char('/'));
    e.feed_key(Key::Char('a')); // previews a match
    e.feed_key(Key::Backspace); // empty again
    assert_eq!(e.cursor(), (1, 2), "emptying the query returns to origin");
    assert!(e.incsearch.is_none());
    e.feed_key(Key::Esc);
}
#[test]
fn registers_list_is_sorted_and_reflects_yanks() {
    let mut r = crate::registers::Registers::new(false);
    r.set(Some('b'), "bee".into(), false);
    r.set(Some('a'), "ay".into(), false);
    let list = r.list();
    let names: Vec<char> = list.iter().map(|(c, _)| *c).collect();
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "sorted: {names:?}");
    assert!(names.contains(&'a') && names.contains(&'b') && names.contains(&'"'));
    assert_eq!(list.iter().find(|(c, _)| *c == 'a').unwrap().1.text, "ay");
}
#[test]
fn set_message_history_dedups_and_bounds() {
    let mut e = editor("");
    e.messages.clear();
    e.set_message("one");
    e.set_message("one"); // consecutive duplicate is not re-added
    e.set_message("two");
    assert_eq!(e.messages, vec!["one".to_string(), "two".to_string()]);
    for i in 0..600 {
        e.set_message(format!("m{i}"));
    }
    assert!(e.messages.len() <= 500);
    assert_eq!(e.messages.last().unwrap(), "m599");
}
#[test]
fn marks_command_lists_marks_with_location() {
    let mut e = editor("l0\nl1\nl2\n");
    e.set_cursor(2, 0);
    e.set_mark('a');
    crate::command::run_ex(&mut e, "marks");
    let r = e.results.as_ref().expect("marks should open a results list");
    assert_eq!(r.title, "Marks");
    let entry = r
        .entries
        .iter()
        .find(|e| e.text.starts_with("'a"))
        .expect("mark a listed");
    assert_eq!(entry.line, 2);
}
#[test]
fn autocmd_runs_matching_command_on_bufwritepre_only_for_matching_pattern() {
    let root = temp();
    let mut e = editor("");
    e.config.autocmd = vec![crate::config::Autocmd {
        event: "BufWritePre".into(),
        pattern: "*.rs".into(),
        command: "s/hello/world/".into(),
    }];
    // Matching pattern: the substitute runs before the write.
    let rs = root.join("f.rs");
    std::fs::write(&rs, "hello\n").unwrap();
    e.open_file(rs.clone()).unwrap();
    e.save_current().unwrap();
    assert_eq!(std::fs::read_to_string(&rs).unwrap(), "world\n");
    // Non-matching pattern (*.rs vs .txt): the command must not run.
    let txt = root.join("f.txt");
    std::fs::write(&txt, "hello\n").unwrap();
    e.open_file(txt.clone()).unwrap();
    e.save_current().unwrap();
    assert_eq!(std::fs::read_to_string(&txt).unwrap(), "hello\n");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn render_unicode_wrap_controls_and_cache() {
    let mut e = editor("界\tabcdefghijklmnopqrstuvwxyz\n\x1b[31m\n");
    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 18, 10);
    let mut a = Vec::new();
    crate::render::draw(&mut a, &e, 18, 10, &mut cache).unwrap();
    assert!(!String::from_utf8_lossy(&a).contains("\x1b[31m"));
    let mut b = Vec::new();
    crate::render::draw(&mut b, &e, 18, 10, &mut cache).unwrap();
    assert!(b.len() < a.len() / 3);
    e.config.wrap = false;
    let mut c = Vec::new();
    crate::render::draw(&mut c, &e, 18, 10, &mut cache).unwrap();
    assert!(c.len() > b.len());
}
#[test]
fn search_match_forces_a_readable_foreground_not_arbitrary_syntax_color() {
    let mut e = editor("needle in a haystack\n");
    e.last_search = Some(("needle".to_string(), true));
    e.hl_search = true;
    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 40, 10);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 40, 10, &mut cache).unwrap();
    let text = String::from_utf8_lossy(&out);
    // DarkYellow background (256-color SGR "48;5;3") and a forced Black
    // foreground ("38;5;0") together -- not whatever arbitrary syntax
    // color the matched token would otherwise have had, which could
    // clash badly against a saturated highlight background.
    assert!(
        text.contains("48;5;3"),
        "search match should have a DarkYellow background. Got: {text:?}"
    );
    assert!(
        text.contains("38;5;0"),
        "search match should force a Black (readable) foreground. Got: {text:?}"
    );
}
#[test]
fn tiny_terminal_does_not_panic() {
    let mut e = editor("界\n");
    for cols in 0..5 {
        for rows in 0..5 {
            crate::render::prepare_view(&mut e, cols, rows);
            crate::render::draw(
                &mut Vec::new(),
                &e,
                cols as u16,
                rows as u16,
                &mut crate::render::FrameCache::new(),
            )
            .unwrap();
        }
    }
}
#[test]
fn empty_completion_does_not_swallow_enter() {
    let mut e = editor("abc\n");
    keys(&mut e, "A");
    e.completion = Some(crate::completion::CompletionState {
        start: (0, 0),
        items: vec![],
        selected: 0,
        request_id: 1,
    });
    e.feed_key(Key::Enter);
    assert_eq!(e.cursor(), (1, 0));
}
#[test]
fn same_file_deduplicates_before_creation() {
    let root = temp();
    let p = root.join("new.rs");
    let mut e = editor("");
    e.open_file(p.clone()).unwrap();
    e.open_file(root.join("./new.rs")).unwrap();
    assert_eq!(
        e.buffers
            .iter()
            .filter(|b| b.path.as_ref() == Some(&p))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn mock_lsp_config_sync_and_features() {
    let root = temp();
    let file = root.join("fixture.rs");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "abc\n").unwrap();
    let mut e = editor("");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["rust".into()],
            settings: serde_json::json!({"test":{"enabled":true}}),
            init_options: serde_json::json!({"custom":123}),
            ..Default::default()
        },
    );
    e.open_file(file.clone()).unwrap();
    e.sync_lsp();
    // Mutate before initialization completes: didOpen must carry newest text.
    e.buf_mut().begin_edit();
    e.buf_mut().insert_str(0, 3, "Z");
    e.buf_mut().commit_edit();
    e.sync_lsp();
    fn wait(e: &mut Editor, predicate: impl Fn(&Editor) -> bool) {
        let start = std::time::Instant::now();
        while !predicate(e) {
            e.poll_lsp_events();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "LSP timeout: {}",
                e.message
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    wait(&mut e, |e| !e.diagnostics.is_empty());
    assert_eq!(e.diagnostic_results().entries.len(), 1);
    e.request_language("outline", None);
    wait(&mut e, |e| {
        e.results.as_ref().is_some_and(|r| r.title == "outline")
    });
    assert_eq!(e.results.as_ref().unwrap().entries[0].text, "symbol");
    e.export_quickfix();
    assert_eq!(e.quickfix.as_ref().unwrap().entries.len(), 1);
    e.enter_normal();
    e.request_language("format", None);
    wait(&mut e, |e| e.buf().line_text(0) == "FMTZ");
    assert!(e.buf().is_modified());
    e.request_language("rename", Some("REN"));
    wait(&mut e, |e| e.buf().line_text(0) == "RENZ");
    e.request_language("actions", None);
    wait(&mut e, |e| {
        e.results
            .as_ref()
            .is_some_and(|r| r.title == "Code actions")
    });
    e.open_result();
    assert_eq!(e.buf().line_text(0), "FIXZ");
    e.buf_mut().begin_edit();
    e.enter_insert();
    e.set_cursor_insert(0, 3);
    e.update_completion();
    wait(&mut e, |e| {
        e.completion.as_ref().is_some_and(|c| {
            c.items
                .iter()
                .any(|i| i.source == crate::completion::Source::Lsp)
        })
    });
    e.feed_key(Key::Tab);
    wait(&mut e, |e| e.buf().line_text(0) == "completedZ");
    crate::insert::leave_insert(&mut e);
    let messages: Vec<serde_json::Value> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(messages
        .iter()
        .any(|m| m["method"] == "initialize"
            && m["params"]["initializationOptions"]["custom"] == 123));
    assert!(messages
        .iter()
        .any(|m| m["id"] == "config-request" && m["result"][0]["enabled"] == true));
    assert!(messages
        .iter()
        .any(|m| m["method"] == "textDocument/didOpen"
            && m["params"]["textDocument"]["text"] == "abcZ\n"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "abc\n");
    drop(e);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn json_language_server_gets_bundled_schemastore_defaults() {
    let root = temp();
    let file = root.join("fixture.json");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "{}\n").unwrap();
    let mut e = editor("");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["json".into()],
            ..Default::default()
        },
    );
    e.open_file(file).unwrap();
    e.sync_lsp();
    // The log records only what the mock reads from stdin, i.e. what the
    // editor sends -- the mock's own outgoing config-request is never
    // logged, only the editor's reply to it (same id, carrying `result`).
    let start = std::time::Instant::now();
    let mut reply = None;
    while reply.is_none() {
        e.poll_lsp_events();
        reply = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|m| m["id"] == "config-request" && m.get("result").is_some());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "editor never replied to the server's own workspace/configuration request"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let reply = reply.unwrap();
    let json_schemas = reply["result"][1].as_array().expect(
        "a JSON-language client should reply with its bundled json.schemas array \
         for the server's own \"json.schemas\" section request",
    );
    assert!(
        json_schemas.len() > 500,
        "expected the real bundled SchemaStore catalog, got {} entries",
        json_schemas.len()
    );
    assert!(
        json_schemas.iter().any(|s| s["fileMatch"]
            .as_array()
            .is_some_and(|fm| fm.iter().any(|p| p == "package.json"))),
        "package.json should be one of the bundled associations"
    );
    assert!(
        reply["result"][2].is_null(),
        "a JSON-language client's settings shouldn't carry yaml.schemas at all, \
         got: {}",
        reply["result"][2]
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn user_configured_json_schemas_override_the_bundled_default() {
    let root = temp();
    let file = root.join("fixture.json");
    let log = root.join("messages.jsonl");
    std::fs::write(&file, "{}\n").unwrap();
    let mut e = editor("");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    e.config.lsp.insert(
        "fixture".into(),
        crate::config::LspServer {
            cmd: vec![
                "python3".into(),
                fixture.display().to_string(),
                log.display().to_string(),
            ],
            filetypes: vec!["json".into()],
            settings: serde_json::json!({"json": {"schemas": [{"fileMatch": ["custom.json"], "url": "custom://schema"}]}}),
            ..Default::default()
        },
    );
    e.open_file(file).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    let mut reply = None;
    while reply.is_none() {
        e.poll_lsp_events();
        reply = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|m| m["id"] == "config-request" && m.get("result").is_some());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "editor never replied to the server's own workspace/configuration request"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let reply = reply.unwrap();
    assert_eq!(
        reply["result"][1],
        serde_json::json!([{"fileMatch": ["custom.json"], "url": "custom://schema"}]),
        "the user's own json.schemas config should replace the bundled default \
         entirely, not merge with it"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn show_tools_lists_every_known_tool_with_an_install_action() {
    let mut e = editor("");
    e.show_tools();
    let r = e
        .results
        .as_ref()
        .expect(":tools should show a Results list");
    assert_eq!(r.title, "Tools");
    assert_eq!(r.entries.len(), crate::tools::TOOLS.len());
    for entry in &r.entries {
        let action = entry
            .action
            .as_ref()
            .expect("every tool entry should carry an install action")
            .get("_vaayu_tool_install")
            .expect("action should be tagged _vaayu_tool_install");
        assert!(action["name"].is_string());
        assert!(action["install"].is_string());
        assert!(action["installed"].is_boolean());
    }
}
#[test]
fn opening_an_already_installed_tool_entry_just_reports_that_and_installs_nothing() {
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    let entry = {
        let mut e = crate::results::Entry::text("fixture-tool (fixture) — ✓ installed");
        e.action = Some(serde_json::json!({"_vaayu_tool_install": {
            "installed": true, "name": "fixture-tool", "install": "echo should_not_run",
        }}));
        e
    };
    e.show_results(crate::results::Results::new("Tools", vec![entry]));
    let terminals_before = e.terminals.len();
    e.open_result();
    assert_eq!(e.message, "fixture-tool is already installed");
    assert_eq!(
        e.terminals.len(),
        terminals_before,
        "an already-installed tool must never spawn an install terminal"
    );
}
#[test]
fn opening_an_uninstalled_tool_entry_runs_its_install_command_in_a_new_terminal() {
    let mut e = editor("");
    e.screen_rows = 24;
    e.screen_cols = 80;
    let entry = {
        let mut e = crate::results::Entry::text("fixture-tool (fixture) — ✗ not installed");
        e.action = Some(serde_json::json!({"_vaayu_tool_install": {
            "installed": false, "name": "fixture-tool", "install": "echo installed_xyz_fixture",
        }}));
        e
    };
    e.show_results(crate::results::Results::new("Tools", vec![entry]));
    e.open_result();
    assert_eq!(e.mode, Mode::Terminal);
    let id = e
        .active_terminal_id()
        .expect("running an install command should open a terminal pane");
    let start = std::time::Instant::now();
    loop {
        let seen = e
            .terminals
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .with_screen(|s| s.contents().contains("installed_xyz_fixture"));
        if seen {
            break;
        }
        assert!(
            start.elapsed().as_secs() < 5,
            "install command output never appeared in the terminal"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
#[test]
fn markdown_table_code_and_alignment() {
    let lines = crate::markdown::render("| left | right |\n|:---|---:|\n| `code` | x |\n");
    let text: Vec<String> = lines
        .iter()
        .map(|l| l.iter().map(|s| s.text.as_str()).collect())
        .collect();
    let row = text.iter().find(|l| l.contains("code")).unwrap();
    assert!(row.contains('│'));
    assert!(row.contains("    x"));
}
#[test]
fn grep_background_results() {
    let root = temp();
    std::fs::write(root.join("a.rs"), "needle\nother\nneedle\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_grep("needle");
    let start = std::time::Instant::now();
    while e.results.as_ref().unwrap().busy {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(e.results.as_ref().unwrap().entries.len(), 2);
    e.export_quickfix();
    assert_eq!(e.quickfix.as_ref().unwrap().entries[1].line, 2);
    drop(e);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn live_grep_filter_survives_a_fresh_batch_of_results() {
    let root = temp();
    std::fs::write(root.join("a.rs"), "needle_apple\nneedle_banana\nother\n").unwrap();
    std::fs::write(root.join("b.rs"), "needle_apple\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_grep("needle");
    let start = std::time::Instant::now();
    while e.results.as_ref().unwrap().busy {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(e.results.as_ref().unwrap().entries.len(), 3);

    // `f` (filter) should work on a live list, not just a frozen one.
    {
        let r = e.results.as_mut().unwrap();
        r.filter = "apple".into();
        r.apply_filter();
    }
    assert_eq!(e.results.as_ref().unwrap().entries.len(), 2);

    // A fresh batch of ripgrep results (e.g. from typing more of the
    // query) must be re-derived through the still-active filter, not
    // overwrite it wholesale.
    e.schedule_grep();
    let start = std::time::Instant::now();
    while e.results.as_ref().unwrap().busy {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let r = e.results.as_ref().unwrap();
    assert_eq!(
        r.all_entries.len(),
        3,
        "the fresh batch itself should be unfiltered"
    );
    assert_eq!(
        r.entries.len(),
        2,
        "the active filter should still narrow the fresh batch"
    );
    assert!(r.entries.iter().all(|e| e.text.contains("apple")));
    drop(e);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn block_delete_and_change() {
    let mut e = editor("abcd\nabcd\nabcd\n");
    keys(&mut e, "l");
    e.feed_key(Key::Ctrl('v'));
    keys(&mut e, "jld");
    assert_eq!(e.buf().rope.to_string(), "ad\nad\nabcd\n");
    keys(&mut e, "u");
    e.set_cursor(0, 1);
    e.feed_key(Key::Ctrl('v'));
    keys(&mut e, "jlcX\x1b");
    assert_eq!(e.buf().rope.to_string(), "aXd\naXd\nabcd\n");
}
#[test]
fn counted_insert() {
    let mut e = editor("\n");
    keys(&mut e, "3iab\x1b");
    assert_eq!(e.buf().line_text(0), "ababab");
    keys(&mut e, "u");
    assert_eq!(e.buf().line_text(0), "");
}
#[test]
fn counted_paragraph() {
    let mut e = editor("a\n\nb\n\nc\n");
    keys(&mut e, "2}");
    assert_eq!(e.cursor().0, 3);
}
#[test]
fn multi_comment_save_and_subset() {
    let root = temp();
    let p = root.join("a.txt");
    std::fs::write(&p, "abc\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.notes = crate::notes::Notes::load(&root);
    e.open_file(p.clone()).unwrap();
    e.new_note(false);
    keys(&mut e, "iFirst note\x1b");
    e.open_file(p.clone()).unwrap();
    e.new_note(true);
    keys(&mut e, "iSecond note\x1b");
    e.save_notes().unwrap();
    assert_eq!(crate::notes::Notes::load(&root).items.len(), 2);
    e.comments_results();
    e.results.as_mut().unwrap().selected.insert(1);
    let text = e.results.as_ref().unwrap().export(false, &root);
    assert!(text.contains("Second note"));
    assert!(!text.contains("First note"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn restore_recovery_does_not_write_source() {
    let root = temp();
    let p = root.join("a.txt");
    std::fs::write(&p, "disk").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.restore_recovery(serde_json::json!({"path":p,"text":"draft","line":0,"col":0}));
    assert_eq!(e.buf().line_text(0), "draft");
    assert!(e.buf().is_modified());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "disk");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn completion_additional_edits_move_caret_correctly() {
    let mut e = editor("abc\n");
    keys(&mut e, "A");
    e.completion = Some(crate::completion::CompletionState {
        start: (0, 0),
        selected: 0,
        request_id: 1,
        items: vec![crate::completion::Item {
            snippet: false,
            raw: None,
            kind: None,
            label: "completed".into(),
            insert_text: "completed".into(),
            source: crate::completion::Source::Lsp,
            detail: None,
            edit: Some(
                serde_json::json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"completed"}),
            ),
            additional: vec![
                serde_json::json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"import x;\n"}),
            ],
        }],
    });
    e.feed_key(Key::Tab);
    assert_eq!(e.buf().rope.to_string(), "import x;\ncompleted\n");
    assert_eq!(e.cursor(), (1, 9));
}
#[test]
fn timed_jk_macro_replays_literal_text() {
    let mut e = editor("\n");
    e.config.jk_escape = true;
    keys(&mut e, "qaij");
    e.flush_pending_jk();
    keys(&mut e, "k\x1bq");
    assert!(e.macros[&'a'].contains(&Key::Literal('j')));
    keys(&mut e, "@a");
    assert_eq!(e.mode, Mode::Normal);
    assert_eq!(e.buf().line_text(0), "jjkk");
}
#[test]
fn invalid_search_preserves_previous_pattern() {
    let mut e = editor("abc\n");
    keys(&mut e, "/abc\n");
    let old = e.last_search.clone();
    keys(&mut e, "/[\n");
    assert_eq!(e.last_search, old);
    assert!(e.message.starts_with("Invalid search"));
}
#[test]
fn unicode_case_expansion() {
    let mut e = editor("ß\n");
    keys(&mut e, "~");
    assert_eq!(e.buf().line_text(0), "SS");
}
#[test]
fn typescript_and_fenced_code_highlight() {
    assert_eq!(
        crate::syntax::lang_for_extension("ts"),
        Some(crate::syntax::Lang::TypeScript)
    );
    let lines = crate::markdown::render("```rust\nfn main() { let x = 42; }\n```\n");
    assert!(lines
        .iter()
        .flatten()
        .any(|s| s.style.syntax == Some(crate::syntax::HlClass::Keyword) && s.text.contains("fn")));
}
#[test]
fn git_hunk_stage_and_unstage() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    let new = old
        .replace("line 1\n", "changed one\n")
        .replace("line 25\n", "changed two\n");
    std::fs::write(&file, &new).unwrap();
    let r = crate::git_tools::hunks(&root, &file, false).unwrap();
    assert_eq!(r.entries.len(), 2);
    let patch = r.entries[0].action.as_ref().unwrap()["_vaayu_git_patch"]
        .as_str()
        .unwrap();
    crate::git_tools::apply_patch(&root, patch, false, true).unwrap();
    let staged = git(&["show", ":sample.txt"]);
    assert!(staged.contains("changed one"));
    assert!(!staged.contains("changed two"));
    let r = crate::git_tools::hunks(&root, &file, true).unwrap();
    let patch = r.entries[0].action.as_ref().unwrap()["_vaayu_git_patch"]
        .as_str()
        .unwrap();
    crate::git_tools::apply_patch(&root, patch, true, true).unwrap();
    assert_eq!(git(&["show", ":sample.txt"]), old);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), new);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_hunk_range_stage_stages_only_the_visual_selections_lines() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    // Five consecutive changed lines within `unified=3`'s merge distance,
    // so they land in a single hunk -- staging only a Visual-selected
    // subset of them needs the range sub-patch machinery, not just a
    // whole-hunk stage.
    let new = old
        .replace("line 10\n", "changed 10\n")
        .replace("line 11\n", "changed 11\n")
        .replace("line 12\n", "changed 12\n")
        .replace("line 13\n", "changed 13\n")
        .replace("line 14\n", "changed 14\n");
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    assert_eq!(
        crate::git_tools::hunks(&root, &file, false)
            .unwrap()
            .entries
            .len(),
        1,
        "the five nearby changes should merge into a single hunk"
    );
    e.set_cursor(11, 0); // "changed 11"
    keys(&mut e, "Vj"); // select lines 11 and 12 only
    keys(&mut e, ",gs");
    assert!(
        e.message.to_lowercase().contains("staged"),
        "got: {}",
        e.message
    );
    assert!(matches!(e.mode, crate::mode::Mode::Normal));

    let staged = git(&["show", ":sample.txt"]);
    assert!(staged.contains("changed 11\n"));
    assert!(staged.contains("changed 12\n"));
    assert!(
        !staged.contains("changed 10\n")
            && !staged.contains("changed 13\n")
            && !staged.contains("changed 14\n"),
        "only the selected lines should be staged, got:\n{staged}"
    );
    // The working tree is untouched by staging -- all five lines are
    // still showing as changed there.
    let working = std::fs::read_to_string(&file).unwrap();
    assert_eq!(working, new);

    std::fs::remove_dir_all(root).ok();
}
#[test]
fn git_hunk_range_reset_discards_only_the_visual_selections_lines() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    let new = old
        .replace("line 10\n", "changed 10\n")
        .replace("line 11\n", "changed 11\n")
        .replace("line 12\n", "changed 12\n")
        .replace("line 13\n", "changed 13\n")
        .replace("line 14\n", "changed 14\n");
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.set_cursor(11, 0);
    keys(&mut e, "Vj"); // select lines 11 and 12 only
    keys(&mut e, ",gx");
    let r = e
        .results
        .as_ref()
        .expect("resetting a range should show a confirmation prompt");
    let value = r.entries[0]
        .action
        .as_ref()
        .and_then(|a| a.get("_vaayu_git_hunk_reset_range"))
        .cloned()
        .expect("prompt entries should carry the range-reset action");
    e.apply_hunk_reset_range(&value);
    assert!(
        e.message.to_lowercase().contains("reset") && !e.message.to_lowercase().contains("fail"),
        "got: {}",
        e.message
    );

    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert!(
        on_disk.contains("line 11\n"),
        "selected line 11 should be reset"
    );
    assert!(
        on_disk.contains("line 12\n"),
        "selected line 12 should be reset"
    );
    assert!(
        on_disk.contains("changed 10\n")
            && on_disk.contains("changed 13\n")
            && on_disk.contains("changed 14\n"),
        "the unselected lines in the same hunk should be left alone, got:\n{on_disk}"
    );
    assert_eq!(
        e.buf().rope.to_string(),
        on_disk,
        "the open buffer should reload to match the file on disk"
    );

    std::fs::remove_dir_all(root).ok();
}
#[test]
fn bracket_c_navigates_to_the_start_of_each_changed_hunk() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..10).map(|i| format!("line {i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    // Two separate single-line hunks: line index 2 and line index 7.
    let new = old
        .replace("line 2\n", "CHANGED\n")
        .replace("line 7\n", "CHANGED\n");
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.open_file(file).unwrap();
    let start = std::time::Instant::now();
    while e.git.as_ref().is_none_or(|g| g.signs.is_empty()) {
        e.ensure_git();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "git diff background job timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        e.git.as_ref().unwrap().signs.len(),
        2,
        "exactly two lines should be marked changed"
    );

    e.set_cursor(0, 0);
    e.next_hunk(true); // -> first hunk (line 2)
    assert_eq!(e.cursor().0, 2);
    e.next_hunk(true); // -> second hunk (line 7)
    assert_eq!(e.cursor().0, 7);
    e.next_hunk(true); // wraps back to the first hunk
    assert_eq!(e.cursor().0, 2);
    e.next_hunk(false); // wraps to the last hunk
    assert_eq!(e.cursor().0, 7);
    e.next_hunk(false); // -> first hunk
    assert_eq!(e.cursor().0, 2);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn bracket_c_treats_a_contiguous_multiline_change_as_one_hunk() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..10).map(|i| format!("line {i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    // Lines 2, 3, 4 (0-indexed) are one contiguous run.
    let new: String = old
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if (2..=4).contains(&i) {
                format!("CHANGED{i}\n")
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.open_file(file).unwrap();
    let start = std::time::Instant::now();
    while e.git.as_ref().is_none_or(|g| g.signs.is_empty()) {
        e.ensure_git();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "git diff background job timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        e.git.as_ref().unwrap().signs.len(),
        3,
        "three lines should be marked changed"
    );
    e.set_cursor(0, 0);
    e.next_hunk(true); // -> the hunk's first line, not each changed line
    assert_eq!(e.cursor().0, 2);
    e.next_hunk(true); // only one hunk -- wraps back to itself
    assert_eq!(e.cursor().0, 2);
    std::fs::remove_dir_all(root).ok();
}

/// Shared fixture for the Git workspace tests below: a repo with one
/// committed file, then a staged-modified file, an unstaged-modified
/// file and an untracked file, each with visibly distinct content so
/// assertions can't accidentally pass by matching the wrong entry.
fn git_workspace_fixture() -> (PathBuf, impl Fn(&[&str]) -> String) {
    let root = temp();
    let git = {
        let root = root.clone();
        move |args: &[&str]| -> String {
            let out = std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    };
    git(&["init", "-q"]);
    // Pin the initial branch to `master` regardless of the machine's
    // `init.defaultBranch` (modern Git defaults to `main`), so the explicit
    // `master` checkouts/pushes in the tests below match the repo's branch.
    git(&["symbolic-ref", "HEAD", "refs/heads/master"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    std::fs::write(root.join("staged.txt"), "original staged\n").unwrap();
    std::fs::write(root.join("unstaged.txt"), "original unstaged\n").unwrap();
    git(&["add", "staged.txt", "unstaged.txt"]);
    git(&["commit", "-qm", "fixture"]);
    std::fs::write(root.join("staged.txt"), "changed staged\n").unwrap();
    git(&["add", "staged.txt"]);
    std::fs::write(root.join("unstaged.txt"), "changed unstaged\n").unwrap();
    std::fs::write(root.join("untracked.txt"), "new file\n").unwrap();
    (root, git)
}

#[test]
fn git_status_lists_staged_unstaged_and_untracked_sections() {
    let (root, _git) = git_workspace_fixture();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_git_status();
    let r = e.results.as_ref().unwrap();
    assert!(r.git_status);
    let text: Vec<&str> = r.entries.iter().map(|e| e.text.as_str()).collect();
    assert!(text.iter().any(|t| t.contains("Staged (1)")));
    assert!(text.iter().any(|t| t.contains("Unstaged (1)")));
    assert!(text.iter().any(|t| t.contains("Untracked (1)")));
    // Exact-row checks, not a raw substring search: "staged.txt" is
    // itself a substring of "unstaged.txt".
    assert!(text.iter().any(|t| t.trim() == "M staged.txt"));
    assert!(text.iter().any(|t| t.trim() == "M unstaged.txt"));
    assert!(text.iter().any(|t| t.trim() == "? untracked.txt"));
    assert!(
        !text.iter().any(|t| t.contains("Conflicts")),
        "no merge in progress -- there should be no conflicts section at all"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_status_stage_and_unstage_move_a_file_between_sections() {
    let (root, git) = git_workspace_fixture();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_git_status();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("unstaged.txt"))
        .expect("unstaged.txt should be listed");
    e.results.as_mut().unwrap().cursor = idx;
    e.git_status_stage();
    assert!(e.message.to_lowercase().contains("staged"));
    let status = git(&["status", "--porcelain"]);
    assert!(
        status.contains("M  unstaged.txt"),
        "unstaged.txt should now be fully staged, got:\n{status}"
    );

    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("unstaged.txt"))
        .expect("still listed, now under Staged");
    e.results.as_mut().unwrap().cursor = idx;
    e.git_status_unstage();
    let status = git(&["status", "--porcelain"]);
    assert!(
        status.contains(" M unstaged.txt"),
        "unstaged.txt should be back to unstaged only, got:\n{status}"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_status_discard_prompt_reverts_unstaged_changes_and_reloads_the_buffer() {
    let (root, _git) = git_workspace_fixture();
    let file = root.join("unstaged.txt");
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.open_git_status();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("unstaged.txt"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.git_status_discard_prompt();
    let r = e
        .results
        .as_ref()
        .expect("discard should show a confirmation prompt");
    let value = r.entries[0]
        .action
        .as_ref()
        .and_then(|a| a.get("_vaayu_git_status_discard"))
        .cloned()
        .expect("prompt entries should carry the discard action");
    e.apply_git_status_discard(&value);
    assert!(e.message.to_lowercase().contains("discarded"));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "original unstaged\n"
    );
    assert_eq!(e.buf().rope.to_string(), "original unstaged\n");
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_status_discard_prompt_refuses_on_a_dirty_buffer() {
    let (root, _git) = git_workspace_fixture();
    let file = root.join("unstaged.txt");
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    keys(&mut e, "x"); // dirty the buffer without saving
    e.open_git_status();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("unstaged.txt"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.git_status_discard_prompt();
    assert!(
        e.message.contains("Save"),
        "a dirty buffer should refuse the discard with a clear message, got: {}",
        e.message
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_commit_commits_staged_changes_and_refuses_an_empty_message() {
    let (root, git) = git_workspace_fixture();
    let mut e = editor("");
    e.project_root = root.clone();

    e.git_commit("", false);
    assert!(e.message.to_lowercase().contains("message required"));
    assert_eq!(
        git(&["log", "--oneline"]).lines().count(),
        1,
        "an empty message must not create a commit"
    );

    e.git_commit("a real commit message", false);
    let log = git(&["log", "-1", "--pretty=%s"]);
    assert_eq!(log.trim(), "a real commit message");
    let status = git(&["status", "--porcelain"]);
    // Checks the exact path field (not a raw substring search): "staged.txt"
    // is itself a substring of "unstaged.txt", which is still legitimately
    // present in this status.
    assert!(
        !status
            .lines()
            .any(|l| l.get(3..).map(str::trim) == Some("staged.txt")),
        "staged.txt should no longer show as changed after committing it, got:\n{status}"
    );
    assert!(
        e.results.as_ref().unwrap().git_status,
        "committing should refresh the workspace view"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_commit_amend_with_no_message_keeps_the_previous_one() {
    let (root, git) = git_workspace_fixture();
    let mut e = editor("");
    e.project_root = root.clone();
    e.git_commit("first message", false);
    let commit_count_before = git(&["log", "--oneline"]).lines().count();

    // Stage the untracked file too, then amend with no new message.
    git(&["add", "untracked.txt"]);
    e.git_commit("", true);
    assert_eq!(
        git(&["log", "-1", "--pretty=%s"]).trim(),
        "first message",
        "an empty amend message should keep the previous one"
    );
    assert_eq!(
        git(&["log", "--oneline"]).lines().count(),
        commit_count_before,
        "amending should not create a new commit"
    );
    assert!(git(&["show", "--stat", "HEAD"]).contains("untracked.txt"));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn semantic_index_maps_token_types() {
    use crate::render::semantic_index;
    assert_eq!(semantic_index("keyword"), Some(0));
    assert_eq!(semantic_index("struct"), Some(1));
    assert_eq!(semantic_index("function"), Some(2));
    assert_eq!(semantic_index("string"), Some(3));
    assert_eq!(semantic_index("comment"), Some(4));
    assert_eq!(semantic_index("number"), Some(5));
    assert_eq!(semantic_index("variable"), None);
    assert_eq!(semantic_index("bogus"), None);
}
#[test]
fn set_semantictokens_toggles_and_clears() {
    let mut e = editor("x\n");
    assert!(!e.config.semantic_tokens);
    keys(&mut e, ":set semantictokens\n");
    assert!(e.config.semantic_tokens);
    e.semantic_tokens = vec![(0, 0, 2, 0, false)];
    keys(&mut e, ":set nosemantic\n");
    assert!(!e.config.semantic_tokens);
    assert!(e.semantic_tokens.is_empty(), "disabling clears tokens");
}
fn rename_edit_for(
    file: &std::path::Path,
    seq: u64,
    new_text: &str,
) -> (serde_json::Value, crate::language::RequestContext) {
    let uri = crate::files::uri(file);
    let edit = serde_json::json!({
        "changes": {
            uri: [{
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                "newText": new_text,
            }]
        }
    });
    let mut versions = std::collections::HashMap::new();
    versions.insert(file.to_path_buf(), seq);
    let ctx = crate::language::RequestContext {
        kind: "rename".into(),
        path: file.to_path_buf(),
        revision: 0,
        client: String::new(),
        versions,
    };
    (edit, ctx)
}
#[test]
fn rename_preview_defers_the_edit_until_applied() {
    let root = temp();
    let file = root.join("m.rs");
    std::fs::write(&file, "abc def\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    let (edit, ctx) = rename_edit_for(&file, e.buf().edit_seq, "XYZ");
    e.preview_rename(edit, ctx);
    // A preview is shown and stashed, but the buffer is untouched.
    assert!(e.pending_rename.is_some(), "edit should be pending");
    assert!(matches!(e.mode, crate::mode::Mode::Results));
    assert!(e
        .results
        .as_ref()
        .is_some_and(|r| r.title.contains("Rename preview")));
    assert_eq!(e.buf().rope.to_string(), "abc def\n");
    // Applying commits it and clears the pending state.
    e.apply_pending_rename();
    assert!(e.pending_rename.is_none());
    assert_eq!(e.buf().rope.to_string(), "XYZ def\n");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn rename_preview_cancel_leaves_the_buffer_untouched() {
    let root = temp();
    let file = root.join("m.rs");
    std::fs::write(&file, "abc def\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    let (edit, ctx) = rename_edit_for(&file, e.buf().edit_seq, "XYZ");
    e.preview_rename(edit, ctx);
    e.cancel_pending_rename();
    assert!(e.pending_rename.is_none());
    assert_eq!(e.buf().rope.to_string(), "abc def\n");
    // Applying now is a no-op with a clear message.
    e.apply_pending_rename();
    assert!(e.message.contains("No pending rename"));
    assert_eq!(e.buf().rope.to_string(), "abc def\n");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn folding_hides_inner_lines_and_motions_skip_them() {
    let mut e = editor("l0\nl1\nl2\nl3\nl4\nl5\n");
    // Fold 1-based lines 3..5 (0-based 2..4).
    keys(&mut e, ":3,5fold\n");
    assert_eq!(e.buf().folds.len(), 1);
    assert!(e.buf().folds[0].closed);
    assert_eq!(e.cursor().0, 2, "cursor snaps to the fold start");
    assert!(e.buf().line_hidden(3) && e.buf().line_hidden(4));
    assert!(!e.buf().line_hidden(2) && !e.buf().line_hidden(5));
    // j from the fold start jumps past the whole fold to line 5.
    keys(&mut e, "j");
    assert_eq!(e.cursor().0, 5);
    // k lands back on the fold start, never inside it.
    keys(&mut e, "k");
    assert_eq!(e.cursor().0, 2);
    // za opens it: the inner lines become visible and motions step normally.
    keys(&mut e, "za");
    assert!(!e.buf().folds[0].closed);
    assert!(!e.buf().line_hidden(3));
    keys(&mut e, "j");
    assert_eq!(e.cursor().0, 3);
}
#[test]
fn folds_track_line_inserts_and_deletes() {
    let mut e = editor("a\nb\nc\nd\ne\nf\n");
    keys(&mut e, ":4,6fold\n"); // 0-based fold 3..5
    assert!(
        e.buf().folds.iter().any(|f| f.start == 3 && f.end == 5),
        "initial fold"
    );
    // Insert a line at the very top: the fold shifts down by one.
    let b = e.buf_mut();
    b.begin_edit();
    b.insert_str_at(0, "NEW\n");
    b.commit_edit();
    assert!(
        e.buf().folds.iter().any(|f| f.start == 4 && f.end == 6),
        "fold shifts down after an insert above it"
    );
    // Delete that line again: the fold shifts back up.
    let b = e.buf_mut();
    b.begin_edit();
    b.delete_char_range(0, 4);
    b.commit_edit();
    assert!(
        e.buf().folds.iter().any(|f| f.start == 3 && f.end == 5),
        "fold shifts back up after the delete"
    );
}
#[test]
fn foldindent_creates_nested_folds_from_indentation() {
    let mut e = editor("def outer():\n    a = 1\n    def inner():\n        b = 2\n    c = 3\n");
    keys(&mut e, ":foldindent\n");
    // The top-level block (lines 0..4) and the nested def (lines 2..3) fold.
    assert!(e
        .buf()
        .folds
        .iter()
        .any(|f| f.start == 0 && f.end == 4 && f.closed));
    assert!(e.buf().folds.iter().any(|f| f.start == 2 && f.end == 3));
    // The outer fold collapses everything under line 0.
    assert!(!e.buf().line_hidden(0));
    assert!(e.buf().line_hidden(1) && e.buf().line_hidden(3));
    // Opening the outer fold reveals the next level, with the inner still folded.
    keys(&mut e, "zo");
    assert!(!e.buf().line_hidden(1), "outer content now visible");
    assert!(e.buf().line_hidden(3), "inner fold still hides its body");
}
#[test]
fn fold_open_close_all_and_delete() {
    let mut e = editor("a\nb\nc\nd\ne\n");
    keys(&mut e, ":1,3fold\n");
    keys(&mut e, ":4,5fold\n");
    assert_eq!(e.buf().folds.len(), 2);
    // zR opens every fold; zM closes every fold.
    keys(&mut e, "zR");
    assert!(e.buf().folds.iter().all(|f| !f.closed));
    keys(&mut e, "zM");
    assert!(e.buf().folds.iter().all(|f| f.closed));
    // zd deletes the fold under the cursor (cursor is on line 0, in fold 0..2).
    keys(&mut e, "gg");
    keys(&mut e, "zd");
    assert_eq!(e.buf().folds.len(), 1);
}
#[test]
fn inccommand_previews_substitution_live_then_clears() {
    let mut e = editor("foo one\nfoo two\nbar three\n");
    // Typing a substitute (no Enter yet) populates a live preview...
    keys(&mut e, ":%s/foo/X/g");
    assert_eq!(e.sub_preview.get(&0).map(String::as_str), Some("X one"));
    assert_eq!(e.sub_preview.get(&1).map(String::as_str), Some("X two"));
    assert!(
        !e.sub_preview.contains_key(&2),
        "an unmatched line is not previewed"
    );
    // ...but the buffer itself is untouched while previewing.
    assert_eq!(e.buf().rope.to_string(), "foo one\nfoo two\nbar three\n");
    // Esc cancels: preview clears, buffer still unchanged.
    keys(&mut e, "\x1b");
    assert!(e.sub_preview.is_empty(), "Esc clears the preview");
    assert_eq!(e.buf().rope.to_string(), "foo one\nfoo two\nbar three\n");
    // Submitting the same command applies it for real and clears the preview.
    keys(&mut e, ":%s/foo/X/g\n");
    assert_eq!(e.buf().rope.to_string(), "X one\nX two\nbar three\n");
    assert!(e.sub_preview.is_empty(), "submitting clears the preview");
}
#[test]
fn inccommand_preview_respects_an_explicit_range() {
    let mut e = editor("hit\nhit\nhit\n");
    // Only line 2 (1-based) is in range, so only index 1 previews.
    keys(&mut e, ":2s/hit/HIT/");
    assert_eq!(e.sub_preview.get(&1).map(String::as_str), Some("HIT"));
    assert!(!e.sub_preview.contains_key(&0));
    assert!(!e.sub_preview.contains_key(&2));
    keys(&mut e, "\x1b");
}
#[test]
fn cfar_replaces_across_every_file_in_the_results_list() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    let c = root.join("c.txt");
    std::fs::write(&a, "old value\nkeep old too\n").unwrap();
    std::fs::write(&b, "another old here\n").unwrap();
    std::fs::write(&c, "nothing to change\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.results = Some(crate::results::Results::new(
        "grep: old",
        vec![
            crate::results::Entry::location(a.clone(), 0, 0, "old value"),
            crate::results::Entry::location(b.clone(), 0, 8, "another old here"),
            crate::results::Entry::location(c.clone(), 0, 0, "nothing to change"),
        ],
    ));
    keys(&mut e, ":cfar/old/new/g\n");
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "new value\nkeep new too\n"
    );
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "another new here\n");
    // A file in the list with no match is left untouched.
    assert_eq!(std::fs::read_to_string(&c).unwrap(), "nothing to change\n");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn loclists_are_per_buffer() {
    let root = temp();
    std::fs::write(root.join("a.txt"), "alpha match\n").unwrap();
    std::fs::write(root.join("b.txt"), "beta\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(root.join("a.txt")).unwrap();
    e.lgrep("alpha");
    let a_id = e.buf().id;
    assert!(e.loclist().is_some_and(|r| !r.entries.is_empty()), "a has a loclist");
    // Switch to b: it has no loclist of its own yet.
    e.open_file(root.join("b.txt")).unwrap();
    assert_ne!(e.buf().id, a_id);
    assert!(e.loclist().is_none(), "b's loclist is independent (empty)");
    // Back to a: its loclist is still there.
    e.open_file(root.join("a.txt")).unwrap();
    assert!(e.loclist().is_some_and(|r| !r.entries.is_empty()), "a's loclist persists");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn lgrep_fills_the_location_list() {
    let root = temp();
    std::fs::write(root.join("a.txt"), "needle here\nother line\n").unwrap();
    std::fs::write(root.join("b.txt"), "no match\nneedle again\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.lgrep("needle");
    let ll = e.loclist().cloned().expect("loclist populated");
    assert_eq!(ll.entries.len(), 2, "two matches across two files");
    assert!(ll.entries.iter().all(|en| en.path.is_some()));
    assert!(ll.entries.iter().any(|en| en.text.contains("needle here")));
    assert!(ll.entries.iter().any(|en| en.text.contains("needle again")));
    // A no-match pattern leaves a clear message and doesn't replace the list.
    e.lgrep("zzz_no_such_token_zzz");
    assert!(e.message.contains("no matches"));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn cfar_without_a_results_list_reports_an_error() {
    let mut e = editor("old\n");
    keys(&mut e, ":cfar/old/new/g\n");
    assert!(e.message.contains("no results list"), "got: {}", e.message);
    // The current buffer is not touched.
    assert_eq!(e.buf().rope.to_string(), "old\n");
}
#[test]
fn diff_mode_marks_differing_lines() {
    let root = temp();
    let a = root.join("a.txt");
    std::fs::write(&a, "same\nold line\ntail\n").unwrap();
    let b = root.join("b.txt");
    std::fs::write(&b, "same\nnew line\ntail\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(a.clone()).unwrap();
    e.diff_this();
    e.open_file(b.clone()).unwrap();
    e.diff_this();
    e.update_diff();
    let id = |name: &str| {
        e.buffers
            .iter()
            .find(|x| x.path.as_ref().is_some_and(|p| p.ends_with(name)))
            .unwrap()
            .id
    };
    assert!(e.diff_lines[&id("a.txt")].contains(&1), "old line differs");
    assert!(!e.diff_lines[&id("a.txt")].contains(&0), "line 0 is equal");
    assert!(e.diff_lines[&id("b.txt")].contains(&1), "new line differs");
    e.diff_off();
    e.update_diff();
    assert!(e.diff_lines.is_empty(), "diffoff clears");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn colorscheme_switches_theme() {
    use crossterm::style::Color;
    let mut e = editor("fn x() {}\n");
    assert!(matches!(e.theme.keyword, Color::Cyan), "default keyword is cyan");
    keys(&mut e, ":colorscheme mono\n");
    assert_eq!(e.config.colorscheme, "mono");
    assert!(matches!(e.theme.keyword, Color::AnsiValue(_)));
    keys(&mut e, ":colorscheme nope\n");
    assert_eq!(e.config.colorscheme, "mono", "unknown scheme leaves it unchanged");
    keys(&mut e, ":colorscheme default\n");
    assert!(matches!(e.theme.keyword, Color::Cyan));
}
#[test]
fn colorscheme_themes_ui_colors() {
    use crossterm::style::Color;
    let mut e = editor("x\n");
    // Default scheme preserves the original UI colors.
    assert!(matches!(e.theme.cursorline_bg, Color::AnsiValue(236)));
    assert!(matches!(e.theme.statusline_active_bg, Color::DarkBlue));
    // A true-color scheme themes the cursorline + statusline too.
    keys(&mut e, ":colorscheme cool\n");
    assert!(matches!(e.theme.cursorline_bg, Color::Rgb { .. }));
    assert!(matches!(e.theme.statusline_active_bg, Color::Rgb { .. }));
    assert!(matches!(e.theme.statusline_inactive_bg, Color::Rgb { .. }));
}
#[test]
fn theme_builtin_names_resolve() {
    for n in crate::theme::NAMES {
        assert!(crate::theme::builtin(n).is_some(), "{n} should resolve");
    }
    assert!(crate::theme::builtin("bogus").is_none());
}
#[test]
fn statusline_format_expands_tokens() {
    use crate::render::{expand_statusline, StatusInfo};
    let info = StatusInfo {
        mode: "NORMAL",
        name: "a.rs",
        line: 3,
        col: 5,
        total: 10,
        modified: true,
        ftype: "rs",
    };
    assert_eq!(
        expand_statusline("%M %f:%l:%c %m [%y] %L%%", &info),
        "NORMAL a.rs:3:5 [+] [rs] 10%"
    );
    let single = StatusInfo {
        mode: "INSERT",
        name: "x",
        line: 1,
        col: 1,
        total: 1,
        modified: false,
        ftype: "",
    };
    assert_eq!(expand_statusline("%p %m", &single), "100% ");
    assert_eq!(expand_statusline("%z%%", &single), "%z%");
}
#[test]
fn set_stickyscroll_toggles() {
    let mut e = editor("x\n");
    assert!(!e.config.sticky_scroll);
    keys(&mut e, ":set stickyscroll\n");
    assert!(e.config.sticky_scroll);
    keys(&mut e, ":set nosticky\n");
    assert!(!e.config.sticky_scroll);
}
#[test]
fn zen_mode_toggles() {
    let mut e = editor("a\nb\n");
    assert!(!e.zen);
    keys(&mut e, ":zen\n");
    assert!(e.zen);
    assert!(e.message.contains("Zen mode on"));
    keys(&mut e, ":zen\n");
    assert!(!e.zen);
}
#[test]
fn notifications_toasts_recorded_only_when_enabled() {
    let mut e = editor("x\n");
    e.set_message("while off");
    assert!(e.toasts.is_empty(), "no toasts when disabled");
    e.config.notifications = true;
    e.set_message("hello toast");
    e.set_message("second toast");
    assert_eq!(e.toasts.len(), 2);
    assert!(e.toasts.iter().any(|(_, m)| m == "hello toast"));
    assert!(!e.has_expired_toast(), "fresh toasts are not expired");
    // Toggle command.
    keys(&mut e, ":set nonotifications\n");
    assert!(!e.config.notifications);
    keys(&mut e, ":set notifications\n");
    assert!(e.config.notifications);
}
#[test]
fn send_to_terminal_writes_current_line() {
    let mut e = editor("hello_repl_line\nsecond line\nthird\n");
    let dir = std::env::temp_dir();
    let s = crate::pty::PtySession::spawn(&["/bin/cat".into()], &dir, 24, 80).unwrap();
    e.terminals.push(s);
    e.set_cursor(0, 0);
    let line = e.buf().line_text(0);
    e.send_to_terminal(&line);
    let start = std::time::Instant::now();
    loop {
        let got = e
            .terminals
            .last()
            .unwrap()
            .with_screen(|scr| scr.contents().contains("hello_repl_line"));
        if got {
            break;
        }
        assert!(
            start.elapsed().as_secs() < 5,
            "the line was never received by the terminal"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // A range send (e.g. from Visual) forwards each line.
    e.termsend_lines(1, 2);
    let start = std::time::Instant::now();
    loop {
        let got = e.terminals.last().unwrap().with_screen(|scr| {
            let c = scr.contents();
            c.contains("second line") && c.contains("third")
        });
        if got {
            break;
        }
        assert!(start.elapsed().as_secs() < 5, "range send never received");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    e.terminals.pop().unwrap().shutdown();
}
#[test]
fn send_to_terminal_without_a_terminal_is_a_message() {
    let mut e = editor("x\n");
    e.send_to_terminal("x");
    assert!(e.message.contains("No terminal"));
}
#[test]
fn code_tour_starts_and_steps() {
    let root = temp();
    std::fs::create_dir_all(root.join(".tours")).unwrap();
    std::fs::write(root.join("a.rs"), "one\ntwo\nthree\nfour\n").unwrap();
    std::fs::write(root.join("b.rs"), "x\ny\nz\n").unwrap();
    std::fs::write(
        root.join(".tours/intro.tour"),
        r#"{"title":"Intro","steps":[
            {"file":"a.rs","line":3,"description":"step one"},
            {"file":"b.rs","line":2,"description":"step two"}]}"#,
    )
    .unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.start_tour("intro");
    assert!(e.buf().path.as_ref().unwrap().ends_with("a.rs"));
    assert_eq!(e.cursor().0, 2, "jumped to a.rs line 3");
    assert!(e.message.contains("step one"));
    e.tour_step(true);
    assert!(e.buf().path.as_ref().unwrap().ends_with("b.rs"));
    assert_eq!(e.cursor().0, 1, "jumped to b.rs line 2");
    assert!(e.message.contains("step two"));
    e.tour_step(true);
    assert!(e.message.contains("End of tour"), "clamps at the last step");
    e.tour_step(false);
    assert!(e.buf().path.as_ref().unwrap().ends_with("a.rs"));
    assert_eq!(e.cursor().0, 2, "prev returns to step one");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn make_parses_errorformat_into_quickfix() {
    let root = temp();
    std::fs::write(root.join("src.rs"), "line1\nline2\nline3\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.run_task("printf 'src.rs:2:5: error: bad thing\\n  --> src.rs:3:1\\nnot a match\\n'");
    let start = std::time::Instant::now();
    while e.quickfix.is_none() {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let qf = e.quickfix.as_ref().unwrap();
    assert_eq!(qf.entries.len(), 2, "two locations parsed");
    assert!(qf.entries[0].text.contains("src.rs:2:5"));
    assert_eq!(qf.entries[0].line, 1, "line is 0-indexed");
    assert_eq!(qf.entries[0].col, 4, "col is 0-indexed");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn loclist_from_diagnostics_and_step() {
    let root = temp();
    let file = root.join("d.txt");
    std::fs::write(&file, "aaa\nbbb\nccc\nddd\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    let path = e.buf().path.clone().unwrap();
    let mk = |line: usize, msg: &str| crate::lsp::Diagnostic {
        line,
        col: 0,
        end_line: line,
        end_col: 1,
        severity: crate::lsp::Severity::Warning,
        message: msg.into(),
        raw: serde_json::Value::Null,
    };
    e.diagnostics
        .insert(path.clone(), vec![mk(0, "first"), mk(2, "second")]);
    e.open_loclist();
    assert!(e.loclist().is_none(), "empty until populated");
    e.loclist_from_diagnostics();
    let ll = e.loclist().cloned().expect("loclist populated");
    assert_eq!(ll.entries.len(), 2);
    assert!(ll.title.contains("Location list"));
    e.loclist_step(true);
    assert_eq!(e.cursor().0, 2, ":lnext jumps to the second diagnostic");
    e.loclist_step(true);
    assert_eq!(e.cursor().0, 0, ":lnext wraps to the first");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn git_revert_creates_an_undo_commit() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "second"]);
    let before = git(&["log", "--oneline"]).lines().count();
    let mut e = editor("");
    e.project_root = root.clone();
    e.git_revert("HEAD");
    let after = git(&["log", "--oneline"]).lines().count();
    assert_eq!(after, before + 1, "revert adds a commit");
    assert_eq!(
        std::fs::read_to_string(root.join("staged.txt")).unwrap(),
        "original staged\n",
        "revert restored the file"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn git_cherry_pick_applies_a_commit() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    git(&["checkout", "-qb", "feature"]);
    std::fs::write(root.join("newfile.txt"), "cherry content\n").unwrap();
    git(&["add", "newfile.txt"]);
    git(&["commit", "-qm", "addnew"]);
    let hash = git(&["rev-parse", "HEAD"]).trim().to_string();
    git(&["checkout", "-q", "master"]);
    assert!(!root.join("newfile.txt").exists());
    let mut e = editor("");
    e.project_root = root.clone();
    e.git_cherry_pick(&hash);
    assert!(
        root.join("newfile.txt").exists(),
        "cherry-pick brought the file over"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn git_file_history_lists_commits_touching_the_file() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "second commit"]);
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(root.join("staged.txt")).unwrap();
    e.git_file_history();
    let start = std::time::Instant::now();
    while !e
        .results
        .as_ref()
        .is_some_and(|r| r.title.starts_with("Git history"))
    {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let entries = &e.results.as_ref().unwrap().entries;
    assert_eq!(entries.len(), 2, "both commits touched staged.txt");
    assert!(entries[0].text.contains("second commit"));
    e.open_result();
    let start = std::time::Instant::now();
    while !e
        .results
        .as_ref()
        .is_some_and(|r| r.title.starts_with("Git show"))
    {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(e
        .results
        .as_ref()
        .unwrap()
        .export(true, &root)
        .contains("staged.txt"));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn git_log_lists_commits_and_enter_shows_a_commits_diff() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "second commit"]);
    let mut e = editor("");
    e.project_root = root.clone();
    e.git_log();
    let start = std::time::Instant::now();
    while e.results.as_ref().map(|r| r.title.as_str())
        != Some("Git log — Enter shows a commit's diff")
    {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let entries_len = e.results.as_ref().unwrap().entries.len();
    assert_eq!(entries_len, 2, "fixture + second commit");
    assert!(e.results.as_ref().unwrap().entries[0]
        .text
        .contains("second commit"));

    e.open_result();
    let start = std::time::Instant::now();
    while !e
        .results
        .as_ref()
        .is_some_and(|r| r.title.starts_with("Git show"))
    {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let text = e.results.as_ref().unwrap().export(true, &root);
    assert!(
        text.contains("staged.txt")
            || text.contains("untracked.txt")
            || text.contains("unstaged.txt")
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_branches_lists_and_checkout_switches_branches() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "second commit"]);
    git(&["checkout", "-qb", "feature"]);
    std::fs::write(root.join("staged.txt"), "on feature branch\n").unwrap();
    git(&["commit", "-qam", "feature commit"]);
    git(&["checkout", "-q", "master"]);

    let mut e = editor("");
    e.project_root = root.clone();
    e.git_branches();
    let r = e.results.as_ref().expect("branch list should show");
    assert_eq!(r.entries.len(), 2);
    assert!(r.entries.iter().any(|en| en.text.contains("feature")));

    e.checkout_branch("feature");
    assert_eq!(
        git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "feature"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("staged.txt")).unwrap(),
        "on feature branch\n"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_branches_checkout_refuses_on_a_dirty_buffer() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "second commit"]);
    git(&["checkout", "-qb", "feature"]);
    git(&["checkout", "-q", "master"]);

    let mut e = editor("abc\n");
    e.project_root = root.clone();
    keys(&mut e, "x"); // dirty a buffer, unrelated to the branch itself
    e.checkout_branch("feature");
    assert!(e.message.contains("Save all buffers"));
    assert_eq!(git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "master");
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_stash_push_stashes_tracked_changes_and_refreshes_status() {
    let (root, git) = git_workspace_fixture();
    let mut e = editor("");
    e.project_root = root.clone();
    e.git_stash_push();
    assert!(
        e.message.to_lowercase().contains("saved"),
        "got: {}",
        e.message
    );
    let status = git(&["status", "--porcelain"]);
    // Only the untracked file remains -- tracked changes were stashed.
    assert!(!status.contains("staged.txt") && !status.contains("unstaged.txt"));
    assert!(status.contains("untracked.txt"));
    assert_eq!(git(&["stash", "list"]).lines().count(), 1);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_push_sends_new_commits_to_a_local_bare_remote() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "second commit"]);

    // A bare local repo as the remote -- no network needed. Push once
    // with plain `git` first to establish the upstream tracking branch
    // (a bare `git push` with no configured upstream at all fails, same
    // as it would for a real user's very first push of a new repo).
    let remote_dir = temp();
    std::fs::remove_dir_all(&remote_dir).unwrap();
    std::process::Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&remote_dir)
        .output()
        .unwrap();
    git(&["remote", "add", "origin", &remote_dir.display().to_string()]);
    git(&["push", "-u", "-q", "origin", "master"]);

    std::fs::write(root.join("staged.txt"), "third change\n").unwrap();
    git(&["commit", "-qam", "third commit"]);

    let mut e = editor("");
    e.project_root = root.clone();
    e.git_push();
    let start = std::time::Instant::now();
    while e.git_task.is_some() {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let remote_log = std::process::Command::new("git")
        .arg("-C")
        .arg(&remote_dir)
        // `--all`: the bare remote's own HEAD may point at its default branch
        // (`main` on modern Git) which has no commits, so inspect every ref to
        // see the pushed `master` regardless of the remote's default branch.
        .args(["log", "--oneline", "--all"])
        .output()
        .unwrap();
    let remote_log = String::from_utf8_lossy(&remote_log.stdout);
    assert!(
        remote_log.contains("third commit"),
        "the bare remote should now have the pushed commit, got:\n{remote_log}"
    );
    std::fs::remove_dir_all(root).ok();
    std::fs::remove_dir_all(remote_dir).ok();
}

#[test]
fn git_fetch_and_pull_run_successfully_against_a_configured_remote() {
    let (root, git) = git_workspace_fixture();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "second commit"]);

    let remote_dir = temp();
    std::fs::remove_dir_all(&remote_dir).unwrap();
    std::process::Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&remote_dir)
        .output()
        .unwrap();
    git(&["remote", "add", "origin", &remote_dir.display().to_string()]);
    git(&["push", "-u", "-q", "origin", "master"]);

    let mut e = editor("");
    e.project_root = root.clone();
    e.git_fetch();
    let start = std::time::Instant::now();
    while e.git_task.is_some() {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(
        e.results.as_ref().map(|r| r.title.as_str()),
        Some("Git error"),
        "fetch against a real, reachable remote should not fail, got: {:?}",
        e.results.as_ref().map(|r| &r.entries)
    );

    e.git_pull();
    let start = std::time::Instant::now();
    while e.git_task.is_some() {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(
        e.results.as_ref().map(|r| r.title.as_str()),
        Some("Git error"),
        "pull with nothing new upstream should still succeed, got: {:?}",
        e.results.as_ref().map(|r| &r.entries)
    );
    std::fs::remove_dir_all(root).ok();
    std::fs::remove_dir_all(remote_dir).ok();
}

#[test]
fn diff_ignore_whitespace_toggle_hides_whitespace_only_hunks_from_gitdiff() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let file = root.join("f.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
    git(&["add", "f.txt"]);
    git(&["commit", "-qm", "fixture"]);
    // "beta" only gains trailing whitespace; "gamma" gets a real content
    // change -- ignoring whitespace should hide the first but keep the
    // second.
    std::fs::write(&file, "alpha\nbeta \nGAMMA\n").unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    assert!(!e.diff_ignore_whitespace);

    e.git_results("diff");
    let start = std::time::Instant::now();
    while e.results.as_ref().map(|r| r.title.as_str()) != Some("Git diff") {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let with_ws: Vec<String> = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|en| en.text.clone())
        .collect();
    assert!(with_ws.contains(&"-beta".to_string()));
    assert!(with_ws.contains(&"+beta ".to_string()));
    assert!(with_ws.contains(&"-gamma".to_string()));
    assert!(with_ws.contains(&"+GAMMA".to_string()));

    e.toggle_diff_ignore_whitespace();
    assert!(e.diff_ignore_whitespace);
    let start = std::time::Instant::now();
    while e.git_task.is_some() {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let without_ws: Vec<String> = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|en| en.text.clone())
        .collect();
    assert!(
        !without_ws.contains(&"-beta".to_string()) && !without_ws.contains(&"+beta ".to_string()),
        "the whitespace-only change to beta should no longer show as a diff line, got: {without_ws:?}"
    );
    assert!(
        without_ws.contains(&"-gamma".to_string()) && without_ws.contains(&"+GAMMA".to_string()),
        "the real content change to gamma should still show, got: {without_ws:?}"
    );

    // Toggling back off (and re-running) restores the whitespace hunk.
    e.toggle_diff_ignore_whitespace();
    assert!(!e.diff_ignore_whitespace);
    let start = std::time::Instant::now();
    while e.git_task.is_some() {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let restored: Vec<String> = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|en| en.text.clone())
        .collect();
    assert!(restored.contains(&"-beta".to_string()));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn diff_ignore_whitespace_toggle_does_not_affect_hunk_stage_or_reset() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let file = root.join("f.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
    git(&["add", "f.txt"]);
    git(&["commit", "-qm", "fixture"]);
    std::fs::write(&file, "alpha\nbeta \ngamma\n").unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.diff_ignore_whitespace = true;
    // `hunks()` (used by stage/unstage/reset) must never see
    // `--ignore-all-space` -- the whitespace-only hunk should still be
    // reported so it can be staged like any other change.
    let r = crate::git_tools::hunks(&root, &e.buf().path.clone().unwrap(), false).unwrap();
    assert_eq!(
        r.entries.len(),
        1,
        "the whitespace-only hunk should still be there"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn gitdiff_entries_jump_to_the_line_they_actually_show() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..9).map(|i| format!("line{i}\n")).collect::<String>();
    let file = root.join("f.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "f.txt"]);
    git(&["commit", "-qm", "fixture"]);
    let new = old.replace("line3\n", "CHANGED3\n");
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.git_results("diff");
    let start = std::time::Instant::now();
    while e.results.as_ref().map(|r| r.title.as_str()) != Some("Git diff") {
        e.poll_jobs();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let r = e.results.as_ref().unwrap();
    let added = r
        .entries
        .iter()
        .find(|en| en.text == "+CHANGED3")
        .expect("the added line should be in the diff");
    assert_eq!(
        added.line, 3,
        "the added line should point at its own (0-indexed) line, not always line 0"
    );
    let removed = r
        .entries
        .iter()
        .find(|en| en.text == "-line3")
        .expect("the removed line should be in the diff");
    assert_eq!(
        removed.line, 3,
        "a removed line should anchor at the position it once occupied"
    );
    // A preamble line (before the first hunk) still has no meaningful
    // position of its own -- confirms this isn't retroactively broken.
    let preamble = r
        .entries
        .iter()
        .find(|en| en.text.starts_with("diff --git"))
        .unwrap();
    assert_eq!(preamble.line, 0);
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn gitstash_lists_stashes_and_enter_shows_a_diff() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let file = root.join("sample.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    std::fs::write(&file, "one\nSTASHED_CHANGE\n").unwrap();
    git(&["stash", "push", "-m", "my stash"]);

    let mut e = editor("");
    e.project_root = root.clone();
    e.show_git_stash();
    let r = e
        .results
        .as_ref()
        .expect("gitstash should open a results list");
    assert_eq!(r.entries.len(), 1);
    assert!(r.entries[0].text.contains("my stash"));
    e.open_result();
    let r = e
        .results
        .as_ref()
        .expect("selecting a stash should show its diff");
    assert!(
        r.entries.iter().any(|e| e.text.contains("STASHED_CHANGE")),
        "the stash's actual diff content should appear"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn gitstash_with_none_shows_a_message_not_an_empty_list() {
    let root = temp();
    std::process::Command::new("git")
        .current_dir(&root)
        .args(["init", "-q"])
        .output()
        .unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.show_git_stash();
    assert!(e.results.is_none());
    std::fs::remove_dir_all(root).ok();
}

fn git_repo_with_github_remote() -> (PathBuf, PathBuf, String) {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    git(&["remote", "add", "origin", "git@github.com:acme/widgets.git"]);
    let file = root.join("src/lib.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .current_dir(&root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    (root, file, sha)
}

#[test]
fn permalink_for_cursor_line_copies_a_head_pinned_github_url() {
    let (root, file, sha) = git_repo_with_github_remote();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.set_cursor(1, 0); // "two", the second line
    keys(&mut e, ",gp");
    assert_eq!(
        e.registers.get(Some('+')).unwrap().text,
        format!("https://github.com/acme/widgets/blob/{sha}/src/lib.rs#L2")
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn permalink_ex_command_uses_the_cursor_line_only() {
    let (root, file, sha) = git_repo_with_github_remote();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.set_cursor(2, 0); // "three", the third line
    keys(&mut e, ":permalink\n");
    assert_eq!(
        e.registers.get(Some('+')).unwrap().text,
        format!("https://github.com/acme/widgets/blob/{sha}/src/lib.rs#L3")
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn permalink_for_visual_selection_covers_the_whole_line_range() {
    let (root, file, sha) = git_repo_with_github_remote();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.set_cursor(0, 0);
    keys(&mut e, "Vj"); // Visual-line select through the second line
    keys(&mut e, ",gp");
    assert_eq!(
        e.registers.get(Some('+')).unwrap().text,
        format!("https://github.com/acme/widgets/blob/{sha}/src/lib.rs#L1-L2")
    );
    assert!(
        matches!(e.mode, crate::mode::Mode::Normal),
        "generating the permalink should leave Visual mode"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn permalink_from_a_gitblame_entry_uses_that_lines_own_commit() {
    let (root, file, sha) = git_repo_with_github_remote();
    let mut e = editor("");
    e.project_root = root.clone();
    e.results = Some(crate::results::Results::new(
        "Git blame",
        vec![crate::results::Entry::location(
            file.clone(),
            0,
            0,
            format!(
                "{} (Vaayu test 2024-01-01 00:00:00 +0000  1) one",
                &sha[..7]
            ),
        )],
    ));
    e.permalink_from_results_entry();
    assert_eq!(
        e.registers.get(Some('+')).unwrap().text,
        format!(
            "https://github.com/acme/widgets/blob/{}/src/lib.rs#L1",
            &sha[..7]
        )
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn permalink_without_a_github_remote_shows_a_message_not_a_url() {
    let root = temp();
    std::process::Command::new("git")
        .current_dir(&root)
        .args(["init", "-q"])
        .output()
        .unwrap();
    let file = root.join("f.txt");
    std::fs::write(&file, "one\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.generate_permalink(&file, 0, 0, None);
    assert!(
        e.message.contains("remote"),
        "no origin remote should produce a clear message, got: {}",
        e.message
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn results_preview_toggle_wrap_and_scroll_show_real_file_content() {
    let root = temp();
    let file = root.join("f.txt");
    let content = "line0\nline1\nline2\nline3\nline4\nline5\n";
    std::fs::write(&file, content).unwrap();

    let mut e = editor(content);
    e.buf_mut().path = Some(file.clone());
    e.results = Some(crate::results::Results::new(
        "Grep",
        vec![crate::results::Entry::location(file.clone(), 3, 0, "hit")],
    ));
    e.mode = Mode::Results;

    assert!(!e.results.as_ref().unwrap().preview, "preview starts off");
    e.feed_key(Key::Char('p'));
    assert!(
        e.results.as_ref().unwrap().preview,
        "'p' should toggle preview on"
    );

    // context_before=1 around entry.line=3 -> starts at line2, matches line3.
    let source = e.preview_source_lines(&file);
    let rows = e
        .results
        .as_ref()
        .unwrap()
        .preview_rows(&source, 4, 80, 1)
        .expect("preview is on and the entry has a path");
    assert_eq!(rows[0].text, "line2");
    assert!(
        rows.iter().any(|r| r.is_match && r.text == "line3"),
        "the entry's own line should be in the preview and marked as the match"
    );

    e.feed_key(Key::Ctrl('e'));
    assert_eq!(
        e.results.as_ref().unwrap().preview_scroll,
        1,
        "Ctrl-e should scroll the preview down"
    );
    e.feed_key(Key::Ctrl('y'));
    assert_eq!(
        e.results.as_ref().unwrap().preview_scroll,
        0,
        "Ctrl-y should scroll the preview back up"
    );

    e.feed_key(Key::Char('w'));
    assert!(
        e.results.as_ref().unwrap().preview_wrap,
        "'w' should toggle preview wrap on"
    );

    e.feed_key(Key::Ctrl('e'));
    assert_eq!(e.results.as_ref().unwrap().preview_scroll, 1);
    e.feed_key(Key::Char('j'));
    assert_eq!(
        e.results.as_ref().unwrap().preview_scroll,
        0,
        "moving the list cursor should reset preview scroll"
    );

    std::fs::remove_dir_all(root).ok();
}

#[test]
fn git_tools_status_reports_modified_untracked_and_clean_files() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let clean = root.join("clean.txt");
    let modified = root.join("modified.txt");
    std::fs::write(&clean, "a\n").unwrap();
    std::fs::write(&modified, "a\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
    std::fs::write(&modified, "b\n").unwrap();
    let untracked = root.join("untracked.txt");
    std::fs::write(&untracked, "x\n").unwrap();

    let status = crate::git_tools::status(&root).unwrap();
    assert_eq!(status.get(&modified), Some(&'M'));
    assert_eq!(status.get(&untracked), Some(&'?'));
    assert_eq!(
        status.get(&clean),
        None,
        "an unmodified tracked file has no status entry"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_tree_refreshes_git_status_on_open_and_r() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let file = root.join("a.txt");
    std::fs::write(&file, "a\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);

    let mut e = editor("");
    e.project_root = root.clone();
    e.toggle_file_tree();
    assert!(
        e.file_tree.as_ref().unwrap().git_status.is_empty(),
        "nothing is modified yet"
    );
    std::fs::write(&file, "changed\n").unwrap();
    crate::filetree::handle_key(&mut e, Key::Char('R'));
    assert_eq!(
        e.file_tree.as_ref().unwrap().git_status.get(&file),
        Some(&'M')
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_tools_ignored_collapses_an_entirely_ignored_directory() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("a.txt"), "x\n").unwrap();
    std::fs::write(target.join("b.txt"), "x\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    git(&["add", ".gitignore"]);
    git(&["commit", "-qm", "fixture"]);

    let ignored = crate::git_tools::ignored(&root).unwrap();
    assert!(
        ignored.contains(&target),
        "target/ itself should be the (only) ignored entry"
    );
    assert!(
        !ignored.contains(&target.join("a.txt")),
        "an entirely-ignored directory should collapse to one entry, \
         not list each file inside it -- this is what keeps the file \
         tree from having to read_dir into it at all"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_tree_hides_gitignored_paths_by_default_and_bang_reveals_them() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target/a.txt"), "x\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(root.join("kept.txt"), "x\n").unwrap();
    git(&["add", ".gitignore"]);
    git(&["commit", "-qm", "fixture"]);

    let mut e = editor("");
    e.project_root = root.clone();
    e.toggle_file_tree();
    let names = |e: &Editor| -> Vec<String> {
        e.file_tree
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .map(|n| n.name.clone())
            .collect()
    };
    assert!(names(&e).contains(&"kept.txt".to_string()));
    assert!(
        !names(&e).contains(&"target".to_string()),
        "target/ is gitignored and must be hidden by default"
    );
    crate::filetree::handle_key(&mut e, Key::Char('!'));
    assert!(
        names(&e).contains(&"target".to_string()),
        "! should reveal gitignored paths"
    );
    crate::filetree::handle_key(&mut e, Key::Char('!'));
    assert!(
        !names(&e).contains(&"target".to_string()),
        "! again should hide them again"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn block_case_toggle_and_repeat() {
    let mut e = editor("abcd\nabcd\nabcd\nabcd\n");
    e.set_cursor(0, 1);
    e.feed_key(Key::Ctrl('v'));
    keys(&mut e, "jl~");
    assert_eq!(e.buf().rope.to_string(), "aBCd\naBCd\nabcd\nabcd\n");
    e.set_cursor(2, 1);
    keys(&mut e, ".");
    assert_eq!(e.buf().rope.to_string(), "aBCd\naBCd\naBCd\naBCd\n");
}

#[test]
fn private_lock_excludes_concurrent_writer() {
    let root = temp();
    let dir = root.join(".vaayu");
    let guard = crate::files::private_lock(&dir, "comments.lock").unwrap();
    assert!(crate::files::private_lock(&dir, "comments.lock").is_err());
    drop(guard);
    assert!(crate::files::private_lock(&dir, "comments.lock").is_ok());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn session_roundtrips_multiple_tabs() {
    let root = temp();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "alpha\n").unwrap();
    std::fs::write(&b, "beta\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(a.clone()).unwrap();
    e.new_tab();
    e.open_file(b.clone()).unwrap();
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active_tab, 1);
    e.save_session().unwrap();
    // Wipe tab state, then restore.
    e.tabs = vec![crate::windows::Tab::default()];
    e.active_tab = 0;
    e.windows.clear();
    e.window_layout = None;
    e.load_session().unwrap();
    assert_eq!(e.tabs.len(), 2, "both tabs restored");
    assert_eq!(e.active_tab, 1, "active tab restored");
    // The active (second) tab shows b.txt.
    assert!(e.buf().path.as_ref().is_some_and(|p| p.ends_with("b.txt")));
    // The first tab shows a.txt.
    e.switch_tab(0);
    assert!(e.buf().path.as_ref().is_some_and(|p| p.ends_with("a.txt")));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn recursive_layout_and_session_roundtrip() {
    let root = temp();
    let file = root.join("a.md");
    std::fs::write(&file, "a\nb\nc\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.split_window(true, false);
    e.split_window(false, false);
    e.set_cursor(2, 0);
    let rects = e.pane_rects(100, 40);
    assert_eq!(rects.len(), 3);
    assert_eq!(rects[0].height, 39);
    assert!(rects[1].y < rects[2].y);
    assert_eq!(rects[1].x, rects[2].x);
    e.save_session().unwrap();
    e.windows.clear();
    e.window_layout = None;
    e.set_cursor(0, 0);
    e.load_session().unwrap();
    assert_eq!(e.windows.len(), 3);
    assert_eq!(e.cursor(), (2, 0));
    e.close_window();
    assert_eq!(e.pane_rects(100, 40).len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn grapheme_motion_delete_and_backspace() {
    let mut e = editor("a\u{301}👩‍💻z\n");
    keys(&mut e, "l");
    assert_eq!(e.cursor().1, 2);
    keys(&mut e, "x");
    assert_eq!(e.buf().line_text(0), "a\u{301}z");
    keys(&mut e, "hxi");
    assert_eq!(e.buf().line_text(0), "z");
    let mut e = editor("a\u{301}z\n");
    keys(&mut e, "li");
    e.feed_key(Key::Backspace);
    assert_eq!(e.buf().line_text(0), "z");

    let mut e = editor("comp ghost\n");
    keys(&mut e, "$a");
    for _ in 0..5 {
        e.feed_key(Key::Backspace);
    }
    assert_eq!(e.buf().line_text(0), "comp ");
    assert_eq!(e.cursor(), (0, 5));
}
#[test]
fn snippet_expansion_and_placeholder_editing() {
    let expansion = crate::snippet::expand("fn ${1:name}(${2|x,y|}) {$0}", &Default::default());
    let (text, stops, mirrors, choices) = (
        expansion.text,
        expansion.stops,
        expansion.mirrors,
        expansion.choices,
    );
    assert_eq!(text, "fn name(x) {}");
    assert_eq!(stops[0], (3, 7));
    let mut e = editor(&text);
    e.enter_insert();
    e.set_cursor_insert(0, 3);
    e.snippet = Some(crate::snippet::Session {
        mirrors,
        stops,
        choices,
        current: 0,
        selected: true,
    });
    keys(&mut e, "hello");
    assert_eq!(e.buf().line_text(0), "fn hello(x) {}");
    e.feed_key(Key::Tab);
    assert_eq!(e.cursor().1, 9);
    keys(&mut e, "arg");
    assert_eq!(e.buf().line_text(0), "fn hello(arg) {}");
    e.feed_key(Key::BackTab);
    assert_eq!(e.cursor().1, 3);
}
#[test]
fn workspace_resources_ordered_and_failed_plan_is_unchanged() {
    let root = temp();
    let a = root.join("a.rs");
    let b = root.join("b.rs");
    std::fs::write(&a, "old\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(a.clone()).unwrap();
    let uri = crate::files::uri;
    let edit = serde_json::json!({"documentChanges":[{"kind":"rename","oldUri":uri(&a),"newUri":uri(&b)},{"textDocument":{"uri":uri(&b),"version":null},"edits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"new"}]}]});
    e.apply_workspace_edit(&edit, None).unwrap();
    assert!(!a.exists());
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "old\n");
    assert_eq!(
        e.buffers
            .iter()
            .find(|v| v.path.as_ref() == Some(&b))
            .unwrap()
            .rope
            .to_string(),
        "new\n"
    );
    let fail = serde_json::json!({"documentChanges":[{"kind":"create","uri":uri(&a)},{"kind":"delete","uri":uri(&b)}]});
    assert!(e.apply_workspace_edit(&fail, None).is_err());
    assert!(!a.exists());
    assert!(b.exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn restored_comment_draft_does_not_overwrite_saved_note() {
    let root = temp();
    let mut e = editor("");
    e.project_root = root.clone();
    e.restore_recovery(serde_json::json!({"path":root,"text":"unsaved thought","line":0,"col":0,"note":{"id":1,"file":"a.rs","start":0,"end":0,"whole_file":false,"anchor":"x","text":"","stale":false}}));
    assert_eq!(e.buf().rope.to_string(), "unsaved thought");
    assert!(e.notes.dirty);
    assert_eq!(e.notes.items.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn review_packet_selection_and_resolved_status() {
    let root = temp();
    let mut e = editor("");
    e.project_root = root.clone();
    let mut r = crate::results::Results::new(
        "review",
        vec![
            crate::results::Entry::text("first"),
            crate::results::Entry::text("second"),
        ],
    );
    r.selected.insert(1);
    e.results = Some(r);
    let path = e.export_review().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    assert_eq!(v["entries"][0]["feedback"], "second");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn extended_regex_modes_backreferences_and_lookaround() {
    let compile =
        |s| crate::search::compile(&crate::vimregex::translate_pattern(s), false, false).unwrap();
    assert!(compile(r"\v(\w+) \1").is_match("word word").unwrap());
    assert!(compile(r"\V(a+b)").is_match("(a+b)").unwrap());
    assert!(compile(r"foo\(bar\)\@=").is_match("foobar").unwrap());
    assert!(compile(r"\cHELLO").is_match("hello").unwrap());
    assert!(compile(r"[()+]").is_match("+").unwrap());
    let mut e = editor("one one\ntwo two\n");
    crate::command::run_ex(&mut e, r"%s/\(\w\+\) \1/\1/g");
    assert_eq!(e.buf().rope.to_string(), "one\ntwo\n");
}
#[test]
fn lsp_timeout_cancel_and_initialization_deadline() {
    use crate::lsp::client::{LspClient, LspEvent};
    let root = temp();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
    let log = root.join("log");
    let mut cfg = crate::config::LspServer {
        cmd: vec![
            "python3".into(),
            fixture.display().to_string(),
            log.display().to_string(),
        ],
        request_timeout_ms: 300,
        ..Default::default()
    };
    let mut c = LspClient::spawn("rust", &crate::files::uri(&root), &cfg).unwrap();
    let deadline = std::time::Instant::now();
    while c.capabilities.is_null() {
        c.poll();
        assert!(deadline.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    c.request("vaayu/hang", serde_json::json!({}), 42).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(c.poll().iter().any(|e| matches!(
        e,
        LspEvent::Response {
            request_id: 42,
            error: Some(_),
            ..
        }
    )));
    c.request("vaayu/hang", serde_json::json!({}), 43).unwrap();
    c.cancel(43);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(!c
        .poll()
        .iter()
        .any(|e| matches!(e, LspEvent::Response { request_id: 43, .. })));
    drop(c);
    cfg.cmd.push("--hang-init".into());
    let mut c = LspClient::spawn("rust", &crate::files::uri(&root), &cfg).unwrap();
    // The initialization deadline is deliberately more generous than a single
    // request (request_timeout_ms * 4 = 1200ms here), so wait past it.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(c
        .poll()
        .iter()
        .any(|e| matches!(e,LspEvent::Error(s) if s.contains("initialization timed out"))));
    drop(c);
    assert!(std::fs::read_to_string(log)
        .unwrap()
        .contains("$/cancelRequest"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "requires clangd; run explicitly for real-server interoperability"]
fn real_clangd_formatting_and_diagnostics() {
    let root = temp();
    let file = root.join("main.c");
    std::fs::write(&file, "int main(){return 0;}\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.config.lsp.insert(
        "clangd".into(),
        crate::config::LspServer {
            cmd: vec!["clangd".into(), "--background-index=false".into()],
            filetypes: vec!["c".into()],
            ..Default::default()
        },
    );
    e.open_file(file).unwrap();
    e.sync_lsp();
    let start = std::time::Instant::now();
    while !e.lsp_clients.values().any(|c| !c.capabilities.is_null()) {
        e.poll_lsp_events();
        assert!(start.elapsed().as_secs() < 15, "{}", e.message);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    e.request_language("format", None);
    while !e.buf().line_text(0).contains("main() {") {
        e.poll_lsp_events();
        assert!(start.elapsed().as_secs() < 15, "{}", e.message);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(e.buf().is_modified());
    drop(e);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn review_agent_receives_packet_and_returns_results() {
    let root = temp();
    let mut e = editor("");
    e.project_root = root.clone();
    e.config.review_command = vec![
        "python3".into(),
        "-c".into(),
        "import json,sys; p=json.load(sys.stdin); print(p['entries'][0]['feedback'])".into(),
    ];
    e.results = Some(crate::results::Results::new(
        "test",
        vec![crate::results::Entry::text("review this")],
    ));
    e.run_review();
    let start = std::time::Instant::now();
    while e.review_job.is_some() {
        e.poll_review();
        assert!(start.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(e
        .review_results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .any(|r| r.text == "review this"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn linked_snippet_fields_follow_edited_placeholder() {
    let x = crate::snippet::expand("${1:name} = $1; $0", &Default::default());
    let mut e = editor(&x.text);
    e.enter_insert();
    e.set_cursor_insert(0, 0);
    e.snippet = Some(crate::snippet::Session {
        stops: x.stops,
        mirrors: x.mirrors,
        choices: x.choices,
        current: 0,
        selected: true,
    });
    keys(&mut e, "value");
    e.feed_key(Key::Tab);
    assert_eq!(e.buf().line_text(0), "value = value; ");
    assert_eq!(e.cursor().1, 15);
}
#[test]
fn snippet_expand_supports_a_placeholder_nested_inside_another_ones_default() {
    let x = crate::snippet::expand("${1:foo ${2:bar} baz}", &Default::default());
    assert_eq!(x.text, "foo bar baz");
    assert_eq!(
        x.stops.len(),
        3,
        "stop 1, nested stop 2, and the implicit end-of-snippet stop"
    );
}
#[test]
fn snippet_expand_ignores_an_unsupported_transform_instead_of_failing() {
    // Transforms aren't implemented -- the tail is dropped, but the
    // numbered stop it was attached to must still exist and be editable,
    // not silently vanish or reject the whole snippet.
    let x = crate::snippet::expand("${1/(.*)/prefix_$1/} done", &Default::default());
    assert_eq!(x.text, " done");
    assert_eq!(x.stops.len(), 2);
    assert_eq!(x.stops[0], (0, 0));
}
#[test]
fn snippet_variable_transform_applies_regex() {
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("TM_FILENAME".to_string(), "foo.rs".to_string());
    // Strip the extension.
    assert_eq!(
        crate::snippet::expand("${TM_FILENAME/(.*)\\..+$/$1/}", &vars).text,
        "foo"
    );
    vars.insert("W".to_string(), "FooBar".to_string());
    // Global flag replaces every match.
    assert_eq!(crate::snippet::expand("${W/o/0/g}", &vars).text, "F00Bar");
    assert_eq!(crate::snippet::expand("${W/[a-z]/x/g}", &vars).text, "FxxBxx");
    // Case-insensitive flag.
    assert_eq!(crate::snippet::expand("${W/FOO/baz/i}", &vars).text, "bazBar");
}
#[test]
fn snippet_variable_transform_unset_variable_is_empty() {
    // Applied to the empty string; `.*` matches empty → "X".
    assert_eq!(
        crate::snippet::expand("${MISSING/.*/X/}", &Default::default()).text,
        "X"
    );
}
#[test]
fn snippet_expand_treats_an_unclosed_brace_as_literal_text() {
    let x = crate::snippet::expand("foo ${1:bar and no closing brace", &Default::default());
    assert_eq!(x.text, "foo ${1:bar and no closing brace");
}
#[test]
fn snippet_expand_bounds_pathological_nesting_instead_of_recursing_forever() {
    let deep = "${1:".repeat(50) + "x" + &"}".repeat(50);
    // The real assertion is that this returns at all (no stack overflow,
    // no hang) rather than any specific text.
    let x = crate::snippet::expand(&deep, &Default::default());
    assert!(!x.text.is_empty());
}
#[test]
fn snippet_choice_cycles_through_options_with_ctrl_n_and_wraps() {
    let x = crate::snippet::expand("${1|red,green,blue|}", &Default::default());
    assert_eq!(x.text, "red");
    let mut e = editor(&x.text);
    e.enter_insert();
    e.set_cursor_insert(0, 0);
    e.snippet = Some(crate::snippet::Session {
        stops: x.stops,
        mirrors: x.mirrors,
        choices: x.choices,
        current: 0,
        selected: true,
    });
    e.feed_key(Key::Ctrl('n'));
    assert_eq!(e.buf().line_text(0), "green");
    e.feed_key(Key::Ctrl('n'));
    assert_eq!(e.buf().line_text(0), "blue");
    e.feed_key(Key::Ctrl('n')); // wraps back to the first choice
    assert_eq!(e.buf().line_text(0), "red");
    e.feed_key(Key::Ctrl('p')); // backwards wraps too
    assert_eq!(e.buf().line_text(0), "blue");
    // Still `selected`: typing now replaces whichever choice is showing,
    // the same as it always did for a plain default.
    keys(&mut e, "x");
    assert_eq!(e.buf().line_text(0), "x");
}
#[test]
fn snippet_variables_include_filename_base_directory_and_current_line() {
    let root = temp();
    let file = root.join("widget.rs");
    std::fs::write(&file, "old content\n").unwrap();
    let mut e = editor("");
    e.open_file(file.clone()).unwrap();
    e.enter_insert();
    e.set_cursor_insert(0, 0);
    e.completion = Some(crate::completion::CompletionState {
        start: (0, 0),
        items: vec![crate::completion::Item {
            label: "cls".into(),
            insert_text: "class $TM_FILENAME_BASE in $TM_DIRECTORY: $TM_CURRENT_LINE".into(),
            detail: None,
            source: crate::completion::Source::Buffer,
            edit: None,
            raw: None,
            snippet: true,
            additional: vec![],
            kind: None,
        }],
        selected: 0,
        request_id: 1,
    });
    e.feed_key(Key::Tab);
    let text = e.buf().line_text(0);
    assert!(
        text.starts_with("class widget in"),
        "TM_FILENAME_BASE should be the filename without extension, got: {text}"
    );
    assert!(
        text.contains(&root.display().to_string()),
        "TM_DIRECTORY should be the file's parent directory, got: {text}"
    );
    assert!(
        text.ends_with("old content"),
        "TM_CURRENT_LINE should be the line's own pre-insertion content, got: {text}"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn block_cells_across_tabs_and_wide_prefixes() {
    let mut e = editor("\tabc\n界  abc\n");
    e.set_cursor(0, 1);
    e.feed_key(Key::Ctrl('v'));
    keys(&mut e, "jld");
    assert_eq!(e.buf().line_text(0), "    c");
    assert_eq!(e.buf().line_text(1), "界  c");
    keys(&mut e, "u");
    assert_eq!(e.buf().line_text(0), "\tabc");
    // Explicit cell rectangle starts after both tab and wide-character prefix.
    crate::visual::apply_block_cells(&mut e, crate::operator::OperatorKind::Yank, 0, 1, 4, 6);
    e.set_cursor(0, 0);
    keys(&mut e, "P");
    assert!(e.buf().line_text(0).starts_with("ab"));
    assert!(e.buf().line_text(1).starts_with("ab"));
}
#[test]
fn file_resource_create_edit_save_delete_and_permissions() {
    let root = temp();
    let a = root.join("a.rs");
    let b = root.join("b.rs");
    let mut e = editor("");
    e.project_root = root.clone();
    let uri = crate::files::uri;
    e.apply_workspace_edit(&serde_json::json!({"documentChanges":[{"kind":"create","uri":uri(&a)},{"textDocument":{"uri":uri(&a),"version":null},"edits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"hello"}]}]}),None).unwrap();
    let i = e
        .buffers
        .iter()
        .position(|b| b.path.as_ref() == Some(&a))
        .unwrap();
    e.buffers[i].save().unwrap();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "hello");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o750)).unwrap();
    }
    e.apply_workspace_edit(&serde_json::json!({"documentChanges":[{"kind":"rename","oldUri":uri(&a),"newUri":uri(&b)}]}),None).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&b).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
    e.apply_workspace_edit(
        &serde_json::json!({"documentChanges":[{"kind":"delete","uri":uri(&b)}]}),
        None,
    )
    .unwrap();
    assert!(!b.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn resource_commit_failure_rolls_back_prior_files() {
    let root = temp();
    let a = root.join("a.rs");
    let b = root.join("b.rs");
    let missing = root.join("zmissing/file.rs");
    std::fs::write(&a, "original\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(a.clone()).unwrap();
    let id = e.buf().id;
    let uri = crate::files::uri;
    let edit = serde_json::json!({"documentChanges":[{"kind":"rename","oldUri":uri(&a),"newUri":uri(&b)},{"kind":"create","uri":uri(&missing)}]});
    assert!(e.apply_workspace_edit(&edit, None).is_err());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "original\n");
    assert!(!b.exists());
    assert_eq!(e.buf().id, id);
    assert_eq!(e.buf().path.as_ref(), Some(&a));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn edited_comment_checkpoint_contains_private_draft() {
    let root = temp();
    let file = root.join("source.rs");
    std::fs::write(&file, "source\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.notes = crate::notes::Notes::load(&root);
    e.open_file(file).unwrap();
    e.new_note(false);
    keys(&mut e, "iunpersisted thought\x1b");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    e.checkpoint_recovery();
    let path = root.join(format!(".vaayu/recovery-{}.json", std::process::id()));
    let start = std::time::Instant::now();
    while !path.exists() {
        assert!(start.elapsed().as_secs() < 3);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(v[0]["text"], "unpersisted thought");
    assert_eq!(v[0]["note"]["file"], "source.rs");
    e.recovery.cleanup();
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn diff_overlay_toggle_shows_deleted_lines_only_when_on() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let old = (0..10).map(|i| format!("line{i}\n")).collect::<String>();
    let file = root.join("sample.txt");
    std::fs::write(&file, &old).unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    // Delete lines 3 and 4 entirely (no replacement) -- a pure removal,
    // so the deleted content only exists as a diff-overlay annotation,
    // never in the buffer itself.
    let new = old.replace("line3\nline4\n", "");
    std::fs::write(&file, &new).unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    let start = std::time::Instant::now();
    while e.git.as_ref().is_none_or(|g| g.deleted_before.is_empty()) {
        e.ensure_git();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "git diff background job timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(!e.diff_overlay);
    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 60, 10);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 10, &mut cache).unwrap();
    assert!(
        !String::from_utf8_lossy(&out).contains("line3"),
        "deleted content should not render while the overlay is off"
    );

    e.toggle_diff_overlay();
    assert!(e.diff_overlay);
    let mut cache = crate::render::FrameCache::new();
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 10, &mut cache).unwrap();
    let text = String::from_utf8_lossy(&out);
    // The annotation is a compact one-line preview (the first removed
    // line's own text plus a count), not every removed line in full.
    assert!(
        text.contains("line3") && text.contains("2 lines"),
        "deleted lines should render as an annotation once the overlay is on. Got: {text:?}"
    );

    e.toggle_diff_overlay();
    assert!(!e.diff_overlay);
    let mut cache = crate::render::FrameCache::new();
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 10, &mut cache).unwrap();
    assert!(
        !String::from_utf8_lossy(&out).contains("line3"),
        "toggling back off should stop rendering deleted content"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn diff_overlay_highlights_only_the_changed_word_on_a_modified_line() {
    let root = temp();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Vaayu test"]);
    git(&["config", "user.email", "vaayu-test@example.invalid"]);
    let file = root.join("sample.txt");
    std::fs::write(&file, "let x = old_value;\nunrelated line\n").unwrap();
    git(&["add", "sample.txt"]);
    git(&["commit", "-qm", "fixture"]);
    std::fs::write(&file, "let x = new_value;\nunrelated line\n").unwrap();

    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    let start = std::time::Instant::now();
    while e.git.as_ref().is_none_or(|g| g.word_diff.is_empty()) {
        e.ensure_git();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "git diff background job timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        e.git.as_ref().unwrap().word_diff.get(&0),
        Some(&vec![(8, 18)])
    );

    e.toggle_diff_overlay();
    let mut cache = crate::render::FrameCache::new();
    crate::render::prepare_view(&mut e, 60, 10);
    let mut out = Vec::new();
    crate::render::draw(&mut out, &e, 60, 10, &mut cache).unwrap();
    let text = String::from_utf8_lossy(&out);
    // "48;5;5" is DarkMagenta's 256-color background SGR code (the
    // word-diff highlight this overlay paints); it should wrap exactly
    // "new_value;" and nothing from the unchanged "let x = " prefix.
    assert!(
        text.contains("\x1b[48;5;5m\x1b[38;5;15mnew_value;"),
        "the changed word (only) should get a distinct background highlight. Got: {text:?}"
    );
    assert!(
        !text.contains("\x1b[48;5;5m\x1b[38;5;15mlet"),
        "the unchanged prefix should not be highlighted. Got: {text:?}"
    );
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn diff_overlay_toggle_refuses_nothing_and_just_flips_the_flag() {
    let mut e = editor("abc\n");
    assert!(!e.diff_overlay);
    e.toggle_diff_overlay();
    assert!(e.diff_overlay);
    assert!(e.message.to_lowercase().contains("on"));
    e.toggle_diff_overlay();
    assert!(!e.diff_overlay);
    assert!(e.message.to_lowercase().contains("off"));
}
#[test]
fn context_picker_offers_selection_only_when_visual_is_active() {
    let mut e = editor("a\nb\nc\n");
    e.open_context_picker();
    let r = e.results.as_ref().unwrap();
    assert!(!r
        .entries
        .iter()
        .any(|en| en.text.contains("Visual selection")));
    assert!(r.entries.iter().any(|en| en.text.contains("Current file")));
    assert_eq!(e.mode, Mode::Results);

    keys(&mut e, "q");
    keys(&mut e, "Vj"); // select lines 0 and 1
    e.open_context_picker();
    let r = e.results.as_ref().unwrap();
    assert!(r
        .entries
        .iter()
        .any(|en| en.text.contains("Visual selection")));
    assert_eq!(
        e.mode,
        Mode::Results,
        "picking a context kind leaves Visual mode"
    );
}
#[test]
fn context_send_file_copies_the_whole_buffer_with_a_path_header() {
    let root = temp();
    let file = root.join("f.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.open_context_picker();
    e.open_result(); // "Current file" is the first entry
    assert!(e.message.contains("Copied"));
    let copied = e.registers.get(Some('+')).unwrap().text.clone();
    assert!(copied.contains("File: f.rs"));
    assert!(copied.contains("fn main() {}"));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn context_send_selection_copies_only_the_selected_lines() {
    let mut e = editor("one\ntwo\nthree\nfour\n");
    e.set_cursor(1, 0);
    keys(&mut e, "Vj"); // select "two" and "three"
    e.open_context_picker();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("Visual selection"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    let copied = e.registers.get(Some('+')).unwrap().text.clone();
    assert!(copied.contains("two") && copied.contains("three"));
    assert!(!copied.contains("one") && !copied.contains("four"));
}
#[test]
fn context_send_clipboard_errors_when_empty_and_copies_when_present() {
    let mut e = editor("x\n");
    e.open_context_picker();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("Clipboard"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    assert!(
        e.message.to_lowercase().contains("empty"),
        "got: {}",
        e.message
    );

    e.registers
        .set(Some('+'), "clipboard payload".into(), false);
    e.open_context_picker();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("Clipboard"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    let copied = e.registers.get(Some('+')).unwrap().text.clone();
    assert!(copied.contains("clipboard payload"));
}
#[test]
fn context_send_diagnostics_lists_the_current_files_diagnostics() {
    let root = temp();
    let file = root.join("f.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.diagnostics.insert(
        file,
        vec![crate::lsp::Diagnostic {
            line: 0,
            col: 0,
            end_line: 0,
            end_col: 2,
            severity: crate::lsp::Severity::Error,
            message: "something's wrong".into(),
            raw: serde_json::Value::Null,
        }],
    );
    e.open_context_picker();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("diagnostics"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    let copied = e.registers.get(Some('+')).unwrap().text.clone();
    assert!(copied.contains("something's wrong"));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn context_send_symbol_body_and_signature_use_the_open_outline() {
    let root = temp();
    let file = root.join("f.rs");
    std::fs::write(&file, "fn one() {}\nfn two() {\n    body_line\n}\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file.clone()).unwrap();
    e.outline = Some(crate::outline::Outline::default());
    e.outline.as_mut().unwrap().buffer_path = Some(file);
    e.outline.as_mut().unwrap().set_nodes(vec![
        crate::outline::SymbolNode {
            name: "one".into(),
            kind: "fn",
            line: 0,
            col: 3,
            depth: 0,
            end_line: 0,
        },
        crate::outline::SymbolNode {
            name: "two".into(),
            kind: "fn",
            line: 1,
            col: 3,
            depth: 0,
            end_line: 3,
        },
    ]);
    e.set_cursor(2, 0); // inside "two"'s body

    e.open_context_picker();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("signature"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    let sig = e.registers.get(Some('+')).unwrap().text.clone();
    assert!(sig.contains("fn two()"));
    assert!(!sig.contains("body_line"));

    e.open_context_picker();
    let idx = e
        .results
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|en| en.text.contains("body"))
        .unwrap();
    e.results.as_mut().unwrap().cursor = idx;
    e.open_result();
    let body = e.registers.get(Some('+')).unwrap().text.clone();
    assert!(body.contains("fn two()") && body.contains("body_line") && body.contains('}'));
    assert!(!body.contains("fn one()"));
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn context_send_reaches_an_attached_agent_sessions_input() {
    let mut e = editor("hello world\n");
    e.screen_rows = 24;
    e.screen_cols = 80;
    e.config
        .agent_commands
        .insert("testagent".into(), vec!["/bin/cat".into()]);
    e.toggle_agent_session("testagent");
    let id = e.active_terminal_id().unwrap();
    e.split_window(true, false); // a second pane to edit from
    e.set_cursor(0, 0);

    e.open_context_picker();
    e.open_result(); // "Current file" is the first entry
    assert!(
        e.message.to_lowercase().contains("sent to testagent"),
        "got: {}",
        e.message
    );
    let start = std::time::Instant::now();
    loop {
        let seen = e
            .terminals
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .with_screen(|s| s.contents().contains("hello world"));
        if seen {
            break;
        }
        assert!(
            start.elapsed().as_secs() < 5,
            "context was never written to the agent's input"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    e.close_window();
    e.close_window();
}
#[test]
fn gv_reselects_charwise_selection() {
    use crate::mode::{Mode, VisualKind};
    let mut e = editor("hello world\nsecond line\n");
    e.set_cursor(0, 0);
    keys(&mut e, "vll\x1b"); // select "hel", then leave visual
    assert!(matches!(e.mode, Mode::Normal), "Esc should return to Normal");
    keys(&mut e, "j"); // move the cursor away
    keys(&mut e, "gv"); // reselect
    assert!(
        matches!(e.mode, Mode::Visual(VisualKind::Char)),
        "gv should restore charwise Visual, got {:?}",
        e.mode
    );
    assert_eq!(e.visual_anchor, Some((0, 0)), "anchor restored");
    assert_eq!(e.cursor(), (0, 2), "cursor restored to end of prior span");
}
#[test]
fn gv_reselects_linewise_selection() {
    use crate::mode::{Mode, VisualKind};
    let mut e = editor("aaa\nbbb\nccc\nddd\n");
    e.set_cursor(0, 0);
    keys(&mut e, "Vj\x1b"); // V-line over lines 0..=1
    keys(&mut e, "G"); // jump to the last line
    keys(&mut e, "gv");
    assert!(
        matches!(e.mode, Mode::Visual(VisualKind::Line)),
        "gv should restore linewise Visual, got {:?}",
        e.mode
    );
    assert_eq!(e.visual_anchor, Some((0, 0)));
    assert_eq!(e.cursor().0, 1, "cursor line restored");
}
#[test]
fn gv_reselects_after_operator() {
    use crate::mode::{Mode, VisualKind};
    let mut e = editor("hello\n");
    e.set_cursor(0, 0);
    keys(&mut e, "vlly"); // yank "hel" — the operator consumes the selection
    assert!(matches!(e.mode, Mode::Normal));
    keys(&mut e, "$"); // move to end of line
    keys(&mut e, "gv");
    assert!(matches!(e.mode, Mode::Visual(VisualKind::Char)));
    assert_eq!(e.visual_anchor, Some((0, 0)));
    assert_eq!(e.cursor(), (0, 2), "gv restores the span the operator used");
}
#[test]
fn gv_without_prior_selection_is_noop() {
    use crate::mode::Mode;
    let mut e = editor("abc\n");
    keys(&mut e, "gv");
    assert!(
        matches!(e.mode, Mode::Normal),
        "gv with no prior selection stays in Normal"
    );
    assert_eq!(e.visual_anchor, None);
}
#[test]
fn gv_clamps_to_shrunken_buffer() {
    use crate::mode::{Mode, VisualKind};
    let mut e = editor("only one line\n");
    // Simulate a stale selection that points past the current buffer.
    e.last_visual = Some((VisualKind::Char, (9, 40), (12, 99)));
    keys(&mut e, "gv");
    assert!(matches!(e.mode, Mode::Visual(VisualKind::Char)));
    let (al, _ac) = e.visual_anchor.unwrap();
    assert_eq!(al, 0, "anchor line clamped into range");
    assert_eq!(e.cursor().0, 0, "cursor line clamped into range");
    // Column stays within the single line's length.
    assert!(e.cursor().1 <= "only one line".len());
}
#[test]
fn encoding_latin1_roundtrips() {
    use crate::buffer::Encoding;
    let root = temp();
    let p = root.join("latin1.txt");
    std::fs::write(&p, b"caf\xe9\n").unwrap(); // 0xE9 = é in latin1 (invalid UTF-8)
    let mut b = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b.encoding, Encoding::Latin1);
    assert_eq!(b.rope.to_string(), "café\n");
    b.begin_edit();
    b.insert_str(0, 0, "X");
    b.commit_edit();
    b.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"Xcaf\xe9\n", "latin1 re-encoded on save");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn encoding_utf16le_roundtrips() {
    use crate::buffer::Encoding;
    let root = temp();
    let p = root.join("u16.txt");
    let mut bytes = vec![0xFFu8, 0xFE];
    for u in "Hi\n".encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&p, &bytes).unwrap();
    let mut b = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b.encoding, Encoding::Utf16Le);
    assert_eq!(b.rope.to_string(), "Hi\n");
    b.save_force().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), bytes, "utf-16le round-trips with its BOM");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn encoding_utf8_is_default() {
    use crate::buffer::Encoding;
    let root = temp();
    let p = root.join("u8.txt");
    std::fs::write(&p, "hello\n").unwrap();
    let b = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b.encoding, Encoding::Utf8);
    assert_eq!(b.encoded_bytes(), b"hello\n");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn fileformat_detects_and_preserves_dos() {
    use crate::buffer::FileFormat;
    let root = temp();
    let p = root.join("dos.txt");
    std::fs::write(&p, b"one\r\ntwo\r\n").unwrap();
    let mut b = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b.fileformat, FileFormat::Dos);
    assert_eq!(b.rope.to_string(), "one\ntwo\n", "rope holds \\n-only text");
    b.begin_edit();
    b.insert_str(0, 0, "X");
    b.commit_edit();
    b.save().unwrap();
    assert_eq!(
        std::fs::read(&p).unwrap(),
        b"Xone\r\ntwo\r\n",
        "save restores CRLF endings"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn fileformat_detects_and_preserves_mac() {
    use crate::buffer::FileFormat;
    let root = temp();
    let p = root.join("mac.txt");
    std::fs::write(&p, b"a\rb\r").unwrap();
    let mut b = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b.fileformat, FileFormat::Mac);
    assert_eq!(b.rope.to_string(), "a\nb\n");
    b.save_force().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"a\rb\r", "save restores CR endings");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn fileformat_unix_is_default_and_plain() {
    use crate::buffer::FileFormat;
    let root = temp();
    let p = root.join("unix.txt");
    std::fs::write(&p, b"a\nb\n").unwrap();
    let b = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b.fileformat, FileFormat::Unix);
    assert!(!b.bom);
    assert_eq!(b.encoded(), "a\nb\n");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn fileformat_bom_is_stripped_and_restored() {
    use crate::buffer::FileFormat;
    let root = temp();
    let p = root.join("bom.txt");
    std::fs::write(&p, b"\xef\xbb\xbfhi\r\n").unwrap(); // UTF-8 BOM + CRLF
    let mut b = Buffer::from_path(p.clone()).unwrap();
    assert!(b.bom, "BOM detected");
    assert_eq!(b.fileformat, FileFormat::Dos);
    assert_eq!(b.rope.to_string(), "hi\n", "BOM + CR stripped from the rope");
    b.save_force().unwrap();
    assert_eq!(
        std::fs::read(&p).unwrap(),
        b"\xef\xbb\xbfhi\r\n",
        "BOM + CRLF restored on save"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn fileformat_set_ff_converts_on_save() {
    use crate::buffer::FileFormat;
    let root = temp();
    let p = root.join("conv.txt");
    std::fs::write(&p, b"x\ny\n").unwrap();
    let mut b = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b.fileformat, FileFormat::Unix);
    b.fileformat = FileFormat::Dos;
    // The external-change guard must still pass: it compares the normalized
    // disk read against the normalized baseline, not raw bytes.
    b.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"x\r\ny\r\n");
    let b2 = Buffer::from_path(p.clone()).unwrap();
    assert_eq!(b2.fileformat, FileFormat::Dos, "re-read detects dos");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn set_cursorline_toggles_config() {
    let mut e = editor("abc\n");
    assert!(!e.config.cursorline, "off by default");
    keys(&mut e, ":set cursorline\n");
    assert!(e.config.cursorline);
    keys(&mut e, ":set nocursorline\n");
    assert!(!e.config.cursorline);
    keys(&mut e, ":set cul\n");
    assert!(e.config.cursorline, "short form works");
}
#[test]
fn shada_persists_and_restores_cursor() {
    let root = temp();
    let file = root.join("a.txt");
    std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\n").unwrap();
    let mut e1 = editor("");
    e1.project_root = root.clone();
    e1.open_file(file.clone()).unwrap();
    e1.set_cursor(3, 2);
    e1.save_shada();
    let mut e2 = editor("");
    e2.project_root = root.clone();
    e2.open_file(file.clone()).unwrap();
    assert_eq!(e2.cursor(), (3, 2), "cursor restored from shada");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn shada_persists_registers_and_history() {
    let root = temp();
    let mut e1 = editor("");
    e1.project_root = root.clone();
    e1.registers.set(Some('a'), "hello".into(), false);
    e1.command_history.push("wq".into());
    e1.search_history.push("needle".into());
    e1.save_shada();
    let mut e2 = editor("");
    e2.project_root = root.clone();
    e2.load_shada();
    assert_eq!(
        e2.registers.get(Some('a')).map(|r| r.text.clone()),
        Some("hello".to_string()),
        "named register restored"
    );
    assert!(
        e2.command_history.contains(&"wq".to_string()),
        "command history restored"
    );
    assert!(
        e2.search_history.contains(&"needle".to_string()),
        "search history restored"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn shada_persists_marks_and_jumps() {
    let root = temp();
    let file = root.join("m.txt");
    std::fs::write(&file, "l0\nl1\nl2\nl3\nl4\n").unwrap();
    let mut e1 = editor("");
    e1.project_root = root.clone();
    e1.open_file(file.clone()).unwrap();
    let bufpath = e1.buf().path.clone();
    let id = e1.buf().id;
    e1.marks.insert(
        'a',
        crate::navigation::Location {
            buffer: id,
            path: bufpath.clone(),
            line: 2,
            col: 0,
        },
    );
    e1.jumps.push(crate::navigation::Location {
        buffer: id,
        path: bufpath.clone(),
        line: 4,
        col: 0,
    });
    e1.save_shada();
    let mut e2 = editor("");
    e2.project_root = root.clone();
    e2.load_shada();
    let m = e2.marks.get(&'a').expect("mark a restored");
    assert_eq!((m.line, m.col), (2, 0));
    assert!(m.path.as_ref().unwrap().to_string_lossy().ends_with("m.txt"));
    assert!(
        e2.jumps.iter().any(|j| j.line == 4
            && j.path.as_ref().is_some_and(|p| p.to_string_lossy().ends_with("m.txt"))),
        "jump restored"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn shada_skips_vcs_message_files() {
    let root = temp();
    let file = root.join("COMMIT_EDITMSG");
    std::fs::write(&file, "subject\n\nbody line\n").unwrap();
    let mut e1 = editor("");
    e1.project_root = root.clone();
    e1.open_file(file.clone()).unwrap();
    e1.set_cursor(2, 0);
    e1.save_shada();
    let mut e2 = editor("");
    e2.project_root = root.clone();
    e2.open_file(file.clone()).unwrap();
    assert_eq!(e2.cursor(), (0, 0), "commit message opens at the top");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn shada_respects_restore_cursor_off() {
    let root = temp();
    let file = root.join("b.txt");
    std::fs::write(&file, "a\nb\nc\nd\n").unwrap();
    let mut e1 = editor("");
    e1.project_root = root.clone();
    e1.open_file(file.clone()).unwrap();
    e1.set_cursor(2, 0);
    e1.save_shada();
    let mut e2 = editor("");
    e2.config.restore_cursor = false;
    e2.project_root = root.clone();
    e2.open_file(file.clone()).unwrap();
    assert_eq!(e2.cursor(), (0, 0), "restore disabled leaves cursor at top");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn spell_spans_and_nav() {
    let mut e = editor("hello wrold\ngood bunes here\n");
    e.dictionary = Some(crate::spell::Dictionary::for_test(&[
        "hello", "world", "good", "here",
    ]));
    // Off by default: no spans computed.
    e.update_spell_spans();
    assert!(e.spell_spans.is_empty(), "spell off → no spans");
    e.config.spell = true;
    e.update_spell_spans();
    // "wrold" (line 0) and "bunes" (line 1) are misspelled.
    assert_eq!(e.spell_spans.len(), 2, "two misspellings: {:?}", e.spell_spans);
    assert_eq!(e.spell_spans[0].0, 0);
    assert_eq!(e.spell_spans[1].0, 1);
    // `]s` jumps to the first misspelling from the top.
    e.set_cursor(0, 0);
    e.spell_nav(true);
    assert_eq!(e.cursor().0, 0);
    assert_eq!(e.cursor().1, e.spell_spans[0].1, "on 'wrold'");
    e.spell_nav(true);
    assert_eq!(e.cursor().0, 1, "]s advances to 'bunes' on line 1");
    e.spell_nav(false);
    assert_eq!(e.cursor().0, 0, "[s goes back to 'wrold'");
}
#[test]
fn set_rainbow_toggles_config() {
    let mut e = editor("(a)\n");
    assert!(!e.config.rainbow);
    keys(&mut e, ":set rainbow\n");
    assert!(e.config.rainbow);
    keys(&mut e, ":set norainbow\n");
    assert!(!e.config.rainbow);
}
#[test]
fn set_list_toggles_config() {
    let mut e = editor("abc\n");
    assert!(!e.config.list, "off by default");
    keys(&mut e, ":set list\n");
    assert!(e.config.list);
    keys(&mut e, ":set nolist\n");
    assert!(!e.config.list);
}
#[test]
fn esc_clears_document_colors() {
    let mut e = editor("red\n");
    e.document_colors = vec![(0, 0, 3, (255, 0, 0))];
    e.document_colors_buffer = Some(e.buf().id);
    e.document_colors_edit_seq = e.buf().edit_seq;
    e.feed_key(Key::Esc);
    assert!(e.document_colors.is_empty(), "Esc clears document colors");
}
#[test]
fn set_runtime_options() {
    let mut e = editor("abc\n");
    keys(&mut e, ":set relativenumber\n");
    assert!(e.config.relativenumber);
    keys(&mut e, ":set nornu\n");
    assert!(!e.config.relativenumber);
    keys(&mut e, ":set noignorecase\n");
    assert!(!e.config.ignorecase);
    keys(&mut e, ":set ic\n");
    assert!(e.config.ignorecase);
    keys(&mut e, ":set noexpandtab\n");
    assert!(!e.config.expandtab);
    assert!(!e.buf().expandtab, "buffer expandtab updated too");
    keys(&mut e, ":set tabstop=8\n");
    assert_eq!(e.config.tabstop, 8);
    assert_eq!(e.buf().tabstop, 8, "buffer tabstop updated too");
    keys(&mut e, ":set sw=2\n");
    assert_eq!(e.config.shiftwidth, 2);
    keys(&mut e, ":set scrolloff=5\n");
    assert_eq!(e.config.scrolloff, 5);
    keys(&mut e, ":set textwidth=100\n");
    assert_eq!(e.config.textwidth, 100);
    keys(&mut e, ":set nosmartcase\n");
    assert!(!e.config.smartcase);
}
#[test]
fn set_colorcolumn_parses_value() {
    let mut e = editor("abc\n");
    assert_eq!(e.config.colorcolumn, 0, "off by default");
    keys(&mut e, ":set colorcolumn=80\n");
    assert_eq!(e.config.colorcolumn, 80);
    keys(&mut e, ":set cc=0\n");
    assert_eq!(e.config.colorcolumn, 0);
    keys(&mut e, ":set cc=100\n");
    assert_eq!(e.config.colorcolumn, 100, "short form works");
}
#[test]
fn reflow_wraps_paragraph_to_width() {
    let text = "the quick brown fox jumps over the lazy dog again today";
    let out = crate::operator::reflow(text, 20);
    for line in out.lines() {
        assert!(line.chars().count() <= 20, "line too long: {line:?}");
    }
    assert_eq!(
        out.split_whitespace().collect::<Vec<_>>(),
        text.split_whitespace().collect::<Vec<_>>(),
        "words preserved in order"
    );
    assert!(out.contains('\n'), "should wrap onto multiple lines");
}
#[test]
fn reflow_keeps_comment_leader_on_each_line() {
    // A `//` comment block reflowed to a narrow width keeps `//` on every line
    // and never treats the leader as a word.
    let text = "// alpha beta gamma delta epsilon zeta eta theta";
    let out = crate::operator::reflow(text, 16);
    assert!(out.contains('\n'), "should wrap: {out:?}");
    for line in out.lines() {
        assert!(line.starts_with("// "), "leader kept: {line:?}");
        // The leader must not appear twice (i.e. not consumed as a word).
        assert_eq!(line.matches("//").count(), 1, "single leader per line: {line:?}");
    }
    // All prose words are preserved in order, with no stray `//`.
    let words: Vec<&str> = out
        .split_whitespace()
        .filter(|w| *w != "//")
        .collect();
    assert_eq!(
        words,
        vec!["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"]
    );
    // An indented ` * ` block-comment continuation keeps its leader too.
    let star = "    * one two three four five six seven eight";
    let out = crate::operator::reflow(star, 18);
    for line in out.lines() {
        assert!(line.starts_with("    * "), "star leader kept: {line:?}");
    }
}
#[test]
fn reflow_preserves_indent_and_paragraphs() {
    let text = "    alpha beta gamma delta epsilon\n\n    second para here now";
    let out = crate::operator::reflow(text, 12);
    assert!(out.contains("\n\n"), "blank line between paragraphs kept:\n{out}");
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        assert!(line.starts_with("    "), "indent preserved: {line:?}");
    }
}
#[test]
fn gqq_reflows_current_line() {
    let mut e = editor("aaa bbb ccc ddd eee fff ggg hhh iii jjj\nunrelated\n");
    e.config.textwidth = 15;
    e.set_cursor(0, 0);
    keys(&mut e, "gqq");
    assert!(
        e.buf().line_text(0).chars().count() <= 15,
        "line 0 should be wrapped: {:?}",
        e.buf().line_text(0)
    );
    assert!(
        e.buf().rope.to_string().contains("\nunrelated\n"),
        "the following line is untouched"
    );
}
#[test]
fn gq_paragraph_motion_reflows_only_that_paragraph() {
    let mut e = editor("one two three four five six seven eight nine ten\n\nnext para stays\n");
    e.config.textwidth = 20;
    e.set_cursor(0, 0);
    keys(&mut e, "gq}");
    let s = e.buf().rope.to_string();
    let end = s.find("\n\n").unwrap_or(s.len());
    for line in s[..end].lines() {
        assert!(line.chars().count() <= 20, "reflowed line within width: {line:?}");
    }
    assert!(s.contains("next para stays"), "second paragraph untouched");
}
#[test]
fn cursor_hold_fires_once_when_cursor_rests() {
    let mut e = editor("hello world\nsecond\n");
    e.config.updatetime_ms = 0; // fire as soon as armed
    e.set_cursor(0, 0);
    assert!(!e.poll_cursor_hold(), "first tick arms the timer");
    assert!(e.poll_cursor_hold(), "second tick fires the hold");
    assert!(!e.poll_cursor_hold(), "does not re-fire at the same spot");
    e.set_cursor(1, 0);
    assert!(!e.poll_cursor_hold(), "re-arms after a move");
    assert!(e.poll_cursor_hold(), "fires again at the new spot");
}
#[test]
fn cursor_hold_inactive_outside_normal_visual() {
    let mut e = editor("abc\n");
    e.config.updatetime_ms = 0;
    e.mode = crate::mode::Mode::Insert;
    assert!(!e.poll_cursor_hold());
    assert!(!e.poll_cursor_hold());
}
#[test]
fn cursor_move_clears_stale_illuminate_highlights() {
    let mut e = editor("foo foo\n");
    e.config.updatetime_ms = 0;
    e.set_cursor(0, 0);
    e.document_highlights = vec![(0, 0, 0, 2)];
    e.document_highlights_buffer = Some(e.buf().id);
    e.document_highlights_edit_seq = e.buf().edit_seq;
    assert!(!e.poll_cursor_hold(), "first observation keeps current highlights");
    assert!(
        !e.document_highlights.is_empty(),
        "highlights at the current spot are preserved"
    );
    e.set_cursor(0, 4);
    assert!(e.poll_cursor_hold(), "a real move triggers a redraw");
    assert!(
        e.document_highlights.is_empty(),
        "stale highlights cleared after the move"
    );
}
fn rust_editor_with_syntax(src: &str) -> Editor {
    let mut e = editor(src);
    let mut syn = crate::syntax::Syntax::new(crate::syntax::Lang::Rust).expect("rust grammar");
    syn.reparse(std::rc::Rc::from(src));
    e.syntax = Some(syn);
    e
}
#[test]
fn gqq_is_dot_repeatable() {
    let mut e = editor("aaaa bbbb cccc dddd eeee\nffff gggg hhhh iiii jjjj\n");
    e.config.textwidth = 9;
    keys(&mut e, "gqq"); // reflow line 0 -> 3 short lines
    // The original second line is now the last line and still long.
    keys(&mut e, "G");
    assert!(
        e.buf().line_text(e.cursor().0).chars().count() > 11,
        "cursor on the still-long line before dot"
    );
    keys(&mut e, "."); // repeat the reflow on this line
    let out = e.buf().rope.to_string();
    assert!(
        out.lines().all(|l| l.chars().count() <= 11),
        "every line reflowed to width: {out:?}"
    );
    assert!(out.contains("ffff") && out.contains("jjjj"), "words preserved: {out:?}");
}
#[test]
fn visual_gq_reflows_the_selection() {
    let mut e = editor("aaaa bbbb cccc dddd eeee ffff\n");
    e.config.textwidth = 10;
    // Visual-line select the line, then `gq` reflows it in place.
    keys(&mut e, "Vgq");
    let out = e.buf().rope.to_string();
    assert!(
        out.lines().count() > 1,
        "reflow wrapped into multiple lines: {out:?}"
    );
    assert!(
        out.lines().all(|l| l.chars().count() <= 12),
        "each line respects textwidth: {out:?}"
    );
    // Back in Normal mode after the visual operator.
    assert!(matches!(e.mode, crate::mode::Mode::Normal));
}
#[test]
fn paragraph_text_object_inner_and_around() {
    // `dip` deletes just the non-blank paragraph run.
    let mut e = editor("a1\na2\n\nb1\nb2\n");
    e.set_cursor(0, 0);
    keys(&mut e, "dip");
    assert_eq!(e.buf().rope.to_string(), "\nb1\nb2\n");
    // `dap` also swallows the trailing blank line.
    let mut e = editor("a1\na2\n\nb1\nb2\n");
    e.set_cursor(0, 0);
    keys(&mut e, "dap");
    assert_eq!(e.buf().rope.to_string(), "b1\nb2\n");
    // On a blank line, `dip` removes the blank run between paragraphs.
    let mut e = editor("a1\n\n\nb1\n");
    e.set_cursor(1, 0);
    keys(&mut e, "dip");
    assert_eq!(e.buf().rope.to_string(), "a1\nb1\n");
}
#[test]
fn large_file_mode_skips_expensive_scans() {
    // A buffer over the (tiny, test-set) threshold skips syntax + todo scans.
    let root = temp();
    let file = root.join("big.rs");
    std::fs::write(&file, "fn f() {}\n".repeat(600)).unwrap(); // ~6 KB
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.config.large_file_kb = 1; // 1 KiB threshold -> this buffer is "large"
    assert!(e.buf_is_large());
    e.ensure_syntax();
    assert!(e.syntax.is_none(), "large file has no tree-sitter tree");
    e.config.todo_highlight = true;
    e.update_todo_spans();
    assert!(e.todo_spans.is_empty(), "large file skips TODO scan");
    // With the cutoff disabled, syntax parses normally.
    e.config.large_file_kb = 0;
    assert!(!e.buf_is_large());
    e.ensure_syntax();
    assert!(e.syntax.is_some(), "with the cutoff off, syntax parses");
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn todo_highlight_marks_comment_keywords_only() {
    let src = "// TODO: a\n// FIXME b\nlet TODO = 1;\n// TODONT z\n";
    let mut e = rust_editor_with_syntax(src);
    e.config.todo_highlight = true;
    e.update_todo_spans();
    // TODO (yellow=0) and FIXME (red=1) in comments are marked.
    assert!(
        e.todo_spans.iter().any(|&(l, s, en, c)| l == 0 && s == 3 && en == 7 && c == 0),
        "TODO in comment: {:?}",
        e.todo_spans
    );
    assert!(
        e.todo_spans.iter().any(|&(l, s, _, c)| l == 1 && s == 3 && c == 1),
        "FIXME in comment: {:?}",
        e.todo_spans
    );
    // `TODO` used as a code identifier (line 2) is not in a comment -> ignored.
    assert!(!e.todo_spans.iter().any(|&(l, _, _, _)| l == 2));
    // `TODONT` is not a whole-word match -> ignored.
    assert!(!e.todo_spans.iter().any(|&(l, _, _, _)| l == 3));
    // Turning it off clears the spans.
    e.config.todo_highlight = false;
    e.update_todo_spans();
    assert!(e.todo_spans.is_empty());
}
#[test]
fn ghost_text_suggests_from_buffer_and_accepts_on_tab() {
    let mut e = editor("hello world foo\nbar\n");
    e.config.ghost_text = true;
    // Insert a prefix on line 1 that matches the start of line 0.
    e.set_cursor(1, 0);
    keys(&mut e, "cchello wo"); // change line -> "hello wo", still in insert
    e.update_ghost();
    assert_eq!(
        e.ghost.as_ref().map(|(_, _, t)| t.as_str()),
        Some("rld foo"),
        "ghost completes the matching line"
    );
    // Ctrl-l accepts the suggestion.
    e.feed_key(Key::Ctrl('l'));
    assert_eq!(e.buf().line_text(1), "hello world foo");
    assert!(e.ghost.is_none(), "ghost cleared after accept");
    // With ghost_text off, no suggestion is produced.
    e.config.ghost_text = false;
    e.update_ghost();
    assert!(e.ghost.is_none());
}
#[test]
fn injection_highlights_markdown_code_fence() {
    let root = temp();
    let file = root.join("doc.md");
    std::fs::write(&file, "text\n```rust\nfn main() {}\n```\nmore\n").unwrap();
    let mut e = editor("");
    e.project_root = root.clone();
    e.open_file(file).unwrap();
    e.update_injections();
    assert!(!e.injection_spans.is_empty(), "fence content should be highlighted");
    // Every injected span lies inside the fenced content (line index 2).
    let content_start = "text\n```rust\n".len();
    let content_end = content_start + "fn main() {}\n".len();
    assert!(
        e.injection_spans
            .iter()
            .all(|&(s, en, _)| s >= content_start && en <= content_end),
        "spans stay within the fence: {:?}",
        e.injection_spans
    );
    // `fn` is a keyword -> a Keyword injected span.
    assert!(e
        .injection_spans
        .iter()
        .any(|&(_, _, c)| c == crate::syntax::HlClass::Keyword));
    // A non-markdown buffer (no fences) has no injections.
    let mut e2 = editor("fn main() {}\n");
    e2.update_injections();
    assert!(e2.injection_spans.is_empty());
    std::fs::remove_dir_all(root).ok();
}
#[test]
fn rainbow_skips_brackets_in_strings_and_comments() {
    // Real brackets on lines 0 and 3; a `(` inside a string (line 1) and a `]`
    // inside a comment (line 2) must be excluded from the rainbow set.
    let src = "fn f() {\n    let s = \"(\";\n    // ]\n}\n";
    let e = rust_editor_with_syntax(src);
    let brackets = crate::render::rainbow_brackets(&e, e.buf());
    let positions: Vec<(usize, usize)> = brackets.iter().map(|&(l, c, _)| (l, c)).collect();
    assert_eq!(
        positions.iter().filter(|&&(l, _)| l == 0).count(),
        3,
        "the three code brackets on line 0 are colored: {positions:?}"
    );
    assert!(
        positions.iter().any(|&(l, _)| l == 3),
        "the closing brace on line 3 is colored: {positions:?}"
    );
    assert!(
        !positions.iter().any(|&(l, _)| l == 1 || l == 2),
        "brackets in the string/comment are excluded: {positions:?}"
    );
}
#[test]
fn foldsyntax_folds_function_bodies() {
    let src = "fn one() {\n    a();\n    b();\n}\nfn two() {\n    c();\n}\nconst X: i32 = 1;\n";
    let mut e = rust_editor_with_syntax(src);
    e.fold_by_syntax();
    // Two functions -> two multi-line folds (the one-line const isn't folded).
    assert_eq!(e.buf().folds.len(), 2, "one fold per multi-line function");
    assert!(e.buf().folds.iter().all(|f| f.closed));
    assert!(e.buf().folds.iter().any(|f| f.start == 0 && f.end == 3));
    assert!(e.buf().folds.iter().any(|f| f.start == 4 && f.end == 6));
    // fn one's body is hidden; its first line and the const stay visible.
    assert!(e.buf().line_hidden(1) && e.buf().line_hidden(2));
    assert!(!e.buf().line_hidden(0) && !e.buf().line_hidden(7));
}
#[test]
fn goto_function_jumps_between_definitions() {
    let src = "fn one() {\n    a();\n}\nfn two() {\n    b();\n}\nfn three() {}\n";
    let mut e = rust_editor_with_syntax(src);
    e.set_cursor(0, 0);
    e.goto_function(true, 1);
    assert_eq!(e.cursor().0, 3, "]f -> fn two");
    e.goto_function(true, 1);
    assert_eq!(e.cursor().0, 6, "]f -> fn three");
    e.goto_function(true, 1);
    assert_eq!(e.cursor().0, 6, "]f past last function stays put");
    e.goto_function(false, 1);
    assert_eq!(e.cursor().0, 3, "[f -> fn two");
    e.goto_function(false, 1);
    assert_eq!(e.cursor().0, 0, "[f -> fn one");
}
#[test]
fn goto_function_honors_count() {
    let src = "fn one() {}\nfn two() {}\nfn three() {}\n";
    let mut e = rust_editor_with_syntax(src);
    e.set_cursor(0, 0);
    e.goto_function(true, 2);
    assert_eq!(e.cursor().0, 2, "2]f skips one function");
}
#[test]
fn bracket_f_navigates_functions_via_keys() {
    let src = "fn one() {}\nfn two() {}\nfn three() {}\n";
    let mut e = rust_editor_with_syntax(src);
    e.set_cursor(0, 0);
    keys(&mut e, "]f");
    assert_eq!(e.cursor().0, 1);
    keys(&mut e, "]f");
    assert_eq!(e.cursor().0, 2);
    keys(&mut e, "[f");
    assert_eq!(e.cursor().0, 1);
}
#[test]
fn goto_function_without_syntax_is_noop() {
    let mut e = editor("fn a() {}\nfn b() {}\n");
    e.set_cursor(0, 0);
    e.goto_function(true, 1);
    assert_eq!(e.cursor().0, 0);
}
fn arg_obj(text: &str, col: usize, seq: &str) -> String {
    let mut e = editor(text);
    e.set_cursor(0, col);
    keys(&mut e, seq);
    e.buf().line_text(0)
}
#[test]
fn argument_object_inner_middle() {
    // `dia` on `b` deletes just the argument, leaving the commas.
    assert_eq!(arg_obj("foo(a, b, c)\n", 7, "dia"), "foo(a, , c)");
}
#[test]
fn argument_object_a_first_takes_trailing_comma() {
    assert_eq!(arg_obj("foo(a, b, c)\n", 4, "daa"), "foo(b, c)");
}
#[test]
fn argument_object_a_last_takes_leading_comma() {
    assert_eq!(arg_obj("foo(a, b, c)\n", 10, "daa"), "foo(a, b)");
}
#[test]
fn argument_object_sole_arg() {
    assert_eq!(arg_obj("foo(x)\n", 4, "dia"), "foo()");
    assert_eq!(arg_obj("foo(x)\n", 4, "daa"), "foo()");
}
#[test]
fn argument_object_skips_nested_and_quoted_commas() {
    // A nested call is one argument; its inner commas don't split.
    assert_eq!(arg_obj("f(a, g(b, c), d)\n", 5, "daa"), "f(a, d)");
    // A comma inside a string is not a separator.
    assert_eq!(arg_obj("f(\"a, b\", c)\n", 3, "daa"), "f(c)");
}
#[test]
fn argument_object_outside_parens_is_noop() {
    assert_eq!(arg_obj("abc def\n", 1, "daa"), "abc def");
}
#[test]
fn auto_indent_adds_level_after_open_bracket() {
    // Default buffer: expandtab, shiftwidth 4.
    let e = editor("fn f() {\n    body\n");
    assert_eq!(e.auto_indent(0, 8), "    ", "after open-brace gains one level");
    assert_eq!(e.auto_indent(1, 8), "    ", "no bracket → copies indent");
    let nested = editor("    if x {\n");
    assert_eq!(nested.auto_indent(0, 10), "        ", "4 base + 4 level");
}
#[test]
fn auto_indent_respects_smartindent_off_and_tabs() {
    let mut e = editor("fn f() {\n");
    e.config.smartindent = false;
    assert_eq!(e.auto_indent(0, 8), "", "smartindent off → copy only");
    let mut t = editor("\tif x {\n");
    t.buf_mut().expandtab = false;
    t.buf_mut().shiftwidth = 4;
    assert_eq!(t.auto_indent(0, 7), "\t\t", "tab base + tab level");
}
#[test]
fn enter_smartindents_after_brace() {
    let mut e = editor("fn f() {\n");
    keys(&mut e, "A\nx\x1b"); // append at EOL, newline, type x
    assert_eq!(e.buf().line_text(1), "    x", "Enter after open-brace indents");
}
#[test]
fn open_below_smartindents_after_brace() {
    let mut e = editor("if x {\n");
    e.set_cursor(0, 0);
    keys(&mut e, "ohi\x1b");
    assert_eq!(e.buf().line_text(1), "    hi", "o after open-brace indents");
}
#[test]
fn open_above_copies_indent_without_bracket_increase() {
    let mut e = editor("    if x {\n");
    e.set_cursor(0, 4);
    keys(&mut e, "Ohi\x1b");
    assert_eq!(
        e.buf().line_text(0),
        "    hi",
        "O copies indent, no bracket bump"
    );
}
#[test]
fn set_ff_command_changes_fileformat() {
    use crate::buffer::FileFormat;
    let mut e = editor("hello\n");
    keys(&mut e, ":set ff=dos\n");
    assert_eq!(e.buf().fileformat, FileFormat::Dos);
    keys(&mut e, ":set ff=bogus\n");
    assert_eq!(
        e.buf().fileformat,
        FileFormat::Dos,
        "an invalid value leaves the format unchanged"
    );
}
