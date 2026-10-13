# Tours UI and UX delivery

Implemented 2026-10-13 against `8cca6f9b62439bf04b2ac7b3817c813e20f892ee`, following the [interactive audit and plan](TOURS_UI_UX_PLAN.md). The evidence directory contains the actual terminal cells captured by PTY tests, plus validation logs and binary hashes.

## Delivered changes

| Finding | Result |
| --- | --- |
| F1: scratch text lost on controls | View changes, asynchronous Diff completion, highlight navigation, indicator toggles, rendering toggles, resize, and same-step retries preserve lesson edits and stored reading positions. Only a step change replaces the scratch lesson. |
| F2: missing files appeared as empty source | Validate project-relative paths and disk availability before opening. A read-only recovery view offers Retry, Skip, and Edit definition. Source/Diff/Flow cannot expose a previous source while unavailable. Validation checks disk even when a new-file buffer exists. |
| F3: cramped small terminals | Automatic layout shows the focused tour pane at ≤16 rows; ordinary terminals use a 55/45 horizontal split; ≥140 columns and ≥24 rows use side by side panes. Explicit session layout preferences remain available, with a focused-pane fallback at ≤10 rows. |
| F4: controls disappeared while scrolling | The scratch document contains the authored lesson and stable visual explanations. Progress, focus, path, warnings, and actions belong to pinned rows. Essential actions fit 40 columns; larger widths add steps, raw/read, and available views. Mouse targets invoke the same commands as keyboard actions. Optional metadata is available through copy/help actions. |
| F5: raw Markdown was the first experience | Lessons open rendered, without editing gutters. Raw scratch editing stays available through `,vm` or Esc. The session preference persists, and help explains the scratch lifetime. |
| F6: step rows flattened entire lessons | Step entries use a clean summary, index, source path/range, and current selection. The authored description remains in the detail view. |
| F7: catalog lacked health and resume | Background catalog loading displays step counts, invalid/empty status, missing-source counts, and resume actions. `r` explicitly starts over, `e` edits, `n` creates, and `R` refreshes. No synchronous full anchor/Git checks run while listing. |
| F8: empty/error states lacked direction | Empty lists offer an AI draft or minimal editable example. Invalid JSON includes filename/line/column and repair guidance. Missing tour, empty tour, and missing source have distinct feedback. |
| F9: final step lacked direction | Final steps offer Finish, Restart, and Tours; Next reports completion while preserving the source and lesson. |
| F10: generated arguments vanished in Markdown | Generated help uses inline code and concrete arguments. Highlight labels/IDs and takeaways remain readable without injecting mutable controls into the lesson. |

The implementation also fixes focus when clicking blank lesson space, keeps the active pane visible after shrinking an explicit split, and positions the terminal cursor inside the focused preview pane. Existing source edits and parent panes survive finishing a tour. The highlight counter reads `Highlights: N` before selection and `Highlight i/N` after selection.

## Visual evidence

The before captures use the original audit fixture; the after captures use a longer lesson and title to exercise clipping and scrolling. They demonstrate layout and behavior rather than a pixel comparison of identical content.

| Scenario | Before | After |
| --- | --- | --- |
| Small terminal, 40×12 | [Raw lesson hidden below chrome](artifacts/tours-ui-audit-2026-10-13/captures/40x12-default-raw.png) | [Compact rendered reading](artifacts/tours-ui-implementation-2026-10-13/captures/pty_tour_ux/40x12-reading.png) |
| Ordinary terminal, 120×30 | [Initial raw split](artifacts/tours-ui-audit-2026-10-13/captures/120x30-default-raw.png) | [Balanced split and pinned actions](artifacts/tours-ui-implementation-2026-10-13/captures/pty_tour_ux/120x30-reading.png) |
| Wide terminal, 180×50 | [Initial raw split](artifacts/tours-ui-audit-2026-10-13/captures/180x50-default-raw.png) | [Side by side reading](artifacts/tours-ui-implementation-2026-10-13/captures/pty_tour_ux/180x50-reading.png) |
| Scroll to the end | [Progress hidden](artifacts/tours-ui-audit-2026-10-13/captures/120x30-scrolled-end.png) | [Pinned progress and actions](artifacts/tours-ui-implementation-2026-10-13/captures/pty_tour_ux/120x30-scrolled.png) |
| Missing source | [Apparently normal empty source](artifacts/tours-ui-audit-2026-10-13/captures/errors-missing-file.png) | [Recoverable missing-source validation](artifacts/tours-ui-implementation-2026-10-13/captures/pty_tour_ux/120x30-missing-source-check.png) |

## Validation

- Rust: **904 passed, 0 failed, 3 ignored per binary**, run from isolated temporary project directories. The ignored tests are two opt-in benchmarks and the existing real-clangd interoperability test. [Results](artifacts/tours-ui-implementation-2026-10-13/rust-results.json), [vaayu log](artifacts/tours-ui-implementation-2026-10-13/rust-vaayu.log), [vy log](artifacts/tours-ui-implementation-2026-10-13/rust-vy.log).
- Formatting and Clippy: `cargo fmt --all -- --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `git diff --check` pass. [Clippy log](artifacts/tours-ui-implementation-2026-10-13/clippy.log).
- Release builds: both executable names built with `cargo build --release --bins --locked`.
- UI: **19 suite runs passed**: seven tour suites per binary, plus review/quickfix/split/wrap, window navigation, sidebar navigation, Markdown mouse, and AI/tour-draft integration. [Machine-readable results and tested binary hashes](artifacts/tours-ui-implementation-2026-10-13/ui-results.json).
- Visual coverage: 12 themes × two palettes × three sizes in each visuals/Markdown suite, with the existing ≥4.5:1 contrast assertions retained. The new UX suite covers automatic layouts at 40×12, 80×24, 120×30, and 180×50; mouse controls; pinned scroll state; scratch preservation; current step selection; resume/restart; unavailable-source validation/retry/edit; completion; and example creation while a tour is active.

The last change hides unavailable view buttons in the recovery state. Its updated UX suite is rerun on both final release binaries; [final recovery results and hashes](artifacts/tours-ui-implementation-2026-10-13/final-recovery-results.json) distinguish those binaries from the preceding full UI matrix.

Reproduce the UI matrix after building both binaries:

```sh
cargo build --release --bins --locked
python artifacts/tours-ui-implementation-2026-10-13/run_ui_checks.py
```

Use a Python environment with `tests/requirements.txt`. The runner isolates XDG state, unsets `NO_COLOR`, and stores per-suite logs, terminal captures, and binary hashes. Rust test executables are run directly from temporary project directories, following [the existing persisted-state isolation instructions](DEVELOPMENT.md#isolate-persisted-state); Cargo's package working directory otherwise loads ignored local `.vaayu` state.

## Latency comparison

Measured after builds and UI tests finished, using isolated 50,005-line fixtures at 100×30. The movement/typing/step comparison alternates the original and final builds across three rounds, using each build's default tour presentation. Raw-navigation comparisons explicitly disable Markdown in both builds; rendered comparisons use a heading, styles, a Rust fence, and 80 list items.

| Metric | Before p50 / p95, ms | After p50 / p95, ms |
| --- | --- | --- |
| Source movement with tour active | 0.58 / 0.80 | 0.54 / 0.86 |
| Source typing with tour active | 0.35 / 0.98 | 0.28 / 1.07 |
| Step text ready | 4.42 / 16.37 | 3.16 / 15.99 |
| Highlight ready with rendered lesson | 4.32 / 10.64 | 4.05 / 10.13 |
| Flow ready with rendered lesson | 3.22 / 10.28 | 2.77 / 7.06 |
| Source return with rendered lesson | 3.22 / 10.78 | 2.90 / 8.34 |
| Raw/rendered toggle ready | 3.92 / 11.93 | 4.20 / 12.26 |

Movement and typing measure first PTY output; the other metrics wait for expected text in the parsed terminal screen. None measures physical display latency. Full counts, hashes, and percentile data: [default presentation comparison](artifacts/tours-ui-implementation-2026-10-13/latency-comparison.json), [rendered before](artifacts/tours-ui-implementation-2026-10-13/markdown-before.json), [rendered after](artifacts/tours-ui-implementation-2026-10-13/markdown-after.json).

The initial raw-navigation run showed Flow/source-return p95s around 5 ms before and 10–11 ms after. A reverse-order confirmation measured **10.65 / 11.10 ms before** and **10.74 / 11.50 ms after**, so the apparent doubling did not repeat. Medians varied between runs; retain these measurements as local PTY evidence, not a display-latency guarantee. [Initial before](artifacts/tours-ui-implementation-2026-10-13/navigation-before.json), [initial after](artifacts/tours-ui-implementation-2026-10-13/navigation-after.json), [confirmation before](artifacts/tours-ui-implementation-2026-10-13/navigation-confirmation-before.json), [confirmation after](artifacts/tours-ui-implementation-2026-10-13/navigation-confirmation-after.json).

```sh
python bench/tour_latency.py /path/to/original-vaayu target/release/vaayu comparison.json
python bench/tour_latency.py --navigation target/release/vaayu navigation.json
python bench/tour_latency.py --markdown target/release/vaayu markdown.json
```

## Scope and defaults

The optional outline rail remains unnecessary: the improved step picker provides direct navigation without taking reading space. Persistent notes and saving scratch edits back into tour JSON remain separate features, as specified in the accepted plan. The catalog's health check covers parseability, step presence, and source paths; `:tourcheck` performs deeper anchor, connection, and comparison checks on demand. Layout thresholds are explicit session defaults and can be overridden with `:tourlayout`.
