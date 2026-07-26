# Long-window growth metrics

This directory is the **committed sample store** that powers the README “Project growth” block (and shields-style badges).

| File | Role |
|------|------|
| `samples.jsonl` | Append-only compact samples (`ts`, `code`, `files`, `size`, …) |
| `latest.json` | Last run: current totals + per-window deltas (`heim.metrics.v1`) |

## Windows

| Label | Span |
|-------|------|
| `1d` | ~today (rolling 24h) |
| `3d` | 3 days |
| `1w` | 1 week |
| `2w` | 2 weeks |
| `1mo` | ~30 days |
| `2mo` | ~60 days |
| `6mo` | ~180 days |
| `1y` | ~365 days |

**LOC / size Δ** compare the current sample to a baseline:
- **rolling** — newest sample at or before `now − window` (true Δ over that span);
- **since birth** — if the project is younger than the window, baseline is the origin seed (code `0` at the first commit).

Daily cron keeps the sample history honest as real time passes.  
**Git +/−** comes from `git log --shortstat` and is available immediately.

## How it updates

1. Workflow [`.github/workflows/metrics.yml`](../../.github/workflows/metrics.yml) runs on a **daily cron**, on **manual dispatch**, and when product code on `main` changes.
2. It builds `heim`, runs `scripts/metrics_readme.py`.
3. The script samples the tree, appends to `samples.jsonl`, writes `latest.json`, and rewrites the README section between:

   ```html
   <!-- HEIM:METRICS:START -->
   …
   <!-- HEIM:METRICS:END -->
   ```

4. If anything changed, the bot commits with `[skip ci]` (no CI loop).

## Local re-run

```bash
cargo build --release
HEIM_BIN=./target/release/heim python3 scripts/metrics_readme.py
# dry-run (stdout only):
python3 scripts/metrics_readme.py --dry-run
```

## Privacy

Only aggregate counts (LOC, file count, bytes) are stored — no file paths, no source text, no secrets.
