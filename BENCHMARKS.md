# Latency benchmarks

## Re-audit release comparison — 2026-09-15

The retained pre-expansion workspace (commit `894a14e` plus the incremental
syntax edits already present) and final implementation were built in release
mode. Both edited the same retained `src/normal.rs` through the same harness at
100×40. The recorded run had no concurrent build or test workload.

| PTY first-response measurement | Before | After |
| --- | ---: | ---: |
| Overall median | 0.834 ms | 0.598 ms |
| p90 | 3.805 ms | 2.935 ms |
| p99 | 5.578 ms | 5.144 ms |
| Down-motion median | 0.673 ms | 0.548 ms |
| Word-motion median | 0.861 ms | 0.558 ms |
| Insert-character median | 3.774 ms | 2.848 ms |
| Timeouts | 0 | 0 |

236 measured responses per binary. Overall median was about 28% lower in this
run. This is one local workload, not a general speed guarantee. The single
enter-Insert sample regressed (1.711 → 5.909 ms), as did leaving Insert
(0.887 → 1.650 ms); mode-transition work deserves further profiling. Maximum
latency was 5.732 → 5.909 ms. See all categories in
[results-review.json](bench/results-review.json).

The harness measures key-to-first-output-byte over a PTY. It does not measure
frame completion, terminal paint, display latency or SSH behavior. Compare
release builds using the same input file, terminal dimensions and settings;
changing the benchmark source alongside the implementation confounds results.

```sh
python3 bench/latency.py --file /path/to/retained/source.rs --cols 100 --rows 40 \
  before:/path/to/baseline/release/vaayu \
  after:/path/to/current/release/vaayu --out bench/results-review.json
```

The expanded Unicode/wrap/split renderer initially rebuilt layout and composed
unchanged rows on each motion. Stage profiling located that cost. Per-line
layout caches and semantic row signatures now reuse unchanged work; the final
row-byte cache suppresses redundant terminal writes. LSP root selection skips
unchanged buffer contexts, and Git/file discovery run in background jobs.
Incremental tree-sitter changes from the prior session were retained with
correctness guards and differential tests against fresh parsing.

The historical investigation below explains the original debug/release
benchmark error and earlier optimizations; its numbers are separate runs.

## Earlier performance investigation

Measured with `bench/latency.py`: wall-clock time from writing a key to a
pty to the first byte of the editor's response arriving (a first-response proxy, not completed-frame latency), over a scripted
session (100x `j`, 50x `w`, an insert-mode typing burst, undo/redo, and a
search), replayed against each editor on the same 100x40 terminal editing
the same ~900-line Rust source file.

```sh
python3 bench/latency.py \
  "vaayu:/path/to/vy" \
  "nvim-bare:nvim -u NONE --cmd set noswapfile" \
  --cols 100 --rows 40 --out bench/results.json
```

Run against neovim's *bare* config (`-u NONE`, no plugins) to isolate editor
core responsiveness from plugin overhead; comparing against this machine's
actual ~40-plugin nvim config, and against Helix, is next (Helix isn't
installed here -- `sudo pacman -S helix` needs a password this session
doesn't have).

**Always benchmark the release binary.** The very first pass of this work
compared vaayu's *debug* build against neovim and concluded vaayu was
several times slower across the board. That comparison was invalid: once
instrumented (below), the debug build turned out to be **~29x slower than
release** on the same operation (6.6ms vs 0.23ms median for a plain cursor
move) -- an artifact of zero optimization, not a real architectural
problem. All numbers below are release-build vs release-build nvim.

## Instrumentation

`VAAYU_PROFILE=/path/to/log vy file` (see `src/profile.rs`) records
cumulative per-stage timing for every frame: terminal I/O, each `ensure_*`
call, and a breakdown of the row-rendering loop. This is what found both
real fixes below -- not re-benchmarking after each guess.

## What was fixed, in the order found

1. **Full-screen clear + repaint on every keystroke.** `draw()` cleared and
   redrew the entire terminal on every single keystroke, even a plain
   cursor move -- the "damage-tracked rendering" the original design doc
   named as the key differentiator, never actually built. Replaced with a
   `FrameCache`: each row's content is built into a buffer, compared
   against what was actually written last frame, and only rewritten if it
   changed (verified by inspecting the raw output stream: 3x `j` now
   touches 4 rows and 358 bytes instead of the full ~4000-cell grid).
2. **O(all spans in the file) linear scan, once per visible row, every
   frame**, to look up a line's syntax highlights. Spans are sorted by
   start position; replaced the scan with a binary search bounded by the
   longest span in the file, so a lookup costs O(log n + k) instead of
   O(n).
3. **The real dominant cost, found by instrumenting *inside* the row loop**
   after (2) barely moved the numbers: rebuilding the full syntax
   highlight span list -- a complete re-walk of the tree-sitter tree from
   its root, not incremental like parsing itself -- ran on every single
   keystroke, ~8ms/keystroke on this file during insert-mode typing,
   because a prior correctness fix (this session's earlier crash-fix pass)
   made every keystroke bump `edit_seq`, and every `edit_seq` change
   triggered a full reparse-and-rewalk. The tree-sitter *parse* is already
   incremental (an `InputEdit` is computed and applied before reparsing);
   only the span-list rebuild wasn't. Throttled the rebuild to at most
   once per 30ms, with a documented trade made explicit in the code: a
   render against momentarily-stale spans is safe (the UTF-8 boundary
   clamp added during the crash fix exists for exactly this case), so a
   few tens of milliseconds of highlighting lag during a fast typing burst
   is the cost, not a correctness or crash risk.
4. **Fixed a real bug the throttle introduced**: if the user stops typing
   before a deferred rebuild's throttle window elapses, nothing was left
   to finish it -- the main loop's idle wait only redraws on a new key or
   an LSP event, so highlighting could stay stale indefinitely after a
   typing burst rather than catching up once idle. Added a third idle-wake
   condition that polls whether a rebuild is due and finishes it.
   Confirmed via raw byte inspection of a full session capture (not a
   partial delta, which is what produced a false "colors never appeared"
   reading on the first check) that highlighting does correctly catch up.

## Results (2026-09-15, release build)

| | p50 | p90 | p99 |
|---|---|---|---|
| vaayu | 0.20 ms | 2.54 ms | 2.83 ms |
| nvim-bare | 0.16 ms | 0.33 ms | 10.24 ms |

By operation (p50, ms) -- vaayu vs nvim-bare:

| operation | vaayu | nvim-bare |
|---|---|---|
| move_down | 0.141 | 0.149 |
| word_fwd | 0.168 | 0.155 |
| undo / redo | 0.204 / 0.224 | 0.135 / 0.125 |
| search_next | 0.424 | 0.147 |
| **insert_char** | **2.496** | **0.304** |

vaayu is now competitive with (and on plain motion, marginally faster than)
bare neovim -- a real result, not a rounding error, confirmed by the same
harness both before and after. **insert-mode character typing remains ~8x
slower** and is what drags the p90/p99 up; it's the one operation still
dominated by the throttled-but-still-firing syntax rebuild, since typing at
the benchmark's pace triggers a rebuild on most keystrokes rather than
collapsing a burst into one.

## What's next

The throttle caps *how often* the expensive full-tree rebuild happens, but
doesn't make the rebuild itself cheap, so a sustained fast-typing session
still pays it often. The correct fix is incremental span updates using
tree-sitter's `Tree::changed_ranges()` against the previous tree -- reuse
spans outside the changed region, re-walk only the affected subtree(s),
shift byte offsets for spans after the edit point. Not done here: getting
the offset-shifting and boundary cases right needs its own focused pass
rather than being rushed at the end of this one.
