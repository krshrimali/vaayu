# Vaayu

A modal terminal text editor with Vim's `operator + motion` grammar, written
in Rust with a rope buffer and a compiled input FSM -- no plugin runtime, no
interpreter hop between a keystroke and the screen.

This is the deeper plan in `~/.config/nvim`'s companion design doc, in
progress: keep Vim's editing model and this config's exact muscle memory,
replace the Lua/plugin-manager runtime underneath it with a single compiled
core. See [Status](#status) for exactly what's real and what's a stub.

## Build & install

```sh
cargo build --release
cargo install --path .        # installs `vaayu` and `vy` to ~/.cargo/bin
```

## Run

```sh
vaayu path/to/file   # or: vy path/to/file
```

## Config

Optional, at `~/.config/vaayu/config.toml`. A starter file matching this
machine's `~/.config/nvim` options (leader `,`, `jk` to escape, swapped
`0`/`^`, `scrolloff = 8`, 4-space indent) is already in place -- edit it
directly. Every field is optional; see `src/config.rs` for the full list and
defaults.

## What works

**Core editing.** Normal, Insert, Visual (char + line), and Command-line
modes; the full operator+motion grammar (`d`/`c`/`y`/`>`/`<` with
`w b e W B E 0 ^ $ gg G f F t T { } h j k l`, plus counts, doubled linewise
forms like `dd`/`cc`/`yy`, and Vim's `cw`-acts-like-`ce` special case); text
objects (`iw aw i( a( i{ a{ i[ a[ i< a< i" a" i' a' i\` a\``); registers
(named + unnamed, `"_` blackhole); undo/redo; macros (`q`/`@`); dot-repeat
(`.`); search (`/`, `?`, `n`, `N`, regex, smartcase, Vim-dialect patterns like
`\( \)`/`\1`/`\< \>`); `:s`/`:%s` substitution (same Vim-regex translation);
multi-buffer `:e`/`:bn`/`:bp`/`:b<N>`/`:bd`; and the `jk` insert-mode escape
with real timing (matches `timeoutlen`).

**Navigation.** A fuzzy file picker on `Ctrl-P` or `,ff`/`,fr` (recursive
scan skipping `.git`/`target`/`node_modules`, subsequence fuzzy match,
arrows or `^n`/`^p` to move, Enter to open); this config's leader bindings
that don't need a missing subsystem (`,w` `,q` `,Q` `,h` `,d` `,ow` `,or`).

**Syntax highlighting.** Real tree-sitter parsing (comments, strings,
numbers, keywords) for Rust, Python, JavaScript/TypeScript, Go, C, Bash,
JSON, TOML, YAML, and Lua -- picked by file extension, reparsed only when the
buffer actually changes, output batched into one escape sequence per
contiguous styled run.

**Git gutter.** Added/modified/removed signs, live against the buffer's
*current, possibly-unsaved* text vs. the file's HEAD blob -- not just
post-save state. No sign column at all outside a git work tree.

**Autocompletion.** A live popup in Insert mode, sourced from buffer words
(always available) merged with real LSP completions when a language server
is running for the buffer. `Ctrl-n`/`Down` and `Ctrl-p`/`Up` cycle, `Tab`/
`Enter` accepts, `Esc` dismisses without leaving Insert mode. Each candidate
is tagged `buf`/`lsp` in the popup.

**LSP client.** Spawns a language server per filetype (best-effort command
list in `src/lsp/client.rs` -- `rust-analyzer`, `pylsp`/`pyright`,
`typescript-language-server`, `gopls`, `clangd`, `lua-language-server`,
`bash-language-server`, JSON/TOML/YAML servers; missing binaries just log
"no language server available", nothing crashes), speaks real JSON-RPC over
stdio on a background thread, and wires up: live diagnostics (gutter
`E`/`W`/`I`/`H`, `textDocument/didOpen`+`didChange` full-sync on every edit),
hover (`K`), goto-definition (`gd`, jumps across files via `:e` if needed),
and completion (feeding the popup above). The child server's own stdin
write and the periodic background-event poll are the only things not on the
"never blocks a keystroke" path yet -- see Known limitations.

Verified against real servers, not just compiled: `clangd` on a file with a
deliberate undefined-symbol error produced the diagnostic gutter mark within
~1.5s, `K` on a call expression returned real hover text, `gd` jumped the
cursor to the exact definition line/column, and the completion popup filled
with real clangd candidates (`INT16_MAX` and friends from `<stdint.h>`) --
end to end, not just "the request compiles."

## What's stubbed

`Ctrl-v` (visual block) and leader sequences that need a subsystem this
build doesn't have yet -- `,e` (file explorer), `,/` (live grep), `,b`
(buffer picker), `,z` (zen mode), `,R` (config hot-reload) -- print a message
naming what's missing instead of silently doing nothing. The Vim-regex
translation for `/` and `:s` covers `\( \) \{ \} \+ \? \|` and `\< \>`, not
the full dialect (no `\v`, `\%(`, etc). Syntax highlighting classifies by
node-kind substring and a literal keyword list (robust across grammar
versions, but doesn't color function/type names -- that needs per-grammar
query files, a later pass). LSP has no rename/code-action/references/
signature-help yet, and completion doesn't request resolve() for
lazily-filled detail. The SSH-latency prediction layer from the design doc
is not started.

## Known limitations

- **Git blob lookup blocks the main thread.** Finding the repo root and
  reading the HEAD blob shells out to `git` synchronously when a file opens
  (not on every keystroke -- cached after that), sitting in the same loop
  the design doc says nothing should block. Fine for typical repos; worth
  moving to a background thread alongside the LSP I/O.
- **LSP full-document sync.** `didChange` sends the whole buffer text on
  every edit rather than incremental ranges -- simpler and correct, but more
  bytes than necessary; matters more for the eventual remote-latency work
  than for local use.
- **One event loop, not one per server.** All LSP clients' background
  threads feed one process, but writes to a server's stdin happen
  synchronously from the main thread. A server that stops reading its stdin
  (hung, deadlocked) would stall the next keystroke that needs to reach it.
  Not observed against `clangd` in testing, but not structurally ruled out.

## Status

Rope buffer, renderer, and the full Vim grammar (M0+M1) are done. Syntax
highlighting and the git gutter (first slices of M2) are done. The file
picker (first slice of M3) is done. A working LSP client with diagnostics,
hover, goto-definition, and completion, plus a buffer-word+LSP
autocompletion engine, are done and verified against real servers
(`clangd`), not just against the protocol on paper.

Every feature above has a pty-driven regression pass behind it -- both logic
(saved-file content after a scripted key sequence) and, separately, the
actual rendered terminal output (gutter, statusline, visual-selection
reverse-video, search highlight, syntax colors, picker layout, completion
popup, diagnostic signs). That separation matters: an earlier pass in this
project's history only checked saved file content and missed that its own
test harness wasn't setting a terminal size, so rendering had silently never
been exercised. Bugs actually found and fixed by this testing discipline
so far: `cw` not special-casing like Vim's `ce`, `:q` staying open across
buffers instead of quitting, `Ctrl-v` silently swallowed instead of saying
"not implemented", `:s`/`/` not understanding Vim's regex dialect, escape
sequences emitted per-character instead of batched per styled run, and (the
big one) the main loop redrawing on a fixed idle timer regardless of whether
anything changed, which both wasted output and broke the same rendering
tests it was meant to keep honest.

Not started: rename/references/code-actions, git hunk stage/preview/blame,
and the remote-latency prediction layer.
