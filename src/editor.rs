use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::buffer::Buffer;
use crate::config::Config;
use crate::key::Key;
use crate::mode::{CommandKind, Mode, VisualKind};
use crate::normal::PendingState;
use crate::registers::Registers;

pub struct Editor {
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
    pub last_find: Option<(char, bool, bool)>,

    pub macro_recording: Option<(char, Vec<Key>)>,
    pub last_macro_reg: Option<char>,
    pub macros: HashMap<char, Vec<Key>>,

    pub recording_change: bool,
    pub cmd_keys: Vec<Key>,
    pub last_change: Vec<Key>,
    pub replaying: bool,

    pub pending_jk: Option<Instant>,
    pub screen_rows: usize,
    pub hl_search: bool,

    pub file_picker: Option<crate::picker::FilePicker>,
    pub all_files: Vec<String>,

    pub syntax: Option<crate::syntax::Syntax>,
    syntax_seq: Option<(usize, u64)>,

    pub completion: Option<crate::completion::CompletionState>,
    next_request_id: u64,

    pub git: Option<crate::gitdiff::GitGutter>,
    git_path: Option<PathBuf>,

    pub lsp_clients: HashMap<&'static str, crate::lsp::LspClient>,
    lsp_unavailable: HashSet<&'static str>,
    lsp_opened_docs: HashSet<PathBuf>,
    lsp_synced_seq: HashMap<PathBuf, u64>,
    pub diagnostics: HashMap<PathBuf, Vec<crate::lsp::Diagnostic>>,
    pub hover_text: Option<String>,
    pending_hover_id: u64,
    pending_definition_id: u64,

    pub markdown_preview: Option<crate::markdown::Preview>,

    /// Shared cache for the current buffer's full text, keyed by (buffer
    /// index, edit_seq). Syntax highlighting, the git gutter, LSP sync, and
    /// the markdown preview each used to call `rope.to_string()`
    /// independently -- an O(n) rope traversal each -- even though on any
    /// given frame they're all reacting to the *same* edit. `buffer_text()`
    /// makes the first caller on a frame pay for the traversal and everyone
    /// else get a cheap Rc clone instead of repeating it.
    text_cache: Option<(usize, u64, std::rc::Rc<str>)>,
}

impl Editor {
    pub fn new(config: Config) -> Editor {
        let registers = Registers::new(config.clipboard_unnamedplus);
        Editor {
            buffers: vec![Buffer::empty()],
            cur: 0,
            mode: Mode::Normal,
            config,
            registers,
            message: String::from("vaayu -- type :help-less, :w to save, :q to quit"),
            should_quit: false,
            pending: PendingState::default(),
            visual_anchor: None,
            cmdline: String::new(),
            last_search: None,
            last_find: None,
            macro_recording: None,
            last_macro_reg: None,
            macros: HashMap::new(),
            recording_change: false,
            cmd_keys: Vec::new(),
            last_change: Vec::new(),
            replaying: false,
            pending_jk: None,
            screen_rows: 24,
            hl_search: true,
            file_picker: None,
            all_files: Vec::new(),
            syntax: None,
            syntax_seq: None,
            completion: None,
            next_request_id: 0,
            git: None,
            git_path: None,
            lsp_clients: HashMap::new(),
            lsp_unavailable: HashSet::new(),
            lsp_opened_docs: HashSet::new(),
            lsp_synced_seq: HashMap::new(),
            diagnostics: HashMap::new(),
            hover_text: None,
            pending_hover_id: 0,
            pending_definition_id: 0,
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
        self.text_cache = None;
        self.syntax_seq = None;
    }

    /// The current buffer's full text, materialized at most once per edit
    /// (see the `text_cache` field docs). Cheap (an Rc clone) for every
    /// caller after the first on a given frame.
    fn buffer_text(&mut self) -> std::rc::Rc<str> {
        let key = (self.cur, self.buffers[self.cur].edit_seq);
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
        if self.markdown_preview.as_ref().is_some_and(|p| p.needs_refresh(seq, width)) {
            let text = self.buffer_text();
            self.markdown_preview.as_mut().unwrap().refresh(&text, seq, width);
        }
    }

    fn doc_uri(path: &std::path::Path) -> String {
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let s = abs.to_string_lossy().replace(' ', "%20");
        format!("file://{}", s)
    }

    fn uri_to_path(uri: &str) -> Option<PathBuf> {
        uri.strip_prefix("file://").map(|s| PathBuf::from(s.replace("%20", " ")))
    }

    /// Spawns a language server for the current buffer's filetype if one
    /// isn't already running (or known unavailable), keeps it in sync with
    /// the buffer's edits, and applies whatever it's sent back since the
    /// last frame (diagnostics, hover/definition/completion responses).
    /// Call once per frame; every branch is a cheap no-op when idle.
    /// Buffer-driven half: spawns a server for the current filetype if
    /// needed and keeps it in sync with edits. Cheap to call every frame --
    /// only does real work when the filetype or edit_seq actually changed.
    /// Does *not* drain server responses; see `poll_lsp_events`.
    pub fn sync_lsp(&mut self) {
        let Some(path) = self.buf().path.clone() else { return };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return };
        let Some(lang_id) = crate::lsp::lang_id_for_extension(&ext.to_lowercase()) else { return };

        if !self.lsp_clients.contains_key(lang_id) && !self.lsp_unavailable.contains(lang_id) {
            let root = path.parent().map(Self::doc_uri).unwrap_or_default();
            match crate::lsp::LspClient::spawn(lang_id, &root) {
                Some(client) => {
                    self.set_message(format!("lsp: started {} for {}", client.server_cmd, lang_id));
                    self.lsp_clients.insert(lang_id, client);
                }
                None => {
                    self.lsp_unavailable.insert(lang_id);
                }
            }
        }

        let seq = self.buf().edit_seq;
        let already_opened = self.lsp_opened_docs.contains(&path);
        let needs_sync = self.lsp_synced_seq.get(&path) != Some(&seq);
        if self.lsp_clients.contains_key(lang_id) && (!already_opened || needs_sync) {
            let text = self.buffer_text();
            let uri = Self::doc_uri(&path);
            let client = self.lsp_clients.get_mut(lang_id).unwrap();
            if !already_opened {
                client.did_open(&uri, lang_id, &text);
                self.lsp_opened_docs.insert(path.clone());
            } else {
                client.did_change(&uri, &text);
            }
            self.lsp_synced_seq.insert(path.clone(), seq);
        }
    }

    /// Server-driven half: non-blocking drain of every active client's
    /// response/notification channel. Returns whether anything was applied
    /// (diagnostics, a hover/definition/completion reply) -- callers use
    /// that to decide whether a redraw is warranted, so idle polling this
    /// every ~150ms doesn't turn into a busy-redraw loop when the servers
    /// have nothing new to say (the common case).
    pub fn poll_lsp_events(&mut self) -> bool {
        let mut changed = false;
        let lang_ids: Vec<&'static str> = self.lsp_clients.keys().copied().collect();
        for lid in lang_ids {
            let mut dead = false;
            let events = match self.lsp_clients.get_mut(lid) {
                Some(c) => {
                    if !c.is_alive() {
                        dead = true;
                        Vec::new()
                    } else {
                        c.poll()
                    }
                }
                None => Vec::new(),
            };
            if dead {
                self.lsp_clients.remove(lid);
                self.lsp_unavailable.insert(lid);
                self.set_message(format!("lsp: {} server exited", lid));
                changed = true;
                continue;
            }
            if !events.is_empty() {
                changed = true;
            }
            for ev in events {
                self.apply_lsp_event(ev);
            }
        }
        changed
    }

    fn apply_lsp_event(&mut self, ev: crate::lsp::LspEvent) {
        use crate::lsp::LspEvent;
        match ev {
            LspEvent::Diagnostics { uri, diags } => {
                if let Some(path) = Self::uri_to_path(&uri) {
                    self.diagnostics.insert(path, diags);
                }
            }
            LspEvent::Hover { request_id, text } => {
                if request_id == self.pending_hover_id {
                    self.hover_text = text.clone();
                    match text {
                        Some(t) => self.set_message(t.lines().next().unwrap_or("").to_string()),
                        None => self.set_message("no hover information"),
                    }
                }
            }
            LspEvent::Definition { request_id, uri, line, col } => {
                if request_id == self.pending_definition_id {
                    if let Some(path) = Self::uri_to_path(&uri) {
                        if self.buf().path.as_ref() != Some(&path) {
                            if let Err(e) = self.open_file(path) {
                                self.set_message(format!("could not open: {}", e));
                                return;
                            }
                        }
                        self.set_cursor(line, col);
                    }
                }
            }
            LspEvent::Completion { request_id, items } => {
                if let Some(comp) = &mut self.completion {
                    if comp.request_id == request_id {
                        let lsp_items: Vec<crate::completion::Item> = items
                            .into_iter()
                            .map(|i| crate::completion::Item {
                                label: i.label,
                                insert_text: i.insert_text,
                                detail: i.detail,
                                source: crate::completion::Source::Lsp,
                            })
                            .collect();
                        comp.items.retain(|i| i.source != crate::completion::Source::Lsp);
                        for item in lsp_items.into_iter().rev() {
                            comp.items.insert(0, item);
                        }
                    }
                }
            }
        }
    }

    pub fn request_hover(&mut self) {
        let Some(path) = self.buf().path.clone() else {
            self.set_message("no LSP for this buffer");
            return;
        };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return };
        let Some(lang_id) = crate::lsp::lang_id_for_extension(&ext.to_lowercase()) else {
            self.set_message("no LSP for this filetype");
            return;
        };
        let (line, col) = self.cursor();
        let uri = Self::doc_uri(&path);
        self.next_request_id += 1;
        self.pending_hover_id = self.next_request_id;
        let id = self.pending_hover_id;
        match self.lsp_clients.get_mut(lang_id) {
            Some(client) => client.request_hover(&uri, line, col, id),
            None => self.set_message(if self.lsp_unavailable.contains(lang_id) {
                format!("no language server available for {}", lang_id)
            } else {
                "language server still starting".to_string()
            }),
        }
    }

    pub fn request_definition(&mut self) {
        let Some(path) = self.buf().path.clone() else {
            self.set_message("no LSP for this buffer");
            return;
        };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return };
        let Some(lang_id) = crate::lsp::lang_id_for_extension(&ext.to_lowercase()) else {
            self.set_message("no LSP for this filetype");
            return;
        };
        let (line, col) = self.cursor();
        let uri = Self::doc_uri(&path);
        self.next_request_id += 1;
        self.pending_definition_id = self.next_request_id;
        let id = self.pending_definition_id;
        match self.lsp_clients.get_mut(lang_id) {
            Some(client) => client.request_definition(&uri, line, col, id),
            None => self.set_message(if self.lsp_unavailable.contains(lang_id) {
                format!("no language server available for {}", lang_id)
            } else {
                "language server still starting".to_string()
            }),
        }
    }

    fn request_lsp_completion(&mut self, line: usize, col: usize, request_id: u64) {
        let Some(path) = self.buf().path.clone() else { return };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return };
        let Some(lang_id) = crate::lsp::lang_id_for_extension(&ext.to_lowercase()) else { return };
        let uri = Self::doc_uri(&path);
        if let Some(client) = self.lsp_clients.get_mut(lang_id) {
            client.request_completion(&uri, line, col, request_id);
        }
    }

    /// (Re)opens the git connection if the current buffer's path changed,
    /// and re-diffs if the buffer was edited since the last check. Cheap
    /// no-op otherwise -- call once per frame.
    pub fn ensure_git(&mut self) {
        let path = self.buf().path.clone();
        if path != self.git_path {
            self.git_path = path.clone();
            self.git = path.as_deref().and_then(crate::gitdiff::GitGutter::new);
        }
        let seq = self.buffers[self.cur].edit_seq;
        if self.git.as_ref().is_some_and(|g| g.needs_refresh(seq)) {
            let text = self.buffer_text();
            self.git.as_mut().unwrap().refresh(&text, seq);
        }
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
        let items = crate::completion::buffer_word_candidates(self.buf(), &prefix, line);
        let has_lsp = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .and_then(|e| crate::lsp::lang_id_for_extension(&e.to_lowercase()))
            .is_some_and(|lang| self.lsp_clients.contains_key(lang));
        if items.is_empty() && !has_lsp {
            self.completion = None;
            return;
        }
        self.next_request_id += 1;
        let request_id = self.next_request_id;
        self.completion =
            Some(crate::completion::CompletionState { start: (line, start_col), prefix, items, selected: 0, request_id });
        if has_lsp {
            self.request_lsp_completion(line, col, request_id);
        }
    }

    pub fn close_completion(&mut self) {
        self.completion = None;
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
        }

        let key = (self.cur, self.buffers[self.cur].edit_seq);
        if self.syntax.is_some() && self.syntax_seq != Some(key) {
            let text = self.buffer_text();
            self.syntax.as_mut().unwrap().reparse(text);
            self.syntax_seq = Some(key);
        } else if let Some(syn) = &mut self.syntax {
            // No new edit this frame, but a previous reparse may have left
            // its (throttled) span rebuild deferred -- finish it once the
            // throttle window passes, so highlighting doesn't stay stale
            // indefinitely after typing pauses. See syntax_catch_up_due's
            // docs for why the idle path also needs to poll this.
            syn.catch_up();
        }
    }

    /// Whether a throttled syntax span rebuild is waiting on its window to
    /// elapse. `ensure_syntax` only runs from the main loop's "something
    /// happened" path (a key arrived, or an LSP event did); the *idle*
    /// wait -- no key, no LSP activity -- never calls it otherwise, so a
    /// rebuild deferred right as the user stops typing would sit finished-
    /// but-unseen (or never finished at all) until the next keystroke with
    /// no reason for this method's caller to poll it. The main loop polls
    /// this once per idle tick and redraws when it flips true.
    pub fn syntax_catch_up_due(&self) -> bool {
        self.syntax.as_ref().is_some_and(|s| s.rebuild_due())
    }

    pub fn open_picker(&mut self) {
        if self.all_files.is_empty() {
            self.all_files = crate::picker::scan_files(&std::env::current_dir().unwrap_or_default());
        }
        self.file_picker = Some(crate::picker::FilePicker::new(&self.all_files));
        self.mode = Mode::Picker;
    }

    pub fn open_file(&mut self, path: PathBuf) -> anyhow::Result<()> {
        // Focus an already-open buffer for this file instead of loading a
        // second, independent copy of it -- without this, :e (and LSP
        // goto-definition) on an already-open file reads disk into a
        // competing buffer, and whichever one saves last silently wins.
        let target_abs = std::fs::canonicalize(&path).ok();
        if let Some(target_abs) = &target_abs {
            if let Some(idx) = self.buffers.iter().position(|b| {
                b.path.as_ref().and_then(|p| std::fs::canonicalize(p).ok()).as_ref() == Some(target_abs)
            }) {
                self.cur = idx;
                return Ok(());
            }
        }

        let buf = Buffer::from_path(path)?;
        if self.buffers.len() == 1 && self.buffers[0].path.is_none() && !self.buffers[0].is_modified() {
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
        let col = match b.clamp_col_normal(line, col) {
            c => c,
        };
        b.cursor_line = line;
        b.cursor_col = col;
    }

    pub fn set_cursor_insert(&mut self, line: usize, col: usize) {
        let b = self.buf_mut();
        let line = line.min(b.line_count().saturating_sub(1));
        let col = b.clamp_col_insert(line, col);
        b.cursor_line = line;
        b.cursor_col = col;
    }

    /// Single entry point for every key: main loop and macro/dot replay funnel through here.
    pub fn feed_key(&mut self, key: Key) {
        // Macro recording: a bare 'q' in Normal mode with nothing pending stops recording
        // instead of being processed as a command.
        if self.macro_recording.is_some()
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
        if let Some((_, keys)) = &mut self.macro_recording {
            keys.push(key);
        }

        // Dot-repeat recording: capture the raw keys of the in-flight change command.
        if self.recording_change {
            self.cmd_keys.push(key);
        }

        match self.mode {
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
            self.cmd_keys = vec![first_key];
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
        let was_replaying = self.replaying;
        self.replaying = true;
        for k in keys.to_vec() {
            self.feed_key(k);
        }
        self.replaying = was_replaying;
    }

    pub fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
    }

    pub fn enter_normal(&mut self) {
        self.mode = Mode::Normal;
        let (l, c) = self.cursor();
        let nc = self.buf().clamp_col_normal(l, c);
        self.buf_mut().cursor_col = nc;
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
            let (line, col) = self.cursor();
            self.buf_mut().insert_char(line, col, 'j');
            self.set_cursor_insert(line, col + 1);
        }
    }
}
