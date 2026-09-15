# Anvil

A modal terminal text editor with Vim's `operator + motion` grammar, written
in Rust with a rope buffer and a compiled input FSM -- no plugin runtime, no
interpreter hop between a keystroke and the screen.

This is the first working milestone (M0 + M1, plus a first slice of M3) of
the deeper plan in `~/.config/nvim`'s companion design doc: keep Vim's
editing model and this config's exact muscle memory, replace the
Lua/plugin-manager runtime underneath it with a single compiled core. LSP,
tree-sitter, git integration, and the remote-latency layer are **not** in
this build yet -- see [Status](#status) below for exactly what is and isn't
here.

## Build & install

```sh
cargo build --release
cargo install --path .        # installs `anvil` to ~/.cargo/bin
```

## Run

```sh
anvil path/to/file
```

## Config

Optional, at `~/.config/anvil/config.toml`. A starter file matching this
machine's `~/.config/nvim` options (leader `,`, `jk` to escape, swapped
`0`/`^`, `scrolloff = 8`, 4-space indent) is already in place -- edit it
directly. Every field is optional; see `src/config.rs` for the full list and
defaults.

## What works

Normal, Insert, Visual (char + line), and Command-line modes; the full
operator+motion grammar (`d`/`c`/`y`/`>`/`<` with `w b e W B E 0 ^ $ gg G f F
t T { } h j k l`, plus counts and doubled linewise forms like `dd`/`cc`/`yy`);
text objects (`iw aw i( a( i{ a{ i[ a[ i< a< i" a" i' a' i\` a\``); registers
(named + unnamed, `"_` blackhole); undo/redo; macros (`q`/`@`); dot-repeat
(`.`); search (`/`, `?`, `n`, `N`, regex, smartcase, Vim-dialect patterns like
`\( \)`/`\1`/`\< \>`); `:s` and `:%s` substitution (same Vim-regex
translation); multi-buffer `:e`/`:bn`/`:bp`/`:b<N>`/`:bd`; the `jk` insert-mode
escape with real timing (matches `timeoutlen`); a fuzzy file picker on
`Ctrl-P` or `,ff`/`,fr` (recursive scan skipping `.git`/`target`/
`node_modules`, subsequence fuzzy match, arrows or `^n`/`^p` to move, Enter
to open); and this config's leader bindings that don't need a missing
subsystem (`,w` `,q` `,Q` `,h` `,d` `,ow` `,or`).

## What's stubbed

`Ctrl-v` (visual block) and leader sequences that need a subsystem this
build doesn't have yet -- `,e` (file explorer), `,/` (live grep), `,b`
(buffer picker), `,z` (zen mode), `,R` (config hot-reload) -- print a message
naming what's missing instead of silently doing nothing. The Vim-regex
translation for `/` and `:s` covers `\( \) \{ \} \+ \? \|` and `\< \>`, not
the full dialect (no `\v`, `\%(`, etc). LSP, tree-sitter highlighting, git
gutter/blame, and the SSH-latency prediction layer are future milestones,
not partial implementations here.

## Status

Milestone M0 (rope buffer, damage-simple renderer, modal FSM) and M1 (full
Vim grammar, matching this config's `keymaps.lua`) are done, plus a first
slice of M3 (the file picker). All of it is covered by a pty-driven
regression pass (motions, operators, text objects, visual mode, counts,
macros, dot-repeat, undo/redo, search, indent, `:s`, leader saves, the
picker's open/cancel paths). LSP, tree-sitter, git, and the remote layer are
not started.
