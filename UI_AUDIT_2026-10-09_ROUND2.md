# Extended UI audit — 2026-10-09

The second audit extended the [first 1,000-operation audit](UI_AUDIT_2026-10-09.md) after the user removed its operation limit. **30,263 recorded UI operations** were executed in this pass, starting from `7baf0e49cbfa3bfd5da3de2941feace355665ff5`. **Twenty defect categories were fixed.** The final 960 deterministic editing cases produce the same saved text as Vim; two cursor-only redo differences remain documented below.

## Plan and method

The plan preceded execution: run all previously untested PTY feature suites at their original terminal sizes; compare deterministic editing combinations with Vim; verify authenticated read-only GitHub operations and the real desktop clipboard; exercise crash recovery and delayed/disconnected language servers; reproduce findings, fix them, add regressions, and commit the results.

An operation is one deliberately submitted keyboard or mouse sequence, often a complete Vim command. Individual inserted characters are not counted separately. The ledger has consecutive, unique IDs. This pass contains 28,288 Vaayu operations, 1,827 `vy` operations, and 148 interactive Vim reference operations. There are 30,148 rendered terminal snapshots and 212 distinct recorded driver names, including reproduction and verification variants. The first audit's 1,000 operations are separate from these counts.

All editor projects, configuration, Git repositories, and language-server fixtures were temporary. Real GitHub requests used the existing authenticated CLI against the public `cli/cli` repository. The real clipboard was backed up privately and restored byte for byte in all seven original formats; its original payloads and authentication credentials are excluded from the evidence archive.

The 171 previously untested feature suites passed at all their original dimensions after correcting a runner name-mangling error in the file-tree follow-up suite. These cover language features, folds, filesystem operations, watchers and conflicts, persistence, tasks, tools, themes, previews, tours, Git, GitHub and terminal behavior. Affected editing, mouse, multicursor, window navigation, undo and GitHub suites were repeated after the implementation changes.

The comparison sets contain 360 initial cases and 600 additional cases, using fixed seeds, counts, empty lines, tabs, Unicode, unequal line lengths, registers, visual selections, yanks, puts, changes, deletes, joins, dot repeat, undo and redo. Vim configuration was aligned with Vaayu: `swap_0_and_caret=false`, automatic indentation, and four-column tabs. Registers were seeded consistently and Vim persistence was disabled. References use `feedkeys()` rather than `:normal!`, which can abort the remainder of a combined command after a failed motion. Every remaining initial discrepancy and all 32 provisional expanded discrepancies were also checked in interactive Vim. The expanded set was replayed on the immutable original binary to establish the before/after behavior independently: 85 of 600 cases differed, including 77 text differences. The fixed binary has zero text differences and two cursor-only differences in that set.

## Confirmed defects and fixes

Each row describes one defect category; related failures caused by the same behavior are grouped. Commands below assume ordinary Vim `0`/`^` mappings. `Ctrl-V` enters Visual Block mode.

| ID | Defect and reproduction | Fixed behavior |
| --- | --- | --- |
| R1 | `w` skipped empty lines. From `a`, `w` in `a\n\n  b c` landed on `b`, so subsequent edits affected the wrong line. | Empty lines count as words for forward and backward word motions. |
| R2 | `dw`/`yw` on the last word of a line consumed the newline and following indentation. `dw` on an empty line stripped indentation from the next line. | The final operator word motion stops at the line end; an empty line is removed linewise. |
| R3 | `dk` at the first line and `dj` at the last line operated on the current line despite the failed motion. | A stationary vertical motion cancels the operator and preserves text, registers and history. |
| R4 | `2dd`/`2yy` on the final single remaining line silently clamped the count and deleted/yanked it. | A doubled line operator with an unavailable count leaves the buffer and registers unchanged. |
| R5 | `jcbQ<Esc>` from column zero joined the previous and current lines. | Backward exclusive ranges apply the column-zero endpoint rule and retain the line break. |
| R6 | `2de` across complete lines left an extra empty line. For `a\n\n  b c\nD e f`, it left `\nD e f`. | A normal delete covering all text on several lines removes those lines and their final newline. Explicit characterwise Visual selections retain their span. |
| R7 | `j$vlcQ<Esc>` could not include the newline in a characterwise selection, preventing the expected join. | Visual movement permits the end-of-line position and characterwise operations include an explicitly selected interior newline. |
| R8 | `$`, Ctrl-V, `j`, `l`, then `d`/`c` selected incorrect columns when adjacent lines had different lengths. | Visual vertical movement retains the desired column and sticky `$`; a later horizontal movement updates it. |
| R9 | Multiline characterwise `p` landed on the last inserted line. Example: `$y2wp`. | The cursor lands at the start of the inserted text; single-line puts retain their existing behavior. |
| R10 | `J` or `3J` at EOF moved the cursor despite making no edit. | Failed joins preserve the cursor and edit history. |
| R11 | Joining an empty next line inserted an unnecessary space. `3J` through an empty line also landed at the wrong join column. | Joining an empty line adds no space and subsequent joins use the correct position. |
| R12 | `dd"1p` on a one-line file replaced the logical empty line instead of putting below it. `ddP` similarly omitted the retained empty line. | Linewise puts preserve the empty line and land on the inserted line. |
| R13 | `cw` on a word's final character advanced to the next word and changed it too. Example: `$cwQ<Esc>` before an indented next line. | `cw` includes the current word's end even when the cursor already sits there. |
| R14 | `cb` at the start of the file entered Insert mode despite failing to move. | Failed backward/left change motions cancel; valid empty-range changes such as `c0` still enter Insert mode. |
| R15 | Vertical movement treated character indices as screen columns. From a leading tab, `j2x` deleted indentation instead of text at the corresponding column. Short lines also lost the desired column. | Normal, Visual and vertical multicursor movement track display cells, including tabs and wide characters, across short lines. |
| R16 | Block yanks padded short or empty source rows, adding unwanted spaces when pasted. Empty final block rows were also lost by `lines()`. | Block registers retain actual row text and empty rows; paste adds only the padding required to reach its destination. |
| R17 | Block delete/change/put expanded tabs across the whole line, including tabs outside the selection. | Whole tabs outside the edited region remain tabs. Only partially intersected tabs split into spaces; whole selected tabs can be yanked and pasted intact. |
| R18 | `ciw` on an empty line did nothing; `caw`/`daw` there failed to select the following word. | Inner-word change enters Insert without overwriting the previous register; around-word includes the following word. |
| R19 | `daw` on whitespace removed only the gap. In `one two`, selecting the separating space left `onetwo`. | Around-word on whitespace includes the next word. |
| R20 | Format-on-save timed out and saved unformatted text, then applied the delayed formatting reply to the buffer. | The save cancels its unfinished formatting request and removes its response context before writing. Late replies cannot modify the saved buffer. |

R3, R5, R13 and R20 can alter or remove text that the user did not intend to change. R16 and R17 can introduce unwanted whitespace or broad tab-to-space changes. Cursor-only defects have lower impact but affect subsequent commands.

During verification, tests and final review also caught implementation regressions before commit: an interior newline selection initially included the final file newline; rejecting all stationary exclusive changes initially rejected valid `c0`; an insertion inside a wide character initially consumed that character; and preserved tab prefixes exposed an old character-column assumption in block-change replication. The final regressions explicitly protect these cases, including undo.

## Integration and regression results

- All 960 final comparison cases agree with Vim on saved text. The initial 360 also agree on cursor position; the expanded 600 contain the two remaining cursor differences below.
- The new `pty_vim_boundaries.py` checks 30 cases at 40×12, 100×24 and 180×50: saved contents, cursor positions and undo. **270 assertions pass for each binary.**
- `pty_async_safety.py` tests edits before a delayed formatter reply, buffer switching before a definition reply, disconnect/restart, and a format-on-save timeout. **54 assertions pass for each binary.** The timeout failure was separately reproduced on the original binary.
- `pty_crash_recovery.py` performs an actual SIGKILL with two unsaved Unicode buffers, verifies private snapshots and unchanged source files, restarts the editor, restores drafts, undoes a restore, protects newer edits, and explicitly saves a recovered draft. **39 assertions pass for each binary.**
- Real Wayland tests passed at three sizes: unnamed and explicit `+`/`*` yanks, external Unicode and multiline pastes, named-register isolation, and black-hole deletes. All seven original clipboard formats were restored and verified.
- Authenticated read-only GitHub tests passed on both the original and fixed binary: open/closed/merged PR lists, PR details, diff, review threads, checks, open/closed issue lists, and selected issue details. Public PR #14620 was used. Checkout, review submission, browser launches, failure paths and races use isolated fixtures from the first audit and repeated GitHub edge tests; no remote writes were made.
- `cargo fmt --all -- --check`, `cargo test --locked`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo build --bins --locked` pass. **855 Rust tests pass for each binary; three are ignored.** Sixteen new Rust regressions were added. Checks ran in a clean source checkout with `NO_COLOR` unset to avoid user ShaDa state affecting tests.
- The three new PTY regressions are wired into CI. Existing multicursor, mouse, window navigation, editing, formatting, persistence, terminal-theme and GitHub edge suites also pass after the changes.

## Remaining findings and limits

**Redo cursor compatibility:** after `dw`, undo and Ctrl-R remove an empty line before an indented line, Vaayu restores the original post-delete cursor at the first nonblank; Vim's redo lands at column zero. Expanded cases 103 and 113 reproduce this. Text, undo and redo contents agree. Vaayu currently restores saved cursor state for redo; changing that policy requires tracking Vim's change-start position across all edit types rather than a special case for these examples. This pass leaves the two cursor differences visible in the comparison evidence.

**Missing `%` bracket matching:** the compatibility gap from the first audit remains. `%` has no motion binding; it is a missing Vim feature rather than a regression in a documented operation.

**Real clangd is unavailable on this machine:** the explicit interoperability test failed because the installed `clangd` cannot load `libLLVM.so.22.1`. Running `clangd --version` directly fails with the same loader error. Mock-server happy paths, delayed replies, disconnect/restart and cancellation passed; this audit cannot claim successful real-clangd interoperability on the current installation.

Testing used real PTYs and rendered terminal models, not a new manual Kitty visual session. Real GitHub coverage is authenticated and read-only on a public repository. Remote mutations, SSH/tmux clipboard behavior, every possible Unicode cluster and arbitrary timing combinations are not exhaustively covered. The report records all confirmed findings from this pass and does not claim that no other bugs exist.

## Evidence and reproduction

[Evidence archive](artifacts/ui-audit-2026-10-09-round2.tar.gz) contains the consecutive operation ledger, terminal snapshots and cursor metadata, suite results, fixed-seed comparison inputs/results, interactive Vim checks, live integration summaries, test drivers, implementation patch, validation metadata and source hashes. Its [SHA-256 checksum](artifacts/ui-audit-2026-10-09-round2.tar.gz.sha256) verifies the archive. Private clipboard backup files, credentials, binaries and unrelated workspace files are excluded.

The raw evidence distinguishes intentional failures on the original binary from passing final regressions. Harness errors encountered during development are also retained: an initial missing executable bit, AST name mangling, incompatible Vim oracle state, combined legacy Esc/Alt encoding, a one-step undo assumption for a two-edit command, and the clipboard restore helper's handling of legacy aliases. These were corrected and rerun. Startup attempts that never launched an editor are excluded from the operation ledger. The native restore helper ultimately verified all original clipboard formats exactly.

Run the committed regressions with the packages in `tests/requirements.txt` installed:

```sh
cargo build --bins --locked
python3 tests/pty_vim_boundaries.py target/debug/vaayu
python3 tests/pty_async_safety.py target/debug/vaayu
python3 tests/pty_crash_recovery.py target/debug/vaayu
```

Repeat with `target/debug/vy` to exercise the second binary. The archived audit drivers additionally reproduce the broad feature run, comparison sets and guarded live integration tests.
