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
    crate::git_tools::apply_patch(&root, patch, false).unwrap();
    let staged = git(&["show", ":sample.txt"]);
    assert!(staged.contains("changed one"));
    assert!(!staged.contains("changed two"));
    let r = crate::git_tools::hunks(&root, &file, true).unwrap();
    let patch = r.entries[0].action.as_ref().unwrap()["_vaayu_git_patch"]
        .as_str()
        .unwrap();
    crate::git_tools::apply_patch(&root, patch, true).unwrap();
    assert_eq!(git(&["show", ":sample.txt"]), old);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), new);
    std::fs::remove_dir_all(root).unwrap();
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
    let expansion =
        crate::snippet::expand("fn ${1:name}(${2|x,y|}) {$0}", &Default::default()).unwrap();
    let (text, stops, mirrors) = (expansion.text, expansion.stops, expansion.mirrors);
    assert_eq!(text, "fn name(x) {}");
    assert_eq!(stops[0], (3, 7));
    let mut e = editor(&text);
    e.enter_insert();
    e.set_cursor_insert(0, 3);
    e.snippet = Some(crate::snippet::Session {
        mirrors,
        stops,
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
        request_timeout_ms: 1000,
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
    std::thread::sleep(std::time::Duration::from_millis(1100));
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
    let x = crate::snippet::expand("${1:name} = $1; $0", &Default::default()).unwrap();
    let mut e = editor(&x.text);
    e.enter_insert();
    e.set_cursor_insert(0, 0);
    e.snippet = Some(crate::snippet::Session {
        stops: x.stops,
        mirrors: x.mirrors,
        current: 0,
        selected: true,
    });
    keys(&mut e, "value");
    e.feed_key(Key::Tab);
    assert_eq!(e.buf().line_text(0), "value = value; ");
    assert_eq!(e.cursor().1, 15);
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
