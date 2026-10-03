# Feature backlog (scheduled roadmap run, 2026-10-01 → 2026-10-04)

A scheduled agent works through this list, one independent feature per run
(every 3 hours), on branch `feature-roadmap`. Each run:

1. Pull `feature-roadmap`; pick the first item that is `[ ]` (or continue a
   `[~]` one -- large items may span several runs; leave notes under it).
2. Implement it fully, matching the surrounding code's style and the
   engineering protocol in ROADMAP.md: Rust unit tests, a PTY test under
   `tests/` for anything user-visible, and HELP.md / config.example.toml /
   README.md updates where relevant.
3. Verify: `cargo build`, `cargo test`, the relevant PTY tests
   (`python3 tests/pty_*.py target/release/vaayu`), and that clippy/rustfmt
   add no *new* findings in touched files (see item 1 for the baseline).
   Never reformat files you didn't otherwise change.
4. Commit (one commit per feature, descriptive message ending with the
   Co-Authored-By line) and `git push origin feature-roadmap`. Never push to
   `master`.
5. Mark the item `[x]` here (or `[~]` with progress notes), commit, push.

Stop once every item is `[x]` or after 2026-10-04.

Status: `[ ]` todo · `[~]` in progress · `[x]` done + tested + pushed.

## Items, in order

- [x] **1. Pin the toolchain.** Add `rust-toolchain.toml` pinning a stable
  Rust whose `rustfmt`/`clippy` agree with the code as committed (HEAD is
  not fmt/clippy-clean under Rust 1.99: `cargo fmt` rewrites ~17 files and
  clippy reports 24 errors). If no released version matches, instead pin
  current stable and fix the lints + format the tree in one isolated
  commit. CI (`.github/workflows/ci.yml`) must use the same toolchain.
  *Done (2026-10-01):* no rustfmt release accepts the committed code (lines
  over the default width), so pinned `1.99.0` (+rustfmt, clippy) in
  `rust-toolchain.toml`, formatted the tree and fixed the 9 distinct clippy
  lints in one isolated commit; CI installs 1.99.0 explicitly. The tree is
  now fmt/clippy-clean -- later runs: zero findings is the baseline.
  Pre-existing, not caused by this item: `filetree::tests::copy_recursive_
  rolls_back_a_partial_copy_on_failure` fails when run as root (mode 000
  doesn't block root reads; CI runs non-root), and
  `tests/pty_live_grep_filter.py` fails on HEAD ("filtering a live list
  should narrow its current matches").
- [x] **2. Refresh NEOVIM_PARITY_PLAN.md's plugin matrix.** Several rows
  say "Missing" for things that exist (todo-comments, illuminate, code
  tours, minimap, ...). Audit each row against the code and fix statuses.
  *Done (2026-10-02):* 1 row → Done (gitsigns/mini.diff), 14 Missing →
  Partial with what remains, 5 still Missing; stale closing log entry
  replaced. Noted, not fixed: `,z` ("Toggle zen") only toggles line
  numbers unlike `:zen`; illuminate lacks a large-file cutoff.
- [x] **3. Floating window primitive + goto-preview.** A reusable floating
  pane (bordered, scrollable, dismissable, focusable) and LSP peek views on
  top of it: definition, type definition, implementation, references
  (references as a list with a source preview). Keys under `,p` or `gp*`
  (check which-key for conflicts).
  *Done (2026-10-02):* `src/float.rs` -- `Float` (text or peek body,
  focused/unfocused, anchor-based dismissal), `place`/`outer_width`
  geometry, `float::handle` keys; drawn by `render::draw_float`. Peeks:
  `gpd/gpt/gpi/gpr` + `,pd/,pt/,pi/,pr`, `,pk` hover float, `,pf`/`,pc`/`gP`
  focus/close. Item 9's tree preview can use `Editor::open_text_float` or
  add a `FloatBody` variant. Not done: mouse interaction with the float,
  syntax highlighting inside it, stacked/nested peeks (one float at a
  time). Pre-existing, unrelated: `tests/pty_results_preview.py` fails on
  HEAD too ("preview on should show real neighboring source lines").
- [x] **4. Progress UI (fidget-style).** A non-blocking task/progress model:
  LSP `$/progress` (workDoneProgress), plus grep / git / format / make /
  file-scan jobs, shown as a compact stacked status in the bottom-right
  corner that fades out when done.
  *Done (2026-10-02):* `src/progress.rs` -- `Progress` (begin/report/
  finish/`track` reconciliation/`tick`), drawn by `render::draw_progress`
  above the status line; `poll_jobs` reconciles the grep/git/blame/make/
  format/file-scan slots, LSP events feed it directly. 250ms show delay,
  2s linger (last 1s dimmed), 5-row cap. `progress = true` /
  `:set [no]progress`. PTY: `tests/pty_progress.py`. Item 6 (watcher) /
  8 (test runner) can call `ed.progress.begin/finish` for their jobs. Not
  done: no `:progress` history view; `:make` with parsed locations counts
  as ✓ even if the command failed (its Results title drops the exit
  status). Pre-existing PTY failures on HEAD too (not caused here):
  pty_cfar, pty_code_lens, pty_everything_picker, pty_file_picker_preview,
  pty_grep_word, pty_live_grep_filter, pty_picker_ranking,
  pty_preview_scroll_bounds, pty_projects_picker, pty_results_preview.
- [x] **5. Theme system.** Move hardcoded UI colors (statusline, tree,
  picker, results, diagnostics, git, popups) into `Theme` tokens; add
  gruvbox, flexoki, tokyonight (dark + light where sensible); a transparent
  background toggle (`:set transparent` / config `transparent = true`);
  `:colorscheme` completion. Existing schemes must look unchanged.
  *Done (2026-10-02):* `src/theme.rs` -- ~45 tokens (defaults = the old
  constants), `Palette` → `Theme` for `gruvbox`/`-light`, `flexoki`/
  `-light`, `tokyonight`/`tokyonight-day`; base bg/fg applied at row
  emission by `render::paint_base` (caches stay theme-independent;
  `FrameCache::base` change repaints all rows). `transparent` config +
  `:set [no]transparent`; `:colorscheme <Tab>`. PTY:
  `tests/pty_themes.py`. Left hardcoded on purpose: rainbow-bracket cycle,
  code-lens DarkCyan, file-type icon colors, terminal-pane colors, and the
  White text `plain_row` draws on non-default row backgrounds (light
  schemes pick bars dark enough for it; a unit test checks contrast).
  Not done: user-defined themes in config, `:highlight`-style overrides.
- [x] **6. Filesystem watcher.** inotify/FSEvents (e.g. the `notify`
  crate, debounced) feeding: the file tree (replacing its 1s stat poll
  while keeping the poll as fallback), autoread of unmodified open buffers
  changed on disk (prompt / mark when the buffer is modified), and the
  project file index used by the picker and tree `/`.
  *Done (2026-10-02):* `src/watcher.rs` -- `Watcher` (notify 8, recursive
  watch on the project root set up off-thread; 75ms debounce / 500ms max
  latency `Batch`es of structural vs content paths, `rescan` on overflow)
  and `Editor::poll_watcher` / `apply_fs_batch` from `poll_jobs`. Tree:
  `FileTree::invalidate` re-reads changed cached dirs; the 1s poll now
  skips dirs the watcher `covers`. Autoread: clean buffers reload, dirty
  ones get `Buffer::disk_changed` (`[changed on disk]` in the default
  status line; cleared by reload/save). Index: rescans on create/remove/
  rename outside `.git`, rate-limited to one per 2s. Parents of buffers
  outside the root are watched non-recursively. Never watches `$HOME` or
  `/` recursively; failures fall back to polling (`:checkhealth` line).
  `watch`/`autoread` config + `:set [no]watch`/`[no]autoread`. PTY:
  `tests/pty_watcher.py`. Not done: a `%`-statusline flag for
  `disk_changed` (custom `statusline` formats don't show it); no
  interactive reload prompt (Vim's W11 dialog) -- it warns and marks;
  no ignore list for huge non-gitignored trees (the inotify watch limit
  just makes it fall back). Note: `tests/pty_picker_ranking.py` /
  `pty_file_picker_preview.py` (pre-existing failures) send one Esc to
  close the picker, but the picker's first Esc only leaves the query --
  likely stale tests rather than a bug.
- [x] **7. Reviewed project-wide replace (grug-far).** One screen: search,
  replace, glob filter, live per-match preview; toggle individual matches
  or files; apply selected; undo-able per buffer. Builds on `:cfar` and the
  existing replace preview.
  *Done (2026-10-02):* `src/far.rs` + `Mode::Far` / `render::draw_far`.
  `,sr` / bare `:far` (resumes the last state), `,sw` (word/selection as a
  literal search). Background debounced scan of `all_files` (+ open
  buffers' live text) with `:s` regex syntax; `Globs` (`*.rs !tests/**`,
  `{a,b}`, bare dir names); per-match/per-file toggles keyed by
  (path, line, byte) so they survive re-scans; `R` applies one undo step
  per buffer (hidden buffers loaded with persistent undo; dirty buffers
  left unsaved; lines changed since the scan skipped); `U` undoes the last
  apply. PTY: `tests/pty_far.py`. Not done: multi-line patterns (per-line
  only, like `:s`); no in-screen "edit the preview line" (grug-far's
  sync-back); no mouse; `:cfarpreview` left as is. Pre-existing PTY
  failures confirmed identical on HEAD: `pty_cfar`, `pty_grep_word`.
- [x] **8. Test runner.** Detect the project's runner (cargo, pytest, go
  test, jest/vitest, ...); run nearest test / file / suite / last; stream
  into a terminal or job pane; parse failures into quickfix; gutter marks
  for pass/fail.
  *Done (2026-10-02):* `src/testrun.rs` -- `detect` (by file extension,
  else Cargo.toml / go.mod / package.json (jest/vitest/npm) / pytest
  markers), `build_command` per runner × scope (cargo: `-p` for workspace
  members, `--test`/`--lib`/`--bins`/`--bin`, `a::b::` module filter for
  file runs; `pytest -v … -k`; `go test -v -run '^X$' ./pkg`; `npx jest` /
  `npx vitest run` / `npm test -- … -t`), `parse` per runner (cargo panics
  old+new format, pytest -v + tracebacks + summary, go -v incl. subtests,
  jest/vitest marks + `●`/`FAIL` blocks). Runs via `sh -c` in its own
  process group with stdout+stderr streamed line by line into a live
  Results pane (`Results::test_output`); failures → quickfix (build errors
  fall back to `task::parse_errorformat`); `✓`/`✗` sign-column marks on
  test declarations (`TestRunner::mark`, matched by name at render time
  so they follow edits). `,Tn/,Tf/,Ts/,Tl/,To/,Tx/,Tc` + `:test [arg]`,
  `:testnearest/file/suite/last/output/stop/clear`; the old
  `task::test_command_for`/`test_nearest` were folded in. Progress-stack
  row "test". PTY: `tests/pty_test_runner.py` (fake `pytest` on PATH).
  Not done: no config override for the runner/command (`:make` covers
  custom commands); no "running" mark per test; nearest-test name is a
  substring filter for cargo/jest (may run same-named tests elsewhere);
  cargo, pytest and go parsers were checked against real runs; jest/
  vitest only against recorded output formats (no network npm install).
- [x] **9. File tree follow-ups.** Sort modes (name / type / mtime / size);
  floating preview (on item 3); restore from `.vaayu/trash`; persist
  expanded dirs, width and bookmarks in shada; stage/unstage the node
  under the cursor (`gs`/`gu`).
  *Done (2026-10-02):* `filetree::SortMode` -- `O` cycles, `:treesort
  <mode>`, config `tree_sort`; dirs always first; listings sorted at load
  (`load_dir(dir, sort)`, stat per entry only for mtime/size; switching
  drops the cache; the watcher re-reads a dir on content changes under
  mtime/size); header shows `↓mode` when it fits. `v`: unfocused
  `Float` with the new `beside_tree` placement (`render::tree_float_
  region`), follows the cursor, dir = entry list; Esc in the tree only
  closes the float. `U` / `:treetrash`: Results list of the trash, newest
  first; Enter restores (origins recorded in `.vaayu/trash.json`, legacy
  entries go to the project root; refuses to overwrite). `gs`/`gu`
  (marks-aware; unstage falls back to `git rm --cached` before the first
  commit). Shada `tree`: expanded dirs, bookmarks, width (a changed
  `tree_width` config wins). Background tree `git status` now runs with
  `--no-optional-locks` (it raced `gs` for `index.lock`). PTY:
  `tests/pty_filetree_followups.py`. Not done: the `/` filtered view
  keeps name order; no "empty trash" command; the preview float is plain
  text (no syntax highlighting, like item 3's floats).
- [x] **10. GitHub workspace (read-first).** Via the `gh` CLI when
  available: PR list/checkout, PR diff + review threads in the existing
  review UI, CI check status and logs, issues list. Degrade with a clear
  message without `gh`/auth.
  *Done (2026-10-03):* `src/github.rs` -- every view a `Results` list from
  a background `gh` job (`Editor::gh_task`, progress row "gh"). `,Gp`/
  `:ghprs [state]` → Enter → `,Gv`/`:ghpr [N]` overview (rollup summary,
  description, action rows); `:ghcheckout N` (dirty guard; buffers
  reloaded via `reload_after_checkout`, shared with `:gitbranch`);
  `,Gd`/`:ghdiff` (hunk lines → local-file locations); `,Gr`/`:ghthreads`
  (REST `pulls/N/comments`, replies grouped under roots, `(outdated)`);
  `,Gc`/`:ghchecks` (failures first) → Enter → Actions job log
  (`--log-failed`, by step, cursor on first error); `,Gi`/`:ghissues` →
  Enter → issue with comments. N defaults to the current branch's PR.
  No gh / not logged in / no GitHub remote → one message each;
  `:checkhealth` lists gh. PTY: `tests/pty_github.py` (fake gh). Not done:
  thread resolved/unresolved state (REST lacks it; needs GraphQL
  `reviewThreads`); top-level PR conversation comments; anything that
  writes (comment/approve/merge); `--limit 100` on lists without paging;
  logs of non-Actions checks just show their URL. Only exercised against
  a fake gh (the real one is untested here: no auth in the sandbox).
- [x] **11. Multiple cursors.** Add cursor at next match (`Ctrl-N`-style),
  column/visual-block to cursors, all-matches; Insert-mode typing, motions
  and basic operators applied to every cursor; single undo step; Esc
  collapses. Largest item -- expect several runs; keep it behind a clean
  module boundary.
  *Done (2026-10-03):* `src/multicursor.rs` (cursors only; the buffer
  cursor is the primary, secondaries are char indices on `Editor::multi`).
  `Ctrl-N` (next whole-word occurrence, wraps, new one = primary), Visual
  `Ctrl-N` (char: selection as literal pattern; block: one per line at the
  left column, short lines skipped; line: one per line), `,ma` all
  occurrences, `,mj`/`,mk` below/above, `Esc`/`,mc` collapse. Normal
  commands run on the primary key by key, then their keys replay at each
  secondary once pending clears (counts, registers, operators, `.`, `@q`
  work; `:`, `/`, Visual, pickers, leader actions, `q{reg}` stay
  primary-only); Insert keys run at every secondary then the primary.
  Positions are mapped through a per-buffer edit log
  (`Buffer::set_edit_log`); an `edit_seq` change the log can't explain
  (undo, reload) collapses; `u`/`Ctrl-R` collapse first. One command / one
  Insert session = one undo step (`Buffer::squash_undo`). Secondaries drawn
  reversed (also past EOL), status line "N cursors"; completion popup off
  while active. PTY: `tests/pty_multicursor.py`. Not done: per-cursor
  selections / extend mode (Visual with cursors acts on the primary only),
  skip/remove-one-cursor keys, per-cursor registers (the last cursor's
  yank wins), mouse add-cursor; `InsertLeave` autocmds fire once per
  cursor.
