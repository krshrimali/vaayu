# Tours UI and UX audit and improvement plan

Implementation: see [delivery and validation](TOURS_UI_UX_IMPLEMENTATION.md).

Audited on 2026-10-13 against `8cca6f9` (`Add GitHub dark and light colorschemes`).

The core tour features work. The highest priorities are preventing scratch-edit loss, handling missing source files honestly, and giving the explanation enough room to read. The next improvement should make the existing capabilities easier to discover, rather than add more tour commands.

## What was tested

- Built the current release binary with `cargo build --release --locked --bin vaayu`.
- Ran **six existing PTY suites; all passed**: basic tours, explanation focus and clipboard, scrolling and split resizing, step highlighting, visual views, and Markdown.
- Those suites cover 28 suite configurations. The visual and Markdown suites each exercise 12 themes and two terminal palettes, including color-contrast assertions in the visual suite. Existing coverage includes keyboard navigation, mouse focus and divider dragging, cross-file highlights, preview return, independent pane positions, and source/tour file preservation.
- Added an exploratory PTY driver and captured **74 terminal frames** at **40×12, 80×24, 120×30, and 180×50**, plus focused error and scratch-edit probes. Tested listing, raw/rendered explanations, scrolling, Source/Diff/Flow, step selection display, first/last boundaries, ending/resuming, live resizing, malformed/empty tours, missing paths, unresolved anchors, and scratch editing.
- Inspected captures reconstructed from actual terminal character/color cells. These are local test fixtures and isolated editor state. This was automated UI testing and visual inspection; no user study or screen-reader evaluation was performed. The live AI-generation service was outside this audit.

[Regression results](artifacts/tours-ui-audit-2026-10-13/regression-results.json) · [Exploratory results and action ledger](artifacts/tours-ui-audit-2026-10-13/exploratory-results.json) · [Reproducible driver](artifacts/tours-ui-audit-2026-10-13/audit_driver.py)

Passing the existing suites does not cover the newly reproduced defects below.

## Findings and priorities

P1 means the first implementation pass; P2 follows the correctness and reading fixes; P3 is further polish. Estimates are relative effort, not delivery commitments.

| ID | Priority | Observed behavior | Recommended improvement | Effort |
| --- | --- | --- | --- | --- |
| F1 | P1 | An editable explanation loses a newly inserted note on Flow, next-highlight, or indicator-toggle actions. Markdown toggling alone preserves it. | Separate the editable body from generated tour chrome. Updating views or highlights must retain scratch text and reading position within the current step. | Medium |
| F2 | P1 | A nonexistent source path opens an empty buffer and appears as a normal tour step. After that, `:tourcheck` no longer reports the missing path. | Check source availability before opening a step; display a specific recoverable error. Validate disk existence separately from synthetic buffers. | Small–medium |
| F3 | P1 | At 40×12, the initial explanation viewport contains the title and controls, with no lesson text visible. At larger sizes, a short source file still receives most of the height. | Add a compact single-pane mode for short terminals and a better balanced split elsewhere. Reserve room for the lesson before optional chrome. | Medium |
| F4 | P1 | Scrolling to the explanation's end hides step progress in every exploratory size. View controls and lesson content scroll together. | Pin compact progress, active view, and essential navigation outside the description. | Medium |
| F5 | P2 | Explanations start as raw Markdown with line numbers. In rendered mode, generated title and view controls merge into a paragraph; useful lesson content competes with editor metadata. | Default to rendered reading, keep Raw/Edit visible, suppress explanation line numbers while reading, and separate title, takeaway, lesson, and metadata visually. | Medium |
| F6 | P2 | The step picker uses the entire description as a row label. A multiline Markdown description produces replacement glyphs and a clipped paragraph. | Use `vaayu.title`, then the first clean heading/line, as the summary; show step number and file/range; retain the full description in the detail preview. | Small |
| F7 | P2 | The tours list shows title/stem only. Empty, malformed, and healthy tours look equally launchable; existing resume state is invisible. | Show step count, health, and resume location. Keep filtering and existing launch keys; give invalid rows a repair action and make resume/start-over explicit. | Medium |
| F8 | P2 | Empty projects produce only `No tours found in .tours/*.tour`. Malformed JSON and missing tour names produce the same generic `Could not read tour` message. | Provide a useful empty view with Create and example instructions; distinguish missing file, invalid JSON with location, and empty tour, with recovery actions. | Small–medium |
| F9 | P2 | Next on the final step only reports `End of tour`. It leaves the tour active without explaining finish, restart, or choosing another tour. | Add an explicit final-step state with Finish, Restart, and Choose tour actions while keeping the source and lesson visible. | Small |
| F10 | P3 | Generated help contains `:tourjump <id>` and `:tourfollow <number>`. Rendering interprets the angle-bracket placeholders as HTML and hides the arguments. | Render generated commands as code spans or as dedicated chrome; use full, actionable examples in help. | Small |

### Evidence for the correctness defects

**F1 — explanation edits disappear without changing steps.**

Reproduction: `:tour visual` → `K` → `:tourmarkdown off` → insert a note at the beginning → `,vf`. The note disappears, the explanation's modified marker clears, and its position resets. Separate probes reproduce this with `]h` and `,vi`; `:tourmarkdown on` retains the note. The source and `.tour` files remain unchanged. This is loss of editable scratch content, not loss of source code.

[Before](artifacts/tours-ui-audit-2026-10-13/captures/editing-scratch-note.png) · [After Flow](artifacts/tours-ui-audit-2026-10-13/captures/editing-scratch-after-view-switch.png) · [After highlight](artifacts/tours-ui-audit-2026-10-13/captures/editing-scratch-after-highlight.txt) · [After indicators](artifacts/tours-ui-audit-2026-10-13/captures/editing-scratch-after-indicators.txt)

The cause is in [`refresh_tour_explanation`](src/tour_visuals.rs): it regenerates the whole scratch buffer using the authored description. [`replace_tour_explanation`](src/tour.rs) also resets the pane's cursor and scroll offsets. The cache shortcut applies only when rendered text equals the regenerated text; user edits break that equality.

**F2 — visiting a missing target masks the validation problem.**

Reproduction: use a step with `file: "missing.rs", line: 1`. Before visiting it, `:tourcheck` reports the missing file. Visiting it opens an empty `missing.rs` buffer and shows a normal step header without a missing-file warning. A subsequent check omits that missing-path issue. No file is written to disk by this action.

[Validation before visiting](artifacts/tours-ui-audit-2026-10-13/captures/errors-validation-before-opening-missing-file.txt) · [Apparently normal missing-file step](artifacts/tours-ui-audit-2026-10-13/captures/errors-missing-file.png) · [Validation after visiting](artifacts/tours-ui-audit-2026-10-13/captures/errors-validation.txt)

[`goto_tour_step`](src/tour.rs) uses ordinary editor file opening; [`Buffer::from_path`](src/buffer.rs) deliberately supports creating new files. Tour validation then consumes the opened buffer as source. Tours need their own availability check while preserving normal editor new-file behavior.

### Evidence for the reading and discovery issues

- [40×12 initial raw view](artifacts/tours-ui-audit-2026-10-13/captures/40x12-default-raw.png) and [rendered view](artifacts/tours-ui-audit-2026-10-13/captures/40x12-rendered.png): the body is below the available viewport; labels are clipped by the indicator count and header.
- [120×30 initial raw view](artifacts/tours-ui-audit-2026-10-13/captures/120x30-default-raw.png): most source rows are empty while the explanation shows only the beginning of the lesson. The bottom message repeats the entire multiline description with control glyphs and clips the useful navigation hint.
- [Scrolled explanation](artifacts/tours-ui-audit-2026-10-13/captures/120x30-scrolled-end.png): there is no persistent step number; generated command arguments disappear in rendered text.
- [Tours list](artifacts/tours-ui-audit-2026-10-13/captures/120x30-list.png): empty and malformed tours have no distinguishing status.
- [Step picker](artifacts/tours-ui-audit-2026-10-13/captures/120x30-steps.png): full Markdown is flattened into a row; the detail view repeats the same text.
- [Empty state](artifacts/tours-ui-audit-2026-10-13/captures/errors-no-tours.txt), [malformed tour](artifacts/tours-ui-audit-2026-10-13/captures/errors-malformed-tour.txt), and [last-step next](artifacts/tours-ui-audit-2026-10-13/captures/120x30-past-end.txt): feedback gives limited direction about the next useful action.

## Proposed experience

The reader should immediately know which step they are on, which code it explains, which pane has focus, and how to continue. Keep the existing Vim and tour bindings.

Use a tour-specific header and action bar around an ordinary source/description pane. The body holds the lesson and optional visual explanations; the UI owns progress and controls. A starting layout for normal terminal sizes is:

```text
Tour: Input to screen     Step 1/3     Source     Highlights: 3
─────────────────────────────────────────────────────────────
demo.txt:2–4                                Source • focused
  ① read_input();       → Read input
  ② update_buffer();    → Change buffer
    request_redraw();
─────────────────────────────────────────────────────────────
Follow one keypress                              Explanation
Understand how input, the Buffer, and rendering work together.

1. Decode the key and choose the current mode's handler.
2. Apply the edit at the cursor.
3. Request a redraw of the affected rows.
─────────────────────────────────────────────────────────────
[t Prev   ]t Next   K Focus   ,vo Steps   ,v? Help
```

The labels illustrate information placement; `,v?` / `:tourhelp` would be new actions. Preserve `K`, `]t`/`[t`, `]h`/`[h`, `,vm`, `Ctrl-O`, and all existing command aliases. Do not take over the editor's global `?` backward-search key.

Suggested layout rules to prototype and verify:

- **Short terminal, initially ≤16 rows:** one tour pane at a time, with a one-row compact header such as `1/3 · Source · K lesson`. Choose the first visible pane from the launch action: Enter opens source; K from the tours picker opens the lesson. K switches panes without losing either position. Give the lesson at least four visible rows at 40×12.
- **Ordinary terminal:** keep the familiar horizontal split, initially around 55% source / 45% explanation, with minimum body heights. Preserve explicit divider resizing. Ellipsize the title before clipping progress or the focused-pane label.
- **Wide terminal, initially ≥140 columns and ≥24 rows:** offer source and explanation side by side so reading gains height. Keep the user's layout choice for the session. An outline rail is a later enhancement; it should not consume needed reading space.
- Show only views available for the current step, with one clear active state. Existing keybindings and mouse controls should invoke the same actions.
- Replace `Indicator 0/N` with a neutral `Highlights: N` before selection, and `Highlight 1/N` after selection. Maintain matching numbered source callouts and full labels in the explanation.
- Keep raw editing available with `,vm`, and label its scratch scope. Initially retain the documented discard-on-step-change lifetime; view and highlight changes within a step must preserve edits. Persistent notes or saving back to JSON can be a separate feature.
- Use the existing theme tokens. Retain tested contrast, numbered markers, and textual active/focus states; do not rely on color alone. Keep optional metadata and advanced command help out of the main lesson's first screen.

The thresholds and ratios are design proposals. They need validation against real code and varied lesson lengths before becoming defaults.

## Implementation sequence

### Change 1: make viewing and validation trustworthy

Scope: F1 and F2. Separate generated chrome from editable explanation content; preserve text and viewport on all same-step actions. Add source-availability handling and ensure validation cannot confuse a synthetic buffer with an existing file. Keep loading/checking asynchronous where it already is.

Primary code: [`src/tour.rs`](src/tour.rs), [`src/tour_visuals.rs`](src/tour_visuals.rs). Avoid changing normal editor new-file behavior in `Buffer::from_path`.

Acceptance:

- An inserted scratch note survives Source/Diff/Flow changes, `Ctrl-O`, highlight navigation, indicator toggling, Markdown toggling, and resizing within the current step.
- View toggles retain explanation focus. Highlight navigation may focus source as it does today, while retaining the explanation's stored cursor and reading position.
- A missing target has a visible missing-source state with Retry, Skip, and Open tour definition actions. It never masquerades as a loaded empty source file.
- The missing-path issue remains detectable both before and after attempting to visit the step. Unresolved anchors show a persistent warning and identify when a fallback location is used.

### Change 2: make the lesson readable at every size

Scope: F3–F5 and F10. Render body content separately from pinned progress/view/navigation chrome, use rendered reading by default, remove reading-mode line numbers, compact the bottom status message, and implement short-terminal layout behavior. Start with the horizontal and compact modes; add wide side-by-side after those pass.

Primary code: [`src/render.rs`](src/render.rs), [`src/tour.rs`](src/tour.rs), [`src/tour_visuals.rs`](src/tour_visuals.rs), existing pane layout and mouse handling.

Acceptance:

- At 40×12, the reader sees step number, pane identity, and at least four lesson rows after focusing the lesson.
- At 80×24, 120×30, and 180×50, progress and essential navigation remain visible after scrolling to the middle or end.
- Long titles are ellipsized without hiding step number or Next/Prev. Steps without Diff/Flow do not show unavailable controls.
- Switching raw/rendered mode preserves edits and independent source/lesson positions. Generated command examples retain their arguments.
- Resize and mouse-divider behavior preserve focus and useful pane positions; existing splits restore correctly when the tour ends.

### Change 3: improve picking, recovery, and finishing

Scope: F6–F9. Add clean summaries and source locations to step entries, expose resume and health in the tours list, add actionable empty/error states, and show an explicit final-step state.

Primary code: [`src/tour.rs`](src/tour.rs), [`src/results.rs`](src/results.rs), tour-specific rendering.

Acceptance:

- A multiline Markdown description produces one clean step row, with a meaningful summary and source location. The current step is selected on opening the picker.
- The tours list exposes `3 steps`, invalid/empty status, and `Resume step 2/3` when applicable. Starting over remains explicit.
- Listing a large collection remains responsive; parse/validation results should be cached or loaded in the background rather than adding synchronous Git or full anchor checking to every row.
- Empty projects expose `:tournew` and a minimal sample tour. Invalid JSON reports filename, line/column, and an edit/retry action; absent tour names and empty step arrays have different messages.
- Next at the final step clearly offers Finish, Restart, and Choose tour. Finish restores the existing editor panes and preserves source edits.

### Change 4: polish navigation and lock in behavior

Scope: test the wide layout, matching mouse/keyboard actions, compact help, focus labels, and highlight labels across themes. Add an optional outline only if long-tour testing shows the existing improved picker is insufficient.

Acceptance:

- Visible view/navigation controls have mouse targets and keyboard equivalents; full labels remain available when inline source callouts are clipped.
- Reading does not accidentally modify source or explanations. Help states the current focus, view, scratch lifetime, and relevant keys.
- All six current PTY suites still pass after updating assertions for intentional presentation changes. Add focused regressions for the reproduced defects and pinned/compact geometry; avoid screenshot tests for incidental styling.
- Recheck the 12 themes and both palettes, including the existing ≥4.5 contrast assertions for tour highlights and diff text. Selected state, focus, and warnings remain understandable without color.
- Benchmark movement, step changes, and Markdown navigation against the current tour latency fixtures; investigate any material regression before adding more persistent UI.

## Follow-up test matrix

| Scenario | Sizes and content | Required outcome |
| --- | --- | --- |
| Read and orient | 40×12, 80×24, 120×30, 180×50; short/long title; short/long lesson | Visible step and useful body; persistent controls; sensible wrapping |
| Switch context | Source/Diff/Flow/raw/rendered; both focus panes | Correct active view; stable note, focus, and viewport |
| Navigate | First/last; direct step jump; cross-file highlight; end/resume | Clear boundaries; correct source; no unexpected pane reset |
| Recover | No tours; empty/invalid JSON; missing file; moved/ambiguous anchor | Specific error and a usable next action |
| Use mouse and resize | Click focus; view actions; divider drag; shrink/expand | Keyboard-equivalent actions; usable small layout; restored geometry |
| Theme and latency | Existing 12 themes, stock/pale palettes, large source fixtures | Existing contrast and responsiveness retained |

## Reproduce the exploratory audit

```bash
python3 -m venv /tmp/vaayu-tour-audit-venv
/tmp/vaayu-tour-audit-venv/bin/python -m pip install -r tests/requirements.txt
cargo build --release --locked --bin vaayu
/tmp/vaayu-tour-audit-venv/bin/python artifacts/tours-ui-audit-2026-10-13/audit_driver.py target/release/vaayu
```

The driver isolates config/data/state and project fixtures, uses the existing PTY capture helper, and overwrites its evidence captures on rerun. Binary hash, commit, input sequences, dimensions, and observations are recorded in `exploratory-results.json`. The regression JSON and adjacent per-suite logs record this audit's separate existing-suite run.
