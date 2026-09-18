Vaayu — editing and review

Save: Ctrl-S or ,w or :w. Quit: :q (checks unsaved work); :qa! discards.
Normal / Insert / Visual use Vim operators and motions. jk exits Insert.

PRIVATE REVIEW COMMENTS
,rc or :comment         Comment on current line / visual line range
,rf or :commentfile     Comment on the entire current file
,rl or :comments        Browse private project comments
Ctrl-S / :w            Save the open comment (ordinary editable buffer)
,rw / :commentswrite   Save comment removals and relocated anchors
Comments live in .vaayu/comments.json, excluded from Git; source files stay clean.
In comments: e edit · Enter visit source · d delete selected · Tab/Space select
              y copy selected/current · Y copy all · a toggle all

RESULTS AND QUICKFIX
Ctrl-Q                 Send current picker/results/output to quickfix
,cq or :copen          Reopen quickfix
:cnext / :cprev        Next / previous quickfix location
:colder / :cnewer      Switch to the previous / next quickfix list
                       (Ctrl-Q appends a new one; browsing/dismissing
                       the current list doesn't)
/ and ?                Search results (regex), Enter submit, n/N repeat
j/k / arrows           Move · Ctrl-D/U page · g/G first/last
Tab/Space              Select · a select all/none · y copy selected · Y copy all
Enter                  Open location / apply selected code action
p                      Toggle a file-content preview pane around the
                       current entry's line (for entries with a path)
w                      While previewing: toggle wrapping long source lines
Ctrl-E / Ctrl-Y        While previewing: scroll the preview down/up
f                      Filter the list by a substring (case-insensitive,
                       matches text or detail); Enter/Esc keeps it applied
                       while browsing, a fresh f clears it. Not for a live
                       grep list, which already replaces entries itself.
q / Esc                Return to editing

PROJECT NAVIGATION
Ctrl-P / ,ff           File picker · Ctrl-Q sends its matches to quickfix
                       Ctrl-V/Ctrl-X open the selection into a new vertical/
                       horizontal split, in the picker and any results list.
                       Ctrl-T opens it into a new tab instead.
                       Status line shows "<shown>/<matched> files": a
                       bounded top-k matcher keeps only the best 500
                       ranked matches in flight rather than sorting every
                       candidate in the whole inventory per keystroke, so
                       <matched> can be far larger than <shown> on a big
                       project without a slower picker.
:resume                Reopens the file picker or Results/quickfix list
                       (whichever was dismissed more recently) exactly
                       as it was left: query, matches, cursor, selection.
,b / :buffer           Buffer list, most-recently-activated first
:blines                Every non-blank line in the current buffer as a
                       jump-to-line picker
:projects              Recently launched-from directories (most recent
                       first, current one excluded); Enter switches
                       project_root and drops any open file tree so
                       the next ,ft rebuilds it at the new root
:everything            Keymaps, commands and recent projects combined
                       into one list; f (filter) narrows across all of
                       them at once. Each entry behaves exactly like its
                       own :keymaps/:commands/:projects source would.
,/ / :grep pattern     Live grep (ripgrep); i edits query, Enter navigates results
,gw                    Live grep the word under cursor, or the Visual
                       selection (Char/Line); Visual-block falls back to
                       the word under cursor.
ma                     Set mark a · 'a line jump · `a exact jump
Ctrl-O / Ctrl-I        Jump backward / forward (Tab also moves forward)
Ctrl-6 / :b#           Toggle to the alternate (previously edited) buffer
/ ? n N                Search jumps recenter the match in the viewport,
                       like zz (Ctrl-D/Ctrl-U already did; Ctrl-F/Ctrl-B
                       intentionally do not, matching Vim's full-page scroll)
[d / ]d                Previous / next diagnostic in current file
[c / ]c                Previous / next changed git hunk (wraps around;
                       a multi-line hunk is one stop, not one per line)

LANGUAGE SERVER
K                      Hover
 gd                    Definition (result list retained for Ctrl-Q)
gy / :typedefinition   Type definition
gI / :implementation   Implementation
gD / :declaration      Declaration
,lw / :workspacesymbols name  Workspace symbol search (picker, no auto-jump)
,lo / :outline         Document symbols
,lR / :references      References
,lh                     Highlight other occurrences of the symbol under
                       the cursor in this buffer; Esc clears it, or an
                       edit makes it stale and it stops painting
,ll / :documentlinks   List document links; Enter opens a file:// link
                       or copies any other link (never opens a browser)
,lc / :codelens        List code lenses (a lens with no command, deferred
                       to codeLens/resolve, is skipped); Enter runs one.
                       Also shown as "» title" virtual text after each
                       lens's own line
,li / :inlayhints      Show inlay hints inline, spliced into each line at
                       their own position among the real characters (not
                       appended after it, unlike blame/code-lens text);
                       Esc clears them along with document highlights
,ld / :diagnostics     Shared diagnostics list, with a "[source(code)]"
                       label when the server sends one, and a "↳ ..."
                       entry right after any relatedInformation (its own
                       jumpable location). A diagnostic's own column
                       range is underlined in-buffer (red/yellow/blue by
                       severity), and its message shows as virtual text
                       on the cursor's own line
diagnostics_virtual_text=false in config.toml turns off that virtual
                       text tail (the underline and gutter marker are
                       unaffected)
diagnostics_update_in_insert=true makes new diagnostics update what's
                       shown immediately, even while still typing in
                       Insert mode (default false, matching Neovim: the
                       visible set freezes until Insert mode ends,
                       though it's still recorded meanwhile)
,lf / :format          Format buffer; in Visual mode formats just the
                       selected lines (textDocument/rangeFormatting)
                       instead. Save separately
,lr / :rename name     Rename across files; save separately
,la / :codeactions     Select code action, Enter applies. The server's
                       preferred action sorts first and is marked "* ";
                       a disabled one is still shown, with its reason,
                       but Enter on it refuses rather than applying
,lI / :organizeimports Apply the server's organize-imports action
                       directly (no picker -- there's normally just one)
,ls / :signature       Signature help
:lspinfo / :lsprestart Server status / restart
:tools                 Known language servers with health status (found
                       on PATH, and its version if so); Enter on one
                       that isn't installed runs its pinned install
                       command in a new terminal (opt-in -- nothing
                       installs without pressing Enter on it). Updating
                       is just re-running the same command; removing is
                       each ecosystem's own package manager's job
,R / :configreload    Reload TOML config and restart servers
LSP $/progress (e.g. rust-analyzer indexing) shows in the message line
                       as title, percentage and message; a later action
                       naturally overwrites it, like any other message.
                       It also shows in the active pane's status line
                       (recomputed fresh every frame from live state),
                       which a later message never hides.

WINDOWS AND DISPLAY
Ctrl-W v / :vsplit     Vertical split (optional file argument)
Ctrl-W s / :split      Horizontal split (optional file argument)
Ctrl-W w/h/j/k/l       Focus pane
Ctrl-W c / :close      Close pane
Ctrl-W o / :only       Keep active pane
,ms / :vpreview        Side-by-side Markdown preview
,mp                    Full-screen Markdown preview
:set wrap / nowrap     Soft wrapping / horizontal scrolling
,ow                    Toggle wrap

:keymaps               Searchable palette of every leader binding; Enter runs it
:commands              Searchable palette of every ex command; Enter fills the
                       command line with it (not run immediately -- most need
                       arguments), so add args and press Enter yourself
Pause after a leader prefix (e.g. ,l) to show a which-key popup of continuations
:indentinfo             Show this buffer's resolved tabstop/shiftwidth/style
                        and where it came from: modeline, .editorconfig,
                        detected (heuristic) or default (config.toml).
Configuration: ~/.config/vaayu/config.toml. See config.example.toml for LSP options.
whichkey_delay_ms (default 500) controls the which-key popup's pause delay.

EDITING AND RECOVERY
Outline sidebar        ,lO toggles a persistent symbol sidebar (LSP
                       documentSymbol, shown as a hierarchy by indentation).
                       j/k move · Enter/o jump into the other pane ·
                       h collapses a symbol with children (▾/▸ marks it) ·
                       l expands a collapsed one, else jumps like Enter ·
                       R refresh · f cycles a symbol-kind filter (all ->
                       each kind present -> back to all) · K hover the
                       symbol without navigating · q/Esc close.
                       Follow-cursor: while editing in the buffer pane, the
                       sidebar highlights the symbol enclosing the cursor
                       line automatically, with no keypress needed.
File tree              ,ft toggles a sidebar, revealing the current file.
                       j/k move · l/Enter/o open or expand · h collapse or
                       go to parent · G/Home/End · R refresh · q/Esc close.
                       a create · r rename · d d delete (two presses; any
                       other key cancels) · t t trash instead (moves into
                       .vaayu/trash/, same two-press confirm) · y copy ·
                       x cut · p paste into the cursor's directory
                       (recursive for a directory; refuses a name
                       collision; a directory copy that fails partway
                       through rolls back rather than leaving a partial
                       destination behind) · m toggles a bookmark (★), listed by
                       :treebookmarks (Enter opens a file, reveals a dir).
                       Refuses to rename/delete/trash/move a path an open
                       buffer has unsaved changes under.
                       / live-filters the currently loaded nodes by
                       substring (not a full project search -- only
                       already-expanded directories); Backspace narrows
                       back, Esc clears it, Enter keeps it and returns to
                       normal navigation.
                       A modified/added/untracked/etc. file shows its
                       git status letter (M/A/?/...); a directory with
                       any changed descendant shows *. Refreshed on open
                       and R, never live.
                       .gitignore'd paths are hidden by default (an
                       entirely-ignored directory collapses to one
                       hidden entry, never read into); ! shows them.
                       Dotfiles are hidden by default; . toggles them
                       (.git always stays hidden). A file or directory
                       with LSP diagnostics shows an E/W/I marker (a
                       collapsed directory shows its worst descendant's).
                       No gitignore filtering, live filter, git
                       decoration, or copy/cut/paste yet.
History                Up/Down (or Ctrl-P/Ctrl-N) in :/  ?  cycles through
                       previously submitted commands/searches, separately;
                       cycling back past the newest restores your draft.
                       In-memory only, not saved across restarts.
:jumps                 Jumplist as a results list; Enter jumps to the entry.
:chistory / :history   Command history as a results list; Enter reruns it.
:shistory              Search history as a results list; Enter reruns it.
Tabs                   gt/gT next/prev tab · {n}gt jumps to tab n
                       :tabnew :tabclose(:tabc) :tabonly(:tabo) :tabs
                       Tabline appears once a second tab exists. Each tab
                       keeps its own panes/cursor; closing one kills any
                       terminals running in it. Not saved across sessions.
Terminal               :terminal (:term) opens $SHELL in a real embedded
                       PTY in a new split, entering Terminal mode so typing
                       goes straight to the shell. Esc leaves to Normal for
                       pane navigation (Ctrl-W h/j/k/l, :close); i re-enters.
                       Closing the pane always kills the child process.
Spelling               :spellcheck lists misspelled words (results list);
                       zg adds the word under cursor to your dictionary;
                       z= shows/replaces with suggestions. Needs a system
                       word list (/usr/share/dict/words or similar);
                       degrades to a message if none is installed.
Mouse                  Click positions cursor and focuses the clicked pane;
                       drag selects (Visual); wheel scrolls the view;
                       Ctrl-click goes to definition. Requires terminal
                       mouse reporting; split-border drag-resize not done.
Persistent undo         :w saves undo history to .vaayu/undo/; u after a
                        restart on the same file restores it, unless the
                        file changed on disk since (checked by content hash).
Ctrl-A / Ctrl-X        Increment / decrement the next number on the line;
                       a count multiplies it; zero-padded width preserved.
,a                     Select entire buffer (Visual line-wise)
Visual > / <           Indents and keeps the selection, so repeated presses
                       (or a count) keep indenting the same block.
Align                  Visual ga{char} aligns selected lines on {char};
                       Normal gap{char} aligns the paragraph around cursor.
                       One undo step; lines without the delimiter untouched.
Subword motion         gw/gb/ge -- camelCase/snake_case/kebab-case aware
                       word motion; works bare, with operators (dgw) and in
                       Visual mode. Separators (_ - space) are gaps, like
                       whitespace; digits get their own subword.
Surround               ys{motion/textobj}{char} add · yss{char} whole line
                       ds{char} delete · cs{from}{to} change · Visual S{char}
                       e.g. ysiw" ds( cs"' -- ( [ { < pad when typed open;
                       b/B/r alias ( { [ . Same-line quotes, multi-line brackets.
Autopairs              Typing ( [ { " ' ` inserts the match; typing the close
                       again skips over it; Backspace on an empty pair deletes
                       both; Enter inside {}/()/[] expands an indented line.
                       Suppressed mid-word, after \ escapes, for Rust
                       lifetimes (&'a), and never applied to pasted text.
                       Toggle with autopairs=false in config.toml.
Ctrl-V                 Rectangular Visual selection; d/c/y/~ operate on block
:recover               Browse source drafts from interrupted sessions
Recovery snapshots run after idle time; restoring leaves a dirty, undoable buffer.

GIT REVIEW
:gitdiff               Current file's saved diff as navigable results
:gitblame              Current file's blame as navigable results
:gitstage              Saved unstaged hunks; Enter stages one hunk
:gitunstage            Staged hunks; Enter unstages one hunk
:gitstash              Stash list as navigable results; Enter shows a stash's diff
,gh                     Preview the diff for the saved hunk under the cursor
                       (read-only; doesn't stage or navigate away)
,gx                     Reset the saved hunk under the cursor: shows it as a
                       confirmation prompt, Enter discards it back to HEAD
                       in the working tree (never the index) and reloads
                       the open buffer to match; q/Esc cancels
:e! / :edit!           Discard in-memory changes and reload the current
                       buffer from disk (undo history is cleared too);
                       with a path, same as :e/:edit
,gB                     Toggle line-blame virtual text: the current line's
                       commit (short hash, author, date) after its own
                       text, computed asynchronously and follows the cursor
,gp / :permalink       Copy a GitHub permalink (pinned to HEAD's commit) for
                       the cursor line, or a Visual selection's line range
                       with ,gp. P on a :gitblame entry uses that line's own
                       commit instead of HEAD. Requires a github.com origin
                       remote; never opens a browser or touches the network.
Save the source before hunk actions. Ctrl-Q exports these lists to quickfix.

RELIABILITY AND EXTENDED EDITING
:lspcancel             Cancel outstanding language requests
LSP requests and initialization time out after request_timeout_ms (default 15000).
Completion acceptance resolves server details/extra edits when supported.
The popup shows a short kind label (fn, var, class, ...) for server items
that provide one, in place of the generic "lsp" source tag.
Typing a path-shaped prefix (contains a /) shows real filesystem entries
under that directory (tagged "path"), relative to the buffer's own folder;
directories sort first and keep a trailing / so you can keep descending.
The selected item shows a multi-line documentation preview below the list
when the server provides one, separate from the short inline detail text.
completion_enabled=false in config.toml turns off the automatic popup entirely.
completion_delay_ms (default 0) delays when the popup appears after a
keystroke; candidates are still computed immediately either way.
Snippets: Tab / Shift-Tab move placeholders; typing replaces defaults.
Linked fields update when leaving a placeholder. Esc finishes the snippet.
A placeholder nested inside another one's default (${1:foo ${2:bar}})
works like any other; typing over the outer one replaces the inner too.
${n|a,b,c|} choices: Ctrl-N / Ctrl-P cycle the current stop through the
list (wraps around) while still selected; typing replaces whichever
choice is showing, same as any other default.
Malformed or unsupported snippet syntax (an unclosed brace, a transform
like ${1/regex/fmt/}, which isn't implemented) degrades to the closest
plain-text reading instead of rejecting the whole completion.
Variables: TM_FILENAME, TM_FILENAME_BASE, TM_FILEPATH, TM_DIRECTORY,
TM_LINE_NUMBER, TM_CURRENT_LINE.
Search supports backreferences, lookaround, \v/\V/\m/\M and \c/\C.
Complex patterns have a backtracking limit; search/substitution report failures.
Syntax highlighting: Rust, Python, JavaScript/JSX, TypeScript/TSX, Go, C,
Bash, JSON, TOML, YAML, Lua, Vim, CSS, HTML, Solidity (by file extension;
comment/string/number classes are generic across all of them, keywords
are per-language). No dedicated grammar exists yet for Markdown (its own
block/inline grammar split doesn't fit this editor's one-parser-per-buffer
model) or Kitty's config format; both still open and edit normally, just
without syntax colors.
The JSON and YAML language servers get a bundled SchemaStore catalog
(well-known files like package.json, tsconfig.json, GitHub Actions
workflows get real $schema-driven validation/completion) with no
network call at startup -- set your own [lsp.json]/[lsp.yaml]
settings.json.schemas/settings.yaml.schemas in config.toml to replace
the bundled defaults entirely for that language.

SESSIONS
:sessionsave           Save named-file panes, positions and recursive layout
:sessionload           Restore saved layout while retaining current buffers
Splits can mix orientations (up to 32 panes). Ctrl-W h/j/k/l uses pane geometry.
Session files live privately in .vaayu/session.json; source drafts use :recover.

AGENT REVIEW
A in a results list    Run configured agent on selected/current feedback
R in comments         Toggle selected/current notes resolved/unresolved
:reviewexport          Write selected/current feedback to a private JSON packet
:reviewrun             Run review_command argv with that packet on stdin
:reviewcancel          Stop the current review process
:reviewresults         Open agent output; Ctrl-Q converts it to quickfix
Configure review_command and review_timeout_secs in TOML before running an agent.
The command runs in the current project; its own permissions/network settings apply.
Resolving feedback is explicit; agent completion does not mark comments resolved.
,rw saves resolved status. Recovery now includes edited unsaved comment drafts.
