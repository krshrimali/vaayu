Vaayu is a modal terminal editor in Rust with Vim-style editing, language servers, project tools, private notes, tasks, tours, and Markdown previews.
Install with "cargo install --path . --locked"; run "vaayu path/to/file" or "vy path/to/file".

[User guide](GUIDE.md) covers installation, editing, navigation, tasks, tours, terminals, agents, and limitations.
[Keymap help](HELP.md) is also available through ":help" in the editor.
[Configuration reference](CONFIGURATION.md) lists every setting; [example configuration](config.example.toml) shows setup.
[Command reference](COMMANDS.md) lists Ex commands and registered leader bindings.
[Architecture](ARCHITECTURE.md) describes the runtime, modules, storage, integrations, and diagram rendering.
[Technical decisions](TECHNICAL_DECISIONS.md) explains algorithms and evidence; [HTML edition](TECHNICAL_DECISIONS.html) provides the same guide.
[Development and validation](DEVELOPMENT.md) documents builds, test results, and reproduction.
[Performance measurements](BENCHMARKS.md) links raw rerun results, methods, provenance, and historical comparisons.
[Change coverage](CHANGELOG.md) accounts for every commit since the previous broad documentation refresh.
[Audit report](AUDIT.md) records verified behavior, historical findings, and remaining limits.
[Feature roadmap](ROADMAP.md) records delivery and future work.
[Neovim parity plan](NEOVIM_PARITY_PLAN.md) tracks implemented capabilities and remaining parity gaps.
[Completed scheduled backlog](FEATURE_BACKLOG.md) preserves the earlier feature run and its validation history.
[Mermaid implementation report](MERMAID_PREVIEW_PLAN.md) documents graphics, fallback, limits, and verification.
[Mermaid gallery](examples/mermaid.md) provides six diagram families to try with ",mp" or ",ms".
