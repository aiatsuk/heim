# heim

**Real-time project monitor for the terminal** — languages (LOC), disk weight, and git activity in one compact TUI.

heim samples a project path on a timer, keeps the UI responsive with a background collector, and shows deltas so you can see growth as you work.

```text
┌ monitor ──────────────────────────────────────────────────────────────────┐
│ heim  ~/code/heim  every 60s · just now  ○ live                           │
│ code 3,697  files 13  blank 375  comments 178  size 155K                  │
│ ◆ Δ 5m ·  10m ·  30m ·  1h —  2h —  · top Rust 94% · via dust · 13 langs  │
├ languages ───────────────┬ weight ────────────────────────────────────────┤
│ # language   code     %  │ #  path              size     %                │
│ 1 Rust       3,697  100% │ 1 › src              126K    81%               │
│ …                        │ 2   Cargo.lock        21K    14%               │
├ git ──────────────────────────────────────────────────────────────────────┤
│ main  +0  -0                                                              │
│ 5dcb7de  +105/-351  Remove dead code and consolidate delta helpers        │
└───────────────────────────────────────────────────────────────────────────┘
  q uit  r efresh  +/- interval  tab:languages  ? help
```

> **Name note:** Unrelated to the older [heim](https://crates.io/crates/heim) system-information crate on crates.io.

---

## Why heim?

| Need | What heim does |
|------|----------------|
| Watch LOC grow while you code | Live `cloc` breakdown + session / window deltas |
| Find what’s eating disk | Ranked weight table with drill-down (`dust` or walk) |
| Glance at git state | Branch, dirty +/- , recent commits, activity strip |
| Script / CI samples | Headless `--once` text output |
| Survive monorepos | Smart ignores + async collectors (UI never blocks) |

---

## Features

- **Languages** — `cloc` totals and per-language ranking (code, blank, comments, %)
- **Weight** — top paths by size; open directories, cache drill-downs, go up
- **Git** — branch, working-tree inserts/deletes, recent log, contribution heatmap
- **Deltas** — code windows over **5m / 10m / 30m / 1h / 2h** (live + local store)
- **Responsive layout** — columns adapt to terminal width; panels resize with mouse
- **Private store** — optional per-project `.heim/` for cross-session history
- **Headless mode** — `--once` for scripts and automation
- **Single binary** — no daemon, no network, no accounts

---

## Requirements

| Dependency | Required? | Role |
|------------|-----------|------|
| [Rust](https://rustup.rs/) **1.74+** | to build / install from source | MSRV |
| [`cloc`](https://github.com/AlDanial/cloc) | **yes** (for LOC) | language stats |
| [`git`](https://git-scm.com/) | recommended | git panel (app still runs without it) |
| [`dust`](https://github.com/bootandy/dust) | optional | faster size backend (`du-dust` crate → binary `dust`) |

### Install dependencies

```bash
# macOS
brew install cloc dust

# Debian / Ubuntu
sudo apt install cloc
cargo install du-dust   # provides `dust`

# Fedora
sudo dnf install cloc
cargo install du-dust
```

---

## Install

### From Git (recommended once the repo is public)

```bash
cargo install --git https://github.com/aiatsuk/heim --locked
```

### From a local clone

```bash
git clone https://github.com/aiatsuk/heim.git
cd heim
cargo install --path . --locked
```

### Run without installing

```bash
cargo run --release --
cargo run --release -- --once .
```

---

## Quick start

```bash
# monitor current directory (refresh every 60s)
heim

# another project, faster refresh
heim ~/code/my-app -i 5

# one-shot sample (no TUI)
heim --once .
```

---

## CLI

```text
heim [OPTIONS] [PATH]

Arguments:
  [PATH]   Project directory (default: current working directory)

Options:
  -i, --interval <SECS>           Refresh interval in seconds [default: 60]
                                  (runtime range 1–300 via +/- keys)
      --size-backend <BACKEND>    auto | dust | walk [default: auto]
      --once                      Print one sample to stdout and exit
  -h, --help                      Print help
  -V, --version                   Print version
```

### Size backends

| Value | Behavior |
|-------|----------|
| `auto` | Use `dust` if available, else walk the tree |
| `dust` | Prefer `dust` (falls back to walk on failure) |
| `walk` | Pure Rust walk with the same ignore list |

### Headless example (`--once`)

```text
path:   /path/to/project
size:   155K  (via dust)
top:
   1. src                 126K    81%
   2. Cargo.lock           21K    14%
loc:    code=3,697 files=13 blank=375 comments=178
git:    main  +0  -0
  5dcb7de  +105/-351  Remove dead code …  (author)
took:   0.11s
```

Useful for scripts, CI summaries, or comparing trees without opening the TUI.

---

## Interface

### Layout

| Panel | Contents |
|-------|----------|
| **monitor** | Project name & path · interval · sample age · live/collecting · LOC totals · size · Δ code over 5m–2h · insight chips |
| **languages** | Ranked languages: code, blank/comments (when wide), %, session Δ |
| **weight** | Ranked paths: size, %, session Δ · `›` marks directories · drill-down |
| **git** | Branch · working tree `+`/`-` · recent commits · activity strip (auto-collapses when empty) |
| **footer** | Key hints for the current width |

### Keyboard

| Key | Action |
|-----|--------|
| `q` / `Ctrl-C` | Quit |
| `r` | Force refresh |
| `+` / `=` | Interval +1s (max 300) |
| `-` / `_` | Interval −1s (min 1) |
| `Tab` / `Shift-Tab` | Focus next / previous panel |
| `j` `k` / `↓` `↑` | Move selection |
| `PageDown` / `PageUp` | Scroll viewport |
| `Home` / `End` | Jump to first / last row |
| `Enter` / `l` / `→` | Open weight directory |
| `Backspace` / `h` / `←` | Weight parent directory |
| `?` | Toggle help |
| `Esc` | Close help |

### Mouse

| Input | Action |
|-------|--------|
| Click | Focus panel + select row |
| Wheel | Scroll focused list |
| Drag vertical `┊` rail | Resize languages / weight split |
| Drag top of git panel | Resize git height |
| Double-click / Enter | Open weight directory |
| Right-click | Weight parent directory |

---

## Smart ignores

Size walks and `cloc` skip common heavy / generated directories, including:

`.git`, `node_modules`, `target`, `dist`, `build`, `.venv`, `venv`, `__pycache__`, `.next`, `vendor`, `.tox`, `coverage`, `.cache`, `.idea`, `.vscode`, `Pods`, `DerivedData`, `.turbo`, `.gradle`, `.heim`, `.dart_tool`, `.build`, `.swiftpm`, `Carthage`, and similar monorepo dumps.

---

## Private store (`.heim/`)

When heim can write next to the project, it creates a **local-only** store:

```text
.heim/
  .gitignore      # ignore store contents
  README          # short privacy note
  sessions.jsonl  # session start / end
  samples.jsonl   # compact metric samples (≈48h retain, rotated)
```

Used to fill longer Δ windows across sessions. Never intended for version control (also listed in this repo’s `.gitignore`).

heim does **not** send data over the network.

---

## How it works

1. A **background worker** runs `cloc`, size measurement, and `git` sampling.
2. The **UI thread** paints at ~120 Hz for smooth spinners; it never waits on collectors.
3. Each successful sample updates session baseline, previous sample, history, and optional `.heim/` store.
4. Code deltas for 5m–2h use wall-clock history (in-memory + store), not just the current process.

---

## Platforms

| Platform | Status |
|----------|--------|
| **macOS** | Primary target |
| **Linux** | Expected to work |
| **Windows** | Untested (crossterm/ratatui may work; path/tooling differ) |

Requires a terminal with UTF-8 and reasonable Unicode glyph support (braille spinner, box drawing).

---

## Troubleshooting

| Symptom | What to try |
|---------|-------------|
| `cloc` errors / empty languages | Install `cloc` and ensure it is on `PATH` |
| Slow first sample on huge trees | Normal; UI stays interactive. Prefer `dust`. Check ignores. |
| Git panel empty / error | Not a git repo, or `git` missing — rest of the app still works |
| Size looks wrong | Try `--size-backend walk` or install `dust`; confirm ignores |
| Colors look flat | Use a truecolor / 256-color terminal theme |
| Store unavailable | Directory not writable; app continues without cross-session history |

---

## Development

```bash
# local checks
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
./target/release/heim --once .

# optional: dump TUI layouts for visual QA
HEIM_DUMP=1 cargo test -- --nocapture dump_layouts
```

### Project layout

```text
src/
  main.rs     CLI, event loop, worker wiring
  app.rs      state, deltas, focus, layout metrics
  collect.rs  cloc / size / git collectors
  store.rs    private .heim/ persistence
  ui.rs       ratatui rendering
  theme.rs    colors, glyphs, animation pacing
  fmt.rs      pure formatting helpers
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for PR expectations and [CHANGELOG.md](CHANGELOG.md) for release notes.

---

## Related tools

heim is a **dashboard**, not a replacement for any one of these:

| Tool | Focus |
|------|--------|
| [`cloc`](https://github.com/AlDanial/cloc) / [`tokei`](https://github.com/XAMPPRocky/tokei) | One-shot LOC counts |
| [`dust`](https://github.com/bootandy/dust) / `du` | Disk usage |
| `git` / `tig` / `lazygit` | Full VCS workflows |
| [`btm`](https://github.com/ClementTsang/bottom) / `htop` | System resources |

---

## Security

Local tool only: reads the project tree, shells out to `cloc` / `git` / optional `dust`, writes under `.heim/`. See [SECURITY.md](SECURITY.md) for reporting issues.

---

## License

[MIT](LICENSE) © 2026 aiatsuk
