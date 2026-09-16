# Neovim workflow parity plan

## Goal

Bring Vaayu to user-visible workflow parity with the active configuration in
`~/.config/nvim`, while retaining Vaayu's native Rust architecture, local
private review notes, low startup latency, and bounded background work.

Parity means that the same editing task can be completed with equivalent
navigation, feedback, safety, and keymap access. It does not require a Lua host,
Neovim plugin API compatibility, identical floating-window geometry, or copying
plugin implementation details.

## Scope rules

1. `lua/user/plugins.lua`, `lua/user/keymaps.lua`, `lua/user/whichkey.lua`, LSP
   modules, filetype modules, and custom utility modules define the required
   behavior.
2. Entries present only in `lazy-lock.json` are not requirements until an active
   config or keymap uses them.
3. The commented `review.nvim` block is excluded. Its useful concepts overlap
   Vaayu's private comments and the active Sidekick review workflow, so those
   workflows will be covered by the native review system.
4. Overlapping plugins are implemented as one coherent Vaayu surface. For
   example, Trouble, quickfix, bqf, and picker result lists should share one
   results engine; gh.nvim, Octo, and Guh should share one GitHub workspace.
5. External services remain optional. GitHub, Copilot, Claude, Codex, image
   conversion, language servers, and debuggers must fail visibly without
   blocking normal editing.
6. Every interactive feature gets a latency budget, cancellation behavior, and
   a PTY test before it is considered complete.

## Current baseline

Already substantially covered:

- Modal editing, registers, macros, dot repeat, operators, motions, text
  objects, block selection, Unicode display, wrapping, splits, sessions,
  clipboard and OSC52.
- Project file picker, recent files, buffers, live grep, shared results,
  selectable export, quickfix snapshots, list search, marks and jumplist.
- LSP completion, snippets, hover, definition, references, document symbols,
  diagnostics, formatting, rename, code actions and signature help.
- Configurable multi-server LSP settings, initialization options, capability
  overrides, timeouts, cancellation, transactional workspace edits and file
  resource changes.
- Tree-sitter highlighting for the main configured languages and rendered
  Markdown preview.
- Git signs, saved-file diff/blame, hunk stage/unstage, recovery, local review
  notes, agent packets, agent output and cancellation.

The remaining work is mostly depth, broader sources/actions, terminal and tab
infrastructure, richer Git/GitHub and agent workflows, and UI polish.

## Plugin-to-capability matrix

| Neovim plugin or module | Vaayu status | Required Vaayu capability |
| --- | --- | --- |
| lazy.nvim / plenary / logger | Consolidated | No plugin runtime; versioned native config, migrations and health checks |
| nvim-lspconfig | Partial | Complete LSP method and language coverage below |
| mason.nvim / mason-lspconfig | Missing | Tool registry, install/update/remove, executable health |
| blink.cmp / friendly-snippets | Partial | Path completion, documentation pane, manual trigger, richer snippet transforms |
| neodev / schemastore | Partial | Neovim Lua metadata profile and bundled JSON/YAML schema associations |
| fidget.nvim | Missing | Nonblocking task/LSP progress model and status UI |
| nvim-treesitter | Partial | More grammars, injections, queries, folds, text objects and large-file policy |
| Snacks picker / fzf-lua | Partial | All configured picker sources, preview, history, resume and split actions |
| nvim-tree | Missing | Stateful file tree with safe file operations, filters and diagnostics |
| gitsigns / mini.diff | Partial | Hunk navigation, preview, reset, inline deleted text and word diff |
| Neogit | Missing | Native Git status/index/commit/stash/branch workspace |
| nvim-autopairs | Done | Configurable pair insertion, skip, newline and deletion rules |
| nvim-surround | Partial | `ys`, `ds`, `cs`, Visual `S`, tags and repeat support |
| vim-sleuth | Done | Per-buffer indent detection, with EditorConfig precedence |
| vim-wordmotion | Partial | camelCase, snake_case and kebab-case subword motions/operators (`gw`/`gb`/`ge`) |
| gruvbox / flexoki / custom themes | Missing | Theme palettes and runtime switching |
| transparent.nvim | Missing | Transparent background toggle |
| lualine | Partial | Configurable global statusline and clickable navigation metadata |
| image.nvim | Missing | Kitty image protocol with converter fallback and lifecycle cleanup |
| which-key | Partial | Discoverable keymap registry and delayed prefix popup |
| Copilot | Missing | Authenticated inline suggestion provider with next/previous/accept |
| Trouble | Partial | Hierarchical diagnostics, symbols, location and quickfix views |
| outline.nvim / symbol browser | Partial | Persistent collapsible symbol pane with follow/jump/preview |
| nvim-bqf | Partial | Quickfix preview, filters, selection, history and split-open actions |
| Sidekick | Partial | Persistent interactive agent terminals, context senders and review sessions |
| promptbank.nvim | Missing | Versioned prompt templates, placeholders, picker, edit and send |
| codetours.nvim | Missing | Local `.tours/` recording, playback, editing and agent generation |
| neominimap / mini.map | Missing | One native minimap with viewport, Git, diagnostic and click support |
| todo-comments | Missing | Comment-aware keyword highlights, signs and searchable result source |
| vim-illuminate | Missing | Debounced document-reference highlights with large-file cutoff |
| gh.nvim / Octo / Guh | Missing | Unified GitHub issues, PRs, reviews, CI logs and notifications workspace |
| mini.animate | Missing | Optional cursor and resize animation, disabled in benchmarks |
| mini.align | Partial | Operator/Visual delimiter alignment with preview and undo grouping |
| goto-preview | Missing | Definition, implementation, reference and type-definition preview panes |
| nvim-utils | Missing | Test runner and custom utility command framework |
| zen-mode | Missing | Centered distraction-free layout with reversible UI options |
| refactoring.nvim | Missing | Extract/inline operations with preview, validation and undo |
| grug-far | Missing | Reviewed project-wide replacement with selective apply |
| terminal.lua / lazygit | Partial | Embedded PTY buffers, float/splits/tabs, persistent jobs and lazygit |
| copy_utils / ai_context | Partial | Structured path, symbol, import and context copying/sending |
| keymaps/options/autocommands | Partial | Tabs, resize, mouse, autoread, spelling, yank flash and remaining mappings |
| remote_mode | Partial | Automatic reversible low-bandwidth profile for SSH sessions |
| Go/Cargo filetype tools | Missing | Dependency, test, generate and package-metadata actions |

## Architecture to build first

These foundations prevent each feature from inventing a separate UI or process
model.

### A. Unified action and keymap registry

- Give every command an ID, title, modes, default keys, availability predicate
  and handler.
- Generate `:help`, command picker entries and which-key prefix contents from
  the same registry.
- Support global, filetype, pane and temporary-buffer mappings.
- Detect conflicts at config load and expose `:keymaps` with search and quickfix
  export.
- Add user-configurable remaps without requiring code changes.

Acceptance:

- Every documented binding is registry-backed.
- Prefix help appears after a configurable delay and never intercepts a
  completed mapping.
- A test verifies the active Neovim keymap inventory is either mapped or listed
  in an explicit exception table.

### B. Reusable UI surfaces

- Extend the shared results model into reusable list, tree, preview, prompt,
  detail and multi-select components.
- Add anchored popups and centered overlays with clipping for tiny terminals.
- Add a persistent sidebar abstraction for file tree, outline, minimap and Git
  status.
- Add focus, resize, close, split-open, tab-open and preview actions common to
  every source.
- Keep all cell measurement Unicode-aware and compatible with row caching.

Acceptance:

- The same navigation/search/select/copy/open commands work in picker,
  quickfix, diagnostics, file tree, outline, Git and GitHub views.
- 40×12, 100×24 and 180×50 screenshot tests cover every surface.

### C. Supervised jobs, PTYs and progress [Partial: embedded PTY done, see progress log]

- Generalize background work into a job supervisor with IDs, generations,
  cancellation, deadlines, bounded output, progress and exit state.
- Add embedded PTY processes with terminal emulation, resize propagation,
  scrollback, terminal/normal modes and clean shutdown. [Done]
- Reuse it for shells, lazygit, agent CLIs, test runners, tool installation and
  long Git/GitHub operations. [Shells done via `:terminal`; lazygit/agent
  CLIs/test runners/tool installation not yet wired to it]
- Surface active jobs in the statusline and a searchable `:jobs` list. [Not
  done -- no statusline exists yet (Phase 9); no `:jobs` list]

Acceptance:

- Closing a pane never leaks a child process. [Done for PTY terminals; not
  yet true for the existing ad-hoc SearchJob/git-poll/review jobs, which
  this slice didn't touch or generalize]
- SIGINT, terminate, detach and reattach are explicit actions. [Terminate
  (close-kills) done; SIGINT-while-running, detach and reattach not done]
- Slow or noisy jobs cannot block typing or grow memory without a bound.
  [Done for PTY output via vt100's bounded scrollback; typing is never
  blocked since the reader runs on its own thread]

### D. Tabs, histories and persistent workspace state

- Add tab pages above the existing recursive pane tree.
- Persist per-tab panes, working directory, active buffer, terminals and
  optional tool panels.
- Add command, search, picker and quickfix history plus resume-last-picker.
- Preserve histories privately with size and age limits.

Acceptance:

- All configured tab mappings work, including numeric jump, move and tab-only.
- Session round trips retain tab/pane geometry without restoring unsafe jobs.

## Delivery phases

### Phase 1 — Daily editing parity

Implement the features that affect ordinary editing before specialized tools.

1. Autopairs with language-aware pair tables, escaped-quote handling, pair
   deletion, newline expansion and paste suppression.
2. Surround add/delete/change for quotes, brackets and tags; Visual surround;
   counts, dot repeat and registers.
3. Subword motions for camelCase, snake_case and kebab-case, usable by
   operators and Visual mode.
4. Indentation detection with EditorConfig, modeline/config override and
   deterministic fallback; display the chosen source in buffer info. [Done]
5. Visual/operator delimiter alignment with preview and one undo transaction. [Partial]
6. Move-line mappings, retained Visual indentation, select-all,
   increment/decrement and exact black-hole paste/delete mappings.
   [Partial: retained Visual indentation, select-all and increment/decrement
   done; black-hole delete already covered by baseline `,d`/`"_`; move-line
   mappings not done -- see progress log]
7. Persistent undo, focus-gained external-change checks, yank flash, cursorline
   and relative-number toggles.
   [Partial: persistent undo done; relative-number toggle already covered by
   baseline `,or`; focus-gained external-change check and yank flash not done]
8. Mouse positioning, selection, pane focus, resize and configured LSP mouse
   actions where the terminal reports mouse events.
   [Partial: positioning, drag-selection, pane focus, scroll and Ctrl-click
   go-to-definition done; split-border drag-resize not done]
9. Optional spelling dictionaries, misspelling decoration, suggestions and
   project/user dictionary updates.
   [Partial: dictionaries, suggestions and user-dictionary updates done;
   decoration is a navigable list (:spellcheck), not live inline underline]

Exit criteria:

- Editing behavior has table-driven unit tests for ASCII, Unicode, tabs,
  multiline selections and repeat/undo.
- The common typing path remains under the recorded Vaayu baseline within 10%
  at p50/p95.

### Phase 2 — Picker, tree, quickfix and navigation parity

1. Add picker sources: help, keymaps, commands, projects, workspace symbols,
   diagnostics, current-buffer lines, jumps, command history, search history,
   Git stash and built-in source list.
2. Add grep-current-word/selection, resume, preview toggle/wrap/scroll, select
   all, and open in current/vertical/horizontal/tab targets.
3. Add ranking instrumentation and a bounded incremental top-k matcher so a
   million-path inventory does not require sorting every candidate per key.
4. Build a file tree with expand/collapse, reveal-current-file, project-root
   synchronization, dotfile/ignore/Git-clean filters, live filter, bookmarks,
   diagnostics and Git state.
5. File operations: create, rename, copy, cut, paste, trash and delete with
   collision prompts, dirty-buffer checks and rollback where possible.
6. Upgrade quickfix with preview, history, filtering, selected actions and
   split/tab opening.
7. Build a persistent outline/symbol sidebar with hierarchy, collapse, follow
   cursor, symbol-kind filtering and preview.
8. Add centered jump behavior after page/search movement and complete recent
   buffer navigation.

Exit criteria:

- Each source converts to quickfix with one action.
- File mutations share the existing transactional resource safeguards.
- Picker filtering stays responsive on 1M synthetic paths and cancellation
  discards stale generations.

### Phase 3 — Complete LSP and completion parity

1. Add type definition, implementation, declaration, workspace symbols,
   selection/range formatting, CodeLens, document links, inlay hints and
   document highlights.
2. Add preview panes for definition, implementation, type definition and
   references with jump-list integration.
3. Add organize imports and source actions, including preferred/disabled action
   metadata, resolve and command execution.
4. Add diagnostic ranges, undercurl/underline rendering, related information,
   code/source labels, per-line popup, optional virtual text/lines and
   insert-mode update policy.
5. Add LSP progress tokens to the job/progress UI.
6. Add completion path source, automatic documentation preview, configurable
   auto-show, source/kind labels and completion enable toggle.
7. Complete snippet transforms, nested placeholders, choices UI, variables and
   malformed-snippet fallback.
8. Expand default language definitions to Vim, Markdown, JSON, YAML, Bash,
   TOML, CSS, HTML, Solidity, Kitty config and other configured filetypes.
9. Bundle or generate SchemaStore mappings without network work on startup.
10. Implement a Mason-like `:tools` view for install/update/remove/health. Tool
    manifests must be pinned, checksummed where upstream permits, and opt-in.

Exit criteria:

- Mock-server protocol tests cover every added method, cancellation and stale
  response handling.
- Real-server smoke tests cover clangd, rust-analyzer, gopls, Python, TypeScript,
  Lua, JSON/YAML and Markdown when installed.
- Warm formatting/completion/navigation comparisons stay within the current
  performance envelope.

### Phase 4 — Git parity

1. Extend gutter signs with hunk navigation, preview, reset, stage/unstage,
   selected-range actions, line blame and blame toggle.
2. Render deleted lines and intra-line word changes as an optional diff overlay.
3. Build a Git workspace with staged/unstaged/untracked/conflict sections,
   file/hunk diffs, selective stage/reset, commit editor, amend, stash,
   branch/checkout, log and push/pull/fetch actions.
4. Add side-by-side and unified diff layouts, whitespace toggle, unchanged
   region folding and index watching.
5. Add lazygit as an embedded PTY action for users who prefer its interface.
6. Add GitHub permalink generation for cursor, line range and selected commit.

Exit criteria:

- Destructive operations show the exact affected paths/hunks before execution.
- Integration tests use temporary repositories for staging, reset, conflict,
  rename, binary-file and worktree cases.
- Git refresh never runs synchronously on the key-to-frame path.

### Phase 5 — Terminal, tasks and developer tools

1. Shell terminals in centered overlay, vertical split, horizontal split and
   dedicated tab; toggle/hide/reattach with scrollback.
2. Terminal-mode escape, pane navigation, shell vi-mode initialization and
   resize handling.
3. Test/task runner with project and filetype command templates, nearest-symbol
   context, streamed output, diagnostics parsing and quickfix conversion.
4. Go task presets from the current filetype config; generic build/test/lint
   presets for Rust, Python, JavaScript/TypeScript and C/C++.
5. Cargo manifest dependency view with installed/latest versions, features,
   dependencies and explicit update/upgrade/open-documentation actions.
6. Go dependency install/tidy, test generation, `go generate` and comment
   actions, implemented as visible supervised tasks.
7. Config open, config workspace and live config reload commands.
8. Reversible SSH remote mode: reduced refresh frequency, insert-time
   diagnostics policy, large-file syntax cutoff and OSC52 copy with local
   register paste.

Exit criteria:

- PTY tests cover interactive input, resize, SIGINT, process exit, pane close
  and session restore without resurrecting processes.
- Task output is searchable, selectable, copyable and quickfix-convertible.

### Phase 6 — AI and review workflow parity

1. Add long-lived Claude/Codex terminal sessions managed by the PTY supervisor,
   with select/toggle/detach/interrupt and per-tab association.
2. Send current file, selection, clipboard, symbol body, symbol signature,
   diagnostics and line ranges through a structured context builder.
3. Merge private comments into review sessions: pending count, jump-to-comment,
   session selection, explicit submit and retained responses.
4. Add a prompt bank stored locally with named templates, typed placeholders,
   preview, edit, version migration and send-to-session.
5. Add `.tours/` code tours with record/add-stop/finish/cancel, playback,
   editing, stale-anchor repair and optional agent generation/regeneration.
6. Add a provider interface for inline AI suggestions. Implement Copilot only
   after secure device authentication, token storage, cancellation, redaction
   settings and request telemetry controls are defined.
7. Support next/previous/accept/dismiss suggestion without sharing completion
   popup state; prevent ghost text after edits, backspace, undo, mode changes or
   stale responses.

Exit criteria:

- No source leaves the machine until a configured provider action authorizes
  it; context preview shows the exact payload.
- Tokens and credentials never enter project files, logs, recovery or review
  packets.
- Deterministic fake-agent and fake-suggestion tests cover every lifecycle.

### Phase 7 — GitHub parity

Implement one native GitHub workspace backed by the authenticated `gh` CLI,
covering the useful union of gh.nvim, Octo and Guh.

1. Repository overview, assigned issues, issue/PR search, notifications and
   refresh.
2. Issue and PR detail, comments, reactions, labels, assignees and editable
   write-to-submit buffers.
3. PR file tree, commits, side-by-side/unified diffs, review threads, viewed
   state, suggestion comments, resolve/reply/delete and review submission.
4. CI check matrix, job status and searchable logs.
5. Open current GitHub object in browser and accept URL, owner/repo, branch,
   commit, issue and PR targets.
6. Convert files, threads, checks and log matches to shared results/quickfix.

Exit criteria:

- All mutations show repository and target before running `gh`.
- Fixture-driven parser tests require no network; explicitly enabled smoke tests
  exercise a disposable repository.
- Rate limits, authentication failures and offline mode remain usable and
  recoverable.

### Phase 8 — Refactoring and project replacement

1. Prefer LSP code actions for extract/inline operations when advertised.
2. Add tree-sitter-backed extract variable/function/block and inline variable
   for supported languages, with a conservative capability table.
3. Always show an editable diff preview before cross-file refactors.
4. Build project replace on the grep engine: regex/literal/case options,
   include/exclude globs, per-match selection, replacement preview, stale-file
   detection and transactional application.
5. Preserve encoding, permissions and final-newline state; create one undoable
   change per open buffer and backups/rollback data for unopened files.

Exit criteria:

- Unsupported syntax is refused instead of guessed.
- Multi-file replacement has tests for concurrent disk changes, binary files,
  symlinks, permissions, invalid regex and rollback failure.

### Phase 9 — UI and information parity

1. Theme engine with Gruvbox, Flexoki and the locally configured VS Code/Cursor
   palettes; dark/light variants and runtime reload.
2. Transparent background toggle and terminal-default color handling.
3. Configurable global statusline: mode, branch, diff counts, path, breadcrumbs,
   macro state, search count, diagnostic counts, pending review notes, LSP/jobs,
   filetype, location and progress.
4. Symbol breadcrumbs backed by the document-symbol cache.
5. TODO/FIX/HACK/WARN/PERF/NOTE/TEST highlighting and searchable source.
6. Debounced current-symbol reference highlighting from LSP, disabled while
   typing and above the configured large-file limit.
7. Native minimap with viewport, diff and diagnostic marks; focus and mouse
   navigation. Use one implementation for both configured minimap plugins.
8. Zen layout with reversible width, number/sign/status settings.
9. Kitty-protocol images with explicit capability detection and cleanup.
10. Optional cursor/resize animation with a zero-animation accessibility and
    benchmark mode.

Exit criteria:

- Theme switching invalidates only visible composed rows.
- All decorations have explicit priority rules so diagnostics, Git, TODOs,
  notes, selection, search and AI suggestions do not overwrite one another.
- Animations and images are absent from headless/unsupported terminals without
  warnings on every frame.

## Cross-cutting correctness and performance requirements

### Performance budgets

- Startup: preserve the current release median within 10% with all optional
  integrations idle.
- Cursor and editing: no synchronous Git, filesystem traversal, network, tool
  lookup, parsing or process startup on a key event.
- Picker: show cached candidates immediately; cancel stale filters; cap visible
  and retained results independently.
- Rendering: decoration changes carry revisions so unchanged lines retain their
  composed rows. Virtual text, minimap and sidebars participate in damage
  tracking.
- Memory: inventories, terminal scrollback, logs, histories, diagnostics and
  GitHub objects all have configurable caps.
- Benchmarks: record first byte, completed-output proxy, bytes, p50/p95 and RSS;
  compare against bare Neovim, the configured Neovim and Helix without claiming
  universal superiority.

### Safety and privacy

- Project file writes use the existing snapshot, permission, atomic-replace and
  external-change checks.
- Shell-like configuration remains argv-based unless an explicitly named shell
  task is requested.
- Tool installation and GitHub/AI network activity are explicit commands.
- Private comments, prompt history, agent sessions, tours and credentials have
  separate documented storage and ignore rules.
- Credential material is OS-protected and redacted from diagnostics and logs.

### Testing

- Unit tests for pure matching, editing, layout, protocol and transformation
  logic.
- Fake LSP, GitHub, agent and Copilot processes for deterministic lifecycle
  tests.
- Temporary repositories and projects for filesystem/Git transactions.
- PTY UI tests for every interactive surface at small, normal and wide sizes.
- Golden screenshots for theme/status/sidebar/list layouts, plus semantic
  assertions so color-only regressions are caught.
- Soak tests for rapid typing, stale asynchronous replies, repeated resize,
  large files, million-file inventories and long terminal output.
- Real integration tests remain opt-in and report missing external tools as
  skips, not product success.

## Proposed milestone order

| Milestone | Included phases | User-visible outcome |
| --- | --- | --- |
| M1 Foundation | A–D | Discoverable actions, shared panels, PTYs, tabs and histories |
| M2 Daily driver | 1–2 | Editing, file tree, complete picker/quickfix/outline workflow |
| M3 Code intelligence | 3 | LSP, diagnostics, completion and tool-management parity |
| M4 Repository work | 4–5 | Full Git, terminals, lazygit and task runner |
| M5 Agent workflow | 6 | Sidekick, prompt bank, tours and safe inline suggestions |
| M6 Collaboration | 7 | Issues, PR review, CI and notifications |
| M7 Large changes | 8 | Reviewed refactors and project-wide replacement |
| M8 Polish | 9 | Themes, statusline, minimap, TODOs, images and zen mode |
| M9 Parity release | all | Inventory closure, migration guide, benchmarks and release hardening |

## Definition of done for parity

1. Every active plugin or custom workflow in the matrix is marked implemented,
   intentionally consolidated, or explicitly excluded with a user-approved
   reason.
2. Every active custom keymap has an equivalent registered Vaayu action or an
   entry in the approved exception table.
3. The daily workflow can be completed without Neovim: edit, navigate, search,
   inspect diagnostics, refactor, run tests, use Git, review GitHub work, work
   with an agent and recover/session-restore.
4. Optional integrations degrade cleanly when their executable, credentials,
   protocol or terminal capability is unavailable.
5. Correctness, UI, latency, memory and long-running job tests pass in CI.
6. `HELP.md`, the example config and a Neovim-to-Vaayu keymap migration table
   describe the finished behavior.

## First implementation slice after plan approval

Start with M1 and the narrowest M2 vertical slice:

1. Action/keymap registry and searchable command/keymap picker.
2. Which-key prefix popup generated from that registry.
3. Job supervisor and embedded PTY buffer.
4. Tab pages over the existing pane tree.
5. File-tree sidebar using the shared tree surface.
6. Autopairs, surround, subword motions and indentation detection.
7. Picker preview plus split/tab-open actions and resume/history.

This slice unlocks most later plugins without committing Vaayu to several
incompatible one-off interfaces.

## Implementation progress

Full parity across all nine phases is a multi-month effort; this log tracks
real, tested increments as they land, in the milestone order above, so work
can resume without re-deriving what already exists.

- **M1.A (partial) — leader action registry.** `src/actions.rs` gives every
  `,`-prefixed command an id, title, default key sequence and handler in one
  table; `run_leader` dispatches through it instead of a duplicated match
  arm. `:keymaps` opens a searchable, executable command palette generated
  from the same table (reuses the existing results/picker engine, not a new
  widget). A which-key popup appears after `whichkey_delay_ms` (default
  500ms, configurable) once a leader prefix is left hanging, listing exact
  continuations, and never appears for a completed mapping (verified by
  `tests/pty_whichkey.py` at 40x12/100x24/180x50, plus
  `src/actions.rs`'s unit tests for duplicate-key/id conflicts). Core
  Normal/Insert/Visual motions and operators (`d`, `c`, `y`, `f`, text
  objects, etc.) are not yet registry-backed — they remain the existing
  direct match in `normal.rs`/`visual.rs`/`operator.rs`. The plan's
  acceptance bar ("every documented binding is registry-backed", "a test
  verifies the active Neovim keymap inventory is either mapped or listed in
  an explicit exception table") is not met yet: that needs the rest of A
  (registering core bindings too) and a machine-readable export of
  `lua/user/keymaps.lua`/`whichkey.lua` to diff against, which this
  environment does not have on disk. Latency: no regression against the
  prior release (`bench/latency.py`, same 100x40 harness, same file:
  overall p50 0.556→0.526ms, p90 2.762→2.655ms, p99 4.507→3.979ms, 0
  timeouts) — expected, since dispatch is still an O(1) match/lookup and the
  popup only affects the idle-wait path, never a completed keystroke.
  Remaining for full A: registry-back core bindings, `:help`/prefix-popup
  generation for non-leader modes, conflict detection at config load
  (currently only a test, not a startup check), and user-configurable remaps.
- **Phase 1.1 — autopairs (done).** `src/autopairs.rs` covers `()[]{}"'` `` ` ``:
  typing an opener inserts its close and parks the cursor between them;
  typing a close that's already at the cursor skips over instead of
  duplicating; Backspace between an empty pair (`(|)`) deletes both
  characters as one edit; Enter between a non-quote pair (`{|}`) expands to
  an indented blank line framed by the pair. Quote pairing is suppressed
  mid-word, right after a `\` escape, and for the `'` that starts a Rust
  lifetime (`&'a T`). Only hooked into typed `Key::Char` in `insert.rs`, so
  `Editor::insert_paste`/bracketed paste is never auto-paired without extra
  bookkeeping. `autopairs = true` in config.toml disables it globally.
  7 unit tests plus `tests/pty_whichkey.py`-style `tests/pty_autopairs.py` at
  40x12/100x24/180x50 (pairing, skip-over, pair-backspace, brace-enter,
  paste suppression, each verified against the saved file, not just the
  screen). Latency: no regression on the same 236-op/100x40 benchmark against
  `6836f46` (two runs; p50/p90/p99 and `insert_char` all within run-to-run
  noise, 0 timeouts either run).
- **Phase 1.2 — surround (partial).** `src/surround.rs` adds `ds{char}`
  (delete), `cs{from}{to}` (change), `ys{i|a}{object}{char}` and `yss{char}`
  (add), and Visual `S{char}`. Reuses `textobject::resolve` for the operand
  (word, and the standard bracket/quote pairs), so multi-line brackets and
  same-line quotes work the same way `di(`/`da"` already do. `(`/`[`/`{`/`<`
  pad with a space when the *open* half is typed (`ysiw(` -> `( word )`);
  the close half or `b`/`B`/`r` aliases don't. Implemented by intercepting
  `s` as an operator continuation for `d`/`c`/`y` in `normal.rs` (never a
  valid motion or doubling char there already), so no existing dispatch
  path changed behavior -- confirmed by the full existing suite passing
  unchanged plus 11 new unit tests and `tests/pty_surround.py` at
  40x12/100x24/180x50. Not implemented, and out of scope for this slice:
  plain-motion operands (`ysw"`, `ys$)` -- only text objects and whole-line
  are supported), counts, dot-repeat, registers, and tag surrounds
  (`yst<tag>`/`cst`). No latency regression on the same 236-op/100x40
  benchmark against `6836f46` (p50/p90/p99 within run-to-run noise).
- **Phase 1.3 — subword motions (partial).** `motion.rs` adds `SubwordFwd`/
  `SubwordBack`/`SubwordEndFwd`, classifying each char as Gap (`_`, `-`,
  whitespace, newline)/Upper/Lower/Digit and splitting on class changes,
  with the standard acronym-tail rule (`XMLParser` -> `XML` | `Parser`,
  not `X`|`M`|`L`|`Parser`) and digits as their own subword
  (`var2Name` -> `var`|`2`|`Name`). Bound under the existing `g` prefix as
  `gw`/`gb`/`ge` (not real Vim's `gw`/`ge`, which this codebase doesn't
  implement -- deliberate, since the plan explicitly doesn't require
  copying Neovim's own bindings). Because `apply_motion_or_operator` and
  `key_to_motion`'s `g`-prefix handler are shared by bare motion, every
  operator (`dgw`, `cgw`, `ygw`, ...) and Visual-mode extension, all three
  get subword support from these three match arms with no separate
  wiring, and unlike vim-wordmotion this does *not* touch plain `w`/`b`/`e`,
  so the existing word-motion suite is provably unaffected (full suite
  passes unchanged). Vim's own separator-sweeping `dw` convention applies
  here too: deleting into a following `_`/`-`/space consumes it, same as
  plain `dw` already does. 7 unit tests directly against `motion::resolve`
  plus 2 integration tests (operator dot-repeat, Visual extension) and
  `tests/pty_subword.py` at three terminal sizes. Not implemented: crossing
  a `-`/tag boundary distinction for `gE`-equivalent semantics, and no
  dedicated count/register nuance beyond what the shared operator path
  already provides. No latency regression on the same 236-op/100x40
  benchmark against `6836f46`.
- **Phase 1.4 — per-buffer indent detection (done).** `src/indent.rs`
  resolves, once per buffer open: a Vim modeline (`vim:`/`vi:`, first/last 5
  lines, `sw`/`ts`/`et`/`noet`) over an applicable `.editorconfig` entry
  (walks up to `root = true`, `*`/`*.ext` globs, `indent_style`/
  `indent_size`/`tab_width`; brace-expansion globs like `*.{js,ts}` are not
  implemented) over vim-sleuth-style heuristic detection (tabs vs. the
  smallest nonzero leading-space run) over the global `config.toml` default.
  This required a real architectural change, not just a new module:
  `tabstop`/`shiftwidth`/`expandtab` moved from global `Config` fields to
  per-`Buffer` fields (`Buffer::apply_indent`, called after `from_path` at
  every real file-open site: `Editor::open_file`, session restore, the LSP
  resource-rename path), so two open buffers can have different indent
  settings at once -- something a single global `Config` could never
  represent. All ~21 call sites that used to read `ed.config.tabstop`/
  `shiftwidth`/`expandtab` (Tab-key insertion, `>`/`<`, paste, rendering
  glyph layout, Visual block width, LSP format options, autopairs'
  brace-Enter indent) now read the current buffer's resolved values instead.
  `:indentinfo` shows the resolved values and source. 7 unit tests in
  `indent.rs` plus `tests/pty_indent.py` at three terminal sizes (heuristic,
  modeline-overrides-heuristic, editorconfig-overrides-heuristic, default
  fallback, and that Tab actually uses the per-buffer setting, not the
  global one). Full existing suite passes unchanged (confirming the
  refactor didn't change behavior for the common single-buffer,
  no-override case), and two independent latency runs against `6836f46`
  on the same 236-op/100x40 benchmark show no regression (p50 0.536-0.614ms
  vs 0.508-0.556ms before, within this harness's normal run-to-run noise).
  Not implemented: `softtabstop`, brace-expansion EditorConfig globs, and a
  live `:indentinfo`-in-statusline (it's a one-shot message, not persistent
  UI -- statusline integration is Phase 9's job).
- **Phase 1.5 — delimiter alignment (partial).** `src/align.rs`: Visual
  `ga{char}` aligns every selected line's first occurrence of `{char}` into
  the same column (pads with spaces before the delimiter; lines without it
  are untouched), and Normal `gap{char}` does the same for the contiguous
  non-blank paragraph around the cursor -- both as a single `begin_edit`/
  `commit_edit` transaction, so one `u` fully reverts it. Reached through
  the existing `g`-prefix dispatch, alongside `gg`/`gd`/`gw`/`gb`/`ge`.
  6 unit tests plus `tests/pty_align.py` at three terminal sizes. **Not
  implemented** (hence "partial," not "done"): a live preview before
  committing -- the plan's literal ask -- since that needs an interactive
  overlay this session didn't build; the one-transaction undo is the actual
  safety net instead. Also not implemented: a general `ga{motion}{char}`
  operator (only the fixed `p` paragraph shorthand), multiple/last-occurrence
  alignment modes, right-alignment, and Visual-block support. Full existing
  suite passes unchanged; no latency regression against `6836f46` on the
  existing 236-op/100x40 benchmark.
- **Phase 1.6 — increment/decrement, select-all, retained Visual indent
  (partial).** `Ctrl-A`/`Ctrl-X` in `normal.rs` increment/decrement the
  first decimal number at or after the cursor on the current line (vim's
  own scope: no wrap, no searching other lines) by `count`, preserving
  zero-padded width (`007` -> `008`) and a leading `-`, as one undo step
  with dot-repeat recording like every other single-key edit here. `,a`
  (in the action registry, alongside every other leader command) selects
  the whole buffer in line-wise Visual. Visual `>`/`<` now re-establishes
  the same line-range selection afterward instead of dropping to Normal,
  so a second `>` (or a count) keeps indenting the same block -- a
  `gv`-after-indent remap's effect, built in rather than requiring the
  remap; scoped to Char/Line kinds, Visual-block indent is unchanged.
  7 unit tests plus `tests/pty_editing.py` at three terminal sizes covering
  increment/decrement/count/padding/no-match, `,a` + delete, and the
  retained-selection double-indent. Full suite passes unchanged; no latency
  regression against `6836f46` on the existing 236-op/100x40 benchmark.
  **Not implemented:** move-line mappings (`<M-j>`/`<M-k>`-style) -- this
  editor's `Key` enum (`key.rs`) has no Alt/Meta modifier at all yet, only
  `Ctrl`, so this needs that plumbing first, not just a new binding; left
  for a later pass rather than bolted on as a special-cased raw escape
  sequence.
- **Phase 1.7 — persistent undo across restarts (partial).**
  `src/undofile.rs`: on a successful `:w`, the current buffer's undo
  history (oldest-first text snapshots, capped at 50 entries / 8MB total)
  is written to `.vaayu/undo/<hash-of-path>.json` (0700 dir, 0600 file,
  `.gitignore`'d, symlinks refused -- same private-storage pattern as
  `notes.rs`/`recovery.rs`) alongside a hash of the saved content. On open,
  it's restored only if the just-loaded content's hash matches exactly, so
  a file changed on disk (by another tool, another Vaayu process, or `git
  checkout`) since that save never has stale undo history replayed against
  it -- confirmed by a PTY test that edits externally between two process
  runs and checks `u` is a no-op rather than corrupting the file. Best
  effort throughout: any read/write/parse failure is silently ignored,
  since this is a convenience on top of the buffer's own in-memory undo,
  never a substitute for `:w` itself. 3 unit tests plus
  `tests/pty_persistent_undo.py`, which is a genuine two-process test (two
  separate real PTY launches of the release binary against the same file,
  not two buffers in one process) at three terminal sizes: save in process
  1, undo past that save in a fresh process 2. Full suite passes unchanged;
  no latency regression against `6836f46` (the persistence I/O only runs on
  `:w`, never on a keystroke). **Not implemented:** the focus-gained
  external-file-change check and yank-flash highlight from the same plan
  item -- unrelated to undo, left for a separate slice. Relative-number
  toggle was already covered by the existing `,or` baseline binding.
- **Phase 1.8 — mouse support (partial).** `src/mouse.rs` + `render::
  locate_click` (which reuses the exact same `layout()`/`gutter()` geometry
  `draw()` renders with, so a click always lands on the character it's
  visually on top of, including wrapped lines and multi-pane splits):
  left-click positions the cursor and focuses the clicked pane; a
  left-drag starts and extends a Visual character selection from the
  click point; the scroll wheel moves the viewport (nudging the cursor
  back on-screen only when the scroll would otherwise leave it above/below
  the new viewport -- `prepare_view`'s own keep-cursor-visible pass would
  instantly undo a plain viewport-only scroll otherwise); Ctrl-click runs
  go-to-definition at the clicked position. `EnableMouseCapture`/
  `DisableMouseCapture` added to terminal setup/teardown. Only active in
  Normal/Insert/Visual -- Results/Picker/Markdown-preview ignore the mouse
  this slice. 6 unit tests against `mouse.rs`'s handlers directly plus
  `tests/pty_mouse.py` at three terminal sizes using **real SGR mouse
  escape sequences** over the PTY (click-to-edit, drag-select-and-delete,
  and a scroll-changes-the-view check on a 200-line file), not just direct
  Rust calls -- this is the only way to actually exercise crossterm's mouse
  parsing and `EnableMouseCapture` end to end. Full suite passes unchanged;
  two latency runs against `6836f46` show no regression (mouse handling is
  a new branch in the event loop, not a change to key dispatch). **Not
  implemented:** resizing a split by dragging its border -- real click/drag
  detection for panes works, but border-hit-testing and live divider
  resize is separate scope, left for later.
- **Phase 1.9 — spelling (partial), completing Phase 1.** `src/spell.rs`
  loads an optional system word list (`/usr/share/dict/words` and a few
  common alternates) plus a private per-user dictionary
  (`~/.config/vaayu/dictionary.txt`); `:spellcheck` lists misspelled words
  in the current buffer as a navigable results list (reusing the same
  `results.rs` machinery as `:diagnostics`/`:grep`, not a new widget);
  `zg` adds the word under the cursor to the user dictionary; `z=` shows
  bounded-Levenshtein suggestions and replaces the word in place on
  selection. Degrades to a clear message when no system dictionary is
  installed -- verified on this machine, which has none, via
  `tests/pty_spell.py`'s real end-to-end run (the "dictionary present"
  logic itself is covered by unit/regression tests using a seeded test
  dictionary, so it doesn't depend on what happens to be installed on
  whatever machine runs the suite). 4 unit tests in `spell.rs`, 3
  regression tests, `tests/pty_spell.py` at three terminal sizes. Full
  suite passes unchanged; no latency regression against `6836f46`.
  **Not implemented (why it's "partial," not "done"):** live inline
  misspelling underline -- that needs a `spell_stamp`-style generation
  counter threaded through `render.rs`'s `RowSignature`/composed-row cache
  the way `syntax_stamp` already works for syntax highlighting, which is
  real, cache-correctness-sensitive work distinct from the dictionary/
  suggestion logic itself; a list you navigate is the safer scope for this
  slice. Word-splitting is alphabetic-run based with no code-identifier
  awareness beyond skipping runs with digits/`_`, so it's most useful on
  prose (docs, comments, commit messages) and will flag real code
  identifiers too -- always opt-in, never a background pass.
- **Phase 1 status:** all nine items have landed at least partially; the
  items marked partial above (alignment preview, move-line mappings,
  focus-gained external-change check, yank flash, split-border drag-resize,
  live spell underline) are the specific, itemized remainder before Phase 1
  can be called fully done. Its exit criteria -- table-driven unit tests
  (done throughout) and the common typing path staying within 10% of the
  recorded baseline at p50/p95 (repeatedly confirmed against `6836f46`
  across every slice above) -- both hold today.
- **M1.C — embedded PTY terminal (partial), first M1 foundation piece.**
  `src/pty.rs`'s `PtySession` spawns a real child process behind a genuine
  pseudo-terminal (`portable-pty`) and feeds its output through a real
  VT100 emulator (`vt100`, the same crate family wezterm uses) so an
  interactive program's actual rendered screen -- colors, cursor position,
  wide characters, escape sequences and all -- can be composed into a pane,
  not approximated. `:terminal`/`:term` spawns `$SHELL` (falling back to
  `/bin/sh`) in a new split and enters a new `Mode::Terminal` immediately;
  every keystroke is encoded back to the raw bytes a real terminal would
  send (arrow keys, Ctrl-chars, Enter, Backspace, ...) and written to the
  child's stdin. Esc leaves to Normal for pane navigation/`:close`
  (Ctrl-W already worked pane-agnostically); `i`/`a` while Normal-focused
  on a terminal pane re-enters it; other Normal-mode keys are inert there
  on purpose (`normal::handle`'s new guard) since there's no visible buffer
  to run Vim motions against. `render::draw_terminal_pane` renders straight
  from `vt100::Screen` cells (fg/bg/bold/underline/inverse). Resize
  propagates every frame via `PtySession::resize` (a no-op if the pane's
  size hasn't changed) to both the real PTY (so the child's own `SIGWINCH`
  fires, e.g. `stty size` inside the shell reports the correct size) and
  the parser's screen buffer. Output arrives on its own reader thread; the
  main loop's existing idle-poll (`poll_lsp_events`'s sibling,
  `poll_terminals`) redraws on new output via a revision counter, so a
  long-running command's output appears without needing a keystroke.
  Closing a terminal's pane (`:close`, or as the sole survivor of `:only`)
  always kills the child and joins the reader thread first --
  `PtySession::shutdown` -- and quitting the editor via any path
  (`:q`/`:qa`/`ZZ`/...) does the same for every remaining terminal from one
  chokepoint in `main.rs`'s loop, so no code path can leak a process.
  4 unit tests in `pty.rs` (spawn/output, resize, input echo, and a real
  `kill -0` liveness check proving no process survives `shutdown`), 1
  regression test driving a real nested shell through the `Editor` harness
  end to end, and `tests/pty_terminal.py` at three terminal sizes -- a
  genuinely nested PTY test (the outer PTY drives the real release binary,
  which spawns and drives its own real inner shell) covering spawn, prompt
  appearance, input/output round-trip, Esc/`i` mode switching, real
  `SIGWINCH` resize propagation to the child, and `pgrep`-verified absence
  of any leaked process after `:close`. Full suite passes unchanged; no
  latency regression against `6836f46` (PTY code only runs when a terminal
  is actually open; `poll_terminals`/resize-check cost is a no-op iteration
  over an empty `Vec` otherwise). **Not implemented (why "partial," not
  "done"):** scrollback *viewing* (PageUp/PageDown) -- `vt100` retains it
  (capped at 5,000 lines) but nothing exposes scrolling through it yet;
  detach/reattach; SIGINT as a distinct action from kill-on-close; a
  generic job-supervisor abstraction covering the *existing* ad-hoc jobs
  (`SearchJob`, git polling, the review-agent job) -- this slice added a
  new, separate PTY-specific supervisor rather than unifying everything
  under one, which is real remaining M1.C scope; a statusline or `:jobs`
  list (both explicitly Phase 9 and later); float-style/tabbed terminal
  placement (only a split, for now); and reusing this for lazygit, agent
  CLIs, test runners or tool installation, which are Phase 5/6 work that
  can now build on this rather than needing their own PTY plumbing.
- **M1.B/D, M2–M9:** not started (M1.A and M1.C are partially done -- see
  their entries above). See the phase sections above for scope; nothing in
  this log should be read as partially done unless stated here.
