use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::buffer::Buffer;
use crate::config::Config;
use crate::key::Key;
use crate::mode::{CommandKind, Mode, VisualKind};
use crate::normal::PendingState;
use crate::registers::Registers;

struct SearchCache {
    buffer: u64,
    revision: u64,
    pattern: String,
    ignorecase: bool,
    smartcase: bool,
    matches: Vec<usize>,
}

pub struct Editor {
    pub project_root: PathBuf,
    pub review_job: Option<crate::review::ReviewJob>,
    pub review_results: Option<crate::results::Results>,
    pub recovery: crate::recovery::Recovery,
    pub layout_cache: std::cell::RefCell<crate::render::LayoutCache>,
    pub preview_panes: std::cell::RefCell<HashMap<u64, crate::markdown::Preview>>,
    pub snippet: Option<crate::snippet::Session>,
    pub word_index: Option<crate::completion::WordIndex>,
    pub recent_files: Vec<PathBuf>,
    pub insert_repeat: usize,
    pub insert_start: usize,
    pub block_insert: Option<(usize, usize, usize)>,
    pub visual_repeat: Option<(VisualKind, usize, usize, crate::operator::OperatorKind)>,
    pub notes: crate::notes::Notes,
    pub results: Option<crate::results::Results>,
    pub quickfix: Option<crate::results::Results>,
    pub marks: HashMap<char, crate::navigation::Location>,
    pub jumps: Vec<crate::navigation::Location>,
    pub jump_index: usize,
    pub search_job: crate::jobs::SearchJob,
    pub windows: Vec<crate::windows::Window>,
    pub active_window: usize,
    pub split_vertical: bool,
    pub window_layout: Option<crate::windows::Layout>,
    pub screen_cols: usize,
    pub window_prefix: bool,
    pub pending_language: HashMap<u64, crate::language::RequestContext>,

    pub buffers: Vec<Buffer>,
    pub cur: usize,
    pub mode: Mode,
    pub config: Config,
    pub registers: Registers,
    pub message: String,
    pub should_quit: bool,

    pub pending: PendingState,
    pub visual_anchor: Option<(usize, usize)>,
    pub cmdline: String,

    pub last_search: Option<(String, bool)>,
    search_cache: Option<SearchCache>,
    pub last_find: Option<(char, bool, bool)>,

    pub macro_recording: Option<(char, Vec<Key>)>,
    pub last_macro_reg: Option<char>,
    pub macros: HashMap<char, Vec<Key>>,

    pub recording_change: bool,
    pub cmd_keys: Vec<Key>,
    pub last_change: Vec<Key>,
    pub replaying: bool,
    pub replay_depth: usize,
    pub replay_budget: usize,
    pub lsp_stamp: Option<(u64, u64, Option<PathBuf>)>,

    pub pending_jk: Option<Instant>,
    pub screen_rows: usize,
    pub hl_search: bool,

    pub file_picker: Option<crate::picker::FilePicker>,
    pub all_files: Vec<String>,

    pub syntax: Option<crate::syntax::Syntax>,
    pub syntax_stamp: u64,
    syntax_seq: Option<(u64, u64)>,
    syntax_pending: Option<((u64, u64), Instant, bool)>,

    pub completion: Option<crate::completion::CompletionState>,
    pub(crate) next_request_id: u64,

    pub git: Option<crate::gitdiff::GitGutter>,
    pub git_job: crate::gitdiff::GitJob,
    pub git_task: Option<crate::git_tools::GitTask>,

    pub lsp_clients: HashMap<String, crate::lsp::LspClient>,
    pub(crate) lsp_unavailable: HashSet<String>,
    pub(crate) lsp_opened_docs: HashSet<(String, PathBuf)>,
    pub(crate) lsp_synced_seq: HashMap<(String, PathBuf), u64>,
    pub diagnostics: HashMap<PathBuf, Vec<crate::lsp::Diagnostic>>,
    pub server_diagnostics: HashMap<(String, PathBuf), Vec<crate::lsp::Diagnostic>>,
    pub hover_text: Option<String>,

    pub markdown_preview: Option<crate::markdown::Preview>,

    /// Shared cache for the current buffer's full text, keyed by (buffer
    /// index, edit_seq). Syntax highlighting, the git gutter, LSP sync, and
    /// the markdown preview each used to call `rope.to_string()`
    /// independently -- an O(n) rope traversal each -- even though on any
    /// given frame they're all reacting to the *same* edit. `buffer_text()`
    /// makes the first caller on a frame pay for the traversal and everyone
    /// else get a cheap Rc clone instead of repeating it.
    text_cache: Option<(u64, u64, std::rc::Rc<str>)>,
}

impl Editor {
    pub fn new(config: Config) -> Editor {
        let registers = Registers::new(config.clipboard_unnamedplus);
        let project_root = crate::files::identity(&std::env::current_dir().unwrap_or_default());
        let notes = crate::notes::Notes::load(&project_root);
        Editor {
            project_root,
            review_job: None,
            review_results: None,
            notes,
            recovery: Default::default(),
            layout_cache: Default::default(),
            preview_panes: Default::default(),
            snippet: None,
            word_index: None,
            recent_files: Vec::new(),
            insert_repeat: 1,
            insert_start: 0,
            block_insert: None,
            visual_repeat: None,
            results: None,
            quickfix: None,
            marks: HashMap::new(),
            jumps: Vec::new(),
            jump_index: 0,
            search_job: Default::default(),
            windows: Vec::new(),
            active_window: 0,
            split_vertical: true,
            window_layout: None,
            screen_cols: 80,
            window_prefix: false,
            pending_language: HashMap::new(),
            buffers: vec![Buffer::empty()],
            cur: 0,
            mode: Mode::Normal,
            config,
            registers,
            message: String::from("vaayu — :help for keys · :w to save · :q to quit"),
            should_quit: false,
            pending: PendingState::default(),
            visual_anchor: None,
            cmdline: String::new(),
            last_search: None,
            search_cache: None,
            last_find: None,
            macro_recording: None,
            last_macro_reg: None,
            macros: HashMap::new(),
            recording_change: false,
            cmd_keys: Vec::new(),
            last_change: Vec::new(),
            replaying: false,
            replay_depth: 0,
            replay_budget: 10000,
            lsp_stamp: None,
            pending_jk: None,
            screen_rows: 24,
            hl_search: true,
            file_picker: None,
            all_files: Vec::new(),
            syntax: None,
            syntax_stamp: 0,
            syntax_seq: None,
            syntax_pending: None,
            completion: None,
            next_request_id: 0,
            git: None,
            git_job: Default::default(),
            git_task: None,
            lsp_clients: HashMap::new(),
            lsp_unavailable: HashSet::new(),
            lsp_opened_docs: HashSet::new(),
            lsp_synced_seq: HashMap::new(),
            diagnostics: HashMap::new(),
            server_diagnostics: HashMap::new(),
            hover_text: None,
            markdown_preview: None,
            text_cache: None,
        }
    }

    /// Caches keyed by buffer *index* (text_cache, syntax_seq) go stale in a
    /// way content alone can't catch: removing a buffer shifts every later
    /// one down an index, so a fresh, unedited buffer landing on index N can
    /// have the same (index, edit_seq) key a just-removed buffer left
    /// behind, and silently inherit its cached text/syntax tree. Call this
    /// whenever the buffer list is structurally changed (not just switched).
    pub fn invalidate_index_caches(&mut self) {
        self.lsp_stamp = None;
        self.text_cache = None;
        self.syntax_seq = None;
        self.syntax_pending = None;
    }

    /// The current buffer's full text, materialized at most once per edit
    /// (see the `text_cache` field docs). Cheap (an Rc clone) for every
    /// caller after the first on a given frame.
    pub(crate) fn buffer_text(&mut self) -> std::rc::Rc<str> {
        let key = (self.buf().id, self.buf().edit_seq);
        if let Some((idx, seq, text)) = &self.text_cache {
            if (*idx, *seq) == key {
                return text.clone();
            }
        }
        let text: std::rc::Rc<str> = self.buffers[self.cur].rope.to_string().into();
        self.text_cache = Some((key.0, key.1, text.clone()));
        text
    }

    pub fn is_markdown_buffer(&self) -> bool {
        self.buf()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
    }

    pub fn toggle_markdown_preview(&mut self) {
        if self.markdown_preview.is_some() {
            self.markdown_preview = None;
            self.enter_normal();
            return;
        }
        if !self.is_markdown_buffer() {
            self.set_message("markdown preview is only available for .md files");
            return;
        }
        self.markdown_preview = Some(crate::markdown::Preview::new());
        self.mode = Mode::MarkdownPreview;
    }

    /// Re-renders the open preview if the buffer changed since the last
    /// render. Cheap no-op otherwise -- call once per frame.
    pub fn ensure_markdown_preview(&mut self, viewport_cols: usize) {
        let seq = self.buf().edit_seq;
        let width = viewport_cols.saturating_sub(2).max(10);
        if self
            .markdown_preview
            .as_ref()
            .is_some_and(|p| p.needs_refresh(seq, width))
        {
            let text = self.buffer_text();
            self.markdown_preview
                .as_mut()
                .unwrap()
                .refresh(&text, seq, width);
        }
    }

    /// (Re)opens the git connection if the current buffer's path changed,
    /// and re-diffs if the buffer was edited since the last check. Cheap
    /// no-op otherwise -- call once per frame.
    pub fn ensure_git(&mut self) {
        self.poll_git();
        self.update_git_background();
    }

    /// (Re)opens the completion popup at the word ending at the cursor, or
    /// closes it if the cursor is no longer inside/after a word.
    pub fn update_completion(&mut self) {
        let (line, col) = self.cursor();
        let (start_col, prefix) = crate::completion::word_prefix(self.buf(), line, col);
        if prefix.is_empty() {
            self.completion = None;
            return;
        }
        if self.word_index.is_none() {
            self.word_index = Some(crate::completion::WordIndex::new(self.buf()));
        }
        let items = if let Some(index) = &mut self.word_index {
            index.candidates(&self.buffers[self.cur], &prefix, line)
        } else {
            crate::completion::buffer_word_candidates(self.buf(), &prefix, line)
        };
        let has_lsp = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .and_then(|e| crate::lsp::lang_id_for_extension(&e.to_lowercase()))
            .is_some_and(|_| !self.clients_for_current().is_empty());
        if items.is_empty() && !has_lsp {
            self.completion = None;
            return;
        }
        self.next_request_id += 1;
        let request_id = self.next_request_id;
        self.completion = Some(crate::completion::CompletionState {
            start: (line, start_col),
            items,
            selected: 0,
            request_id,
        });
        if has_lsp {
            self.request_lsp_completion(line, col, request_id);
        }
    }

    pub fn close_completion(&mut self) {
        self.completion = None;
    }

    pub fn find_search(
        &mut self,
        pattern: &str,
        from: usize,
        forward: bool,
    ) -> Result<Option<usize>, String> {
        let key = (self.buf().id, self.buf().edit_seq);
        let ignorecase = self.config.ignorecase;
        let smartcase = self.config.smartcase;
        let stale = self.search_cache.as_ref().is_none_or(|cache| {
            cache.buffer != key.0
                || cache.revision != key.1
                || cache.pattern != pattern
                || cache.ignorecase != ignorecase
                || cache.smartcase != smartcase
        });
        if stale {
            let matches = crate::search::positions(self.buf(), pattern, ignorecase, smartcase)?;
            self.search_cache = Some(SearchCache {
                buffer: key.0,
                revision: key.1,
                pattern: pattern.into(),
                ignorecase,
                smartcase,
                matches,
            });
        }
        Ok(crate::search::find_position(
            &self.search_cache.as_ref().unwrap().matches,
            from,
            forward,
        ))
    }

    /// (Re)creates the parser if the current buffer's filetype changed, and
    /// reparses if the buffer was edited since the last parse. Cheap no-op
    /// otherwise -- call once per frame before drawing.
    pub fn ensure_syntax(&mut self) {
        let want_lang = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .and_then(|e| crate::syntax::lang_for_extension(&e.to_lowercase()));

        let have_lang = self.syntax.as_ref().map(|s| s.lang());
        if have_lang != want_lang {
            self.syntax = want_lang.and_then(crate::syntax::Syntax::new);
            self.syntax_seq = None;
            self.syntax_pending = None;
        }

        let key = (self.buf().id, self.buf().edit_seq);
        if self.syntax.is_some() && self.syntax_seq != Some(key) {
            // Materializing and incrementally parsing a multi-megabyte rope can
            // still take longer than one input frame. While the user is
            // actively typing, draw the text immediately with the previous
            // highlight spans and catch syntax up after a short idle window.
            // Initial parsing, buffer switches, and normal-mode edits remain
            // synchronous so navigation never opens on an unparsed buffer.
            const LARGE_BUFFER: usize = 256 * 1024;
            const INSERT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);
            let same_buffer = self.syntax_seq.is_some_and(|(id, _)| id == key.0);
            let pending_current = self
                .syntax_pending
                .is_some_and(|(pending_key, _, _)| pending_key == key);
            if (self.mode == Mode::Insert || pending_current)
                && same_buffer
                && self.buf().rope.len_bytes() >= LARGE_BUFFER
            {
                // Leaving Insert must paint the mode change before paying for
                // the deferred parse. Give that first Normal frame a fresh
                // idle window, then let the idle loop perform the catch-up.
                if self.mode != Mode::Insert {
                    if let Some((_, since, defer_exit_frame)) = &mut self.syntax_pending {
                        if *defer_exit_frame {
                            *since = Instant::now();
                            *defer_exit_frame = false;
                            return;
                        }
                    }
                }
                match self.syntax_pending {
                    Some((pending_key, since, _)) if pending_key == key => {
                        if since.elapsed() < INSERT_DEBOUNCE {
                            return;
                        }
                    }
                    _ => {
                        self.syntax_pending = Some((key, Instant::now(), true));
                        return;
                    }
                }
            }
            let text = self.buffer_text();
            self.syntax.as_mut().unwrap().reparse(text);
            self.syntax_stamp += 1;
            self.syntax_seq = Some(key);
            self.syntax_pending = None;
        } else if let Some(syn) = &mut self.syntax {
            // No new edit this frame, but a prior reparse may have deferred
            // an expensive full rebuild (see Syntax::full_rebuild_pending's
            // docs -- this only happens for the rare invalid-syntax-churn
            // case, not on every edit). Finish it once its throttle window
            // passes so highlighting doesn't stay stale indefinitely.
            if syn.catch_up() {
                self.syntax_stamp += 1;
            }
        }
    }

    /// Whether a syntax full-rebuild fallback is waiting on its throttle
    /// window to elapse. `ensure_syntax` only runs from the main loop's
    /// "something happened" path; the idle wait polls this so a rebuild
    /// deferred right as the user stops typing still gets finished (and the
    /// result redrawn) rather than sitting stale until the next keystroke.
    pub fn syntax_catch_up_due(&self) -> bool {
        const INSERT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);
        self.syntax_pending
            .is_some_and(|(_, since, _)| since.elapsed() >= INSERT_DEBOUNCE)
            || self.syntax.as_ref().is_some_and(|s| s.rebuild_due())
    }

    /// Syntax catch-up is idle work. Any user event restarts its quiet window
    /// so a deferred large-buffer parse cannot begin between two keystrokes
    /// and block the next one for tens of milliseconds.
    pub fn note_input_activity(&mut self) {
        if let Some((_, since, _)) = &mut self.syntax_pending {
            *since = Instant::now();
        }
    }

    pub fn open_picker(&mut self) {
        self.start_file_scan();
        self.file_picker = Some(crate::picker::FilePicker::new(&self.all_files));
        self.mode = Mode::Picker;
    }

    pub fn start_file_scan(&mut self) {
        if !self.search_job.files_ready && self.search_job.files_rx.is_none() {
            let root = self.project_root.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.search_job.files_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(crate::picker::scan_files(&root));
            });
        }
    }

    pub fn open_file(&mut self, path: PathBuf) -> anyhow::Result<()> {
        // Focus an already-open buffer for this file instead of loading a
        // second, independent copy of it -- without this, :e (and LSP
        // goto-definition) on an already-open file reads disk into a
        // competing buffer, and whichever one saves last silently wins.
        let path = crate::files::identity(&path);
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path.clone());
        self.recent_files.truncate(100);
        let target_abs = Some(path.clone());
        if let Some(target_abs) = &target_abs {
            if let Some(idx) = self
                .buffers
                .iter()
                .position(|b| b.path.as_ref() == Some(target_abs))
            {
                self.cur = idx;
                return Ok(());
            }
        }

        let buf = Buffer::from_path(path)?;
        if self.buffers.len() == 1
            && self.buffers[0].path.is_none()
            && !self.buffers[0].is_modified()
        {
            self.buffers[0] = buf;
        } else {
            self.buffers.push(buf);
            self.cur = self.buffers.len() - 1;
        }
        // A buffer at an existing index can be swapped out for one with the
        // same (index, edit_seq==0) key as the buffer it replaced -- see
        // invalidate_index_caches' docs.
        self.invalidate_index_caches();
        Ok(())
    }

    pub fn buf(&self) -> &Buffer {
        &self.buffers[self.cur]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.cur]
    }

    pub fn buf_and_registers_mut(&mut self) -> (&mut Buffer, &mut Registers) {
        (&mut self.buffers[self.cur], &mut self.registers)
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.buf().cursor_line, self.buf().cursor_col)
    }

    pub fn set_cursor(&mut self, line: usize, col: usize) {
        let b = self.buf_mut();
        let line = line.min(b.line_count().saturating_sub(1));
        let col = b.clamp_col_normal(line, col);
        b.cursor_line = line;
        b.cursor_col = col;
        b.desired_col = col;
    }

    pub fn set_cursor_insert(&mut self, line: usize, col: usize) {
        let b = self.buf_mut();
        let line = line.min(b.rope.len_lines().saturating_sub(1));
        let col = b.clamp_col_insert(line, col);
        b.cursor_line = line;
        b.cursor_col = col;
    }

    /// Single entry point for every key: main loop and macro/dot replay funnel through here.
    pub fn feed_key(&mut self, key: Key) {
        if !self.windows.is_empty() && self.windows[self.active_window].preview {
            let w = &mut self.windows[self.active_window];
            match key {
                Key::Char('j') | Key::Down => w.preview_scroll += 1,
                Key::Char('k') | Key::Up => w.preview_scroll = w.preview_scroll.saturating_sub(1),
                Key::Char('g') => w.preview_scroll = 0,
                Key::Char('q') => {
                    self.close_window();
                    return;
                }
                _ => {}
            }
            if key != Key::Ctrl('w') && !self.window_prefix {
                return;
            }
        }

        if key == Key::Ctrl('q') {
            self.export_quickfix();
            return;
        }
        if key == Key::Ctrl('s') {
            self.flush_pending_jk();
            let result = if self.mode == Mode::Results {
                self.save_notes()
            } else {
                self.save_current()
            };
            self.set_message(match result {
                Ok(()) => "Saved".into(),
                Err(e) => format!("Save failed: {e}"),
            });
            return;
        }
        if self.window_prefix {
            self.window_prefix = false;
            self.window_key(key);
            return;
        }
        if key == Key::Ctrl('w') && self.mode == Mode::Normal {
            self.window_prefix = true;
            return;
        }

        // Macro recording: a bare 'q' in Normal mode with nothing pending stops recording
        // instead of being processed as a command.
        if !self.replaying
            && self.macro_recording.is_some()
            && matches!(self.mode, Mode::Normal)
            && self.pending.is_empty()
            && key == Key::Char('q')
        {
            if let Some((reg, keys)) = self.macro_recording.take() {
                self.macros.insert(reg, keys);
                self.message = format!("recorded @{}", reg);
            }
            return;
        }
        if !self.replaying {
            if let Some((_, keys)) = &mut self.macro_recording {
                keys.push(key);
            }
        }

        // Dot-repeat recording: capture the raw keys of the in-flight change command.
        if self.recording_change {
            self.cmd_keys.push(key);
        }

        match self.mode {
            Mode::Results => crate::results::handle(self, key),
            Mode::Normal => crate::normal::handle(self, key),
            Mode::Insert => crate::insert::handle(self, key),
            Mode::Visual(_) => crate::visual::handle(self, key),
            Mode::Command(_) => crate::command::handle(self, key),
            Mode::Picker => crate::picker::handle(self, key),
            Mode::MarkdownPreview => crate::preview::handle(self, key),
        }
    }

    pub fn start_change_recording(&mut self, first_key: Key) {
        if !self.replaying {
            self.recording_change = true;
            if first_key != Key::Char('v') {
                self.visual_repeat = None;
            }
            self.cmd_keys = Vec::new();
            if let Some(r) = self.pending.register {
                self.cmd_keys.extend([Key::Char('"'), Key::Char(r)]);
            }
            let n = self.pending.total_count();
            if n > 1 {
                self.cmd_keys.extend(n.to_string().chars().map(Key::Char));
            }
            self.cmd_keys.push(first_key);
        }
    }

    pub fn finish_change_recording(&mut self) {
        if self.recording_change && !self.replaying {
            self.last_change = self.cmd_keys.clone();
        }
        self.recording_change = false;
    }

    pub fn abort_change_recording(&mut self) {
        self.recording_change = false;
        self.cmd_keys.clear();
    }

    pub fn replay(&mut self, keys: &[Key]) {
        if self.replay_depth == 0 {
            self.replay_budget = 10000;
        }
        if self.replay_budget == 0 {
            return;
        }
        if self.replay_depth >= 32 {
            self.set_message("macro recursion limit reached");
            return;
        }
        self.replay_depth += 1;
        let was_replaying = self.replaying;
        self.replaying = true;
        for k in keys.iter().copied() {
            if self.replay_budget == 0 {
                self.set_message("Macro work limit reached");
                break;
            }
            self.replay_budget -= 1;
            self.feed_key(k);
        }
        self.replaying = was_replaying;
        self.replay_depth -= 1;
    }

    pub fn enter_insert(&mut self) {
        self.word_index = None;
        self.mode = Mode::Insert;
    }

    pub fn enter_normal(&mut self) {
        self.mode = Mode::Normal;
        let (l, c) = self.cursor();
        let nc = self.buf().clamp_col_normal(l, c);
        self.buf_mut().cursor_col = nc;
    }

    pub fn insert_paste(&mut self, text: &str) {
        self.flush_pending_jk();
        self.close_completion();
        self.buf_mut().begin_edit();
        let (l, c) = self.cursor();
        let start = self.buf().char_idx(l, c);
        self.buf_mut().insert_str_at(start, text);
        let (l, c) = self.buf().pos_from_char_idx(start + text.chars().count());
        self.set_cursor_insert(l, c);
        self.buf_mut().commit_edit();
        if !matches!(self.mode, Mode::Insert) {
            self.enter_normal();
        } else {
            self.buf_mut().begin_edit();
        }
    }
    pub fn enter_visual(&mut self, kind: VisualKind) {
        self.visual_anchor = Some(self.cursor());
        self.mode = Mode::Visual(kind);
    }

    pub fn enter_command(&mut self, kind: CommandKind) {
        self.cmdline.clear();
        self.mode = Mode::Command(kind);
    }

    pub fn set_message<S: Into<String>>(&mut self, msg: S) {
        self.message = msg.into();
    }

    /// Called by the main loop when the jk-escape timeout elapses with no
    /// follow-up key: the buffered 'j' becomes a literal character.
    pub fn flush_pending_jk(&mut self) {
        if self.pending_jk.take().is_some() && matches!(self.mode, Mode::Insert) {
            if self.cmd_keys.last() == Some(&Key::Char('j')) {
                *self.cmd_keys.last_mut().unwrap() = Key::Literal('j');
            }
            if let Some((_, keys)) = &mut self.macro_recording {
                if keys.last() == Some(&Key::Char('j')) {
                    *keys.last_mut().unwrap() = Key::Literal('j');
                }
            }
            let (line, col) = self.cursor();
            self.buf_mut().insert_char(line, col, 'j');
            self.set_cursor_insert(line, col + 1);
        }
    }
}
