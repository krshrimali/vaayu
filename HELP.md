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

PERSONAL TASK TRACKER
:today                  Open today's editable structured task document
:ystd                   Open yesterday's document
:on YYYY-MM-DD          Open a chosen date, including a future date
:week                   Browse this Monday–Sunday's entries
:taskadd [date] [time] text
                        Add an open task; date and time are optional (ISO
                        date, 24-hour time). Capture time is recorded even
                        when a future date is supplied.
:tasknote [date] [time] text
                        Add an activity note with its capture timestamp
Edit the TOML buffer normally and use :w / Ctrl-S. Records are stored
privately under the user's Vaayu data directory, across projects.

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
                       while browsing, a fresh f clears it. Also works on
                       a live grep list -- it keeps narrowing every fresh
                       batch of ripgrep matches, not just the one active
                       when you started filtering.
q / Esc                Return to editing

TEST RUNNER
,Tn / :testnearest     Run the test under the cursor
,Tf / :testfile        Run the current file's tests
,Ts / :testsuite       Run the whole suite (also :test [nearest|file|...])
,Tl / :testlast        Re-run the last test command
,To / :testoutput      Reopen the last run's full output
,Tx / :teststop        Stop the running tests (kills the runner's
                       process group)
,Tc / :testclear       Clear the pass/fail gutter marks
The runner follows the file's language (cargo test, pytest -v, go test -v,
npx jest / npx vitest run / npm test --), or for a suite run with no such
file, the project's Cargo.toml / go.mod / package.json / pytest config.
Output streams live into a Results pane; when the run ends, failures
replace it as a quickfix list (Enter jumps to the failing line; a build
error with no test results falls back to file:line:col parsing), and
test declarations get ✓ / ✗ in the sign column. A green run keeps the
output pane open with a "✓ … N passed" title. Runs show in the progress
stack too. For any other command use :make.

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
                       Ctrl-r toggles a content-preview pane for the
                       selected match (off by default); Ctrl-e/Ctrl-y
                       scroll it. Reads are cached by file mtime so
                       browsing matches in the same file doesn't re-read
                       it from disk every frame.
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
,sr / :far             Reviewed project-wide search & replace: Search,
                       Replace and Files fields (Tab cycles, Enter goes to
                       the list) re-scan the project as you type and list
                       every match, grouped by file, as the line it would
                       become (with a -/+ preview where there is room).
                       Search/replace syntax is :s's (\( \) groups, \1 and &
                       in the replacement), per line. Files: globs split
                       by space/comma, ! excludes (*.rs !tests/** src).
  In the list:         Space/Tab/x toggle the match, or a whole file on its
                       header row · a all/none · n/N next/prev file ·
                       s/r/f edit a field (i the last one) · c case
                       auto/ignore/match · F literal (no regex, no \1) ·
                       Enter open the match · R replace the selected ·
                       U undo the last R · q/Esc close (state is kept;
                       ,sr resumes). Each file is one undo step in its
                       buffer (u there reverts it); a buffer with unsaved
                       edits of its own is replaced but left unsaved, and
                       a line edited since the scan is skipped.
,sw                    ,sr with the word under cursor / Visual selection
                       as a literal search, focused on Replace.
:cfar/pat/repl/[g]     Non-interactive: :s across every file in the
                       current results list (e.g. after :grep), saved.
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
gpd / ,pd              Peek definition in a floating window over the
                       cursor (source preview; the cursor stays put)
gpt / ,pt              Peek type definition
gpi / ,pi              Peek implementation
gpr / ,pr              Peek references: the locations listed above a
                       preview of the selected one
,pk                    Hover documentation in a floating window
  In a focused float:  j/k select (list) or scroll · Ctrl-E/Ctrl-Y,
                       Ctrl-D/Ctrl-U scroll the preview · Enter/o open
                       the location (back with Ctrl-O; :resume reopens
                       the list) · s/v open in a split · q/Esc close ·
                       Tab/Ctrl-W unfocus (it stays up until the cursor
                       moves or Esc) · : closes it and starts a command
,pf / ,pc / gP         Focus / close the floating window
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
Progress stack         Running jobs show bottom-right above the status
                       line, newest lowest: LSP $/progress tokens (title,
                       percentage, message, server name), live grep, git
                       commands, :make/:task (its command), LSP
                       formatting and the project file scan. A spinner
                       while running, then ✓ (or ✗ on failure) that fades
                       after ~2s. Jobs under 0.25s never appear; at most
                       5 rows (older ones fold into "+N more").
                       :set progress / noprogress (config: progress)

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
:colorscheme [name]    Switch the colorscheme (Tab completes names; no
                       name lists them): default, mono, warm, cool keep
                       the terminal's own background; gruvbox,
                       gruvbox-light, flexoki, flexoki-light, tokyonight
                       and tokyonight-day are true-color palettes that
                       paint the background, text and every UI surface
                       (statusline, tree, picker, results, diagnostics,
                       git, popups). Config: colorscheme = "gruvbox"
:set transparent       Keep the terminal's background under a palette
                       scheme (its text and UI colors still apply);
                       :set notransparent restores it (config:
                       transparent)

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

MULTIPLE CURSORS
Ctrl-N                 Add a cursor at the next occurrence (wrapping) of the
                       word under the cursor; the new one becomes the primary
                       (the one the view follows). Repeat to add more.
Visual Ctrl-N          Charwise: the selected text is the pattern (any match,
                       not just whole words). Blockwise: one cursor per line
                       at the block's left column (short lines skipped).
                       Linewise: one per line at the cursor's column.
,ma                    A cursor at every occurrence of the word
,mj / ,mk              Add a cursor on the line below / above
Esc / ,mc              Collapse to the primary cursor (Esc in Normal mode)
With several cursors   Normal-mode commands (motions, operators, counts,
                       registers, ., @q, p, x, ~, J...) and Insert-mode typing
                       apply at every cursor; secondaries are drawn reversed
                       and the status line shows "N cursors". One command
                       (or one Insert session) is one undo step. u / Ctrl-R
                       collapse to the primary first. :, /, Visual mode,
                       pickers and leader actions act on the primary only;
                       the completion popup is off while cursors exist.

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
File tree              ,e opens an explorer sidebar pinned to the left edge
                       (tree_position = "right" moves it) at a fixed width
                       (tree_width, default 32; < / > in the tree, Ctrl-W
                       < / >, or dragging its border resize it), focused
                       and revealing the current file. ,e on an open tree
                       focuses it; ,e in a focused tree closes it. ,ft and
                       :tree are a plain toggle; ,fe / :treefind / F reveal
                       the current file. ? in the tree lists every key.
                       Moving: j/k (counts: 5j) · gg/G · Ctrl-D/U half page
                       · Ctrl-F/B page · Ctrl-E/Y scroll · zz/zt/zb · P
                       parent · K/J first/last sibling · ]c/[c next/prev
                       git change · ]d/[d next/prev diagnostic.
                       Opening: l expands / steps in / opens · h collapses
                       or goes to the parent · Enter/o toggle or open ·
                       Tab previews (focus stays in the tree) · v toggles a
                       floating preview beside the tree that follows the
                       cursor (a directory shows its entries; ,pf focuses
                       it to scroll; Esc closes it) · s / S (or
                       Ctrl-V / Ctrl-X) open in a vertical / horizontal
                       split · T / Ctrl-T in a new tab · gx with the system
                       app. Files open in the editing pane you used last;
                       if the tree is alone, one is created beside it.
                       E expands everything under the cursor (capped at
                       20k entries; skips symlinks and hidden/ignored
                       dirs) · W / zM collapse all.
                       / (or f) fuzzy-finds across the whole project, not
                       just expanded directories (the file picker's
                       background index and scorer), showing hits with
                       their ancestors and the cursor on the best one;
                       Enter keeps the filter, Esc clears it and keeps the
                       selected file revealed.
                       Root: - / Backspace up a directory · C into the
                       cursor's directory · ~ back to the project root ·
                       :treeroot [dir].
                       Files: a create (dir/ for a directory, a/b/c.rs
                       creates the parents) · r rename (prefilled) · d d
                       delete · t t trash into .vaayu/trash/ (both need a
                       second press; any other key cancels) · U or
                       :treetrash lists the trash, newest first, and Enter
                       restores an item to where it was trashed from
                       (re-creating its directory; refuses to overwrite) ·
                       gs / gu git stage / unstage the node (a directory:
                       everything under it; marks: all of them) · y / x / p
                       copy / cut / paste into the cursor's directory
                       (recursive; refuses collisions and pasting a
                       directory into itself; a failed copy rolls back) ·
                       Space marks nodes, and d/t/y/x then act on every
                       mark (u or Esc clears) · Y / gy copy the relative /
                       absolute path · i shows size, age and mode · m
                       bookmarks (★), B or :treebookmarks lists them.
                       Sort: O cycles name / type (extension) / mtime
                       (newest first) / size (largest first), directories
                       always first; :treesort <mode>; tree_sort sets the
                       default; the header shows a non-name sort (↓mtime).
                       Expanded directories, bookmarks and the width are
                       remembered across restarts (.vaayu/shada.json; a
                       changed tree_width setting wins over the saved
                       width).
                       Refuses to rename/delete/trash/move a path an open
                       buffer has unsaved changes under.
                       Display: indent guides, Nerd Font icons
                       (tree_icons = false for plain arrows), git status
                       letters right-aligned and colored (a directory shows
                       its most significant descendant's), E/W/I
                       diagnostics (a collapsed directory shows its worst
                       descendant's), ● for unsaved buffers, open files in
                       bold and the current one underlined, symlink
                       targets. Dotfiles (. or H) and .gitignore'd paths
                       (! or I) are hidden by default; .git always is.
                       Speed: directories are read only when expanded and
                       their listings cached; expand/collapse and the
                       hidden/ignored toggles never touch the disk. About
                       once a second, while visible, it stats the open
                       directories and re-reads only the ones that changed,
                       so new/removed files appear on their own. git status
                       runs on a background thread (on open, R, after a
                       save, and after any file change it sees).
                       tree_follow (default on) reveals each buffer you
                       switch to. Esc hands focus back to the editor; q
                       closes; :q from the last editing pane quits.
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
                       the open buffer to match; q/Esc cancels. With a
                       Visual selection, resets only the selected lines
                       within whichever hunk(s) it overlaps, not the whole
                       hunk -- via a reconstructed sub-patch, the same
                       line-level split git add -p performs internally
,gs                     Stage the saved hunk under the cursor directly into
                       the index (no confirmation prompt -- reversible via
                       :gitunstage). With a Visual selection, stages only
                       the selected lines, same split as ,gx
:e! / :edit!           Discard in-memory changes and reload the current
                       buffer from disk (undo history is cleared too);
                       with a path, same as :e/:edit
File watcher           Changes on disk show up without a keypress
                       (inotify/FSEvents, debounced): an open buffer with
                       no unsaved edits reloads ("<file> reloaded"); one
                       with unsaved edits is never clobbered -- it shows
                       [changed on disk] in the status line with a warning
                       until :e! (reload) or :w! (overwrite). The file tree
                       re-reads changed directories and the file picker /
                       tree / index re-scans when files appear or vanish
                       (at most every 2s). Not used for $HOME or /; if the
                       watch can't start, the tree's 1s poll and
                       focus-gained reload remain (:checkhealth says which).
                       :set [no]watch · :set [no]autoread (config: watch,
                       autoread; both default on)
,gB                     Toggle line-blame virtual text: the current line's
                       commit (short hash, author, date) after its own
                       text, computed asynchronously and follows the cursor
,gd                     Toggle the diff overlay: a removed line's HEAD
                       content shows as compact virtual text on the line
                       right after it (first removed line + a count, not
                       every one in full -- ,gh still shows the whole
                       hunk), and a modified line's actually-changed
                       word(s) get a distinct background highlight
                       instead of the whole line. Off by default; no
                       separate fetch, reads the same background git
                       diff data the gutter signs already use
,gp / :permalink       Copy a GitHub permalink (pinned to HEAD's commit) for
                       the cursor line, or a Visual selection's line range
                       with ,gp. P on a :gitblame entry uses that line's own
                       commit instead of HEAD. Requires a github.com origin
                       remote; never opens a browser or touches the network.
,gW                     Toggle whitespace-ignoring for :gitdiff's view
                       (git diff --ignore-all-space); re-runs it
                       immediately if it's the list currently shown.
                       Never affects hunk stage/unstage/reset -- those
                       always build their patches from the real diff.
Save the source before hunk actions. Ctrl-Q exports these lists to quickfix.

GIT WORKSPACE
,gS / :gitstatus       Git workspace: staged/unstaged/untracked/conflict
                       sections in one list, built from a single `git
                       status`. Enter on a file opens it; the usual
                       Results-list p/f/Tab (preview/filter/select) all
                       work here too.
s / u                  Stage / unstage the file under the cursor (or
                       every Tab-selected one), then refresh the view
D                      Discard an unstaged (tracked) file's working-tree
                       changes back to the index -- confirmation prompt
                       first; refuses if the buffer has unsaved edits.
                       Untracked files aren't discarded here -- use the
                       file tree's trash (,ft then t) for those.
c / C                  Commit staged changes / amend the last commit --
                       prefills :gitcommit /:gitcommitamend on the
                       command line for a one-line message (an empty
                       amend message keeps the previous one via
                       --no-edit; git's own $EDITOR flow still works for
                       :gitcommitamend run bare, without this prefill)
r                      Refresh the workspace (re-runs `git status`)
:gitstashpush          Stash all current tracked changes
:gitlog                Commit log; Enter shows that commit's diff
:gitbranch             Local branches; Enter checks one out (refuses on
                       any unsaved buffer; reloads every open buffer
                       after switching)
:gitpush/:gitpull/:gitfetch
                       Run against the configured remote in the
                       background (the one place in this workspace that
                       needs it -- everything else here is a fast, local
                       command); output shown as a Results list, errors
                       surface the same way
,gl / :lazygit         Open lazygit in an embedded terminal split, for
                       anyone who prefers its interface. External and
                       optional: a missing lazygit fails with a clear
                       message, same as any other external tool.

GITHUB WORKSPACE (read-first, via the gh CLI)
Every view is a Results list filled on a background thread. Without gh,
when gh isn't logged in (gh auth login), or in a project with no GitHub
remote, you get one clear message instead. N defaults to the current
branch's PR wherever it's optional.
,Gp / :ghprs [state]   Pull requests (open; or closed/merged/all); Enter
                       opens a PR's overview
,Gv / :ghpr [N]        PR overview: state, author, branches, size, review
                       decision, check summary, description, and action
                       rows (Enter): checkout, diff, review threads, checks
:ghcheckout N          gh pr checkout N, then reload every open buffer
                       (refused while any buffer has unsaved edits)
,Gd / :ghdiff [N]      The PR's diff; each hunk line jumps to that line
                       of the local file (exact once the PR is checked out)
,Gr / :ghthreads [N]   Review comments grouped into threads (replies
                       under their root, "(outdated)" when the code
                       moved); Enter jumps to the commented line
,Gc / :ghchecks [N]    CI checks, failures first; Enter on a GitHub
                       Actions check shows its job log (just the failed
                       steps for a failed job), grouped by step, cursor on
                       the first error line
,Gi / :ghissues [state]
                       Issues (open; or closed/all); Enter shows one with
                       its comments

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

AGENT TERMINAL SESSIONS
:claude / :codex       Start a long-lived interactive session in a new
                       split; running it again toggles: detaches (hides
                       the pane, keeps the process running) if attached,
                       reattaches the same session (not a new one) if
                       detached. Ctrl-C in Terminal mode sends a real
                       SIGINT to whatever the session is running, same
                       as any other embedded terminal.
:agent <name>          Same toggle, for any other named agent CLI.
                       Configure its argv with agent_commands.<name> in
                       TOML (a table of string arrays); an unconfigured
                       name just runs itself, so :claude/:codex already
                       work with no config once those CLIs are on PATH.
:agents                List running sessions (attached-in-this-tab or
                       detached); Enter attaches the selected one here,
                       or just focuses it if it's already attached.
,cx                    Send context to an agent: a picker over the
                       current file, a Visual selection, the clipboard,
                       the enclosing symbol's body/signature (needs the
                       outline already open once for this buffer -- ,lO
                       or :outline) and this buffer's diagnostics.
                       Enter copies the built text to the + register,
                       and also types it into an attached agent
                       session's input if one exists in this tab.
