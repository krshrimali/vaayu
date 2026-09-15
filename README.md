# Anvil

A modal terminal text editor with Vim's `operator + motion` grammar, written
in Rust with a rope buffer and a compiled input FSM -- no plugin runtime, no
interpreter hop between a keystroke and the screen.

This is the first working milestone (M0 + M1) of the deeper plan in
`~/.config/nvim`'s companion design doc: keep Vim's editing model and this
config's exact muscle memory, replace the Lua/plugin-manager runtime
underneath it with a single compiled core. LSP, tree-sitter, a fuzzy picker,
git integration, and the remote-latency layer are **not** in this build yet --
see [Status](#status) below for exactly what is and isn't here.

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
(`.`); search (`/`, `?`, `n`, `N`, regex, smartcase); `:s` and `:%s`
substitution; multi-buffer `:e`/`:bn`/`:bp`/`:b<N>`; the `jk` insert-mode
escape with real timing (matches `timeoutlen`); and this config's leader
bindings that don't need a missing subsystem (`,w` `,q` `,Q` `,h` `,d` `,ow`
`,or`).

## What's stubbed

Leader sequences that need a subsystem this build doesn't have yet --
`,ff`/`,fr` (fuzzy picker), `,e` (file explorer), `,/` (live grep), `,b`
(buffer picker), `,z` (zen mode), `,R` (config hot-reload) -- print a message
naming what's missing instead of silently doing nothing. LSP, tree-sitter
highlighting, git gutter/blame, and the SSH-latency prediction layer are
future milestones, not partial implementations here.

## Status

Milestone M0 (rope buffer, damage-simple renderer, modal FSM) and M1 (full
Vim grammar, matching this config's `keymaps.lua`) are done and covered by a
pty-driven regression pass (motions, operators, text objects, visual mode,
counts, macros, dot-repeat, undo/redo, search, indent, `:s`, leader saves).
M2 onward (LSP, tree-sitter, picker, git, remote layer) are not started.
