# Vaayu user guide

Reviewed against `4f1b81d` on 2026-10-08. See [change coverage](CHANGELOG.md) for all changes since the prior broad refresh, [architecture](ARCHITECTURE.md) for implementation, and [configuration](CONFIGURATION.md) for every setting.

A modal terminal editor in Rust: Vim-style editing, project review notes,
language servers, searchable results, and Markdown preview in one compiled core.

## Build and run

```sh
cargo install --path . --locked
vaayu 'path/to/file'     # `vy` is an equivalent command
```

Install `ripgrep` for project file discovery and live grep, Git for Git features,
the GitHub CLI (`gh`, logged in) for the GitHub workspace, and the language
servers you configure. Clipboard integration uses `wl-copy` /
`wl-paste` or X11 tools; SSH copying can use OSC52.

## Buffer close and theme selection

`,q` closes the current clean buffer while preserving its pane if another buffer exists; all panes showing that buffer move to its replacement. Closing the last buffer quits after unsaved-work checks. `:q` closes a pane or quits through the usual checks; `:bd!` explicitly discards a dirty buffer.

Bare `:colorscheme` opens a live theme picker: navigation previews colors, Enter applies, and `q`/Esc restores the original theme. `:colorscheme name` applies directly; Tab completes names. Runtime selection does not rewrite the config file.

## Private review notes

Use `,rc` on a line or Visual selection, or `,rf` for a file comment. Edit the
comment as a normal buffer and press **Ctrl-S** or `:w`. `,rl` opens the review
list: `e` edits, Enter visits the source, `d` deletes, Tab/Space selects,
`y` copies selected/current notes, and `Y` copies everything. `?` shows the
list's keymaps. `,rw` saves all
open comments, deletions, and updated anchors.

Notes live in `.vaayu/comments.json` under the launch working directory, with owner-only directory
and file permissions and a local Git ignore rule. They do not modify source
files. Unique source-text anchors follow moved lines and whitespace-only changes; ambiguous or missing
anchors are marked stale. OS file locks serialize note writes; changed snapshots are detected before
saving. Unsaved edited comments can be recovered with `:recover`. This is local OS-account privacy, not encryption.

## Personal task tracker

`:today` / `:taskview` opens today's editable task document; `:ystd`,
`:on YYYY-MM-DD`, and `:week` navigate other dates and the current
Monday–Sunday week. `:tasklist` browses every saved entry, including past and
future dates. Open tasks scheduled before today move into today's document at
startup and when opening the task views; completed tasks stay in their original
day. Running bare `:taskadd` or pressing `,tl` opens a task draft with an
integer Priority field (default 1), a Task field, and an optional multiline
Notes section. Use `:wq` to save it. Inline form:
`:taskadd [YYYY-MM-DD] [HH:MM] [--priority=N] text`; omitted priority is stored
as 1.
Use `:tasknote [YYYY-MM-DD] [HH:MM] text` to log an activity. Vaayu records
the local capture timestamp automatically, separate from an optional future
date/time. Task shortcuts: `,td` today, `,ty` yesterday, `,tw` this week,
`,tL` all entries, `,tl` or `,ta` open a task draft, `,tN` add a note, `,tO`
opens the date prompt, and `,tD` marks the task block under the cursor done.
The task list sorts higher priorities first. Press `?` to see its keymaps,
`D` to complete the current task, `O` to reopen it, or `dd` to delete the
current entry after confirming with `y` (yes) or `n` (no). These changes save
immediately and also work after sending the list to quickfix with Ctrl-Q.
`:tasknotes` or `,tn` opens the activity notes view. Notes attached to a task
appear in that task row's detail area and its editable day document.
Private review notes attached to source files use `:comments` or `,rl`.
Tabs use `,u...` (`un`, `u]`, `u[`, `uq`) and tours use `,v...` (`vs`, `vn`,
`vp`, `ve`, `vo`, `vx`, `vc`, `vr` to restart, `vy` to copy the current step,
and `vY` to copy the full tour as JSON). During a tour, `K` toggles focus to a
bottom explanation split and back to the source. Drag the divider with the
mouse to resize either pane; Vim motions and editing work in the explanation.
`]t`/`[t` navigate steps, `]q` ends, and `[q` restarts.
Day documents are editable structured TOML buffers
saved with `:w` or Ctrl-S, under the user's Vaayu data directory across
projects.

Task draft labels remain `Priority:`, `Task:`, and `Notes:`. `Priority:` must start the draft and contain an integer; the other labels accept leading whitespace and inline or following multiline content. `:w`, Ctrl-S, `:wq`, and `:x` save a draft and open its day view. `:taskdone` / `:taskclose` complete the task block containing the cursor in a day document.

The version-1 day schema contains `date`, `entries`, and per-entry `id`, `created_at`, `kind`, `status`, `priority`, optional `time`, `text`, optional `notes`, and optional `rollover_id`. Legacy entries without priority read as 1. Writes validate the document date against its filename, unique IDs, timestamp/time fields, and supported kind/status values. Locked, expected-snapshot atomic writes reject external changes and symlinks. Ordinary save hooks cannot bypass validation; day documents must be saved in place. List mutations reject conflicts with unsaved day buffers. Deletion rechecks its captured snapshot after confirmation. Overdue rollover retains a stable identity to avoid duplicates after interruption; it writes two files separately. Unsaved day documents are eligible for project recovery. See [task implementation](src/task_tracker.rs), [save handling](src/notes.rs), and [recovery](src/recovery.rs).

## Navigation and review

Tab shortcuts: `,un` opens a tab, `,u]` moves to the next tab, `,u[` moves to
the previous tab, and `,uq` closes the current tab. The last tab cannot be
closed. Existing Vim-style `gt` / `gT` navigation and `:tabnew`, `:tabnext`,
`:tabprevious`, and `:tabclose` commands remain available.

| Action | Binding / command |
| --- | --- |
| File picker / recent files / buffers | Ctrl-P / `,fr` / `,b` |
| File tree explorer (open / focus / close; `?` inside for keys) | `,e` |
| Tree: sort / floating preview / stage-unstage / restore from trash | `O` (`:treesort`) / `v` / `gs` `gu` / `U` (`:treetrash`) |
| Live grep | `,/` or `:grep pattern`; `,fw` searches the word under the cursor across the project |
| Reviewed project-wide search & replace (toggle matches, `R` applies, `U` undoes) | `,sr` / `:far`; `,sw` for the word under cursor |
| Run nearest test / file / suite / last (failures → quickfix, ✓/✗ gutter marks) | `,Tn` / `,Tf` / `,Ts` / `,Tl` (or `:test …`) |
| Convert current output to quickfix | **Ctrl-Q** |
| Open quickfix / next / previous | `,cq` / `:cn` / `:cp` |
| List keymaps / search | `?` shows keys; `/`, `g?`, `n`, `N` search; `,f.` resumes the last search |
| Marks / jumplist | `ma`, `'a`, `` `a `` / Ctrl-O, Ctrl-I |
| Diagnostics / next / previous | `,ld` / `]d` / `[d` |
| Definition / references / outline sidebar / outline list | `gd` / `,lR` / `,lo` (also `,lO`) / `:outline` |
| Peek definition / type / implementation / references in a float | `gpd` / `gpt` / `gpi` / `gpr` (or `,pd` …) |
| Format / rename / code actions | `,lf` / `:rename name` / `,la` |
| Vertical / horizontal split | Ctrl-W v / Ctrl-W s or `,kv` / `,ks` |
| Focus left / down / up / right | Ctrl-W h / j / k / l (Ctrl can stay held for the direction) |
| Focus / close / only pane | Ctrl-W w / Ctrl-W c / Ctrl-W o |
| Side-by-side / full Markdown preview | `,ms` / `,mp` |
| Toggle soft wrap | `,ow` or `:set wrap` / `:set nowrap` |
| Git changes / blame / stage / unstage | `:gitdiff` / `:gitblame` / `:gitstage` / `:gitunstage` |
| GitHub PRs / current PR / diff / review threads / CI checks / issues | `,Gp` / `,Gv` / `,Gd` / `,Gr` / `,Gc` / `,Gi` (`:ghprs`, `:ghpr`, `:ghcheckout N`, …) |
| Recover interrupted-session drafts | `:recover` |
| Save / restore recursive pane layout | `:sessionsave` / `:sessionload` |
| Export / run selected agent feedback | `:reviewexport` / `A` in results |
| Agent output / cancellation | `:reviewresults` / `:reviewcancel` |
| Toggle resolved review notes | `R` in comments, then `,rw` to save |

Results share selection, clipboard export, location navigation and quickfix
conversion. Git staging lists saved-file hunks; Enter applies one hunk after
checking it still applies. The GitHub workspace (via `gh`) is read-first: PR
list and overview, checkout, diff, review threads and CI check logs (failed
steps first) and issues, each as a jumpable list. Formatting and language-server edits remain unsaved
and undoable. Recovery snapshots are written privately after an idle interval;
explicit saves remain essential.

## Lists, outline, and terminals

Results, including quickfix, have context-specific `?` help; `q`/Esc returns to the list. Search uses `/` or `g?`, with `n`/`N` repeats. Task lists use `D`, `O`, and confirmed `dd`; comments retain their own edit/delete/resolution commands. Quickfix keeps task identity and action metadata. Help and delete confirmation consume input before global leaders or Ctrl-Q.

File-picker queries start in Insert submode. Esc leaves query entry; in Normal submode `j/k`, `gg/G`, Ctrl-D/U and PageDown/PageUp move the selection, `:` opens Ex, and leader mappings work. Another Esc dismisses the picker; `i` resumes query entry. Results query/filter fields also support Normal/Insert editing and leader commands outside text entry.

`,lo` / `,lO` toggles the persistent outline sidebar, initially 30% of the current pane width. It selects the enclosing symbol, follows the source cursor while source is focused, and supports hierarchy collapse/expand, kind filtering, hover, paging, `gg/G`, `0/^/$`, and Ctrl-Q export of visible nodes. `:outline` opens the transient symbol list. Focused tree and outline panes display a cursor on the selected row.

`:terminal` / `:term` opens `$SHELL` in a split. Esc leaves Terminal input mode for pane navigation; `i` returns, and closing stops its child. In Normal mode, `Ctrl-\` opens a floating shell, and the same key hides/reopens that live session. `Ctrl-W` unfocuses the float; Esc/Ctrl-Q closes it and stops the child. Embedded terminal probe replies support interactive shell startup. Named agents use the separate detach/reattach workflow below.

Tours use ordinary source and explanation splits, so wrapped text, winbar/minimap/global-statusline geometry, independent cursors, and mouse divider resizing share the normal viewport code. Tour explanation edits are scratch state; they do not automatically rewrite the `.tour` JSON.

## Editing and display

Normal, Insert, character/line/block Visual, operators, motions, text objects,
registers, undo/redo, bounded macros, dot-repeat, regex search and substitution.
Bracketed paste inserts literal text. Saves use atomic replacement and detect
external changes. Quit checks unsaved buffers. A filesystem watcher (`watch`,
default on) reloads unmodified buffers changed on disk (`autoread`), flags
modified ones `[changed on disk]` instead of clobbering them, and keeps the
file tree and file-picker index current without polling.
Multiple cursors: `Ctrl-N` adds one at the next occurrence of the word (or the
Visual selection), Visual-block `Ctrl-N` makes a column of them, `,ma` puts one
on every occurrence and `,mj`/`,mk` add one below/above; Normal-mode commands
and Insert-mode typing then apply at every cursor as one undo step, and `Esc`
collapses them.

Tree-sitter highlights Rust, Python, JavaScript, TypeScript/TSX, Go, C, Bash,
JSON, TOML, YAML, Lua, Vim, CSS, HTML and Solidity. Markdown renders tables, nested lists, styles, links
and highlighted code fences.

Mermaid fences render themed graphical diagrams in terminals supporting Kitty
Unicode placeholders, with Unicode diagrams where supported elsewhere. Rendering
runs locally in a background worker, with cached results and source fallback for
unsupported or incomplete diagrams. In either preview, `h/l` pans wide diagrams,
`+/-` zooms graphical diagrams, and `0` resets zoom/pan. Set `mermaid_preview` to `auto` (default),
`unicode`, `kitty`, or `off`; auto uses Unicode in tmux/screen and WezTerm. Try
[the diagram gallery](examples/mermaid.md).

Display handles tabs, wide characters and
combining graphemes; horizontal motions and deletion respect grapheme boundaries,
and block operations use display columns. Cached rows avoid redrawing unchanged content.
Long-running jobs (LSP `$/progress`, grep, git, `:make`, test runs, formatting, the file
scan) show as a fidget-style stack in the bottom-right corner that fades out
when they finish (`progress = false` or `:set noprogress` to hide it).
`:colorscheme` switches between built-in themes -- the terminal-colored
`default`, `mono`, `warm`, `cool` and the true-color `gruvbox`, `flexoki` and
`tokyonight` palettes (each with a light variant: `gruvbox-light`,
`flexoki-light`, `tokyonight-day`) that theme the background and every UI
surface; `transparent = true` (or `:set transparent`) keeps the terminal's own
background under them.

## Performance

Current measurements, exact commands, hardware, raw results, and historical comparisons are in [BENCHMARKS.md](BENCHMARKS.md). [TECHNICAL_DECISIONS.md](TECHNICAL_DECISIONS.md) explains the implementation choices and what the measurements establish.

## Configuration

Copy [config.example.toml](config.example.toml) to
`~/.config/vaayu/config.toml`. All fields are optional. Named LSP configurations
accept `cmd` argv, `filetypes`, `root_markers`, `env`, `init_options`, `settings`,
and capability overrides. Multiple servers can serve the same language.
Use `:configreload` to reload settings and restart servers. Requests have a
configurable `request_timeout_ms` deadline; `:lspcancel` cancels pending requests.
Completion resolution and common snippet placeholders, choices and linked
fields are supported. File resource edits support ordered create/rename/delete
of regular project files, with preflight checks and rollback on commit failure.

Set `statusline = "%M %f %C"` to include the enclosing-symbol breadcrumb in a
custom statusline. `%C` uses tree-sitter when the current file has a grammar,
and an already-open LSP outline otherwise. The right-side `line:column` ruler
remains visible.

Search supports pattern backreferences, lookarounds and Vim magic/case switches,
with bounded backtracking. Mixed recursive splits support up to 32 panes.
`:sessionsave` persists named-file pane layout and positions; `:sessionload`
restores them without discarding existing buffers.

AI prompts (`:ai` / `,ca`) and code tours (`:toursave` / `:tourexplain`) use
Claude by default. Choose Codex or another interactive CLI with `ai_agent`.
Set executable paths and arguments in `agent_commands`, or omit an entry to
launch that agent by name from `PATH`. For example, place these top-level
options before any TOML tables in `~/.config/vaayu/config.toml`:

```toml
ai_agent = "codex"
agent_commands = { codex = ["/absolute/path/to/codex"], custom = ["/absolute/path/to/agent", "--interactive"] }
```

Use `ai_agent = "custom"` to select the custom command. `:claude`, `:codex`,
and `:agent <name>` open or toggle a specific agent session independently of
this setting. Prompts are pasted into the selected CLI; press Enter there to
submit them.

For agent review, configure top-level `review_command` as an argv array.
`:reviewexport` writes a versioned private JSON packet of selected/current
feedback. `A` in results (or `:reviewrun`) starts the configured command with
that packet on stdin, in the project folder. Its stdout/stderr are retained
privately; `:reviewresults` opens them. Execution is explicit, cancellable and
time-limited. Review completion does not automatically resolve notes.

`:help` opens the [keymap guide](HELP.md). See [AUDIT.md](AUDIT.md) for the
re-audit, verified fixes and limitations, and [BENCHMARKS.md](BENCHMARKS.md) for
performance measurements. Vaayu implements a useful Vim subset; it is not a
complete Vim emulator or a Lua-plugin host.

## Validation and development

See [DEVELOPMENT.md](DEVELOPMENT.md) for builds, all test results, isolated-state instructions, and known test limitations. The Rust toolchain is pinned to `1.99.0`.
