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

### Notes

- First public-oriented release
- External tools: `cloc` required; `git` and `dust` recommended
