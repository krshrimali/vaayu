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

/// Node kinds whose multi-line extents `:foldsyntax` collapses into folds --
/// functions plus the common class/impl/module container kinds.
pub(crate) const FOLD_KINDS: &[&str] = &[
    "function_item",
    "function_declaration",
    "function_definition",
    "method_declaration",
    "method_definition",
    "function",
    "arrow_function",
    "function_expression",
    "impl_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "mod_item",
    "class_declaration",
    "class_definition",
    "interface_declaration",
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
    /// Cached rainbow bracket positions `(line, col, depth)` for `(buffer,
    /// edit_seq)` — recomputed on edit. See `src/render.rs` `rainbow_brackets`.
    #[allow(clippy::type_complexity)]
    pub rainbow_cache:
        std::cell::RefCell<Option<(u64, u64, std::rc::Rc<Vec<(usize, usize, u8)>>)>>,
    pub preview_panes: std::cell::RefCell<HashMap<u64, crate::markdown::Preview>>,
    pub snippet: Option<crate::snippet::Session>,
    pub word_index: Option<crate::completion::WordIndex>,
    pub recent_files: Vec<PathBuf>,
    pub insert_repeat: usize,
    pub insert_start: usize,
    /// Set when the pending Insert session was opened by `O` on the very first
    /// line (buffer start): its repeat can't be newline-first (there is no
    /// preceding newline to anchor on), so on leaving Insert the whole opened
    /// line -- including its trailing newline -- is prepended `count-1` times.
    pub insert_open_bof: bool,
    pub block_insert: Option<(usize, usize, usize)>,
    pub visual_repeat: Option<(VisualKind, usize, usize, crate::operator::OperatorKind)>,
    pub notes: crate::notes::Notes,
    pub results: Option<crate::results::Results>,
    /// Set when the command line was opened (`:`) from within a Results-panel
    /// overlay, so `:q`/`:quit` dismisses the panel (returns to the buffer)
    /// instead of quitting the editor. Consumed by the next `run_ex`.
    pub cmdline_over_results: bool,
    pub quickfix: Option<crate::results::Results>,
    /// Per-buffer location lists — each buffer has its own independent
    /// quickfix-like list (`:lopen`/`:lnext`/`:lprev`), keyed by buffer id, so
    /// `:ldiagnostics`/`:lgrep` in one buffer don't clobber another's.
    pub loclists: std::collections::HashMap<u64, crate::results::Results>,
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
    /// Conceal rules compiled once from `config.conceal_rules`: each is a regex
    /// and either `Some(cchar)` (replace a match with that char) or `None`
    /// (hide the match). Applied at render time when `config.conceal` is on.
    pub conceal_compiled: Vec<(regex::Regex, Option<char>)>,
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
    /// Set while a format-on-save pump is waiting for the LSP format response;
    /// the `format` result arm clears it so the save can proceed.
    pub format_pending: bool,
    /// Inline ghost-text suggestion: `(line, col, text)` shown dimmed after the
    /// cursor and accepted with Tab. Recomputed by `update_ghost`.
    pub ghost: Option<(usize, usize, String)>,
    /// Task watch mode: a command re-run into the quickfix on every save
    /// (`:taskwatch`), or `None` when off.
    pub watch_task: Option<String>,
    /// inccommand: while typing a `:s`/`:%s` substitute, the live replacement
    /// preview — line index -> the text that line would become. Rendered as an
    /// overlay; empty when the command line isn't a valid substitute.
    pub sub_preview: std::collections::HashMap<usize, String>,
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
    /// Path (per `Layout`) to the split whose divider a mouse drag is currently
    /// resizing, set on mouse-down over a divider and cleared on button-up.
    pub resize_drag: Option<Vec<bool>>,
    /// Lazily loaded on first spell-check use, not at startup: reading a
    /// system word list (hundreds of KB to a few MB) on every launch --
    /// and every test's `Editor::new`, of which there are many -- for a
    /// feature most sessions never touch would be wasted work.
    pub dictionary: Option<crate::spell::Dictionary>,
    /// A rename WorkspaceEdit awaiting confirmation (`refactor_preview` on): the
    /// raw edit plus the request context to validate against at apply time.
    /// `:renameapply` applies it; `:renamecancel` (or a new preview) drops it.
    pub pending_rename: Option<(serde_json::Value, crate::language::RequestContext)>,
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
    /// LSP-symbol fallback for sticky scroll when the buffer has no tree-sitter
    /// grammar: enclosing container `(start_line, end_line)` ranges from
    /// `textDocument/documentSymbol`, sorted by start. The tree-sitter path is
    /// always preferred when a grammar is available.
    pub sticky_symbols: Vec<(usize, usize)>,
    pub sticky_symbols_buffer: Option<u64>,
    pub sticky_symbols_edit_seq: u64,
    /// `(buffer id, edit_seq)` of the last sticky documentSymbol request, so it
    /// is issued at most once per edit instead of on every frame.
    pub sticky_request: Option<(u64, u64)>,
    /// LSP semantic tokens as `(line, start_col, end_col, palette_index)`
    /// spans, gated on `semantic_tokens_buffer`/`_edit_seq`. Palette index is
    /// derived from the token type name (see `render::semantic_color`).
    /// Decoded semantic tokens: `(line, start_col, end_col, palette, deprecated,
    /// readonly)` — from the token's modifier bitmask, `deprecated` draws it
    /// struck-through and `readonly` draws it italic.
    pub semantic_tokens: Vec<(usize, usize, usize, u8, bool, bool)>,
    pub semantic_tokens_buffer: Option<u64>,
    pub semantic_tokens_edit_seq: u64,
    /// The `(buffer, edit_seq)` a semantic-tokens request was last sent for,
    /// so sync_lsp issues at most one request per edit.
    pub semantic_requested_seq: Option<(u64, u64)>,
    /// `(buffer id, resultId)` from the last semantic-tokens response, so the
    /// next request can be a `full/delta` (sending `previousResultId`) when the
    /// server supports it — falling back to a full request otherwise.
    pub semantic_result: Option<(u64, String)>,
    /// The last full semantic-token data stream (the flat 5-tuple `u64` array)
    /// for `semantic_result`'s buffer, so a delta response's edits can be
    /// spliced into it before decoding.
    pub semantic_raw: Vec<u64>,
    /// Live spell-check underline spans `(line, start_col, end_col)`, gated on
    /// `spell_spans_buffer`/`_edit_seq`. Recomputed by `update_spell_spans`.
    pub spell_spans: Vec<(usize, usize, usize)>,
    pub spell_spans_buffer: Option<u64>,
    pub spell_spans_edit_seq: u64,
    /// Inline TODO-comment highlight spans: `(line, start_col, end_col,
    /// color_index)`, recomputed by `update_todo_spans` on a
    /// `(buffer, edit_seq)` stamp. Empty when `todo_highlight` is off.
    pub todo_spans: Vec<(usize, usize, usize, u8)>,
    pub todo_spans_buffer: Option<u64>,
    pub todo_spans_edit_seq: u64,
    /// Injected-language highlight spans (byte ranges) for embedded code, e.g.
    /// a ```rust fence in Markdown, recomputed by `update_injections` on a
    /// `(buffer, edit_seq)` stamp.
    pub injection_spans: Vec<(usize, usize, crate::syntax::HlClass)>,
    pub injection_buffer: Option<u64>,
    pub injection_edit_seq: u64,
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
    /// `:make`/`:task` background command result → quickfix. See `src/task.rs`.
    pub make_task: Option<crate::git_tools::GitTask>,
    /// The replacement text stashed by `:linkededit` while its async
    /// `linkedEditingRange` request is in flight; applied to every returned
    /// range when the response arrives.
    pub pending_linked_edit: Option<String>,
    /// Set while a `:linkededit` (no name) live-editing request is in flight, so
    /// its `linkedEditingRange` response starts a live-mirroring session instead
    /// of a one-shot rename.
    pub pending_linked_live: bool,
    /// The active code tour `(tour, step index)`, if `:tour` is running.
    pub active_tour: Option<(crate::tour::Tour, usize)>,
    /// In-progress `:tournew` draft `(name, buffer id)`: the scratch buffer the
    /// user is describing a tour in, which `:toursave` sends to Claude.
    pub tour_draft: Option<(String, u64)>,
    /// A prompt/instruction queued for a *just-spawned* AI sidebar
    /// `(terminal id, text, spawned_at, start_revision)`: delivered by
    /// `flush_pending_agent_send` once the CLI's TUI has started, so the paste
    /// doesn't race its initialization and land garbled.
    pub pending_agent_send: Option<(u64, String, std::time::Instant, u64)>,
    /// Transient toast notifications `(shown_at, text)` — mirrors of recent
    /// messages, shown top-right when `config.notifications` is on.
    pub toasts: Vec<(Instant, String)>,
    /// Zen/focus mode: hide the gutter and per-pane status line. `:zen` toggles.
    pub zen: bool,
    /// Active syntax colorscheme (`:colorscheme`). See `src/theme.rs`.
    pub theme: crate::theme::Theme,
    /// Buffers marked for diff mode (`:diffthis`); the first two are compared.
    pub diff_buffers: Vec<u64>,
    /// Per-buffer differing line numbers, recomputed by `update_diff`.
    pub diff_lines: HashMap<u64, std::collections::HashSet<usize>>,
    /// Staleness stamp `(a_id, a_seq, b_id, b_seq)` for the last diff.
    pub diff_stamp: Option<(u64, u64, u64, u64)>,
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
        let theme = crate::theme::builtin(&config.colorscheme).unwrap_or_default();
        // Compile the conceal rules once (invalid patterns are skipped). Each
        // rule maps to `Some(cchar)` (its first char) or `None` (hide entirely).
        let conceal_compiled = config
            .conceal_rules
            .iter()
            .filter(|r| !r.pattern.is_empty())
            .filter_map(|r| {
                regex::Regex::new(&r.pattern)
                    .ok()
                    .map(|re| (re, r.cchar.chars().next()))
            })
            .collect();
        Editor {
            conceal_compiled,
            keymaps,
            project_root,
            review_job: None,
            review_results: None,
            notes,
            recovery: Default::default(),
            layout_cache: Default::default(),
            rainbow_cache: std::cell::RefCell::new(None),
            preview_panes: Default::default(),
            snippet: None,
            word_index: None,
            recent_files: Vec::new(),
            insert_repeat: 1,
            insert_start: 0,
            insert_open_bof: false,
            block_insert: None,
            visual_repeat: None,
            results: None,
            cmdline_over_results: false,
            quickfix: None,
            loclists: std::collections::HashMap::new(),
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
            format_pending: false,
            ghost: None,
            watch_task: None,
            sub_preview: std::collections::HashMap::new(),
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
            resize_drag: None,
            dictionary: None,
            pending_rename: None,
            terminals: Vec::new(),
            file_tree: None,
            outline: None,
            document_highlights: Vec::new(),
            document_highlights_buffer: None,
            document_highlights_edit_seq: 0,
            document_colors: Vec::new(),
            document_colors_buffer: None,
            document_colors_edit_seq: 0,
            sticky_symbols: Vec::new(),
            sticky_symbols_buffer: None,
            sticky_symbols_edit_seq: 0,
            sticky_request: None,
            semantic_tokens: Vec::new(),
            semantic_tokens_buffer: None,
            semantic_tokens_edit_seq: 0,
            semantic_requested_seq: None,
            semantic_result: None,
            semantic_raw: Vec::new(),
            spell_spans: Vec::new(),
            spell_spans_buffer: None,
            spell_spans_edit_seq: 0,
            todo_spans: Vec::new(),
            todo_spans_buffer: None,
            todo_spans_edit_seq: 0,
            injection_spans: Vec::new(),
            injection_buffer: None,
            injection_edit_seq: 0,
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
            make_task: None,
            pending_linked_edit: None,
            pending_linked_live: false,
            active_tour: None,
            tour_draft: None,
            pending_agent_send: None,
            toasts: Vec::new(),
            zen: false,
            theme,
            diff_buffers: Vec::new(),
            diff_lines: HashMap::new(),
            diff_stamp: None,
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
    /// True when the current buffer is in "large-file mode" (over
    /// `config.large_file_kb`), so expensive per-buffer scans are skipped.
    pub fn buf_is_large(&self) -> bool {
        self.config.large_file_kb > 0
            && self.buf().rope.len_bytes() > self.config.large_file_kb.saturating_mul(1024)
    }

    pub fn ensure_syntax(&mut self) {
        // Large-file mode: skip tree-sitter entirely (drop any existing tree).
        if self.buf_is_large() {
            if self.syntax.is_some() {
                self.syntax = None;
                self.syntax_seq = None;
                self.syntax_pending = None;
            }
            return;
        }
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
        self.feed_key_inner(key);
        // Marks, the jumplist, and background windows' cached cursors live on
        // the editor, out of reach of the buffer's edit primitives; shift them
        // for any line-count change this key produced (drains every buffer's
        // pending shifts, so cross-buffer LSP edits are covered too).
        self.apply_pending_line_shifts();
    }

    fn feed_key_inner(&mut self, key: Key) {
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

        // In Terminal mode these are raw control bytes for the child (emacs
        // C-s, nano save, flow control); don't let the editor steal them (and
        // Ctrl-S would otherwise save the pane's placeholder buffer).
        if !matches!(self.mode, Mode::Terminal) {
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

    /// Seed dot-repeat recording with a multi-key operator prefix (e.g. `gq`)
    /// whose earlier keys were already consumed before recording could start.
    /// Mirrors `start_change_recording`'s register/count prefix.
    pub fn start_change_recording_seq(&mut self, keys: &[Key]) {
        if !self.replaying {
            self.recording_change = true;
            self.visual_repeat = None;
            self.cmd_keys = Vec::new();
            if let Some(r) = self.pending.register {
                self.cmd_keys.extend([Key::Char('"'), Key::Char(r)]);
            }
            let n = self.pending.total_count();
            if n > 1 {
                self.cmd_keys.extend(n.to_string().chars().map(Key::Char));
            }
            self.cmd_keys.extend_from_slice(keys);
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
        // A paste while focused on a terminal pane belongs to the child, not to
        // the pane's underlying (real) buffer -- forward it and stop, so a paste
        // never silently edits a file behind the terminal. Guard on the actual
        // terminal focus, not `mode`, so a paste one tick after Esc is still safe.
        if let Some(id) = self.active_terminal_id() {
            if let Some(pty) = self.terminals.iter_mut().find(|p| p.id == id) {
                pty.write_pasted_input(text);
            }
            return;
        }
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
        let trimmed = prefix.trim_end();
        if trimmed.ends_with(['{', '(', '[']) {
            format!("{base}{}", self.indent_unit())
        } else if self.buf_is_python() && trimmed.ends_with(':') {
            // Python block opener (`def`/`if`/`for`/`while`/`class`/… `:`) —
            // the tree-sitter grammar has no brackets to key off, so the
            // colon drives the extra indent level.
            format!("{base}{}", self.indent_unit())
        } else if self.buf_is_python() && Self::python_dedent_keyword(trimmed) {
            // A statement that ends the current suite (`return`/`pass`/`raise`/
            // `break`/`continue`) — the next line dedents one level.
            self.dedent_one(&base)
        } else {
            base
        }
    }

    /// Remove one indent level from the end of a leading-whitespace string
    /// (a single trailing tab, else up to `shiftwidth` trailing spaces).
    fn dedent_one(&self, base: &str) -> String {
        if let Some(stripped) = base.strip_suffix('\t') {
            stripped.to_string()
        } else {
            let sw = self.buf().shiftwidth.max(1);
            let drop = base.chars().rev().take_while(|c| *c == ' ').count().min(sw);
            base[..base.len() - drop].to_string()
        }
    }

    /// Whether a (trimmed) Python line is a suite-ending statement — `return`,
    /// `pass`, `raise`, `break`, or `continue` as a whole word — after which the
    /// next line dedents one level.
    fn python_dedent_keyword(line: &str) -> bool {
        let t = line.trim();
        ["return", "pass", "raise", "break", "continue"].iter().any(|kw| {
            t.strip_prefix(kw)
                .is_some_and(|rest| rest.is_empty() || !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        })
    }

    /// Whether the current buffer is a Python file (by extension), for
    /// language-specific indent rules that brackets don't cover.
    fn buf_is_python(&self) -> bool {
        self.buf()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_lowercase().as_str(), "py" | "pyi"))
    }

    /// Vim-style smartindent electric dedent: when a closing bracket
    /// (`}`/`)`/`]`) is typed as the first non-blank character on its line,
    /// strip one indent level from the line's leading whitespace so the bracket
    /// lines up with the block it closes. Returns true if it dedented; the
    /// caller still inserts the bracket. No-op unless `smartindent` is on.
    pub fn electric_dedent(&mut self, c: char) -> bool {
        if !self.config.smartindent || !matches!(c, '}' | ')' | ']') {
            return false;
        }
        let (line, col) = self.cursor();
        if col == 0 {
            return false;
        }
        let text = self.buf().line_text(line);
        // Only when everything before the cursor on this line is whitespace.
        if !text.chars().take(col).all(|c| c == ' ' || c == '\t') {
            return false;
        }
        // Remove one indent unit from the front: a leading tab, else up to
        // `shiftwidth` leading spaces.
        let removed = if text.starts_with('\t') {
            1
        } else {
            let spaces = text.chars().take_while(|c| *c == ' ').count();
            spaces.min(self.buf().shiftwidth.max(1))
        };
        if removed == 0 {
            return false;
        }
        let start = self.buf().char_idx(line, 0);
        let end = self.buf().char_idx(line, removed);
        self.buf_mut().delete_char_range(start, end);
        self.set_cursor_insert(line, col - removed);
        true
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
        // Translate the Vim-dialect query into the PCRE form `find_search` /
        // the render highlight expect, exactly as `run_search` does -- otherwise
        // the live preview matches (and highlights) differently from the search
        // that runs on Enter for patterns like `\(grp\)`, `\+` or `\<word\>`.
        let pat = crate::vimregex::translate_pattern(&self.cmdline);
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

    /// Recompute `todo_spans` — TODO/FIXME/etc. keyword ranges inside comments —
    /// for the current buffer when `todo_highlight` is on and the cache is
    /// stale. Uses the tree-sitter Comment spans so keywords in code/strings
    /// aren't matched; whole-word matches only.
    pub fn update_todo_spans(&mut self) {
        if !self.config.todo_highlight || self.buf_is_large() {
            if !self.todo_spans.is_empty() {
                self.todo_spans.clear();
                self.todo_spans_buffer = None;
            }
            return;
        }
        let id = self.buf().id;
        let seq = self.buf().edit_seq;
        if self.todo_spans_buffer == Some(id) && self.todo_spans_edit_seq == seq {
            return;
        }
        const KEYWORDS: &[(&str, u8)] = &[
            ("TODO", 0),
            ("NOTE", 0),
            ("FIXME", 1),
            ("BUG", 1),
            ("XXX", 1),
            ("HACK", 2),
            ("WARNING", 2),
        ];
        let mut spans = Vec::new();
        if let Some(syn) = &self.syntax {
            let b = self.buf();
            let total = b.rope.len_bytes();
            for (s, e, class) in syn.spans_in(0, total) {
                if !matches!(class, crate::syntax::HlClass::Comment) {
                    continue;
                }
                let e = e.min(total);
                let text = b.rope.byte_slice(s..e).to_string();
                let bytes = text.as_bytes();
                for &(kw, cidx) in KEYWORDS {
                    let mut from = 0;
                    while let Some(pos) = text[from..].find(kw) {
                        let at = from + pos;
                        let before_ok = at == 0
                            || !(bytes[at - 1] as char).is_ascii_alphanumeric()
                                && bytes[at - 1] != b'_';
                        let after = at + kw.len();
                        let after_ok = after >= bytes.len()
                            || !(bytes[after] as char).is_ascii_alphanumeric()
                                && bytes[after] != b'_';
                        if before_ok && after_ok {
                            let ci = b.rope.byte_to_char(s + at);
                            let (line, col) = b.pos_from_char_idx(ci);
                            spans.push((line, col, col + kw.chars().count(), cidx));
                        }
                        from = at + kw.len();
                    }
                }
            }
        }
        self.todo_spans = spans;
        self.todo_spans_buffer = Some(id);
        self.todo_spans_edit_seq = seq;
    }

    /// Recompute the inline ghost-text suggestion: when `ghost_text` is on, the
    /// cursor is at a line's end in Insert mode with no completion popup up, and
    /// another buffer line starts with the current line, suggest that line's
    /// remainder. A simple, local, deterministic "Copilot-style" provider.
    pub fn update_ghost(&mut self) {
        self.ghost = None;
        if !self.config.ghost_text || !matches!(self.mode, Mode::Insert) || self.buf_is_large() {
            return;
        }
        let (line, col) = self.cursor();
        let cur = self.buf().line_text(line);
        // End of line, with a non-trivial prefix.
        if col != cur.chars().count() || cur.trim().len() < 3 {
            return;
        }
        let n = self.buf().line_count().min(10_000);
        for l in 0..n {
            if l == line {
                continue;
            }
            let other = self.buf().line_text(l);
            if other.len() > cur.len() && other.starts_with(&cur) {
                self.ghost = Some((line, col, other[cur.len()..].to_string()));
                return;
            }
        }
    }

    /// Accept the current ghost-text suggestion (Tab in Insert mode): insert it
    /// at the cursor and advance. No-op if the cursor moved off the suggestion.
    pub fn accept_ghost(&mut self) {
        if let Some((line, col, text)) = self.ghost.take() {
            if self.cursor() == (line, col) {
                self.buf_mut().insert_str(line, col, &text);
                let newcol = col + text.chars().count();
                self.set_cursor_insert(line, newcol);
            }
        }
    }

    /// Recompute injected-language highlight spans for the current buffer:
    /// Markdown fenced code blocks (```lang … ```) are parsed with the embedded
    /// language's grammar and their spans offset into the buffer. Stamped on
    /// `(buffer, edit_seq)`; a no-op for non-Markdown or large-file buffers.
    pub fn update_injections(&mut self) {
        let is_md = self
            .buf()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"));
        if !is_md || self.buf_is_large() {
            if !self.injection_spans.is_empty() {
                self.injection_spans.clear();
                self.injection_buffer = None;
            }
            return;
        }
        let id = self.buf().id;
        let seq = self.buf().edit_seq;
        if self.injection_buffer == Some(id) && self.injection_edit_seq == seq {
            return;
        }
        let text = self.buf().rope.to_string();
        let mut spans = Vec::new();
        let mut byte = 0usize;
        // (embedded lang, content-start byte) while inside an open fence.
        let mut fence: Option<(Option<crate::syntax::Lang>, usize)> = None;
        for line in text.split_inclusive('\n') {
            let is_fence = line.trim_start().starts_with("```");
            if is_fence {
                match fence.take() {
                    None => {
                        let info = line.trim_start().trim_start_matches('`').trim();
                        let name = info.split_whitespace().next().unwrap_or("");
                        let lang = crate::syntax::lang_for_fence(name);
                        fence = Some((lang, byte + line.len()));
                    }
                    Some((Some(lang), content_start)) => {
                        if content_start <= byte {
                            if let Some(mut syn) = crate::syntax::Syntax::new(lang) {
                                let content = &text[content_start..byte];
                                syn.reparse(std::rc::Rc::from(content));
                                for (s, e, class) in syn.spans_in(0, content.len()) {
                                    spans.push((content_start + s, content_start + e, class));
                                }
                            }
                        }
                    }
                    Some((None, _)) => {} // fence with an unknown/absent language
                }
            }
            byte += line.len();
        }
        spans.sort_by_key(|&(s, _, _)| s);
        self.injection_spans = spans;
        self.injection_buffer = Some(id);
        self.injection_edit_seq = seq;
    }

    /// If the cursor sits inside a closed fold (past its first row), snap it to
    /// the fold's first, still-visible line. The safety net for any motion that
    /// isn't itself fold-aware; a no-op when there are no closed folds.
    pub fn clamp_cursor_folds(&mut self) {
        let line = self.buf().cursor_line;
        if let Some(f) = self.buf().hidden_by_fold(line) {
            let col = self.buf().cursor_col;
            let c = self.buf().clamp_col_normal(f.start, col);
            let b = self.buf_mut();
            b.cursor_line = f.start;
            b.cursor_col = c;
        }
    }

    /// `:fold`/visual `zf`: create a closed fold over an inclusive line range.
    pub fn create_fold(&mut self, start: usize, end: usize) {
        let last = self.buf().line_count().saturating_sub(1);
        let s = start.min(end).min(last);
        let e = start.max(end).min(last);
        if e <= s {
            self.set_message("Need at least two lines to fold");
            return;
        }
        self.buf_mut().folds.push(crate::buffer::Fold {
            start: s,
            end: e,
            closed: true,
        });
        self.buf_mut().cursor_line = s;
        self.set_message(format!("Folded {} lines", e - s + 1));
    }

    /// Index of the innermost fold containing `line`, if any.
    fn fold_at(&self, line: usize) -> Option<usize> {
        self.buf()
            .folds
            .iter()
            .enumerate()
            .filter(|(_, f)| f.start <= line && line <= f.end)
            .min_by_key(|(_, f)| f.end - f.start)
            .map(|(i, _)| i)
    }

    /// `za` -- toggle the innermost fold under the cursor open/closed.
    pub fn toggle_fold(&mut self) {
        match self.fold_at(self.cursor().0) {
            Some(i) => {
                let b = self.buf_mut();
                b.folds[i].closed = !b.folds[i].closed;
                if b.folds[i].closed {
                    b.cursor_line = b.folds[i].start;
                }
            }
            None => self.set_message("No fold under cursor"),
        }
    }

    /// `zo` -- open the innermost fold under the cursor.
    pub fn open_fold(&mut self) {
        match self.fold_at(self.cursor().0) {
            Some(i) => self.buf_mut().folds[i].closed = false,
            None => self.set_message("No fold under cursor"),
        }
    }

    /// `zc` -- close the innermost fold under the cursor.
    pub fn close_fold(&mut self) {
        match self.fold_at(self.cursor().0) {
            Some(i) => {
                let b = self.buf_mut();
                b.folds[i].closed = true;
                b.cursor_line = b.folds[i].start;
            }
            None => self.set_message("No fold under cursor"),
        }
    }

    /// `zd` -- delete the innermost fold under the cursor.
    pub fn delete_fold(&mut self) {
        match self.fold_at(self.cursor().0) {
            Some(i) => {
                self.buf_mut().folds.remove(i);
                self.set_message("Fold deleted");
            }
            None => self.set_message("No fold under cursor"),
        }
    }

    /// `:foldindent` -- replace the fold set with indentation-based folds (each
    /// line that heads a more-indented block becomes a closed fold spanning it),
    /// collapsing the buffer to a nested overview. Blank lines are absorbed into
    /// the surrounding block. A no-op message when nothing is foldable.
    pub fn fold_by_indent(&mut self) {
        let b = self.buf();
        let n = b.line_count();
        let level = |line: usize| -> Option<usize> {
            let t = b.line_text(line);
            if t.trim().is_empty() {
                return None; // blank line: no indent level of its own
            }
            let mut w = 0;
            for c in t.chars() {
                match c {
                    ' ' => w += 1,
                    '\t' => w += b.tabstop.max(1),
                    _ => break,
                }
            }
            Some(w)
        };
        let mut folds = Vec::new();
        for i in 0..n {
            let Some(cur) = level(i) else { continue };
            let mut j = i + 1;
            let mut deeper = false;
            while j < n {
                match level(j) {
                    Some(l) if l > cur => {
                        deeper = true;
                        j += 1;
                    }
                    Some(_) => break,      // dedent ends the block
                    None => j += 1,        // blank line: keep scanning
                }
            }
            if deeper {
                let mut end = j - 1;
                while end > i && level(end).is_none() {
                    end -= 1; // trim trailing blank lines out of the fold
                }
                if end > i {
                    folds.push(crate::buffer::Fold {
                        start: i,
                        end,
                        closed: true,
                    });
                }
            }
        }
        if folds.is_empty() {
            self.set_message("No indented blocks to fold");
            return;
        }
        let count = folds.len();
        let b = self.buf_mut();
        b.folds = folds;
        b.cursor_line = 0;
        b.cursor_col = 0;
        self.set_message(format!("Created {count} indent fold(s)"));
    }

    /// `:foldsyntax` -- fold every function/class/module extent from the
    /// tree-sitter tree (nodes spanning more than one line), collapsing the
    /// buffer to a structural overview. Falls back to a message when there is
    /// no parsed tree or nothing multi-line to fold.
    pub fn fold_by_syntax(&mut self) {
        let Some(syn) = self.syntax.as_ref() else {
            self.set_message("No tree-sitter tree for this buffer");
            return;
        };
        let ranges = syn.node_ranges(crate::editor::FOLD_KINDS);
        let b = self.buf();
        let total_bytes = b.rope.len_bytes();
        let last = b.line_count().saturating_sub(1);
        let mut folds: Vec<crate::buffer::Fold> = Vec::new();
        for (sb, eb) in ranges {
            let sl = b.pos_from_char_idx(b.rope.byte_to_char(sb.min(total_bytes))).0;
            let el = b
                .pos_from_char_idx(b.rope.byte_to_char(eb.saturating_sub(1).min(total_bytes)))
                .0
                .min(last);
            if el > sl && !folds.iter().any(|f| f.start == sl && f.end == el) {
                folds.push(crate::buffer::Fold {
                    start: sl,
                    end: el,
                    closed: true,
                });
            }
        }
        if folds.is_empty() {
            self.set_message("Nothing to fold");
            return;
        }
        folds.sort_by_key(|f| (f.start, f.end));
        let count = folds.len();
        let b = self.buf_mut();
        b.folds = folds;
        b.cursor_line = 0;
        b.cursor_col = 0;
        self.set_message(format!("Created {count} syntax fold(s)"));
    }

    /// `zR` -- open every fold.
    pub fn open_all_folds(&mut self) {
        for f in &mut self.buf_mut().folds {
            f.closed = false;
        }
    }

    /// `zM` -- close every fold.
    pub fn close_all_folds(&mut self) {
        for f in &mut self.buf_mut().folds {
            f.closed = true;
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
        self.sub_preview.clear();
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
            // With notifications on, mirror new messages as transient toasts.
            if self.config.notifications {
                self.toasts.push((Instant::now(), msg.clone()));
                let excess = self.toasts.len().saturating_sub(8);
                if excess > 0 {
                    self.toasts.drain(0..excess);
                }
            }
        }
        self.message = msg;
    }

    /// Whether a toast has expired but is still stored — the idle loop uses
    /// this to trigger one redraw (after `prune_toasts`) so it visibly fades.
    pub fn has_expired_toast(&self) -> bool {
        self.toasts
            .iter()
            .any(|(t, _)| t.elapsed() >= std::time::Duration::from_secs(4))
    }

    /// Drops expired toasts.
    pub fn prune_toasts(&mut self) {
        self.toasts
            .retain(|(t, _)| t.elapsed() < std::time::Duration::from_secs(4));
    }

    /// Called by the main loop when the jk-escape timeout elapses with no
    /// follow-up key: the buffered 'j' becomes a literal character.
    pub fn flush_pending_jk(&mut self) {
        if self.pending_jk.take().is_none() {
            return;
        }
        if matches!(self.mode, Mode::Insert) {
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
        } else if matches!(self.mode, Mode::Terminal) {
            // A buffered jk-escape `j` that timed out without a `k`: send it to
            // the terminal child so a lone `j` isn't swallowed.
            if let Some(id) = self.active_terminal_id() {
                if let Some(pty) = self.terminals.iter_mut().find(|p| p.id == id) {
                    pty.write_input(b"j");
                }
            }
        }
    }
}
