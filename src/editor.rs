use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::buffer::Buffer;
use crate::config::Config;
use crate::key::Key;
use crate::mode::{CommandKind, Mode, VisualKind};
use crate::normal::PendingState;
use crate::registers::Registers;

/// A recorded visual selection: `(kind, anchor, cursor)` where anchor and
/// cursor are `(line, col)` positions. Used by `gv` to reselect.
pub type VisualSelection = (VisualKind, (usize, usize), (usize, usize));

/// A document-color span: `(line, start_col, end_col, (r, g, b))`.
pub type ColorSpan = (usize, usize, usize, (u8, u8, u8));

/// Tree-sitter node kinds that count as a function/method definition, across
/// the grammars vaayu bundles. Shared by `af`/`if` text objects and `]f`/`[f`
/// function navigation.
pub(crate) const FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_declaration",
    "function_definition",
    "method_declaration",
    "method_definition",
    "function",
    "arrow_function",
    "function_expression",
    "function_signature_item",
];

struct SearchCache {
    buffer: u64,
    revision: u64,
    pattern: String,
    ignorecase: bool,
    smartcase: bool,
    matches: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResumeTarget {
    Results,
    Picker,
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
    /// `:colder`/`:cnewer` history of quickfix lists, oldest first.
    /// `quickfix` always mirrors `quickfix_history[quickfix_history_pos]`.
    /// Only `export_quickfix` (Ctrl-Q -- a genuinely new list) appends;
    /// merely revisiting/dismissing the current one updates that slot in
    /// place instead of growing history (see `remember_results`).
    pub quickfix_history: Vec<crate::results::Results>,
    pub quickfix_history_pos: usize,
    pub marks: HashMap<char, crate::navigation::Location>,
    pub jumps: Vec<crate::navigation::Location>,
    pub jump_index: usize,
    pub alternate_buffer: Option<u64>,
    pub search_job: crate::jobs::SearchJob,
    pub windows: Vec<crate::windows::Window>,
    pub active_window: usize,
    pub split_vertical: bool,
    pub window_layout: Option<crate::windows::Layout>,
    /// All tabs, including the active one -- but the active tab's entry is
    /// only kept in sync on switch (`store_tab`), not continuously; the
    /// live `windows`/`window_layout`/`active_window`/`cur` fields above
    /// are the source of truth for whichever tab is active right now,
    /// mirroring how `store_window` already treats `windows` vs. the
    /// live cursor/top/left fields on `Buffer`.
    pub tabs: Vec<crate::windows::Tab>,
    pub active_tab: usize,
    /// Capped, in-memory only for this slice (see `command.rs`'s
    /// `MAX_HISTORY`) -- not yet persisted across restarts the way notes/
    /// recovery/undo are.
    pub command_history: Vec<String>,
    pub search_history: Vec<String>,
    /// `Some(index)` while cycling through history with Up/Down in the
    /// command line; `history_draft` holds what was typed before cycling
    /// started, restored when cycling back past the newest entry.
    pub history_browse: Option<usize>,
    pub history_draft: String,
    /// Ex command-line Tab-completion (wildmenu): the candidate full command
    /// lines and which one is currently selected. Reset on any non-Tab key.
    pub cmdline_completions: Vec<String>,
    pub cmdline_completion_index: Option<usize>,
    pub screen_cols: usize,
    pub window_prefix: bool,
    pub pending_language: HashMap<u64, crate::language::RequestContext>,

    pub buffers: Vec<Buffer>,
    pub cur: usize,
    pub mode: Mode,
    pub config: Config,
    /// User key remaps parsed from `[[keymap]]` (see keymap.rs).
    pub keymaps: Vec<crate::keymap::Keymap>,
    pub registers: Registers,
    pub message: String,
    /// Bounded, consecutive-deduped history of messages shown via
    /// `set_message`, for the `:messages` viewer.
    pub messages: Vec<String>,
    pub should_quit: bool,
    /// Re-entrancy guard for the autocommand bus (see `event.rs`): an autocmd
    /// whose Ex command fires the same event again must not recurse forever.
    pub event_depth: usize,

    pub pending: PendingState,
    pub visual_anchor: Option<(usize, usize)>,
    /// The most recent visual selection — `(kind, anchor, cursor)` in
    /// (line, col) coordinates — recorded on each visual-mode keystroke so
    /// `gv` can reselect it after the selection has been used or cancelled.
    pub last_visual: Option<VisualSelection>,
    /// Byte ranges of prior selections during tree-sitter incremental
    /// selection, so shrink can walk back the exact expand path.
    pub select_stack: Vec<(usize, usize)>,
    pub cmdline: String,

    pub last_search: Option<(String, bool)>,
    /// While typing a `/`/`?` search (incsearch): the in-progress pattern to
    /// highlight and preview. `None` when not actively searching.
    pub incsearch: Option<String>,
    /// The cursor+scroll to restore if a `/`/`?` search is cancelled, and the
    /// position the live/submitted search runs from: (line, col, top_line,
    /// top_wrap).
    pub search_origin: Option<(usize, usize, usize, usize)>,
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
    /// Buffer position of the last mouse-down, so a subsequent drag knows
    /// where to anchor the Visual selection it starts.
    pub mouse_down_at: Option<(usize, usize)>,
    /// Lazily loaded on first spell-check use, not at startup: reading a
    /// system word list (hundreds of KB to a few MB) on every launch --
    /// and every test's `Editor::new`, of which there are many -- for a
    /// feature most sessions never touch would be wasted work.
    pub dictionary: Option<crate::spell::Dictionary>,
    pub terminals: Vec<crate::pty::PtySession>,
    pub file_tree: Option<crate::filetree::FileTree>,
    pub outline: Option<crate::outline::Outline>,
    /// `textDocument/documentHighlight` results: every occurrence of the
    /// symbol under the cursor at request time, as `(line1, col1, line2,
    /// col2)` char ranges in `document_highlights_buffer`. Painted as a
    /// background highlight in `render.rs`, only while
    /// `document_highlights_edit_seq` still matches that buffer's current
    /// `edit_seq` -- a stale set (buffer edited since the request) is
    /// silently skipped rather than painting now-wrong ranges. Cleared by
    /// a plain Esc in Normal mode, or replaced by the next request.
    pub document_highlights: Vec<(usize, usize, usize, usize)>,
    pub document_highlights_buffer: Option<u64>,
    pub document_highlights_edit_seq: u64,
    /// LSP `textDocument/documentColor`: `(line, start_col, end_col, (r,g,b))`
    /// spans, painted by colorizing the literal in its own color. Gated on
    /// `document_colors_buffer`/`_edit_seq` the same way document highlights are.
    pub document_colors: Vec<ColorSpan>,
    pub document_colors_buffer: Option<u64>,
    pub document_colors_edit_seq: u64,
    /// CursorHold / illuminate bookkeeping: the `(buffer, line, col)` the
    /// cursor currently rests at, when it arrived there, and whether the hold
    /// has already fired for it (so it fires once per resting position).
    pub hold_pos: Option<(u64, usize, usize)>,
    hold_since: Instant,
    hold_fired: bool,
    /// Per-file last cursor position (Vim's shada `'"`), loaded lazily from
    /// `.vaayu/shada.json` and persisted on quit. See `src/shada.rs`.
    pub file_positions: HashMap<PathBuf, (usize, usize)>,
    pub file_positions_loaded: bool,

    /// `textDocument/codeLens` results: one `(line, title, runnable_action)`
    /// per lens that actually has a `command` (a lens with only `data`,
    /// deferred to `codeLens/resolve`, is skipped rather than adding
    /// another resolve round trip -- the same choice already made for
    /// document links without a `target`). Painted as virtual text after
    /// each lens's own line's content (like line blame) *and* shown as a
    /// "Code lenses" Results list so a lens can actually be run, not just
    /// seen -- `runnable_action` is pre-shaped exactly like a code
    /// action's own `action` value so `apply_code_action` (via
    /// `results.rs`'s existing fallback dispatch for an untagged action)
    /// runs it with no new dispatch code. Staleness follows the same
    /// buffer-id/edit_seq pattern as `document_highlights`.
    pub code_lenses: Vec<(usize, String, serde_json::Value)>,
    pub code_lenses_buffer: Option<u64>,
    pub code_lenses_edit_seq: u64,

    /// `textDocument/inlayHint` results: `(line, col, label)` triples,
    /// `col` being the char column the hint is inserted *before* (e.g. a
    /// parameter name before an argument, or a type after a variable
    /// name -- whatever the server's `position` says). Painted inline
    /// -- not appended after the line like blame/code-lens text, since a
    /// real inlay hint's whole point is sitting at its own position
    /// among the real characters -- and cleared by the same Esc that
    /// already clears `document_highlights`. Same buffer-id/edit_seq
    /// staleness pattern as `document_highlights`/`code_lenses`.
    pub inlay_hints: Vec<(usize, usize, String)>,
    pub inlay_hints_buffer: Option<u64>,
    pub inlay_hints_edit_seq: u64,

    pub file_picker: Option<crate::picker::FilePicker>,
    pub all_files: Vec<String>,
    /// A dismissed file picker's state (query/matches/selection), kept so
    /// `:resume` can reopen it exactly as it was left, not a fresh one.
    pub last_picker: Option<crate::picker::FilePicker>,
    /// Which of `last_picker` / the (never-cleared) `results` was more
    /// recently active, so `:resume` knows which one to reopen.
    pub last_resume: Option<ResumeTarget>,
    /// Buffer ids in most-recently-activated order (front = most recent),
    /// touched at the same genuine-user-switch sites `alternate_buffer`
    /// already is. `:buffers`/`,b` lists in this order rather than
    /// insertion order.
    pub buffer_mru: Vec<u64>,

    pub syntax: Option<crate::syntax::Syntax>,
    pub syntax_stamp: u64,
    syntax_seq: Option<(u64, u64)>,
    syntax_pending: Option<((u64, u64), Instant, bool)>,

    pub completion: Option<crate::completion::CompletionState>,
    /// When the current `completion` was (re)triggered -- reset on every
    /// keystroke that recomputes it, not just when a session first opens,
    /// so `completion_delay_ms` measures quiet time since the *last*
    /// keystroke, the same debounce shape `whichkey_delay_ms` uses for
    /// its own popup. `render.rs` gates painting the popup on this;
    /// `close_completion` clears it along with `completion` itself.
    pub completion_since: Option<Instant>,
    pub(crate) next_request_id: u64,

    pub git: Option<crate::gitdiff::GitGutter>,
    pub git_job: crate::gitdiff::GitJob,
    pub git_task: Option<crate::git_tools::GitTask>,
    /// `,gB`: whether the line-blame virtual text (drawn at the end of
    /// the buffer's current line) is on. `line_blame` is one metadata
    /// string per line ("<short hash> <author/date>", line number
    /// stripped) for `line_blame_path`, populated asynchronously by
    /// `blame_task` the same way other potentially-slow git calls
    /// (`git_results`'s own "diff"/"blame" kinds) already avoid blocking
    /// the main loop on a large file/history.
    pub blame_toggle: bool,
    pub line_blame: Option<Vec<String>>,
    pub line_blame_path: Option<PathBuf>,
    pub(crate) blame_task: Option<crate::git_tools::BlameTask>,
    /// `,gd`: whether the diff overlay (deleted-line content and
    /// intra-line word-diff highlighting, both from the same `GitGutter`
    /// data the gutter signs already use) is on. No separate fetch is
    /// needed to toggle it -- `GitGutter::refresh` always computes this
    /// data alongside the signs, in the same background thread, so
    /// turning it on just changes what the renderer reads.
    pub diff_overlay: bool,
    /// `,gW`: whether `:gitdiff`'s read-only diff view ignores
    /// whitespace-only changes (`git diff --ignore-all-space`). Off by
    /// default; deliberately not threaded into anything that stages or
    /// resets from the result -- see `git_results`'s own doc comment.
    pub diff_ignore_whitespace: bool,

    pub lsp_clients: HashMap<String, crate::lsp::LspClient>,
    pub(crate) lsp_unavailable: HashSet<String>,
    pub(crate) lsp_opened_docs: HashSet<(String, PathBuf)>,
    pub(crate) lsp_synced_seq: HashMap<(String, PathBuf), u64>,
    pub diagnostics: HashMap<PathBuf, Vec<crate::lsp::Diagnostic>>,
    pub server_diagnostics: HashMap<(String, PathBuf), Vec<crate::lsp::Diagnostic>>,
    /// Active `$/progress` tokens, keyed by (client key, token). Surfaced
    /// through the ordinary message line (`set_message`) -- see
    /// `Editor::format_lsp_progress` -- rather than a separate persistent
    /// panel; a later action naturally overwrites it, the same as any
    /// other message in this editor.
    pub lsp_progress: HashMap<(String, String), crate::lsp::LspProgress>,
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
        let keymaps = crate::keymap::build(
            &config.keymap,
            config.leader.chars().next().unwrap_or(','),
        );
        Editor {
            keymaps,
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
            quickfix_history: Vec::new(),
            quickfix_history_pos: 0,
            marks: HashMap::new(),
            jumps: Vec::new(),
            jump_index: 0,
            alternate_buffer: None,
            search_job: Default::default(),
            windows: Vec::new(),
            active_window: 0,
            split_vertical: true,
            window_layout: None,
            tabs: vec![crate::windows::Tab::default()],
            active_tab: 0,
            command_history: Vec::new(),
            search_history: Vec::new(),
            history_browse: None,
            history_draft: String::new(),
            cmdline_completions: Vec::new(),
            cmdline_completion_index: None,
            screen_cols: 80,
            window_prefix: false,
            pending_language: HashMap::new(),
            buffers: vec![Buffer::empty()],
            cur: 0,
            mode: Mode::Normal,
            config,
            registers,
            message: String::from("vaayu — :help for keys · :w to save · :q to quit"),
            messages: Vec::new(),
            should_quit: false,
            event_depth: 0,
            pending: PendingState::default(),
            visual_anchor: None,
            last_visual: None,
            select_stack: Vec::new(),
            cmdline: String::new(),
            last_search: None,
            incsearch: None,
            search_origin: None,
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
            mouse_down_at: None,
            dictionary: None,
            terminals: Vec::new(),
            file_tree: None,
            outline: None,
            document_highlights: Vec::new(),
            document_highlights_buffer: None,
            document_highlights_edit_seq: 0,
            document_colors: Vec::new(),
            document_colors_buffer: None,
            document_colors_edit_seq: 0,
            hold_pos: None,
            hold_since: Instant::now(),
            hold_fired: false,
            file_positions: HashMap::new(),
            file_positions_loaded: false,
            code_lenses: Vec::new(),
            code_lenses_buffer: None,
            code_lenses_edit_seq: 0,
            inlay_hints: Vec::new(),
            inlay_hints_buffer: None,
            inlay_hints_edit_seq: 0,
            screen_rows: 24,
            hl_search: true,
            file_picker: None,
            last_picker: None,
            last_resume: None,
            buffer_mru: Vec::new(),
            all_files: Vec::new(),
            syntax: None,
            syntax_stamp: 0,
            syntax_seq: None,
            syntax_pending: None,
            completion: None,
            completion_since: None,
            next_request_id: 0,
            git: None,
            git_job: Default::default(),
            git_task: None,
            diff_overlay: false,
            diff_ignore_whitespace: false,
            blame_toggle: false,
            line_blame: None,
            line_blame_path: None,
            blame_task: None,
            lsp_clients: HashMap::new(),
            lsp_unavailable: HashSet::new(),
            lsp_opened_docs: HashSet::new(),
            lsp_synced_seq: HashMap::new(),
            diagnostics: HashMap::new(),
            server_diagnostics: HashMap::new(),
            lsp_progress: HashMap::new(),
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
        if !self.config.completion_enabled {
            self.close_completion();
            return;
        }
        let (line, col) = self.cursor();
        // Path-shaped input (contains a `/`) wins outright: it's never
        // simultaneously a valid identifier, so there's no ambiguity to
        // resolve, and buffer-word/LSP completion would just be noise.
        if let Some((start_col, prefix)) = crate::completion::path_prefix(self.buf(), line, col) {
            let base_dir = self
                .buf()
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| self.project_root.clone());
            let items = crate::completion::path_candidates(&prefix, &base_dir);
            if items.is_empty() {
                self.close_completion();
            } else {
                self.next_request_id += 1;
                self.completion = Some(crate::completion::CompletionState {
                    start: (line, start_col),
                    items,
                    selected: 0,
                    request_id: self.next_request_id,
                });
                self.completion_since = Some(Instant::now());
            }
            return;
        }
        let (start_col, prefix) = crate::completion::word_prefix(self.buf(), line, col);
        if prefix.is_empty() {
            self.close_completion();
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
            self.close_completion();
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
        self.completion_since = Some(Instant::now());
        if has_lsp {
            self.request_lsp_completion(line, col, request_id);
        }
    }

    pub fn close_completion(&mut self) {
        self.completion = None;
        self.completion_since = None;
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

    /// Called from the idle loop: detects a cursor that has come to rest and
    /// fires `CursorHold` once for that position (driving illuminate). Returns
    /// true if the screen should be redrawn (stale highlights cleared, or the
    /// hold just fired). Only active in Normal/Visual so it never interrupts
    /// insert- or command-line editing.
    pub fn poll_cursor_hold(&mut self) -> bool {
        if !matches!(self.mode, Mode::Normal | Mode::Visual(_)) {
            return false;
        }
        let pos = (self.buf().id, self.cursor().0, self.cursor().1);
        if self.hold_pos != Some(pos) {
            // Cursor moved: restart the timer. If it moved *between* two rest
            // positions (not the first observation), drop now-stale illuminate
            // highlights so they don't linger on the previous symbol. The
            // first observation must not clear freshly-set (e.g. manual)
            // highlights at the current position.
            let was_armed = self.hold_pos.is_some();
            self.hold_pos = Some(pos);
            self.hold_since = Instant::now();
            self.hold_fired = false;
            if was_armed && self.config.illuminate && !self.document_highlights.is_empty() {
                self.document_highlights.clear();
                return true;
            }
            return false;
        }
        if self.hold_fired {
            return false;
        }
        if self.hold_since.elapsed() < std::time::Duration::from_millis(self.config.updatetime_ms) {
            return false;
        }
        self.hold_fired = true;
        self.fire_event(crate::events::Event::CursorHold);
        if self.config.illuminate {
            self.illuminate();
        }
        true
    }

    /// Requests LSP document highlights for the symbol under the cursor, but
    /// only when a capable server is attached (so it stays silent otherwise).
    fn illuminate(&mut self) {
        if self.buf().path.is_some() && self.has_language_capability("documentHighlightProvider") {
            self.request_language("documentHighlight", None);
        }
    }

    pub fn open_picker(&mut self) {
        self.start_file_scan();
        self.file_picker = Some(crate::picker::FilePicker::new(&self.all_files));
        self.mode = Mode::Picker;
    }

    /// `:resume`: reopens whichever of the file picker or a Results/
    /// quickfix list was more recently dismissed, exactly as it was left
    /// (query, matches, cursor, selection) rather than starting fresh.
    pub fn resume(&mut self) {
        match self.last_resume {
            Some(ResumeTarget::Picker) if self.last_picker.is_some() => {
                self.start_file_scan();
                self.file_picker = self.last_picker.take();
                self.mode = Mode::Picker;
            }
            Some(ResumeTarget::Results) if self.results.is_some() => {
                self.mode = Mode::Results;
            }
            _ => self.set_message("Nothing to resume"),
        }
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

    /// Records the current buffer as the alternate before switching away
    /// from it, so `Ctrl-6`/`:b#` can toggle back to it (matching Vim's
    /// `Ctrl-^`). Only call this at genuine user-driven buffer switches
    /// (opening a different file, `:b`/`:bnext`/`:bprev`, a picker/results
    /// selection) -- not at window/tab/session-restore bookkeeping sites
    /// that reassign `cur` to reflect pane focus rather than a real switch.
    pub fn note_alternate_buffer(&mut self) {
        if let Some(b) = self.buffers.get(self.cur) {
            self.alternate_buffer = Some(b.id);
        }
    }

    /// Moves `id` to the front of `buffer_mru` (inserting it if new).
    /// Call this right after `self.cur` is reassigned to a genuinely
    /// different buffer, at the same sites `note_alternate_buffer` is
    /// already called before that assignment.
    pub fn touch_buffer_mru(&mut self, id: u64) {
        self.buffer_mru.retain(|&x| x != id);
        self.buffer_mru.insert(0, id);
    }

    /// Toggles to the alternate buffer (`Ctrl-6` / `:b#`), matching Vim's
    /// `Ctrl-^`. A second press returns to where you started.
    pub fn switch_to_alternate(&mut self) {
        let Some(id) = self.alternate_buffer else {
            self.set_message("No alternate buffer");
            return;
        };
        let Some(i) = self.buffers.iter().position(|b| b.id == id) else {
            self.set_message("Alternate buffer no longer exists");
            return;
        };
        self.note_alternate_buffer();
        self.push_jump();
        self.cur = i;
        self.touch_buffer_mru(id);
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
                if idx != self.cur {
                    self.note_alternate_buffer();
                }
                self.cur = idx;
                self.touch_buffer_mru(self.buffers[idx].id);
                self.fire_event(crate::events::Event::BufEnter);
                return Ok(());
            }
        }

        let mut buf = Buffer::from_path(path)?;
        buf.apply_indent(&self.config);
        crate::undofile::restore(&self.project_root, &mut buf);
        if self.buffers.len() == 1
            && self.buffers[0].path.is_none()
            && !self.buffers[0].is_modified()
        {
            self.buffers[0] = buf;
        } else {
            self.note_alternate_buffer();
            self.buffers.push(buf);
            self.cur = self.buffers.len() - 1;
        }
        self.touch_buffer_mru(self.buffers[self.cur].id);
        // A buffer at an existing index can be swapped out for one with the
        // same (index, edit_seq==0) key as the buffer it replaced -- see
        // invalidate_index_caches' docs.
        self.invalidate_index_caches();
        // Restore the last-known cursor for a freshly loaded file (shada).
        self.restore_file_position();
        self.fire_event(crate::events::Event::BufEnter);
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

        // User single-key remaps (Normal/Insert/Visual, clean pending, not while
        // replaying a mapping -- noremap). The lhs is already recorded above, so
        // macros/dot-repeat re-trigger the remap on replay.
        if let Some(rhs) = self.single_key_remap(key) {
            self.apply_remap(rhs);
            return;
        }

        match self.mode {
            Mode::Results => crate::results::handle(self, key),
            Mode::Normal => crate::normal::handle(self, key),
            Mode::Insert => crate::insert::handle(self, key),
            Mode::Visual(_) => crate::visual::handle(self, key),
            Mode::Command(_) => crate::command::handle(self, key),
            Mode::Picker => crate::picker::handle(self, key),
            Mode::MarkdownPreview => crate::preview::handle(self, key),
            Mode::Terminal => crate::pty::handle_terminal_mode(self, key),
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
        // Cheap even when nothing was actually deferred (Insert mode's
        // `diagnostics_update_in_insert=false` freeze is the only thing
        // that ever defers anything) -- catches up whatever arrived
        // while frozen, every time Insert mode could have just ended.
        self.flush_deferred_diagnostics();
    }

    pub fn insert_paste(&mut self, text: &str) {
        self.flush_pending_jk();
        self.close_completion();
        // In command-line mode a bracketed paste (e.g. Ctrl+Shift+V while
        // typing `:e <path>`, or into a `/`/`?` search prompt) belongs in the
        // command line, not the buffer. Append the pasted text with newlines
        // and carriage returns stripped so it can neither submit the command
        // nor corrupt the prompt, then stop -- never touch the buffer.
        if matches!(self.mode, Mode::Command(_)) {
            self.cmdline
                .extend(text.chars().filter(|&c| c != '\n' && c != '\r'));
            return;
        }
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
    /// The current selection as an exclusive char range `[start, end)`: the
    /// Visual selection if active, else the single char under the cursor.
    fn selection_char_range(&self) -> (usize, usize) {
        let (cl, cc) = self.cursor();
        let cursor_ci = self.buf().char_idx(cl, cc);
        if let (Mode::Visual(_), Some((al, ac))) = (self.mode, self.visual_anchor) {
            let anchor_ci = self.buf().char_idx(al, ac);
            (cursor_ci.min(anchor_ci), cursor_ci.max(anchor_ci) + 1)
        } else {
            // A collapsed cursor: zero-width, so the first expand selects the
            // token under the cursor rather than jumping to its parent.
            (cursor_ci, cursor_ci)
        }
    }

    /// Set the Visual selection to a byte range (from tree-sitter node bounds).
    fn set_selection_bytes(&mut self, sb: usize, eb: usize) {
        let (sc, ec) = {
            let rope = &self.buf().rope;
            let total = rope.len_bytes();
            let sc = rope.byte_to_char(sb.min(total));
            let ec = rope.byte_to_char(eb.min(total)).saturating_sub(1);
            (sc, ec.max(sc))
        };
        let (al, ac) = self.buf().pos_from_char_idx(sc);
        let (el, ecol) = self.buf().pos_from_char_idx(ec);
        self.visual_anchor = Some((al, ac));
        self.mode = Mode::Visual(VisualKind::Char);
        self.set_cursor(el, ecol);
    }

    /// Move the current line down/up by `count`, carrying the cursor with it.
    /// One undo step. Swaps line *content* only, so trailing-newline structure
    /// (incl. a no-final-newline last line) is preserved.
    pub fn move_lines(&mut self, down: bool, count: usize) {
        let count = count.max(1);
        let l0 = self.cursor().0;
        if (down && l0 + 1 >= self.buf().line_count()) || (!down && l0 == 0) {
            return;
        }
        self.buf_mut().begin_edit();
        for _ in 0..count {
            let l = self.cursor().0;
            let a = if down {
                if l + 1 >= self.buf().line_count() {
                    break;
                }
                l
            } else {
                if l == 0 {
                    break;
                }
                l - 1
            };
            self.swap_line_content(a);
            self.buf_mut().cursor_line = if down { l + 1 } else { l - 1 };
        }
        self.buf_mut().commit_edit();
        let (l, c) = self.cursor();
        self.set_cursor(l, c);
    }

    /// Swap the content (not the newlines) of adjacent lines `a` and `a + 1`.
    fn swap_line_content(&mut self, a: usize) {
        let buf = self.buf_mut();
        let a_start = buf.char_idx(a, 0);
        let a_len = buf.line_len(a);
        let b_start = buf.char_idx(a + 1, 0);
        let b_len = buf.line_len(a + 1);
        let ta = buf.text_range(a_start, a_start + a_len);
        let tb = buf.text_range(b_start, b_start + b_len);
        // Replace the later line first so the earlier line's indices stay valid.
        buf.delete_char_range(b_start, b_start + b_len);
        buf.insert_str_at(b_start, &ta);
        buf.delete_char_range(a_start, a_start + a_len);
        buf.insert_str_at(a_start, &tb);
    }

    /// Tree-sitter text object range (inclusive `(sl, sc, el, ec)`) for a
    /// function (`f`) or class (`c`) around/inner the cursor, or None if the
    /// cursor isn't inside one (or there is no syntax tree).
    pub fn tree_object_range(&self, obj: char, inner: bool) -> Option<(usize, usize, usize, usize)> {
        const CLASS_KINDS: &[&str] = &[
            "struct_item",
            "enum_item",
            "impl_item",
            "trait_item",
            "union_item",
            "class_declaration",
            "class_definition",
            "class",
            "interface_declaration",
            "enum_declaration",
            "struct_specifier",
            "class_specifier",
        ];
        let kinds: &[&str] = match obj {
            'f' => FUNCTION_KINDS,
            'c' => CLASS_KINDS,
            _ => return None,
        };
        let (line, col) = self.cursor();
        let ci = self.buf().char_idx(line, col);
        let byte = self.buf().rope.char_to_byte(ci);
        let (sb, eb) = self.syntax.as_ref()?.object_range(byte, kinds, inner)?;
        let rope = &self.buf().rope;
        let total = rope.len_bytes();
        let sc = rope.byte_to_char(sb.min(total));
        let ec = rope.byte_to_char(eb.min(total)).saturating_sub(1).max(sc);
        let (sl, scol) = self.buf().pos_from_char_idx(sc);
        let (el, ecol) = self.buf().pos_from_char_idx(ec);
        Some((sl, scol, el, ecol))
    }

    /// `]f` / `[f`: move the cursor to the start of the next (`forward`) or
    /// previous function/method definition, `count` of them away. Records a
    /// jumplist entry so `Ctrl-o` returns. No-op without a parsed tree or when
    /// there is no such definition in that direction.
    pub fn goto_function(&mut self, forward: bool, count: usize) {
        let starts = match self.syntax.as_ref() {
            Some(s) => s.node_starts(FUNCTION_KINDS),
            None => return,
        };
        if starts.is_empty() {
            return;
        }
        let (line, col) = self.cursor();
        let ci = self.buf().char_idx(line, col);
        let byte = self.buf().rope.char_to_byte(ci);
        let count = count.max(1);
        let target = if forward {
            starts.iter().filter(|&&b| b > byte).nth(count - 1).copied()
        } else {
            starts
                .iter()
                .rev()
                .filter(|&&b| b < byte)
                .nth(count - 1)
                .copied()
        };
        let Some(b) = target else {
            return;
        };
        self.push_jump();
        let rope = &self.buf().rope;
        let ci = rope.byte_to_char(b.min(rope.len_bytes()));
        let (l, c) = self.buf().pos_from_char_idx(ci);
        self.set_cursor(l, c);
    }

    /// Tree-sitter incremental selection: grow the selection to the next
    /// enclosing syntax node. Starting from Normal mode begins a fresh chain.
    pub fn expand_selection(&mut self) {
        if !matches!(self.mode, Mode::Visual(_)) {
            self.select_stack.clear();
        }
        let (lo_c, hi_c) = self.selection_char_range();
        let (lo_b, hi_b) = {
            let rope = &self.buf().rope;
            let len = rope.len_chars();
            (
                rope.char_to_byte(lo_c.min(len)),
                rope.char_to_byte(hi_c.min(len)),
            )
        };
        let Some(syn) = &self.syntax else {
            self.set_message("No syntax tree for incremental selection");
            return;
        };
        let Some((ns, ne)) = syn.expand_range(lo_b, hi_b) else {
            return;
        };
        if (ns, ne) == (lo_b, hi_b) {
            return; // already at the root; nothing larger
        }
        self.select_stack.push((lo_b, hi_b));
        self.set_selection_bytes(ns, ne);
    }

    /// Shrink the incremental selection back along the expand path.
    pub fn shrink_selection(&mut self) {
        if let Some((sb, eb)) = self.select_stack.pop() {
            self.set_selection_bytes(sb, eb);
        }
    }

    pub fn enter_visual(&mut self, kind: VisualKind) {
        self.visual_anchor = Some(self.cursor());
        self.mode = Mode::Visual(kind);
    }

    /// Reselect the last visual selection (`gv`). Restores the recorded mode,
    /// anchor, and cursor, each clamped to the current buffer in case it has
    /// shrunk since the selection was made.
    pub fn reselect_visual(&mut self) {
        let Some((kind, anchor, cursor)) = self.last_visual else {
            self.set_message("No previous visual selection");
            return;
        };
        let last = self.buf().line_count().saturating_sub(1);
        let al = anchor.0.min(last);
        let ac = self.buf().clamp_col_normal(al, anchor.1);
        self.visual_anchor = Some((al, ac));
        self.mode = Mode::Visual(kind);
        self.set_cursor(cursor.0, cursor.1);
    }

    /// One level of indentation for the current buffer: `shiftwidth` spaces
    /// when `expandtab`, otherwise a single tab.
    pub fn indent_unit(&self) -> String {
        let b = self.buf();
        if b.expandtab {
            " ".repeat(b.shiftwidth.max(1))
        } else {
            "\t".to_string()
        }
    }

    /// The auto-indent for a line opened off `line` by splitting it at
    /// `split_col` (Enter) or opening below it (`o`, with `split_col` = line
    /// length). Copies `line`'s leading whitespace, and — when `smartindent`
    /// is on — adds one indent level if the text up to `split_col`, ignoring
    /// trailing whitespace, ends with an opening bracket. Pass `split_col` = 0
    /// (as `O` does) to get the copied indent with no bracket increase.
    pub fn auto_indent(&self, line: usize, split_col: usize) -> String {
        let text = self.buf().line_text(line);
        let base: String = text
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if !self.config.smartindent {
            return base;
        }
        let prefix: String = text.chars().take(split_col).collect();
        if prefix.trim_end().ends_with(['{', '(', '[']) {
            format!("{base}{}", self.indent_unit())
        } else {
            base
        }
    }

    pub fn enter_command(&mut self, kind: CommandKind) {
        self.cmdline.clear();
        self.mode = Mode::Command(kind);
        self.history_browse = None;
        self.history_draft.clear();
        self.incsearch = None;
        // For `/`/`?`, remember where to preview from and where to restore to
        // if the search is cancelled (incsearch).
        if matches!(kind, CommandKind::SearchFwd | CommandKind::SearchBack) {
            let (l, c) = self.cursor();
            self.search_origin = Some((l, c, self.buf().top_line, self.buf().top_wrap));
            self.hl_search = true;
        } else {
            self.search_origin = None;
        }
    }

    /// Live-preview the in-progress `/`/`?` query (incsearch): move the cursor
    /// to the first match from the search origin and mark the pattern for
    /// highlighting. Empty query or no match returns to the origin; an invalid
    /// (mid-typed) regex is a no-op with no highlight, not an error.
    pub fn update_incsearch(&mut self) {
        let Some((ol, oc, otop, owrap)) = self.search_origin else {
            return;
        };
        let forward = matches!(self.mode, Mode::Command(CommandKind::SearchFwd));
        let pat = self.cmdline.clone();
        let restore = |ed: &mut Editor| {
            ed.set_cursor(ol, oc);
            ed.buf_mut().top_line = otop;
            ed.buf_mut().top_wrap = owrap;
        };
        if pat.is_empty() {
            self.incsearch = None;
            restore(self);
            return;
        }
        let from = self.buf().char_idx(ol, oc);
        match self.find_search(&pat, from, forward) {
            Ok(Some(idx)) => {
                let (l, c) = self.buf().pos_from_char_idx(idx);
                self.incsearch = Some(pat);
                self.set_cursor(l, c);
            }
            Ok(None) => {
                // Valid pattern, no match: highlight nothing, stay at origin.
                self.incsearch = Some(pat);
                restore(self);
            }
            Err(_) => {
                // Half-typed / invalid regex: no highlight, stay put.
                self.incsearch = None;
                restore(self);
            }
        }
    }

    /// Cancel an in-progress `/`/`?` search: restore the origin and clear the
    /// preview highlight (keeps any prior hlsearch intact).
    pub fn cancel_incsearch(&mut self) {
        if let Some((ol, oc, otop, owrap)) = self.search_origin.take() {
            self.set_cursor(ol, oc);
            self.buf_mut().top_line = otop;
            self.buf_mut().top_wrap = owrap;
        }
        self.incsearch = None;
    }

    pub fn set_message<S: Into<String>>(&mut self, msg: S) {
        let msg = msg.into();
        // Record a bounded, consecutive-deduped history for `:messages`.
        if !msg.is_empty() && self.messages.last().map(String::as_str) != Some(msg.as_str()) {
            self.messages.push(msg.clone());
            if self.messages.len() > 500 {
                let drop = self.messages.len() - 500;
                self.messages.drain(0..drop);
            }
        }
        self.message = msg;
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
