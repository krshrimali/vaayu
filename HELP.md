Vaayu — editing and review

Save: Ctrl-S or ,w or :w. Quit: :q (checks unsaved work); :qa! discards.
Normal / Insert / Visual use Vim operators and motions. jk exits Insert.
,q closes a clean buffer while keeping its pane; the last buffer quits.
Current bindings reviewed against 4f1b81d on 2026-10-08. GUIDE.md covers workflows;
CONFIGURATION.md and COMMANDS.md contain the full setting/registry references.

BROWSER EDITOR
vaayu --web [file]      Run this editor in a browser tab (from a shell)
:web [file]             The same from here; again reuses the running session
Alt-W / Alt-T / Alt-N   Ctrl-W / Ctrl-T / Ctrl-N, which browsers keep for themselves
F11                     Page fullscreen; Chromium then passes the real Ctrl keys (hold Esc to leave)
Ctrl-Shift-V            Paste through the browser
Shift+drag              Select on the display and copy; Ctrl-Shift-C copies it again
                        Reloading or closing the tab keeps the session and unsaved text.

PRIVATE REVIEW COMMENTS
,rc or :comment         Comment on the cursor line / Visual lines (opens a split below)
,rf or :commentfile     Comment on the whole file
  In the comment:       type, Esc, then :w saves · :wq saves and closes · :q! discards
                        Empty drafts are never saved; :q closes only the comment.
,rl or :comments        List the review's comments (sorted by file and line)
,rv / ,rr / ,rx         View the thread / reply / resolve-reopen the comment on this line
,rm or :commentmove     Re-anchor a comment to this line (or after m in the list)
]r / [r                 Next / previous open comment, across files
,rs or :reviews         Reviews in this folder: Enter switches, n new, r rename,
                        c close/reopen, D delete (asks)
,rn or :reviewnew [--base REV] [title]   Start another review
:reviewswitch NAME · :reviewrename · :reviewclose · :reviewreopen · :reviewbase REV
:reviewfiles            Files changed since the review base, with comment counts
In the list: Enter go · e edit · r reply · R resolve (saved at once) · d delete (y/n)
             m move · o open → needs attention → resolved → all · A agent · s reviews
Gutter: ● open  │ rest of a range  ◌ code changed (check, then ,rm)  ○ resolved
Comments save immediately to .vaayu/reviews/ at the repository root (Git-ignored)
and merge with other editors working in the same review. :set reviewtext=all|cursor|off
Agent: ,ra / :reviewrun sends open (or selected) comments to review_command;
       replies attach to their comments, findings are jumpable (c makes a comment).
       :reviewresults · :reviewruns · :reviewcancel · :reviewexport
GitHub: :reviewpublish [N] (pending PR review) · :reviewimport [N] (threads → review)

PERSONAL TASK TRACKER
:today                  Open today's editable structured task document
:ystd                   Open yesterday's document
:on YYYY-MM-DD          Open a chosen date, including a future date
:week                   Browse this Monday–Sunday's entries
:tasklist / ,tL         Browse all dates, including future tasks and notes
:tasknotes / ,tn        Browse activity notes added with :tasknote
:taskadd [date] [time] [--priority=N] text
                        Add an open task; date/time are optional. Priority
                        defaults to 1 (higher values sort first).
                        Bare :taskadd, ,tl or ,ta opens a draft with Priority, Task and multiline
                        Notes fields. :w / Ctrl-S / :wq / :x saves to its day view.
                        Task:/Notes: accept leading spaces and inline text.
                        Open overdue tasks move to today at
                        startup/task view; completed tasks remain in place.
:tasknote [date] [time] text
                        Add an activity note with its capture timestamp

In task lists: ? keymaps · D mark done · O reopen · dd delete current entry
               Deletion asks y (yes) / n (no); Esc cancels. Changes save
               immediately, including from task lists exported to quickfix.
:taskdone / :taskclose / ,tD completes the cursor's task block in a day document.
Task draft Notes appear in the task row's detail and editable day document.
Review comments attached to files are in :comments / ,rl.
Edit the TOML buffer normally and use :w / Ctrl-S. Records are stored
privately under the user's Vaayu data directory, across projects. External
changes, invalid documents, symlinks and unsaved-day mutation conflicts are
refused; task documents must be saved in place. Unsaved day edits support :recover.
Tasks: ,td today · ,ty yesterday · ,tw week · ,tL all · ,tn notes · ,tO date prompt.

CODE TOURS
:tour [name]             Start a named tour; bare :tour resumes the last one
:tourrestart             Restart the active or last tour at step one
,vs / ,vn / ,vp / ,ve   Browse / next / previous / end
,vo / ,vx / ,vc / ,vr   Step list / AI explanation / draft / restart
K                        During a tour, switch Lesson / Source focus
                         K on a tour-list entry starts it with explanation focused.
Mouse drag on divider    Resize the source and explanation panes
]t / [t                 Next / previous step
]h / [h                 Next / previous highlight within this step (counts supported)
,v] / ,v[               Next / previous highlight (:tourhnext / :tourhprev)
]q / [q                 End / restart the tour
,vy / ,vY               Copy current step metadata / the full tour JSON
,vd / ,vf / ,vS         Show step Diff / Flow / restore Source and its cursor
Ctrl-O in Diff / Flow   Return to Source, preserving step, cursor and jump history
,vi / ,vk               Toggle indicators / check tour paths and anchors
,vm                     Toggle rendered Markdown / raw description
:tourmarkdown [on|off|toggle]  Set or toggle description rendering
:tourjump id            Jump to an annotation; :tourfollow 1 follows a connection
Optional step.vaayu JSON adds annotations, connections and example/Git diffs.
Statuslines show Highlight X/N, or Highlights: N before selecting one.
Highlights follow JSON order across files; broken anchors are skipped. At
the first/last highlight, navigation stays within this step. ]t/[t changes steps.
Views appear in the pinned bar on wide terminals and in ,v? / :tourhelp.
The bar accepts mouse clicks; progress remains visible while scrolling.
:tourlayout auto|horizontal|vertical chooses the layout for this session.
Auto shows one tour pane at ≤16 rows, splits normally, and uses side by side
panes at ≥140 columns and ≥24 rows. Very short terminals keep one pane.
Comparisons run in the background
and use scratch buffers; they are never applied to source. :tourview diff
refreshes after buffer edits. :tourcheck [name] reports broken visual targets.
Descriptions start rendered. ,vm works from either pane; ,mp also
toggles while the description is focused. Esc returns rendered text to raw
editing. Headings, styles, lists, tables, links and code fences use the cached
Markdown renderer. j/k, Ctrl-D/U and g/G scroll rendered text; K, tour keys
and Ex commands still work. Rendering preference lasts for this session.
View/highlight/indicator changes and retries preserve scratch edits and
the lesson position. Relative image paths use
the project root. Explanation edits are scratch state; step changes replace
them. Missing source: ,vR / :tourretry retries; ]t skips; ,vE / :touredit
opens the definition. The unavailable view is read-only. Final steps offer
Finish / Restart / Tours. The tour picker shows counts and resume state;
r starts over, e edits, n creates, R refreshes. :tourexample makes a sample.
Saving the :tournew draft requests a name, then :toursave sends it to ai_agent.
Press Enter in that CLI to submit. .tours/*.tour contains CodeTour JSON.

RESULTS AND QUICKFIX
Ctrl-Q                 Send current picker/results/output to quickfix
,cq or :copen          Reopen quickfix
:cnext / :cprev        Next / previous quickfix location
:colder / :cnewer      Switch to the previous / next quickfix list
                       (Ctrl-Q appends a new one; browsing/dismissing
                       the current list doesn't)
?                      Show scrollable keymap help; q/Esc returns to the list
/ and g?               Search forward/backward (regex), Enter submit, n/N repeat
j/k / arrows           Move · Ctrl-D/U page · g/G first/last (gg also first)
                       Leader commands and : work outside query Insert mode.
                       Help/delete confirmation consumes keys before leaders.
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
                       Query starts in Insert mode. Esc leaves query entry;
                       in Normal mode j/k, gg/G, Ctrl-D/U and PageDown/Up
                       move the list, : opens Ex, leader mappings work.
                       i edits the query; another Esc closes the picker.
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
                       the next ,e rebuilds it at the new root
:cd [dir] / :chdir     Change the working directory and project root.
                       No argument goes home, ~ expands, :cd - returns
                       to the previous directory; Tab completes
                       directories only
:pwd                   Show the current working directory
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
% / 50%                Match brackets, C comments or conditionals / jump to
                       a percentage of the file; operators and Visual work.
Visual-block p / P     Replace the rectangle; P preserves the source register.
                       Characterwise text repeats down rows, block text keeps
                       its height, linewise text goes below (p) / above (P).
                       Counts repeat the text; . repeats the replacement.
/ ? n N                Search jumps recenter the match in the viewport,
                       like zz (Ctrl-D/Ctrl-U already did; Ctrl-F/Ctrl-B
                       intentionally do not, matching Vim's full-page scroll)
* / #                  Search forward / backward for the word under the cursor
                       (whole word; the next word on the line if the cursor
                       is between words); n/N continue, a count skips ahead
g* / g#                Same, but also match inside longer words
H / M / L              Top / middle / bottom of the window ({n}H, {n}L count in)
- / + / Enter / _      First non-blank of the previous / next / current line
{n}|                   Screen column n · g_ last non-blank · gm / gM middle
( / )                  Previous / next sentence (empty lines are boundaries)
[[ ]] [] ][            Previous / next section start ({ or a top-level line
                       ending in {) and end (} in column 0)
[( [{ ]) ]}            Enclosing unmatched bracket · [m ]m [M ]M method
                       starts / ends · [* ]* C-comment start / end
[' ]' [` ]`            Previous / next lowercase mark (line / exact)
[p ]p                  Paste a linewise register re-indented to this line
{n}go / gE             Byte n of the buffer · back to the end of a WORD
is as it at            Sentence / tag text objects (dit, cat, vis, …)
gu gU g~ g? =          Lowercase / uppercase / toggle / rot13 / reindent
                       operators (guu, gUU, g??, ==, =ip; Visual u U = g?)
d/pat d?pat dn d* d;   Operators accept search, n/N, */# and ;/, motions
gn / gN                Select the next / previous match; cgn then . repeats
U                      Undo all recent changes on the last changed line
R / gR                 Replace mode (Backspace restores what was overwritten)
gJ                     Join lines without adding or removing spaces
gi                     Insert where Insert mode was last left
g; / g,                Older / newer position in the change list (:changes)
& / g&                 Repeat the last :s on this line / on every line
gf / gF / gx           Open the file under the cursor (gF: at its :line) /
                       open the URL or file with the system opener
g8 / ga (:ascii)       Character under the cursor as UTF-8 hex / code point
Ctrl-G                 File name, modified flag, lines and position
Ctrl-] / Ctrl-T        Go to definition / back (a tag stack)
ZZ / ZQ                Write and quit / quit without writing
Q / gQ                 Replay the last recorded macro (Neovim) / command line
@:                     Repeat the last command line (then @@)
". ": "/ "%            Last insert / command / search / file name registers
zf{motion} zF zE       Create a fold over a motion / count lines; delete all
zj / zk / zv           Next fold start / previous fold end / open to cursor
z<CR> z. z-            Like zt zz zb, cursor to the first non-blank
zh zl zH zL zs ze      Scroll sideways (nowrap): column, half screen, cursor
Insert Ctrl-W Ctrl-U   Delete the word / back to the indent before the cursor
Insert Ctrl-T Ctrl-D   Indent / dedent the line · Ctrl-H Backspace · Ctrl-J Enter
Insert Ctrl-R {reg}    Insert a register (Ctrl-R Ctrl-W: word under cursor)
Insert Ctrl-V {key}    Insert a key literally, or Ctrl-V u00e9 / x41 / 065
Insert Ctrl-K {a}{b}   Digraph (e' é, a* α, -> →, Eu €, OK ✓, …)
Insert Ctrl-A / Ctrl-@ Insert the last inserted text (Ctrl-@ also leaves)
Insert Ctrl-E / Ctrl-Y Copy the character below / above the cursor
Insert Ctrl-O          Run one Normal-mode command, then keep inserting
Insert Ctrl-N / Ctrl-P Complete the word from the buffer (and LSP)
Insert Ctrl-X Ctrl-F   Complete a file name · Ctrl-X Ctrl-L a whole line
Ctrl-W q n x r R p t b Close / new / exchange / rotate / previous / first /
                       last window · H J K L move to an edge · T to a tab ·
                       _ | maximize · f split + gf · ] split + definition ·
                       ^ split + alternate · z close the preview
Visual J gJ r * #      Join / join raw / replace every character / search
                       for the selection
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
,lo / ,lO              Toggle persistent document outline sidebar
:outline               Document symbols as a transient results list
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
Ctrl-W v / ,kv / :vsplit  Vertical split (optional file argument)
Ctrl-W s / ,ks / :split   Horizontal split (optional file argument)
Ctrl-W w/h/j/k/l       Focus pane; Ctrl may remain held on h/j/k/l
                       (legacy Ctrl-H/J may arrive as Backspace/Enter)
Ctrl-W c / :close      Close pane
Ctrl-W o / :only       Keep active pane
,ms / :vpreview        Side-by-side Markdown preview
,mp                    Full-screen Markdown preview
                       Markdown images load local paths relative to the document
                       or HTTP(S) URLs. PNG/JPEG/GIF/WebP/BMP/ICO/SVG supported;
                       GIF uses its first frame. Local changes refresh live.
                       Graphics require Kitty Unicode placeholders; otherwise
                       alt text and paths appear. Tables use text fallback.
                       Mermaid fences render local diagrams. Graphics-capable
                       terminals use themed PNGs; others use Unicode. In either
                       preview: j/k, arrows or mouse wheel scroll; Ctrl-D/U page,
                       g/G top/end. The wheel scrolls the pane under the pointer.
                       h/l pan wide images/diagrams. +/- zoom graphics;
                       0 resets zoom/pan. q closes. Config mermaid_preview:
                       auto (default), unicode, kitty (force graphics), off.
                       This setting also controls Markdown image graphics;
                       unicode/off show image text without loading images.
                       Auto uses Unicode through tmux/screen and in WezTerm. Invalid or
                       unsupported diagrams show their source and a reason.
:set wrap / nowrap     Soft wrapping / horizontal scrolling
,ow                    Toggle wrap
:colorscheme [name]    Switch the colorscheme (Tab completes names; no
                       name opens a live picker): navigation previews; Enter
                       applies; q/Esc restores the original. default, mono, warm, cool keep
                       the terminal's own background; gruvbox,
                       gruvbox-light, flexoki, flexoki-light, tokyonight,
                       tokyonight-day, github-dark and github-light are
                       true-color palettes that paint the background,
                       text and every UI surface
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
Outline sidebar        ,lo / ,lO toggles a persistent symbol sidebar (LSP
                       documentSymbol, shown as a hierarchy by indentation).
                       Starts at 30% of the current pane width; Ctrl-W resize
                       or divider drag changes it. Initial cursor follows source.
                       gg/G, 0/^/$ and paging move; Ctrl-Q exports visible nodes.
                       Focused tree/outline panes show the selected-row cursor.
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
                       Command/search history is persisted in project shada.
:jumps                 Jumplist as a results list; Enter jumps to the entry.
:chistory / :history   Command history as a results list; Enter reruns it.
:shistory              Search history as a results list; Enter reruns it.
Tabs                   ,un new · ,u] next · ,u[ previous · ,uq close
                       gt/gT next/prev tab · {n}gt jumps to tab n
                       :tabnew :tabclose(:tabc) :tabonly(:tabo) :tabs
                       Tabline appears once a second tab exists. Each tab
                       keeps its own panes/cursor; closing one kills any
                       terminals running in it. :sessionsave/:sessionload
                       persists tabs/layouts; terminal processes are not restored.
Terminal               :terminal (:term) opens $SHELL in a real embedded
                       PTY in a new split, entering Terminal mode so typing
                       goes straight to the shell. Esc leaves to Normal for
                       pane navigation (Ctrl-W h/j/k/l, :close); i re-enters.
                       Closing the pane always kills the child process.
                       Its statusline shows mode, name, exit status and how
                       far it is scrolled back.
Terminal scrolling     Wheel scrolls history (programs that requested mouse
                       reports get the wheel; full-screen ones get arrows).
                       Normal mode: j/k, Ctrl-E/Y, Ctrl-U/D, Ctrl-B/F,
                       PageUp/PageDown, gg, G. Typing: Shift-PageUp/PageDown.
                       Any typing returns to the live screen.
                       / ? v V y or :termcapture open all output in a buffer
                       to search, select and yank.
Terminal keys          Ctrl-\ then a key sends that key literally (Esc,
                       Ctrl-W, Ctrl-\). Alt-keys, Shift/Alt+Enter (newline
                       in agent prompts), Ctrl/Shift+arrows and F1-F12 are
                       forwarded. Click a terminal to type in it; click
                       elsewhere to leave it.
Floating shell         Normal-mode Ctrl-\ opens/hides/reopens the same PTY.
                       Esc or Ctrl-Q closes it and stops the child; Ctrl-W
                       unfocuses. Terminal probe replies support shell startup.
Spelling               :spellcheck lists misspelled words (results list);
                       zg adds the word under cursor to your dictionary;
                       z= shows/replaces with suggestions. Needs a system
                       word list (/usr/share/dict/words or similar);
                       degrades to a message if none is installed.
Mouse                  Click positions cursor and focuses the clicked pane;
                       drag selects (Visual); wheel scrolls the view, including
                       full/split Markdown previews, without changing focus;
                       Ctrl-click goes to definition. Requires terminal
                       mouse reporting; dragging split dividers resizes panes.
                       Drag over a statusline or the message line to copy
                       that text (+ register); double-click copies a word,
                       triple-click the whole line.
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
Variable and numbered-stop regex transforms (${VAR/regex/fmt/flags} /
${1/regex/fmt/flags}) support capture references and g/i flags; numbered
mirrors update on stop synchronization. Unsupported/malformed syntax
degrades to plain text rather than rejecting the whole completion.
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
:sessionsave           Save named-file panes, tabs, positions, folds and layout
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
:claude / :codex       Start a long-lived interactive session in a
                       right-hand sidebar; running it again toggles:
                       detaches (hides the pane, keeps the process
                       running) if attached,
                       reattaches the same session (not a new one) if
                       detached. Ctrl-C in Terminal mode sends a real
                       SIGINT to whatever the session is running, same
                       as any other embedded terminal.
:agent <name>          Same toggle, for any other named agent CLI.
                       Configure its argv with agent_commands.<name> in
                       TOML (a table of string arrays); an unconfigured
                       name just runs itself, so :claude/:codex already
                       work with no config once those CLIs are on PATH.
:agents                List running sessions (attached-in-this-tab,
                       detached or exited); Enter attaches the selected one
                       here, focuses it if it's already attached, or starts
                       a fresh session if it had exited. An exited agent is
                       also restarted by :claude/:codex/:agent and :ai.
:agentstop <name>      Stop a named agent session and close its pane.
:aistart               Open or focus the configured AI agent's sidebar.
:ainew [instruction]   Start another session of the configured agent
                       (claude #2, claude #3, ...) stacked under the
                       running one; with an instruction (same flags as
                       :ai) the prompt goes to that new session. :ai,
                       :aistart and :aistop act on the agent pane you
                       focused last; :agents lists them all.
:aistop / :airestart   Stop, or stop and start fresh, the configured agent.
:aistop!               Stop every session of the configured agent.
Clickable output       In an agent (or any terminal) pane, click a file
                       reference (src/a.rs, src/a.rs:12:5, Update(src/a.rs),
                       a.rs#L12) to open it in the editor pane next to the
                       agent, on that line; click a row of a diff the agent
                       printed (  12 +  code, or a git diff hunk) to open
                       its file at that line; click a URL to open it in the
                       browser. Links underline under the pointer. Ctrl-W p
                       goes back to the agent. A program that takes the
                       mouse itself (lazygit) gets plain clicks; Ctrl-click
                       opens links there.
Scrolling              The wheel, Ctrl-u/d and Ctrl-b/f scroll an agent's
                       history smoothly over a few frames (smooth_scroll =
                       false jumps instead). Typing returns to the live view.
:ai [instruction]      Send a prompt plus code/cursor/diagnostic context to
                       the configured AI sidebar; without an instruction,
                       choose Start/Focus to open the agent with nothing
                       sent, a prompt template (Review changes sends this
                       file's uncommitted diff), or Restart/Stop. Also ,ca.
                       From Visual mode (:'<,'>ai, ,ca) only the selection
                       is sent -- character-exact for v selections.
:ai --file|--selection|--diff|--buffers|--none <instruction>
                       Choose the context: whole file, selection, diff
                       since HEAD (unsaved edits included), every open
                       file, or none. Default: selection, else file.
                       After sending, the sidebar is focused so Enter
                       submits (ai_focus_on_send = false keeps focus here;
                       ai_auto_submit = true presses Enter for you).
                       Agent panes open to the right; ai_split =
                       "horizontal" opens them below. :hor ai / :vert ai
                       (also :aistart, :claude, :codex, :agent) choose the
                       direction for that one command.
                       [[ai_prompts]] tables (label, instruction, context)
                       add templates or replace a built-in by label.
                       ai_agent = "claude" is the default; use "codex" or
                       another CLI name to switch. Code tours (:toursave /
                       :tourexplain) use the same setting.
                       Override executable paths/arguments with, for example:
                       ai_agent = "codex"
                       agent_commands = { codex = ["/absolute/path/to/codex"] }
                       Both options belong before any TOML tables. Reload
                       with :configreload; explicit :claude / :codex /
                       :agent <name> still target that named CLI.
,cx                    Send context to an agent: a picker over the
                       current file, a Visual selection, the clipboard,
                       the enclosing symbol's body/signature (needs the
                       outline already open once for this buffer -- ,lO
                       / ,lo or :outline) and this buffer's diagnostics.
                       Enter copies the built text to the + register,
                       and also types it into an attached agent
                       session's input if one exists in this tab.
