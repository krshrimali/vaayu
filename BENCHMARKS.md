# Latency benchmarks

Measured with `bench/latency.py`: wall-clock time from writing a key to a
pty to the first byte of the editor's response arriving (the standard
definition of input latency, not "time until fully quiet"), over a scripted
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

## 2026-09-15, commit-by-commit

### Before row-diffed rendering (full-screen clear + repaint every keystroke)

| | p50 | p90 | p99 |
|---|---|---|---|
| vaayu | 3.5 ms | 26.6 ms | 32.1 ms |
| nvim-bare | 0.66 ms | 0.99 ms | 1.35 ms |

`move_down` alone: vaayu 3.05 ms vs nvim-bare 0.63 ms. `insert_char`: vaayu
21.2 ms vs nvim-bare 0.97 ms.

### After row-diffed rendering (only changed rows rewritten)

| | p50 | p90 | p99 |
|---|---|---|---|
| vaayu | ~3.3 ms* | 13.7 ms | 31.5 ms |
| nvim-bare | 0.40 ms | 0.87 ms | 10.9 ms |

`move_down`: 3.2 ms. `insert_char`: 13.5 ms (down from 21.2 ms).
`word_fwd`: 6.3 ms.

*p50 not captured in the terminal scrollback for this run; by-label medians
above are from the same run's raw JSON.

## What the fix actually did, and what it didn't

Confirmed structurally correct and a real improvement, not just a number
that moved: after 3x `j`, vaayu now writes 358 bytes touching 4 screen rows
(the old and new cursor-line rows, the status line, the message line)
instead of clearing and repainting the full ~4000-cell grid every time --
inspected the raw byte stream directly, not inferred from the timing alone.
`insert_char` dropped by roughly a third.

It did **not** close the gap with neovim. vaayu's `move_down` p50 barely
moved (3.05 ms -> 3.2 ms) despite touching far fewer cells, which means
byte count was never the dominant cost for the simplest possible operation
-- something else in the per-keystroke path is spending the time. Not yet
diagnosed; candidates, in rough suspected order:

1. `crossterm::terminal::size()` -- an ioctl syscall -- runs unconditionally
   every frame in the main loop, not just on actual resize.
2. Per-frame allocations on the hot path: `format!()` for the gutter number
   on every changed row, `Vec<u8>` allocation per row even when most rows
   are unchanged and get skipped, `HashMap` construction for
   `diag_by_line` every frame regardless of whether diagnostics changed.
3. `ensure_syntax`/`ensure_git`/`sync_lsp` all run their (cheap-looking)
   staleness checks every frame; "cheap-looking" hasn't been measured.
4. The benchmark harness itself: a Python `pty.fork()` + `select()` loop
   crossing two process boundaries per sample. This should apply equal
   overhead to both editors being measured in the same run, but hasn't been
   validated against an independent measurement method.

Next step is instrumentation, not another guess: time-stamp entry/exit of
`run()`'s loop body for a batch of frames and print where the milliseconds
actually go, rather than fixing the next plausible-looking thing.
