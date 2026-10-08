# Latency benchmarks

## Current refresh — 2026-10-08

The application baseline is `4f1b81d`; the release was rebuilt with the updated embedded [HELP.md](HELP.md), without Rust behavior changes. The same-machine reference is `e3a1b3a`, the last broad README/performance refresh (its application code matches the previously benchmarked `91e9145`). Every commit between these baselines is covered in [CHANGELOG.md](CHANGELOG.md). Binary and harness SHA-256 hashes, exact argv, versions and environment are in [metadata.json](bench/2026-10-08/metadata.json); all core runs are preserved in [the aggregate](bench/editor-comparison-2026-10-08.json). Historical results below retain their original dates and machines.

### Method and evidence

This run used an Intel Core i9-12900K, 24 logical CPUs available to the process, Linux `7.2.8-1-cachyos`, and Rust `1.99.0`. Editors were Vaayu `0.1.0`, official Neovim `0.12.5` with `-u NONE`, and official Helix `25.07.1` with its matching runtime. No personal Neovim plugin configuration was measured. All workloads used a 100×40 PTY; large-buffer comparisons used 50,000 generated Rust lines. Three sequential core runs and three sequential baseline runs used the same archived project inventory (319 files), temporary XDG directories, no persisted `.vaayu` state, Rust LSP disabled, `NO_COLOR` unset, and no concurrent builds or test suites. [Harness](bench/editor_compare.py), [individual runs 1](bench/2026-10-08/editors-1.json), [2](bench/2026-10-08/editors-2.json), [3](bench/2026-10-08/editors-3.json), and [baseline runs 1](bench/2026-10-08/baseline-large-1.json), [2](bench/2026-10-08/baseline-large-2.json), [3](bench/2026-10-08/baseline-large-3.json) provide the evidence.

Unless specified otherwise, a table value is the median of the three runs' reported statistic: p50 is the median of run medians, p95 is the median of run p95s. These are not pooled percentiles or confidence intervals. Individual action sample counts range from one (mode transitions/picker opening) to 80 (down motion); a single-action p95 equals that action's sample. Startup is measured separately. Timings measure terminal output, never physical terminal painting. [Harness summaries and sample counts](bench/editor_compare.py) document that distinction.

The benchmark now sends two separately timed Esc presses to leave picker query Insert mode and then dismiss the picker before live grep. The previous harness could leave the picker active when the grep command was sent. Historical picker-close/grep labels therefore have an unverified UI-state assumption; their recorded numbers remain below for provenance and are not used for current speed claims. The repaired two-Esc sequence is applied identically to current and baseline Vaayu releases; Helix closes with one measured Esc. [Fixture reconciliation](DEVELOPMENT.md#corrected-fixture-assumptions) explains the correction.

### Large-buffer editor comparison

| Process measurement | Vaayu | Neovim bare | Helix |
| --- | ---: | ---: | ---: |
| Startup first output: median (ms) | 7.190 | 9.191 | 397.957 |
| Startup first output: min–max (ms) | 6.288–8.094 | 8.870–14.009 | 382.024–398.524 |
| Idle RSS: median (MiB) | 63.176 | 9.617 | 121.426 |

| Operation (ms) | Vaayu | Neovim bare | Helix |
| --- | ---: | ---: | ---: |
| Cursor down | 1.001 | 0.504 | 3.178 |
| Page scroll | 0.654 | 0.484 | 3.165 |
| Enter Insert | 0.567 | 0.537 | 3.004 |
| Insert character | 0.794 | 0.887 | 177.195 |
| Leave Insert | 0.811 | 50.805 | 0.754 |
| Submit buffer search | 0.319 | 3.404 | 2.135 |
| Next search match | 2.501 | 0.507 | 4.404 |
| Open project picker | 0.559 | n/a | 3.600 |
| Filter project picker | 0.901 | n/a | 3.851 |
| Close project picker | 0.707 | n/a | 2.945 |
| Open live grep | 1.083 | n/a | 3.637 |
| Filter live grep | 0.483 | n/a | 3.011 |

All measured core and baseline operations had zero timeouts. Bare Neovim reached the first byte earlier for down/page motion and distant next-match navigation; Vaayu responded earlier for search submission and Insert exit. Neovim's roughly 51 ms Esc result includes the bare editor's default terminal-key timeout. Character insertion differs by only 0.093 ms between Vaayu and Neovim in these samples, which does not establish a useful speed distinction. Helix's roughly 177 ms large-buffer insertion is specific to this file/configuration. [All run statistics](bench/editor-comparison-2026-10-08.json) include per-operation byte counts and tails.

The 12 ms quiet-window proxy includes an intentional wait and can finish before later asynchronous work. The following medians show it separately; they do not measure a fully painted frame or completed project search. [Implementation](bench/editor_compare.py) and [raw data](bench/editor-comparison-2026-10-08.json) define the window.

| Operation quiet proxy (ms) | Baseline | Current | Neovim bare | Helix |
| --- | ---: | ---: | ---: | ---: |
| Cursor down | 13.234 | 13.152 | 13.010 | 15.361 |
| Page scroll | 13.545 | 13.481 | 13.220 | 15.382 |
| Enter Insert | 12.773 | 12.762 | 13.073 | 15.176 |
| Insert character | 13.025 | 12.920 | 13.049 | 190.071 |
| Leave Insert | 13.000 | 12.983 | 63.045 | 12.947 |
| Submit buffer search | 27.643 | 37.962 | 19.664 | 28.032 |
| Next search match | 14.555 | 14.748 | 13.890 | 16.610 |
| Open project picker | 13.077 | 12.907 | n/a | 28.065 |
| Filter project picker | 12.829 | 13.182 | n/a | 16.132 |
| Close project picker | 23.599 | 23.892 | n/a | 15.783 |
| Open live grep | 23.516 | 23.988 | n/a | 22.666 |
| Filter live grep | 12.611 | 12.595 | n/a | 15.161 |

Vaayu's separate post-query grep settle window observed further output at a median 8.626 ms and ended at 265.146 ms; baseline observed it at 50.194 ms and ended at 262.663 ms. These timers begin after all per-key transactions and include the settle window's 250 ms minimum, so they are not query-to-result latency. Helix had no additional grep output in those windows (252.360 ms median wait). Vaayu and baseline had no delayed picker-inventory output (126.277 and 125.987 ms median waits, with a 125 ms minimum). Helix produced delayed output in 0 of three inventory windows. `null` first-response values with zero timeouts mean no additional output was observed in an optional settle window. [Raw current](bench/2026-10-08/editors-1.json) and [aggregate windows](bench/editor-comparison-2026-10-08.json) retain every sample.

### Same-machine changes since the documentation baseline

| Operation (ms) | Baseline p50 | Current p50 | Baseline p95 | Current p95 |
| --- | ---: | ---: | ---: | ---: |
| Cursor down | 1.020 | 1.001 | 1.266 | 1.127 |
| Page scroll | 0.649 | 0.654 | 0.839 | 0.782 |
| Enter Insert | 0.637 | 0.567 | 0.637 | 0.567 |
| Insert character | 0.848 | 0.794 | 1.179 | 0.999 |
| Leave Insert | 0.814 | 0.811 | 0.814 | 0.811 |
| Submit buffer search | 0.252 | 0.319 | 0.252 | 0.319 |
| Next search match | 2.393 | 2.501 | 2.543 | 2.615 |
| Open project picker | 0.870 | 0.559 | 0.870 | 0.559 |
| Filter project picker | 0.660 | 0.901 | 0.881 | 1.068 |
| Close project picker | 0.511 | 0.707 | 0.511 | 0.707 |
| Open live grep | 0.944 | 1.083 | 0.944 | 1.083 |
| Filter live grep | 0.414 | 0.483 | 0.542 | 0.630 |

Large-buffer insertion moved from 0.848 to 0.794 ms (-6.4%); opening live grep from 0.944 to 1.083 ms (+14.7%). Median startup moved from 8.049 to 7.190 ms; the startup ranges were 6.169–8.858 ms baseline and 6.288–8.094 ms current. Idle RSS moved from 60.117 to 63.176 MiB (+5.1%). The release binary grew from 17,066,144 bytes (16.276 MiB) to 38,397,928 bytes (36.619 MiB), +125.0%. Every operation's p50 and p95, including improvements and regressions, is retained in the table above. These are observations from this run; they do not isolate a causal feature or establish statistical significance. [Baseline/current results](bench/editor-comparison-2026-10-08.json) and [binary provenance](bench/2026-10-08/metadata.json) support the comparison.

### Fixed-source editing workload

The [legacy harness](bench/latency.py) ran 236 measured responses per editor, three times, against the exact same 1,622-line `src/normal.rs` from `4f1b81d`. Current/baseline order was reversed in run two. Unlike the large-file harness, this harness uses `time.time()` and drains the remaining burst for 80 ms outside the measured first-response time; compare results within this workload. All nine editor sessions had zero timeouts. [Runs 1](bench/2026-10-08/editing-1.json), [2](bench/2026-10-08/editing-2.json), [3](bench/2026-10-08/editing-3.json), and [source hash](bench/2026-10-08/metadata.json) preserve the evidence.

| Editing statistic (ms) | Baseline | Current | Neovim bare |
| --- | ---: | ---: | ---: |
| Overall p50 | 1.057 | 1.105 | 0.559 |
| Overall p90 | 2.960 | 2.976 | 0.908 |
| Overall p99 | 8.713 | 9.228 | 1.178 |
| Median of run maxima | 17.298 | 24.409 | 50.806 |

| Operation p50 (ms) | Baseline | Current | Neovim bare |
| --- | ---: | ---: | ---: |
| Cursor down | 0.999 | 1.021 | 0.521 |
| Word forward | 1.022 | 0.982 | 0.522 |
| Enter Insert | 7.623 | 2.682 | 0.569 |
| Insert character | 2.876 | 2.885 | 0.894 |
| Leave Insert | 1.014 | 1.060 | 50.806 |
| Undo | 0.849 | 0.919 | 0.530 |
| Redo | 0.882 | 1.014 | 0.571 |
| Open search | 0.449 | 0.542 | 0.549 |
| Type search | 2.170 | 2.346 | 0.535 |
| Submit search | 1.398 | 1.581 | 0.522 |
| Next search match | 1.065 | 1.157 | 0.563 |

Overall p50 moved from 1.057 to 1.105 ms (+4.5%); median run p99 moved from 8.713 to 9.228 ms (+5.9%), and median run maximum from 17.298 to 24.409 ms. Entering Insert improved from 7.623 to 2.682 ms. The single-action mode transition values and maxima are especially sensitive to scheduling. Neovim's maximum includes its Esc timeout. This workload supports a mixed result, with faster Insert entry and higher sampled tails, rather than a blanket no-regression claim. [Individual editing runs](bench/editor-comparison-2026-10-08.json) include all operation categories.

### Warm language-server formatting

Five fresh processes per editor warmed standalone clangd `23.1.0`, then measured the format key through the visible text edit: Vaayu **22.840 ms**, bare Neovim with a minimal LSP setup **22.986 ms**, Helix **23.187 ms**. Each editor succeeded 5/5 times. The sub-millisecond differences are below this screen-polling harness's useful resolution and should be treated as a tie. This is warm formatting, not server startup, diagnostics latency, completion, or every LSP method. [Raw results](bench/2026-10-08/lsp.json), [harness](bench/lsp_compare.py), and [version provenance](bench/2026-10-08/metadata.json) describe the setup.

### Native Mermaid cold and cached output

The new [Mermaid harness](bench/mermaid_latency.py) ran five fresh processes per mode, using one three-node flowchart, 100×40 cells and an 8×16 pixel cell size. Cold diagram timings include the implementation's 150 ms worker debounce. Each process then closed and reopened the preview to exercise the cached render. Forced Kitty mode uses a synthetic PTY: it checks completed PNG upload, placement and Unicode placeholder bytes, without capability negotiation or physical pixel inspection. [Source fixture and all samples](bench/2026-10-08/mermaid.json) are retained.

| Mode and state | First output median (ms) | Diagram/upload output median (ms) | PNG uploads per sample |
| --- | ---: | ---: | ---: |
| Unicode cold | 0.760 | 177.227 | 0 |
| Unicode cached reopen | 0.694 | 13.597 | 0 |
| Kitty cold | 0.728 | 303.840 | 1 |
| Kitty cached reopen | 0.637 | 26.127 | 1 |

All ten processes completed both cold and cached checks. Closing deletes the terminal image, so cached reopening uploads the already-rendered PNG once; it does not rerun the layout/render worker. These values do not replace the historical prototype-process timings or the original implementation's actual-Kitty visual inspection. Unsupported diagrams and terminal fallback remain documented in [the Mermaid implementation report](MERMAID_PREVIEW_PLAN.md). [Renderer source](src/markdown.rs) and [worker/cache source](src/mermaid.rs) explain the debounce and lifecycle.

### Reproduction

Build the current and baseline release with `cargo build --release --bins --locked` in separate checkouts. Install [PTY dependencies](tests/requirements.txt) into a virtual environment; obtain the official editor/runtime and clangd versions above. Use absolute binary paths. Run all measurements sequentially from a temporary `git archive '4f1b81d'` project initialized with `git init`, with separate temporary XDG directories and `NO_COLOR`/`VAAYU_PROFILE` unset. For the legacy workload, put `jk_escape=false`, `clipboard_unnamedplus=false` and `[lsp.rust] enabled=false` in the temporary Vaayu config; core and LSP harnesses create their own configs. Put the working clangd directory first on `PATH` for LSP. [Metadata argv](bench/2026-10-08/metadata.json) records every actual command, artifact destination and binary hash; [development instructions](DEVELOPMENT.md#isolate-persisted-state) explain isolation.

The following commands are templates with deliberately quoted paths; replace `/absolute/...` with your installation paths and run from that archived fixture. Repeat the first three three times, reverse current/baseline legacy argument order on run two, and use distinct output filenames. The baseline large-file call selects only Vaayu.

```sh
python3 '/absolute/repo/bench/editor_compare.py' \
  --vaayu '/absolute/current/vaayu' --nvim '/absolute/nvim/bin/nvim' \
  --helix '/absolute/helix/hx' --helix-runtime '/absolute/helix/runtime' \
  --only 'vaayu' --only 'nvim_bare' --only 'helix' \
  --cols 100 --rows 40 --out '/absolute/results/editors-1.json'
python3 '/absolute/repo/bench/editor_compare.py' \
  --vaayu '/absolute/baseline/vaayu' --only 'vaayu' \
  --cols 100 --rows 40 --out '/absolute/results/baseline-large-1.json'
python3 '/absolute/repo/bench/latency.py' \
  --file '/absolute/fixture/src/normal.rs' --cwd '/absolute/fixture' \
  --cols 100 --rows 40 --out '/absolute/results/editing-1.json' \
  'current:/absolute/current/vaayu' 'baseline:/absolute/baseline/vaayu' \
  'nvim_bare:/absolute/nvim/bin/nvim -u NONE --cmd "set noswapfile shadafile=NONE"'
python3 '/absolute/repo/bench/mermaid_latency.py' \
  --vaayu '/absolute/current/vaayu' --attempts 5 \
  --out '/absolute/results/mermaid.json'
python3 '/absolute/repo/bench/lsp_compare.py' \
  --vaayu '/absolute/current/vaayu' --nvim '/absolute/nvim/bin/nvim' \
  --helix '/absolute/helix/hx' --helix-runtime '/absolute/helix/runtime' \
  --attempts 5 --out '/absolute/results/lsp.json'
```

No authenticated SSH host was supplied, so [ssh_latency.py](bench/ssh_latency.py) was not rerun. No loopback latency is substituted. These runs cover local editor actions, warm formatting and one native Mermaid fixture; they do not assert universal performance across projects, terminals, plugins, remote connections or diagram families.

## Vaayu, Neovim and Helix after the feature roadmap — 2026-10-03

`bench/editor_compare.py` was re-run on `91e9145` (master after the feature
roadmap merge), in a 4-vCPU Linux cloud container rather than the machine used
for the 2026-09-16 run below, so the two runs are not directly comparable. The
editor versions match the earlier run: Neovim 0.12.5 (official tarball, `-u NONE`) and
Helix 25.07.1 (official release, Rust LSP disabled). There is no personal
Neovim config in the container, so this run has no configured-Neovim column.
Same 50,000-line generated file and 100×40 PTY. The harness ran three times
back to back; values are the median across runs of each run's median
key-to-first-output milliseconds.

| Operation | Vaayu | Neovim bare | Helix |
| --- | ---: | ---: | ---: |
| Startup (min–max of 3) | 15.5–29.4 | **6.3–8.2** | 782.0–887.5 |
| Idle RSS | 59.3 MiB | **9.6 MiB** | 120.2 MiB |
| Cursor down | 0.68 | **0.55** | 1.77 |
| Page scroll | 0.57 | **0.56** | 1.81 |
| Enter Insert | **0.60** | 0.63 | 1.57 |
| Insert character | **0.70** | 0.82 | 406.19 |
| Leave Insert | **0.50** | 51.03 | 1.73 |
| Submit buffer search | **0.45** | 1.94 | 1.76 |
| Next search match | 1.17 | **0.63** | 2.26 |
| Open project picker | **0.58** | n/a | 1.81 |
| Filter project picker | **0.53** | n/a | 2.31 |
| Open live grep | **0.41** | n/a | 1.93 |
| Filter live grep | **0.43** | n/a | 1.75 |

All three editors had zero timeouts in every run. Vaayu's startup was noisy:
15.5, 29.4 and 29.4 ms in the three runs, and 14.0–15.1 ms in four more
Vaayu-only launches. The same four launches of the pre-roadmap build
(`bfaf9c8`) gave 14.0–15.1 ms too, so the roadmap did not regress startup. The
gap from the 3.8 ms measured on 2026-09-16 comes either from this container or
from changes made before `bfaf9c8`; this run cannot tell which. Neovim's Insert
exit is its default 50 ms `ttimeoutlen` wait after Esc. The 12 ms quiet-window
completion proxy sits at 12.6–14.5 ms for nearly every single-key operation in
all three editors, so it does not separate them here. The `grep_results`
and `picker_inventory` settle windows saw no further output from Vaayu or
Helix after the last filter key, because results were already on screen, so
there is no separate results latency to report. Raw data for all three runs:
[editor-comparison-2026-10-03.json](bench/editor-comparison-2026-10-03.json).

## Action registry + which-key — 2026-09-16

Same 100x40 PTY harness and file (`src/normal.rs`) comparing retained release
`6836f46` against the leader-key action registry and which-key popup
(NEOVIM_PARITY_PLAN.md M1.A, partial). 236 measured responses per binary.

| First-response measurement | `6836f46` | With action registry |
| --- | ---: | ---: |
| Overall median | 0.556 ms | 0.526 ms |
| p90 | 2.762 ms | 2.655 ms |
| p99 | 4.507 ms | 3.979 ms |
| Maximum | 5.191 ms | 5.230 ms |
| Timeouts | 0 | 0 |

No regression, as expected: leader dispatch is still an O(1) match/lookup,
and the new which-key poll-timeout branch in the main loop only executes
while a leader prefix is left hanging, never on a completed keystroke.

## Vaayu, Neovim and Helix — 2026-09-16

Vaayu is not faster in every measured path. A release-build PTY run used a
50,000-line generated Rust file at 100×40. Neovim 0.12.5 was measured both with
`-u NONE` and with this machine's normal Snacks-based configuration. Helix
25.07.1 used its official release runtime with the Rust language server disabled,
matching Vaayu's core-editor run. Values below are median key-to-first-output
milliseconds; lower is better.

| Operation | Vaayu | Neovim bare | Neovim configured | Helix |
| --- | ---: | ---: | ---: | ---: |
| Startup | **3.793** | 13.543 | 15.965 | 402.873 |
| Cursor down | 0.472 | **0.339** | 1.759 | 3.195 |
| Page scroll | 0.556 | **0.392** | 4.878 | 3.272 |
| Insert character | 0.837 | **0.723** | 1.995 | 181.180 |
| Leave Insert | **0.391** | 50.772 | 52.327 | 0.688 |
| Submit buffer search | **0.202** | 3.614 | 1.662 | 1.592 |
| Next search match | 1.741 | **0.228** | 10.528 | 4.508 |
| Open project picker | **0.530** | n/a | 35.404 | 3.477 |
| Filter project picker | **0.156** | n/a | 2.182 | 1.484 |
| Open live grep | **0.435** | n/a | 20.421 | 2.644 |
| Filter live grep | **0.317** | n/a | 2.154 | 2.632 |

Vaayu prewarms its asynchronous picker inventory after the first frame, so
opening the picker produced no delayed inventory output; the harness's minimum
settle window was 125.418 ms. Full grep results became visible after 68.614 ms
and settled after 269.648 ms, including a 60 ms debounce. Bare Neovim has no
comparable built-in project picker in this setup. Raw first-byte, 12 ms
quiet-window, p95, byte-count,
startup and resident-memory data are in
[editor-comparison.json](bench/editor-comparison.json); the reproducible harness
is [editor_compare.py](bench/editor_compare.py).

Large-buffer syntax parsing is deferred while typing, then caught up after a
150 ms idle window. Per-line content identities keep unchanged rendered rows
valid after an edit; viewport and composed-row caches avoid repeated layout;
terminal insert/delete-line scrolling reduces page-motion work; repeated search
uses revision-aware match positions; and picker filtering scores borrowed paths
before cloning only the visible results. The latest 50,000-line run measured
0.837 ms median first response for an inserted character, down from 1.450 ms in
the first cross-editor run.

A separate five-process clangd formatting run warmed each server, then measured
the format key through the visible edit. Medians were 22.685 ms for Vaayu,
22.640 ms for minimal Neovim and 22.430 ms for Helix, with five successes each.
Those differences are below this PTY harness's useful resolution and should be
read as a tie. Reducing Vaayu's idle LSP poll from 30 ms to 10 ms removed the
earlier extra polling tick. See [lsp-comparison.json](bench/lsp-comparison.json)
and [lsp_compare.py](bench/lsp_compare.py).

Bare Neovim still reaches the first terminal byte sooner for cursor motion,
page scrolling, insertion and distant next-match navigation. Vaayu's completed
output proxy is slightly lower for cursor motion and materially lower for the
distant search jump (13.997 vs 16.013 ms), while page scrolling is effectively
tied (13.061 vs 13.079 ms). These runs cover representative editor and
formatting paths, not every LSP method, syntax grammar, project size, terminal,
plugin set, cache state or hardware platform. They support targeted
comparisons, not a universal speed claim.

## Follow-up comparison — 2026-09-16

The 2026-09-16 feature pass was compared with the retained `05ed934` release
binary on the same source file, 100×40 PTY and 236-operation script:

| First-response measurement | `05ed934` | Current |
| --- | ---: | ---: |
| Overall median | 0.675 ms | 0.653 ms |
| p90 | 2.647 ms | 2.756 ms |
| p99 | 6.643 ms | 5.851 ms |
| Maximum | 7.068 ms | 8.334 ms |
| Enter Insert | 6.681 ms | 2.517 ms |
| Insert character | 2.618 ms | 2.724 ms |
| Leave Insert | 1.506 ms | 0.648 ms |

There were no timeouts. Eager whole-buffer completion indexing was removed,
cutting the sampled Insert-entry response by more than half; removing the false
revision bump also cut Insert exit by more than half. Overall p50 and p99 fell,
while p90, maximum and per-character insertion moved slightly higher. The table
records both directions without treating one local run as a general guarantee.
Full data is in [results-followup.json](bench/results-followup.json).

`bench/ssh_latency.py` now runs the same workload over a real authenticated SSH
PTY and cleans its private remote temporary file. It deliberately requires an
explicit host and installed remote Vaayu binary; no loopback result is presented
as remote-network evidence.

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

## Historical performance investigation — 2026-09-15

Measured with `bench/latency.py`: wall-clock time from writing a key to a
pty to the first byte of the editor's response arriving (a first-response proxy, not completed-frame latency), over a scripted
session (100x `j`, 50x `w`, an insert-mode typing burst, undo/redo, and a
search), replayed against each editor on the same 100x40 terminal editing
the same ~900-line Rust source file.

```sh
python3 bench/latency.py \
  "vaayu:/path/to/vy" \
  'nvim-bare:nvim -u NONE --cmd "set noswapfile"' \
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

## Historical results (2026-09-15, release build)

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

## Historical follow-up recommendation — 2026-09-15

The text below records that investigation's then-unimplemented work. Incremental span updates subsequently shipped; [TECHNICAL_DECISIONS.md](TECHNICAL_DECISIONS.md) quotes the current implementation. This is retained history, not today's outstanding task list.

The throttle caps *how often* the expensive full-tree rebuild happens, but
doesn't make the rebuild itself cheap, so a sustained fast-typing session
still pays it often. The correct fix is incremental span updates using
tree-sitter's `Tree::changed_ranges()` against the previous tree -- reuse
spans outside the changed region, re-walk only the affected subtree(s),
shift byte offsets for spans after the edit point. Not done here: getting
the offset-shifting and boundary cases right needs its own focused pass
rather than being rushed at the end of this one.
