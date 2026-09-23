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

- [x] 0.1 Event/autocommand bus (BufWritePre/Post, BufEnter, InsertLeave, FocusGained, CursorHold fired; FileType defined) — **M**
- [x] 1.6 On-save hooks: trim-trailing-whitespace + insert-final-newline + `[[autocmd]]` **and** format-on-save (`:set format_on_save`) — `:w`/`:wq`/`,w` run LSP formatting via a bounded synchronous pump (Neovim `format({async=false})` style) before writing, falling back to a plain save on timeout — **S**
- [x] 0.2 Config-driven keymap remapping (`[[keymap]]`): single-key + leader remaps, keys/Ex rhs, noremap; multi-key non-leader lhs = follow-up — **M**
- [x] 0.3 Command-line completion + wildmenu (command names + file-path args; history browse already present) — **M**
- [x] 1.7 Registers viewer (`:reg`), marks viewer (`:marks`), messages log (`:messages`) — **S**
- [x] 1.2 incsearch (highlight/jump while typing `/`) — **S**
- [x] 1.1 inccommand — live `:s///` match highlight **and** replacement-text preview (1.1b): affected lines show a tinted overlay of what they'd become while you type, reverted on Esc, committed on Enter — **M**

## Wave B — Code intelligence & tree-sitter

- [ ] 0.5 Tree-sitter query infrastructure (highlights/locals/textobjects/folds/injections) — **L**
- [x] 1.3 Tree-sitter textobjects: `af/if` function + `ac/ic` class + `aa/ia` argument objects (nesting/quote-aware comma split) + `]f`/`[f` function navigation (count + jumplist) — **M**
- [x] 1.4 Incremental selection (tree-sitter node expand/shrink, `,=`/`,-`); LSP selectionRange fallback = follow-up — **S–M**
- [~] 2.8 Sticky scroll / context header: `:set stickyscroll` pins the enclosing function/class/impl/trait/mod declaration lines (tree-sitter) at the top of the pane once they scroll off; up to 3, default off. LSP fallback = follow-up — **M**
- [~] 2.1 Semantic-token highlighting: `:set semantictokens` requests `textDocument/semanticTokens/full`, decodes the delta stream via the server legend, and overlays token colors (keyword/type/function/string/comment/number) on top of tree-sitter. Full-buffer only; delta/range updates + modifiers = follow-up — **M**
- [~] 2.3 Pull diagnostics: `textDocument/diagnostic` requested on open/change for servers advertising `diagnosticProvider`, responses routed as diagnostics (bypassing the stale-response guard, so they merge like push). Workspace diagnostics = remaining — **S–M**
- [x] 2.2 Call + type hierarchy: `:callers`/`:callees` (incoming/outgoing calls) and `:supertypes`/`:subtypes`, each a two-step LSP chain (prepare → direction request) listing jumpable Results locations — **M**
- [~] 2.4 Linked editing range: `:linkededit <name>` requests `linkedEditingRange` and renames all linked ranges at once (e.g. an open/close tag pair). Live type-to-mirror = follow-up — **S**
- [~] 2.5 Document color: `,lC` (`lsp.document_color`) requests `textDocument/documentColor` and paints each color literal in its own RGB; clears on edit/Esc. Swatch glyphs + a color picker = follow-up — **S**
- [~] 2.9 Rainbow delimiters (`:set rainbow`, `()[]{}` colored by nesting depth, matching pairs share a color, cached per edit; **brackets inside strings/comments are skipped** via tree-sitter spans so they don't miscolor or skew depth); injection highlighting = remaining — **M**
- [~] 2.10 Auto-indentation: **bracket-aware smartindent done** — Enter/`o`/`O` copy the source line's indent and add one level after an opening `{`/`(`/`[` (config `smartindent`, default on); also removed a dead per-keystroke whole-buffer alloc in the Enter path. **Full tree-sitter indent queries = remaining** — **M**

## Wave C — Visual identity & UX polish

- [~] 0.4 Theme/colorscheme engine: a `Theme` (syntax palette) with built-in schemes + runtime `:colorscheme` swap (incl. true-color RGB schemes) done; UI-color theming, undercurl, transparent bg = remaining — **L**
- [x] Built-in colorschemes (`default`/`mono`/`warm`/`cool`) + `:colorscheme [name]` runtime switch (lists when bare) — **M**
- [x] Live inline spell underline (`:set spell`, magenta underline, cached per edit) + `]s`/`[s` navigation — **M**
- [x] illuminate (references under cursor): auto `documentHighlight` on CursorHold (config `illuminate`, `updatetime_ms`), clears on move, silent without a capable server; also wires the previously-defined `CursorHold` event to actually fire — **S–M**
- [x] `:todo` index (TODO/FIXME/HACK/XXX via grep) **and** inline TODO highlighting (`:set todohighlight`: TODO/NOTE→yellow, FIXME/BUG/XXX→red, HACK/WARNING→magenta, whole-word, comment-only via tree-sitter) — **S**
- [ ] conceal support — **M**
- [~] 2.11 Configurable statusline: `statusline` config format string (`%f`/`%F`/`%l`/`%c`/`%L`/`%m`/`%y`/`%p`/`%M`/`%%`), ruler stays on the right. **Winbar done** (`:set winbar`: per-pane top row with the relative path + tree-sitter enclosing-symbol breadcrumb, content shifts down, mouse-mapping aware). Global statusline + statuscolumn + multi-level breadcrumb = remaining — **M**
- [x] Notifications: `:messages` history (pre-existing) + transient top-right toasts (`:set notifications`, mirror recent messages, auto-fade after ~4s) — **M**
- [x] Zen/focus layout: `:zen` toggles hiding the line-number gutter + per-pane status line (reclaiming that row for content); scrolling/splits unaffected — **S**
- [~] (Optional/stretch) minimap (`:set minimap`): a right-hand strip showing a compressed per-line silhouette (indent/length shape) with the current viewport region tinted, config-gated (default off), reclaimed cleanly when toggled off. Animations, Kitty inline images = remaining — **L**

## Wave D — Folding & diff

- [x] 0.6 Folding engine — `:{range}fold` / visual `:fold` (manual); `:foldindent` (indentation), `:foldsyntax` (tree-sitter), `:foldlsp` (LSP `textDocument/foldingRange`) auto-fold; `za`/`zo`/`zc`/`zd`/`zR`/`zM` toggle/open/close/delete/open-all/close-all; `:set foldcolumn` shows `+`/`-` markers. Closed folds hide their inner lines and render a tinted foldtext row; `j`/`k` are fold-aware; a cursor left inside a fold snaps to its start; **fold ranges track inserts/deletes**. — **L**
- [~] 4.1 Diff mode: `:diffthis` on two buffers line-diffs them (`similar`) and highlights each side's differing lines; `:diffoff` clears; recomputed live on edit. Side-by-side sync-scroll, unchanged-region folding, per-side colors, 3-way merge = remaining — **L**

## Wave E — Repository & workflow

- [x] 4.5 Shell/terminal UX: embedded terminal split + Terminal-mode nav + toggle/reattach long-lived sessions (pre-existing) + **send-to-terminal/REPL (`:termsend`, current line or Visual/range → the focused-or-latest terminal)** — **M**
- [~] 4.3 Task/test runner → quickfix: `:make [cmd]`/`:task [cmd]` runs a command (async) and parses `file:line:col: message` (incl. Rust `-->`) output into the quickfix list; defaults from Cargo.toml/go.mod/package.json/Makefile. Test-under-cursor + watch mode = remaining — **L**
- [x] 4.2 Git deepening: commit browser (`:gitlog`), file history (`:gitfilehistory`), cherry-pick (`:gitcherrypick`), and revert (`:gitrevert`) — on top of existing status/stage/commit/blame/stash/branch — **M**
- [~] 4.7 `.tours/` code tours: `:tours` lists CodeTour-format `.tours/*.tour` files, `:tour [name]` starts one, `:tournext`/`:tourprev` step through (jump + description). Prompt bank = remaining — **M**
- [ ] 4.6 Remote editing (`ssh://` open/save, remote grep/pickers) — **L**
- [ ] 4.8 Inline-suggestion (Copilot-style) provider + ghost text — **L**
- [ ] 4.4 Native GitHub workspace (issues, PRs, review threads, CI, notifications) — **XL**

## Wave F — Big bets & completeness

- [ ] 5.1 Multiple cursors — **XL**
- [x] 5.3 Cross-session (shada) persistence: per-file cursor position, named registers, command/search history, named marks, and jumplist — all in `.vaayu/shada.json`, loaded at startup, saved on quit — **M**
- [~] 5.4 Location list distinct from quickfix: `:ldiagnostics` populates a separate list from the current buffer's diagnostics; `:lopen`/`:lnext`/`:lprev` open & step it independently of quickfix. Per-window loclists + `:lgrep` = follow-up — **S–M**
- [~] 2.6 LSP refactors with diff preview: `:set refactor_preview` makes `:rename` show a per-occurrence diff (line → replacement) across all affected files and defer the WorkspaceEdit; `:renameapply` commits it (version-guarded), `:renamecancel` drops it. Extending the preview to code-action refactors (extract/inline) = remaining — **L**
- [~] 2.7 Project-wide replace: `:cfar/pat/repl/[flags]` (also `:cfar /pat/repl/`) rewrites every file in the current results/quickfix list (e.g. from a prior `:grep`), reusing `run_substitute` so regex/flags/capture-group semantics match `:s`; saves each changed file and refocuses the original buffer. Live inline preview / per-hunk review UI = remaining — **M–L**
- [~] 1.8 Snippet: choice dropdown (`${n|a,b,c|}` with `,`-cycle) already present; **variable regex transforms (`${VAR/regex/fmt/flags}`, capture refs + `g`/`i` flags, applied at expand) done**. Live numbered-stop transforms = remaining — **M**
- [x] 1.9 Encoding / fileformat: CRLF/CR/LF + UTF-8 BOM (`:set ff=`), and non-UTF-8 encodings — latin1 + UTF-16 LE/BE detected on load, decoded to the internal UTF-8 rope, and re-encoded on save; `[latin1]`/`[utf-16le]` ruler tag — **M**
- [x] 1.5 Move lines (`]e`/`[e`, with count + undo); visual-block move + swap-argument = follow-up — **S**
- [~] 1.10 `Ctrl-W </>/+/-/=` split resize done (ratio-based, session-persisted); mouse drag-resize = follow-up — **S**
- [~] 1.11 `:earlier N`/`:later N` (count-based undo/redo) done; undo-tree viewer = follow-up — **M**
- [x] 1.12 `gv` reselect last visual selection (charwise/linewise/blockwise, survives operators, clamps to shrunken buffer) — **S**
- [~] 1.13 `gq` reflow operator (`gqq`, `gq{motion}` e.g. `gq}`/`gqG`; paragraph-aware, indent-preserving, `textwidth` config) done; **`ip`/`ap` paragraph text objects done** (linewise); **visual `gq` done** (reflows the selection immediately); comment-leader-aware reflow + `gw` + dot-repeat = follow-up — **S**
- [~] 6.x completeness: `:checkhealth` **done**; config surface: **`cursorline`, `colorcolumn`, `list`, + runtime `:set` for `relativenumber`/`ignorecase`/`smartcase`/`smartindent`/`expandtab`/`autopairs` and `tabstop`/`shiftwidth`/`scrolloff`/`textwidth`/`updatetime`=N done** (short forms too); **large-file mode done** (`large_file_kb`, default 5 MiB; `:set largefilekb=N`) — files over the cutoff skip tree-sitter/spell/TODO/rainbow scans; **configurable `listchars` done** (`listchars="tab:xy,trail:z"`); **EditorConfig `end_of_line` done** (lf/crlf/cr overrides the detected ending on load, applied on save); fillchars, more EditorConfig keys (charset, max_line_length), session completeness = remaining — **M**

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

### 0.2 keymap remapping
User-facing cases:
- `[[keymap]]` `{ mode, lhs, rhs }`: single-key remaps (any of n/i/v) and
  leader-sequence remaps (`<leader>x`). rhs is either keys to replay, or an Ex
  command when it starts with `:` (a trailing `<CR>` is stripped).
- Key notation: `<leader>`, `<C-x>`, `<CR>`/`<Enter>`, `<Esc>`, `<Tab>`, `<BS>`,
  `<Space>`, arrows, `<lt>`; plain chars are literal.

Edge cases:
- Mode isolation: a normal remap doesn't fire in insert (and vice-versa).
- `noremap`: rhs is not itself remapped (a→b, b→c; pressing `a` yields b, not c).
- No remap while an operator/count/register/awaiting is pending (clean state only).
- Leader remaps take precedence over built-in leader actions; a user leader
  keymap that's a prefix keeps waiting for more keys.
- Invalid/unknown notation is ignored (no crash).
- Multi-key non-leader lhs (e.g. `jj`) is out of scope for this slice
  (jk-escape already covers the common insert case).

UI-test plan (PTY): config `Y`→`y$`, `<leader>w`→`:w<CR>`, and an insert remap;
verify `Y`+`p` pastes to-EOL text, `,w` saves, and the insert remap inserts.

Unit tests: keys-remap yanks to EOL; Ex-remap runs; mode isolation; noremap;
leader remap runs its command; invalid notation ignored.

### 1.3 tree-sitter textobjects (function/class)
Cases: `af`/`if` (function around/inner), `ac`/`ic` (class around/inner), working
with operators (`daf`,`cif`) and Visual (`vaf`). around = the whole node; inner =
the body block's content (statements between braces). Argument objects (`aa`/`ia`)
= follow-up 1.3b. Edge cases: cursor not inside any function/class → no-op; no
syntax tree → no-op; nested functions pick the innermost; empty body inner is
inside the braces. UI test (PTY): `dif` clears a function body; `daf` deletes the
whole function; `vac` selects a struct. Unit: af/if/ac/ic ranges on a Rust file.

---

## Progress Log

(Newest first. Each entry: what shipped, tests added, verification.)

### 2026-09-23 — format-on-save (completes 1.6)
- **Shipped:** `:set format_on_save` (`fos`, default off). `save_current_formatted` (used by `:w`, `:wq`/`:x`, and the `,w` action) requests LSP formatting and drives a bounded synchronous pump — poll LSP events until the `format` result arm clears `format_pending`, or a 2s deadline — then writes, so the saved file reflects the formatting. Only engages when a `documentFormattingProvider` server is attached; otherwise (and on timeout, or a version-mismatch reject) it falls back to a plain save, so a slow/unresponsive server never blocks saving. This is the previously-noted "needs synchronous LSP-format-with-timeout" piece, done Neovim-`format({async=false})`-style.
- **Tests:** tests/pty_format_on_save.py (2 geometries, real mock-LSP: `:w` formats the first token and the on-disk file becomes `FMT\n`). Re-ran pty_code_actions + pty_rename_preview (shared LSP/apply-edit path): no regression.
- **Verified:** 521 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — EditorConfig end_of_line (6.x completeness)
- **Shipped:** the `.editorconfig` reader now honors `end_of_line` (lf→Unix, crlf→Dos, cr→Mac). `indent::editorconfig_eol` walks parent dirs to `root = true` with the same glob/closest-file/last-matching-section rules as the indent reader, and `Buffer::apply_indent` applies it after load so the detected ending is overridden and `:w` normalizes to the configured style. Extends the previously indent-only EditorConfig support.
- **Tests:** 1 Rust unit (`[*]`=crlf resolves to Dos; a later `[*.lf]`=lf overrides for a matching file; no `.editorconfig` → None) + tests/pty_editorconfig_eol.py (2 geometries: a file loaded as LF under an `end_of_line=crlf` config is written back with CRLF — asserted on disk bytes).
- **Verified:** 529 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — configurable listchars (6.x completeness)
- **Shipped:** the `list` glyphs are now configurable via a Vim-style `listchars` string (`tab:xy,trail:z` — x=tab lead, y=tab fill, z=trailing-space marker), default `tab:>-,trail:·` (unchanged from before). `parse_listchars` turns it into `(lead, fill, trail)` (single-char tab value sets both, missing/unknown keys fall back), computed once per pane and used in the render substitution. eol/space markers + fillchars = follow-up.
- **Tests:** 1 Rust unit (`parse_listchars` cases incl. unicode, single-char tab, defaults) + tests/pty_listchars_custom.py (3 geometries: `tab:!~,trail:@` renders the custom glyphs, defaults absent from the content row). The existing pty_listchars.py (default glyphs + `:set list`/`nolist`) still passes.
- **Verified:** 528 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — large-file mode (6.x completeness)
- **Shipped:** buffers over `config.large_file_kb` (default 5120 KiB; `:set largefilekb=N`, 0 disables) enter large-file mode: `ensure_syntax` drops/skips the tree-sitter tree, and `update_spell_spans`, `update_todo_spans`, and the rainbow scan all bail out — so opening a multi-megabyte file stays responsive instead of stalling on a whole-buffer parse/scan. A single `buf_is_large()` helper gates them (render checks the pane's buffer size directly since it may not be the current one).
- **Tests:** 1 Rust unit (a ~6 KB buffer with a 1 KiB threshold: no syntax tree, no TODO spans; with the cutoff off, syntax parses) + tests/pty_large_file.py (2 geometries: a ~500 KiB file with a 50 KiB threshold renders, navigates with `G`/`gg`, and quits cleanly — no hang).
- **Verified:** 527 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — visual gq reflow (1.13 follow-up)
- **Shipped:** `gq` in Visual mode now reflows the selected lines to `textwidth` immediately (previously it set the operator pending and hung, since the shared `g`-prefix handler only called `begin_operator`). The Visual branch routes to `apply_to_selection(OperatorKind::Format, kind)` (made `pub(crate)`), reusing the same reflow the Normal-mode `gqq`/`gq{motion}` operator uses; Normal mode is unchanged.
- **Tests:** 1 Rust unit (`Vgq` wraps a long line to textwidth and returns to Normal) + tests/pty_visual_gq.py (3 geometries, end-to-end + on-disk assertion).
- **Verified:** 526 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — inline TODO highlighting
- **Shipped:** `:set todohighlight` recolors TODO/FIXME/HACK/XXX/NOTE/BUG/WARNING keywords **inside comments** (TODO/NOTE→yellow, FIXME/BUG/XXX→red, HACK/WARNING→magenta). `update_todo_spans` (per-frame, `(buffer, edit_seq)`-stamped like spell) scans the tree-sitter Comment spans for whole-word keyword matches and records `(line, start, end, color)`; a new `todo_ranges` field in `RowSignature` keeps the row cache correct, and the paint loop overrides those glyphs' fg. Keywords in code/strings are left alone. Default off.
- **Tests:** 1 Rust unit (TODO/FIXME in comments marked with the right color; a `TODO` identifier in code and a `TODONT` non-word-match ignored; toggling off clears) + tests/pty_todo_highlight.py (3 geometries, real color assertions: comment TODO yellow, FIXME red, code TODO not highlighted). Re-ran pty_rainbow + pty_spell (shared paint chain): no regression.
- **Verified:** 525 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — paragraph text objects (1.13 follow-up)
- **Shipped:** `ip`/`ap` paragraph text objects (`ObjectKind::Paragraph`). A paragraph is a maximal run of same-kind lines (all non-blank, or all blank); `ip` is that run, `ap` also takes the following opposite-kind run (blank lines after a text block) or the preceding one when none follows. Applied linewise — the normal-mode operator dispatch uses `Span::Linewise` for this object (so `dip`/`dap`/`cip` remove/replace whole lines) and visual `vip`/`vap` switch to linewise Visual. Works from anywhere in the paragraph, including on a blank line.
- **Tests:** 1 Rust unit (`dip` deletes the run, `dap` swallows the trailing blank, `dip` on a blank line deletes the blank run) + tests/pty_paragraph_object.py (3 geometries, end-to-end `dip` + on-disk assertion).
- **Verified:** 524 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — rainbow skips strings/comments (2.9)
- **Shipped:** `rainbow_brackets` now excludes brackets that fall inside tree-sitter `String`/`Comment` spans — they aren't delimiters, so they neither get a rainbow color nor affect nesting depth (a `(` in a string no longer shifts the colors of the real brackets after it). Collects the string/comment byte ranges once (current buffer only, where a live tree exists) and skips brackets whose byte offset lands in one; still cached per `(buffer, edit_seq)`.
- **Tests:** 1 Rust unit (code brackets on lines 0/3 colored; a `(` in a string and a `]` in a comment excluded) + re-ran pty_rainbow: no regression.
- **Verified:** 523 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — fold edit-tracking (completes 0.6)
- **Shipped:** fold line ranges now shift/grow through edits so they keep covering the same region after inserts and deletes. `Buffer::adjust_folds_for_edit(edit_line, delta)` is called from the insert/delete primitives (`insert_char`/`insert_str`/`insert_char_at`/`insert_str_at`/`delete_char_range`) with the newline delta at the edit line — folds below the edit shift, a fold straddling it grows/shrinks, and folds that collapse below two lines are dropped. Guarded by `folds.is_empty()` and only touches `self.folds` (never the rope), so the hot edit path is unaffected when there are no folds. This was the last 0.6 follow-up, so folding is now complete.
- **Tests:** 1 Rust unit (fold shifts down on a top insert, back up on the matching delete) + re-ran pty_folding + pty_foldindent + pty_inccommand (edit-path-sensitive): no regression.
- **Verified:** 522 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — LSP foldingRange folds (0.6, extends folding)
- **Shipped:** `:foldlsp` requests `textDocument/foldingRange` and installs the returned ranges as closed folds (a new `foldingRange` request kind + `foldingRangeProvider` capability gate + a `language_result` arm that maps `{startLine,endLine}` to `Fold`s on the response's buffer, clamped and deduped). Reuses the same fold model/render/motions. Now all four fold sources exist: manual, indent, tree-sitter, and LSP.
- **Tests:** tests/pty_foldlsp.py (2 geometries, real mock-LSP returning ranges 1..3 and 5..7: inner lines collapse to foldtext, fold-start/other lines stay, `zR` reopens). Extended mock_lsp with `foldingRangeProvider` + a `textDocument/foldingRange` handler. Re-ran pty_code_actions + pty_foldsyntax: no regression.
- **Verified:** 521 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — foldcolumn (0.6, extends folding)
- **Shipped:** `:set foldcolumn` (`fdc`, default off) adds a one-cell gutter marker — `+` where a closed fold starts, `-` where an open fold starts, blank otherwise. Threaded through the single `gutter()` width helper (so both `draw_pane` and the mouse coord→position mapper widen the gutter consistently), prepended to the per-row margin, and added to `RowSignature` as `fold_marker` so toggling a fold repaints its start row's marker.
- **Tests:** tests/pty_foldcolumn.py (3 geometries: no marker with no folds, `+` on a closed fold, `-` after `zo`, `+` again after `za`). Re-ran pty_folding + pty_mouse + pty_cursorline (gutter-width-sensitive): no regression.
- **Verified:** 521 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — tree-sitter auto-folds (0.6, extends folding)
- **Shipped:** `:foldsyntax` (`fold_by_syntax`) folds function/class/module extents from the tree-sitter tree. A new `Syntax::node_ranges(kinds)` returns the byte spans of matching nodes (preorder); the editor converts them to line ranges via a new `FOLD_KINDS` set (functions + impl/struct/enum/trait/mod/class/interface), keeps the multi-line ones, and installs them as closed folds — a structural overview built on the same fold model/render/motions. No-op with a message when there's no parsed tree.
- **Tests:** 1 Rust unit (two functions fold, a one-line const doesn't; bodies hidden, headers/const visible) + tests/pty_foldsyntax.py (3 geometries: both fn bodies collapse to foldtext, `zR` reopens).
- **Verified:** 521 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — indent auto-folds (0.6, extends folding)
- **Shipped:** `:foldindent` (`fold_by_indent`) computes indentation-based folds — each line heading a more-indented block becomes a closed fold spanning it (blank lines absorbed, trailing blanks trimmed), producing a nested overview that reuses the manual-fold model/render/motions from the previous slice. Opening an outer fold reveals the next level with inner folds still closed, matching Vim's `foldmethod=indent` feel.
- **Tests:** 1 Rust unit (nested indent folds; outer collapses all, opening it keeps the inner folded) + tests/pty_foldindent.py (3 geometries: block collapses to foldtext, top-level lines stay, `zR` reveals all). 
- **Verified:** 520 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — manual folding (0.6, partial)
- **Shipped:** a `Fold {start,end,closed}` model on `Buffer` plus manual-fold UX. Creation: `:{range}fold` / visual-selection `:fold` (`create_fold`). Management (normal-mode `z` prefix): `za` toggle, `zo` open, `zc` close, `zd` delete, `zR` open-all, `zM` close-all (also `:foldopen`/`:foldclose`). A closed fold hides all but its first row: `layout` skips `line_hidden` lines (with `folds_stamp()` added to `LayoutKey` so toggling invalidates the viewport cache), and `draw_pane` overlays the fold-start row with tinted foldtext (`first line ⋯ N lines`) as a post-loop overlay (no RowSignature change). Motions: `j`/`k` (`Motion::Down`/`Up`) use `visible_line_below`/`visible_line_above` to step over closed folds (a no-op when there are none), and `clamp_cursor_folds` (in `prepare_view`) snaps a cursor left inside a fold by any other motion to the fold's start. Auto-folds (indent/tree-sitter/LSP), foldcolumn, and edit-tracking of ranges = follow-ups.
- **Tests:** 2 Rust units (fold hides inner lines, `j`/`k` skip it, `za` reopens; open-all/close-all/delete) + tests/pty_folding.py (3 geometries: collapse to foldtext, inner lines hidden, following lines pulled up, `za` reopen/reclose). Re-ran pty_split_open + pty_cursorline + pty_winbar + pty_mouse + pty_incsearch: no regression.
- **Verified:** 519 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — winbar (2.11 follow-up)
- **Shipped:** `:set winbar` (`wbr`, config `winbar`, default off) draws a per-pane top row showing the buffer's project-relative path plus, when a tree-sitter tree exists for the current buffer, the enclosing function/class declaration as a `›` breadcrumb (via `context_starts` at the cursor byte, innermost). Implemented with a single `top_off` threaded through `draw_pane`: content rows, the sticky/inccommand overlays, the minimap strip, and the returned cursor position all shift down by it, and the mouse coord→position mapper subtracts it too (clicks on the winbar map to nothing). Suppressed in zen and when the pane is too short to keep a content row.
- **Tests:** tests/pty_winbar.py (3 geometries: winbar shows the path with a tint, content shifts under it, the breadcrumb names the enclosing fn once the cursor is in the body, `:set nowinbar` reclaims the row). Re-ran pty_split_open + pty_stickyscroll + pty_minimap + pty_cursorline + pty_mouse (all coordinate-sensitive): no regression.
- **Verified:** 517 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — LSP rename with diff preview (2.6, partial)
- **Shipped:** opt-in `:set refactor_preview` (`rfp`, config `refactor_preview`, default off). With it on, a `:rename` response no longer applies immediately — `preview_rename` parses the WorkspaceEdit (`changes`/`documentChanges`), reads each affected file (open buffer or disk, read-only), and builds a Results list showing every occurrence's line and what it becomes, stashing the raw edit + `RequestContext` in `pending_rename`. `:renameapply` commits it via the existing `apply_workspace_edit` (so the document-version guard still protects against edits made during the preview); `:renamecancel` discards it. Default behavior (immediate rename) is unchanged. Extending previews to extract/inline code-action refactors = follow-up.
- **Tests:** 2 Rust units (preview defers the edit until `:renameapply`, buffer untouched until then; cancel leaves it untouched and a later apply is a no-op) + tests/pty_rename_preview.py (2 geometries, real mock-LSP: preview shown, buffer unchanged, cancel path, then apply + save writes the renamed text to disk). Re-ran pty_code_actions (shared rename/workspace-edit path): no regression.
- **Verified:** 517 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — inccommand replacement preview (1.1b, completes 1.1)
- **Shipped:** typing a `:s`/`:%s`/`[range]s` substitute now shows a *live replacement* preview, not just a pattern highlight. `compute_sub_preview` (driven from `update_inccommand` on every cmdline change) parses the range + body (`parse_substitute_body`), builds the same regex `run_substitute` would (flags/ignorecase/smartcase), and maps each affected line to its replaced text (bounded to 4000 lines so `%s` on huge files stays cheap). `draw_pane` overlays those lines' content area with the preview text, tinted `INCCOMMAND_BG`, keeping the gutter — the proven post-loop overlay pattern (like sticky-scroll), so no RowSignature change and the buffer is never mutated. Cleared on Esc (`cancel_incsearch`), on submit, and whenever the line stops being a valid substitute.
- **Tests:** 2 new Rust units (live preview of matched lines with the buffer untouched, Esc clears, submit applies + clears; explicit-range preview) on top of the existing inccommand highlight units + tests/pty_inccommand.py (3 geometries: preview text + tint appear while typing, Esc reverts, submit commits, on-disk result). Re-ran pty_incsearch + pty_stickyscroll + pty_minimap: no regression.
- **Verified:** 515 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — project-wide replace (2.7, partial)
- **Shipped:** `:cfar/pat/repl/[flags]` (`run_far_replace`) — find & replace across every file in the current results/quickfix list. Validates the pattern up front, collects the unique files, and for each one opens the buffer, runs a whole-file `run_substitute` (so `:s` regex/flags/`\1` capture-group semantics apply unchanged), saves if it changed, then refocuses the buffer that was active before. No-space (`:cfar/…`, via an `is_far` prefix arm mirroring `is_substitute`) and spaced (`:cfar /…`) forms both parse. Reports "replaced in N of M file(s)". Live inline preview / per-hunk review = follow-ups.
- **Tests:** 2 Rust units (multi-file replace with an unmatched file left intact; no-results-list error path leaves the buffer untouched) + tests/pty_cfar.py (3 geometries: a real `:grep` builds the list, `:cfar` rewrites both files on disk and leaves a non-listed file alone).
- **Verified:** 513 Rust tests pass; clippy clean; PTY green; manual grep→cfar pipeline confirmed on the release binary.

### 2026-09-23 — minimap (optional/stretch, partial)
- **Shipped:** `:set minimap` (`mmp`) reserves a fixed 12-col strip on the right of each pane (only when the pane stays usably wide). `draw_minimap` renders a dim `│` separator + a per-row `minimap_shape`: each source line compressed to a block-glyph silhouette spanning its first→last non-whitespace column (source cols 0..80 scaled across the strip), so indentation depth and line length read at a glance. The minimap rows covering the on-screen logical lines are tinted as a viewport indicator. Content width shrinks by the strip; `RowSignature.width` already keys the row cache, so toggling repaints cleanly. Drawn after sticky-scroll so it always owns its columns. Animations / Kitty images = follow-ups.
- **Tests:** 1 Rust unit (`minimap_shape` width + indent/length silhouette) + tests/pty_minimap.py (3 geometries: separator + blocks appear, viewport rows tinted while off-screen rows aren't, `:set nominimap` reclaims the strip). Re-ran pty_split_open + pty_stickyscroll + pty_cursorline (shared width/draw path): no regression.
- **Verified:** 511 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — diff mode (4.1, partial)
- **Shipped:** `src/diff.rs` — `:diffthis` marks buffers (first two compared), `:diffoff` clears. `update_diff` (per-frame, stamped on both buffers' edit_seqs) line-diffs them with `similar::TextDiff` and records each side's differing line numbers; those lines get a dark-green row background. The three per-row background sites (gutter, run fill, trailing pad) were unified into one `row_bg` (diff > cursorline), and `diff_line` is in `RowSignature` so it repaints on change. Side-by-side sync-scroll, unchanged-region folding, per-side add/delete colors, and 3-way merge = follow-ups.
- **Tests:** 1 Rust unit (two files → the changed line is marked on both sides, equal lines aren't, `:diffoff` clears) + tests/pty_diff.py (3 geometries; the differing line is tinted, an equal line isn't, cleared on `:diffoff`). Re-ran pty_cursorline + pty_colorcolumn + pty_listchars (the `row_bg` refactor): no regression.
- **Verified:** 510 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — non-UTF-8 encodings (1.9 complete)
- **Shipped:** `Encoding` (Utf8/Latin1/Utf16Le/Utf16Be) + `decode_bytes`/`encode_bytes`. `Buffer::from_path`/`reload` now read raw bytes and detect the encoding (UTF-16 by BOM `FF FE`/`FE FF`, else valid UTF-8, else latin1), decode to the internal UTF-8 rope, and record it; `save`/`save_force`/`save_as` write `encoded_bytes()` (fileformat + BOM + target encoding). The external-change guards (`save`/`changed_on_disk`) decode before comparing, so an encoding/EOL-only difference isn't a false change. The status ruler shows `[latin1]`/`[utf-16le]` etc. This finishes checklist item 1.9.
- **Tests:** 3 Rust units (latin1 `é` round-trips through an edit; UTF-16LE round-trips with its BOM; UTF-8 unaffected) + tests/pty_encoding.py (3 geometries; a latin1 file shows `café`, marks `[latin1]`, and `:w` re-encodes on disk). Re-ran pty_fileformat + pty_focus_reload + pty_on_save_hooks: no regression.
- **Verified:** 509 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — LSP semantic tokens (2.1, partial)
- **Shipped:** `:set semantictokens` requests `textDocument/semanticTokens/full` once per `(buffer, edit_seq)` when a capable server is ready (triggered at the top of `sync_lsp`, before its synced-early-return, so enabling on an already-open buffer still fires; deduped via `semantic_requested_seq`). The delta-encoded stream is decoded against the server's `legend.tokenTypes`; `render::semantic_index` maps type names → a palette, and the paint loop overlays that color over the base tree-sitter color (yielding to documentColor/rainbow). Gated on a `(buffer, edit_seq)` stamp so stale tokens never paint. Advertised via `semanticTokensProvider`. Delta/range updates + modifiers = follow-up.
- **Tests:** 2 Rust units (type-name→palette mapping; `:set`/`nosemantic` toggle clears) + tests/pty_semantic_tokens.py (3 geometries; mock `--semantic` marks an identifier as `keyword` → it turns cyan only when enabled, gone when disabled). Mock LSP extended with `semanticTokensProvider` + a `semanticTokens/full` handler. Re-ran pty_diagnostic_rendering + pty_pull_diagnostics + pty_goto_lsp: no regression.
- **Verified:** 506 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — colorschemes + :colorscheme (0.4, partial)
- **Shipped:** `src/theme.rs` — a `Theme` mapping the four syntax `HlClass`es to colors, with built-in schemes `default`, `mono` (256-color greys), and true-color `warm`/`cool`. The two hardcoded `HlClass → Color` match arms in render now read `ed.theme.syntax(class)`. `:colorscheme [name]` swaps the active theme live (bumping `syntax_stamp` to invalidate the row cache) and lists the schemes when bare; `config.colorscheme` sets the startup scheme. UI-color theming, undercurl, transparent backgrounds = remaining.
- **Tests:** 2 Rust units (`:colorscheme` switch/unknown/restore; all built-in names resolve, bogus doesn't) + tests/pty_colorscheme.py (3 geometries; a keyword is cyan `00ffff` by default, recolored after `:colorscheme warm`, restored after `default`). Re-ran pty_highlight_colors: no regression.
- **Verified:** 504 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — configurable statusline (2.11, partial)
- **Shipped:** a `statusline` config format string (empty = built-in layout). `expand_statusline` (taking a `StatusInfo`) expands `%f`/`%F` (name), `%l`/`%c` (cursor), `%L` (total), `%m` (modified), `%y` (filetype), `%p` (percent), `%M` (mode), `%%`; unknown `%x` passes through. When set, it replaces the left segment of the status line; the `line:col` (+ LSP progress) ruler always stays on the right. Global statusline, statuscolumn, and winbar/breadcrumbs remain.
- **Tests:** 1 Rust unit (token expansion incl. percent, modified-flag, unknown-token passthrough) + tests/pty_statusline.py (3 geometries; a `FT=%y FILE=%f LN=%l/%L` format renders and `%l` updates on `G`). Re-ran pty_resize_status + pty_zen: no regression.
- **Verified:** 502 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — sticky scroll / context header (2.8, partial)
- **Shipped:** `:set stickyscroll` (config, default off). `Syntax::context_starts(byte, kinds)` walks the ancestors of the top-of-viewport byte, collecting function/class/impl/trait/mod nodes whose declaration is scrolled off above it (outermost first). `draw_pane` overlays up to 3 of those source lines (grey `STICKY_BG`) onto the top rows — drawn after the content loop into the frame, so the row-diff repaints them as you scroll. Only for the current buffer's pane with a parsed tree, and never in zen. LSP `foldingRange` fallback for non-tree-sitter buffers = follow-up.
- **Tests:** 1 Rust unit (`:set stickyscroll`/`nosticky` toggle) + tests/pty_stickyscroll.py (3 geometries; a long function's signature is pinned at row 0 after `G`, absent by default and after disabling). Re-ran pty_zen + pty_textobjects: no regression.
- **Verified:** 501 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — zen/focus mode (Wave C)
- **Shipped:** `:zen` toggles a distraction-free layout: `gutter()` returns 0 (no line numbers/signs) and `draw_pane` skips the per-pane status line, reclaiming that row for buffer content (`n = height` instead of `height-1`). The frame row-diff is the correctness safety net for the reclaimed row, so scrolling and splits keep working. Toggling back restores both.
- **Tests:** 1 Rust unit (`:zen` toggle + message) + tests/pty_zen.py (3 geometries; status line + gutter hidden on, content at col 0, `G`/`gg` scroll correctly, restored on toggle). Re-ran pty_window_resize + pty_resize_status + pty_split_open: no regression.
- **Verified:** 500 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — notification toasts (Wave C)
- **Shipped:** `:set notifications` (config, default off) mirrors each new message into a bounded (8) `toasts` list; `draw_toasts` overlays the live ones (age < 4s) as a top-right stack (newest on top, `DarkCyan`, clipped) drawn into the frame just before the row-diff so it composes over any mode. They auto-fade: `draw` filters by TTL for display, and the idle loop calls `prune_toasts` + redraws once when a toast expires, so a toast disappears on its own with no further input. `:messages` remains the full history.
- **Tests:** 1 Rust unit (recorded only when enabled, bounded, not-expired-when-fresh, `:set` toggle) + tests/pty_notifications.py (3 geometries; an `E492` message appears as a top-row toast then fades within its TTL). Re-ran pty_cursorline + pty_colorcolumn: no regression.
- **Verified:** 499 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — send-to-terminal / REPL (4.5 complete)
- **Shipped:** `Editor::send_to_terminal` writes text (with a trailing newline so it executes) to the terminal focused in the active window, else the most recently opened one, without leaving the current buffer. `:termsend` (`:tsend`) sends the current line, or — with an explicit Ex range or a Visual selection (via the dispatch's `effective_range`) — every line in that range, giving a send-block-to-REPL workflow. Reports when there's no terminal. This finishes checklist item 4.5 (split/tab/float, toggle+reattach, and terminal-mode nav were already present).
- **Tests:** 2 Rust units (against a real `/bin/cat` terminal: current line then a 2-line range are both received; no-terminal message) — the deterministic approach the terminal write path itself is tested with.
- **Verified:** 498 Rust tests pass; clippy clean; release builds.

### 2026-09-23 — code tours (4.7, partial)
- **Shipped:** `src/tour.rs` — CodeTour-compatible `.tours/*.tour` JSON (`{title, steps:[{file, line, description}]}`). `:tours` lists them as a Results picker (Enter reruns `:tour <name>`); `:tour [name]` starts a tour (or the first) at step 1; `:tournext`/`:tourprev` walk the steps, opening each step's file, moving the cursor to its line, and showing `[i/n] description` in the message line. Robust to missing/malformed files (serde defaults, clamped navigation). Prompt bank = remaining.
- **Tests:** 1 Rust unit (start → jump to a.rs:3 with description, next → b.rs:2, clamp at end, prev returns) + tests/pty_tours.py (3 geometries; `:tour`/`:tournext` jump across files with descriptions). 
- **Verified:** 496 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — snippet variable transforms (1.8, partial)
- **Shipped:** LSP snippet variable transforms `${VAR/regex/format/flags}` are now applied at expand time (previously the tail was dropped). `split_transform` splits the spec on unescaped `/`; `apply_transform` builds the regex (honoring `i`) and replaces (all matches with `g`, else the first), with `$1`/`${1}` capture references in the format. Unset variables transform the empty string. Infallible: any regex error returns the value unchanged. Numbered-stop transforms (which need live re-transform) still degrade to a plain stop. Choice placeholders (`${n|a,b,c|}`) were already implemented.
- **Tests:** 2 Rust units (filename-extension strip, `g`/`i` flags, unset-variable → empty) alongside the existing snippet unit suite; the unimplemented numbered-transform test still passes unchanged.
- **Verified:** 495 Rust tests pass; clippy clean.

### 2026-09-23 — LSP linked editing (2.4, partial)
- **Shipped:** `:linkededit <name>` stashes the new name, requests `textDocument/linkedEditingRange` (advertised via `linkedEditingRangeProvider`), and on the response replaces every returned range with the name — applied right-to-left so earlier char indices stay valid — in a single undoable edit. This covers the common "rename an open/close tag pair together" case without a full live-mirror. Live type-to-mirror = follow-up.
- **Tests:** tests/pty_linkededit.py (3 geometries; mock links line 0 and line 2, `:linkededit XYZ` renames both, middle untouched, verified on disk). Mock LSP extended with `linkedEditingRangeProvider` + a `linkedEditingRange` handler. Re-ran pty_callhierarchy: no regression.
- **Verified:** 493 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — LSP pull diagnostics (2.3, partial)
- **Shipped:** client-side pull diagnostics. `LspClient::pull_diagnostics` sends `textDocument/diagnostic` and tracks its wire id in a separate `pending_diag` map; the response is emitted as `LspEvent::Diagnostics` (parsing the `full` report's `items`, ignoring `unchanged`) — deliberately NOT through the generic `Response` path, whose stale-revision guard would drop diagnostics after any edit. `sync_lsp` fires the pull after each open/change for servers advertising `diagnosticProvider` (also advertised in the client's init capabilities); docs opened before the server was ready get their first pull when `pending_docs` replays on init. Merges into the same store as push, so gutter marks / `:ldiagnostics` / underlines all work. Workspace diagnostics = remaining.
- **Tests:** tests/pty_pull_diagnostics.py (3 geometries; mock runs `--pull` — advertises `diagnosticProvider`, does NOT publish, replies to `textDocument/diagnostic` — and the error marker appears + `:ldiagnostics` lists `PULLEDDIAG`). Re-ran pty_diagnostic_rendering + pty_filetree_diagnostics + pty_goto_lsp + pty_lsp_progress: no regression (push path unaffected; mock pull gated behind `--pull`).
- **Verified:** 493 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — LSP call + type hierarchy (2.2 complete)
- **Shipped:** the two-step LSP chain now drives four directions from one generalized prepare→chain arm: `:callers`/`:callees` (`callHierarchy/incomingCalls`/`outgoingCalls`) and `:supertypes`/`:subtypes` (`typeHierarchy/supertypes`/`subtypes`). The step-2 arm parses `from`/`to`-wrapped call items and bare type items uniformly into jumpable Results locations. Advertised via `callHierarchyProvider`/`typeHierarchyProvider`. This finishes checklist item 2.2.
- **Tests:** tests/pty_callhierarchy.py (3 geometries; callers list + Enter jump, then callees, supertypes, subtypes each resolve their list). Mock LSP extended with `typeHierarchyProvider` + `prepareTypeHierarchy`/`supertypes`/`subtypes`/`outgoingCalls` handlers. Re-ran pty_goto_lsp: no regression.
- **Verified:** 493 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — task/test runner → quickfix (4.3, partial)
- **Shipped:** `src/task.rs` — `:make [cmd]`/`:task [cmd]` runs a command via the shell in the project root on a background thread (polled through `poll_make_task` in `poll_jobs`), parses `file:line[:col][:] message` output (a path must contain `.`/`/` to avoid matching bare `12:34`; also accepts Rust's `--> file:line:col`) into quickfix `Entry::location`s, and installs it as the quickfix list (with history) — Enter jumps, `:copen`/`:cnext` work. With no argument the command defaults from Cargo.toml→`cargo build`, go.mod→`go build ./...`, package.json→`npm run build`, Makefile→`make`. When nothing parses, the raw output is shown so compiler messages are still visible. Test-under-cursor + watch = remaining.
- **Tests:** 1 Rust unit (fake `printf` compiler output → 2 quickfix locations, 0-indexed line/col) + tests/pty_make.py (3 geometries; `:make` populates quickfix, Enter jumps to `code.rs:4`). Re-ran pty_quickfix_history + pty_loclist: no regression.
- **Verified:** 493 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — live spell underline + ]s/[s (Wave C)
- **Shipped:** `:set spell` underlines misspelled words inline (magenta, folded into the existing diagnostic-underline mechanism so diagnostics still win a glyph); `]s`/`[s` jump to the next/prev misspelling (with a jumplist entry). `update_spell_spans` recomputes the whole-buffer misspelled `(line, start, end)` spans on a `(buffer, edit_seq)` stamp (called once per frame, cheap when unchanged); per-row `spell_ranges` in `RowSignature` keep the row cache correct. Degrades cleanly with no system dictionary.
- **Tests:** 1 Rust unit (Dictionary::for_test: two misspellings detected, `]s`/`[s` advance/wrap) + tests/pty_spell_live.py (3 geometries; underlines the misspelling when a dictionary exists, else verifies the graceful `]s` message). Re-ran pty_spell + pty_diagnostic_rendering: no regression.
- **Verified:** 492 Rust tests pass; clippy clean; PTY green (dict path exercised on this machine).

### 2026-09-23 — rainbow delimiters (2.9, partial)
- **Shipped:** `:set rainbow` colors `()[]{}` by nesting depth (7-color palette, matching pairs share a color). A whole-buffer bracket scan (`rainbow_brackets`) is cached in a `RefCell` on the Editor keyed by `(buffer, edit_seq)` — recomputed only on edit; per-row `(col, depth)` lists go into `RowSignature` so the row cache stays correct, and the paint loop overrides the bracket glyph's fg. Default off. Skipping brackets inside strings/comments + tree-sitter injection highlighting = remaining.
- **Tests:** 1 Rust unit (`:set rainbow`/`norainbow` toggle) + tests/pty_rainbow.py (3 geometries; nested `(` get distinct non-default fgs when on, both default when off/disabled). Re-ran pty_documentcolor + pty_listchars + pty_highlight_colors: no regression.
- **Verified:** 491 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — location list (5.4, partial)
- **Shipped:** a second, independent quickfix-like list. `:ldiagnostics` fills it from the current buffer's diagnostics (scoped to one buffer, unlike the all-files quickfix); `:lopen` reopens the stored list and `:lnext`/`:lprev` (`:lne`/`:lp`) step through it and jump — all backed by a new `loclist: Option<Results>` slot that's separate from `quickfix`, so the two lists don't clobber each other. Per-window loclists and `:lgrep`/`:lvimgrep` producers = follow-up.
- **Tests:** 1 Rust unit (populate from two diagnostics, `:lnext` jumps + wraps) + tests/pty_loclist.py (3 geometries; `:ldiagnostics` lists the mock's warning, Enter jumps to its line). Re-ran pty_quickfix_history + pty_filetree_diagnostics: no regression.
- **Verified:** 490 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — git revert + cherry-pick (4.2 complete)
- **Shipped:** `:gitrevert [hash]` (default HEAD; `git revert --no-edit`) and `:gitcherrypick <hash>`. On success they surface the result and call a new `after_git_tree_change` helper that reloads any unmodified buffer whose file changed on disk and refreshes git decorations; conflicts/errors are surfaced verbatim for manual resolution. This finishes checklist item 4.2.
- **Tests:** 2 Rust units (real git fixture: revert adds a commit and restores the file; cherry-pick brings a feature-branch file onto master) + tests/pty_gitrevert.py (3 geometries; `:gitrevert HEAD` restores the buffer content live and adds a commit). Re-ran pty_git_file_history + pty_git_status: no regression.
- **Verified:** 489 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — git file history (4.2, partial)
- **Shipped:** `:gitfilehistory` (alias `:gitfilelog`) runs `git log --follow -- <current file>` on a background thread and lists the commits that touched it as a Results picker; Enter reuses the existing `_vaayu_git_show_commit` action to show that commit's diff. Mirrors the existing `:gitlog` commit browser. Cherry-pick/revert remain.
- **Tests:** 1 Rust unit (real git fixture: two commits touch the file → 2 entries; Enter → `git show`) + tests/pty_git_file_history.py (3 geometries; an unrelated commit is correctly excluded, Enter shows the diff). Re-ran pty_gitstash: no regression.
- **Verified:** 487 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — LSP document color (2.5, partial)
- **Shipped:** `,lC` (`lsp.document_color`) requests `textDocument/documentColor` (advertised via `colorProvider`) and paints each returned color literal in its own true-color RGB, mirroring the documentHighlight decoration pattern: results stored as `ColorSpan`s gated on `document_colors_buffer`/`_edit_seq` (so an edit invalidates them), a per-row `color_ranges` added to `RowSignature`, and the glyph `color` overridden with `Color::Rgb` in the paint loop. Clears on Esc / LSP restart. Swatch glyphs + an interactive picker = follow-up.
- **Tests:** 1 Rust unit (Esc clears colors) + tests/pty_documentcolor.py (3 geometries; mock returns red for chars 0-3 → those cells' fg becomes `ff0000`, then Esc clears). Mock LSP extended with `colorProvider` + a `documentColor` handler. Re-ran pty_document_highlight + pty_highlight_colors: no regression.
- **Verified:** 486 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — runtime :set for existing options (config surface, 6.x)
- **Shipped:** `:set` now toggles the previously config-file-only options at runtime: `[no]relativenumber`/`rnu`, `[no]ignorecase`/`ic`, `[no]smartcase`/`scs`, `[no]smartindent`/`si`, `[no]expandtab`/`et`, `[no]autopairs`, and numeric `tabstop`/`ts`, `shiftwidth`/`sw`, `scrolloff`/`so`, `textwidth`/`tw`, `updatetime`/`ut` = N (with Vim short forms). `tabstop`/`shiftwidth`/`expandtab` apply to both the current buffer and the config default (so they take effect live and new buffers inherit them); the layout cache keys on `b.tabstop`, so a tab-width change repaints immediately.
- **Tests:** 1 Rust unit (bools + numerics incl. buffer-local tabstop/expandtab) + tests/pty_set_options.py (3 geometries; `:set tabstop=8` widens a tab from `>---` to `>-------` live via listchars). Re-ran pty_listchars + pty_indent: no regression.
- **Verified:** 485 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — shada: marks + jumplist (5.3 complete)
- **Shipped:** shada now also persists named marks (a-z/A-Z/0-9 that point at a real file) and the jumplist (last 100 path-bearing entries). Persisted as a `SavedLoc { path, line, col }` — only the path survives, and on restore the `Location` is rebuilt with buffer id 0 so navigation resolves by path (opening the file if needed). Restored at startup into `marks`/`jumps`. This finishes checklist item 5.3.
- **Tests:** 1 Rust unit (mark + jump round-trip) + tests/pty_shada_marks.py (3 geometries; set mark on line 3 but quit from line 10, then `` `a `` in a fresh process jumps to line 3 — proving the mark, not the restored cursor, drove it). Re-ran pty_shada + pty_shada_registers: no regression.
- **Verified:** 484 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — shada: registers + command/search history (5.3)
- **Shipped:** extended `.vaayu/shada.json` (backward-compatible via `#[serde(default)]`) to also persist named registers (`RegisterEntry` made serde, `Registers::restore` accessor; clipboard/blackhole and >100 KB entries skipped) and the last 100 command/search-history entries. `load_shada` now restores all three and is called eagerly at startup (so `q:`/`@a` see prior state even before a file opens); `save_shada` writes them on quit. Marks + jumplist remain.
- **Tests:** 1 Rust unit (register + both histories round-trip) + tests/pty_shada_registers.py (3 geometries; yank into `a` in one process, paste `"ap` in a fresh process on a different file). Re-ran pty_shada: no regression.
- **Verified:** 483 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — shada per-file cursor position (5.3, partial)
- **Shipped:** `src/shada.rs` — per-file last cursor position persisted to `.vaayu/shada.json` (per project, atomic write + lock + gitignore, mirroring session.rs). Loaded lazily; `open_file` restores the cursor for a freshly loaded file (clamped); `save_shada` (called on quit from the main loop) records every open buffer's cursor and prunes entries whose file no longer exists. Config `restore_cursor` (default on); VCS message files (COMMIT_EDITMSG/MERGE_MSG/…) are always left at the top. Marks, registers, jumplist, and command/search history = remaining.
- **Tests:** 3 Rust units (round-trip restore, VCS-message skip, `restore_cursor=false`) + tests/pty_shada.py (3 geometries; two real processes — jump to line 10, quit, relaunch → cursor restored). Re-ran pty_persistent_undo (shares `.vaayu`): no regression.
- **Verified:** 482 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — listchars / :set list (config surface, 6.x)
- **Shipped:** `:set list`/`nolist` (config `list`, default off) reveals a tab as `>` + `-` fill and trailing whitespace as `·`, dimmed. Done at the paint site by substituting the glyph's printed text and reusing the existing `color` field for dimming — glyph widths/positions (and cursor math) are untouched. `trail_start` is computed from `d.text` (the whole logical line), so trailing detection stays correct even for a wrapped segment (its glyphs keep full-line columns). `list` added to `RowSignature` so toggling repaints. Configurable listchars string, `eol`, and `nbsp`/`space` markers = follow-up.
- **Tests:** 1 Rust unit (`:set list`/`nolist` toggle) + tests/pty_listchars.py (3 geometries; leading tab → `>---`, trailing spaces → `·`, clean line untouched, restored on `nolist`). Re-ran pty_colorcolumn + pty_cursorline + pty_indent + pty_editing: no regression.
- **Verified:** 479 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — colorcolumn (config surface, 6.x)
- **Shipped:** `:set colorcolumn=N`/`cc=N` (config `colorcolumn`, 0 = off) draws a one-cell vertical ruler (dark-red bg) at that display column on every row — over text (a new `colorcol` flag in the per-glyph `GlyphStyle`, so the ruler cell becomes its own run) and past end-of-line (the trailing-pad fill splits into pre-ruler / ruler / post-ruler segments, composing correctly with cursorline). Ruler column added to `RowSignature` so toggling/moving it repaints. Selection/search/doc-highlight/word-diff still take precedence over the ruler.
- **Tests:** 1 Rust unit (`:set` parse incl. short form + `cc=0` disable) + tests/pty_colorcolumn.py (3 geometries; exactly one ruler cell over text, past EOL, and on an empty line; removed at `cc=0`). Re-ran pty_cursorline + pty_highlight_colors + pty_diagnostic_rendering: no regression.
- **Verified:** 478 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — cursorline (config surface, 6.x)
- **Shipped:** `:set cursorline`/`cul` (config `cursorline`, default off) tints the active window's cursor-line background (gutter + text runs + trailing pad) with a subtle 256-color grey. It rides the row cache's existing `current` signal plus a new `cursorline` field in `RowSignature`, so it repaints as the cursor moves and never bleeds onto an inactive split or under a stronger highlight (selection/search/doc-highlight/word-diff all take precedence). Virtual-text suffixes (diag/blame/lens) on the cursor line aren't tinted = minor follow-up.
- **Tests:** 1 Rust unit (`:set` toggles + short form) + tests/pty_cursorline.py (3 geometries; off by default, tint appears on enable, follows the cursor across a move, clears on disable). Re-ran pty_highlight_colors + pty_diagnostic_rendering + pty_document_highlight: no regression.
- **Verified:** 477 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — gq reflow operator (1.13)
- **Shipped:** `OperatorKind::Format` + `crate::operator::reflow`. `gq` is a normal-mode operator: `gqq` reflows the current line, `gq{motion}` reflows a range (`gq}` paragraph, `gqG` to EOF, `3gqq`, …). Reflow is paragraph-aware (blank lines split and are preserved), greedily packs words to `textwidth` (new config; 0 → 79 like Vim), and keeps each paragraph's leading indent. Comment-leader-aware reflow, `gw`, visual `gq`, `ip`/`ap` paragraph text objects, and dot-repeat are follow-ups.
- **Tests:** 4 Rust units (reflow width + word order, indent/paragraph preservation, `gqq`, `gq}` scoping) + tests/pty_reflow.py (3 geometries; `gq}` wraps only the first paragraph, verified on disk). Re-ran pty_editing + pty_align: no regression.
- **Verified:** 476 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — illuminate + CursorHold firing (Wave C)
- **Shipped:** `Editor::poll_cursor_hold` (called from the idle loop) detects the cursor coming to rest for `updatetime_ms` (new config, default 250) and fires the `CursorHold` event once per resting spot — wiring up the previously-defined-but-unfired event. When `illuminate` (new config, default on) is set it then auto-requests LSP `documentHighlight` for the symbol under the cursor, reusing the existing highlight rendering. A genuine cursor move clears stale highlights (but the first observation preserves freshly-set ones, so the manual `,lh` action still works); a silent capability check (`has_language_capability`) keeps it quiet when no server is attached.
- **Tests:** 3 Rust units (fires once then re-arms on move; inactive in Insert; move clears stale highlights while the first observation preserves them) + tests/pty_illuminate.py (3 geometries; resting auto-lights both occurrences via the mock LSP). Re-ran pty_document_highlight: no regression.
- **Verified:** 472 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — `]f`/`[f` function navigation (1.3 complete)
- **Shipped:** `Syntax::node_starts(kinds)` (iterative preorder walk → sorted, deduped byte offsets) + `Editor::goto_function(forward, count)`. `]f`/`[f` jump to the next/previous function or method definition start, honor a count (`2]f`), record a jumplist entry (so `Ctrl-o` returns), and no-op without a parsed tree or past the last/first definition. Factored the shared `FUNCTION_KINDS` list out of `tree_object_range` so text objects and navigation agree. This finishes checklist item 1.3.
- **Tests:** 4 Rust units (forward/back sequence + past-end clamp, count, `]f`/`[f` via keys, no-syntax no-op) + tests/pty_funcnav.py (3 geometries; ruler line verifies the jumps). Re-ran pty_textobjects + pty_incremental_selection: no regression.
- **Verified:** 469 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — argument text objects `aa`/`ia` (1.3b)
- **Shipped:** `ObjectKind::Argument` (`aa`/`ia`) resolving within the nearest enclosing `(...)`. `top_level_commas` splits arguments while skipping commas nested in `()[]{}` or inside `"`/`'`/`` ` `` strings; `ia` selects the trimmed argument, `aa` additionally takes the trailing comma + following whitespace (or the leading comma for the last argument, nothing for a sole argument). Works with any operator and in Visual mode (both dispatch through `object_kind`).
- **Tests:** 6 Rust units (inner middle, `aa` first/last, sole arg, nested + quoted commas, no-op outside parens) + tests/pty_argobject.py (3 geometries; `cia` change + `daa` drop, verified on disk). Re-ran pty_textobjects: no regression.
- **Verified:** 465 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — bracket-aware auto-indent (2.10, partial)
- **Shipped:** `Editor::auto_indent(line, split_col)` + `indent_unit()`. Enter, `o`, and `O` now copy the source line's leading whitespace and — when `smartindent` (new config, default on) is set — add one indent level if the text up to the split point ends with an opening bracket. `O` (open-above) copies indent only (split_col 0, no bracket bump). Also dropped a dead `rope.to_string().contains("\r\n")` check that ran on every Enter (now that the rope is always `\n`-only, it was both dead and a whole-buffer allocation per keystroke).
- **Tests:** 5 Rust units (helper: bracket bump / no-bump / nested / smartindent-off / tabs; Enter, `o`, `O` end-to-end) + tests/pty_smartindent.py (3 geometries; `:w` verifies the indented body on disk). Re-ran pty_autopairs + pty_indent + pty_editing: no regression.
- **Verified:** 459 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — fileformat / line endings (1.9, partial)
- **Shipped:** DOS/old-Mac/Unix line endings and a UTF-8 BOM are detected on load (`normalize_content`), the rope is kept `\n`-only, and the original format + BOM round-trip on save (`Buffer::encoded`, written by `save_force`/`save_as`). External-change guards (`save`/`changed_on_disk`) now compare `\n`-normalized content so a pure line-ending difference isn't a false "changed on disk". `:set ff=unix|dos|mac` re-encodes on next write, `:set ff?` reports it, and the status ruler shows `[dos]`/`[mac]`. Non-UTF-8 encodings (latin1/UTF-16) remain.
- **Tests:** 6 Rust units (dos/mac detect+preserve, unix default, BOM strip+restore, `:set ff=` convert-on-save with the guard passing, `:set` command validation) + tests/pty_fileformat.py (3 geometries: `[dos]` tag, no `^M` leak, `:w` preserves CRLF, `:set ff=unix`+`:w` converts to LF). Re-ran pty_on_save_hooks + pty_focus_reload: no regression.
- **Verified:** 454 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — gv reselect last visual selection (1.12)
- **Shipped:** `gv` in Normal mode reselects the most recent visual span. Each visual-mode keystroke records `(kind, anchor, cursor)` into `Editor::last_visual` at the top of `visual::handle`, so the operator/Esc key that ends visual mode captures the final span just before it is consumed — `gv` then restores mode + anchor + cursor. Works charwise/linewise/blockwise, survives operators (`vlly` then `gv`), and clamps a stale selection into a shrunken buffer. No-op with a friendly message when there is no prior selection.
- **Tests:** 5 Rust units (charwise, linewise, after-operator, no-prior-selection no-op, clamp-to-shrunken-buffer) + tests/pty_gv.py (3 geometries; `gv`+`d` deletes exactly the reselected span). Re-ran pty_textobjects + pty_incremental_selection: no regression.
- **Verified:** 448 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — split resize (Ctrl-W </>/+/-/=)
- **Shipped:** resizable splits. Added a `ratio` (serde-defaulted for old sessions) to `Layout::Split`; `rects` splits by ratio (0.5 reproduces the old exact split byte-for-byte, so un-resized layouts are unchanged). `Ctrl-W >`/`<` resize width, `+`/`-` height (nearest matching ancestor split, clamped 0.1..0.9), `=` equalizes all. Mouse drag-resize = follow-up.
- **Tests:** 1 Rust unit (resize changes widths; = restores) + tests/pty_window_resize.py (3 geometries; separator moves/restores). Re-ran 8 split/tab/tree/preview PTY tests: no regression.
- **Verified:** 444 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — FocusGained + autoread-on-focus (0.1 / Phase 1.7)
- **Shipped:** enabled terminal focus-change tracking; on focus-in the editor fires the FocusGained autocmd event and auto-reloads the current buffer if its file changed on disk with no unsaved edits (a dirty buffer is only warned, never clobbered). New Buffer::changed_on_disk + Editor::on_focus_gained.
- **Tests:** 1 Rust unit (clean buffer reloads; dirty buffer keeps its edit) + tests/pty_focus_reload.py (3 geometries; external edit + ESC[I reloads).
- **Verified:** 442 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — :todo index (Wave C, partial)
- **Shipped:** `:todo` runs a fixed-pattern grep for TODO/FIXME/HACK/XXX and shows a navigable "TODO / FIXME / HACK" results list (Enter jumps). Inline highlighting of those keywords = follow-up.
- **Tests:** 1 Rust unit (poll grep, asserts TODO+FIXME listed) + tests/pty_todo.py (3 geometries).
- **Verified:** 441 Rust tests pass; clippy clean; PTY green.

### 2026-09-23 — :earlier / :later (1.11a)
- **Shipped:** `:earlier [N]` undoes and `:later [N]` redoes N changes (default 1), stopping at the ends; reports how many. Added to the command list. Undo-tree viewer = follow-up.
- **Tests:** 1 Rust unit (three edits, earlier 2 / later 1). Command-only; covered by unit test.
- **Verified:** 440 Rust tests pass; clippy clean.

### 2026-09-23 — :checkhealth (6.x, partial)
- **Shipped:** `:checkhealth`/`:health` — a "Health" results report of external tools
  (rg/git/lazygit/clipboard, via `tools::on_path`), configured LSP servers (✓/✗ on
  PATH), and built-in tree-sitter grammars. Added to the command list.
- **Tests:** 1 Rust unit (sections + git present) + `tests/pty_checkhealth.py` (3 geometries).
- **Verified:** 439 Rust tests pass; clippy clean; PTY green.

### 2026-09-22 — 1.5 move lines (`]e` / `[e`)
- **Shipped:** `Editor::move_lines` (down/up by count, cursor follows, one undo step;
  swaps line *content* so newline structure incl. a no-final-newline last line is
  preserved). Wired to `]e`/`[e` in the `]`/`[` handler.
- **Tests:** 1 Rust unit (down/up, count, bottom no-op, undo) + `tests/pty_move_lines.py`
  (3 geometries, verified on disk).
- **Verified:** 438 Rust tests pass; clippy clean; PTY green.

### 2026-09-22 — 1.3 tree-sitter textobjects (function/class)
- **Shipped:** `af`/`if` (function around/inner) and `ac`/`ic` (class around/inner),
  working with operators (`daf`,`cif`,`dac`) and Visual (`vaf`). `Syntax::object_range`
  climbs to the nearest node of a function/class kind (per-language kind sets);
  inner = the body block's named-child span (between braces). `Editor::tree_object_range`
  maps `f`/`c` and converts byte→(line,col); wired into the normal + visual text-object
  handlers. Argument objects (`aa`/`ia`) and `]f`/`[f` nav = 1.3b.
- **Tests:** 1 Rust unit test (`daf` deletes fn, `dif` clears body, `dac` deletes struct)
  + `tests/pty_textobjects.py` (3 geometries: dif clears body → undo restores → daf deletes).
- **Verified:** 437 Rust tests pass; clippy clean; PTY green on 60×14 / 100×24 / 180×50.

### 2026-09-22 — 1.4 tree-sitter incremental selection (Wave B begins)
- **Shipped:** `,=` expands the selection to the enclosing syntax node, `,-` shrinks
  back along the expand path. `Syntax::expand_range` walks the parsed tree
  (`descendant_for_byte_range` + parent climb); `Editor::expand_selection`/
  `shrink_selection` convert char↔byte via ropey and drive a Visual selection, with
  a `select_stack` for exact shrink. Two leader actions registered. (No `.scm`
  query files needed — that infra, 0.5, is for textobjects/rainbow next.)
- **Tests:** 1 Rust unit test (expand grows, keeps growing, shrink returns to the
  prior ranges) + `tests/pty_incremental_selection.py` (3 geometries; counts
  reverse-video selection cells growing then shrinking).
- **Verified:** 436 Rust tests pass; clippy clean; PTY green on 60×14 / 100×24 / 180×50.

### 2026-09-22 — 0.2 keymap remapping (Wave A complete)
- **Shipped:** `[[keymap]]` config → `src/keymap.rs` (`parse_keys` notation parser
  for `<leader>`/`<C-x>`/`<CR>`/…; `Keymap`/`Rhs`; mode sets). Single-key remaps
  intercepted in `feed_key` (clean pending, not-replaying = noremap); leader-sequence
  remaps consulted in the leader handler with precedence over built-ins and
  prefix-wait. rhs is keys-to-replay or an Ex command (`:…<CR>`). New `Editor.keymaps`.
  Documented in `config.example.toml`. (Multi-key non-leader lhs like `jj` is a
  documented follow-up.)
- **Tests:** 6 Rust unit tests (keys remap; Ex remap; mode isolation; noremap
  non-chaining; leader remap; invalid-notation ignored) + `tests/pty_keymap.py`
  (3 geometries: `Y`→`y$`, `,w`→`:w`, insert `<C-l>`→text).
- **Verified:** 435 Rust tests pass; clippy clean; PTY green on 60×14 / 100×24 / 180×50.

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
