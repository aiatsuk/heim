---
name: heim-audit
description: >
  Self-audit project growth with heim (LOC/size/git deltas) during AI coding
  sessions. Use after large scaffolds, multi-file edits, or before declaring
  work done — especially when vibe-code bloat is a risk.
---

# heim-audit

Stop vibe-code bloat with measurable stats.

## Preconditions

- `heim` is on `PATH` (`cargo install heim-monitor --locked` or see project README).
- Working directory is the project root (or pass the path explicitly).

## Procedure

1. **Sample**

   ```bash
   heim --once --json .
   ```

   Optionally re-read the auto snapshot:

   ```bash
   cat .heim/stats.json
   ```

2. **Parse** the JSON (`schema` must be `heim.stats.v1`).

3. **Decide** using `deltas` for `"30m"` and `"2h"`:

   | Signal | Action |
   |--------|--------|
   | Any `hints[]` starts with `ALERT:` | Stop feature work; delete dead/generated code first |
   | `deltas[2h].code` large (e.g. > 500–2000) | Review `size.top` + `git.recent_commits`; trim |
   | `deltas[30m].code` large (e.g. > 300–1000) | Same, tighter window |
   | `ready: false` | Sample again later if you need that window |
   | Growth is intentional (new product surface) | Document why; still drop unused scaffolds |

4. **Clean**

   - Prefer deleting unused generated modules, duplicate helpers, “just in case” files.
   - Do not delete user data or unrelated project code.
   - After cleanup, re-run `heim --once --json .` and confirm `loc.code` / deltas moved the right way.

5. **Report to the human** (short)

   - Before/after `loc.code`
   - 30m / 2h `code` deltas
   - What was removed and why

## Do not

- Treat heim as a linter or test runner — it only measures growth.
- Commit `.heim/` history.
- Skip re-sample after a large cleanup.

## Reference

Full field contract: [docs/for-agents.md](../../docs/for-agents.md)
