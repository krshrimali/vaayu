# Development and validation

Documentation/source baseline: `4f1b81d`, refreshed 2026-10-08. [CHANGELOG.md](CHANGELOG.md) covers all subsequent documentation, benchmark, and fixture reconciliation in this refresh. Application Rust behavior was not changed. `HELP.md` is embedded in the executable, so release binaries and tests were rebuilt after editing it.

## Build and tools

[rust-toolchain.toml](rust-toolchain.toml) pins Rust `1.99.0`, rustfmt and Clippy; [CI](.github/workflows/ci.yml) uses the same toolchain. Build both executable names with the lockfile. `vy` includes `main.rs`, so each executable compiles the same Rust test suite.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --bins --locked
cargo test --locked
```

Use a virtual environment for the PTY dependencies, [pyte and pexpect](tests/requirements.txt). Tests invoking `python3` for a mock LSP should see that environment on `PATH`.

```sh
python3 -m venv '/tmp/vaayu-check-venv'
. '/tmp/vaayu-check-venv/bin/activate'
python3 -m pip install -r 'tests/requirements.txt'
python3 'tests/pty_regression.py' 'target/release/vaayu'
python3 'tests/pty_regression.py' 'target/release/vy'
python3 'tests/pty_extended.py' 'target/release/vaayu'
python3 'tests/pty_extended.py' 'target/release/vy'
python3 'tests/pty_ui.py' 'target/release/vaayu'
python3 'tests/pty_typing_ui.py' 'target/release/vaayu'
python3 'tests/pty_window_navigation.py' 'target/release/vaayu'
python3 'tests/pty_mermaid.py' 'target/release/vaayu'
python3 'tests/pty_markdown_images.py' 'target/release/vaayu'
python3 'tests/pty_markdown_mouse.py' 'target/release/vaayu'
cargo test --locked real_clangd_formatting_and_diagnostics -- --ignored
```

The real-clangd test needs a functioning `clangd`, not merely an executable path. This machine's installed clangd could not load `libLLVM.so.22.1`; this refresh used the official standalone clangd `23.1.0` under `/tmp`, without modifying system packages. Neovim `0.12.5`, Helix `25.07.1`, their official runtimes, and Python dependencies were also installed only under `/tmp` for the benchmarks. Exact versions/hashes are in [benchmark provenance](bench/2026-10-08/metadata.json).

## Highlight contrast

`tests/pty_highlight_contrast.py` checks picker prompts/selections, result lists,
status lines, and search matches in all ten built-in themes at three terminal
sizes. It runs with both standard ANSI colors and a pale remapped palette like
the one that exposed unreadable picker rows. Highlighted text must reach a 4.5:1
contrast ratio; plain rows keep the terminal or theme defaults.

```sh
python3 tests/pty_highlight_contrast.py target/release/vaayu
python3 tests/pty_highlight_contrast.py target/release/vy
```

## Isolate persisted state

Run Rust tests from a clean checkout/project with `NO_COLOR` unset. This repository has ignored local `.vaayu/shada.json` state that seeds the jump-list fixture: an initial ordinary `cargo test` run produced 833 passes and one failure in `regression::jumps_command_lists_jumplist_and_navigates_to_entries`. Cargo runs tests in the package directory even when invoked from a temporary directory using `--manifest-path`, so that alone does not isolate project state. Existing color tests also depend on SGR output and fail if `NO_COLOR` is inherited.

This refresh compiled the test executables with `cargo test --locked --no-run --message-format=json`, extracted each test executable from the compiler-artifact JSON, and ran them directly from a temporary directory with isolated `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, and `XDG_STATE_HOME`, with `NO_COLOR` unset. Both regular suites and explicitly enabled real-clangd tests passed there. The user's project state was preserved. A clean Git checkout is the simpler normal development setup.

Each PTY suite builds its own temporary files/configuration. For a full local pass:

```sh
for suite in tests/pty_*.py; do
  env -u NO_COLOR python3 "$suite" 'target/release/vaayu' || exit
done
```

For additional isolation, use temporary XDG data/state/config/cache directories around the suite runner. Avoid running performance measurements alongside builds or test suites.

## Refresh coverage

- Formatting, strict locked Clippy across all targets, and locked release builds for `vaayu` / `vy`.
- Both Rust binaries: 834 regular tests passed each, three ignored each. The counts describe the same suite twice. Real-clangd formatting/diagnostics passed explicitly for each binary; the other ignored tests remain opt-in.
- Complete PTY inventory: 207 suites were run. The initial run passed 202; five reproducible fixture failures were investigated and corrected as described below, then rerun (including separately timed Esc writes to avoid Alt-Esc decoding). All 207 unique suites passed across the complete run and targeted repairs. The rebuilt release also passed 18 affected/general Vaayu PTY suites plus the regression/extended suites for `vy`. Final results and raw validation summaries are in [the refresh artifact](bench/2026-10-08/validation.json).
- Mermaid coverage includes six PTY scenarios, both Unicode/graphical paths, resize/zoom/pan/cleanup, stale requests and modal precedence. The existing implementation report also records the original actual-Kitty/Xvfb inspection; this refresh used PTYs and does not claim a new visual terminal session.
- Documentation checks cover all Markdown local links/anchors, all source modules in architecture, every top-level config field and discovery/action entry, every post-refresh commit ID, source-exact Rust excerpts, synchronized technical HTML, JSON validity and an 18-line README with no headings, tables, bullets or code fences.

### Corrected fixture assumptions

| Fixture | Cause and correction |
| --- | --- |
| [pty_colorscheme.py](tests/pty_colorscheme.py), [pty_themes.py](tests/pty_themes.py) | Automatic Rust LSP diagnostics on this machine changed the cells the theme checks inspect. Disable Rust LSP in these fixtures so the assertions measure syntax/theme colors alone. |
| [pty_outline_collapse.py](tests/pty_outline_collapse.py) | The new 30% sidebar truncates full names at 40 columns. Equalize the splits before the collapse assertions; dedicated outline tests still cover default width. |
| [pty_resume.py](tests/pty_resume.py) | First Esc leaves picker query Insert mode; second dismisses. Exercise both steps before resume. |
| [pty_task_tracker.py](tests/pty_task_tracker.py) | Expected tour-list title predates `K` explanation focus. Assert the current title. |
| [requirements.txt](tests/requirements.txt) | The complete suite includes pexpect-based task/tour tests; pin pexpect alongside pyte so documented installation is sufficient. |
| [editor_compare.py](bench/editor_compare.py) | Send separately timed Esc presses before live grep, so current and baseline builds receive the same valid sequence. Benchmark state/config are isolated and Rust LSP is disabled. |

These changes repair validation/reproduction assumptions without changing application behavior. The current [help](HELP.md) explains the corresponding UI state transitions.

## Structured tour visuals validation

The optional tour extension has a shared [authoring schema](assets/tour.schema.json)
and a [working example](examples/visuals.tour). Both Rust suites ran in isolated
project/configuration/data/state directories: 893 tests passed per executable,
with three ignored. Formatting, strict Clippy across all targets, and locked
release builds passed.

`tests/pty_tour_visuals.py` runs the application in real PTYs and checks
callout arrows, distinct range colors, 4.5:1 highlight contrast, indicator
toggles, live relocation, search layering, Source/Diff/Flow switching,
cross-file jumps, explanation focus, resize, teardown, and unsaved Git
snapshots. It covers ten themes, stock and remapped pale palettes, and
40×12, 80×24 and 140×40 terminals. Both executable names were exercised.
The existing tour/navigation/scroll/focus/clipboard/AI suites and diff,
cursorline and general highlight-contrast PTYs also passed.

```sh
python3 tests/pty_tour_visuals.py target/release/vaayu artifacts/tour-visuals
python3 bench/tour_latency.py /path/to/before-vaayu target/release/vaayu bench/tour-visuals-latency.json
```

[Source](artifacts/tour-visuals/source.png),
[Diff](artifacts/tour-visuals/diff.png), and
[Flow](artifacts/tour-visuals/flow.png) captures reproduce the actual PTY's
character/color cells using installed fonts; their adjacent `.txt` files
retain the terminal text. These are PTY evidence, not graphical-terminal
screenshots. The [latency results](bench/tour-visuals-latency.json) compare
isolated 50,005-line fixtures before/after, with and without an active tour,
over three alternating rounds. Movement and typing measure key-to-first-PTY
output; step navigation measures receipt of the expected step text in the
parsed screen. Neither measurement is physical display latency. Benchmark
only after builds and other tests finish.

For this 50,005-line run, p95 key-to-first-output was 0.818 ms for movement
and 0.706 ms for typing with visuals enabled (before: 0.639 / 0.606 ms).
Median step-ready time was 4.864 ms (before: 4.794 ms).

## Tour highlight navigation validation

The navigation follow-up adds `]h`/`[h`, `,v]`/`,v[`, and
`:tourhnext`/`:tourhprev` within the current step. Statuslines retain the
selected annotation index across Source/Diff/Flow and explanation focus.
Returning with `Ctrl-O` restores the source cursor and viewport without
consuming source-history jumps or recording the preview in that history.
Explicit Source restoration also recovers when another navigation action
has replaced the preview buffer.

Both isolated Rust suites passed 897 tests, with three ignored per binary.
Formatting, strict Clippy and locked release builds of both executables
passed. The final release binaries also passed PTY runs at 140×40 (`vaayu`)
and 40×12 (`vy`). The real-PTY matrix passed at 40×12,
80×24 and 140×40, across ten themes and stock/pale palettes, including
highlight boundaries, cross-file targets, counter resets, global statuslines,
indicator toggles and preview returns from either focused pane. Existing
tour, highlighting, focus and scrolling PTYs passed as well. Unit coverage
also checks custom statusline formats, counts, broken anchors, empty jump
history and source restoration after the preview buffer is replaced.

[Selected highlight](artifacts/tour-visuals/highlight.png) and
[returned Source](artifacts/tour-visuals/source-return.png) captures show
`Indicator 2/3` and the restored source line. Like the captures above, these
reproduce real PTY character/color cells.

The [navigation measurements](bench/tour-navigation-latency.json) use the
same isolated 50,005-line fixture over three rounds. The p95 times to the
expected parsed-screen state were 7.898 ms for highlight navigation,
8.407 ms for opening Flow, and 9.816 ms for returning to Source with Ctrl-O.
The [before/after comparison](bench/tour-navigation-comparison.json) records
both binary hashes and alternates the prior visuals build and this follow-up.
With a tour active, p95 key-to-first-output was 0.977 ms for movement and
0.868 ms for typing (before: 0.721 / 1.020 ms); median step-ready time was
8.182 ms (before: 7.890 ms). These PTY measurements exclude physical display
latency and were taken after all builds and UI tests finished.

```sh
python3 bench/tour_latency.py --navigation target/release/vaayu bench/tour-navigation-latency.json
python3 bench/tour_latency.py /path/to/before-vaayu target/release/vaayu bench/tour-navigation-comparison.json
```

## Tour Markdown validation

`,vm` / `:tourmarkdown [on|off|toggle]` renders the existing description
scratch pane using the Markdown layout cache. Raw Markdown is the default;
`Esc` returns to editing, and `,mp` also toggles in the description. No toggle
rewrites the buffer or tour JSON. Unchanged annotation navigation retains
the rendered layout and reading position.

Both isolated Rust suites passed 901 tests, with three ignored per binary.
Formatting, strict locked Clippy across all targets, and release builds for
both executable names passed. Unit coverage
checks edits, source/raw/rendered viewports, pane reuse and cleanup, tour
navigation, counters, Flow/Diff returns, styled spans, and cache reuse until
an edit or resize.

The new [Markdown PTY suite](tests/pty_tour_markdown.py) passed at 40×12,
80×24 and 140×40 across ten themes and stock/pale palettes. It checks actual
SGR bold/italic/code colors, lists, links, tables, scroll bounds, resize,
raw edits, commands, step changes, highlight navigation, source restoration
and unchanged source/tour files. The highlight fixture now expects original
Markdown markers in the default raw pane, replacing its obsolete assertion
that description formatting is stripped.

The optimized binaries also passed Markdown PTY runs at 140×40 (`vaayu`)
and 40×12 (`vy`). Existing tour, focus, scroll, highlight, Markdown mouse and
preview-bound suites passed, along with the Source/Diff/Flow visuals matrix.
[Rendered text](artifacts/tour-markdown/rendered.png),
[highlighted code](artifacts/tour-markdown/code.png) and
[raw Markdown](artifacts/tour-markdown/raw.png) captures reproduce real PTY
cells, including bold/italic font variants; adjacent `.txt` files retain
the terminal text. These are PTY captures, not graphical-terminal screenshots.

The [Markdown measurements](bench/tour-markdown-latency.json) use a
50,005-line source buffer and a description with a heading, emphasis,
inline code, a Rust fence and 80 list items, over three rounds. With rendering
enabled, p95 parsed-screen readiness was 4.001 ms for toggling, 3.467 ms for
highlight navigation, 4.358 ms for Flow, and 4.299 ms for returning to Source.
The [before/after comparison](bench/tour-markdown-comparison.json) alternates
the previous navigation build and this build. With a tour active in the
default raw view, p95 movement/typing key-to-first-output was 0.369/0.324 ms
(before: 0.366/0.552 ms); median step readiness was 4.811 ms (before: 4.828 ms).
Both result files include binary hashes. Measurements ran after builds and
UI tests finished and exclude physical display latency.

```sh
python3 tests/pty_tour_markdown.py target/release/vaayu artifacts/tour-markdown
python3 bench/tour_latency.py --markdown target/release/vaayu bench/tour-markdown-latency.json
python3 bench/tour_latency.py /path/to/before-vaayu target/release/vaayu bench/tour-markdown-comparison.json
```

## Benchmark reproduction

[BENCHMARKS.md](BENCHMARKS.md) contains workload, hardware, raw results and exact quoted commands. The refresh uses three sequential core-editor runs, three same-machine large-file baseline runs, three fixed-source legacy editing runs with alternating current/baseline order, five warmed clangd attempts per editor, and five fresh Mermaid processes per mode with cached reopening. [Metadata](bench/2026-10-08/metadata.json) records binary sizes/hashes and argv; [validation](bench/2026-10-08/validation.json) records check results. SSH remains unmeasured without an explicitly supplied authenticated host.
