# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **README growth metrics** (1d / 3d / 1w / 2w / 1mo / 2mo / 6mo / 1y): CI cron samples via `scripts/metrics_readme.py`, stores history in `docs/metrics/`, rewrites shields-style badges + table between `HEIM:METRICS` markers
- Workflow `.github/workflows/metrics.yml` (daily schedule + manual + path-filtered push)

## [0.1.0] — 2026-07-26

### Added

- Real-time TUI project monitor: languages, disk weight, git activity
- Interval / session deltas and multi-window code history
- Optional local `.heim/` sample store for cross-session history
- Size backends: `auto` / `walk` (in-process parallel walk), optional `dust`
- Headless `--once` mode for scripts and CI
- Mouse, panel focus, weight drill-down, contribution heatmap
- **AI / agent JSON stats**: `heim --once --json` and `-o/--output`
- Auto-updated `<project>/.heim/stats.json` on every sample (schema `heim.stats.v1`)
- Window deltas in JSON for 5m–2h plus 4h / 8h / 1d (code + size + git insertions/deletions)
- Soft volume **hints / ALERT lines** in JSON for 30m and 2h growth
- Agent docs: `docs/for-agents.md` and skill `skills/heim-audit`
- CI (fmt, clippy, test, sample) and multi-target GitHub Release workflow
- crates.io package name **`heim-monitor`** (binary **`heim`**)
- `HEIM_TRACE=1` prints per-collector timings (sample latency is the slowest
  collector, not their sum, so a regression in one hides until it becomes the max)
- Hero demo GIF + generator (`docs/assets/demo.gif`, `scripts/gen_demo_gif.py`)
- VHS tape optional live capture (`docs/demo.tape`)
- Launch / outreach docs (`docs/launch-post.md`, `docs/soft-feedback.md`, `docs/awesome-list-prs.md`)
- Skill packaging notes (`docs/skill-packaging.md`)
- One-line binary installer (`scripts/install.sh`)
- Homebrew formula template (`dist/homebrew/heim.rb`)
- GitHub FUNDING.yml

### Changed

- **LOC is in-process via tokei** — no required `cloc` install; samples are much faster
  (measured on a 4k-file / 80MB Rust tree: sample `took` 9.13s → 0.21s, ~43×)
- Default size engine is the parallel walk (`auto` → walk); `dust` remains opt-in
  (same tree: walk ~17ms vs the `dust` subprocess ~920ms)
- Size walk is parallel (`ignore` work-stealing walker) with per-thread
  accumulators merged once at the end — no shared lock on the per-file path
- TUI repaints only when state, input, or the quantized animation changes
  instead of unconditionally at 120Hz — idle CPU ~4× lower (4.0% → 1.0%
  measured over a 20s window). Animation pacing is unchanged.
- The four `git` invocations per sample now run concurrently
- Subprocess wait uses adaptive backoff (1ms → 20ms) instead of a flat 20ms poll
- Ignore-directory test uses a hash set instead of a linear scan of 35 strings,
  which ran once per directory entry and again per file
- Release profile keeps function symbols (`strip = "debuginfo"`) so profiles and
  backtraces stay readable; `panic = "unwind"` is explicit because `collect()`
  recovers from collector-thread panics via `join()`
- Positioning: *Stop vibe-code bloat: real-time LOC/size/git deltas + JSON agents can self-audit*

### Fixed

- **Crash** on terminals shorter than 15 rows: `effective_git_h` built an
  inverted `clamp(5, max_git)` range and panicked
- **LOC silently reported nothing on large repos**: `cloc` appends a plaintext
  "files took longer than expected" warning *after* its JSON, and the trailing
  bytes made `serde_json` reject the whole payload. Gone with the `cloc` removal
- `dust --version` and `git rev-parse` probes ran unbounded; both are now timed
  like every other subprocess and can no longer hang a sample

### Notes

- Local-only: no network; writes under `.heim/`
- External tools: `git` recommended; `dust` optional
