# Vaayu Feature-Completeness Roadmap

Goal: a self-sufficient, single-binary modal editor covering a modern
Neovim-with-plugins + Helix daily workflow — no Lua plugin host required.
"Complete" means you never *need* another editor for editing, navigation,
code intelligence, git, or repository work.

**Non-goals:** a general Lua/plugin scripting host; a byte-for-byte Vim
emulator; a remote-plugin ecosystem. Everything below ships as compiled
features configured declaratively (TOML/JSON).

Out of scope for this program (by explicit request): **DAP / debugging (5.2)**.

---

## Engineering protocol (applied to EVERY feature, no exceptions)

1. **Test plan first.** Enumerate user-facing cases, edge cases, and failure
   modes in this file under the feature, *before* any implementation.
2. **UI-test plan.** Decide how it will be driven through a real PTY (keys,
   geometries, what to assert on screen / on disk).
3. **Tests before code.** Write Rust unit tests and/or `tests/pty_*.py`
   end-to-end tests that fail for the right reason.
4. **Implement** the feature.
5. **Run** the Rust suite + the new/affected PTY tests; **fix** every failure.
6. **Self-verify** by driving it in a real PTY and eyeballing the frames.
7. **Gate:** `cargo test` green (both binary targets) + `cargo clippy
   --all-targets` clean + no first-response latency regression.
8. Only then check the item's box below and log it in the Progress Log.

Legend: effort **S** ≤2d · **M** ~1wk · **L** 2–4wk · **XL** multi-month.
Status: [ ] todo · [~] in progress · [x] done+tested.

---

## Wave A — Foundations & quick wins

- [x] 0.1 Event/autocommand bus (BufWritePre/Post, BufEnter, InsertLeave fired; FocusGained/CursorHold/FileType defined, not yet dispatched) — **M**
- [~] 1.6 On-save hooks: trim-trailing-whitespace + insert-final-newline + `[[autocmd]]` **done**; **format-on-save pending** (needs synchronous LSP-format-with-timeout) — **S**
- [ ] 0.2 Config-driven keymap remapping (`[keymaps]`) + conflict detection — **M**
- [x] 0.3 Command-line completion + wildmenu (command names + file-path args; history browse already present) — **M**
- [x] 1.7 Registers viewer (`:reg`), marks viewer (`:marks`), messages log (`:messages`) — **S**
- [x] 1.2 incsearch (highlight/jump while typing `/`) — **S**
- [~] 1.1 inccommand — live `:s///` **match highlight** done; replacement-text preview = 1.1b (follow-up) — **M**

## Wave B — Code intelligence & tree-sitter

- [ ] 0.5 Tree-sitter query infrastructure (highlights/locals/textobjects/folds/injections) — **L**
- [ ] 1.3 Tree-sitter textobjects (`af/if`, `ac/ic`, `aa/ia`, nav) — **M**
- [ ] 1.4 Incremental selection (+ LSP selectionRange fallback) — **S–M**
- [ ] 2.8 Sticky scroll / context header — **M**
- [ ] 2.1 Semantic-token highlighting — **M**
- [ ] 2.3 Pull diagnostics (`textDocument/diagnostic`) + workspace diagnostics — **S–M**
- [ ] 2.2 Call hierarchy + type hierarchy views — **M**
- [ ] 2.4 Linked editing range — **S**
- [ ] 2.5 Document color + swatches/picker — **S**
- [ ] 2.9 Rainbow delimiters + injection highlighting — **M**
- [ ] 2.10 Tree-sitter indentation — **M**

## Wave C — Visual identity & UX polish

- [ ] 0.4 Theme/colorscheme engine (true-color, undercurl, transparent, runtime reload) — **L**
- [ ] Built-in colorschemes + `:colorscheme` picker — **M**
- [ ] Live inline spell underline + `]s/[s` — **M**
- [ ] illuminate (references under cursor) — **S–M**
- [ ] TODO/FIX/HACK highlighting + `:todo` index — **S**
- [ ] conceal support — **M**
- [ ] Configurable global statusline + statuscolumn + winbar/breadcrumbs (2.11) — **M**
- [ ] Notifications center + `:messages` toasts — **M**
- [ ] Full zen/focus layout — **S**
- [ ] (Optional/stretch) minimap, animations, Kitty inline images — **L**

## Wave D — Folding & diff

- [ ] 0.6 Folding engine (manual/indent/treesitter/LSP foldingRange; foldcolumn) — **L**
- [ ] 4.1 Diff mode / vimdiff + side-by-side git diff w/ region folding + 3-way merge — **L**

## Wave E — Repository & workflow

- [ ] 4.5 Shell/terminal UX (split/tab/float, toggle+reattach, terminal-mode nav, send-to-terminal/REPL) — **M**
- [ ] 4.3 Task/test runner → quickfix (Cargo/Go/npm aware; test-under-cursor; watch) — **L**
- [ ] 4.2 Git deepening (commit browser, file history, cherry-pick/revert) — **M**
- [ ] 4.7 `.tours/` code tours + prompt bank — **M**
- [ ] 4.6 Remote editing (`ssh://` open/save, remote grep/pickers) — **L**
- [ ] 4.8 Inline-suggestion (Copilot-style) provider + ghost text — **L**
- [ ] 4.4 Native GitHub workspace (issues, PRs, review threads, CI, notifications) — **XL**

## Wave F — Big bets & completeness

- [ ] 5.1 Multiple cursors — **XL**
- [ ] 5.3 Cross-session (shada) persistence: marks, registers, jumplist, history, per-file cursor — **M**
- [ ] 5.4 Location list distinct from quickfix (`:lopen`/`:lne`) — **S–M**
- [ ] 2.6 LSP refactors with diff preview (extract/inline) — **L**
- [ ] 2.7 Project-wide reviewed replace (grug-far) — **M–L**
- [ ] 1.8 Snippet regex transforms + choice dropdown — **M**
- [ ] 1.9 Encoding / fileformat handling (latin1/UTF-16/BOM, CRLF↔LF) — **M**
- [ ] 1.5 Move lines/blocks + swap argument — **S**
- [ ] 1.10 Split-border drag-resize + `Ctrl-W </>/+/-/=` — **S**
- [ ] 1.11 `:earlier`/`:later` + undo-tree viewer — **M**
- [ ] 6.x EditorConfig completeness, config surface (listchars/fillchars/cursorline/colorcolumn/…), large-file mode, session completeness, `:checkhealth` — **M**

---

## Feature test plans (written before implementation)

### 0.1 Event/autocommand bus + 1.6a on-save trim / final-newline
User-facing cases:
- `trim_trailing_whitespace = true`: `:w` removes trailing spaces/tabs from every line.
- `insert_final_newline = true`: `:w` ensures a non-empty file ends with exactly one `\n`.
- `[[autocmd]]` `{ event, pattern, command }`: the Ex `command` runs when `event` fires on a buffer whose name matches the glob `pattern`.
- Events fired: `BufWritePre`/`BufWritePost` (on save), `InsertLeave` (leaving Insert), `BufEnter` (opening a file).

Edge cases:
- Trim leaves interior/leading whitespace intact; whitespace-only lines become empty; does not touch a `\r` (CRLF preserved).
- Final-newline: a file already ending in `\n` is unchanged (no double newline); an **empty** buffer stays empty (no newline added).
- On-save edits are a single undo step; cursor is re-clamped if it sat on trimmed text.
- A bad/failing autocmd command must not crash or abort the save.
- Glob: `*.rs` matches `foo.rs` not `foo.txt`; `*` matches everything; matches basename or full path.
- Default (both bools false, no autocmds): save behaviour byte-identical to before.

UI-test plan (PTY, 3 geometries): open a file with a trailing-space line and no final newline; config enables trim+final-newline; append text, `:w`; assert the on-disk bytes are trimmed and end in exactly one `\n`. A `[[autocmd]]` BufWritePre with a `:%s` command is asserted to have run.

Unit tests: trim; final-newline (incl. empty-buffer no-op and already-terminated no-op); autocmd fires on matching event+pattern and not on mismatched; default no-op.

### 1.7 Registers / marks / messages viewers
User-facing cases:
- `:reg`/`:registers` → a results list of set registers (`"` unnamed, named a–z,
  numbered, `+`/`*`), each showing a one-line content preview.
- `:marks` → a results list of set marks (`'a  line:col  path`); Enter jumps to
  the mark's location.
- `:messages` → a results list of the recent message history.

Edge cases:
- Empty register/mark/message set → a clear "No …" message, not an empty list.
- Register content with newlines is previewed on one line (escaped), long content clipped.
- Uppercase-append register semantics already fold into lowercase; list shows the folded set.
- Message log dedups consecutive identical messages and is bounded (≤500).
- `:marks` Enter navigates via buffer_id/path/line/col like `:jumps`.

UI-test plan (PTY, 3 geometries): `yy` then `:reg` shows the yanked text; `ma`,
move, `:marks` shows `'a` at its line and Enter jumps back; trigger a message
(`u` → "undo") then `:messages` shows it.

Unit tests: `Registers::list` returns the set registers sorted; `set_message`
appends to the log, dedups consecutive, and bounds length; `:marks` builds an
entry carrying the right line/col.

### 1.2 incsearch
User-facing cases:
- Typing `/pat` previews the first match (from the pre-search cursor) by moving
  the cursor there and highlighting matches live as you type; `<CR>` keeps it.
- `?pat` previews backward.
- `Esc` restores the original cursor **and** scroll position.

Edge cases:
- Empty query → cursor at origin, no preview highlight.
- Invalid regex mid-typing (`\(`) → no crash, cursor at origin, no highlight;
  completing to a valid pattern resumes the preview.
- No match → cursor stays at origin.
- Backspace re-widens the query and updates the preview.
- `<CR>` lands on exactly the previewed match (search runs from the origin, not
  from the previewed position).
- Post-submit hlsearch still works; `:noh` clears.

UI-test plan (PTY, 3 geometries): multi-line file; `/gamma` moves the cursor to
the match line while still in COMMAND mode; `Esc` returns to origin; `/gamma<CR>`
lands on it; `?` previews backward.

Unit tests: `update_incsearch` moves the cursor to the first match from origin;
Esc restores cursor+scroll; invalid regex is a no-op (no crash); empty restores.

### 0.3 command-line completion + wildmenu
User-facing cases:
- In `:` (Ex) mode, Tab completes the command name from the command list; repeated
  Tab cycles forward, Shift-Tab (BackTab) cycles back; a wildmenu row shows the
  candidates with the current one marked.
- For path-taking commands (`:e`/`:edit`/`:w`/`:write`/`:tabnew`/`:split`/`:vsplit`),
  Tab after a space completes the file-path argument (relative to cwd, or absolute).

Edge cases:
- No candidates → no change, no crash.
- One candidate → completes to it directly.
- Any non-Tab key resets the completion cycle.
- Directory candidates get a trailing `/`; results sorted.
- Absolute vs relative path tokens both work.
- Completion is Ex-only (not `/`?` search).

UI-test plan (PTY, 3 geometries): `:regi`+Tab → `:registers`; with `alpha.txt`
present, `:e al`+Tab → `:e alpha.txt` and a wildmenu row lists the candidate.

Unit tests: name completion for a unique prefix; Tab cycles + BackTab reverses for a
multi-candidate prefix; absolute-path arg completion (no cwd mutation).

### 1.1 inccommand (live `:s///` preview)
User-facing cases:
- While typing `:[range]s/pat/…`, matches of `pat` are highlighted live in the
  buffer (like incsearch); Esc clears it; Enter applies the substitution and
  clears the preview.

Edge cases:
- Range forms `:s/`, `:%s/`, `:1,3s/` all preview; alternate delimiter `:s#pat#`.
- Empty pattern (`:%s//`) → no highlight, no crash.
- Non-substitute Ex command (`:set …`) → no highlight.
- Invalid regex mid-typing → no crash, no highlight.
- (Replacement-text preview — showing the substituted result inline — is a
  follow-up 1.1b; this slice previews the affected matches.)

UI-test plan (PTY): buffer with `foo`; typing `:%s/foo/` highlights the `foo`
cells (cell background set) on the content row; Esc clears; `:%s/foo/BAR/g<CR>`
applies.

Unit tests: pattern extracted for `s/`, `1,3s/`, `s#…#`; incsearch set while
typing and cleared on Esc/Enter; non-substitute and empty-pattern are no-ops;
substitute still applies on submit.

---

## Progress Log

(Newest first. Each entry: what shipped, tests added, verification.)

### 2026-09-22 — 1.1 inccommand (live `:s///` match highlight)
- **Shipped:** while typing an Ex `[range]s/pat/…`, the pattern's matches are
  highlighted live (reuses the incsearch highlight); `on_cmdline_changed` dispatches
  incsearch (`/`?`) vs inccommand (Ex); `substitute_pattern` extracts the pattern
  across ranges/delimiters; cleared on Esc and after submit. Replacement-text
  preview deferred to 1.1b.
- **Tests:** 3 Rust unit tests (pattern for `s/`,`%s/`,`1,3s/`,`s#…#`; non-substitute /
  empty no-op; cleared-on-submit + substitute applies) + `tests/pty_inccommand.py`
  (3 geometries; verifies live cell-background highlight, Esc clears, submit applies).
- **Verified:** 429 Rust tests pass; clippy clean; PTY green on 40×12 / 100×24 / 180×50.

### 2026-09-22 — 0.3 command-line completion + wildmenu
- **Shipped:** Ex-mode Tab/BackTab completion. `cmdline_complete` computes candidates
  for the current token — command names from `EX_COMMANDS`, or file-path arguments
  (relative to cwd or absolute) for path-taking commands — applies the first and
  cycles on repeat; any non-Tab key ends the cycle. A wildmenu row shows the
  candidates with the selected one bracketed. New `Editor.cmdline_completions` +
  `cmdline_completion_index`.
- **Tests:** 3 Rust unit tests (unique-name completion; Tab cycle + BackTab reverse +
  reset; absolute file-path completion) + `tests/pty_cmdline_complete.py` (3 geometries:
  `:regi`→`:registers` with wildmenu; `:e al`→`:e alpha.txt`).
- **Verified:** 426 Rust tests pass; clippy clean; PTY green on 60×14 / 100×24 / 180×50.

### 2026-09-22 — 1.2 incsearch
- **Shipped:** live `/`/`?` preview. New `Editor.incsearch` (pattern to highlight)
  and `Editor.search_origin` (cursor+scroll to restore / search from).
  `enter_command` records the origin for search kinds; `update_incsearch` moves the
  cursor to the first match from the origin and highlights as you type;
  `cancel_incsearch` (Esc / emptied query) restores; `<CR>` runs from the origin so
  it lands on exactly the previewed match. Invalid mid-typed regex is a no-op.
  Renderer prefers `incsearch` over `last_search` for match highlighting.
- **Tests:** 4 Rust unit tests (preview+Esc restore; Enter lands on preview;
  invalid-regex no-op; empty-query restore) + `tests/pty_incsearch.py` (3 geometries:
  a neighbour marker only visible when the view scrolls proves a real preview vs the
  prompt echo; Esc restores; submit lands).
- **Verified:** 423 Rust tests pass; clippy clean; PTY green on 40×12 / 100×24 / 180×50.

### 2026-09-22 — 1.7 registers / marks / messages viewers
- **Shipped:** `:reg`/`:registers` (via `Registers::list`), `:marks` (Enter jumps,
  like `:jumps`), `:messages` backed by a new bounded, consecutive-deduped
  `Editor.messages` history that `set_message` appends to. All added to the
  `:commands` list.
- **Tests:** 3 Rust unit tests (`Registers::list` sorted; message-log dedup+bound;
  `:marks` entry carries the location) + `tests/pty_viewers.py` (3 geometries:
  `yy`→`:reg` shows the yank; `ma`→`:marks`→Enter jumps; `u`→`:messages` shows it).
- **Verified:** 419 Rust tests pass; clippy clean; PTY green on 60×14 / 100×24 / 180×50.

### 2026-09-22 — 0.1 event bus + 1.6a on-save hooks
- **Shipped:** `src/events.rs` autocommand bus (`Event` enum + `Editor::fire_event`,
  re-entrancy-bounded); built-in `BufWritePre` handlers `trim_trailing_whitespace`
  and `insert_final_newline` (config-gated, one undo step, cursor re-clamped);
  declarative `[[autocmd]]` (event + glob `pattern` + Ex `command`). Events fired:
  `BufWritePre`/`BufWritePost` (save), `InsertLeave`, `BufEnter`. Config gained
  `trim_trailing_whitespace`, `insert_final_newline`, `[[autocmd]]`; documented in
  `config.example.toml`.
- **Tests:** 4 Rust unit tests (trim+final-newline; empty/already-terminated no-ops;
  defaults-off byte-identical; autocmd runs only on matching event+pattern) +
  `tests/pty_on_save_hooks.py` (3 geometries: edit→`:w` trims+terminates on disk;
  `*.rs` BufWritePre `s/TODO/DONE/` autocmd applied before write).
- **Verified:** 416 Rust tests pass (both targets); clippy clean; PTY test green on
  40×12 / 100×24 / 180×50.
- **Follow-up:** 1.6b format-on-save once a synchronous LSP-format-with-timeout helper
  exists; dispatch FocusGained (needs crossterm focus events) / CursorHold / FileType.
