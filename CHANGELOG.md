# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-07-26

### Added

- Real-time TUI project monitor: languages (`cloc`), disk weight, git activity
- Interval / session deltas and multi-window code history
- Optional local `.heim/` sample store for cross-session history
- Size backends: `auto` (prefer `dust`), `dust`, `walk`
- Headless `--once` mode for scripts and CI
- Mouse, panel focus, weight drill-down, contribution heatmap
- **AI / agent JSON stats**: `heim --once --json` and `-o/--output`
- Auto-updated `<project>/.heim/stats.json` on every sample (schema `heim.stats.v1`)
- Window deltas in JSON for 5m–2h plus 4h / 8h / 1d (code + size)

### Notes

- Positioned for AI coding sessions: watch generation, then let agents self-audit
- External tools: `cloc` required; `git` and `dust` recommended
