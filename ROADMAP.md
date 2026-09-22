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
- [ ] 0.3 Command-line completion + wildmenu + reverse history search — **M**
- [ ] 1.7 Registers viewer (`:reg`), marks viewer (`:marks`), messages log (`:messages`) — **S**
- [ ] 1.2 incsearch (highlight/jump while typing `/`) — **S**
- [ ] 1.1 inccommand (live `:s///` preview) — **M**

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

---

## Progress Log

(Newest first. Each entry: what shipped, tests added, verification.)

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
