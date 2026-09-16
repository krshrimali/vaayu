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
| nvim-autopairs | Missing | Configurable pair insertion, skip, newline and deletion rules |
| nvim-surround | Missing | `ys`, `ds`, `cs`, Visual `S`, tags and repeat support |
| vim-sleuth | Missing | Per-buffer indent detection, with EditorConfig precedence |
| vim-wordmotion | Partial | camelCase, snake_case and kebab-case subword motions/operators |
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
| mini.align | Missing | Operator/Visual delimiter alignment with preview and undo grouping |
| goto-preview | Missing | Definition, implementation, reference and type-definition preview panes |
| nvim-utils | Missing | Test runner and custom utility command framework |
| zen-mode | Missing | Centered distraction-free layout with reversible UI options |
| refactoring.nvim | Missing | Extract/inline operations with preview, validation and undo |
| grug-far | Missing | Reviewed project-wide replacement with selective apply |
| terminal.lua / lazygit | Missing | Embedded PTY buffers, float/splits/tabs, persistent jobs and lazygit |
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

### C. Supervised jobs, PTYs and progress

- Generalize background work into a job supervisor with IDs, generations,
  cancellation, deadlines, bounded output, progress and exit state.
- Add embedded PTY processes with terminal emulation, resize propagation,
  scrollback, terminal/normal modes and clean shutdown.
- Reuse it for shells, lazygit, agent CLIs, test runners, tool installation and
  long Git/GitHub operations.
- Surface active jobs in the statusline and a searchable `:jobs` list.

Acceptance:

- Closing a pane never leaks a child process.
- SIGINT, terminate, detach and reattach are explicit actions.
- Slow or noisy jobs cannot block typing or grow memory without a bound.

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
   deterministic fallback; display the chosen source in buffer info.
5. Visual/operator delimiter alignment with preview and one undo transaction.
6. Move-line mappings, retained Visual indentation, select-all,
   increment/decrement and exact black-hole paste/delete mappings.
7. Persistent undo, focus-gained external-change checks, yank flash, cursorline
   and relative-number toggles.
8. Mouse positioning, selection, pane focus, resize and configured LSP mouse
   actions where the terminal reports mouse events.
9. Optional spelling dictionaries, misspelling decoration, suggestions and
   project/user dictionary updates.

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
- **M1.B/C/D, M2–M9:** not started. See the phase sections above for scope;
  nothing in this log should be read as those being partially done unless
  stated here.
