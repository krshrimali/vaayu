Vaayu is a modal terminal editor in Rust with Vim-style editing, language servers, project tools, private notes, tasks, tours, and Markdown previews (fyi --- markdown previews do support mermaid diagrams as well).

Why Vaayu? I wanted an editor that is feature complete w.r.t my neovim config (all third-party plugins, AI, GitHub support and more) + which is fast (just like helix).

Install with "cargo install --path . --locked"; run "vaayu path/to/file" or "vy path/to/file".

1. [User guide](GUIDE.md) covers installation, editing, navigation, tasks, tours, terminals, agents, and limitations.
2. [Keymap help](HELP.md) is also available through ":help" in the editor.
3. [Configuration reference](CONFIGURATION.md) lists every setting; [example configuration](config.example.toml) shows setup.
4. [Command reference](COMMANDS.md) lists Ex commands and registered leader bindings.
5. [Architecture](ARCHITECTURE.md) describes the runtime, modules, storage, integrations, and diagram rendering.
6. [Technical decisions](TECHNICAL_DECISIONS.md) explains algorithms and evidence; [HTML edition](TECHNICAL_DECISIONS.html) provides the same guide.
7. [Development and validation](DEVELOPMENT.md) documents builds, test results, and reproduction.
8. [Performance measurements](BENCHMARKS.md) links raw rerun results, methods, provenance, and historical comparisons.
9. [Change coverage](CHANGELOG.md) accounts for every commit since the previous broad documentation refresh.
10. [Audit report](AUDIT.md) records verified behavior, historical findings, and remaining limits.
11. [Feature roadmap](ROADMAP.md) records delivery and future work.
12. [Neovim parity plan](NEOVIM_PARITY_PLAN.md) tracks implemented capabilities and remaining parity gaps.
13. [Completed scheduled backlog](FEATURE_BACKLOG.md) preserves the earlier feature run and its validation history.
14. [Mermaid implementation report](MERMAID_PREVIEW_PLAN.md) documents graphics, fallback, limits, and verification.
15. [Mermaid gallery](examples/mermaid.md) provides six diagram families to try with ",mp" or ",ms".
