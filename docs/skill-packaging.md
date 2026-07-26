# Skill packaging

Canonical skill: [`skills/heim-audit/SKILL.md`](../skills/heim-audit/SKILL.md)  
JSON contract: [`for-agents.md`](for-agents.md)

## Layout (this repo)

```text
skills/
  heim-audit/
    SKILL.md          # Claude-style skill frontmatter + procedure
```

## Install into agent tools

### Claude Code / Claude-compatible skills dir

```bash
# user-level example
mkdir -p ~/.claude/skills
ln -s "$(pwd)/skills/heim-audit" ~/.claude/skills/heim-audit

# or copy
cp -R skills/heim-audit ~/.claude/skills/
```

### Cursor / generic agent rules

Add to project or user rules:

```text
After large multi-file edits, run `heim --once --json .` and follow
https://github.com/aiatsuk/heim/blob/main/docs/for-agents.md
Treat hints starting with ALERT: as stop-and-clean signals.
```

Or vendor the skill:

```bash
mkdir -p .cursor/skills
cp -R skills/heim-audit .cursor/skills/
```

### Codex / other CLIs

Paste the “Canonical agent prompt” section from `for-agents.md` into the agent’s instruction file (`AGENTS.md`, custom system prompt, etc.).

## Preconditions for agents

1. `heim` on `PATH` (`cargo install heim-monitor --locked`)  
2. Project cwd is the repo root (or pass the path)  
3. Prefer re-running the command after edits; don’t trust a stale `.heim/stats.json` blindly  

## Versioning

Skill documents schema **`heim.stats.v1`**. When the schema changes, bump the skill + `for-agents.md` in the same release.
