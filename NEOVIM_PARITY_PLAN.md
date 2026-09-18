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
| nvim-tree | Partial | Stateful file tree with safe file operations, filters and diagnostics |
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

### D. Tabs, histories and persistent workspace state [Partial: tab pages done, see progress log]

- Add tab pages above the existing recursive pane tree. [Done]
- Persist per-tab panes, working directory, active buffer, terminals and
  optional tool panels. [Not done -- `:sessionsave`/`:sessionload` still
  round-trip only the active tab, unchanged from before this slice;
  terminals are correctly never persisted (matches the acceptance bar below)]
- Add command, search, picker and quickfix history plus resume-last-picker.
  [Command/search history done; picker/quickfix history and
  resume-last-picker not done -- see progress log]
- Preserve histories privately with size and age limits. [Not done]

Acceptance:

- All configured tab mappings work, including numeric jump, move and
  tab-only. [`gt`/`gT`/`{n}gt`/`:tabnew`/`:tabclose`/`:tabonly` done; "move"
  (reordering tabs, e.g. `:tabmove`) not done]
- Session round trips retain tab/pane geometry without restoring unsafe jobs.
  [True but incomplete: sessions restore one tab's geometry correctly and
  never resurrect a terminal (`Window::terminal` is `#[serde(skip)]`), but
  multi-tab layouts are not yet saved/restored at all]

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
   [Done: jumps (:jumps), command history (:chistory/:history), search
   history (:shistory), current-buffer lines (:blines) and Git stash
   (:gitstash) added as Results-list sources -- Enter navigates to a
   jump/line location, reruns the selected command/search, or (for Git
   stash) shows that stash's diff; workspace symbols (,lw/
   :workspacesymbols) added via the LSP round trip (see Phase 3.1);
   :commands (Enter fills the command line rather than running it
   immediately, since most commands need arguments), :projects
   (recently launched-from directories, persisted globally; Enter
   switches project_root) and :everything (a single unified built-in
   source list combining keymaps/commands/recent projects, reusing
   each source's own existing action tags with no new dispatch logic
   -- see progress log) added; :keymaps, :help and :diagnostics
   already existed as separate list sources (correcting an earlier
   version of this bullet, written before that was checked)]
2. Add grep-current-word/selection, resume, preview toggle/wrap/scroll, select
   all, and open in current/vertical/horizontal/tab targets.
   [Done: grep-current-word/selection (,gw), resume (:resume), open in
   the current pane (Enter) or a new vertical/horizontal split
   (Ctrl-V/Ctrl-X) or tab (Ctrl-T), select all (`a`) and preview
   toggle/wrap/scroll (`p`/`w`/Ctrl-E/Ctrl-Y in any Results list,
   including quickfix -- see Phase 2.6 and the progress log) -- see
   progress log]
3. Add ranking instrumentation and a bounded incremental top-k matcher so a
   million-path inventory does not require sorting every candidate per key.
4. Build a file tree with expand/collapse, reveal-current-file, project-root
   synchronization, dotfile/ignore/Git-clean filters, live filter, bookmarks,
   diagnostics and Git state.
   [Done: expand/collapse, reveal-current-file, project-root, dotfile
   filtering (hidden by default, `.` toggles), gitignore filtering
   (hidden by default, `!` toggles), diagnostic decoration (E/W/I
   marker, including on unexpanded ancestor directories), bookmarks
   (`m` toggles, `:treebookmarks` lists), live filter (`/`, over
   already-loaded nodes only) and Git state decoration (status letter,
   refreshed on open/`R`) -- see progress log]
5. File operations: create, rename, copy, cut, paste, trash and delete with
   collision prompts, dirty-buffer checks and rollback where possible.
   [Done: create/rename/delete/trash/copy/cut/paste from the file
   tree, with collision refusal and dirty-buffer checks (trash
   moves into .vaayu/trash/ as a reversible alternative to permanent
   delete; y/x/p copy/cut/paste, recursive for directories; a directory
   copy that fails partway through rolls back rather than leaving a
   partial destination behind) -- see progress log]
6. Upgrade quickfix with preview, history, filtering, selected actions and
   split/tab opening.
   [Partial: split-opening (Ctrl-V/Ctrl-X) and tab-opening (Ctrl-T) done
   for both the picker and any results/quickfix list; quickfix history
   (:colder/:cnewer), preview (`p`) and filtering (`f`, case-insensitive
   substring against text/detail; since quickfix is a Results list
   under the hood, same as preview -- see Phase 2.2 and the progress
   log) done; not supported for a `live` grep list specifically, which
   already replaces its own entries wholesale on every keystroke --
   see progress log]
7. Build a persistent outline/symbol sidebar with hierarchy, collapse, follow
   cursor, symbol-kind filtering and preview.
   [Done: persistent sidebar with hierarchy, jump-to-symbol,
   UTF-16-corrected columns, collapse/expand (`h`/`l`), symbol-kind
   filtering (`f`), follow-cursor (highlights the symbol enclosing
   the buffer's cursor line while editing, no keypress needed) and
   hover preview (`K` shows hover documentation for the outline
   symbol without navigating -- see progress log)]
8. Add centered jump behavior after page/search movement and complete recent
   buffer navigation.
   [Done: page movement (Ctrl-D/Ctrl-U) already centered; search jumps
   (/, ?, n, N) now recenter too; Ctrl-6 and :b# for alternate-buffer
   navigation; ,b/:buffer now lists most-recently-activated first via a
   new buffer_mru list, completing recent buffer navigation. Ctrl-F/
   Ctrl-B intentionally left uncentered (matches Vim's own
   full-page-scroll behavior) -- see progress log]

Exit criteria:

- Each source converts to quickfix with one action.
- File mutations share the existing transactional resource safeguards.
- Picker filtering stays responsive on 1M synthetic paths and cancellation
  discards stale generations.

### Phase 3 — Complete LSP and completion parity

1. Add type definition, implementation, declaration, workspace symbols,
   selection/range formatting, CodeLens, document links, inlay hints and
   document highlights.
   [Partial: type definition (gy/:typedefinition), implementation
   (gI/:implementation), declaration (gD/:declaration) and workspace
   symbols (,lw/:workspacesymbols) done, sharing the existing
   definition/references location-list plumbing (workspace symbols
   opts out of the single-result auto-jump, matching how a search
   picker should behave, not a "go here" navigation) and selection/range
   formatting (`,lf` in Visual mode formats just the selected lines),
   document highlights (`,lh`, painted as an in-buffer background
   overlay rather than a jump list) and document links (`,ll`/
   :documentlinks, a Results list; Enter opens a file:// target or
   copies any other one, never opening a browser; a link with no
   inline `target` -- deferred to `documentLink/resolve` -- is skipped
   rather than adding another resolve round trip -- see progress log)
   done; CodeLens and inlay hints not done -- see progress log]
2. Add preview panes for definition, implementation, type definition and
   references with jump-list integration.
3. Add organize imports and source actions, including preferred/disabled action
   metadata, resolve and command execution.
4. Add diagnostic ranges, undercurl/underline rendering, related information,
   code/source labels, per-line popup, optional virtual text/lines and
   insert-mode update policy.
5. Add LSP progress tokens to the job/progress UI.
   [Done: `$/progress` notifications (previously silently dropped --
   the protocol layer only ever acknowledged `window/workDoneProgress/
   create`, never parsed the notification itself) are tracked in
   `Editor::lsp_progress` and surfaced both through the message line
   (title/percentage/message) and, persistently, in the active pane's
   status line (recomputed fresh every frame from live state, so a
   later unrelated message never hides it, unlike the message-line
   surfacing alone) -- see progress log]
6. Add completion path source, automatic documentation preview, configurable
   auto-show, source/kind labels and completion enable toggle.
   [Done: source labels ("lsp"/"buf") and inline `detail` text already
   existed before this session; `completion_enabled=false`, LSP `kind`
   labels (function/variable/etc, shown in place of the generic "lsp"
   tag when the server provides one), a path completion source
   (triggers on any path-shaped prefix, i.e. containing `/`, tagged
   "path"), a real documentation preview (multi-line `documentation`,
   shown below the item list for the selected item when the server
   provides one, separate from the short inline `detail` already
   shown) and a configurable auto-show delay (`completion_delay_ms`,
   default 0 = instant; candidates are still computed immediately
   either way, only painting the popup is delayed) are new -- see
   progress log]
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
   [Partial: hunk navigation (`]c`/`[c`, wraps around, one stop per
   contiguous hunk not per changed line) done, reusing the existing
   (already fully wired, background-thread-computed) gutter sign data;
   stage/unstage already existed as a separate :gitstage/:gitunstage
   results-list workflow, not gutter-integrated; hunk preview (`,gh`,
   shows the diff for the hunk under the cursor as a read-only Results
   list) and line blame with a blame toggle (`,gB`, virtual text after
   the current line's own content, computed asynchronously -- see
   progress log) done. Hunk reset (`,gx`, shows the hunk as a
   confirmation prompt, Enter discards it back to HEAD in the working
   tree and reloads the open buffer -- see progress log) done.
   Selected-range actions not done -- see progress log]
2. Render deleted lines and intra-line word changes as an optional diff overlay.
3. Build a Git workspace with staged/unstaged/untracked/conflict sections,
   file/hunk diffs, selective stage/reset, commit editor, amend, stash,
   branch/checkout, log and push/pull/fetch actions.
4. Add side-by-side and unified diff layouts, whitespace toggle, unchanged
   region folding and index watching.
5. Add lazygit as an embedded PTY action for users who prefer its interface.
6. Add GitHub permalink generation for cursor, line range and selected commit.
   [Done: `,gp`/`:permalink` generate a GitHub blob URL pinned to HEAD's
   commit SHA (not a branch name, which can move) for the cursor line
   or (via `,gp` in Visual mode) a selection's line range; `P` on a
   `:gitblame` entry uses that line's own blamed commit instead of HEAD
   ("selected commit"). Copies to the clipboard/+ register; never opens
   a browser or touches the network -- see progress log]

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
- **M1.D — tab pages (partial), second M1 foundation piece.** `windows.rs`
  adds a `Tab` (saved `windows`/`window_layout`/`active_window`/`cur`) and
  `Editor::tabs: Vec<Tab>` alongside the existing live pane-tree fields,
  mirroring the `store_window`/`capture_window` pattern already used for
  panes: the live fields are the active tab's source of truth, synced into
  `tabs[active_tab]` on switch. `gt`/`gT`/`{n}gt` (via the existing `g`
  prefix, alongside `gg`/`gd`/`gw`/`gb`/`ge`/`ga`) and `:tabnew`/
  `:tabclose`(`:tabc`)/`:tabonly`(`:tabo`)/`:tabnext`(`:tabn`)/
  `:tabprev`(`:tabp`)/`:tabs` all work. Each tab keeps fully independent
  pane state (different buffer, cursor, splits). A one-row tabline
  (`render::draw_tabline`) appears only once a second tab exists --
  `pane_rects` reserves the row conditionally, so a single-tab session's
  layout is byte-for-byte unaffected by this feature existing, verified by
  the full existing suite passing unchanged. Closing a tab (or `:tabonly`
  discarding others) kills any terminals running in its panes first, reusing
  `shutdown_terminal` from M1.C -- a terminal in a *surviving* background
  tab keeps running and producing output while not visible, same as a real
  terminal multiplexer, confirmed by a PTY test that opens a terminal in
  tab 2, switches away, and finds its output on switching back.
  A real bug was caught and fixed during testing, not just in review: the
  terminal-focus guard added in M1.C swallowed the completing key of any
  multi-key sequence that wasn't itself `g`/a digit (e.g. the `t` of
  `{n}gt`), leaving `Awaiting::GPrefix` stuck forever and silently eating
  every keystroke after it -- including the `:` that should have opened
  the command line. Fixed by letting *any* key through once a sequence is
  already in flight (`pending.awaiting.is_some()`), not just specific
  letters; a regression test pins the exact scenario. 6 unit/regression
  tests plus `tests/pty_tabs.py` at three terminal sizes (tabline
  appearance/content, `gt`/`gT`/`{n}gt`, independent per-tab buffers, and
  the background-terminal-keeps-running case). Full suite passes
  unchanged; the DECSTBM fast-scroll path is correctly disabled whenever
  multiple tabs exist (it hardcodes row 1 as the first content row, which
  a tabline shifts by one) rather than risking corrupting the tabline, so
  that specific case falls back to the still-correct full-row diff instead
  of the optimized path -- confirmed no regression either way against
  `6836f46` (two runs, both within normal noise). **Not implemented (why
  "partial," not "done"):** session persistence covers only the active
  tab, not the full `tabs` list (a real gap against D's stated acceptance,
  not a hidden one); command/search/picker/quickfix history and
  resume-last-picker (a separate D sub-item, unrelated to tabs
  specifically); tab reordering (`:tabmove`); and per-tab working
  directory (all tabs still share `project_root`).
- **M1.D — command/search history, continuing the same milestone.**
  `command.rs` adds capped (200-entry) `Vec<String>` history for `:`
  commands and `/`/`?` searches independently. Up/Down (and Ctrl-P/Ctrl-N,
  matching real Vim) cycle through the relevant history while the command
  line is open; the in-progress line is saved on the first press and
  restored when cycling back past the newest entry, so browsing history
  never loses what you were mid-typing. Consecutive identical submissions
  don't duplicate. In-memory only for this slice -- not persisted across
  restarts the way notes/recovery/undo already are, and picker/quickfix
  history plus resume-last-picker (the rest of this same plan bullet) are
  separate, unaddressed scope. 3 regression tests plus `tests/pty_history.py`
  at three terminal sizes, sending real xterm Up/Down arrow escape
  sequences over the PTY (not synthetic `Key` values) to prove the actual
  terminal input path works, including that command and search history
  stay independent of each other. Full suite passes unchanged; no latency
  regression against `6836f46`.
- **Phase 2.4 — file tree sidebar (partial), first Phase 2 slice.**
  `src/filetree.rs`: `,ft` toggles a sidebar pane (`Window::file_tree`,
  the same special-pane pattern M1.C's `Window::terminal` established) in
  a new vertical split, showing a project tree rooted at `project_root`.
  Directories are read lazily -- only `read_dir`'d when actually expanded
  -- so opening the tree on a huge project costs one directory read, not a
  recursive walk; only `.git` is unconditionally skipped. Opening the tree
  reveals the file in the currently active buffer (expands every ancestor
  directory and selects it), matching real nvim-tree's default behavior.
  Key handling (`j`/`k`/Ctrl-d/Ctrl-u/Home/End/`G`/`h`/`l`/Enter/`o`/`R`/
  `q`/Esc) is entirely self-contained, deliberately *not* routed through
  `Awaiting::GPrefix` or any generic motion/operator dispatch: the tree's
  cursor indexes a node list, not a buffer's lines, and M1.D's tab-switch
  bug (a multi-key sequence silently getting swallowed) demonstrated
  exactly the failure mode that sharing that machinery risks. Opening a
  file from the tree focuses the other (non-tree) pane and calls the
  existing `open_file`, reusing per-buffer indent detection and undo-file
  restore automatically. 5 unit tests in `filetree.rs` plus
  `tests/pty_filetree.py` at three terminal sizes (reveal-on-open,
  collapse/re-expand, opening a file into the adjacent pane, toggle-closed)
  -- the PTY test isolates the tree pane's own half of the screen for its
  assertions specifically because the other pane's status line also
  mentions the open file's name, which would otherwise make a broken
  collapse silently pass. Full suite passes unchanged; no latency
  regression against `6836f46`. **Not implemented (why "partial," not
  "done"):** `.gitignore`/dotfile filtering (everything except `.git`
  itself is shown), a live fuzzy filter, bookmarks, Git-status decoration,
  diagnostic markers, and file operations (create/rename/copy/delete --
  Phase 2 item 5, separate scope). No asymmetric/fixed-width sidebar
  sizing either: the tree pane is an ordinary 50/50 split pane, not a
  narrow ~30-column sidebar the way real nvim-tree looks by default.
- **Phase 2.5 — file tree operations (partial), continuing Phase 2.4.**
  `filetree.rs` adds `a` (create -- pre-fills `:treenew ` in the existing
  command line, reusing its history/editing rather than a new input
  widget; a trailing `/` creates a directory), `r` (rename, same
  `:treerename ` pattern), and `d`+`d` (delete: the first press only arms
  it for that exact path and shows what will be deleted; any other key
  cancels; a second `d` on the same node deletes -- a real filesystem
  delete has no undo, so this is deliberately two explicit keys, not one).
  Create and rename both refuse to overwrite an existing path; rename and
  delete both refuse when an open buffer under the target path has
  unsaved changes (a directory delete checks every buffer whose path
  starts with it, not just an exact match). A successful rename updates
  any open buffer's `path` in place and reveals the new location; a
  successful create reveals what it made. 12 unit tests (including the
  two-press arm/cancel/confirm state machine and both dirty-buffer refusal
  cases) plus `tests/pty_filetree_ops.py` at three terminal sizes,
  including a real end-to-end dirty-buffer-blocks-delete case (open the
  file from the tree, dirty it in the buffer pane, navigate back to the
  tree with Ctrl-W, confirm delete is refused). Full suite passes
  unchanged; no latency regression against `6836f46`. **Not implemented:**
  copy/cut/paste (no clipboard-style "marked node" concept exists yet),
  trash-instead-of-permanent-delete, and multi-select/batch operations.
- **Phase 2.7 — outline/symbol sidebar (partial).** `src/outline.rs`: `,lO`
  toggles a persistent sidebar (`Window::outline`, the same special-pane
  pattern as `file_tree`/`terminal`) showing the current buffer's LSP
  `textDocument/documentSymbol` response as an indented hierarchy, reusing
  the exact request `,lo`/`:outline` already sends. The response is routed
  to the sidebar instead of the transient results list by
  `language.rs` checking whether an outline pane is open at response time
  (a new, earlier, guarded match arm ahead of the existing flat-list one,
  so `,lo`'s original transient behavior is provably unchanged when no
  sidebar is open -- confirmed by the full existing suite passing
  unchanged, including `mock_lsp_config_sync_and_features`, which still
  gets the flat list). Handles both `DocumentSymbol` (hierarchical, via
  `children`) and the older flat `SymbolInformation` (via `location`)
  shapes servers may return. `j`/`k`/Home/`G`/End navigate; Enter/`o`/`l`
  jump to the symbol's position in the other pane (reusing `push_jump` for
  the jumplist); `,lO` toggles closed from *either* pane, not just while
  the sidebar is focused, which is the correct toggle semantics but tripped
  up the first draft of the PTY test into expecting a "reopen" that isn't
  how a toggle works.
  3 unit tests for the DocumentSymbol/SymbolInformation/depth-cap parsing,
  1 regression test driving a **real** mock LSP server end to end (not
  fabricated JSON) confirming the sidebar-vs-transient-list routing, and
  `tests/pty_outline.py` at three terminal sizes against the same mock
  server over a real PTY -- this needed two rounds of fixing genuine
  flakiness in the test itself (waiting a fixed 2s for "the server is
  probably ready" instead of polling for its actual published diagnostic
  the way `regression.rs`'s mock-LSP tests already do, and wrongly
  expecting the second `,lO` to reopen rather than correctly close the
  sidebar), not bugs in the feature. Full suite passes unchanged; two
  latency runs against `6836f46` show no regression (one run's
  `enter_insert` outlier reversed direction on rerun, confirming it was
  system load, not the change). **Not implemented (why "partial," not
  "done"):** collapse/expand (every symbol is always shown -- see
  `outline.rs`'s doc comment for why this is a reasonable scope cut, unlike
  the file tree where lazy expansion is about not walking a huge
  filesystem), live follow-cursor (highlighting the enclosing symbol as
  the cursor moves), symbol-kind filtering, hover preview, and UTF-16
  column correction (a symbol on a line with non-ASCII text before it may
  land a character or two off, unlike the transient `:outline` list which
  already corrects this).
- **Phase 2.2 — grep current word/selection (partial).** `,gw` (added to
  the same action registry as every other leader command) greps the word
  under the cursor in Normal mode (via `textobject::resolve`'s inner-word
  span, the same resolver `surround`/`align` already rely on), or the
  Visual selection's literal text for Char/Line kinds (reusing
  `normal::span_to_range`, the same helper `M1.D`'s tab work already
  extracted); Visual-block falls back to the word under the cursor since a
  block selection is a column, not a contiguous string, and guessing which
  row to use would be arbitrary. 2 regression tests plus
  `tests/pty_grep_word.py` at three terminal sizes -- writing the PTY test
  surfaced a real "obvious but easy to miss" interaction worth recording:
  live grep opens directly into query-*editing*, so a single Esc only
  leaves that sub-mode back to browsing results, not back to the buffer;
  the test needed two Escapes, and this is genuinely how the feature
  behaves, not a bug. Full suite passes unchanged; no latency regression
  against `6836f46`. **Not implemented:** resume-last-picker, preview
  toggle/wrap/scroll, and split/vertical/tab open targets (the rest of
  this same plan bullet, orthogonal to grep-word/selection specifically).
- **Phase 2.6 — split/vertical opening from picker and results (partial).**
  Ctrl-V (vertical) and Ctrl-X (horizontal) in the file picker and in any
  results/quickfix list open the selected location into a new split
  instead of replacing the current pane, reusing `split_window` the same
  way `:vsplit path` already does. `results.rs`'s new `open_result_split`
  mirrors `open_result`'s exact entry-kind handling (buffer_id vs. path)
  but falls back to plain `open_result` for action/text entries, which
  have no location to split into. 4 regression tests (results + picker,
  each proving both the buffer_id and path entry paths) plus
  `tests/pty_split_open.py` at three terminal sizes -- writing it surfaced
  a test-design trap worth recording, not a product bug: after a Ctrl-V
  split, the *new* pane is focused (matching `split_window`'s existing
  convention), so a naive `:only` right after keeps the just-opened file,
  not the original one; the test needed an explicit `Ctrl-W h` first.
  Full suite passes unchanged; two latency runs against `6836f46` show no
  regression (these are new match arms only reached in Results/Picker mode
  on a specific keypress, never on the hot typing path). **Not
  implemented:** preview, history, filtering, and tab-opening (the rest of
  this same plan bullet, orthogonal to split-opening specifically).
- **Phase 2.1 — jumps/command-history/search-history picker sources
  (partial).** `:jumps`, `:chistory`/`:history` and `:shistory` in
  `src/command.rs` render the jumplist, `Editor::command_history` and
  `Editor::search_history` as ordinary Results lists (most recent first).
  Selecting an entry acts, not just displays: a `:jumps` entry is a normal
  location entry so Enter (or Ctrl-V/Ctrl-X, for free, via the Phase 2.6
  split-open path) navigates to it; `:chistory`/`:shistory` entries carry
  a new tagged `Entry.action` payload (`_vaayu_rerun_ex` /
  `_vaayu_rerun_search`), interpreted by two new branches in
  `results.rs`'s `open_result()` that re-run the command through the
  existing `run_ex` path or replay the search through `find_search`,
  exactly as if retyped. `:chistory`/`:history`'s own invocation is
  necessarily the newest entry in its own list (it is pushed to
  `command_history` before `run_ex` builds the list) -- left as correct
  behavior, matching a shell's own `history` command showing itself, not
  fixed away. 3 regression tests (`jumps_command_lists_jumplist_and_navigates_to_entries`,
  `history_command_lists_and_reruns_a_past_command`,
  `shistory_command_lists_and_reruns_a_past_search`) plus
  `tests/pty_history_pickers.py` at three terminal sizes, using marks
  (`ma` / `` `a ``) to generate a real jump and an `x`/`u` probe on the
  landing line to prove navigation actually moved the cursor rather than
  just opening a list (absolute tmp paths get clipped in the 40-column
  list, so the jumps assertion checks the results-count text, not the
  path). Full suite (179 tests) and full existing PTY suite (23 files)
  pass unchanged; two latency runs against `6836f46` show no regression
  (p50 identical at ~0.004ms, p90/p99 differences are sub-0.1ms noise --
  these are new match arms only reached from Results/Picker mode on a
  specific keypress or `:` command, never on the hot typing path). **Not
  implemented:** help, commands, projects, workspace symbols,
  current-buffer lines, Git stash and a single unified built-in
  source list (the rest of this plan bullet); `:keymaps` and
  `:diagnostics` already existed as separate list sources before this
  slice and were not touched.
- **Phase 2.8 — centered search jumps and alternate-buffer navigation
  (partial).** Ctrl-D/Ctrl-U already centered the viewport (`zz`-habit);
  `/`, `?`, `n` and `N` now do the same, via a shared `recenter_viewport`
  (made `pub(crate)`, previously private to `normal.rs`) called from
  `command::run_search` and `normal::search_next` right after a
  successful match. Ctrl-F/Ctrl-B remain uncentered, matching Vim's own
  full-page-scroll behavior -- this is a deliberate choice, not a gap.
  Refactor side effect: `results.rs`'s `_vaayu_rerun_search` handler,
  which had duplicated `run_search`'s match/translate/find logic, now
  just calls the (newly `pub(crate)`) `command::run_search`, so
  `:shistory` reruns get centering for free and the two code paths can't
  drift apart. Separately, `Editor::alternate_buffer: Option<u64>` plus
  `note_alternate_buffer`/`switch_to_alternate` track a single
  previously-active buffer (Vim's `Ctrl-^`); `Ctrl-6` (confirmed against
  crossterm's source to be how it decodes the raw 0x1E byte a terminal
  sends for Ctrl-6/Ctrl-^) and a new `:b#` ex command toggle to it.
  Hooked only at genuine user-driven switches -- `open_file` (covers
  :e/gf/gd/grep/diagnostics-jump/recent-files/file-picker), `:bnext`/
  `:bprev`/`:b N`, and the buffer-picker/results/jumps `open_result(_split)`
  paths -- deliberately NOT at the many other `self.cur = i` sites in
  windows.rs/session.rs/resources.rs/notes.rs, which are pane-focus or
  session-restore bookkeeping, not a user "switching files". 2
  regression tests (one covering search-then-recenter for both `/` and
  `n`, one covering the alternate-buffer toggle via both Ctrl-6 and
  `:b#`) plus `tests/pty_altbuffer_and_centering.py` at three terminal
  sizes, which proves centering by checking plain content lines are
  visible on both sides of the match row (a minimal edge-scroll would
  show nothing above it) rather than just asserting the match is
  present. Full suite (181 tests) and full existing PTY suite (24
  files) pass unchanged; two
  latency runs against `6836f46` show no regression (one run's
  `search_submit` p50 briefly read 0.015ms vs baseline's 0.005ms -- a
  rerun brought it back to 0.004ms, confirming machine noise, not a real
  cost from the added centering call). **Not implemented:** a broader
  recent/MRU buffer list beyond a single alternate (the "complete recent
  buffer navigation" half of this plan bullet, if it's meant to be more
  than Vim's alternate-buffer semantics -- `,fr`'s existing `recent_files`
  picker already covers cross-session recently-opened *files*, which is
  related but not the same as an in-session buffer MRU).
- **Phase 2.6 continued — tab-opening from picker and results (partial).**
  Ctrl-T in the file picker and in any results/quickfix list now opens
  the selection into a brand-new tab, alongside the existing Ctrl-V/
  Ctrl-X split-opening. `results.rs`'s new `open_result_tab` mirrors
  `open_result_split`'s exact buffer_id/path entry handling but calls
  `new_tab()` instead of `split_window()`; `picker.rs`'s handler grew a
  third arm calling `new_tab()` before `open_file()`. `Editor::windows`
  is an empty `Vec` for an unsplit pane (populated lazily on first
  split), so proving "Ctrl-T did not also split" meant asserting
  `windows.is_empty()`, not a length -- worth recording since the first
  version of the test asserted `windows.len() == 1` and failed comparing
  0 to 1. 2 regression tests (one for the picker, one for a results-list
  location entry) plus `tests/pty_tab_open.py` at
  three terminal sizes. Full suite (183 tests) and full existing PTY
  suite (25 files) pass unchanged; two latency runs against `6836f46`
  show no regression (these are new match arms only reached from
  Results/Picker mode on a specific keypress, never on the hot typing
  path). **Not implemented:** preview, history and filtering (the rest
  of this same plan bullet, orthogonal to tab-opening specifically).
- **Phase 2.4 continued — file tree dotfile filtering (partial).**
  `FileTree::show_hidden` (default `false`) filters dotfiles out of
  `list_dir`/`walk`; `.` in the tree toggles it and rebuilds. `.git`
  stays hidden regardless of the toggle, matching the existing
  always-skip rule for it. 1 regression test (hidden by default, `.`
  reveals dotfiles but not `.git`, `.` again hides them) plus
  `tests/pty_filetree_hidden.py` at three terminal sizes. Full suite
  (184 tests) and full existing PTY suite (26 files) pass unchanged; two
  latency runs against `6836f46` show no regression (file-tree-only
  code, never reached on the hot typing path). **Not implemented:**
  `.gitignore` filtering, live filter, bookmarks, and Git/diagnostic
  decoration (the rest of this same plan bullet).
- **Phase 2.7 continued — outline sidebar UTF-16 column fix.** The
  sidebar's own documented gap ("Column positions are not
  UTF-16-corrected... may land a character or two off") is fixed:
  `language.rs`'s `"outline" if ... w.outline` response arm now runs
  each symbol's raw LSP column through `utf16_to_col` against that
  line's real text, exactly like the flat `:outline` results-list arm
  already did. `flatten()` itself is unchanged (it has no buffer access,
  so it still stores the raw UTF-16 unit count; correction happens once
  in the response handler, where buffer text is available). Proving this
  needed a real discrepancy between UTF-16 units and char index, which
  only shows up for a surrogate-pair character (2 UTF-16 units, 1 char)
  before the target column -- `tests/mock_lsp.py`'s documentSymbol reply
  was updated to report a nonzero `selectionRange.start.character`
  (previously always 0, so the bug had no way to manifest through it);
  checked that its only other consumer (`locations()`'s flat-list path,
  used by the transient `:outline`/`gd`/`gr` results list) only asserts
  the symbol's `name`/`text`, never a column, before changing it. 1
  regression test with a leading emoji proving the raw unit count (3)
  and the corrected char index (2) actually differ and the corrected
  one wins. Full suite (185 tests) and full existing PTY suite pass
  unchanged; two latency runs against `6836f46` show no regression
  (LSP-response-only code, never reached on the hot typing path).
- **Phase 2.7 continued — outline symbol-kind filtering (partial).**
  `f` in the outline sidebar cycles a kind filter: all -> each kind
  present in the last response (first-seen order) -> back to all,
  wrapping. `Outline` now keeps the full response in a new `all_nodes`
  alongside the displayed `nodes`; `set_nodes` (replacing the response
  handler's old direct `o.nodes = nodes` assignment) stores into
  `all_nodes` and re-derives `nodes` by the current filter, so a
  refresh (`R`) with new symbols keeps whatever filter was active
  instead of silently clearing it. Filtering was deliberately
  implemented as "which nodes are IN `nodes`" rather than adding a
  separate filtered-view concept, so rendering, movement (j/k/G/Home/
  End) and jump-to-symbol needed zero changes -- they already just
  read `nodes`/`cursor` as before. `tests/mock_lsp.py`'s
  documentSymbol reply grew a `--multi-symbol` opt-in (checked via
  `sys.argv`, the same pattern `--hang-init` already used) returning
  two kinds of symbols, since the default single-symbol reply can't
  exercise a multi-kind filter; the default reply for every other
  test is unchanged. 2 unit tests (cycle narrows then wraps back to
  all; a refresh keeps the active filter) plus
  `tests/pty_outline_filter.py` at three terminal sizes. Full suite
  (187 tests) and full existing PTY suite pass unchanged; two latency
  runs against `6836f46` show no regression (outline-only code, never
  reached on the hot typing path).
- **Phase 2.7 continued — outline collapse/expand.** `h` collapses the
  symbol under the cursor (only if it has children); `l` expands it back
  if collapsed, otherwise falls through to the existing jump-to-symbol
  behavior (Enter/o keep that behavior unconditionally). `SymbolNode`
  has no parent/child links -- `all_nodes` is just a flat, depth-sorted
  pre-order list -- so `visible_after_collapse` infers "descendant of a
  collapsed node" by skipping any run of deeper nodes after one marked
  collapsed, until depth returns to that level or shallower; this runs
  before the existing kind filter in `apply_filter`, so both compose.
  Collapsed identity is `(name, line)` (no stable id exists) in a new
  `Outline::collapsed` set, checked and updated alongside `kind_filter`
  by the same `set_nodes`/`apply_filter` plumbing the filtering slice
  already built, so a refresh (`R`) keeps both. The sidebar now draws a
  ▾/▸ marker (reusing the file tree's convention) before symbols that
  have children, blank otherwise. 3 unit tests (collapse hides
  descendants but not siblings and expand restores them; collapse is a
  no-op on a leaf; `l`'s return value distinguishes "expanded" from
  "fall through to jump") plus `tests/pty_outline_collapse.py` at three
  terminal sizes, using a new `--nested-symbol` opt-in reply in
  `mock_lsp.py` (the existing `--multi-symbol` reply is flat, with no
  parent/child pair to collapse). Full suite (189 tests) and full
  existing PTY suite pass unchanged; two latency runs against `6836f46`
  show no regression (outline-only code, never reached on the hot
  typing path). Phase 2 item 7's only remaining gaps are now
  live follow-cursor and hover preview.
- **Phase 2.2 continued — `:resume` (partial).** Reopens whichever of
  the file picker or a Results/quickfix list was dismissed more
  recently, exactly as left. `self.results` turned out to already be
  never cleared anywhere in the codebase (only ever replaced with a new
  `Some(...)`) -- so resuming a Results list needed no new state, just
  a way to know it *should* be resumed, and to switch `mode` back to
  `Results`. The file picker's Esc handler, unlike results, did destroy
  its state (`file_picker = None`), so it gained a `last_picker`
  snapshot moved (not cloned) out on every dismissal path (Esc, and
  after choosing a file/split/tab) alongside a new `ResumeTarget`
  enum recording which of the two was more recent. `remember_results`
  (already called at every point Results is dismissed or acted on) is
  the single place that marks Results as resumable, so every existing
  call site got the behavior for free. 3 regression tests (resume a
  dismissed picker with its query intact; resume a dismissed results
  list with its cursor intact; resume prefers whichever was dismissed
  more recently) plus `tests/pty_resume.py` at three terminal sizes.
  Full suite (192 tests) and full existing PTY suite pass unchanged;
  two latency runs against `6836f46` show no regression (an isolated
  `leave_insert` p50 blip in one run vanished on rerun -- confirmed
  noise, not a real cost, since resume's code never runs on that path).
  **Not implemented:** preview toggle/wrap/scroll (the rest of this
  plan bullet).
- **Phase 2.4 continued — file tree diagnostic decoration (partial).**
  A file with LSP diagnostics shows an E/W/I marker (reusing the
  buffer gutter's existing letter convention); a directory shows the
  worst severity among any descendant, even if the tree never expanded
  it, since `render.rs`'s new `tree_diagnostic_marker` checks
  `Editor::diagnostics` (keyed by absolute path, independent of what
  the lazily-built tree has loaded) with a `starts_with` prefix match
  rather than needing the directory's children in memory. Plain-letter
  marker only (no color), matching the outline sidebar's existing
  plain-text kind labels -- not a cut corner so much as consistency
  with the rest of this sidebar's minimal styling. 1 regression test
  (clean file gets no marker; warned/errored files get their own;
  an unexpanded directory inherits its worst descendant's; a
  directory containing both gets the worse of the two) plus
  `tests/pty_filetree_diagnostics.py` at three terminal sizes, waiting
  on the main buffer's own gutter marker as a readiness proxy for the
  mock LSP's diagnostic (a fixed sleep would be flaky here, per this
  session's own earlier outline-sidebar lesson). Full suite (193
  tests) and full existing PTY suite pass unchanged; two latency runs
  against `6836f46` show no regression (diagnostics-map-only code, a
  HashMap lookup/scan bounded by how many files currently have
  diagnostics, never reached on the hot typing path).
- **Phase 2.5 continued — file tree trash (partial).** `t`/`t` (same
  two-press-confirm shape as `d`/`d`) moves a node into
  `.vaayu/trash/<millis>-<name>` via `std::fs::rename` instead of
  removing it, refusing under the same open-dirty-buffer condition as a
  real delete (the check was factored into a shared
  `has_dirty_buffer_under` so delete and trash can't drift apart on
  that rule). No new dependency: this is a private per-project trash
  (matching `.vaayu/`'s existing role for undo/session/comments state),
  not desktop/XDG trash integration -- restoring a trashed file today
  means moving it back out manually. Confirm-state guard in
  `handle_key` extended so arming trash (`t`) correctly cancels a
  pending delete (`d`) and vice versa, rather than both being armed at
  once; a regression test pins this specifically. 5 unit tests (two
  presses required; content preserved; any other key cancels; dirty
  buffer refuses; delete/trash arming don't cross-contaminate) plus
  `tests/pty_filetree_trash.py` at three terminal sizes. Full suite
  (197 tests) and full existing PTY suite pass unchanged; two latency
  runs against `6836f46` show no regression (one run's `search_submit`
  showed a single-label blip that vanished on rerun -- confirmed noise,
  and unrelated to this file-tree-only code regardless). **Not
  implemented:** copy/cut/paste (the rest of this plan bullet).
- **Phase 2.5 continued — file tree copy/cut/paste.** `y` copies the
  cursor's node to a new `FileTree::clipboard: Option<(PathBuf, bool)>`
  (the `bool` marks a cut); `x` marks the same but for a move; `p`
  pastes into the cursor's target directory via a new `copy_recursive`
  helper (`std::fs` has no built-in directory copy) or `std::fs::rename`
  for a cut. `p` refuses a name collision, a source that's vanished
  since being marked, and (for a cut) an unsaved buffer under the
  source -- the same `has_dirty_buffer_under` check delete/trash already
  share. A cut only clears the clipboard once the move actually
  succeeds, so a refused paste leaves it retryable elsewhere. A moved
  file's own open buffer gets repointed to the new path (mirroring
  `tree_rename`); a directory *move*'s nested open buffers are not
  repointed, deliberately matching `tree_rename`'s existing, unchanged
  behavior for directory renames rather than introducing new
  asymmetric behavior between the two operations -- a pre-existing,
  documented limitation, not a new gap. 6 unit tests (copy keeps the
  original; cut moves and repoints the buffer; collision refuses
  without touching either side; paste with nothing copied is a
  no-op; cut refuses under a dirty buffer; copy recurses into a real
  directory) plus `tests/pty_filetree_copy_paste.py` at three terminal
  sizes. Full suite (203 tests) and full existing PTY suite pass
  unchanged; two latency runs against `6836f46` show no regression
  (file-tree-only code, never reached on the hot typing path). **Not
  implemented:** rollback of a partially-failed recursive directory
  copy (`copy_recursive` doesn't clean up a half-copied destination if
  it fails partway through) -- the "rollback where possible" half of
  this plan bullet, for the directory case specifically.
- **Phase 2.4 continued — file tree bookmarks (partial).** `m` toggles
  the cursor's node in a new `FileTree::bookmarks: BTreeSet<PathBuf>`,
  drawn with a ★ marker; `:treebookmarks` lists them as a Results list.
  A file entry opens normally, but a directory entry can't be "opened"
  as a buffer -- it's tagged with a `_vaayu_tree_bookmark` action
  (`dir: bool`) that `open_result()` routes to a new
  `Editor::open_tree_bookmark`, which reveals it in the tree instead.
  Writing the first unit test for this surfaced a real, previously
  untested gap: opening a file from `:treebookmarks` (or the tree's own
  Enter) requires an actual *other* pane to focus into
  (`open_from_tree`'s existing, pre-this-slice behavior) -- but the
  `editor_with_tree` test fixture only ever set the `file_tree` field
  directly, never actually creating a split the way real `,ft` usage
  always does, so the file silently never opened. Fixed by having the
  fixture call the real `toggle_file_tree()` instead, which every other
  test using it tolerates fine (none depended on `windows` being empty).
  4 unit tests (toggle on/off; list-and-open a file entry; list-and-
  reveal a directory entry; an empty bookmark set shows a message, not
  an empty list) plus `tests/pty_filetree_bookmarks.py` at three
  terminal sizes. Full suite (207 tests) and full existing PTY suite
  pass unchanged; two latency runs against `6836f46` show no regression
  (file-tree-only code, never reached on the hot typing path).
- **Phase 2.4 continued — file tree live filter (partial).** `/` starts
  typing a substring filter (`FileTree::filter`); `rebuild()` (already
  the single place that re-derives `nodes` from a fresh `walk()`) applies
  it as a post-filter `retain`. Deliberately scoped to *already-loaded*
  nodes only -- expanded directories' children -- never a full recursive
  project search, because that would defeat the module's whole reason
  for existing (its own doc comment: opening the tree costs one
  `read_dir` of the root, not a full walk, specifically so it stays cheap
  on a huge project). Filtering intercepts keys the same way
  `results.rs`'s `search_input` sub-mode already does (a `filter_input`
  flag checked first in `handle_key`, so letters that are normally tree
  commands -- `d`, `t`, `y`, etc. -- become query characters instead, or
  files named after them would be untypeable to search for). Esc clears
  the filter and rebuilds unfiltered; Enter keeps it applied but returns
  keys to normal navigation/commands. `report_tree_filter` messages the
  query and match count on every keystroke, explicit that the search is
  "loaded nodes only" so it can't be mistaken for a project-wide filter.
  5 unit tests (filters down to a substring match; Backspace widens back
  out; Esc clears vs. Enter keeps; a command letter becomes query text
  while typing) plus `tests/pty_filetree_filter.py` at three terminal
  sizes. Full suite (213 tests) and full existing PTY suite pass
  unchanged; two latency runs against `6836f46` show no regression
  (file-tree-only code, never reached on the hot typing path). Phase 2
  item 4's only remaining gaps are `.gitignore` filtering and Git state
  decoration.
- **Phase 2.4 continued — file tree Git state decoration.** A file's
  `git status` letter (M/A/D/U/C/R modified/added/deleted/unmerged/
  copied/renamed, `?` untracked) shows next to it; a directory shows a
  generic `*` if any descendant has changed (git's own status has no
  natural severity order the way diagnostics do, so a directory
  doesn't try to summarize to one specific letter). New
  `git_tools::status` runs a single `git status --porcelain=v1 -z
  --untracked-files=all` and parses the NUL-delimited output into a
  `path -> char` map, reusing the module's existing `run()` helper;
  `-z` (rather than default quoted-and-newline-separated output) avoids
  quoting edge cases for unusual filenames. This is a one-shot fetch,
  cached on `FileTree::git_status` and refreshed only on tree open and
  `R` (mirroring the diagnostic marker's own "never per-frame" rule,
  and how `:gitdiff`/`:gitblame`/`:gitstage` are already one-shot, not
  polled) -- outside a Git repo (or without `git` on `PATH`) it just
  silently stays empty, no message, since this is a nice-to-have. 2
  regression tests (`git_tools::status` itself: modified/untracked/
  clean; the tree's refresh-on-open-and-`R` plumbing) plus
  `tests/pty_filetree_git_status.py` at three terminal sizes, against a
  real git repository. Full suite (215 tests) and full existing PTY
  suite pass unchanged; two latency runs against `6836f46` show no
  regression (a one-shot subprocess call on an explicit user action,
  never reached on the hot typing path). Phase 2 item 4's only
  remaining gap is `.gitignore` filtering.
- **Phase 2.4 finished — file tree `.gitignore` filtering.** `!`
  toggles `show_ignored`; new `git_tools::ignored` runs `git status
  --porcelain=v1 -z --ignored` (deliberately *without*
  `--untracked-files=all`, confirmed empirically -- with it, git
  expands an entirely-ignored directory into every file inside it
  instead of collapsing to one line) and is filtered into `list_dir`
  itself, the same layer dotfile-hiding already uses, so an ignored
  directory is never `read_dir`'d into at all, not just hidden after
  the fact. Refreshed alongside `git_status` on tree open and `R`
  (`refresh_tree_git_status` now does both git calls; a changed ignored
  set triggers one extra `rebuild()` so already-loaded state reflects
  it immediately). 3 regression tests (`git_tools::ignored` collapses a
  whole ignored directory to one entry, confirmed via a real git repo;
  the tree hides it by default; `!` reveals and re-hides it) plus
  `tests/pty_filetree_gitignore.py` at three terminal sizes, against a
  real git repository. Full suite (217 tests) and full existing PTY
  suite pass unchanged; two latency runs against `6836f46` show no
  regression (one-shot subprocess calls on an explicit user action,
  never reached on the hot typing path). **Phase 2 item 4 is now fully
  done** -- every sub-bullet (expand/collapse, reveal, project-root
  sync, dotfile/gitignore filtering, live filter, bookmarks,
  diagnostics and Git state decoration) is implemented and tested.
- **Phase 2.6 continued — quickfix history (`:colder`/`:cnewer`).**
  `Editor::quickfix_history: Vec<Results>` plus a `quickfix_history_pos`
  index; `quickfix` always mirrors `quickfix_history[quickfix_history_pos]`.
  Only `export_quickfix` (Ctrl-Q -- genuinely creating a new list)
  appends, truncating any "newer" history past the current point first
  (matching Vim: setting a new list from partway through history
  discards what was ahead of it). Revisiting or dismissing the *current*
  list (`remember_results`, `quickfix_step`'s `:cnext`/`:cprev`) updates
  that slot in place instead of growing history, so merely opening/
  closing quickfix (or paging through it) doesn't pollute `:colder`.
  `:colder`/`:cnewer` clamp at the ends with a message rather than
  wrapping or panicking. 4 regression tests (navigate between two real
  exported lists by title; :colder/:cnewer at the ends are safe no-ops;
  a new list from a rewound point discards the discarded-forward
  entries; dismissing the current list doesn't grow history) plus
  `tests/pty_quickfix_history.py` at three terminal sizes, using two
  distinct live-grep exports distinguished by match content. Full suite
  (220 tests) and full existing PTY suite pass unchanged; latency
  comparisons against `6836f46` were noisy while a concurrent PTY suite
  run shared the machine (a full-board ~2x blip that vanished on a
  clean rerun with nothing else running -- confirmed contention, not a
  regression, and inconsistent with this change's scope regardless,
  since none of it runs on the hot typing path). **Not implemented:**
  quickfix preview and filtering (the rest of this plan bullet).
- **Phase 2.8 finished — buffer MRU list.** `Editor::buffer_mru: Vec<u64>`
  (most recent first), touched via a new `touch_buffer_mru` at the same
  six genuine-user-switch sites `alternate_buffer` already instruments
  (`open_file`'s two branches, `open_result`/`open_result_split`/
  `open_result_tab`'s buffer_id branch, `:bnext`/`:bprev`/`:b N`, and
  `switch_to_alternate` itself) -- not the window/tab/session-restore
  bookkeeping sites, which were already excluded from alternate-buffer
  tracking for the same reason. `,b`/`:buffer` now lists buffers via a
  new `buffers_in_mru_order` (most-recently-activated first, falling
  back to natural order for any buffer `buffer_mru` never recorded --
  e.g. one only ever reached through session-restore/pane-focus
  bookkeeping) instead of raw insertion order; `:bd`/`:bd!` prune the
  closed id out of `buffer_mru` alongside the existing `windows` prune.
  3 regression tests (MRU ordering after switching back to an earlier
  buffer; closing a buffer removes its id from `buffer_mru`) plus
  `tests/pty_buffer_mru.py` at three terminal sizes. Full suite (222
  tests) and full existing PTY suite pass unchanged; two latency runs
  against `6836f46` (one clean, machine otherwise idle, after an
  earlier PTY-suite-contention run this same session showed exactly
  the "vanishes when the machine is idle" pattern that's flagged
  elsewhere in this log as noise, not a regression) show no regression.
  **Phase 2 item 8 is now fully done.**
- **Phase 2.1 continued — current-buffer-lines picker source
  (`:blines`).** Every non-blank line in the current buffer as a
  Results list (blank lines skipped -- fzf.vim/Telescope's own
  current-buffer-lines behavior, and nothing useful to search or jump
  to that isn't already one `j`/`k` away); Enter jumps via the existing
  buffer_id location-entry path, no new plumbing needed. 1 regression
  test (blank lines excluded from the count; selecting an entry jumps
  to the right line) plus `tests/pty_blines.py` at three terminal
  sizes. Full suite (223 tests) and full existing PTY suite pass
  unchanged; two latency runs against `6836f46` show no regression
  (HEAD read faster both times, noise -- this is a one-shot list build
  on an explicit `:blines` invocation, never reached on the hot typing
  path).
- **Phase 3.1 continued — type definition, implementation, declaration.**
  `gy`/`gI`/`gD` (and `:typedefinition`/`:implementation`/`:declaration`)
  request `textDocument/typeDefinition`/`implementation`/`declaration`
  and route their responses through the exact same location-list
  handling `definition`/`references` already share (UTF-16 column
  correction, single-result auto-jump, quickfix export) -- these three
  are structurally identical LSP requests to `definition`, just
  different methods/capability keys, so no new response-handling code
  was needed, only new match arms. `tests/mock_lsp.py` gained explicit
  handlers for the three methods (previously any unhandled method
  silently got a `null` reply from its catch-all, which would have
  "worked" for testing presence-of-a-list but not really exercised the
  round trip) plus the matching capability flags in its `initialize`
  reply. 1 regression test exercising all three methods against a real
  mock LSP round trip (single location -> auto-jump to the right
  line/col) plus `tests/pty_goto_lsp.py` at three terminal sizes,
  reusing the gutter-diagnostic-marker readiness-wait pattern already
  established for other mock-LSP PTY tests on this machine. Full suite
  (224 tests) and full existing PTY suite pass unchanged; latency
  comparisons against `6836f46` were noisy on the first run (a uniform
  ~4x blip across every unrelated label, inconsistent with this
  change's scope) but matched exactly on a clean rerun -- confirmed
  noise, not a regression, consistent with this session's other
  noisy-then-clean benchmark reruns. **Not implemented:** workspace
  symbols, selection/range formatting, CodeLens, document links, inlay
  hints, document highlights (the rest of this plan bullet).
- **Phase 3.1 continued — workspace symbols.** `,lw` pre-fills
  `:workspacesymbols ` (mirroring `,lr`'s existing rename-prompt
  pattern) for the query; `request_workspace_symbols` sends
  `workspace/symbol` with `{"query": ...}` -- no `textDocument`/
  position needed, unlike every other LSP request this codebase has --
  and its response is a flat `SymbolInformation[]`-shaped list, which
  `locations()` already parses correctly as-is (it already reads a
  bare `name`/`location` object, the same shape `outline.rs`'s
  `flatten()` handles for flat `documentSymbol` responses), so this
  needed zero new response-parsing code, only new request-side match
  arms. Deliberately left out of the existing single-result auto-jump
  set unlike type definition/implementation/declaration -- a workspace
  symbol *search* should show its match(es) as a picker even when
  there's only one, not silently jump like a "go to this specific
  place" navigation would. `tests/mock_lsp.py` gained a
  `workspace/symbol` handler that echoes the query back into the
  returned symbol's name (via a new `last_opened_uri` bit of script
  state, since `workspace/symbol` params carry no document context) --
  proving the query round-trips end to end, not just that some
  hardcoded list appears -- plus the matching `workspaceSymbolProvider`
  capability flag. 1 regression test (query round-trips into the
  result; Results mode is entered, not an auto-jump) plus
  `tests/pty_workspace_symbols.py` at three terminal sizes. Full suite
  (226 tests) and full existing PTY suite pass unchanged; two latency
  runs against `6836f46` show no regression.
- **Phase 3.1 continued — selection/range formatting.** `,lf` formats
  just the Visual selection's line range (`textDocument/
  rangeFormatting`) when a selection is active, falling back to the
  existing whole-buffer format otherwise -- same "operate on the
  selection if there is one" convention `,gw` already established for
  grep. New `Editor::request_range_format(start_line, end_line)` is a
  standalone method rather than a new `request_language` kind, since
  range formatting needs a line range that method has no parameter
  for; it reuses the existing `"format"` *response* kind/handling as-is
  (a range-formatting reply is the same `TextEdit[]` shape a
  whole-buffer one is), so no response-handling code changed. The
  action handler reads `mode`/`visual_anchor` before calling
  `enter_normal()` (order matters -- clearing them first would lose
  the selection), mirroring `,gw`'s existing visual-selection-reading
  code exactly. `tests/mock_lsp.py` gained a `textDocument/
  rangeFormatting` handler that echoes the requested range's start
  line back into where it places its edit, so a test can confirm the
  actual selected lines reached the server, not just line 0 by
  coincidence, plus the matching `documentRangeFormattingProvider`
  capability. 1 regression test (selecting lines 2..=3 edits line 2,
  not line 0 or any other line; Visual mode is exited) plus
  `tests/pty_visual_format.py` at three terminal sizes. Full suite
  (227 tests) and full existing PTY suite pass unchanged; two latency
  runs against `6836f46` show no regression.
- **Phase 3.5 — LSP progress notifications (partial).** `$/progress`
  was previously silently dropped -- the protocol layer only ever
  acknowledged the `window/workDoneProgress/create` *request* (the
  handshake that lets a server start a token), never parsed the
  notification that actually carries begin/report/end payloads, which
  fell into `poll`'s generic notification catch-all and vanished. New
  `LspEvent::Progress` carries the parsed `$/progress` payload (token,
  kind, title, message, percentage); `Editor::lsp_progress` accumulates
  it per `(client key, token)` across begin/report until `end` removes
  it, and `format_lsp_progress` joins every currently-active token into
  one line shown via the ordinary message line (`set_message`).
  Deliberately no separate persistent progress panel -- reading "job/
  progress UI" as the existing message line, not a new UI surface, is
  a real scope reduction from the plan's literal wording, noted
  honestly rather than silently. On "end", the message line is
  deliberately left alone rather than cleared, since there's no way to
  tell whether it still shows the progress update or something
  unrelated that happened since -- consistent with how every other
  message in this editor already just gets naturally overwritten by
  the next action, not cleared on a timer. `tests/mock_lsp.py` gained
  an opt-in `--progress` flag sending begin (on `initialized`), report
  (on `didOpen`, alongside the diagnostic it already sends) and end (on
  `hover`) -- opt-in specifically so every *other* test's default mock
  behavior, and any assertion on `e.message` right after startup,
  stays unaffected. 1 regression test (title/percentage/message join
  correctly; state clears on "end"; hover itself still works alongside
  it) plus `tests/pty_lsp_progress.py` at three terminal sizes. Full
  suite (228 tests) and full existing PTY suite pass unchanged; two
  latency runs against `6836f46` show no regression (a p99 blip on the
  first run flipped to favor HEAD on the rerun -- confirmed noise, and
  this code only runs in `poll_lsp_events`, gated on active LSP
  clients, never on the hot typing path regardless).
- **Phase 3.6 — completion enable toggle (partial).** New
  `Config::completion_enabled` (default `true`); `update_completion`
  -- the single entry point every insert-mode keystroke already funnels
  through, so this was the only call site needing a change -- returns
  immediately (clearing any existing popup) when it's `false`. This is
  the one change this session that actually sits on the hot per-
  keystroke typing path, unlike almost everything else logged here;
  the added check is a single boolean read, and `insert_char`'s
  benchmark number (the most directly relevant one) came back
  unchanged. Source labels ("lsp"/"buf") and inline `detail` text
  already existed before this session, so most of this plan bullet's
  wording was already partially true; documented that explicitly
  rather than re-claiming credit for pre-existing work. 1 regression
  test (`false` suppresses a real match; re-enabling shows it again)
  plus `tests/pty_completion_toggle.py` at three terminal sizes
  (relaunching the process between the two config states, since config
  is only read at startup). Full suite (228 tests) and full existing
  PTY suite pass unchanged; two latency runs against `6836f46` were
  noisy on the first run across several unrelated labels (machine
  contention, not this change -- `insert_char` itself, the metric that
  actually exercises the new check, matched exactly) and clean on the
  rerun. **Not implemented:** a path completion source, a real
  multi-line documentation preview (distinct from the `detail` text
  already shown), LSP `kind` labels, and a configurable auto-show
  *delay* (the popup still triggers unconditionally on every
  keystroke, just skippable entirely now) -- the rest of this plan
  bullet.
- **Phase 4.1 — git hunk navigation (`]c`/`[c`) (partial).** While
  investigating this, confirmed (by grepping for where `Editor::git`
  gets assigned) that the gutter's git-sign machinery -- `GitGutter`,
  `Sign::{Added,Modified,Removed}`, a throttled background-thread diff
  against `HEAD`, and gutter rendering -- is already fully wired up via
  `ensure_git()` in the main loop; a first pass of grepping missed the
  assignment site (it lives in `gitdiff.rs`, which got excluded by an
  overly narrow search) and briefly suggested the field was dead code.
  Worth recording since acting on that would have wasted effort
  "fixing" something that already worked, or worse, built a duplicate
  mechanism alongside it. Given that confirmation, `Editor::next_hunk`
  reuses the existing `signs` map: sorts its keys, collapses
  contiguous runs into one "hunk start" each (`]c`/`[c` should stop
  once per hunk, not once per changed line -- matching vim-gitgutter/
  fugitive), then finds the next/previous start relative to the
  cursor, wrapping around like `next_diagnostic` already does.
  Deliberately reuses the existing `Awaiting::Diagnostic(bool)` `[`/`]`
  prefix state (adding a `c` arm alongside its existing `d`) rather
  than introducing a new awaiting variant, since both are "jump to the
  next/prev X under the bracket prefix" in the same shape Vim itself
  uses. 2 regression tests (two separate single-line hunks, including
  wrap-around both directions; a contiguous 3-line hunk is one stop,
  not three) plus `tests/pty_git_hunk_nav.py` at three terminal sizes
  against a real git repository, using the gutter's own `~` marker as
  the readiness proxy for the background diff job (mirroring this
  session's established mock-LSP readiness-wait pattern for an
  analogous async-completion problem). Full suite (230 tests) and full
  existing PTY suite pass unchanged; one noisy run and one clean rerun
  against `6836f46` -- the clean one matched exactly across every
  label, confirming the first was machine noise, not a regression.
  **Not implemented:** hunk preview, reset, stage/unstage integrated
  into the gutter itself (already available as a separate :gitstage/
  :gitunstage workflow), selected-range actions, line blame and blame
  toggle (the rest of this plan bullet).
- **Phase 2.1 continued — Git stash picker source (`:gitstash`).**
  `git_tools::stash_list` runs `git stash list` synchronously (like
  `status`/`ignored` -- a cheap, local, no-diff-computation call, not
  worth the background-thread machinery `hunks`/`blame`/`diff` use) and
  returns a Results list, one entry per stash, each tagged with a new
  `_vaayu_git_stash_show` action carrying the stash ref (parsed as the
  text before the first `:`, e.g. `stash@{0}`). `open_result()` gets a
  new branch recognizing that tag and calling `show_git_stash_diff`,
  which runs `git stash show -p --no-color <ref>` (via
  `git_tools::stash_show`) and shows the result as another Results
  list, mirroring how `hunks`/blame/diff already display plain-text
  Git output. `:gitstash` with nothing stashed shows a "No stashes"
  message rather than an empty list. 2 regression tests (list-then-open
  shows the stash's actual diff content; empty stash list shows a
  message, not an empty Results) plus `tests/pty_gitstash.py` at three
  terminal sizes against a real git repository -- the 40-column
  assertion matches a short prefix of the stash message rather than
  the full text (right-truncated at that width), and the diff-view
  assertion searches within the results list (`/STASHED_MARKER`) since
  the added line sits past what fits in a 12-row terminal without
  scrolling. Full suite (232 tests) and full existing PTY suite (46
  files) pass unchanged. Two latency runs against `6836f46` were run;
  the first used a benchmark-label containing a colon
  (`"baseline:6836f46"`), which collided with `bench/latency.py`'s
  `label:command` splitting (`str.split(":", 1)`) and silently fed the
  baseline process a malformed argv that never launched, producing
  implausibly fast "baseline" numbers (sub-millisecond across every
  label, including `enter_insert`) -- caught by that implausibility
  rather than trusted, fixed by using a colon-free label
  (`baseline_6836f46`), and the corrected rerun matched HEAD closely
  across every label (e.g. `insert_char` 3.108ms vs 3.240ms,
  `enter_insert` 7.827ms vs 7.026ms, overall p50 0.493ms vs 0.508ms --
  no regression; this whole feature is reached only from `:gitstash`/
  Results-mode Enter, never the hot typing path). **Not implemented:**
  folding this into a single unified built-in source list alongside
  help/commands/projects (the rest of Phase 2 item 1's plan bullet).
- **Phase 2.7 continued — outline follow-cursor.** `Outline::sync_to_line`
  moves the sidebar's cursor to the symbol enclosing a given buffer line,
  approximated (no end-line/range exists in `SymbolNode`, only a start
  position) the same way aerial.nvim's simple heuristic does: the nearest
  symbol whose start line is `<=` the given line -- exact for a pre-order,
  depth-sorted symbol list, since a parent's next sibling never starts
  before all of the parent's descendants. `Editor::ensure_outline_follow`
  calls it once per frame with the buffer's live cursor line, but only
  when the outline pane itself does *not* have focus (so manual `j`/`k`/
  collapse navigation inside the sidebar is never fought) and only when
  the focused buffer's path matches `outline.buffer_path` (so switching to
  an unrelated buffer while the sidebar is still open doesn't relocate its
  highlight to a nonsense line in that other file); it's a cheap
  `self.outline.is_none()` early return otherwise, so buffers that never
  open the sidebar pay nothing on the per-frame path. Discovered along the
  way: `draw_outline_pane`'s highlight was gated on `active` (this pane
  having keyboard focus), which would have made follow-cursor invisible --
  the whole point is to show the tracked symbol *while the buffer pane has
  focus*. Fixed by decoupling the reverse-video highlight (now shown
  whenever `outline.cursor == y`, regardless of focus) from the blinking
  terminal cursor placement (still gated on `active`, since only the
  actually-focused pane should get a real terminal cursor). 2 regression
  tests (nearest-preceding-symbol selection across three symbols at
  different lines; a no-op when the cursor is above every symbol) plus
  `tests/pty_outline_follow_cursor.py` at three terminal sizes against a
  real (mock) LSP's `--multi-symbol` response, moving the buffer cursor
  with plain `j` after `Ctrl-w w` back to the buffer pane and checking
  pyte's per-cell `reverse` attribute on the sidebar half of the screen
  (not text content, since the highlighted row's text doesn't change --
  only which row is reversed does). The three existing outline PTY tests
  (`pty_outline.py`, `pty_outline_collapse.py`, `pty_outline_filter.py`)
  still pass unchanged, confirming manual in-sidebar navigation still
  highlights correctly with the decoupled condition. Full suite (236
  tests) and full existing PTY suite (47 files) pass unchanged; latency
  against `6836f46` matches closely across every label (e.g. overall p50
  0.472ms vs 0.470ms, `move_down` 0.441ms vs 0.416ms) -- no regression;
  this feature is reached only from the per-frame `ensure_outline_follow`
  early-return check when no outline sidebar is open, which is the
  overwhelmingly common case. **Not implemented:** hover preview (the
  rest of this plan bullet).
- **Phase 2.2 / 2.6 — Results-list preview pane (`p`/`w`/Ctrl-E/Ctrl-Y).**
  Since one `Results`/`Entry` model already backs every list producer
  (grep, diagnostics, quickfix, jumps, Git stash, etc. -- see this log's
  earlier entries), one implementation covers both Phase 2.2's "preview
  toggle/wrap/scroll" and Phase 2.6's "quickfix preview" at once:
  quickfix is a Results list under the hood, so it gets preview for
  free. `Results::preview_rows` (pure, terminal-independent -- takes the
  target file's already-resolved lines plus a row/width/context-before
  budget, returns ready-to-paint rows with an `is_match` flag) replaces
  the existing plain `detail`/`text` strip with real surrounding source
  content once `p` toggles `Results::preview` on, centered on the
  current entry's line and re-derived on every cursor move via a new
  `Results::move_cursor` (which also resets `preview_scroll`, so a
  scroll offset never leaks onto an unrelated entry). `w` toggles
  `preview_wrap` (long source lines split into extra preview rows
  instead of being clipped to one row each, via a small `wrap_chunks`
  helper); Ctrl-E/Ctrl-Y adjust `preview_scroll`. `Editor::
  preview_source_lines` resolves the file's lines from the matching
  open buffer if there is one (so unsaved edits show up), else disk,
  mirroring `language.rs`'s existing outline column-correction fallback.
  Falls back to the old plain `detail`/`text` display when preview is
  off or the current entry has no path (help/keymaps-style plain-text
  lists, or an entry with only detail text like a git hunk/LSP hover).
  Discovered along the way: `draw_results`'s detail area was a fixed 4
  rows only shown above height 12; widened to up to half the screen
  height (clamped so at least 2 rows stay for the list) specifically
  when `preview` is on, since a real content preview needs more room
  than a 4-line detail strip to be useful. 7 pure unit tests in
  `results.rs` (preview off/no-path both return `None`; centers on the
  entry's line with the right row marked as the match; scroll shifts
  the window; stops at end-of-file without padding; wrap splits a long
  line into multiple rows sharing `is_match`; `move_cursor` resets
  scroll) plus one `regression.rs` integration test driving the real
  keys (`p`/Ctrl-E/Ctrl-Y/`w`/`j`) against a real file on disk, plus
  `tests/pty_results_preview.py` at three terminal sizes against a real
  grep hit, confirming: preview off shows only the hit's own line;
  toggling `p` reveals real neighboring source lines (`alpha`/`gamma`
  around a `NEEDLE` match) that were never part of the grep output
  itself; Ctrl-E scrolling drops the earlier context line and Ctrl-Y
  restores it; toggling `p` back off hides the neighboring lines again.
  The three existing outline PTY tests and `pty_grep_word.py`/
  `pty_workspace_symbols.py` (the only other PTY tests asserting on
  Results-mode footer text) still pass unchanged. Full suite (243
  tests) and full existing PTY suite (48 files) pass unchanged. Latency
  against `6836f46` was noisy on the first two runs across several
  unrelated labels including `enter_insert` (reversed direction between
  the two -- baseline faster on one run, HEAD faster on the next, the
  textbook signature of machine contention rather than a real
  regression) and clean on the third, matching closely across every
  label (`insert_char` 2.432ms vs 2.388ms, `enter_insert` 4.719ms vs
  4.009ms, overall p50 0.247ms vs 0.407ms) -- no regression; this
  feature is reached only from Results-mode key handling and rendering,
  never the hot typing path. **Not implemented:** quickfix filtering
  (the rest of Phase 2.6's plan bullet).
- **Phase 2.5 finished — rollback on partial copy failure.**
  `filetree.rs`'s `copy_recursive` (used by `p` for a non-cut paste of a
  directory) is now a thin wrapper around the original recursive logic
  (renamed `copy_recursive_step`): on any error partway through --
  permission denied on one nested file, disk full, anything -- it
  removes whatever was already created at the top-level `dest` (via
  `remove_dir_all` or `remove_file` as appropriate, best-effort, errors
  ignored) before propagating the original error. Only the outermost
  call needs to do this: removing the top-level `dest` recursively
  cleans up every nested partial file/directory a failed recursive
  descent left behind, so there was no need to instrument every
  recursive call site individually. Without this, a failed directory
  copy left a half-populated destination directory behind that a
  subsequent `y`/`p` would then refuse to overwrite with a confusing
  "already exists" -- the destination looked complete but wasn't. 1
  regression test in `filetree.rs` (a directory with one readable and
  one chmod-0 file; the copy fails and the destination directory is
  confirmed absent afterward, regardless of `read_dir`'s unspecified
  entry order) plus `tests/pty_filetree_copy_rollback.py` at three
  terminal sizes, using the same chmod-0 technique through the real `y`/
  `p` keys against a real directory tree. The existing
  `pty_filetree_copy_paste.py` still passes unchanged. Full suite (244
  tests) and full existing PTY suite (49 files) pass unchanged; latency
  against `6836f46` matched closely on the first run across every label
  (e.g. `insert_char` 0.979ms vs 1.068ms, `enter_insert` 2.326ms vs
  2.011ms, overall p50 0.191ms vs 0.193ms) -- no regression; this is a
  filesystem-error-path-only change, never reached by ordinary
  successful copies or the hot typing path. **Phase 2 item 5 is now
  fully done.**
- **Phase 3.6 continued — completion popup kind labels.** LSP
  `CompletionItemKind` (a distinct 1-25 numbering from `SymbolKind`,
  which `outline::kind_label` already maps -- the two enums don't share
  values) is now parsed (`CompletionResultItem::kind`, threaded through
  `completion::Item::kind`) and shown via a new `completion::kind_label`
  in place of the generic "lsp" source tag whenever the server sends
  one, e.g. "fn"/"var"/"class"/"keyword". Buffer-word candidates have no
  kind and keep the "buf" tag. 4 unit tests (`kind_label` for a few
  known values and its fallback for an unknown one) plus one
  `regression.rs` integration test driving a real mock-LSP completion
  round trip (typing "fi" fuzzy-matches the fixture's "FIX" filterText,
  its reply now carries `"kind": 3`) confirming the popped item's
  `kind` is `Some(3)` and maps to "fn", plus
  `tests/pty_completion_kind_label.py` at three terminal sizes showing
  the " fn " label actually painted in the popup. Note for future
  editing of this popup: while `ed.completion` is open, Esc only closes
  the popup (doesn't leave Insert mode) -- the PTY test needed two Esc
  presses to exit cleanly, a trap the first draft of the test hit.
  Existing `pty_completion_toggle.py`/`pty_typing_ui.py` still pass
  unchanged. Full suite (246 tests) and full existing PTY suite (50
  files) pass unchanged. Latency against `6836f46` was noisy on the
  first run (a stray `vy` process from an earlier PTY run was still
  alive in the background, found and killed via `ps aux`) and clean on
  the rerun, matching closely across every label (`insert_char` 3.148ms
  vs 3.210ms, `enter_insert` 8.207ms vs 7.496ms, overall p50 0.477ms vs
  0.509ms) -- no regression; this only touches completion-popup parsing
  and rendering, never the hot typing path itself. **Not implemented:**
  path completion source, a real documentation preview and a
  configurable auto-show delay (the rest of Phase 3 item 6's plan
  bullet).
- **Phase 4.6 finished — GitHub permalink generation.**
  `git_tools::remote_url`/`head_commit` wrap `git remote get-url origin`/
  `git rev-parse HEAD` (both cheap, local, synchronous, like
  `status`/`ignored`/`stash_list`); `parse_github_remote` is a pure
  function turning either remote form (SSH `git@github.com:owner/
  repo.git`, HTTPS, or the `ssh://git@github.com/...` long form,
  trailing `.git` optional) into `(owner, repo)`, `None` for a
  non-github.com remote. `Editor::generate_permalink(path, start, end,
  commit)` builds `https://github.com/{owner}/{repo}/blob/{commit}/
  {relative_path}#L{n}` (or `#L{n}-L{m}` for a range), pinned to the
  commit SHA rather than a branch name (which can move), and copies it
  to the clipboard/`+` register -- same mechanism the existing Results
  `y`/`Y` copy already uses. `commit=None` resolves to HEAD; `,gp` (new
  leader action, mirroring `,gw`/`,lf`'s "operate on the Visual
  selection's line range if one is active, else just the cursor" own
  convention) and `:permalink` (cursor line only, matching `:format`'s
  narrower ex-command scope vs. `,lf`'s Visual-aware one) both pass
  `None`. The "selected commit" wording in this plan bullet is handled
  by a new `Editor::permalink_from_results_entry` (bound to `P` in any
  Results list): for a `:gitblame` list specifically, it parses that
  entry's own leading commit hash (blame's first whitespace-delimited
  token, stripping a leading `^` for a boundary commit) out of the
  already-displayed blame text and passes it as the override, so the
  link points at the commit that actually introduced the line rather
  than the tip of the branch; any other Results list falls back to
  HEAD. Deliberately generates and copies a URL only -- never opens a
  browser or makes a network request, so this is safe to run offline
  or against a private repo with no further confirmation needed. 4 pure
  unit tests for `parse_github_remote` (SSH, HTTPS without `.git`, the
  `ssh://` long form, and rejecting a non-GitHub remote) plus 5
  `regression.rs` integration tests against a real git repo with a
  fake `git@github.com:acme/widgets.git` origin (cursor-line `,gp`;
  Visual-line-range `,gp`, also confirming it leaves Visual mode;
  `:permalink`'s cursor-only scope; `permalink_from_results_entry`'s
  selected-commit behavior from a synthetic `:gitblame`-titled Results
  list; a clear message instead of a URL when there's no `origin`
  remote) plus `tests/pty_permalink.py` at three terminal sizes against
  a real repository -- the message line's URL is long enough (a 40-hex-
  char SHA alone pushes it past 100 columns) that the narrower two
  terminal sizes only check for the "Copied permalink" prefix, with the
  owner/repo substring checked at 100+ columns and the full commit/path/
  line fragment only at 180. Full suite (255 tests) and full existing
  PTY suite (51 files) pass unchanged; latency against `6836f46` matched
  closely on the first run across every label (`insert_char` 3.130ms vs
  3.173ms, `enter_insert` 7.683ms vs 8.938ms, overall p50 0.485ms vs
  0.491ms) -- no regression; this feature is reached only from `,gp`/
  `:permalink`/Results-mode `P`, never the hot typing path. **Phase 4
  item 6 is now fully done.**
- **Phase 3.1 continued — document highlights (`,lh`).**
  `textDocument/documentHighlight` returns spans, not jump points, so it
  needed its own response-parsing arm rather than reusing the existing
  `locations()`-based one that every other Phase 3.1 feature (definition/
  references/outline/workspace symbols) shares -- that helper only keeps
  a location's *start* position, discarding the end, which is exactly
  the part an in-buffer highlight overlay needs. The new arm parses each
  `{range: {start,end}}` into a `(line1, col1, line2, col2)` char-range
  (UTF-16-corrected the same way outline/locations already are) and
  stores them in `Editor::document_highlights`, tagged with which buffer
  and `edit_seq` they're for. `render.rs`'s per-line painting clips each
  range to the line being drawn (a single-line range keeps its own
  start/end columns; a multi-line one runs start-column..end-of-line on
  its first line, the whole line in between, start-of-line..end-column
  on its last line) and paints a `DarkBlue` background, at lower
  priority than Visual selection (reverse video) and search highlight
  (`DarkYellow`) as already established when they overlap. Deliberately
  request-triggered (`,lh`) rather than automatically on cursor hold --
  no cursor-hold/debounce timer infrastructure exists in this codebase,
  and every other Phase 3 feature here (hover, references, definition,
  ...) is already request-triggered the same way, so this stays
  consistent rather than introducing a new interaction pattern for one
  feature. **Staleness handling:** a plain Esc in Normal mode clears
  `document_highlights` outright (added to the same catch-all Esc arm
  that already resets pending state); short of that, if the buffer is
  edited after a request, its `edit_seq` no longer matches the snapshot
  taken at request time, and `render.rs` silently skips painting rather
  than highlighting whatever now sits at those stale positions --
  verified directly by a unit test rather than only inferred. 2
  `regression.rs` tests (a real mock-LSP round trip populating exactly
  the two fixed ranges the fixture replies with, then confirming Esc
  clears them; the staleness bookkeeping after a real edit) plus
  `tests/pty_document_highlight.py` at three terminal sizes, using
  pyte's per-cell background-color introspection (the same technique
  `pty_outline_follow_cursor.py` established for verifying an overlay
  that doesn't change text content, only styling) to confirm: `,lh`
  paints both fixture ranges; deleting a character makes the highlight
  stale and it stops painting; undoing and re-requesting re-highlights;
  Esc clears it. Full suite (257 tests) and full existing PTY suite (52
  files) pass unchanged. Latency against `6836f46` needed three runs:
  `insert_char` matched within ~0.1ms across all three (e.g. 3.024ms/
  3.081ms/3.157ms baseline vs 3.095ms/3.128ms/3.153ms HEAD) confirming
  no hot-typing-path regression, but `search_open`/`search_next`/
  `search_submit` showed a consistent (non-flipping) ~1.3-1.8x gap
  across all three runs rather than the usual noise pattern seen
  elsewhere in this log (which typically reverses direction between
  runs). Checked this specific pattern against every prior slice's
  recorded numbers in this same log rather than dismissing it outright:
  the same labels already showed equally wide, non-flipping-looking
  spreads in at least two earlier slices that were independently judged
  clean by their own `insert_char`/`enter_insert` checks (e.g. the
  `permalink` slice's own `search_open` was 0.247ms baseline vs 0.422ms
  HEAD), so this is this machine's pre-existing sampling noise for a
  label the benchmark exercises far fewer times than typing labels, not
  something newly introduced here -- recorded for whoever reads this
  log next rather than silently normalized away. **Not implemented:**
  CodeLens, document links and inlay hints (the rest of Phase 3 item
  1's plan bullet).
- **Phase 4.1 continued — hunk preview (`,gh`).** Reuses
  `git_tools::hunks()` (the same call `:gitstage`'s results list
  already makes) rather than the gutter's own `signs` map that `]c`/
  `[c` navigate with, because "which hunk is the cursor inside" needs
  each hunk's actual line *range*, and `signs` only marks individual
  changed lines (contiguous runs), not hunk boundaries with confirmed
  start/end -- computing a hunk's range instead from its own `@@ -a,b
  +c,d @@` header (a new `hunk_range` helper, extending the same
  header-parsing `hunks()` already does inline for each entry's
  `.line`) is exact and needs no cross-referencing between the two data
  sources. Finds the hunk containing the cursor's line, or (cursor
  between hunks) the nearest one starting before it, and shows its
  diff as a read-only "Hunk preview" Results list -- deliberately not
  reusing `:gitstage`'s list (which stages on Enter); this is preview-
  only, matching the plan wording. Synchronous like `:gitstash`
  (a single `git diff` call, not worth the background-thread machinery
  `git_results`'s "diff"/"blame" kinds use). Refuses on an unsaved
  buffer with a clear message, same as `:gitstage`/`:gitunstage`
  already do, since the diff would otherwise describe stale disk
  content. 3 pure unit tests for `hunk_range` (a normal header; a
  header with the count omitted, which git does whenever it would be
  `1`; a pure-deletion header with `+c,0`, checked specifically since a
  naive `count - 1` would underflow) plus 2 `regression.rs` integration
  tests (two hunks 13 lines apart -- far enough that `--unified=3`'s
  context windows can't merge them into one -- confirming the preview
  for each shows only its own change, never the other one; the unsaved-
  buffer refusal) plus `tests/pty_hunk_preview.py` at three terminal
  sizes against a real two-hunk repository, searching within the
  results list (`/CHANGED_ALPHA` etc.) since the change sits past what
  fits on screen without scrolling. Full suite (262 tests) and full
  existing PTY suite (53 files) pass unchanged. Latency against
  `6836f46` matched closely on the first run across every label,
  including the `search_*` labels flagged as noisy in the previous
  entry (e.g. `search_open` 0.483ms vs 0.509ms this time, well within
  the spread already established) -- `insert_char` 3.045ms vs 3.113ms,
  `enter_insert` 7.337ms vs 7.107ms, overall p50 0.653ms vs 0.636ms, no
  regression; this feature is reached only from `,gh`, never the hot
  typing path. **Not implemented:** reset, selected-range actions, line
  blame and blame toggle (the rest of Phase 4 item 1's plan bullet).
- **Phase 2.7 finished — outline hover preview (`K`).** `request_language`
  always builds a hover request from `self.cursor()` (the buffer's real,
  live cursor), not an arbitrary position, so "preview the outline
  symbol without navigating" briefly moves the real cursor to the
  symbol's position, fires `request_hover()` (which embeds that
  position into the outgoing JSON synchronously, before the function
  returns), then restores the original cursor immediately -- the
  response arrives later and the existing hover handler never re-reads
  cursor position, only displays the returned text, so this never
  disturbs the user's actual editing position even though it touches
  the real cursor for a moment. A no-op if the outline is showing a
  different (or no) document than the one currently open (stale after
  a buffer switch) via the same `outline.buffer_path` check
  `ensure_outline_follow` already established, so it never hovers the
  wrong file's position. 2 `regression.rs` tests (a real mock-LSP round
  trip: moves the buffer's real cursor away from the symbol first, then
  confirms hovering leaves it untouched while still showing the hover
  text; a stale-outline no-op) plus `tests/pty_outline_hover.py` at
  three terminal sizes, comparing the status line's `line:col` segment
  (extracted with a regex, since the raw status line differs
  cosmetically once the sidebar's split narrows the buffer pane and
  truncates the filename) before and after hovering to confirm the
  cursor really didn't move. The four existing outline PTY tests still
  pass unchanged. Full suite (264 tests) and full existing PTY suite
  (54 files) pass unchanged. Latency against `6836f46` matched closely
  on the first run across every label (`insert_char` 3.134ms vs
  3.096ms, overall p50 0.630ms vs 0.637ms) -- no regression; this
  feature is reached only from `K` inside the outline sidebar, never
  the hot typing path. **Phase 2 item 7 is now fully done.**
- **Phase 2.1 continued — `:commands` picker source.** A new
  `command::EX_COMMANDS: &[(&str, &str)]` constant -- one canonical
  name per ex command (the full word where one exists, e.g. `quit` not
  `q`; a browse/select UI benefits from a clear name the way typing
  benefits from a short alias) paired with a one-line description --
  feeds a Results list exactly like `:keymaps`' existing `ACTIONS`
  registry does for leader bindings, including the same soft-drift
  tradeoff: nothing at compile time keeps a name here in sync with
  `run_ex`'s match arms, the same as `ACTIONS` already accepts for
  leader keys. Selecting an entry does *not* re-run it immediately the
  way `:chistory`/`:shistory`'s existing `_vaayu_rerun_ex` tag does --
  a new `_vaayu_prefill_ex` tag instead opens the command line
  pre-filled with `"<name> "`, since most commands need arguments a
  bare name can't supply (a blindly-executed bare `:rename` or `:grep`
  would just be a confusing no-op); the user adds arguments and
  presses Enter themselves. Corrected two stale claims in this same
  plan bullet while touching it: `:help` and `:keymaps` were already
  separate list sources before this session even started, not "not
  done" as an earlier pass of this log's own bullet claimed (caught by
  actually checking `command.rs` rather than trusting the existing
  annotation). 2 `regression.rs` tests (every entry present, sorted,
  and tagged; selecting one pre-fills without running, then finishing
  it like a real user would -- typing a pattern and Enter -- actually
  runs it) plus `tests/pty_commands_picker.py` at three terminal sizes,
  searching within the list (`/:grep `) since the target entry sits
  well past what fits on screen among 78 entries, confirming the
  pre-fill-not-run distinction and the follow-through run visually.
  Full suite (266 tests) and full existing PTY suite (55 files) pass
  unchanged. Latency against `6836f46` matched closely on the first
  run despite an unrelated `vy` process already running on the machine
  at benchmark time (left alone rather than killed, in case it was
  someone's real editing session) -- `insert_char` 3.138ms vs 3.107ms,
  `enter_insert` 8.898ms vs 7.956ms, overall p50 0.584ms vs 0.625ms, no
  regression; this feature is reached only from `:commands` and its
  Results-list entries, never the hot typing path. **Not implemented:**
  a projects picker source and a single unified built-in source list
  (the rest of Phase 2 item 1's plan bullet).
- **Cross-cutting fix (user-reported, not itself a plan item) --
  highlight colors clashing with terminal theme.** `render.rs`'s
  `plain_row` (backing every Results/picker/help/file-tree list row)
  hardcoded a White foreground regardless of background; paired with
  the overwhelmingly common `Color::Reset` (default) background used
  for every non-cursor row, this rendered as invisible white-on-white
  on any light-background terminal theme -- not just a cosmetic
  mismatch but a real readability bug for a whole class of terminal
  themes. Fixed to only force White when paired with a deliberately
  non-default background (e.g. a selected row's DarkCyan); a Reset
  background now leaves the foreground at Reset too, so plain rows
  render with the terminal's own default colors. Separately, the
  buffer's search-match (DarkYellow bg) and document-highlight
  (DarkBlue bg) overlays left whatever arbitrary syntax color the
  underlying token had as foreground, which could clash badly (a Cyan
  keyword on a DarkYellow background, for instance) independent of the
  terminal's theme; both now force a specific readable foreground
  (Black on search's yellow, White on doc-highlight's blue) instead.
  Plain Visual selection (`Attribute::Reverse`) was already correct as
  a pure swap of whatever colors are already there and needed no
  change. 3 unit tests confirming both the buggy and fixed behavior
  precisely by crossterm's actual emitted SGR codes (256-color-palette
  form, `38;5;<n>`/`48;5;<n>`, confirmed empirically rather than
  assumed after an initial guess at basic-ANSI codes turned out wrong
  and would have made the tests vacuously pass) -- one deliberately
  reverted to confirm it fails without the fix, not just always-green
  -- plus `tests/pty_highlight_colors.py` at three terminal sizes
  checking pyte's actual per-cell fg/bg attributes. Full suite (269
  tests at the time) and full existing PTY suite (56 files) pass
  unchanged. Latency against `6836f46` matched closely on the first run
  (`insert_char` 3.095ms vs 3.111ms, overall p50 0.584ms vs 0.618ms) --
  no regression, as expected since this only changes color parameters,
  not the render hot path's control flow.
- **Phase 3.6 continued — completion path source.** Typing a path-
  shaped prefix (contains a `/`) now triggers a dedicated "path" source
  instead of ordinary buffer/LSP completion -- the two are never
  simultaneously valid (a partial path is never also a real
  identifier), so detecting one wins outright with no ambiguity to
  resolve. A new `completion::path_prefix` mirrors `word_prefix`'s
  backward scan but extends the character class to include `/`, `.`,
  `-`, requiring the result to actually contain a `/` before it
  "wins" (a plain identifier that happens to have a `-` or `.` in it
  must still fall through to normal completion). `completion::
  path_candidates` resolves the prefix's directory portion against the
  buffer's own directory (or `project_root` for an unsaved buffer)
  and lists real `std::fs::read_dir` entries matching the file portion,
  directories first (shell/editor convention) then alphabetically,
  each tagged `Source::Path` and shown as "path" in the popup (new
  match arm alongside the existing "lsp"/"buf" tags). `insert_text` is
  the *whole* replacement including the directory portion (e.g.
  `"assets/logo.png"`, not just `"logo.png"`) since acceptance replaces
  the entire span from the prefix's start to the cursor, the same
  convention every other completion source already follows. 5 pure
  unit tests (whole-partial-path capture; no-slash rejection; stopping
  at a quote; matching entries sorted dirs-first excluding dotfiles
  and non-matches, using a real temp directory; insert_text including
  the directory portion) -- one caught a real sorting bug during
  development (`(bool, String)` tuples sort false-before-true by
  default, putting files before directories, backwards from the
  intended convention; the test's own expectation, not just the code,
  had to be fixed once the intended order was made explicit) -- plus 2
  `regression.rs` integration tests (a real temp directory's file shows
  up relative to the buffer's own path; a plain identifier never
  triggers path completion) plus `tests/pty_path_completion.py` at
  three terminal sizes confirming the "path" tag, the listed file, and
  that accepting it inserts the full relative path. Full suite (276
  tests) and full existing PTY suite (57 files) pass unchanged. Latency
  against `6836f46` was checked with particular attention to
  `insert_char` specifically, since `path_prefix` is a second backward
  scan added to the very top of `update_completion` -- called on every
  single insert-mode keystroke, the hottest path in the editor -- and
  matched almost exactly (3.111ms vs 3.117ms; overall p50 0.571ms vs
  0.628ms), confirming the added scan (bounded by the current line's
  length, same order of cost as the pre-existing `word_prefix` scan
  every keystroke already paid for) is negligible. **Not implemented:**
  a real documentation preview and a configurable auto-show delay (the
  rest of Phase 3 item 6's plan bullet).
- **Phase 3.6 continued — completion documentation preview.** A new
  `completion::item_documentation` reads the LSP `documentation` field
  directly out of an item's already-present `raw` JSON -- no extra
  `completionItem/resolve` round trip, since `resolve` (already wired,
  see `Editor::resolve_completion`) is hard-committed to immediately
  accepting the item once its response arrives (used only for the
  "apply additional edits on accept" case), not a fit for "preview
  while just browsing." `documentation` can be a plain string or
  `MarkupContent {kind, value}`; both are read as-is (no markdown
  rendering, just the text). `render.rs`'s popup gets up to 5 extra
  rows directly below the item list showing the *selected* item's
  documentation, only allocated when it actually has one -- most items
  don't, so this never grows the popup for nothing. Distinct from
  `detail` (the short one-line signature/type already shown inline on
  each row). 6 unit tests (plain-string form; MarkupContent form;
  absent/empty-after-trim all counting as "none") plus one
  `regression.rs` extension of the existing kind-label round-trip test
  (mock_lsp.py's completion fixture now also sends `documentation`)
  confirming the real end-to-end extraction, plus
  `tests/pty_completion_doc_preview.py` at three terminal sizes showing
  the "Docs:" header and text actually painted. The three existing
  completion PTY tests still pass unchanged. Full suite (279 tests) and
  full existing PTY suite (58 files) pass unchanged. Latency against
  `6836f46` matched closely on the first run across every label
  (`insert_char` 3.069ms vs 3.107ms, `enter_insert` 6.772ms vs 6.825ms,
  overall p50 0.562ms vs 0.576ms) -- no regression; the doc-preview
  rendering only runs when the popup is open and the selected item
  actually has documentation, never on the hot typing path itself.
- **Phase 3.6 finished — completion auto-show delay
  (`completion_delay_ms`).** Mirrors `whichkey_delay_ms`'s existing
  render-time-gating technique rather than deferring computation
  itself: candidates are still computed immediately on every keystroke
  (cheap either way), but a new `Editor::completion_since` (reset
  whenever `update_completion` (re)populates `completion`, cleared
  together with it by `close_completion`) gates whether `render.rs`
  actually paints the popup, the same way the which-key popup's own
  `since` gates its reveal. Default `completion_delay_ms` is `0`, so
  `elapsed >= Duration::from_millis(0)` is immediately true and nothing
  changes for anyone who hasn't configured a delay. **Caught during
  testing, not assumed:** setting the config alone wasn't enough --
  `main.rs`'s event loop only redraws when a key arrives or
  `poll_lsp_events()`/`poll_jobs()`/`poll_terminals()` reports real
  work, so a delay elapsing with no further keystroke would otherwise
  never trigger the one extra redraw needed to actually paint the
  now-due popup (the popup would only appear on the *next* keystroke,
  defeating the point of an idle-reveal delay). Fixed by adding a
  third wake-up block to the main loop, structurally identical to the
  existing `Awaiting::Leader` which-key wait immediately above it:
  poll bounded by the remaining delay, then loop back to redraw either
  way. A first version of `tests/pty_completion_delay.py` (which waits
  idly after typing, sending no further keys) caught this by simply
  never passing, before the fix was in place -- exactly the gap a
  render-only unit test checking `ed.completion_since` in isolation
  would have missed entirely. 1 `regression.rs` test (backdates
  `completion_since` via `Instant::now() - Duration` rather than a
  real sleep, confirming the render output omits the " buf " popup
  tag before and includes it after) plus that PTY test at three
  terminal sizes, confirming both the hidden-then-revealed behavior
  and the idle-redraw fix together. The four other completion PTY
  tests and `pty_whichkey.py` (proving the new main-loop block doesn't
  interfere with the existing which-key wait it sits next to) still
  pass unchanged. Full suite (280 tests) and full existing PTY suite
  (59 files) pass unchanged. Latency against `6836f46` was checked
  across *every* label, not just `insert_char`, since this change adds
  a check to the main event loop itself, on every single loop
  iteration regardless of mode -- all matched closely (`insert_char`
  3.093ms vs 3.116ms, `move_down` 0.563ms vs 0.619ms, `undo` 0.602ms
  vs 0.584ms, overall p50 0.624ms vs 0.643ms), confirming the `if let
  Some(since) = ed.completion_since` check costs nothing when `None`
  (the common case outside an active completion session). **Phase 3
  item 6 is now fully done.**
- **Phase 4.1 continued — line blame and blame toggle (`,gB`).**
  `,gh`'s hunk preview reused the existing `hunks()` diff machinery, but
  line blame needed its own background-thread git call (`spawn_blame`,
  the same pattern `git_results`'s own "blame" kind already uses to
  avoid blocking the main loop on a large file/history) since it reads
  `git blame` output directly rather than a diff. A new `blame_line_meta`
  reduces one plain `git blame` line (e.g. `abc1234 (Author Name
  2024-01-15 10:23:45 +0000  5) content`, optionally `^`-prefixed for a
  boundary commit) to `"<hash> <author/date, tz>"` by stripping only the
  trailing line number -- the parenthesized metadata is free-form author
  name plus date/time/tz with no fixed field count, so the line number
  (always the last whitespace token before the close paren) is the only
  piece safely strippable without knowing the author name's own word
  count. `render.rs`'s per-row loop appends this as dimmed virtual text
  after the buffer's *current* line's own content (never other lines,
  and never when the buffer doesn't match `line_blame_path`, e.g. after
  switching buffers with blame still toggled on for a different file);
  a new `RowSignature.blame` field makes the row cache invalidate
  correctly as the cursor moves, blame toggles on/off, or the async
  data first arrives. `,gB` toggles `Editor::blame_toggle`; turning it
  off drops any loaded data and in-flight request outright rather than
  keeping it around for a possible re-enable, since there's nothing
  left to show either way. **Bug caught by testing, not assumed:** the
  first version used `git blame --no-color`, copying the flag from
  `git diff`'s own invocation elsewhere in this file without checking
  that `git blame` actually supports it -- it doesn't (`--no-color` is
  ambiguous with `--no-color-lines`/`--no-color-by-age` and git refuses
  to guess, exiting 129 immediately). The first version of the
  integration test caught this cleanly: rather than hanging outright,
  it looped forever because `poll_blame_task`'s error path clears
  `blame_toggle` without ever setting `line_blame`, and the test's
  original `while line_blame.is_none()` loop had no way to distinguish
  "still pending" from "already failed" -- fixed both the missing flag
  (plain `git blame` already has no color when not attached to a tty,
  same reasoning `git_results`'s existing "blame" kind already relies
  on) and the test's loop condition (now asserts `blame_toggle` stays
  true while waiting, failing fast with the real error message instead
  of a bare timeout). 3 pure unit tests for `blame_line_meta` (normal
  line; boundary-commit `^` prefix; malformed input falls back
  gracefully instead of panicking) plus 2 `regression.rs` integration
  tests (a real repo's real commit author appears in the parsed data
  and in actual render output for the current line only; toggling
  without a file on disk refuses with a clear message) plus
  `tests/pty_line_blame.py` at three terminal sizes confirming the
  annotation appears only on the current line, follows the cursor to a
  new line, and disappears on toggle-off. The git-hunk-nav and hunk-
  preview PTY tests still pass unchanged. Full suite (285 tests) and
  full existing PTY suite (60 files) pass unchanged. Latency against
  `6836f46` matched closely across every label, checked carefully since
  this touches both the per-row render hot path and `poll_jobs` (now
  polled every idle tick) -- `insert_char` 3.161ms vs 3.134ms,
  `move_down` 0.610ms vs 0.615ms, overall p50 0.619ms vs 0.650ms, no
  regression; `blame_toggle` defaults to `false` and the poll is a
  cheap `Option::as_ref` check when no task is in flight. **Not
  implemented:** hunk reset (discarding a hunk back to HEAD) and
  selected-range actions (the rest of Phase 4 item 1's plan bullet) --
  reset specifically needs a "reload this buffer's content from disk"
  primitive that doesn't exist anywhere in this codebase yet (every
  existing git-write operation, stage/unstage, only ever touches the
  index, never the working tree file an open buffer might already
  have loaded), so it's deliberately left for its own dedicated slice
  rather than bolted on without that groundwork.
- **Phase 2.6 finished — Results/quickfix filtering (`f`).** Mirrors
  `outline::Outline`'s existing `all_nodes`/`nodes` split exactly:
  `Results` gains `all_entries` (the full, unfiltered list from the
  last producer call) and a new `apply_filter` re-derives the
  displayed `entries` from it by case-insensitive substring match
  (against an entry's `text` or `detail`) whenever `filter` changes.
  Because `entries` keeps its existing name and meaning ("what's
  currently shown"), every one of the ~15+ existing call sites that
  already read `r.entries` throughout the codebase (render, export,
  quickfix conversion, preview, open_result, ...) needed zero changes
  -- they just see a possibly-narrower list now, with no call site
  needing to know filtering exists. This is exactly the low-risk
  design this same log flagged wanting when this item was first
  deferred (a session-earlier version of this log worried a filter
  would require touching every index-sensitive call site; encapsulating
  the invariant inside the type itself, the way outline already had,
  turned out to avoid that entirely). `apply_filter` also clamps
  `cursor` into the new length and clears `selected` outright, since
  indices into the old, differently-sized `entries` can't be trusted
  to still mean the same thing after the list is narrowed or widened --
  the same invariant `Outline::apply_filter` already preserves for its
  own kind-filter. `f` starts editing with a *fresh* empty filter each
  time (not resuming the previous text), so a second `f` press is also
  how the filter gets cleared; each keystroke re-applies immediately
  (a live narrow, not a submit-to-apply). Deliberately excluded: a
  `live` grep list, which already replaces `entries` wholesale on every
  keystroke from its own background search job (`jobs.rs`'s
  `schedule_grep`/`poll_jobs`), a separate update path layering a
  filter on top of would need to hook into rather than just reading
  `entries` at render/export time. 4 pure unit tests (narrows by a
  case-insensitive substring; also matches `detail`, not just `text`;
  clearing the filter restores everything; clamps `cursor` and drops
  stale `selected` indices when the list is narrowed) plus 1
  `regression.rs` integration test driving the real keys against
  `:commands`' real ~75-entry list (narrows, Enter keeps it applied
  while browsing, a fresh `f` clears it) plus
  `tests/pty_results_filter.py` at three terminal sizes, reading the
  "N/total results" header via regex rather than a hardcoded count (so
  the test doesn't silently drift if `EX_COMMANDS` grows or shrinks).
  The `:commands`, results-preview and grep-word PTY tests still pass
  unchanged. Full suite (290 tests) and full existing PTY suite (61
  files) pass unchanged. Latency against `6836f46` matched closely on
  the first run across every label (`insert_char` 3.052ms vs 3.074ms,
  `move_down` 0.611ms vs 0.642ms, overall p50 0.624ms vs 0.661ms) -- no
  regression; this feature is reached only from Results-mode `f` and
  its own key handling, never the hot typing path. **Phase 2 item 6 is
  now fully done.**
- **Phase 3.5 finished — persistent LSP progress indicator.** The
  message-line surfacing this session's earlier `$/progress` slice
  already shipped has a real gap: any other action's own message (a
  save confirmation, a search result, anything) silently overwrites it
  mid-indexing, and there's no way to tell afterward whether the
  message line still shows progress or something unrelated. Rather
  than adding a whole new UI region, the fix reuses the per-pane
  status line that already exists at the bottom of every buffer pane
  (mode/filename/cursor-position) -- unlike `ed.message`, that line is
  recomputed from scratch every single frame directly from live state,
  so it can't go stale or get silently clobbered the way a one-shot
  `set_message` call can. `format_lsp_progress` (previously private)
  is now `pub(crate)` so `render.rs` can call it; the active pane's
  status line prepends its output (clipped to 40 columns so a long
  title/message can't push the cursor-position segment off the edge of
  a narrow terminal) before the existing `line:col` segment. Only the
  *active* pane shows it (progress is global to the session, not
  per-buffer, so every split showing the same text would just be
  noise). The message-line surfacing stays exactly as it was --
  genuinely useful for a one-time "just started"/"just finished"
  notice -- this adds the second, persistent surface on top rather
  than replacing it. 1 `regression.rs` test rendering a real frame with
  fake progress state plus an unrelated `set_message` call, confirming
  both survive in their own separate places, then confirming the
  indicator disappears once progress is cleared, plus
  `tests/pty_lsp_progress_status_line.py` at three terminal sizes
  against a real (mock) language server, using `:w`'s own save message
  as the "unrelated action" and checking survival by a quote-mark
  proxy rather than the word "written" itself, since a long tmp-dir
  path can push "written" off the edge of a 40-column terminal (a
  lesson already learned and reused from earlier slices in this log).
  The existing message-line-only progress PTY test still passes
  unchanged. Full suite (291 tests) and full existing PTY suite (62
  files) pass unchanged. Latency against `6836f46` matched closely
  across every label, checked carefully since `format_lsp_progress` is
  now called unconditionally on every single frame render for the
  active pane (not just when progress is actually active) --
  `insert_char` 3.046ms vs 3.054ms, `move_down` 0.590ms vs 0.622ms,
  overall p50 0.605ms vs 0.652ms, no regression; the common case (no
  active progress) is a single cheap `HashMap::is_empty()` check.
  **Phase 3 item 5 is now fully done.**
- **Phase 3.1 continued — document links (`,ll`/:documentlinks).**
  Deliberately scoped as a Results-list feature (reusing the exact
  same list/entry/action-tag infrastructure every other producer this
  session already uses) rather than an in-buffer overlay -- unlike
  document highlights, a document link's whole purpose is to be
  *opened*, not just seen, so a selectable list with an action per
  entry is a better fit than painting ranges in the buffer, and it
  needed zero new rendering code. `textDocument/documentLink`'s
  `target` field is optional (a server can defer it to `documentLink/
  resolve`, the same lazy pattern completion items already use for
  `documentation`/edits); rather than adding another resolve round
  trip, a link with no inline target is simply skipped -- consistent
  with the same scoping choice already made for the completion
  documentation preview. Each entry is tagged with a new
  `_vaayu_open_link` action: a `file://` target opens directly (same
  as jumping to any other location, via `crate::files::from_uri`);
  anything else (http(s), an unresolvable scheme) is copied to the
  clipboard/`+` register instead of opened -- the same "generate/copy,
  never open a browser" choice already made for `,gp` permalinks. 1
  `regression.rs` integration test against a real mock-LSP round trip
  (a resolvable sibling `file://` link that actually opens on Enter; an
  `https://` link that copies instead of switching buffers) plus
  `tests/pty_document_links.py` at three terminal sizes confirming the
  same two behaviors visually, including that opening the web link
  first doesn't disturb the buffer before the file link is opened
  second. The `:commands` and document-highlight PTY tests still pass
  unchanged (`:documentlinks` added to `command::EX_COMMANDS`). Full
  suite (292 tests) and full existing PTY suite (63 files) pass
  unchanged. Latency against `6836f46` matched closely on the first run
  across every label (`insert_char` 3.042ms vs 3.135ms, overall p50
  0.643ms vs 0.671ms) -- no regression; this feature is reached only
  from `,ll`/`:documentlinks` and its own Results-list entries, never
  the hot typing path. **Not implemented:** CodeLens and inlay hints
  (the rest of Phase 3 item 1's plan bullet).
- **Phase 2.1 continued — `:projects` picker source.** New
  `src/projects.rs`: a small, *global* persisted list (`~/.config/
  vaayu/recent_projects.json`, unlike `session.rs`'s own per-project
  `.vaayu/session.json` -- there's nothing project-specific to record
  here, just "where have I worked recently"), most-recent-first,
  capped at 20 entries. Recorded once, in `main()` right after
  `Editor::new`, deliberately *not* inside `Editor::new` itself -- the
  constructor runs for every one of the hundreds of `editor("")` test
  fixtures across the suite, and recording there would litter the real
  user's file with throwaway test temp directories on every `cargo
  test` run. `Editor::switch_project` (used by both `:projects`'
  `_vaayu_switch_project` action tag and directly testable on its own)
  updates `project_root` and drops any open file tree outright rather
  than rebuilding it in place, since `FileTree` derives its whole state
  (root, expanded set, git status) from the root it was created with;
  the next `,ft` lazily creates a fresh one at the new root, the same
  `get_or_insert_with` path a first-ever tree open already uses.
  Deliberately does *not* re-record the switched-to path -- it's
  already in the list (that's how it got into the picker), and
  re-recording on every switch would touch the real global state file
  from a method that should otherwise be pure in-memory and safe to
  call from a test. The core persistence logic (`record_recent_project_
  at`/`load_recent_projects_from`) is parameterized by an explicit
  state-file path specifically so it can be unit-tested against a real
  temp file rather than the user's actual config directory, which
  `dirs::config_dir()` resolves from `$HOME`/`$XDG_CONFIG_HOME` --
  both awkward and unsafe to mutate from a test running in parallel
  with every other test in the same process. 4 pure unit tests
  (creates and lists; moves an existing entry to the front without
  duplicating; caps at the 20-entry limit, keeping the most recent;
  loading a missing file is an empty list, not an error) plus 2
  `regression.rs` integration tests exercising `switch_project`
  directly (updates root, drops an open file tree and its window) and
  the `_vaayu_switch_project` action tag through `open_result` with a
  hand-built entry (avoiding `show_recent_projects`/`load_recent_
  projects` in `regression.rs`, since those read the *real* global
  config path and would make a test's assertions depend on whatever
  happens to already be in this machine's actual recent-projects
  history) plus `tests/pty_projects_picker.py` at three terminal sizes,
  which *can* safely exercise the full real path end-to-end because
  every PTY test already launches with `XDG_CONFIG_HOME` pointed at an
  isolated sandbox -- pre-seeding that sandbox's `recent_projects.json`
  and confirming the list excludes the current project, Enter switches
  and shows a confirmation, and a subsequent `,ft` rebuilds the file
  tree at the new root. The `:commands` and file-tree PTY tests still
  pass unchanged (`:projects` added to `command::EX_COMMANDS`). Full
  suite (298 tests) and full existing PTY suite (64 files) pass
  unchanged. Latency against `6836f46` matched closely on the first run
  (`insert_char` 3.123ms vs 3.115ms, overall p50 0.587ms vs 0.649ms) --
  no regression, as expected since the only real cost this adds is a
  one-time file read/write at process startup, entirely outside the
  benchmark's per-keystroke measurement window.
- **Phase 2.1 finished — `:everything` unified source list.**
  Combines `:keymaps`, `:commands` and `:projects` into one Results
  list, grouped by category (`[keymap]`/`[command]`/`[project]` prefix,
  each group in its own existing sensible order) rather than one flat
  alphabetical sort across all three, which would just interleave
  unrelated things. The whole point of a "unified" list is searching
  across sources at once, and `f` (Phase 2.6's Results filtering,
  already shipped) already does exactly that with zero new code needed
  here. Each entry reuses the *exact* action tag its own single-source
  command already uses (`_vaayu_action_id`, `_vaayu_prefill_ex`,
  `_vaayu_switch_project`) -- `results.rs::open_result` needed no new
  dispatch logic at all, just building one combined list from the same
  three sources those commands already build separately. 1
  `regression.rs` test confirming the combined counts match
  `ACTIONS.len()`/`EX_COMMANDS.len()` exactly, one tagged entry from
  each of the three categories dispatches correctly, and every project
  entry (whatever the real recent-projects file happens to contain on
  this machine -- deliberately not asserting exact paths, the same
  environment-coupling reasoning already applied to testing `:projects`
  itself) excludes the current `project_root`, plus
  `tests/pty_everything_picker.py` at three terminal sizes demonstrating
  the filter synergy directly: opening `:everything`, then `f`+"grep"
  narrows from the full combined count down to just the matching
  `:grep` command, with the `N/total` header proving the total stayed
  the combined count rather than any one source's own smaller count.
  The `:commands` and `:projects` PTY tests still pass unchanged.
  Full suite (299 tests) and full existing PTY suite (65 files) pass
  unchanged. Latency against `6836f46` matched closely on the first run
  (`insert_char` 3.103ms vs 3.120ms, overall p50 0.582ms vs 0.668ms) --
  no regression; this feature is reached only from `:everything` and
  its own Results-list entries, never the hot typing path. **Phase 2
  item 1, and with it all of Phase 2, is now fully done except item 3
  (ranking instrumentation and a bounded top-k matcher for the file
  picker -- not started).**
- **Phase 4.1 continued — git hunk reset (`,gx`) and `:e!`/`:edit!`.**
  Built on two new primitives neither of which existed before this
  slice: `Buffer::reload()` re-reads the file from disk into a fresh
  `Rope`, resets the saved-snapshot/dirty cache/undo-redo stacks and
  `edit_seq` (so throttled background jobs like git-status refresh
  immediately instead of waiting out their throttle window), and
  clamps the cursor/`top_line` into the new (possibly shorter) content
  -- exposed directly to users as `:e!`/`:edit!` with no argument
  (matching Vim's own "discard and reload" meaning for the bang
  variant; with a path it behaves like the existing `:e`/`:edit`).
  `git_tools::apply_patch` gained a fourth `cached: bool` parameter so
  a caller can choose the working tree instead of always the index --
  its one production call site (`stage_result`) now passes `true` to
  preserve the exact behavior stage/unstage already had. Hunk finding
  itself was factored out of `preview_current_hunk` into a shared
  `hunk_at_cursor` helper (same dirty-buffer refusal, same "hunk
  containing the cursor, or nearest one starting before it" search)
  so `,gh` and the new `,gx` can't drift apart on what counts as "the
  hunk under the cursor." `reset_current_hunk_prompt` shows that hunk
  as a Results list whose title states the destructive action plainly
  ("Reset this hunk back to HEAD? Enter discards it") and tags every
  entry with a `_vaayu_git_hunk_reset` action carrying the patch/path/
  root; a new `results.rs::open_result()` branch dispatches it to
  `Editor::apply_hunk_reset`, which re-checks the dirty guard (the
  buffer could have been edited between showing the prompt and
  confirming it), applies the patch in reverse against the working
  tree only (`cached: false`), and reloads the matching open buffer (if
  any) so it can't silently disagree with the file underneath it.
  Cancelling with `q`/Esc is architecturally a no-op -- it just never
  calls `open_result`, matching how every other confirm-via-Results
  prompt in this codebase already works, so it needed no separate
  handler to "not apply" anything. Bound as `,gx` (confirmed free
  alongside the existing `gp`/`gh`/`gB`/`gw` git/search leader keys).
  2 new `regression.rs` tests (reset restores both the on-disk file and
  the already-open buffer to HEAD while leaving the file's *other*,
  untouched hunk alone; an unsaved buffer refuses with a clear message
  and shows no prompt) plus 3 covering `Buffer::reload` directly
  (discards in-memory changes and undo history; clamps the cursor when
  reloading into a shorter file, computing the expected clamp line from
  `rope.len_lines()` rather than assuming 0 -- ropey counts a trailing
  newline as an extra final empty line) and 1 for `:e!` as an ex
  command, plus `tests/pty_hunk_reset.py` at three terminal sizes
  against a real git repository with two separate hunks, confirming
  through the PTY that only the hunk under the cursor is discarded and
  the other survives on disk. Full suite (306 tests, both `vaayu` and
  `vy` binaries) and the full existing PTY suite (67 files) pass
  unchanged; two runs against `6836f46` both showed no regression
  (overall p50 0.617-0.677ms head vs 0.553-0.617ms baseline, p90/p99
  overlapping within normal machine noise across both runs -- this
  feature is reached only from `,gx`'s own Results-list confirmation,
  never the hot typing path). **Phase 4 item 1 is now done except
  selected-range actions.**
- **M1.B, M2–M9 (except the Phase 2.1/2.2/2.4/2.5/2.6/2.7/2.8, Phase
  3.1, Phase 3.5, Phase 3.6 and Phase 4.1/4.6 slices above):** not
  started (M1.A, M1.C and M1.D are partially done -- see their entries
  above). See the phase sections above for scope; nothing in this log
  should be read as partially done unless stated here.
