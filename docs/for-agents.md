# heim for agents

**Schema:** `heim.stats.v1`  
**Binary:** `heim` (crate on crates.io: `heim-monitor`)  
**Skill:** [`skills/heim-audit`](../skills/heim-audit/SKILL.md) · packaging notes in [`skill-packaging.md`](skill-packaging.md)

Use this when you need a **machine-readable self-audit** of project growth during an AI coding session.

> **Pitch for prompts:** Stop vibe-code bloat — real-time LOC/size/git deltas + JSON agents can self-audit.

## When to call

| Moment | Command |
|--------|---------|
| After a large scaffold / multi-file edit | `heim --once --json .` |
| Before declaring a task done | same |
| Continuous watch (human) | `heim -i 10` in another pane |
| Cheap re-read mid-session | `cat .heim/stats.json` (refreshed every sample) |

## Commands

```bash
# Full report to stdout (also writes .heim/stats.json)
heim --once --json .

# Write only to a path (stdout silent unless you also want it)
heim --once --json -o /tmp/heim-stats.json .

# Human one-shot (no TUI)
heim --once .
```

Exit code is non-zero only on hard failures (bad path, I/O). Missing git is not fatal — `git` is `null` and `git_error` explains.

## Report contract (`heim.stats.v1`)

| Field | Type | Meaning |
|-------|------|---------|
| `schema` | string | Always `"heim.stats.v1"` |
| `version` | string | heim package version |
| `purpose` | string | Human/agent blurb |
| `path` | string | Project root sampled |
| `collected_at` | string | RFC3339 UTC |
| `collected_at_unix` | u64 | Unix seconds |
| `took_secs` | f64 | Sample wall time |
| `loc` | object\|null | Totals + per-language |
| `loc_error` | string\|null | Why LOC failed |
| `size` | object | bytes, human, engine, top paths. Excludes `IGNORE_DIRS` and every dot-entry, so it is **not** comparable to `du -sh` |
| `git` | object\|null | branch, working tree +/-, recent commits |
| `git_error` | string\|null | Why git missing |
| `deltas[]` | array | Time windows (see below) |
| `session` | object | Deltas since this process baseline |
| `history` | object | Sample count / span / store path |
| `hints[]` | string[] | Actionable guidance (incl. ALERT lines) |

### `loc`

```json
{
  "code": 4133,
  "files": 13,
  "blank": 485,
  "comment": 526,
  "languages": [
    { "name": "Rust", "code": 4077, "blank": 369, "comment": 251, "pct": 98.65 }
  ]
}
```

LOC is counted **in-process** (tokei). Heavy dirs (e.g. `node_modules`, `target`) are excluded. `.gitignore` is honoured by the LOC engine.

### `deltas[]`

Windows: **5m, 10m, 30m, 1h, 2h, 4h, 8h, 1d**.

```json
{
  "window": "2h",
  "window_secs": 7200,
  "ready": true,
  "code": 420,
  "size_bytes": 8192,
  "insertions": 500,
  "deletions": 80
}
```

| Field | Meaning |
|-------|---------|
| `ready` | Enough **heim sample history** for `code` / `size_bytes` |
| `code` | Net LOC change vs oldest sample still inside the window |
| `size_bytes` | Net disk change over the window |
| `insertions` / `deletions` | Sum of **git commit** shortstats in the window (not working-tree dirty) |

If `ready` is false, keep sampling (run TUI or `--once` again later). Cross-session history lives in `.heim/samples.jsonl` (~48h retain).

### `hints[]`

Always includes usage reminders. Threshold-style lines may appear:

- `code +N over 30m` / `ALERT: … over 30m` when volume is high  
- `ALERT: code +N over 2h` for large sessions  
- heaviest-path tip when one path dominates size  

Treat any line starting with `ALERT:` as a stop-and-clean signal.

## Canonical agent prompt

Copy into agent rules / skills:

```text
After large multi-file edits or before finishing a coding task:

1. Run: heim --once --json .
2. Parse JSON (schema heim.stats.v1).
3. Inspect deltas for windows "30m" and "2h":
   - If code is large (or any hints[] starts with "ALERT:"), list size.top
     and git.recent_commits, then delete unused generated code, dead
     scaffolds, and duplicate helpers.
4. Re-run heim --once --json . and confirm deltas / loc.code improved
   (or justify why growth is intentional).
5. Prefer reading .heim/stats.json only when you just sampled; re-run
   the command after further edits.
```

## Example jq probes

```bash
# Total code + 2h delta
heim --once --json . | jq '{
  code: .loc.code,
  d2h: (.deltas[] | select(.window=="2h")),
  alerts: [.hints[] | select(startswith("ALERT:"))]
}'

# Heaviest paths
heim --once --json . | jq '.size.top[:5]'
```

## Privacy

- Local only — no network  
- Writes under `<project>/.heim/` (gitignored by default)  
- Do not commit `.heim/`

## Install (for agents / sandboxes)

```bash
# From crates.io (package name ≠ binary name)
cargo install heim-monitor --locked

# From git
cargo install --git https://github.com/aiatsuk/heim --locked

# Prebuilt (GitHub Releases), when tagged
# https://github.com/aiatsuk/heim/releases
```

Binary on PATH must be named **`heim`**.
