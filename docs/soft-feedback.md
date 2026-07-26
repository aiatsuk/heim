# Soft feedback — 5 agent power-users (before Show HN)

Goal: get honest signal before a public launch. Not stars—**usage + friction**.

## Who to ask (fill names)

| # | Person | Stack (Claude/Cursor/Codex/…) | Contact | Status |
|---|--------|-------------------------------|---------|--------|
| 1 |  |  |  | pending |
| 2 |  |  |  | pending |
| 3 |  |  |  | pending |
| 4 |  |  |  | pending |
| 5 |  |  |  | pending |

Pick people who **already** run agents daily and complain about bloat / review load.

## Message template (copy/paste)

Subject / opener: **quick 15‑min tool ask — live metrics next to your coding agent**

Hey {name} —

I built a tiny local tool called **heim**: a TUI + JSON self-audit for AI coding sessions (LOC / disk / git **deltas** over 5m–2h). Pitch: *stop vibe-code bloat with numbers agents can read*.

Would you try it for **one real session** this week?

```bash
cargo install heim-monitor --locked   # binary: heim
cd <some-project>
heim -i 10                            # human pane
# after big agent edits:
heim --once --json . | jq '{code: .loc.code, d2h: (.deltas[]|select(.window=="2h")), hints}'
```

Repo + demo: https://github.com/aiatsuk/heim  

3 questions only:

1. Did Δ / JSON change what you told the agent to do?  
2. What was confusing or slow in the first 10 minutes?  
3. What threshold would you want as a default ALERT?

No obligation to star/share. Brutal notes > polite ones.

Thanks — {you}

---

## After they reply — log here

### User 1

- Session context:
- Install path used:
- Feedback:
- Action items for heim:

### User 2

- …

## Go / no-go for Show HN

Post publicly when:

- [ ] ≥3 people completed a real session  
- [ ] No “couldn’t install / binary missing / needs cloc” blockers  
- [ ] At least one person says they would run it again  
- [ ] Launch post + GIF absolute URL verified (`docs/launch-post.md`)

If feedback says “I already use X” — write it down; adjust positioning, don’t expand scope into a platform.
