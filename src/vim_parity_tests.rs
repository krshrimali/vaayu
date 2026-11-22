//! Tests for the Vim keys and Ex commands added for parity: motions, text
//! objects, operators, insert-mode keys, registers, windows and Ex.
use crate::{config::Config, editor::Editor, mode::Mode};
use std::path::PathBuf;

fn editor(text: &str) -> Editor {
    let cfg = Config {
        clipboard_unnamedplus: false,
        jk_escape: false,
        ..Config::default()
    };
    let mut e = Editor::new(cfg);
    e.project_root = temp();
    e.buf_mut().rope = ropey::Rope::from_str(text);
    e.buf_mut().mark_saved();
    e.screen_rows = 40;
    e
}

fn temp() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(1);
    let p = std::env::temp_dir().join(format!(
        "vaayu-parity-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Feeds keys written in Vim notation (`<Esc>`, `<CR>`, `<C-w>`, `<BS>`).
fn feed(e: &mut Editor, keys: &str) {
    for k in crate::keymap::parse_keys(keys, '\u{1}') {
        e.feed_key(k);
    }
}

fn text(e: &Editor) -> String {
    e.buf().rope.to_string()
}

fn at(text_: &str, line: usize, col: usize, keys: &str) -> Editor {
    let mut e = editor(text_);
    e.set_cursor(line, col);
    feed(&mut e, keys);
    e
}

// ---------------------------------------------------------------- motions

#[test]
fn line_motions_minus_plus_enter_underscore_and_bar() {
    let t = "  a\n  b\n    c\nabcdef\n";
    assert_eq!(at(t, 1, 0, "-").cursor(), (0, 2));
    assert_eq!(at(t, 0, 0, "+").cursor(), (1, 2));
    assert_eq!(at(t, 0, 0, "2<CR>").cursor(), (2, 4));
    assert_eq!(at(t, 2, 0, "_").cursor(), (2, 4));
    assert_eq!(at(t, 0, 0, "2_").cursor(), (1, 2));
    assert_eq!(at(t, 3, 0, "4|").cursor(), (3, 3));
    // Linewise with an operator.
    assert_eq!(text(&at(t, 0, 0, "d+")), "    c\nabcdef\n");
    assert_eq!(text(&at(t, 1, 0, "d-")), "    c\nabcdef\n");
}

#[test]
fn screen_motions_h_l_m() {
    let t: String = (0..10).map(|i| format!("line {i}\n")).collect();
    let mut e = editor(&t);
    e.screen_rows = 5;
    e.config.scrolloff = 0;
    e.buf_mut().top_line = 0;
    e.set_cursor(2, 0);
    feed(&mut e, "L");
    assert_eq!(e.cursor().0, 4);
    feed(&mut e, "H");
    assert_eq!(e.cursor().0, 0);
    feed(&mut e, "M");
    assert_eq!(e.cursor().0, 2);
    feed(&mut e, "2H");
    assert_eq!(e.cursor().0, 1);
    // `dL` deletes to the bottom of the screen, linewise.
    let mut e = editor(&t);
    e.screen_rows = 3;
    e.config.scrolloff = 0;
    feed(&mut e, "dL");
    assert!(text(&e).starts_with("line 3\n"), "{}", text(&e));
}

#[test]
fn sentence_motions_and_operators() {
    let t = "One two. Three four! Five?\n\nNext para here.\n";
    let mut e = at(t, 0, 0, ")");
    assert_eq!(e.cursor(), (0, 9));
    feed(&mut e, ")");
    assert_eq!(e.cursor(), (0, 21));
    feed(&mut e, ")");
    assert_eq!(e.cursor(), (1, 0), "an empty line is a sentence boundary");
    feed(&mut e, ")");
    assert_eq!(e.cursor(), (2, 0));
    feed(&mut e, "(");
    assert_eq!(e.cursor(), (1, 0));
    let e = at(t, 0, 12, "(");
    assert_eq!(
        e.cursor(),
        (0, 9),
        "( goes to the start of the current sentence"
    );
    assert_eq!(
        text(&at(t, 0, 0, "d)")),
        "Three four! Five?\n\nNext para here.\n"
    );
}

#[test]
fn g_underscore_gm_go_and_ge_big() {
    assert_eq!(at("abc   \n", 0, 0, "g_").cursor(), (0, 2));
    assert_eq!(at("abcdefghij\n", 0, 0, "gM").cursor(), (0, 5));
    assert_eq!(at("abc\ndef\n", 1, 0, "3go").cursor(), (0, 2));
    assert_eq!(at("abc\ndef\n", 0, 0, "6go").cursor(), (1, 1));
    assert_eq!(at("aa.b cc\n", 0, 6, "gE").cursor(), (0, 3));
    let mut e = editor("abcdefghij\n");
    e.view_width = 8;
    feed(&mut e, "gm");
    assert_eq!(e.cursor(), (0, 4));
}

#[test]
fn bracket_motions_sections_unmatched_methods_comments() {
    let t = "fn a() {\n    x(1, (2));\n}\n\nfn b() {\n    y\n}\n";
    assert_eq!(at(t, 0, 0, "]]").cursor(), (4, 0));
    assert_eq!(at(t, 5, 0, "[[").cursor(), (4, 0));
    assert_eq!(at(t, 0, 0, "][").cursor(), (2, 0));
    assert_eq!(at(t, 5, 0, "[]").cursor(), (2, 0));
    assert_eq!(at(t, 1, 7, "[(").cursor(), (1, 5));
    assert_eq!(
        at(t, 1, 10, "[(").cursor(),
        (1, 9),
        "the innermost open paren"
    );
    assert_eq!(at(t, 1, 7, "])").cursor(), (1, 12));
    assert_eq!(at(t, 1, 4, "[{").cursor(), (0, 7));
    assert_eq!(at(t, 1, 4, "]}").cursor(), (2, 0));
    assert_eq!(at(t, 0, 0, "]m").cursor(), (0, 7));
    assert_eq!(at(t, 1, 0, "]m").cursor(), (4, 7));
    assert_eq!(at(t, 5, 0, "[m").cursor(), (4, 7));
    assert_eq!(at(t, 1, 0, "]M").cursor(), (2, 0));
    // Operators take them as motions.
    assert_eq!(
        text(&at("f(a, b)\n", 0, 4, "d[(")),
        "f b)\n",
        "exclusive of the cursor"
    );
    let c = "x /* one\ntwo */ y\n";
    assert_eq!(at(c, 1, 0, "[*").cursor(), (0, 2));
    assert_eq!(at(c, 0, 0, "]*").cursor(), (1, 5), "on the / of */");
}

#[test]
fn mark_navigation_brackets() {
    let mut e = editor("a\nb\nc\nd\ne\n");
    e.set_cursor(1, 0);
    feed(&mut e, "ma");
    e.set_cursor(3, 0);
    feed(&mut e, "mb");
    e.set_cursor(0, 0);
    feed(&mut e, "]'");
    assert_eq!(e.cursor().0, 1);
    feed(&mut e, "]'");
    assert_eq!(e.cursor().0, 3);
    feed(&mut e, "['");
    assert_eq!(e.cursor().0, 1);
}

// ----------------------------------------------------------- text objects

#[test]
fn sentence_and_tag_text_objects() {
    // Like Vim, `is` leaves the whitespace around the sentence.
    assert_eq!(
        text(&at("One. Two three. Four.\n", 0, 6, "dis")),
        "One.  Four.\n"
    );
    assert_eq!(
        text(&at("One. Two three. Four.\n", 0, 6, "das")),
        "One. Four.\n"
    );
    assert_eq!(text(&at("<a>xy</a>\n", 0, 3, "dit")), "<a></a>\n");
    assert_eq!(text(&at("q <a>xy</a> r\n", 0, 4, "dat")), "q  r\n");
    // Nested: the innermost pair around the cursor.
    assert_eq!(
        text(&at("<div><b>bold</b> rest</div>\n", 0, 9, "dit")),
        "<div><b></b> rest</div>\n"
    );
    assert_eq!(
        text(&at("<div><b>bold</b> rest</div>\n", 0, 18, "dit")),
        "<div></div>\n"
    );
    // `cit` on an empty tag inserts inside it.
    let mut e = at("<p></p>\n", 0, 1, "citX<Esc>");
    assert_eq!(text(&e), "<p>X</p>\n");
    feed(&mut e, "u");
    assert_eq!(text(&e), "<p></p>\n");
}

// -------------------------------------------------------------- operators

#[test]
fn case_operators_and_rot13() {
    assert_eq!(text(&at("Hello World\n", 0, 0, "guiw")), "hello World\n");
    assert_eq!(text(&at("Hello World\n", 0, 0, "gUU")), "HELLO WORLD\n");
    assert_eq!(text(&at("Hello World\n", 0, 0, "gUgU")), "HELLO WORLD\n");
    assert_eq!(text(&at("Hello World\n", 0, 0, "g~w")), "hELLO World\n");
    assert_eq!(text(&at("Hello\n", 0, 0, "g??")), "Uryyb\n");
    assert_eq!(text(&at("ab cd\n", 0, 0, "guu")), "ab cd\n");
    // Dot-repeat.
    let mut e = at("aa\nbb\n", 0, 0, "gUiw");
    feed(&mut e, "j.");
    assert_eq!(text(&e), "AA\nBB\n");
    // Visual.
    assert_eq!(text(&at("abc def\n", 0, 0, "veU")), "ABC def\n");
    assert_eq!(text(&at("ABC def\n", 0, 0, "vlu")), "abC def\n");
    assert_eq!(text(&at("abc\n", 0, 0, "Vg?")), "nop\n");
    // Visual block.
    assert_eq!(text(&at("abc\nabc\n", 0, 1, "<C-v>jU")), "aBc\naBc\n");
}

#[test]
fn reindent_operator() {
    let t = "fn a() {\nx();\nif y {\nz();\n}\n}\n";
    let e = at(t, 0, 0, "=G");
    assert_eq!(
        text(&e),
        "fn a() {\n    x();\n    if y {\n        z();\n    }\n}\n"
    );
    let e = at("fn a() {\n        x();\n}\n", 1, 0, "==");
    assert_eq!(text(&e), "fn a() {\n    x();\n}\n");
    let e = at("{\nx\ny\n}\n", 1, 0, "Vj=");
    assert_eq!(text(&e), "{\n    x\n    y\n}\n");
}

#[test]
fn operators_take_search_motions() {
    assert_eq!(text(&at("aa bb cc\n", 0, 0, "d/cc<CR>")), "cc\n");
    assert_eq!(text(&at("aa bb cc\n", 0, 6, "d?bb<CR>")), "aa cc\n");
    let mut e = at("aa bb cc bb\n", 0, 0, "/bb<CR>0");
    feed(&mut e, "dn");
    assert_eq!(text(&e), "bb cc bb\n");
    assert_eq!(text(&at("aa bb aa\n", 0, 0, "d*")), "aa\n");
    assert_eq!(text(&at("a,b,c\n", 0, 0, "f,d;")), "ac\n");
    // Esc during the search cancels the operator.
    let mut e = at("aa bb\n", 0, 0, "d/bb<Esc>");
    feed(&mut e, "l");
    assert_eq!(text(&e), "aa bb\n");
    assert_eq!(e.cursor(), (0, 1));
}

#[test]
fn gn_selects_and_cgn_repeats() {
    let mut e = at("foo bar foo bar\n", 0, 0, "/bar<CR>0");
    feed(&mut e, "gn");
    assert!(matches!(e.mode, Mode::Visual(_)));
    assert_eq!(e.cursor(), (0, 6));
    feed(&mut e, "<Esc>0cgnBAZ<Esc>");
    assert_eq!(text(&e), "foo BAZ foo bar\n");
    feed(&mut e, ".");
    assert_eq!(text(&e), "foo BAZ foo BAZ\n");
}

#[test]
fn folds_zf_ze_zj_zk() {
    let mut e = at("a\nb\nc\nd\ne\nf\n", 0, 0, "zfj");
    assert_eq!(e.buf().folds.len(), 1);
    assert_eq!((e.buf().folds[0].start, e.buf().folds[0].end), (0, 1));
    e.set_cursor(3, 0);
    feed(&mut e, "2zF");
    assert_eq!(e.buf().folds.len(), 2);
    e.set_cursor(2, 0);
    feed(&mut e, "zk");
    assert_eq!(e.cursor().0, 1);
    e.set_cursor(2, 0);
    feed(&mut e, "zj");
    assert_eq!(e.cursor().0, 3);
    feed(&mut e, "zE");
    assert!(e.buf().folds.is_empty());
    let mut e = at("a\nb\nc\nd\n", 0, 0, "zfipzv");
    assert!(!e.buf().folds[0].closed, "zv opens the fold at the cursor");
    feed(&mut e, "zc");
    assert!(e.buf().folds[0].closed);
}

#[test]
fn z_scroll_variants() {
    let t: String = (0..50).map(|i| format!("    l{i}\n")).collect();
    let e = at(&t, 20, 0, "z<CR>");
    assert_eq!((e.buf().top_line, e.cursor()), (20, (20, 4)));
    let e = at(&t, 20, 0, "z.");
    assert_eq!(e.cursor(), (20, 4));
    assert!(e.buf().top_line < 20);
    let e = at(&t, 45, 0, "z-");
    assert_eq!(e.cursor(), (45, 4));
    // Horizontal scrolling needs nowrap.
    let mut e = editor(&"x".repeat(200));
    e.config.wrap = false;
    e.view_width = 20;
    feed(&mut e, "10zl");
    assert_eq!(e.buf().left_col, 10);
    assert_eq!(e.cursor().1, 10, "cursor kept on screen");
    feed(&mut e, "zh");
    assert_eq!(e.buf().left_col, 9);
    feed(&mut e, "50|zs");
    assert_eq!(e.buf().left_col, 49);
    feed(&mut e, "ze");
    assert_eq!(e.buf().left_col, 30);
}

// ----------------------------------------------------------- normal keys

#[test]
fn zz_zq_u_and_q_replay() {
    let mut e = editor("a\n");
    feed(&mut e, "ZQ");
    assert!(e.should_quit);
    let dir = temp();
    let mut e = editor("a\n");
    e.buf_mut().save_as(dir.join("f.txt")).unwrap();
    feed(&mut e, "ixy<Esc>ZZ");
    assert!(e.should_quit);
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "xya\n");
    // U restores the line, then undoes itself.
    let mut e = at("abc\nxyz\n", 0, 0, "xxAq<Esc>");
    assert_eq!(e.buf().line_text(0), "cq");
    feed(&mut e, "U");
    assert_eq!(e.buf().line_text(0), "abc");
    feed(&mut e, "U");
    assert_eq!(e.buf().line_text(0), "cq");
    // Q replays the last recorded macro.
    let mut e = at("1\n2\n3\n4\n", 0, 0, "qaddq");
    feed(&mut e, "Q");
    assert_eq!(text(&e), "3\n4\n");
    feed(&mut e, "2Q");
    assert_eq!(text(&e), "");
}

#[test]
fn replace_mode() {
    let mut e = at("abcdef\n", 0, 1, "RXY<Esc>");
    assert_eq!(text(&e), "aXYdef\n");
    assert_eq!(e.cursor(), (0, 2));
    let mut e2 = at("abc\n", 0, 1, "RXYZW<BS><BS><BS><Esc>");
    assert_eq!(text(&e2), "aXc\n", "Backspace restores overwritten text");
    feed(&mut e2, "u");
    assert_eq!(text(&e2), "abc\n");
    feed(&mut e, "u");
    assert_eq!(text(&e), "abcdef\n");
}

#[test]
fn g_keys_join_resume_changelist_and_info() {
    assert_eq!(text(&at("a\n  b\nc\n", 0, 0, "gJ")), "a  b\nc\n");
    assert_eq!(text(&at("a\nb\nc\n", 0, 0, "3gJ")), "abc\n");
    assert_eq!(text(&at("a\nb\nc\n", 0, 0, "VjgJ")), "ab\nc\n");
    assert_eq!(text(&at("a\nb\nc\n", 0, 0, "VjjJ")), "a b c\n");
    // gi resumes where Insert ended.
    let mut e = at("abc def\n", 0, 3, "iX<Esc>0giY<Esc>");
    assert_eq!(text(&e), "abcXY def\n");
    // g; / g, walk the change list.
    e = at("a\nb\nc\nd\n", 0, 0, "x3Gxgg");
    feed(&mut e, "g;");
    assert_eq!(e.cursor().0, 2);
    feed(&mut e, "g;");
    assert_eq!(e.cursor().0, 0);
    feed(&mut e, "g,");
    assert_eq!(e.cursor().0, 2);
    // g8 / ga / Ctrl-G report on the cursor / file.
    let mut e = at("é\n", 0, 0, "g8");
    assert_eq!(e.message, "c3 a9");
    crate::command::run_ex(&mut e, "ascii");
    assert!(e.message.contains("233"), "{}", e.message);
    feed(&mut e, "<C-g>");
    assert!(e.message.contains("1 line"), "{}", e.message);
}

#[test]
fn ampersand_repeats_substitute() {
    let mut e = at("a a\na a\na a\n", 0, 0, ":s/a/b/g<CR>j");
    feed(&mut e, "&");
    assert_eq!(text(&e), "b b\nb a\na a\n", "& drops the flags");
    feed(&mut e, "j:&&<CR>");
    assert_eq!(text(&e), "b b\nb a\nb b\n");
    let mut e = at("x a\nx a\n", 0, 0, ":s/a/b/<CR>");
    feed(&mut e, "g&");
    assert_eq!(text(&e), "x b\nx b\n");
}

#[test]
fn tag_stack_and_file_info() {
    let mut e = editor("abc\ndef\n");
    e.set_cursor(1, 2);
    feed(&mut e, "<C-]>");
    e.set_cursor(0, 0);
    feed(&mut e, "<C-t>");
    assert_eq!(e.cursor(), (1, 2));
    feed(&mut e, "<C-t>");
    assert!(e.message.starts_with("E73"));
}

#[test]
fn gf_opens_the_file_under_the_cursor() {
    let mut e = editor("");
    let dir = e.project_root.clone();
    std::fs::write(dir.join("target.txt"), "1\n2\n3\n4\n").unwrap();
    e.buf_mut().rope = ropey::Rope::from_str("see target.txt:3 here\n");
    e.set_cursor(0, 5);
    feed(&mut e, "gF");
    assert!(e.buf().path.as_ref().unwrap().ends_with("target.txt"));
    assert_eq!(e.cursor().0, 2);
    assert_eq!(
        crate::normal::file_under_cursor("x (src/a.rs:12) y", 4),
        Some(("src/a.rs".into(), Some(12)))
    );
    let mut e = editor("nothing.here\n");
    feed(&mut e, "gf");
    assert!(e.message.starts_with("E447"), "{}", e.message);
}

#[test]
fn bracket_p_pastes_with_indent() {
    let mut e = at("        x\ny\n", 1, 0, "yyk");
    feed(&mut e, "]p");
    assert_eq!(text(&e), "        x\n        y\ny\n");
    feed(&mut e, "gg[p");
    assert_eq!(text(&e), "        y\n        x\n        y\ny\n");
}

// -------------------------------------------------------------- registers

#[test]
fn read_only_registers() {
    let mut e = at("abc\n", 0, 0, "iZ<Esc>");
    feed(&mut e, "\".p");
    assert_eq!(text(&e), "ZZabc\n");
    let mut e = at("abc\n", 0, 0, ":set nu<CR>\":P");
    assert_eq!(text(&e), "set nuabc\n");
    feed(&mut e, "/bc<CR>\"/P");
    assert!(text(&e).contains("bc"));
    let mut e = editor("x\n");
    e.buf_mut().path = Some(e.project_root.join("dir/name.rs"));
    feed(&mut e, "\"%P");
    assert_eq!(text(&e), "dir/name.rsx\n");
    // @: repeats the last command line.
    let mut e = at("a\nb\nc\nd\n", 0, 0, ":d<CR>");
    feed(&mut e, "@:");
    assert_eq!(text(&e), "c\nd\n");
    feed(&mut e, "@@");
    assert_eq!(text(&e), "d\n");
}

// --------------------------------------------------------------- insert

#[test]
fn insert_mode_deletion_keys() {
    assert_eq!(text(&at("foo bar\n", 0, 7, "a<C-w><Esc>")), "foo \n");
    assert_eq!(text(&at("foo bar  \n", 0, 9, "a<C-w><Esc>")), "foo \n");
    assert_eq!(text(&at("foo.bar\n", 0, 6, "a<C-w><C-w><Esc>")), "foo\n");
    assert_eq!(text(&at("    foo bar\n", 0, 10, "a<C-u><Esc>")), "    \n");
    assert_eq!(text(&at("    foo\n", 0, 4, "i<C-u><Esc>")), "foo\n");
    assert_eq!(text(&at("abc\n", 0, 2, "a<C-h><Esc>")), "ab\n");
    assert_eq!(text(&at("abc\n", 0, 2, "a<C-j>x<Esc>")), "abc\nx\n");
}

#[test]
fn insert_mode_indent_register_literal_digraph() {
    let e = at("foo\n", 0, 0, "A<C-t>!<Esc>");
    assert_eq!(text(&e), "    foo!\n");
    let e = at("        foo\n", 0, 10, "a<C-d><Esc>");
    assert_eq!(text(&e), "    foo\n");
    let e = at("word x\n", 0, 0, "yiwA <C-r>\"<Esc>");
    assert_eq!(text(&e), "word x word\n");
    let e = at(
        "\n",
        0,
        0,
        "i<C-v><Tab>|<C-v>u00e9|<C-v>065|<C-v><C-a><Esc>",
    );
    assert_eq!(text(&e), "\t|é|A|\u{1}\n");
    let e = at("\n", 0, 0, "i<C-k>e'<C-k>a*<C-k>->x<Esc>");
    assert_eq!(text(&e), "éα→x\n");
}

#[test]
fn insert_mode_copy_last_insert_and_ctrl_o() {
    assert_eq!(
        text(&at("abc\nx\n", 1, 0, "i<C-y><C-y><Esc>")),
        "abc\nabx\n"
    );
    assert_eq!(text(&at("x\nabc\n", 0, 0, "i<C-e><Esc>")), "ax\nabc\n");
    let e = at("\n", 0, 0, "ihi<Esc>o<C-a>!<Esc>");
    assert_eq!(text(&e), "hi\nhi!\n");
    let mut e = at("abc def\n", 0, 0, "A!<C-o>0");
    assert_eq!(e.mode, Mode::Insert);
    assert_eq!(e.cursor(), (0, 0));
    feed(&mut e, "><Esc>");
    assert_eq!(text(&e), ">abc def!\n");
    // Ctrl-O with an operator waits for the whole command.
    let e = at("abc def\n", 0, 0, "A<C-o>0<C-o>dwX<Esc>");
    assert_eq!(text(&e), "Xdef\n");
}

#[test]
fn insert_mode_completion_keys() {
    let mut e = at("alpha alphabet\nal\n", 1, 2, "a<C-n>");
    assert!(e.completion.as_ref().is_some_and(|c| !c.items.is_empty()));
    feed(&mut e, "<CR><Esc>");
    assert!(e.buf().line_text(1).starts_with("alpha"));
    let mut e = at("let x = compute();\nlet\n", 1, 2, "a<C-x><C-l>");
    assert_eq!(e.completion.as_ref().map(|c| c.items.len()), Some(1));
    feed(&mut e, "<CR><Esc>");
    assert_eq!(e.buf().line_text(1), "let x = compute();");
    let mut e = editor("");
    std::fs::write(e.project_root.join("hello.txt"), "").unwrap();
    e.buf_mut().path = Some(e.project_root.join("main.txt"));
    e.buf_mut().rope = ropey::Rope::from_str("hel\n");
    e.set_cursor(0, 2);
    feed(&mut e, "a<C-x><C-f><CR><Esc>");
    assert_eq!(e.buf().line_text(0), "hello.txt");
}

#[test]
fn abbreviations() {
    let mut e = editor("\n");
    crate::command::run_ex(&mut e, "iabbrev teh the");
    feed(&mut e, "iteh cat teh.<Esc>");
    assert_eq!(text(&e), "the cat the.\n");
    crate::command::run_ex(&mut e, "unabbreviate teh");
    feed(&mut e, "oteh <Esc>");
    assert_eq!(e.buf().line_text(1), "teh ");
}

// --------------------------------------------------------------- windows

#[test]
fn window_commands() {
    let mut e = editor("a\n");
    feed(&mut e, "<C-w>n");
    assert_eq!(e.windows.len(), 2);
    feed(&mut e, "<C-w>q");
    assert!(e.windows.len() <= 1, "back to a single window");
    // p goes back to the previous window; t / b to the first / last.
    let mut e = editor("a\n");
    crate::command::run_ex(&mut e, "vsplit");
    crate::command::run_ex(&mut e, "split");
    assert_eq!(e.windows.len(), 3);
    feed(&mut e, "<C-w>t");
    assert_eq!(e.active_window, 0);
    feed(&mut e, "<C-w>b");
    assert_eq!(e.active_window, 2);
    feed(&mut e, "<C-w>p");
    assert_eq!(e.active_window, 0);
    // x exchanges buffers with the next pane, the cursor stays put.
    let mut e = editor("first\n");
    let first = e.buf().id;
    crate::command::run_ex(&mut e, "vnew");
    let second = e.buf().id;
    assert_ne!(first, second);
    feed(&mut e, "<C-w>x");
    assert_eq!(e.windows[e.active_window].buffer, first);
    // r rotates; the cursor follows its pane.
    feed(&mut e, "<C-w>r");
    assert_eq!(e.buf().id, first);
    // H / K / J / L restructure the layout around the pane.
    feed(&mut e, "<C-w>K");
    match e.window_layout.as_ref().unwrap() {
        crate::windows::Layout::Split {
            vertical, first, ..
        } => {
            assert!(!vertical);
            assert!(matches!(**first, crate::windows::Layout::Leaf(i) if i == e.active_window));
        }
        l => panic!("{l:?}"),
    }
    feed(&mut e, "<C-w>L");
    assert!(matches!(
        e.window_layout.as_ref().unwrap(),
        crate::windows::Layout::Split { vertical: true, .. }
    ));
    // _ maximizes.
    feed(&mut e, "<C-w>_");
    // T moves the pane to a new tab.
    let tabs = e.tabs.len();
    feed(&mut e, "<C-w>T");
    assert_eq!(e.tabs.len(), tabs + 1);
}

#[test]
fn window_split_with_file_and_alternate() {
    let mut e = editor("");
    std::fs::write(e.project_root.join("other.txt"), "o\n").unwrap();
    e.buf_mut().rope = ropey::Rope::from_str("other.txt\n");
    feed(&mut e, "<C-w>f");
    assert_eq!(e.windows.len(), 2);
    assert!(e.buf().path.as_ref().unwrap().ends_with("other.txt"));
    feed(&mut e, "<C-w>^");
    assert_eq!(e.windows.len(), 3);
    // A missing file leaves the layout alone.
    let mut e = editor("missing.txt\n");
    feed(&mut e, "<C-w>f");
    assert!(e.windows.len() <= 1);
    assert!(e.message.starts_with("E447"));
}

// -------------------------------------------------------------------- Ex

fn ex(e: &mut Editor, cmd: &str) {
    crate::command::run_ex(e, cmd);
}

#[test]
fn ex_split_new_enew_tabe_and_alternate() {
    let mut e = editor("a\n");
    ex(&mut e, "sp");
    assert_eq!(e.windows.len(), 2);
    ex(&mut e, "vs");
    assert_eq!(e.windows.len(), 3);
    ex(&mut e, "new");
    assert_eq!(e.windows.len(), 4);
    assert_eq!(text(&e), "\n");
    let before = e.buffers.len();
    ex(&mut e, "enew");
    assert_eq!(e.buffers.len(), before + 1);
    ex(&mut e, "e#");
    let tabs = e.tabs.len();
    let dir = e.project_root.clone();
    std::fs::write(dir.join("t.txt"), "t\n").unwrap();
    ex(&mut e, &format!("tabe {}", dir.join("t.txt").display()));
    assert_eq!(e.tabs.len(), tabs + 1);
    assert_eq!(text(&e), "t\n");
}

#[test]
fn ex_write_variants() {
    let mut e = editor("one\ntwo\n");
    let dir = e.project_root.clone();
    let path = dir.join("w.txt");
    e.buf_mut().path = Some(path.clone());
    ex(&mut e, "w !cat");
    assert!(!path.exists(), ":w !cmd writes nothing to disk");
    assert!(!dir.join("!cat").exists());
    assert_eq!(
        e.results.as_ref().map(|r| r.entries.len()),
        Some(2),
        "cat's output is shown"
    );
    e.results = None;
    e.enter_normal();
    ex(&mut e, "w ++enc=utf-8");
    assert!(e.message.starts_with("E474"));
    assert!(!dir.join("++enc=utf-8").exists());
    std::fs::write(&path, "one\ntwo\n").unwrap();
    e.buf_mut().reload().unwrap();
    std::fs::write(&path, "stale\n").unwrap();
    ex(&mut e, "up");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "stale\n",
        ":update skips an unmodified buffer"
    );
    std::fs::write(&path, "one\ntwo\n").unwrap();
    e.buf_mut().reload().unwrap();
    feed(&mut e, "x");
    ex(&mut e, "update");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ne\ntwo\n");
    ex(&mut e, "w >> ");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "ne\ntwo\nne\ntwo\n"
    );
    ex(&mut e, &format!("saveas {}", dir.join("w.txt").display()));
    assert!(e.message.starts_with("E13"));
    ex(
        &mut e,
        &format!("saveas {}", dir.join("copy.txt").display()),
    );
    assert!(dir.join("copy.txt").exists());
    assert!(e.buf().path.as_ref().unwrap().ends_with("copy.txt"));
    // :wa writes every modified named buffer.
    feed(&mut e, "x");
    ex(&mut e, "wa");
    assert_eq!(
        std::fs::read_to_string(dir.join("copy.txt")).unwrap(),
        "e\ntwo\n"
    );
}

#[test]
fn ex_read_undo_redo_retab_center() {
    let mut e = at("a\nb\n", 0, 0, "");
    ex(&mut e, "r !printf 'x\\ny\\n'");
    assert_eq!(text(&e), "a\nx\ny\nb\n");
    ex(&mut e, "undo");
    assert_eq!(text(&e), "a\nb\n");
    ex(&mut e, "redo");
    assert_eq!(text(&e), "a\nx\ny\nb\n");
    let mut e = editor("\tx\n  \ty\n");
    ex(&mut e, "retab");
    assert_eq!(text(&e), "    x\n    y\n");
    let mut e = editor("        x\n");
    e.buf_mut().expandtab = false;
    ex(&mut e, "retab!");
    assert_eq!(text(&e), "\t\tx\n");
    let mut e = editor("ab\n");
    ex(&mut e, "center 10");
    assert_eq!(text(&e), "    ab\n");
    ex(&mut e, "right 10");
    assert_eq!(text(&e), "        ab\n");
    ex(&mut e, "left 2");
    assert_eq!(text(&e), "  ab\n");
}

#[test]
fn ex_marks_changes_and_lines() {
    let mut e = editor("a\nb\nc\n");
    ex(&mut e, "2mark x");
    assert_eq!(e.marks.get(&'x').map(|m| m.line), Some(1));
    ex(&mut e, "3ky");
    assert_eq!(e.marks.get(&'y').map(|m| m.line), Some(2));
    ex(&mut e, "delmarks x-y");
    assert!(!e.marks.contains_key(&'x') && !e.marks.contains_key(&'y'));
    feed(&mut e, "xjx");
    ex(&mut e, "changes");
    assert_eq!(e.results.as_ref().map(|r| r.entries.len()), Some(2));
    let mut e = editor("a\nb\nc\n");
    ex(&mut e, "=");
    assert_eq!(e.message, "3");
    ex(&mut e, "2p");
    assert_eq!(e.message, "b");
    ex(&mut e, "1,2nu");
    assert_eq!(e.results.as_ref().unwrap().entries[0].text, "   1 a");
    ex(&mut e, "goto 3");
    assert_eq!(e.cursor(), (1, 0));
}

#[test]
fn ex_bufdo_windo_and_modifiers() {
    let mut e = editor("x\n");
    ex(&mut e, "enew");
    feed(&mut e, "iy<Esc>");
    ex(&mut e, "bufdo s/^/-/");
    assert_eq!(e.buffers[0].line_text(0), "-x");
    assert_eq!(e.buffers[1].line_text(0), "-y");
    let mut e = editor("a\n");
    ex(&mut e, "vsplit");
    ex(&mut e, "windo s/a/b/");
    assert_eq!(text(&e), "b\n");
    ex(&mut e, "silent echo \"hi\"");
    assert!(e.message.is_empty());
    ex(&mut e, "echo \"a\" . \"b\"");
    assert_eq!(e.message, "ab");
    ex(&mut e, "exe \"normal! x\"");
    assert_eq!(text(&e), "\n");
    ex(&mut e, "keepjumps normal! ix");
    assert_eq!(text(&e), "x\n");
}

#[test]
fn ex_source_map_and_unmap() {
    let mut e = editor("abc\n");
    let file = e.project_root.join("rc.vim");
    std::fs::write(&file, "\" a comment\nnnoremap Y y$\n:set nonumber\n").unwrap();
    ex(&mut e, &format!("source {}", file.display()));
    assert!(e
        .keymaps
        .iter()
        .any(|k| k.lhs == vec![crate::key::Key::Char('Y')]));
    assert!(!e.config.number);
    ex(&mut e, "nunmap Y");
    assert!(!e
        .keymaps
        .iter()
        .any(|k| k.lhs == vec![crate::key::Key::Char('Y')]));
    ex(&mut e, "nnoremap Q dd");
    feed(&mut e, "Q");
    assert_eq!(text(&e), "");
}

#[test]
fn ex_vimgrep_cbuffer_and_cnfile() {
    let mut e = editor("one\nfoo two\nthree foo\n");
    e.buf_mut().path = Some(e.project_root.join("a.txt"));
    std::fs::write(e.project_root.join("a.txt"), text(&e)).unwrap();
    ex(&mut e, "vimgrep /foo/ %");
    assert_eq!(e.quickfix.as_ref().map(|q| q.entries.len()), Some(2));
    assert_eq!(e.cursor(), (1, 0));
    let dir = e.project_root.clone();
    std::fs::write(dir.join("b.rs"), "x\n").unwrap();
    std::fs::write(dir.join("c.rs"), "y\n").unwrap();
    let mut e = editor(&format!(
        "{0}/b.rs:1:1: error one\n{0}/c.rs:1:1: error two\n",
        dir.display()
    ));
    e.project_root = dir.clone();
    ex(&mut e, "cbuffer");
    assert_eq!(e.quickfix.as_ref().map(|q| q.entries.len()), Some(2));
    assert!(e.buf().path.as_ref().unwrap().ends_with("b.rs"));
    ex(&mut e, "cnfile");
    assert!(e.buf().path.as_ref().unwrap().ends_with("c.rs"));
    ex(&mut e, "cpfile");
    assert!(e.buf().path.as_ref().unwrap().ends_with("b.rs"));
}

#[test]
fn ex_args_list_and_cq() {
    let mut e = editor("");
    let dir = e.project_root.clone();
    for n in ["1.txt", "2.txt"] {
        std::fs::write(dir.join(n), n).unwrap();
    }
    ex(
        &mut e,
        &format!(
            "args {} {}",
            dir.join("1.txt").display(),
            dir.join("2.txt").display()
        ),
    );
    assert_eq!(text(&e), "1.txt");
    ex(&mut e, "next");
    assert_eq!(text(&e), "2.txt");
    ex(&mut e, "next");
    assert!(e.message.starts_with("E165"));
    ex(&mut e, "prev");
    assert_eq!(text(&e), "1.txt");
    ex(&mut e, "last");
    assert_eq!(text(&e), "2.txt");
    ex(&mut e, "cq 3");
    assert!(e.should_quit);
    assert_eq!(e.exit_code, Some(3));
}

#[test]
fn ex_checktime_reloads_clean_buffers() {
    let mut e = editor("");
    let path = e.project_root.join("c.txt");
    std::fs::write(&path, "old\n").unwrap();
    e.open_file(path.clone()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "new\n").unwrap();
    ex(&mut e, "checktime");
    assert_eq!(text(&e), "new\n");
}

#[test]
fn visual_replace_and_star() {
    assert_eq!(text(&at("abcd\n", 0, 1, "vlrx")), "axxd\n");
    assert_eq!(text(&at("ab\ncd\n", 0, 0, "Vjr-")), "--\n--\n");
    assert_eq!(text(&at("abc\nabc\n", 0, 1, "<C-v>jr*")), "a*c\na*c\n");
    let e = at("a.b x a.b\n", 0, 0, "vll*");
    assert_eq!(e.cursor(), (0, 6), "the selection is searched literally");
}
