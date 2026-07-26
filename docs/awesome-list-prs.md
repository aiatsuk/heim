# Awesome-list PR drafts (post soft feedback + v0.1.0)

Open these **after** install works from crates.io or GitHub Releases.  
One PR per list; don’t flood the same day as Show HN if list maintainers hate launch spam—wait 24–48h or reference the Show HN.

---

## 1. awesome-ratatui

**Repo:** search `awesome-ratatui` on GitHub  
**Section:** Monitoring / developer tools (or “Applications”)

```markdown
- [heim](https://github.com/aiatsuk/heim) - Real-time project monitor for AI coding sessions (LOC/size/git deltas + agent JSON)
```

---

## 2. awesome-tuis

```markdown
- [heim](https://github.com/aiatsuk/heim) - Stop vibe-code bloat: live LOC/size/git deltas and machine-readable stats for coding agents
```

---

## 3. awesome-rust (Applications → Utilities or Development tools)

```markdown
- [heim](https://github.com/aiatsuk/heim) — [heim-monitor](https://crates.io/crates/heim-monitor) - Real-time LOC/size/git monitor for AI coding sessions with agent-readable JSON
```

Follow that repo’s formatting rules (often requires crates.io link + CI badge).

---

## 4. awesome-claude-code / agent skill lists

```markdown
- [heim-audit](https://github.com/aiatsuk/heim/tree/main/skills/heim-audit) - Self-audit project growth with `heim --once --json` after large agent edits
```

---

## PR body template

```text
## What

Adds heim — a local TUI + JSON control surface for AI coding sessions
(wall-clock LOC/size/git deltas; agents self-audit via heim.stats.v1).

## Why it fits

- Actively maintained, MIT, documented install
- Distinct from one-shot tokei/cloc: live session feedback + agent schema
- Demo + agent docs: https://github.com/aiatsuk/heim

## Checklist

- [x] Not a duplicate entry
- [x] Link works
- [x] Description matches list style
```

---

## Do not

- Claim “best” / hype metrics you can’t prove  
- Open 10 lists with identical spammy text in one hour  
- PR before `cargo install heim-monitor` or release binaries work
