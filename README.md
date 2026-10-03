# Vaayu

A modal terminal editor in Rust: Vim-style editing, project review notes,
language servers, searchable results, and Markdown preview in one compiled core.

## Build and run

```sh
cargo install --path . --locked
vaayu path/to/file       # `vy` is an equivalent command
```

Install `ripgrep` for project file discovery and live grep, Git for Git features,
the GitHub CLI (`gh`, logged in) for the GitHub workspace, and the language
servers you configure. Clipboard integration uses `wl-copy` /
`wl-paste` or X11 tools; SSH copying can use OSC52.

## Private review notes

Use `,rc` on a line or Visual selection, or `,rf` for a file comment. Edit the
comment as a normal buffer and press **Ctrl-S** or `:w`. `,rl` opens the review
list: `e` edits, Enter visits the source, `d` deletes, Tab/Space selects,
`y` copies selected/current notes, and `Y` copies everything. `,rw` saves all
open comments, deletions, and updated anchors.

Notes live in `.vaayu/comments.json` under the launch working directory, with owner-only directory
and file permissions and a local Git ignore rule. They do not modify source
files. Unique source-text anchors follow moved lines and whitespace-only changes; ambiguous or missing
anchors are marked stale. OS file locks serialize note writes; changed snapshots are detected before
saving. Unsaved edited comments can be recovered with `:recover`. This is local OS-account privacy, not encryption.

## Navigation and review

| Action | Binding / command |
| --- | --- |
| File picker / recent files / buffers | Ctrl-P / `,fr` / `,b` |
| File tree explorer (open / focus / close; `?` inside for keys) | `,e` |
| Tree: sort / floating preview / stage-unstage / restore from trash | `O` (`:treesort`) / `v` / `gs` `gu` / `U` (`:treetrash`) |
| Live grep | `,/` or `:grep pattern` |
| Reviewed project-wide search & replace (toggle matches, `R` applies, `U` undoes) | `,sr` / `:far`; `,sw` for the word under cursor |
| Run nearest test / file / suite / last (failures → quickfix, ✓/✗ gutter marks) | `,Tn` / `,Tf` / `,Ts` / `,Tl` (or `:test …`) |
| Convert current output to quickfix | **Ctrl-Q** |
| Open quickfix / next / previous | `,cq` / `:cn` / `:cp` |
| Search results | `/`, `?`, `n`, `N` |
| Marks / jumplist | `ma`, `'a`, `` `a `` / Ctrl-O, Ctrl-I |
| Diagnostics / next / previous | `,ld` / `]d` / `[d` |
| Definition / references / outline | `gd` / `,lR` / `,lo` |
| Peek definition / type / implementation / references in a float | `gpd` / `gpt` / `gpi` / `gpr` (or `,pd` …) |
| Format / rename / code actions | `,lf` / `:rename name` / `,la` |
| Vertical / horizontal split | Ctrl-W v / Ctrl-W s |
| Focus / close / only pane | Ctrl-W w / Ctrl-W c / Ctrl-W o |
| Side-by-side / full Markdown preview | `,ms` / `,mp` |
| Toggle soft wrap | `,ow` or `:set wrap` / `:set nowrap` |
| Git changes / blame / stage / unstage | `:gitdiff` / `:gitblame` / `:gitstage` / `:gitunstage` |
| GitHub PRs / current PR / diff / review threads / CI checks / issues | `,Gp` / `,Gv` / `,Gd` / `,Gr` / `,Gc` / `,Gi` (`:ghprs`, `:ghpr`, `:ghcheckout N`, …) |
| Recover interrupted-session drafts | `:recover` |
| Save / restore recursive pane layout | `:sessionsave` / `:sessionload` |
| Export / run selected agent feedback | `:reviewexport` / `A` in results |
| Agent output / cancellation | `:reviewresults` / `:reviewcancel` |
| Toggle resolved review notes | `R` in comments, then `,rw` to save |

Results share selection, clipboard export, location navigation and quickfix
conversion. Git staging lists saved-file hunks; Enter applies one hunk after
checking it still applies. The GitHub workspace (via `gh`) is read-first: PR
list and overview, checkout, diff, review threads and CI check logs (failed
steps first) and issues, each as a jumpable list. Formatting and language-server edits remain unsaved
and undoable. Recovery snapshots are written privately after an idle interval;
explicit saves remain essential.

## Editing and display

Normal, Insert, character/line/block Visual, operators, motions, text objects,
registers, undo/redo, bounded macros, dot-repeat, regex search and substitution.
Bracketed paste inserts literal text. Saves use atomic replacement and detect
external changes. Quit checks unsaved buffers. A filesystem watcher (`watch`,
default on) reloads unmodified buffers changed on disk (`autoread`), flags
modified ones `[changed on disk]` instead of clobbering them, and keeps the
file tree and file-picker index current without polling.
Multiple cursors: `Ctrl-N` adds one at the next occurrence of the word (or the
Visual selection), Visual-block `Ctrl-N` makes a column of them, `,ma` puts one
on every occurrence and `,mj`/`,mk` add one below/above; Normal-mode commands
and Insert-mode typing then apply at every cursor as one undo step, and `Esc`
collapses them.

Tree-sitter highlights Rust, Python, JavaScript, TypeScript/TSX, Go, C, Bash,
JSON, TOML, YAML and Lua. Markdown renders tables, nested lists, styles, links
and highlighted code fences. Display handles tabs, wide characters and
combining graphemes; horizontal motions and deletion respect grapheme boundaries,
and block operations use display columns. Cached rows avoid redrawing unchanged content.
Long-running jobs (LSP `$/progress`, grep, git, `:make`, test runs, formatting, the file
scan) show as a fidget-style stack in the bottom-right corner that fades out
when they finish (`progress = false` or `:set noprogress` to hide it).
`:colorscheme` switches between built-in themes -- the terminal-colored
`default`, `mono`, `warm`, `cool` and the true-color `gruvbox`, `flexoki` and
`tokyonight` palettes (each with a light variant: `gruvbox-light`,
`flexoki-light`, `tokyonight-day`) that theme the background and every UI
surface; `transparent = true` (or `:set transparent`) keeps the terminal's own
background under them.

## Configuration

Copy [config.example.toml](config.example.toml) to
`~/.config/vaayu/config.toml`. All fields are optional. Named LSP configurations
accept `cmd` argv, `filetypes`, `root_markers`, `env`, `init_options`, `settings`,
and capability overrides. Multiple servers can serve the same language.
Use `:configreload` to reload settings and restart servers. Requests have a
configurable `request_timeout_ms` deadline; `:lspcancel` cancels pending requests.
Completion resolution and common snippet placeholders, choices and linked
fields are supported. File resource edits support ordered create/rename/delete
of regular project files, with preflight checks and rollback on commit failure.

Search supports pattern backreferences, lookarounds and Vim magic/case switches,
with bounded backtracking. Mixed recursive splits support up to 32 panes.
`:sessionsave` persists named-file pane layout and positions; `:sessionload`
restores them without discarding existing buffers.

For agent review, configure top-level `review_command` as an argv array.
`:reviewexport` writes a versioned private JSON packet of selected/current
feedback. `A` in results (or `:reviewrun`) starts the configured command with
that packet on stdin, in the project folder. Its stdout/stderr are retained
privately; `:reviewresults` opens them. Execution is explicit, cancellable and
time-limited. Review completion does not automatically resolve notes.

`:help` opens the [keymap guide](HELP.md). See [AUDIT.md](AUDIT.md) for the
re-audit, verified fixes and limitations, and [BENCHMARKS.md](BENCHMARKS.md) for
performance measurements. Vaayu implements a useful Vim subset; it is not a
complete Vim emulator or a Lua-plugin host.

## Validation

The Rust toolchain is pinned in `rust-toolchain.toml` (rustup installs it
automatically, with `rustfmt` and `clippy`), so local checks match CI.

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo build --release --bins --locked
python3 -m pip install -r tests/requirements.txt
python3 tests/pty_regression.py target/release/vaayu
python3 tests/pty_extended.py target/release/vaayu
python3 tests/pty_ui.py target/release/vaayu
python3 tests/pty_typing_ui.py target/release/vaayu
cargo test real_clangd_formatting_and_diagnostics -- --ignored # requires clangd
```
