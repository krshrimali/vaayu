# Vaayu re-audit and implementation report

Reviewed 2026-09-15; follow-up implementation 2026-09-16. This supersedes the earlier audit of the five-test editor.
Scope: the Rust source, terminal loop, editing model, integrations, persistence,
configuration, documentation, tests, and recent performance changes. This is a
source review with targeted behavioral verification, not proof that every
possible bug has been found.

## Intent and baseline

Vaayu preserves a personal Neovim workflow inside a compiled modal terminal
editor. The new review workflow adds private project notes and a common way to
inspect, select, search, copy and revisit outputs from editor commands.

The starting commit was `894a14e`, following `72d6c48`'s safety fixes and
`f91bef7`'s row-diff renderer. Uncommitted incremental syntax work in
`editor.rs` and `syntax.rs` was preserved and extended. The earlier audit's
findings cannot be treated as unchanged: quit checks, count bounds, control
sanitization and several revision fixes had already landed. The initial five
tests passed. A pre-expansion source snapshot was retained for release latency
comparison.

## Findings addressed

| Area | Failure or missing behavior | Result and evidence |
| --- | --- | --- |
| Persistence | Direct writes, external changes, failed save-as identity, EOF bytes | Atomic sibling replacement, saved-content comparison, refusal of conflicting ordinary saves, permission preservation and byte-exact empty/no-final-newline saves; filesystem regression tests |
| Buffer lifecycle | Dirty state after undo; hidden unsaved buffers and notes; save-all failures | Saved-text identity and revision updates; centralized all-buffer exit checks; failed writes stay open; dispatch tests |
| Editing | Empty inner objects, EOF ranges, Unicode join/case expansion, counted paste/insert, vertical desired column | Correct boundaries and undo transactions; tests for each path |
| Repeat and registers | Blackhole/append semantics, macro expansion re-recording, timed jk replay, visual shape repeat | Shape-aware registers and repeat, bounded macro replay, literal timed keys; tests including rectangular repeat from nonzero column |
| Search | Escaped substitution delimiters, invalid regex handling, multiline/Unicode anchors | Cached compiled patterns, explicit errors, replacement support and validated flags; regression tests |
| Display | Tabs/wide/combining text, wrap/nowrap scrolling, control bytes, tiny terminals | Cell-aware layout, control sanitization, cached row composition; render tests and real terminal assertions |
| Syntax | Stale incremental spans after deferred rebuild; atomic ancestor and deletion boundaries | Invalidate unsafe reuse, widen incremental ranges, compare incremental output with fresh parses; TypeScript/TSX grammars |
| Completion | Empty popup swallowed Enter, textEdit ignored, extra edits shifted caret | Authoritative validated edits and adjusted caret; completion tests |
| LSP | File URI/UTF-16 errors, stale or cross-buffer responses, incomplete configuration and lifecycle | Stable buffer/revision contexts, URI encoding, UTF-16 boundaries, per-root clients, configuration requests, queued document state and visible errors; mock-server integration |
| Workspace edits | Partial application or overlapping invalid edits | Validate all edits and stage unopened buffers before mutation; reject stale/overlapping/unsupported operations; transaction tests |
| Markdown | Code inside table cells, alignment, nested/loose lists, combined styles | Correct event handling, cell alignment and styles, highlighted fences; parser and PTY tests |
| Background work | Git/file discovery on interactive path, redundant wrap/layout rebuilding | Background jobs with stale-result rejection, cached layouts and semantic rows; release profiling and benchmark |

## Requested features implemented

### Private review comments

`notes.rs` stores editable line/range/file notes in `.vaayu/comments.json` under
the launch working directory. The directory is owner-only (0700), stored files
are 0600 on Unix, and a local `.gitignore` excludes contents. Source files are
not annotated. Corrupt/unknown-version stores and changed on-disk stores are
not silently overwritten; symlink destinations are refused on save.

`,rc` creates a line or Visual-range note; `,rf` creates a file note. The note
opens as an ordinary editable buffer with explicit Ctrl-S / `:w` saving. `,rl`
opens the list, with edit/delete, selection, source navigation and selected/all
clipboard export. `,rw` saves all open note buffers and pending list changes.
Text anchors relocate unique matches and flag lost anchors. Tests exercise
permissions, persistence, conflicts, corrupt data, multiple edits, relocation,
selective export and the real terminal workflow.

### Shared results and quickfix

`results.rs` supplies one result model for files, buffers, live grep, LSP
locations/symbols/diagnostics/actions, notes, recovery and Git output. Ctrl-Q
converts the current producer to a retained quickfix snapshot; selections limit
export. Completion and hover can also be exported. Lists support `/ ? n N`,
selection, paging, detail preview, clipboard copying and location navigation.
`:copen`, `:cn` and `:cp` retain a usable review loop after returning to a file.

The file inventory and debounced live grep run in background jobs. Grep ignores
superseded generations and limits output to 5,000 matches. File discovery uses
ripgrep's ignore rules. Marks and a bounded jumplist use stable buffer IDs;
diagnostic navigation works without opening a list.

### Language actions and configuration

Formatting, references, outline, symbol rename, signature help and code actions
use the same results/navigation machinery. Applied text edits remain dirty and
undoable. Named TOML servers support command argv, environment, root markers,
initialization options, settings and capability overrides, including multiple
servers for a language. `config.example.toml` translates representative options
from the local Neovim setup. `:configreload` restarts clients with new settings.

### Windows, preview and editing

Recursive mixed-orientation layouts retain independent positions for up to 32
panes; focus/close/only commands and side-by-side Markdown preview are available.
Soft wrap and horizontal scrolling share a Unicode-aware display layout.
Character, line and block Visual editing, literal bracketed paste and counted
insertion now have regression coverage.

### Recovery and Git review

Private asynchronous source-draft checkpoints support `:recover` after an
interrupted session. Restores are unsaved and undoable and refuse to overwrite
an already-dirty buffer. Normal exit removes the current journal.

`:gitdiff`, `:gitblame`, `:gitstage` and `:gitunstage` produce shared results.
Stage/unstage acts on saved-file hunks after `git apply --check`; a test stages
one of two separated hunks, verifies the index, then unstages it without
changing the working file. Git gutter refresh is asynchronous and periodically
checks HEAD.

## Performance assessment

The previous session correctly identified release builds and incremental
highlighting as essential. The expanded renderer initially introduced a new
layout/composition bottleneck. Profiling located repeated wrap layout and
unchanged-row composition; the fixes cache both and skip unchanged LSP root
work. Existing incremental syntax edits were retained with correctness guards.
See [BENCHMARKS.md](BENCHMARKS.md) and `bench/results-review.json` for the final
same-file release comparison. First-response PTY timing is a responsiveness
proxy, not completed-frame or physical display latency.

## Verification

- 77 regular Rust tests pass for each binary target (the same suite, not 154
  distinct tests). The real-clangd test is ignored in the regular run and passes
  explicitly for both targets when clangd is installed.
- Strict Clippy across all targets and rustfmt checks pass.
- Release binaries `vaayu` and `vy` build with the lockfile.
- `tests/mock_lsp.py` exercises real stdio framing, configuration, document
  synchronization, diagnostics and language requests in the Rust suite.
- `tests/pty_regression.py` drives a real PTY through private note creation,
  range anchors, save, clipboard export, selection/quickfix/search, split
  preview, wrap and grep. Clipboard tools are isolated test doubles.
- Git tests use temporary repositories; persistence tests use temporary files.
- CI runs formatting, tests, strict Clippy, release build and both PTY targets.

## Follow-up implementation — 2026-09-16

- OS file locks now cover the note read/check/replace transaction. Explicit unlock
  avoids transient inherited-descriptor locks during concurrent process launches.
- Comment recovery includes edited, not-yet-saved note buffers. Restores create
  a new note instead of replacing a newer saved note. Anchors also match unique
  whitespace-normalized source, and LSP file renames relocate note paths.
- Review notes have persistent resolved status. Versioned selected-feedback
  packets feed an explicitly configured agent command; output is retained and
  browsable through results/quickfix. Cancellation, timeout and output limits
  bound the job. Tests use a local deterministic command, not a paid AI service.
- LSP initialization and requests have deadlines; stale requests and explicit
  `:lspcancel` send cancellation. Completion resolution is guarded by popup,
  selection, mode and buffer revision. Common snippet defaults, choices,
  variables, tab stops and linked fields are supported with visible selection.
- Ordered LSP regular-file create/rename/delete operations preflight paths,
  dirty buffers and disk snapshots, then commit with rollback on failure.
  Text edits stay unsaved; resource changes happen on disk. Renames preserve
  file permissions and buffer identity. Private-store paths are excluded.
- Mixed recursive splits (up to 32) replace the flat four-pane layout; focus uses
  geometry. Named-file pane layout and positions persist in a private session.
- Horizontal motion, deletion and inclusive operators respect graphemes.
  Rectangular edits, repeats, paste and selection use display columns across
  tabs and wide-character prefixes; affected block lines expand tabs to spaces.
- Search/substitution/results support pattern backreferences and lookaround,
  magic switches and case overrides. Backtracking is bounded; substitution
  computes the full replacement plan before mutating a buffer.
- Insert-mode entry no longer builds a whole-source completion index. The lazy
  word index covers a bounded 401-line neighborhood instead of allocating or
  scanning every source line. LSP completions remain available independently.

Validation includes the expanded Rust suite, real clangd formatting, original
PTY workflows and a new PTY matrix at 40×12, 100×24 and 180×50 on 50,001 Unicode
lines, including nested layouts, session restore and terminal resize. See
[BENCHMARKS.md](BENCHMARKS.md) for final release measurements. The real SSH harness
in `bench/ssh_latency.py` requires an explicitly supplied authenticated host;
no real remote measurement is claimed without one.

Against retained release `05ed934`, the final same-run, same-file 236-operation
PTY comparison measured 0.675 → 0.653 ms median, 6.643 → 5.851 ms p99,
6.681 → 2.517 ms to enter Insert and 1.506 → 0.648 ms to leave it. P90 and
maximum moved higher, and there were no timeouts. These are first-output-byte
measurements.

The final UI-specific PTY suite also checks visual reverse-video selection,
mode transitions, private-note editing and resolution, quickfix conversion,
mixed split separators, side-by-side Markdown preview and resize at 40×12,
100×24 and 180×50. Captured frames were inspected after the assertions passed.

The cross-editor run does not support an “always faster” claim. Vaayu led this
machine's startup, search submission, picker interaction and live-grep input;
bare Neovim led page scrolling and next-match navigation, while Neovim was also
faster for large-file character insertion. Warm clangd formatting was tied at
the harness's resolution: Vaayu 22.754 ms, minimal Neovim 22.824 ms and Helix
22.740 ms. The raw measurements and exact configurations are in
[BENCHMARKS.md](BENCHMARKS.md).

## Remaining scope boundaries

- Snippet regex transforms and a choice dropdown are not implemented; choices
  insert their first value and can be edited. Linked values update on leaving
  a placeholder. This remains a Vim/LSP subset, not full Vim or snippet-engine
  compatibility.
- File resource operations cover regular files inside the launch project;
  directory trees and symlink resources are rejected. Multi-file rollback is
  best effort on filesystem failure, not a crash-atomic filesystem transaction.
- Sessions persist named-file panes and positions, not unsaved source contents
  or a complete process image. Recovery and explicit save handle drafts.
- Comment anchors remain heuristics across arbitrary semantic rewrites; lost
  or ambiguous matches require review. Notes and agent packets are local
  plaintext protected by permissions, not encrypted.
- Configured agent commands run with the user's permissions. Vaayu supplies
  feedback and records output; the chosen agent supplies model access and its
  own execution restrictions. Resolution remains a human action.
- Common editing paths respect graphemes; complete Vim word/text-object and
  virtual-cursor behavior across every Unicode sequence remains broader work.
- Actual GitHub push and real SSH measurement depend on available credentials
  and an authenticated target; missing external access is reported explicitly.
