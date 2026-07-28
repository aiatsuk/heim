# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **MSRV 1.88 CI**: pin `sysinfo` to **0.38** (`0.39` requires rustc 1.95)

### Added

- **Host resource strip** in the monitor panel: **CPU %** / **Memory** / **Downlink·Uplink** (every 3s; rates from interface counters, not a speedtest), and multi-target **Ping** every 5m (mean of anycast DNS `1.1.1.1` + `8.8.8.8`, plus separate default-**GW** RTT). Force net+ping with **`n`**. In-memory **ping history**: 30m sparkline + **avg/max** over 5m / 10m / 30m. Also printed on `heim --once` (text mode)
- **Weight panel metric toggle** (`m`): switch between disk **size** and **code** (LOC) per path, with the same drill-down into directories (tokei, parallel per-child)
- **README growth metrics** (1d / 3d / 1w / 2w / 1mo / 2mo / 6mo / 1y): CI cron samples via `scripts/metrics_readme.py`, stores history in `docs/metrics/`, rewrites shields-style badges + table between `HEIM:METRICS` markers
- README **top-7 languages** (code + % of LOC) from the same sample as the TUI languages panel
- Workflow `.github/workflows/metrics.yml` (daily schedule + manual + path-filtered push)
- `tests/contract.rs` pins the `heim.stats.v1` JSON schema agents depend on — field
  names, the eight delta windows, and the rule that a non-ready window reports
  `code: null` rather than a number readable as "no growth"
- `clippy.toml` requires every subprocess to go through `collect::run_timed`
  (timeout + pipe draining); `[lints]` in `Cargo.toml` moves lint policy out of a
  CI-only flag so local clippy and rust-analyzer agree

### Fixed

- **Auto-refresh could die for the rest of the session.** The worker coalesced
  queued jobs by keeping only the newest one *of any kind*, so a drill-down
  request silently dropped a pending full sample; `refreshing` then stayed `true`
  and `due()` returns `false` while refreshing. Coalescing is now per kind.
- **Panics left the terminal unusable** — no hook, raw mode + alternate screen +
  mouse capture survived the process. Added a restore hook, gated to the UI
  thread so recovered collector-thread panics do not tear down a live session,
  plus unconditional teardown (the old `?` chain skipped every step after a
  failing one).
- **Window deltas reported `ready: true, code: 0` after a gap in history**, which
  reads as "nothing was written" when the truth is "no data" — the baseline found
  was the current sample itself. Now requires history to span the window *and* a
  baseline strictly older than the current sample.
- **Concurrent writers corrupted the store.** `serde_json::to_writer` on a raw
  `File` emitted ~66 syscalls per record and `O_APPEND` only makes individual
  writes atomic, so two heim processes interleaved mid-record; unparseable lines
  were then dropped silently by `load_recent`. A real store measured **15% of
  records corrupt**. Records are now written with a single `write_all`, and
  compaction is buffered.
- **Phantom session events**: `write_store_stats` opened a throwaway `Store` on
  every sample whose `Drop` appended an `end` for a session that never started —
  a live store had **177 `end` against 81 `start`**. `end_session` is now
  idempotent and a no-op for unbegun sessions; both `std::mem::forget` calls that
  masked this are gone.
- **`-o FILE` also dumped the report to stdout**, contradicting `--help` and
  `docs/for-agents.md`; agents redirecting stdout got the report twice. `-o -`
  remains stdout-only.
- Report building no longer shells out to git: 24h commit churn is collected with
  the sample instead of on the TUI's render thread, where a locked index could
  freeze input for up to 20s. Subprocesses per `--once --json`: **9 → 6**.
- A deleted working directory now produces `heim: resolve current directory: …`
  and exit 1 instead of a panic banner and exit 101.
- History retains only the fields it is read for; whole samples kept up to 100
  commits, churn, `top_dirs` and the language breakdown alive for all 10k entries.

## [0.1.0] — 2026-07-26

### Added

- Real-time TUI project monitor: languages, disk weight, git activity
- Interval / session deltas and multi-window code history
- Optional local `.heim/` sample store for cross-session history
- Size backends: `auto` / `walk` (in-process parallel walk), optional `dust`
- Headless `--once` mode for scripts and CI
- Mouse, panel focus, weight drill-down
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
