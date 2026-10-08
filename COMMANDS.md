# Command and leader reference

Reviewed against `4f1b81d` on 2026-10-08. The tables cover every entry in [the Ex discovery registry](src/command.rs) and [the leader action registry](src/actions.rs). Descriptions are quoted from those registries, except `:taskadd` and `:colorscheme`, whose older registry descriptions are corrected against their current dispatch. This is a discovery inventory: argument/range syntax, aliases, modes, and context-specific keys are explained in [HELP.md](HELP.md) and [GUIDE.md](GUIDE.md). `:commands` fills the Ex line; `:keymaps` runs the selected leader action.

## Ex discovery commands

| Command | Description |
| --- | --- |
| `:gitdiff` | Current file's saved diff as navigable results |
| `:gitstage` | Saved unstaged hunks; Enter stages one hunk |
| `:gitunstage` | Staged hunks; Enter unstages one hunk |
| `:gitblame` | Current file's blame as navigable results |
| `:gitstash` | Stash list; Enter shows a stash's diff |
| `:gitstashpush` | Stash all current tracked changes |
| `:gitstatus` | Git workspace: staged/unstaged/untracked/conflicts |
| `:gitcommit` | Commit staged changes with a message |
| `:gitcommitamend` | Amend the last commit |
| `:gitlog` | Commit log; Enter shows a commit's diff |
| `:gitfilehistory` | Current file's commit history; Enter shows a commit |
| `:gitrevert` | Revert a commit (default HEAD) as a new commit |
| `:gitcherrypick` | Cherry-pick a commit onto the current branch |
| `:gitbranch` | Local branches; Enter checks one out |
| `:gitpush` | Push the current branch |
| `:gitpull` | Pull the current branch |
| `:gitfetch` | Fetch from the remote |
| `:lazygit` | Open lazygit in an embedded terminal |
| `:ghprs` | GitHub pull requests (gh); Enter opens one |
| `:ghpr` | GitHub PR overview (default: current branch's PR) |
| `:ghcheckout` | Check out a GitHub PR locally (gh pr checkout) |
| `:ghdiff` | GitHub PR diff as navigable results |
| `:ghthreads` | GitHub PR review threads as jumpable results |
| `:ghchecks` | GitHub PR CI checks; Enter opens a job's log |
| `:ghissues` | GitHub issues; Enter shows one |
| `:claude` | Start/toggle a long-lived Claude terminal session |
| `:codex` | Start/toggle a long-lived Codex terminal session |
| `:agent` | Start/toggle a long-lived agent terminal session by name |
| `:agents` | List running agent sessions; Enter attaches one |
| `:ai` | AI prompt to the configured agent sidebar (code/selection + cursor + diagnostics) |
| `:permalink` | Copy a GitHub permalink for the cursor line |
| `:recover` | Browse source drafts from interrupted sessions |
| `:reviewrun` | Run the configured agent review command |
| `:reviewcancel` | Cancel an in-progress agent review |
| `:reviewresolve` | Mark the current review comment resolved |
| `:reviewresults` | Reopen the last agent review output |
| `:reviewexport` | Export a review packet to a file |
| `:sessionsave` | Save the current tab/window layout |
| `:sessionload` | Restore the last saved session |
| `:help` | Search this editor's own help text |
| `:keymaps` | List leader-key bindings; Enter runs one |
| `:commands` | List ex commands; Enter fills the command line |
| `:everything` | Keymaps, commands and recent projects combined |
| `:comments` | List private review comments |
| `:comment` | Add a comment anchored to the cursor line |
| `:commentfile` | Add a comment anchored to the whole file |
| `:commentswrite` | Save comment edits and relocated anchors |
| `:copen` | Reopen the quickfix list |
| `:make` | Run a build/test command; output → quickfix |
| `:test` | Run tests: :test [nearest\|file\|suite\|last\|output\|stop\|clear] |
| `:testnearest` | Run the test under the cursor |
| `:testfile` | Run the current file's tests |
| `:testsuite` | Run the whole test suite |
| `:testlast` | Re-run the last test command |
| `:testoutput` | Show the last test run's output |
| `:teststop` | Stop the running tests |
| `:testclear` | Clear the test pass/fail gutter marks |
| `:taskwatch` | Re-run a command into the quickfix on every save |
| `:today` | Open today's personal task log |
| `:taskview` | Open today's editable task log |
| `:ystd` | Open yesterday's personal task log |
| `:week` | Browse this week's task and activity entries |
| `:tasklist` | Browse every saved task and activity entry, including future dates |
| `:on` | Open a task log date: :on YYYY-MM-DD |
| `:taskadd` | Open a task draft when bare; otherwise add [date] [time] [--priority=N] text |
| `:tasknote` | Add a timestamped activity note |
| `:tasknotes` | Browse all saved activity notes |
| `:taskdone` | Mark the task entry under the cursor as done |
| `:termsend` | Send the current line (or range) to a terminal (REPL) |
| `:diffthis` | Mark this buffer for diff mode (compare two buffers) |
| `:diffoff` | Turn off diff mode |
| `:difffold` | Collapse unchanged regions in diff mode (:difffold [N]) |
| `:conflictours` | Resolve the merge conflict here keeping our side |
| `:conflicttheirs` | Resolve the merge conflict here keeping their side |
| `:conflictboth` | Resolve the merge conflict here keeping both sides |
| `:conflictnext` | Jump to the next merge conflict |
| `:conflictprev` | Jump to the previous merge conflict |
| `:normal` | Run normal-mode keys (:normal[!] {keys}) |
| `:global` | Run a command on matching lines (:g/pat/cmd) |
| `:vglobal` | Run a command on non-matching lines (:v/pat/cmd) |
| `:delete` | Delete the range or current line (:[range]d) |
| `:sort` | Sort lines (:[range]sort[!] [u][n][i]) |
| `:move` | Move lines after {addr} (:[range]m {addr}) |
| `:copy` | Copy lines after {addr} (:[range]t {addr}) |
| `:join` | Join the range lines into one (:[range]j[!]) |
| `:yank` | Yank the range/current line into a register (:[range]y [reg]) |
| `:put` | Put a register's lines below the range/current line (:[range]put [reg]) |
| `:colorpick` | Report the hex color under the cursor |
| `:colorlighten` | Lighten the hex color under the cursor (:colorlighten [pct]) |
| `:colordarken` | Darken the hex color under the cursor (:colordarken [pct]) |
| `:colorscheme` | Open a live theme picker when bare; apply a named theme directly |
| `:zen` | Toggle zen/focus mode (hide gutter + status line) |
| `:tours` | List .tours/*.tour code tours; Enter starts one |
| `:tour` | Start a code tour (:tour [name]) |
| `:tournew` | Describe a tour; the AI agent generates it (:tournew [name] + :toursave) |
| `:toursave` | Send the :tournew prompt to the AI agent to generate the .tour |
| `:tourend` | Stop the active code tour (hide the step panel) |
| `:tourrestart` | Restart the active code tour at its first step |
| `:toursteps` | List the active tour's steps; Enter jumps to one |
| `:tourexplain` | Ask the AI agent to explain the current tour step's code |
| `:tournext` | Next code-tour step |
| `:tourprev` | Previous code-tour step |
| `:lopen` | Reopen the location list |
| `:lnext` | Next location-list entry |
| `:lprev` | Previous location-list entry |
| `:ldiagnostics` | Fill the location list from this buffer's diagnostics |
| `:lgrep` | Grep the project into the location list (:lgrep <pattern>) |
| `:cclose` | Close the quickfix list |
| `:cnext` | Next quickfix location |
| `:cprev` | Previous quickfix location |
| `:colder` | Switch to the previous quickfix list |
| `:cnewer` | Switch to the next quickfix list |
| `:grep` | Live grep for a pattern |
| `:fold` | Fold a line range (:{range}fold); za/zo/zc/zd/zR/zM manage folds |
| `:foldindent` | Auto-fold by indentation into a nested overview |
| `:foldsyntax` | Auto-fold functions/classes via tree-sitter |
| `:foldlsp` | Auto-fold via the language server's foldingRange |
| `:far` | Reviewed project-wide search & replace screen (toggle matches, then R) |
| `:cfar` | Find/replace across every file in the results list (cfar/pat/repl/g) |
| `:cfarpreview` | Preview cfar changes across the results files without applying |
| `:todo` | Index TODO/FIXME/HACK/XXX comments |
| `:diagnostics` | Shared diagnostics list |
| `:workspacediagnostics` | Pull project-wide diagnostics from the language server |
| `:outline` | Document symbols as navigable results |
| `:documentlinks` | Document links; Enter opens or copies one |
| `:codelens` | Code lenses; Enter runs one (also shown as virtual text) |
| `:inlayhints` | Show inlay hints inline; Esc clears them |
| `:projects` | Recently launched-from directories; Enter switches |
| `:references` | References to the symbol under the cursor |
| `:callers` | Incoming calls (callers) of the function under the cursor |
| `:callees` | Outgoing calls of the function under the cursor |
| `:linkededit` | Rename all linked ranges to a new name (no name = live) |
| `:linkedlive` | Live linked editing: type in one range, mirror the others |
| `:supertypes` | Supertypes of the type under the cursor |
| `:subtypes` | Subtypes of the type under the cursor |
| `:typedefinition` | Type definition of the symbol under the cursor |
| `:implementation` | Implementation of the symbol under the cursor |
| `:declaration` | Declaration of the symbol under the cursor |
| `:workspacesymbols` | Workspace symbol search |
| `:format` | Format the buffer (or Visual selection) |
| `:rename` | Rename the symbol under the cursor across files |
| `:renameapply` | Apply a previewed rename (refactor_preview) |
| `:renamecancel` | Discard a previewed rename (refactor_preview) |
| `:codeactions` | List and apply a code action |
| `:organizeimports` | Apply the server's organize-imports action |
| `:signature` | Signature help at the cursor |
| `:lsprestart` | Restart language servers for this buffer |
| `:lspinfo` | Show language server status |
| `:tools` | Known language servers: health, and Enter to install one |
| `:lspcancel` | Cancel outstanding language requests |
| `:spellcheck` | Toggle spell-check underlines |
| `:indentinfo` | Show detected/configured indent settings |
| `:terminal` | Open an embedded terminal pane |
| `:tree` | Toggle the file tree sidebar |
| `:treefind` | Reveal the current file in the file tree |
| `:treeroot` | Re-root the file tree (default: project root) |
| `:treenew` | Create a new file/directory in the file tree |
| `:treerename` | Rename the file tree's selected path |
| `:tabnew` | Open a new tab |
| `:tabclose` | Close the current tab |
| `:tabonly` | Close every tab except this one |
| `:tabnext` | Next tab |
| `:tabprevious` | Previous tab |
| `:chistory` | Command-line history; Enter reruns one |
| `:shistory` | Search history; Enter reruns one |
| `:jumps` | Jump list as navigable results |
| `:registers` | Show registers |
| `:marks` | Show marks (Enter jumps) |
| `:messages` | Show recent messages |
| `:checkhealth` | Health: external tools, LSP servers, grammars |
| `:earlier` | Undo N changes (:earlier [N]) |
| `:later` | Redo N changes (:later [N]) |
| `:undolist` | Undo history viewer (Enter jumps to a state) |
| `:resume` | Reopen the last picker or Results/quickfix list |
| `:treebookmarks` | List file tree bookmarks |
| `:treesort` | Sort the file tree: name, type, mtime, size (none: next) |
| `:treetrash` | List .vaayu/trash; Enter restores an item |
| `:tabs` | List open tabs |
| `:vsplit` | Split the window vertically |
| `:vpreview` | Open a Markdown preview split |
| `:close` | Close the current window/pane |
| `:only` | Close every window except this one |
| `:set` | Toggle wrap/number (nowrap/nonumber to disable) |
| `:configreload` | Reload config.toml and restart servers |
| `:buffer` | Switch to buffer N, or list buffers |
| `:write` | Save the current buffer (or Save As <path>) |
| `:quit` | Close the window, or quit if it's the last one |
| `:wq` | Save and quit |
| `:quitall` | Quit, refusing if any buffer is unsaved |
| `:wqall` | Save every buffer, then quit |
| `:nohlsearch` | Clear search-match highlighting |
| `:edit` | Open a file by path |
| `:buffers` | List open buffers |
| `:blines` | Current buffer's non-blank lines as navigable results |
| `:b#` | Switch to the alternate buffer |
| `:bnext` | Next buffer |
| `:bprevious` | Previous buffer |
| `:bdelete` | Close the current buffer |

## Leader actions

Default leader is `,`; replace it with your configured leader. Keys are literal and case-sensitive.

| Binding | Action ID | Description |
| --- | --- | --- |
| `,rc` | `review.note` | New line/range review note |
| `,rf` | `review.note_file` | New file review note |
| `,rl` | `review.list` | List review notes |
| `,rw` | `review.save` | Save review notes |
| `,cq` | `quickfix.open` | Open quickfix |
| `,ld` | `lsp.diagnostics` | Diagnostics list |
| `,lf` | `lsp.format` | Format buffer (or Visual selection) |
| `,lh` | `lsp.document_highlight` | Highlight other occurrences of the symbol under the cursor |
| `,ll` | `lsp.document_links` | Document links; Enter opens or copies one |
| `,lC` | `lsp.document_color` | Colorize color literals in the buffer (documentColor) |
| `,lc` | `lsp.code_lens` | Code lenses; Enter runs one (also shown as virtual text) |
| `,li` | `lsp.inlay_hints` | Show inlay hints inline; Esc clears them |
| `,lr` | `lsp.rename` | Rename symbol |
| `,la` | `lsp.actions` | Code actions |
| `,lI` | `lsp.organize_imports` | Organize imports (applies directly, no picker) |
| `,lo` | `lsp.outline` | Document outline sidebar |
| `,lO` | `lsp.outline_sidebar` | Toggle outline sidebar |
| `,lR` | `lsp.references` | References |
| `,ls` | `lsp.signature` | Signature help |
| `,lw` | `lsp.workspace_symbols` | Workspace symbols |
| `,pd` | `peek.definition` | Peek definition in a floating window |
| `,pt` | `peek.type_definition` | Peek type definition in a floating window |
| `,pi` | `peek.implementation` | Peek implementation in a floating window |
| `,pr` | `peek.references` | Peek references (list + preview) in a floating window |
| `,pk` | `peek.hover` | Hover documentation in a floating window |
| `,pf` | `peek.focus` | Focus the open floating window |
| `,pc` | `peek.close` | Close the floating window |
| `,ms` | `window.vsplit_preview` | Vertical split + Markdown preview |
| `,kv` | `window.vsplit` | Open a vertical split |
| `,ks` | `window.split` | Open a horizontal split |
| `,w` | `file.write` | Write current buffer |
| `,q` | `file.quit` | Close buffer (quit if it is the last one) |
| `,Q` | `file.quit_force` | Quit without saving |
| `,h` | `search.clear_highlight` | Clear search highlight |
| `,d` | `edit.delete_blackhole` | Delete to black-hole register |
| `,ow` | `option.toggle_wrap` | Toggle line wrap |
| `,or` | `option.toggle_relativenumber` | Toggle relative line numbers |
| `,ol` | `option.cursorline_info` | Show cursorline hint |
| `,R` | `config.reload` | Reload config + restart LSP |
| `,ft` | `explorer.toggle` | Toggle file tree sidebar |
| `,e` | `explorer.focus` | File tree: open / focus / close |
| `,fe` | `explorer.find` | Reveal current file in the file tree |
| `,fr` | `file.recent` | Recent files |
| `,ff` | `file.picker_alt` | Project file picker |
| `,ma` | `cursors.add_all` | Multiple cursors: every occurrence of the word |
| `,mj` | `cursors.add_below` | Multiple cursors: add a cursor below |
| `,mk` | `cursors.add_above` | Multiple cursors: add a cursor above |
| `,mc` | `cursors.clear` | Multiple cursors: collapse to one |
| `,mp` | `markdown.preview_toggle` | Toggle Markdown preview |
| `,b` | `buffer.list` | Buffer list |
| `,/` | `search.grep` | Live grep |
| `,sr` | `search.replace` | Search & replace across the project (reviewed) |
| `,sw` | `search.replace_word` | Replace word under cursor / selection across the project |
| `,gp` | `git.permalink` | Copy GitHub permalink (cursor line / Visual selection) |
| `,gh` | `git.hunk_preview` | Preview the saved hunk under the cursor |
| `,gB` | `git.blame_toggle` | Toggle line-blame virtual text |
| `,gd` | `git.diff_overlay_toggle` | Toggle the diff overlay (deleted lines, changed-word highlight) |
| `,gS` | `git.status` | Open the Git workspace (staged/unstaged/untracked/conflicts) |
| `,gl` | `git.lazygit` | Open lazygit in an embedded terminal |
| `,Gp` | `github.prs` | GitHub: pull requests |
| `,Gv` | `github.pr` | GitHub: current branch's PR overview |
| `,Gd` | `github.diff` | GitHub: current branch's PR diff |
| `,Gr` | `github.threads` | GitHub: current branch's PR review threads |
| `,Gc` | `github.checks` | GitHub: current branch's PR CI checks |
| `,Gi` | `github.issues` | GitHub: issues |
| `,gW` | `git.diff_ignore_whitespace_toggle` | Toggle ignoring whitespace in :gitdiff's view |
| `,gx` | `git.hunk_reset` | Reset the saved hunk under the cursor to HEAD (or the Visual selection's lines) |
| `,gs` | `git.hunk_stage` | Stage the saved hunk under the cursor (or the Visual selection's lines) |
| `,gw` | `search.grep_word` | Live grep word under cursor / selection |
| `,fw` | `search.find_word` | Find word under cursor / selection (live grep) |
| `,f.` | `search.repeat_last` | Resume the last buffer search |
| `,fb` | `search.grep_current_buffer` | Live grep scoped to the current buffer |
| `,fB` | `search.grep_open_buffers` | Live grep scoped to every open buffer |
| `,a` | `edit.select_all` | Select entire buffer |
| `,z` | `ui.zen_toggle` | Toggle zen (line numbers) |
| `,cx` | `agent.context_send` | Send context to an agent (file/selection/clipboard/symbol/diagnostics) |
| `,ca` | `ai.prompt` | AI prompt to the configured agent sidebar (code/selection + cursor + diagnostics) |
| `,td` | `tasks.view` | Tasks: open today's editable task log (:taskview) |
| `,un` | `tabs.new` | Tabs: open a new tab (:tabnew) |
| `,u]` | `tabs.next` | Tabs: move to the next tab (:tabnext / gt) |
| `,u[` | `tabs.previous` | Tabs: move to the previous tab (:tabprevious / gT) |
| `,uq` | `tabs.close` | Tabs: close the current tab (:tabclose) |
| `,tL` | `tasks.list` | Tasks: browse all saved entries, including future dates (:tasklist) |
| `,tl` | `tasks.add` | Tasks: write a new task in a buffer; :wq saves it (:taskadd) |
| `,ta` | `tasks.add_alias` | Tasks: write a new task in a buffer; :wq saves it (:taskadd) |
| `,tN` | `tasks.note` | Tasks: log an activity note (:tasknote) |
| `,tn` | `tasks.notes` | Tasks: browse activity notes (:tasknotes) |
| `,tD` | `tasks.close` | Tasks: mark the task under the cursor done (:taskdone) |
| `,ty` | `tasks.yesterday` | Tasks: open yesterday's task log (:ystd) |
| `,tw` | `tasks.week` | Tasks: browse this week's entries (:week) |
| `,tO` | `tasks.on_date` | Tasks: open a date (:on YYYY-MM-DD) |
| `,vs` | `tour.start` | Code tours: pick one to start (:tours) |
| `,vn` | `tour.next` | Code tour: next step (:tournext) |
| `,vp` | `tour.prev` | Code tour: previous step (:tourprev) |
| `,ve` | `tour.end` | Code tour: end (:tourend) |
| `,vr` | `tour.restart` | Code tour: restart from the first step (:tourrestart) |
| `,vy` | `tour.copy_step` | Code tour: copy this step and source metadata |
| `,vY` | `tour.copy_all` | Code tour: copy the complete tour as JSON |
| `,vo` | `tour.steps` | Code tour: jump to a step (:toursteps) |
| `,vx` | `tour.explain` | Code tour: explain this step with the AI agent (:tourexplain) |
| `,vc` | `tour.new` | Code tour: describe a new one for the AI agent to generate (:tournew) |
| `,Tn` | `test.nearest` | Test: run the test under the cursor (:testnearest) |
| `,Tf` | `test.file` | Test: run this file's tests (:testfile) |
| `,Ts` | `test.suite` | Test: run the whole suite (:testsuite) |
| `,Tl` | `test.last` | Test: re-run the last test command (:testlast) |
| `,To` | `test.output` | Test: show the last run's output (:testoutput) |
| `,Tx` | `test.stop` | Test: stop the running tests (:teststop) |
| `,Tc` | `test.clear` | Test: clear the pass/fail gutter marks (:testclear) |
| `,=` | `select.expand` | Expand selection to the enclosing syntax node |
| `,-` | `select.shrink` | Shrink incremental selection |

## Additional dispatch and aliases

The registry is not the full Ex parser. Ordinary Vim-style commands include `:w[rite]`, `:w!`, `:wq`, `:x`, `:q[uit]`, `:qa[ll]`, `:e[dit]`, `:e!`, `:bd[!]`, `:bn`/`:bp`, splits/tabs, search/substitution, ranges, and `:set`. See [run_ex](src/command.rs) and [help](HELP.md) for the authoritative parser and documented forms. Task aliases include `:taskview`/`:today`, `:ystd`/`:yesterday`, and `:taskdone`/`:taskclose`; bare `:taskadd` opens the draft. `:term` aliases `:terminal`; `Ctrl-\` is the floating-shell toggle. `:tour` without a name resumes the last tour; `:tourrestart` restarts at step one.

Task keys use `,t...`, tabs use `,u...`, tours use `,v...`, and tests use `,T...`. Obsolete `,tt`/`,t]`/`,t[`/`,tq` and tour `,ts`/`,tn`/`,tp`/`,te`/`,to`/`,tx`/`,tc` should be migrated to the current registry. Results use `?` for help and `g?` for backward search. `,lo`/`,lO` toggle the sidebar; `:outline` opens a transient list. The registered `,z` action currently toggles line numbers, while `:zen` hides editor chrome; [the parity plan](NEOVIM_PARITY_PLAN.md) tracks that mismatch.
