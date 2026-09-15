# Vaayu

A modal terminal editor in Rust: Vim-style editing, project review notes,
language servers, searchable results, and Markdown preview in one compiled core.

## Build and run

```sh
cargo install --path . --locked
vaayu path/to/file       # `vy` is an equivalent command
```

Install `ripgrep` for project file discovery and live grep, Git for Git features,
and the language servers you configure. Clipboard integration uses `wl-copy` /
`wl-paste` or X11 tools; SSH copying can use OSC52.

## Private review notes

Use `,rc` on a line or Visual selection, or `,rf` for a file comment. Edit the
comment as a normal buffer and press **Ctrl-S** or `:w`. `,rl` opens the review
list: `e` edits, Enter visits the source, `d` deletes, Tab/Space selects,
`y` copies selected/current notes, and `Y` copies everything. `,rw` saves all
open comments, deletions, and updated anchors.

Notes live in `.vaayu/comments.json` under the launch working directory, with owner-only directory
and file permissions and a local Git ignore rule. They do not modify source
files. Unique source-text anchors follow moved lines; ambiguous or missing
anchors are marked stale. Concurrent note-store changes are detected before
saving. This is local OS-account privacy, not encryption.

## Navigation and review

| Action | Binding / command |
| --- | --- |
| File picker / recent files / buffers | Ctrl-P / `,fr` / `,b` |
| Live grep | `,/` or `:grep pattern` |
| Convert current output to quickfix | **Ctrl-Q** |
| Open quickfix / next / previous | `,cq` / `:cn` / `:cp` |
| Search results | `/`, `?`, `n`, `N` |
| Marks / jumplist | `ma`, `'a`, `` `a `` / Ctrl-O, Ctrl-I |
| Diagnostics / next / previous | `,ld` / `]d` / `[d` |
| Definition / references / outline | `gd` / `,lR` / `,lo` |
| Format / rename / code actions | `,lf` / `:rename name` / `,la` |
| Vertical / horizontal split | Ctrl-W v / Ctrl-W s |
| Focus / close / only pane | Ctrl-W w / Ctrl-W c / Ctrl-W o |
| Side-by-side / full Markdown preview | `,ms` / `,mp` |
| Toggle soft wrap | `,ow` or `:set wrap` / `:set nowrap` |
| Git changes / blame / stage / unstage | `:gitdiff` / `:gitblame` / `:gitstage` / `:gitunstage` |
| Recover interrupted-session drafts | `:recover` |

Results share selection, clipboard export, location navigation and quickfix
conversion. Git staging lists saved-file hunks; Enter applies one hunk after
checking it still applies. Formatting and language-server edits remain unsaved
and undoable. Recovery snapshots are written privately after an idle interval;
explicit saves remain essential.

## Editing and display

Normal, Insert, character/line/block Visual, operators, motions, text objects,
registers, undo/redo, bounded macros, dot-repeat, regex search and substitution.
Bracketed paste inserts literal text. Saves use atomic replacement and detect
external changes. Quit checks unsaved buffers.

Tree-sitter highlights Rust, Python, JavaScript, TypeScript/TSX, Go, C, Bash,
JSON, TOML, YAML and Lua. Markdown renders tables, nested lists, styles, links
and highlighted code fences. Display handles tabs, wide characters and
combining graphemes; cached rows avoid redrawing unchanged content.

## Configuration

Copy [config.example.toml](config.example.toml) to
`~/.config/vaayu/config.toml`. All fields are optional. Named LSP configurations
accept `cmd` argv, `filetypes`, `root_markers`, `env`, `init_options`, `settings`,
and capability overrides. Multiple servers can serve the same language.
Use `:configreload` to reload settings and restart servers.

`:help` opens the [keymap guide](HELP.md). See [AUDIT.md](AUDIT.md) for the
re-audit, verified fixes and limitations, and [BENCHMARKS.md](BENCHMARKS.md) for
performance measurements. Vaayu implements a useful Vim subset; it is not a
complete Vim emulator or a Lua-plugin host.

## Validation

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo build --release --bins --locked
python3 -m pip install -r tests/requirements.txt
python3 tests/pty_regression.py target/release/vaayu
```
