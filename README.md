# heim

**Real-time project monitor built for AI coding sessions** — watch how much code a model generates, how heavy the tree gets, and what git is doing, then feed the same metrics back to the agent as JSON.

Run the TUI while an agent works. When you want the agent to *self-check* (“how many lines did you add in the last two hours?”), call:

```bash
heim --once --json .
```

…or let it read `<project>/.heim/stats.json`, which is refreshed on every sample.

```text
┌ monitor ──────────────────────────────────────────────────────────────────┐
│ heim  ~/code/my-app  every 60s · just now  ○ live                         │
│ code 12,480  files 86  blank 1,204  comments 890  size 4.2M               │
│ ◆ Δ 5m +120  10m +340  30m +910  1h +2.1k  2h +4.8k  · top Rust 71%       │
├ languages ───────────────┬ weight ────────────────────────────────────────┤
│ # language   code     %  │ #  path              size     %                │
│ 1 Rust       8,900   71% │ 1 › src              3.1M    74%               │
│ 2 Markdown   1,200   10% │ 2 › target           …ignored…                 │
├ git ──────────────────────────────────────────────────────────────────────┤
│ feature/agent  +42  -8                                                    │
│ a1b2c3d  +120/-3  feat: scaffold generated module                         │
└───────────────────────────────────────────────────────────────────────────┘
```

> **Name note:** Unrelated to the older [heim](https://crates.io/crates/heim) system-information crate on crates.io.

---

## Why heim? (AI workflow)

Large language models can dump a lot of code quickly. heim is a **control surface** for that loop:

| Goal | How heim helps |
|------|----------------|
| See generation in real time | TUI live-updates LOC, size, git while the agent works |
| Catch “too much code” early | Δ windows (5m → 1d) show how many lines landed recently |
| Let the agent inspect itself | `heim --once --json` / `.heim/stats.json` with full detail |
| Clean up with evidence | Agent can compare `deltas[2h].code`, language mix, top dirs, git log |
| Stay local | No network, no accounts — only your project tree |

Typical loop:

1. You start `heim` (or keep sampling with `--once`).
2. The agent edits the repo.
3. You ask: *“Check how many lines you added in 2h and trim noise.”*
4. The agent runs `heim --once --json .`, reads `deltas`, and fixes the tree.

---

## Features

- **Languages** — `cloc` totals and per-language ranking (code, blank, comments, %)
- **Weight** — top paths by size; directory drill-down; `dust` or walk
- **Git** — branch, dirty +/- , recent commits, activity strip
- **Deltas** — wall-clock windows **5m / 10m / 30m / 1h / 2h** in the TUI; JSON also includes **4h / 8h / 1d**
- **JSON for agents** — `heim --once --json` (+ optional `-o file`) and auto-updated `.heim/stats.json`
- **Private store** — `.heim/` history for cross-session windows
- **Headless text** — `heim --once` human-readable sample
- **Single binary** — no daemon, no network

---

## Requirements

| Dependency | Required? | Role |
|------------|-----------|------|
| [Rust](https://rustup.rs/) **1.74+** | to build / install from source | MSRV |
| [`cloc`](https://github.com/AlDanial/cloc) | **yes** (for LOC) | language stats |
| [`git`](https://git-scm.com/) | recommended | git panel / commits |
| [`dust`](https://github.com/bootandy/dust) | optional | faster size backend |

```bash
# macOS
brew install cloc dust

# Debian / Ubuntu
sudo apt install cloc
cargo install du-dust   # binary name: dust
```

---

## Install

```bash
cargo install --git https://github.com/aiatsuk/heim --locked

# or from a clone
git clone https://github.com/aiatsuk/heim.git
cd heim && cargo install --path . --locked
```

---

## Quick start

```bash
# 1) Watch an AI coding session live
heim                 # current directory, refresh every 60s
heim -i 10           # faster refresh while the agent works

# 2) Agent / script: full stats as JSON
heim --once --json .
heim --once --json -o /tmp/heim-stats.json .

# 3) Agent reads the auto snapshot (updated on every sample)
cat .heim/stats.json
```

---

## CLI

```text
heim [OPTIONS] [PATH]

Arguments:
  [PATH]   Project directory [default: cwd]

Options:
  -i, --interval <SECS>         TUI refresh interval [default: 60]
      --size-backend <BACKEND>  auto | dust | walk [default: auto]
      --once                    One sample, no TUI (human text unless --json)
      --json                    Machine-readable JSON report (implies one-shot)
  -o, --output <FILE>           Write JSON to FILE (implies --json); use - for stdout only
  -h, --help
  -V, --version
```

### JSON report (`heim.stats.v1`)

```bash
heim --once --json . | jq '{code: .loc.code, d2h: .deltas[] | select(.window=="2h")}'
```

Includes:

| Field | Meaning |
|-------|---------|
| `loc` | code / files / blank / comment + per-language breakdown |
| `size` | bytes, human size, engine, top directories |
| `git` | branch, dirty +/- , recent commits |
| `deltas[]` | per window: `ready`, `code`, `size_bytes` (5m…1d) |
| `session` | deltas since this process baseline |
| `history` | sample count, span, store path |
| `hints` | short guidance for agents / humans |

Every sample (TUI or `--once`) also refreshes:

```text
<project>/.heim/stats.json
```

so an agent can either **shell out** to `heim --once --json` or **read the file**.

### Example agent prompt

> Run `heim --once --json .` in the project root. Look at `deltas` for `"2h"`.
> If `code` is large, list the heaviest paths under `size.top` and recent
> `git.recent_commits`, then remove dead generated code and re-check.

---

## Interface (TUI)

### Layout

| Panel | Contents |
|-------|----------|
| **monitor** | Path · interval · age · live state · LOC totals · size · Δ code 5m–2h |
| **languages** | Ranked languages + session Δ |
| **weight** | Ranked paths + drill-down |
| **git** | Branch · dirty +/- · commits · activity (collapses when empty) |
| **footer** | Key hints |

### Keyboard

| Key | Action |
|-----|--------|
| `q` / `Ctrl-C` | Quit |
| `r` | Force refresh |
| `+` / `-` | Interval ±1s (1–300) |
| `Tab` / `Shift-Tab` | Focus next / prev panel |
| `j` `k` / arrows | Move selection |
| `Enter` / `l` | Open weight directory |
| `Backspace` / `h` | Weight parent |
| `?` | Help |

### Mouse

Click to focus/select, wheel to scroll, drag the vertical rail or git top edge to resize, double-click to open a weight dir, right-click to go up.

---

## Smart ignores

Skips common heavy dirs for size + `cloc`, including:  
`.git`, `node_modules`, `target`, `dist`, `build`, `.venv`, `__pycache__`, `.next`, `vendor`, `.heim`, `.dart_tool`, `.build`, and similar monorepo dumps.

---

## Private store (`.heim/`)

```text
.heim/
  .gitignore
  README
  sessions.jsonl   # session start/end
  samples.jsonl    # history for Δ windows (~48h retain)
  stats.json       # latest full report for agents
```

Local only — do not commit. heim does not send data over the network.

---

## How it works

1. Background worker runs `cloc`, size (`dust`/walk), and `git` in parallel.
2. UI thread stays smooth (~120 Hz animations); collectors never block paint.
3. Samples append to `.heim/samples.jsonl` and rewrite `.heim/stats.json`.
4. Time windows use wall-clock history (this session + store), so “last 2 hours” works across restarts.

---

## Platforms

| Platform | Status |
|----------|--------|
| **macOS** | Primary |
| **Linux** | Expected to work |
| **Windows** | Untested |

UTF-8 terminal recommended.

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Empty languages | Install `cloc` on `PATH` |
| Slow huge monorepos | Prefer `dust`; check ignores |
| Git panel empty | Not a repo / no `git` — rest still works |
| `deltas[].ready == false` | Not enough history yet — keep sampling |
| Agent sees stale numbers | Re-run `heim --once --json .` |

---

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
./target/release/heim --once --json . | head
```

```text
src/
  main.rs     CLI, event loop, worker
  app.rs      state, deltas, focus
  collect.rs  cloc / size / git
  report.rs   JSON report for agents
  store.rs    .heim persistence
  ui.rs       ratatui
  theme.rs    colors / glyphs
  fmt.rs      formatting helpers
```

See [CONTRIBUTING.md](CONTRIBUTING.md) and [CHANGELOG.md](CHANGELOG.md).

---

## Related tools

| Tool | Focus |
|------|--------|
| [`cloc`](https://github.com/AlDanial/cloc) / [`tokei`](https://github.com/XAMPPRocky/tokei) | One-shot LOC |
| [`dust`](https://github.com/bootandy/dust) | Disk usage |
| `git` / lazygit | Full VCS workflows |
| heim | **Live + agent-readable project growth control** |

---

## Security

Local tool: reads the project, shells out to `cloc` / `git` / optional `dust`, writes under `.heim/`. See [SECURITY.md](SECURITY.md).

---

## License

[MIT](LICENSE) © 2026 aiatsuk
