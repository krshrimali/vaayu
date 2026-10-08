# Mermaid preview feasibility and implementation

Status: implemented. The native-renderer prototype passed, and the graphical path was added alongside Unicode after approval to proceed with real visual diagrams.

## Feasibility decision

Vaayu's existing pulldown-cmark parser and styled-row compositor provide suitable integration points. Mermaid fences can be collected without replacing the Markdown parser. Unicode diagrams fit the existing renderer when their rows remain fixed-width. Actual graphical diagrams need terminal image support, image lifetime management, and cell-aware clipping.

Use [Merman 0.8.0](https://docs.rs/merman/0.8.0/merman/) with defaults disabled and the `all-diagrams`, `ascii`, `png`, and `layout-cytoscape` features. It provides native parsing, layout, Unicode, SVG/PNG export, cooperative cancellation, and resource policies. Its MIT/Apache licensing fits this project. The smaller graphs-tui alternative was rejected after its published AGPL license was checked. No new Mermaid parser was written.

The native approach works locally without Node, Chromium, helper processes, or network access. The [official Mermaid CLI](https://github.com/mermaid-js/mermaid-cli) remains a possible future compatibility/export backend if documents need syntax beyond native support.

The prototype rendered the existing runtime-flow fixture, including branching and a cycle. A small prototype flowchart took approximately 5 ms for Unicode and 825 ms for its first PNG in an unoptimized build. The six-diagram graphical gallery then passed in both light and dark palettes, with warmed unoptimized renders generally taking hundreds of milliseconds. These measurements establish feasibility; they are not a guarantee for arbitrary graph sizes.

Enabling native graphics increased the release executable from approximately 17 MiB to 37 MiB on this Linux build. The first release build after adding the dependencies took about four minutes. That is the main build/distribution cost of avoiding a separate browser runtime.

## Delivered behavior

- Full-screen `,mp` and split `,ms` / `:vpreview` recognize `mermaid` as the first fence-info token, including backtick/tilde fences and fences nested in lists or quotes.
- `mermaid_preview = "auto"` probes terminal graphics support and selects themed PNG diagrams when supported. Terminals without image support receive Unicode diagrams where a complete layout is available.
- The image backend uses [Kitty Unicode placeholders](https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders). Image cells travel through the normal text-row compositor, which clips them to panes and lets dialogs cover them. Uploads and dimension-specific placements are reused and freed on close, resize, or exit.
- Automatic mode uses Unicode through tmux/screen because passthrough is not implemented. It also uses Unicode in WezTerm: its [graphics tracking issue](https://github.com/wezterm/wezterm/issues/986) distinguishes basic image support from the required Unicode-placeholder support. `kitty` mode explicitly forces graphics for a terminal known to support placeholders.
- `unicode` forces text diagrams; `off` preserves fenced source. Incomplete, unsupported, over-budget, or degraded text layouts show source with a concise reason instead of silently dropping graph structure.
- Diagram rows retain fixed geometry while prose wraps normally. Both preview modes support `h/l` panning, `+/-` graphical zoom from 50% to 300%, and `0` to reset. Scroll bounds keep the final page visible, including documents ending in a diagram.
- PNG canvases, labels, borders, and table rows follow the editor's light/dark palette. Aspect ratio and terminal cell dimensions determine the display size; diagrams are centered when they fit.

## Architecture and limits

`src/mermaid.rs` owns the native adapter and a single lazy worker. Diagram requests debounce for 150 ms; input and painting never wait for layout. Idle polling repaints completed work without a keypress. Obsolete requests are cancelled and stale results cannot replace current-source artifacts.

Artifacts are shared by source, palette, and output mode. Prose edits, scrolling, zoom, and different pane widths reuse the rendered artifact. Wrapped document layouts are cached independently by buffer, pane width, and zoom, so differently sized panes do not alternate through one cache slot. Cancelling pending work invalidates saved layouts; reopening a pane requests it again, and replies from cancelled workers cannot replace a reopened request even when the source is identical.

The adapter caps source at 64 KiB per diagram, parsed model items at 800, cached artifacts at 64 entries / 32 MiB, and individual PNGs at 16 MiB. Text output has finite byte/grid budgets. PNG rasterization fits a 1600×2200 logical-pixel box at 1.5× scale; terminal placements are capped at 256 cells in each direction. Rendering uses a three-second cooperative deadline and constrained resource policies. This is cooperative cancellation, not a process-level kill guarantee.

Strict parsing/security configuration and the safe SVG pipeline disable interactive HTML labels and external resource loading. Terminal control characters are removed from text/error output. Graphics capability replies are recognized by the application's query ID; unrelated keyboard and mouse events are preserved. Image-ID colors remain functional even when `NO_COLOR` disables ordinary UI colors.

## Validation

- Native tests render flowchart, sequence, state, class, entity-relationship, and pie diagrams in dark and light themes. They decode the PNGs, check canvas colors/dimensions and budgets, and check Unicode labels and the repository's runtime-flow fixture.
- Parser/layout tests cover nested fences, ordinary highlighted code, fixed diagram geometry, and readable source fallback. Worker tests cover artifact reuse and rejection of stale completions. Graphics tests cover reply filtering, cropping coordinates, and unique placement IDs.
- `tests/pty_mermaid.py` exercises full-screen/split previews at narrow and wider sizes, reopening a preview before its debounce expires, idle completion, rapid edits, resizing, scroll/pan bounds, negotiated PNG uploads, zoom/cache reuse, closing images, `NO_COLOR`, WezTerm fallback, and disabled mode.
- Actual Kitty 0.49.1 was run in an isolated Xvfb desktop for visual inspection of graphical shapes, labels, clipping, scrolling, zoom, split panes, and cleanup. PTY text emulation alone cannot establish that images are visible.
- Final checks passed: formatting, strict Clippy, locked release builds, 834 tests in each Rust binary (three ignored), all six Mermaid PTY scenarios against the release binary, and eight existing UI/preview/session/resize PTY suites. Rust tests run from a fresh project directory with `NO_COLOR` unset: the repository's local `.vaayu/shada.json` otherwise seeds the jump-list fixture, and `NO_COLOR` suppresses the SGR colors expected by existing color tests.

## Practical limits and follow-ups

Graphical support is verified in Kitty. Other compatible terminals still need their own visual checks. tmux passthrough, Sixel/iTerm graphics, browser/export integration, and ordinary Markdown-image rendering are separate follow-ups.

Native coverage is not complete Mermaid.js compatibility. Unsupported families or syntax retain source. Unicode styles/shapes are approximations, and some graphical families have no complete Unicode layout. The six-family gallery in `examples/mermaid.md` is the starting acceptance set.

Full-screen refresh keeps a matching text-row anchor where possible; split panes clamp their stored positions. Precise source-range anchoring through arbitrary document edits remains a follow-up.
