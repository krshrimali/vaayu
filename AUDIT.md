# Vaayu re-audit and implementation report

Reviewed 2026-09-15. This supersedes the earlier audit of the five-test editor.
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

Up to four panes retain independent positions; vertical/horizontal splits,
focus/close/only commands and side-by-side Markdown preview are available.
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

- 62 Rust tests pass for each binary target (the same suite, not 124 distinct tests).
- Strict Clippy across all targets and rustfmt checks pass.
- Release binaries `vaayu` and `vy` build with the lockfile.
- `tests/mock_lsp.py` exercises real stdio framing, configuration, document
  synchronization, diagnostics and language requests in the Rust suite.
- `tests/pty_regression.py` drives a real PTY through private note creation,
  range anchors, save, clipboard export, selection/quickfix/search, split
  preview, wrap and grep. Clipboard tools are isolated test doubles.
- Git tests use temporary repositories; persistence tests use temporary files.
- CI runs formatting, tests, strict Clippy, release build and both PTY targets.

## Remaining boundaries and next work

These are explicit limitations, not claims of implemented behavior:

- Splits use one flat orientation at a time, with four panes; recursive layouts
  and persistent sessions are future work.
- Display respects graphemes, but editing and block coordinates use Unicode
  scalar columns, not virtual tab cells or full grapheme motions.
- Search implements a Vim-like subset. Pattern backreferences/lookarounds and
  the complete Vim regex dialect are unsupported; invalid expressions report
  errors. Replacement references are supported.
- LSP text edits and symbol rename work; file create/rename/delete resource
  operations are rejected. Snippet expansion, completion resolution and
  request cancellation/timeouts remain future work. Pending requests are
  bounded; a stuck server can be restarted with `:lsprestart`.
- TOML configuration does not execute Neovim Lua callbacks or plugins.
- Private notes are local plaintext protected by Unix permissions, not
  encryption. Concurrent-save checking detects changed snapshots but is not a
  cross-process transactional database. Anchors are text heuristics, not AST
  identities across arbitrary refactors.
- Recovery covers named source buffers inside the launch folder, with a delay;
  unsaved new scratch comments require explicit save. Dead-session detection
  currently uses Linux `/proc`.
- Git hunk actions cover saved tracked-file diffs, not a full interactive Git
  client. Live grep deliberately caps results.
- The terminal harness validates specific dimensions and workflows; broader
  terminal compatibility, large-file soak testing, real-server interoperability
  and remote latency testing should expand over time.

Further priorities: cancellation and timeouts, completion snippets, stronger
cross-session note locking, recursive split/session persistence, grapheme-aware
editing and measured SSH behavior. Agentic review can consume the versioned
note store or exported selections; automated agent execution is not included.
