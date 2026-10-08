# Configuration reference

Reviewed against `4f1b81d` on 2026-10-08. Settings load from `~/.config/vaayu/config.toml` on Linux (the platform config directory elsewhere). Copy [config.example.toml](config.example.toml) as a starting point. Fields are optional through serde defaults; the example intentionally customizes `statusline`, while the built-in default is empty. `:configreload` reloads settings and restarts language servers. Unsupported runtime `:set` options report an error; not every TOML field has a `:set` spelling. See [config](src/config.rs) and [runtime option dispatch](src/command.rs).

Top-level scalar/inline-table options must appear before any `[lsp.*]`, `[[autocmd]]`, `[[keymap]]`, or other TOML table; otherwise TOML assigns them to that table. Commands use argv arrays, with one array element per argument, including paths containing spaces.

## AI CLI selection

```toml
ai_agent = "codex"
agent_commands = { codex = ["/absolute/path/to/codex"], custom = ["/absolute/path/to/agent", "--interactive"] }
```

`ai_agent` defaults to `"claude"`; empty/whitespace-only values select Claude. An unconfigured name launches from `PATH`. `:ai` / `,ca` and tour generation/explanation use the selected name; explicit `:claude`, `:codex`, and `:agent <name>` target their own name. Prompts are bracketed-pasted; press Enter in the CLI to submit. See [context](src/context.rs), [PTY](src/pty.rs), and [tour](src/tour.rs). `review_command` is a separate JSON-on-stdin job interface, bounded by `review_timeout_secs`.

## Display and previews

`mermaid_preview` accepts `"auto"`, `"unicode"`, `"kitty"`, and `"off"`. Auto negotiates Kitty Unicode placeholders, with Unicode/source fallback; tmux/screen and WezTerm use Unicode automatically. Kitty mode forces the required protocol. `h/l` pans diagrams, `+/-` changes graphical zoom in 25-point steps from 50% to 300%, and `0` resets zoom/pan. These options apply to full and split Markdown preview. See [Mermaid report](MERMAID_PREVIEW_PLAN.md).

Custom `statusline` supports `%f`/`%F` name, `%l` line, `%c` column, `%L` line count, `%m` modified flag, `%y` filetype, `%p` percentage, `%M` mode, `%C` enclosing-symbol breadcrumb, and `%%`. Unknown tokens pass through. `%C` uses tree-sitter, or the cached outline for a file without a grammar; the right-side ruler remains. See [statusline expansion](src/render.rs).

`:colorscheme` previews available themes in a picker; Enter applies, Esc restores. Built-ins are `default`, `mono`, `warm`, `cool`, `gruvbox`, `gruvbox-light`, `flexoki`, `flexoki-light`, `tokyonight`, and `tokyonight-day`. `transparent = true` preserves the terminal background under palette themes.

## Complete top-level setting inventory

Names, types, and defaults below are extracted from `Config` and its `Default` implementation in [src/config.rs](src/config.rs). Type names are Rust types; default values use their equivalent TOML notation. Brief notes quote the field's source comment where present; a missing note means the source has no field comment. Full workflows are in [GUIDE.md](GUIDE.md) and [HELP.md](HELP.md).

| Setting | Type | Default | Source comment |
| --- | --- | --- | --- |
| `review_command` | `Vec<String>` | `[]` | — |
| `review_timeout_secs` | `u64` | `300` | — |
| `ai_agent` | `String` | `"claude"` | Named CLI used by AI prompts and code tours. Defaults to `claude`; `agent_commands` can override its executable path and arguments. |
| `agent_commands` | `std::collections::BTreeMap<String, Vec<String>>` | `{}` | `:claude`/`:codex`/`:agent <name>`: argv for each named long-lived agent terminal session. A name with no entry here falls back to running its own bare name as the command (so `:claude`/`:codex` work out of the box when those CLIs are already on `PATH`, without requiring any config at all); this map only exists for overriding that (a wrapper script, extra flags, a different binary name entirely). |
| `lsp` | `std::collections::BTreeMap<String, LspServer>` | `{}` | — |
| `leader` | `String` | `","` | — |
| `tabstop` | `usize` | `4` | — |
| `shiftwidth` | `usize` | `4` | — |
| `expandtab` | `bool` | `true` | — |
| `smartindent` | `bool` | `true` | When on (the default), a new line opened by Enter/`o`/`O` after a line ending in an opening bracket (`{`/`(`/`[`) gains one extra indent level. When off, the new line only copies the source line's indent. |
| `illuminate` | `bool` | `true` | Automatically highlight other occurrences of the symbol under the cursor (LSP `documentHighlight`) once it rests for `updatetime_ms`. Default on; silently does nothing without a capable language server. |
| `updatetime_ms` | `u64` | `250` | Idle time (ms) the cursor must rest before the `CursorHold` event fires (and, if enabled, illuminate requests document highlights). |
| `textwidth` | `usize` | `0` | Column width used by the `gq`/`gw` reflow operator. 0 (the default, matching Vim's `textwidth=0`) falls back to 79 columns when reflowing. |
| `number` | `bool` | `true` | — |
| `relativenumber` | `bool` | `false` | — |
| `cursorline` | `bool` | `false` | Tint the background of the line the cursor is on (Vim's `cursorline`). Only the active window's cursor line; default off. |
| `colorcolumn` | `usize` | `0` | Draw a vertical ruler at this 1-based text column (Vim's `colorcolumn`). 0 (the default) disables it. |
| `list` | `bool` | `false` | Reveal tabs (`>`, `-`) and trailing whitespace (`·`) — Vim's `list`. Default off. |
| `listchars` | `String` | `"tab:>-,trail:·"` | Glyphs used by `list`, Vim-style `tab:xy,trail:z` (x=tab lead, y=tab fill, z=trailing-space marker). Default `tab:>-,trail:·`. |
| `fillchars` | `String` | `"eob:~,vert:│"` | UI fill glyphs, Vim-style `eob:x,vert:y` (x=end-of-buffer marker, y=vertical split separator). Default `eob:~,vert:│`. |
| `statuscolumn` | `String` | `""` | Gutter component order, space-separated from `fold`, `diag`, `git`, `num`. Empty (the default) uses the built-in `fold diag git num` order. |
| `rainbow` | `bool` | `false` | Colorize matching brackets `()[]{}` by nesting depth. Default off. |
| `colorswatch` | `bool` | `false` | Paint LSP `documentColor` literals as background swatch chips (their own RGB as the background, with a contrasting foreground) instead of only tinting the foreground. Needs document colors live (`,lC`). Default off. |
| `spell` | `bool` | `false` | Underline misspelled words inline (Vim's `spell`) and enable `]s`/`[s` navigation. Default off; needs a system/user dictionary. |
| `notifications` | `bool` | `false` | Mirror messages as transient toast notifications in the top-right corner (they fade after a few seconds). Default off. |
| `progress` | `bool` | `true` | Show running jobs (LSP `$/progress`, grep, git, `:make`, formatting, the file scan) as a fading stack in the bottom-right corner. Default on. |
| `mermaid_preview` | `crate::mermaid::Mode` | `"auto"` | Mermaid preview: auto probes terminal graphics; unicode, kitty, or off explicitly select text, images, or source. All rendering stays local. |
| `watch` | `bool` | `true` | Watch the project for changes on disk (inotify/FSEvents): the file tree refreshes, the file index re-scans and open buffers autoread without polling. Default on; off falls back to polling. |
| `autoread` | `bool` | `true` | Reload an open buffer when its file changes on disk and it has no unsaved edits (a modified buffer is only flagged). Default on. |
| `sticky_scroll` | `bool` | `false` | Pin the enclosing function/class declaration lines at the top of the pane when they've scrolled off (sticky scroll). Default off. |
| `minimap` | `bool` | `false` | Show a compressed file overview in a strip on the right of the pane (a minimap), with the current viewport region highlighted. Default off. |
| `refactor_preview` | `bool` | `false` | Preview an LSP rename's edits as a diff list and require `:renameapply` to commit, instead of applying the rename immediately. Default off. |
| `winbar` | `bool` | `false` | Show a winbar: a per-pane top row with the file path and (with a parsed tree) the enclosing symbol as a breadcrumb. Default off. |
| `global_statusline` | `bool` | `false` | Use one global statusline at the bottom for the active window instead of a status line per split (Vim's `laststatus=3`). Default off. |
| `tree_width` | `usize` | `32` | File tree sidebar (`,e`): width in cells, which edge it's pinned to (`"left"` or `"right"`), whether to draw Nerd Font file icons (needs a patched font; `false` falls back to plain arrows), and whether it follows the current buffer, revealing its file. |
| `tree_position` | `String` | `"left"` | — |
| `tree_icons` | `bool` | `true` | — |
| `tree_follow` | `bool` | `true` | — |
| `tree_sort` | `String` | `"name"` | Initial entry order in the file tree: `"name"` (default), `"type"` (extension), `"mtime"` (newest first) or `"size"` (largest first). `O` / `:treesort` change it for the session. |
| `foldcolumn` | `bool` | `false` | Show a foldcolumn: a one-cell gutter marker (`+` closed fold, `-` open fold start) indicating where folds are. Default off. |
| `format_on_save` | `bool` | `false` | Run LSP formatting before writing the buffer (format-on-save). Default off; a bounded synchronous pump falls back to a plain save on timeout. |
| `todo_highlight` | `bool` | `false` | Highlight TODO/FIXME/HACK/XXX/NOTE/BUG/WARNING keywords inside comments with a distinct color. Default off. |
| `ghost_text` | `bool` | `false` | Inline ghost-text suggestions (Copilot-style) from buffer context, shown dimmed after the cursor and accepted with Tab. Default off. |
| `large_file_kb` | `usize` | `5120` | Above this size (in KiB) a buffer enters "large-file mode": tree-sitter parsing, spell, TODO, and rainbow scans are skipped to stay responsive. 0 disables the cutoff (always full features). Default 5120 (5 MiB). |
| `statusline` | `String` | `""` | Custom statusline format (Vim-like `%f %l %c %m %y %p %M %C`). `%C` expands to the enclosing-symbol breadcrumb. Empty (the default) uses the built-in `MODE name [+]` layout. The `line:col` ruler is always shown on the right. |
| `colorscheme` | `String` | `"default"` | Colorscheme name (`default`, `mono`, `warm`, `cool`, `gruvbox`, `gruvbox-light`, `flexoki`, `flexoki-light`, `tokyonight`, `tokyonight-day`). See `src/theme.rs`. |
| `transparent` | `bool` | `false` | Keep the terminal's own background instead of the colorscheme's (`:set transparent`). Only matters for schemes that paint one. |
| `semantic_tokens` | `bool` | `false` | Overlay LSP semantic-token colors on top of tree-sitter highlighting. Default off; needs a server with `semanticTokensProvider`. |
| `restore_cursor` | `bool` | `true` | Remember each file's cursor position across sessions (stored per project in `.vaayu/shada.json`) and restore it when the file is reopened. Default on; VCS message files are always left at the top. |
| `scrolloff` | `usize` | `8` | — |
| `timeoutlen_ms` | `u64` | `300` | — |
| `whichkey_delay_ms` | `u64` | `500` | Delay before the which-key prefix popup appears after a leader sequence like `,l` is typed with no continuation yet. Kept separate from `timeoutlen_ms`: unlike `jk` escape or ambiguous-motion timeouts, showing this popup never changes what a completed mapping does, so it can default independently. |
| `wrap` | `bool` | `true` | — |
| `showbreak` | `String` | `""` | The marker shown in the gutter of soft-wrapped continuation rows (Vim's `showbreak`). Empty -- the default -- shows nothing, so a wrapped line's continued rows have a blank gutter; set e.g. "↪" or "> " to mark them. Only applies when `wrap` is on. |
| `swap_0_and_caret` | `bool` | `true` | — |
| `jk_escape` | `bool` | `true` | — |
| `ignorecase` | `bool` | `true` | — |
| `smartcase` | `bool` | `true` | — |
| `clipboard_unnamedplus` | `bool` | `true` | Mirrors `vim.opt.clipboard = "unnamedplus"`: the unnamed register reads/writes the system clipboard, same as explicit `"+`/`"*` always do. Set false to keep yanks purely internal, like plain Vim. |
| `autopairs` | `bool` | `true` | — |
| `completion_enabled` | `bool` | `true` | Disables the completion popup (both the buffer-word and LSP sources) entirely when false; `update_completion` becomes a no-op. Manual insertion still works fine -- this only stops the automatic as-you-type popup, matching an editor-wide "I find this distracting" preference rather than per-source tuning. |
| `completion_delay_ms` | `u64` | `0` | How long the completion popup waits, after the triggering keystroke, before actually appearing -- `0` (the default) shows it instantly, matching this editor's existing behavior. The candidates are still computed immediately either way; this only gates when the popup is *painted*, the same render-time delay technique `whichkey_delay_ms` already uses, so raising it doesn't change what shows up, only how long a fast typist goes without the popup flashing in and out on every keystroke. |
| `diagnostics_virtual_text` | `bool` | `true` | Whether a line's real (non-gutter-marker) diagnostic message shows as virtual text after its own content, the current line only (to avoid cluttering every line with an error/warning tail). The gutter's E/W/I marker and the underline on the diagnostic's own range are unaffected by this -- only the extra text tail. |
| `diagnostics_update_in_insert` | `bool` | `false` | Whether newly published diagnostics update what's shown while in Insert mode. `false` (the default, matching Neovim's own default) means new diagnostics are still recorded but the visible set doesn't change mid-typing -- it catches up the moment Insert mode ends -- so a fast typist isn't distracted by error underlines/messages flickering on every keystroke. |
| `trim_trailing_whitespace` | `bool` | `false` | On `:w`, remove trailing spaces/tabs from every line (opt-in). |
| `insert_final_newline` | `bool` | `false` | On `:w`, ensure a non-empty buffer ends with exactly one `\n` (opt-in). |
| `autocmd` | `Vec<Autocmd>` | `[]` | Declarative autocommands: run an Ex `command` when `event` fires on a buffer whose name matches the glob `pattern` (default `*`). Parsed from `[[autocmd]]` tables. See `event.rs` for the supported event names. |
| `keymap` | `Vec<KeymapCfg>` | `[]` | User key remaps, parsed from `[[keymap]]` tables. See `keymap.rs`. |
| `conceal` | `bool` | `false` | Conceal: replace regex matches with a single char (or hide them) on every line except the one the cursor is on. Toggled with `:set conceal`; the rules come from `[[conceal_rules]]` tables. Default off. |
| `conceal_rules` | `Vec<ConcealRule>` | `[]` | — |

## Language-server tables

The quoted `ghost_text` field comment above still says Tab. Current Insert dispatch uses **Ctrl-L** to accept a ghost suggestion; Tab is reserved for completion/snippet/indent behavior. See [the actual key handling](src/insert.rs) and [ghost-text history](ROADMAP.md#2026-09-23--inline-ghost-text-48-partial).

Named `[lsp.NAME]` tables can share filetypes and supply multiple servers. Configured matching tables override language defaults; `enabled = false` disables that match. The generic `LspServer` defaults below are distinct from the per-language launch defaults in [lsp/mod.rs](src/lsp/mod.rs).

| Field | Default | Meaning |
| --- | --- | --- |
| `cmd` | `[]` | Executable and arguments; configure a usable command for a named server. |
| `filetypes` | `[]` | Matched editor filetypes. |
| `root_markers` | `["Cargo.toml", "pyproject.toml", "package.json", "go.mod", ".git"]` | Root discovery markers. |
| `settings` | `{}` | Server configuration payload. |
| `init_options` | null/unset | Initialization options. |
| `capabilities` | null/unset | Client capability overrides. |
| `env` | `{}` | Additional child environment variables. |
| `request_timeout_ms` | `15000` | Initialization/request deadline; effective values clamp to 100–300000 ms. |
| `enabled` | `true` | Enable this matching configuration. |

JSON/YAML servers receive bundled SchemaStore associations unless explicit schema settings replace them. Lua receives the built-in Neovim metadata profile where applicable. See [SchemaStore](src/schemastore.rs), [language client](src/lsp/client.rs), and [example server configurations](config.example.toml).

## Array tables and indentation precedence

`[[autocmd]]` has `event` (default empty), `pattern` (default `"*"`), and `command` (default empty). Recognized names are `BufWritePre`, `BufWritePost`, `BufEnter`, `InsertLeave`, `FocusGained`, `CursorHold`, and `FileType`; the first six have runtime firing paths, while `FileType` remains reserved. Save hooks run before and after writes; task documents are validated around pre-save hooks. See [events](src/events.rs) and [save handling](src/notes.rs).

`[[keymap]]` has `mode` (default `"n"`, combinations of `n`, `i`, `v`), `lhs`, and `rhs` (both default empty). Key notation includes `<leader>`, `<C-x>`, `<CR>`, `<Esc>`, `<Tab>`, `<BS>`, `<Space>`, arrows, and `<lt>`. A `rhs` beginning with `:` is an Ex command; other values replay nonrecursive keys. See [keymap](src/keymap.rs).

`[[conceal_rules]]` has `pattern` and `cchar` (both default empty). With `conceal = true`, a match is replaced by `cchar`'s first character or hidden if empty, except on the cursor line. See [renderer](src/render.rs).

Indentation precedence is a Vim modeline, applicable `.editorconfig`, detected file indentation, then config defaults. `:indentinfo` reports the resolved values and origin. EditorConfig also supplies supported charset, line endings, trimming, final-newline, and maximum-line-length settings. See [indent](src/indent.rs) and [buffer](src/buffer.rs).
