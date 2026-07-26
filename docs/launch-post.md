# Launch post draft (EN)

Use the same core story on Hacker News, Reddit, X, Lobsters.  
Tune length per channel; keep the pitch line identical.

---

## One-liner (title)

**Show HN: Heim – stop vibe-code bloat with live LOC/size/git deltas + agent JSON**

Alt titles:

- Show HN: Heim – real-time project monitor for AI coding sessions
- Heim: the missing feedback loop between you and your coding agent

---

## Short body (HN / Lobsters / r/rust)

AI coding agents changed the bottleneck from typing speed to judgment. A single session can dump thousands of lines in minutes—scaffolds, “just in case” helpers, generated junk. Classic tools (`tokei`, `cloc`, `du`) answer one-off questions. They don’t sit beside the agent and show how much landed in the last 5 minutes or 2 hours.

**heim** is a small local control surface for that loop:

1. **Live TUI** — languages, disk weight, git, and wall-clock Δ windows (5m → 2h).
2. **Agent JSON** — `heim --once --json` (schema `heim.stats.v1`) plus auto-updated `.heim/stats.json`.
3. **Self-audit hints** — soft `ALERT:` lines when 30m / 2h code growth is huge.

Demo (agent flood → alert → cleanup):

![heim demo](https://raw.githubusercontent.com/aiatsuk/heim/main/docs/assets/demo.gif)

### How agents use it

```bash
heim --once --json .
# or: cat .heim/stats.json
```

Canonical prompt / skill:  
https://github.com/aiatsuk/heim/blob/main/docs/for-agents.md  
https://github.com/aiatsuk/heim/tree/main/skills/heim-audit

### Install

```bash
# binary name is `heim`; crates.io package is heim-monitor
cargo install heim-monitor --locked

# or prebuilt from GitHub Releases
# https://github.com/aiatsuk/heim/releases
```

No `cloc` required — LOC is in-process (tokei). Local only; no network.

### Roadmap (next 3)

1. Sharper agent hints (thresholds / config) and better monorepo path filters  
2. Homebrew formula with release checksums  
3. Ecosystem packaging — Cursor/Claude skills in more agent registries  

MIT · Rust · macOS + Linux (Windows experimental)  
https://github.com/aiatsuk/heim

---

## X / Twitter thread

1/ Stop vibe-code bloat.

AI agents write thousands of lines in minutes. Without feedback, repos silently bloat.

heim = live LOC/size/git Δ + JSON agents can self-audit.

2/ [GIF]

Human watches the TUI. Agent runs:

`heim --once --json .`

If `deltas[2h].code` is huge (or `hints` says `ALERT:`), trim generated junk and re-check.

3/ Install:

`cargo install heim-monitor --locked`

Binary is `heim`. Local only. No cloc.

Repo: https://github.com/aiatsuk/heim

---

## Reddit variants

**r/ClaudeAI / r/cursor / r/LocalLLaMA** — lead with agent loop + GIF, soft on Rust.  
**r/rust / r/commandline** — lead with TUI + tokei in-process + ratatui, then AI angle.  
**r/programming** — problem/solution framing, link Show HN if live.

Avoid identical copy-paste across all subs in the same hour (spam filters).

---

## First comment (HN) — post immediately after submission

Happy to answer questions. Design notes:

- **Wall-clock windows** (not just “this process”) so “last 2 hours” works across restarts via local `.heim/` history.
- **JSON schema `heim.stats.v1`** is the product surface for agents; the TUI is the human dashboard.
- **No network** — samples stay on disk under `<project>/.heim/` (gitignored by default).
- Package on crates.io is `heim-monitor` because the older `heim` system-info crate owns that name; binary stays `heim`.

If you try it next to Claude Code / Cursor / Codex for a day, I’d love a short note on what Δ thresholds feel useful.

---

## Checklist before posting

- [ ] `main` has demo GIF + README pitch  
- [ ] CI green  
- [ ] `v0.1.0` release assets online **or** `cargo install heim-monitor` works  
- [ ] Absolute GIF URL works in a private browser window  
- [ ] Soft feedback from ≥3 agent-power-users already collected (see `docs/soft-feedback.md`)
