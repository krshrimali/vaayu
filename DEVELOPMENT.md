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
cargo test --locked real_clangd_formatting_and_diagnostics -- --ignored
```

The real-clangd test needs a functioning `clangd`, not merely an executable path. This machine's installed clangd could not load `libLLVM.so.22.1`; this refresh used the official standalone clangd `23.1.0` under `/tmp`, without modifying system packages. Neovim `0.12.5`, Helix `25.07.1`, their official runtimes, and Python dependencies were also installed only under `/tmp` for the benchmarks. Exact versions/hashes are in [benchmark provenance](bench/2026-10-08/metadata.json).

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

## Benchmark reproduction

[BENCHMARKS.md](BENCHMARKS.md) contains workload, hardware, raw results and exact quoted commands. The refresh uses three sequential core-editor runs, three same-machine large-file baseline runs, three fixed-source legacy editing runs with alternating current/baseline order, five warmed clangd attempts per editor, and five fresh Mermaid processes per mode with cached reopening. [Metadata](bench/2026-10-08/metadata.json) records binary sizes/hashes and argv; [validation](bench/2026-10-08/validation.json) records check results. SSH remains unmeasured without an explicitly supplied authenticated host.
