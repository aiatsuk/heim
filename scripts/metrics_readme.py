#!/usr/bin/env python3
"""Refresh long-window project growth stats for the README.

Pipeline (designed for GitHub Actions cron, also works locally):

1. Run `heim --once --json` for a current LOC / size snapshot.
2. Append a compact sample to ``docs/metrics/samples.jsonl`` (committed history).
3. Compute wall-clock deltas for long windows (1d … 1y) against that history.
4. Optionally enrich with ``git log --shortstat`` churn for the same windows.
5. Write ``docs/metrics/latest.json`` and rewrite the README block between
   ``<!-- HEIM:METRICS:START -->`` … ``<!-- HEIM:METRICS:END -->``.

Usage:
  python3 scripts/metrics_readme.py
  HEIM_BIN=./target/release/heim python3 scripts/metrics_readme.py --no-git
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
METRICS_DIR = ROOT / "docs" / "metrics"
SAMPLES_PATH = METRICS_DIR / "samples.jsonl"
LATEST_PATH = METRICS_DIR / "latest.json"
README_PATH = ROOT / "README.md"

START_MARK = "<!-- HEIM:METRICS:START -->"
END_MARK = "<!-- HEIM:METRICS:END -->"

# Long-horizon windows for the public README (seconds, short label, human label).
WINDOWS: list[tuple[int, str, str]] = [
    (1 * 86_400, "1d", "today"),
    (3 * 86_400, "3d", "3 days"),
    (7 * 86_400, "1w", "1 week"),
    (14 * 86_400, "2w", "2 weeks"),
    (30 * 86_400, "1mo", "1 month"),
    (60 * 86_400, "2mo", "2 months"),
    (180 * 86_400, "6mo", "6 months"),
    (365 * 86_400, "1y", "1 year"),
]

# Keep ~14 months of daily-ish samples so 1y windows stay ready.
MAX_AGE_SECS = 420 * 86_400
MAX_SAMPLES = 5_000
# How many languages from the TUI / JSON `loc.languages` panel to show in README.
TOP_LANGUAGES = 7


@dataclass
class Sample:
    ts: int
    code: int | None
    files: int | None
    size: int
    branch: str | None = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Sample":
        return cls(
            ts=int(d["ts"]),
            code=d.get("code"),
            files=d.get("files"),
            size=int(d.get("size") or 0),
            branch=d.get("branch"),
        )

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "ts": self.ts,
            "code": self.code,
            "files": self.files,
            "size": self.size,
        }
        if self.branch:
            out["branch"] = self.branch
        return out


@dataclass
class LangStat:
    name: str
    code: int
    blank: int
    comment: int
    pct: float

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "code": self.code,
            "blank": self.blank,
            "comment": self.comment,
            "pct": self.pct,
        }


def now_unix() -> int:
    return int(time.time())


def rfc3339(ts: int) -> str:
    return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def signed(n: int | None) -> str:
    if n is None:
        return "—"
    if n > 0:
        return f"+{n:,}"
    return f"{n:,}"


def signed_bytes(n: int | None) -> str:
    if n is None:
        return "—"
    sign = "+" if n > 0 else ""
    v = abs(n)
    if v < 1024:
        return f"{sign}{n}B"
    for unit, div in (("K", 1024), ("M", 1024**2), ("G", 1024**3)):
        if v < div * 1024 or unit == "G":
            return f"{sign}{n / div:.1f}{unit}"
    return f"{sign}{n}"


def human_bytes(n: int) -> str:
    if n < 1024:
        return f"{n}B"
    for unit, div in (("K", 1024), ("M", 1024**2), ("G", 1024**3)):
        if n < div * 1024 or unit == "G":
            val = n / div
            return f"{val:.1f}{unit}" if val < 10 or unit != "K" else f"{int(val)}{unit}"
    return f"{n}B"


def badge_color(code_delta: int | None) -> str:
    if code_delta is None:
        return "lightgrey"
    if code_delta > 2000:
        return "red"
    if code_delta > 500:
        return "orange"
    if code_delta > 0:
        return "yellowgreen"
    if code_delta < 0:
        return "blue"
    return "lightgrey"


def shields_badge(label: str, message: str, color: str) -> str:
    # shields static badge — no external state, fully rendered in README markdown
    left = quote(label.replace("-", "--"), safe="")
    right = quote(message.replace("-", "--"), safe="")
    return f"https://img.shields.io/badge/{left}-{right}-{color}"


def run(cmd: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd or ROOT,
        text=True,
        capture_output=True,
        check=False,
    )


def find_heim_bin(explicit: str | None) -> str:
    if explicit:
        return explicit
    env = os.environ.get("HEIM_BIN")
    if env:
        return env
    for cand in (
        ROOT / "target" / "release" / "heim",
        ROOT / "target" / "debug" / "heim",
    ):
        if cand.is_file() and os.access(cand, os.X_OK):
            return str(cand)
    # fall back to PATH
    return "heim"


def origin_git_timestamp() -> int | None:
    """Unix time of the root commit(s) (repo birth), if available.

    Note: ``git log --reverse -n 1`` is *wrong* — ``-n`` applies before reverse,
    so you get the tip. Use ``rev-list --max-parents=0`` instead.
    """
    roots = run(["git", "rev-list", "--max-parents=0", "HEAD"])
    if roots.returncode != 0 or not roots.stdout.strip():
        return None
    root = roots.stdout.strip().splitlines()[0].strip()
    proc = run(["git", "log", "-1", "--format=%ct", root])
    if proc.returncode != 0:
        return None
    try:
        return int(proc.stdout.strip())
    except ValueError:
        return None


def ensure_origin_baseline(samples: list[Sample], cur: Sample) -> list[Sample]:
    """Seed a zero-LOC sample at the first commit so long windows become ready immediately.

    Without this, 1w/1mo/1y LOC badges stay on \"building\" until the cron has
    run for that long. Git churn still works; this makes LOC Δ meaningful from day one
    as \"growth since repo birth\" for any window that reaches origin.
    """
    origin = origin_git_timestamp()
    if origin is None or origin >= cur.ts:
        return samples
    if any(s.ts <= origin + 1 for s in samples):
        return samples
    seed = Sample(ts=origin, code=0, files=0, size=0, branch=None)
    return sorted(samples + [seed], key=lambda s: s.ts)


def top_languages_from_loc(loc: dict[str, Any], limit: int = TOP_LANGUAGES) -> list[LangStat]:
    """Take the ranked language list from heim JSON (already sorted by code desc)."""
    raw = loc.get("languages") or []
    out: list[LangStat] = []
    for item in raw:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name") or "").strip()
        if not name:
            continue
        try:
            code = int(item.get("code") or 0)
            blank = int(item.get("blank") or 0)
            comment = int(item.get("comment") or 0)
            pct = float(item.get("pct") or 0.0)
        except (TypeError, ValueError):
            continue
        out.append(LangStat(name=name, code=code, blank=blank, comment=comment, pct=pct))
    # heim already ranks by code, but sort defensively.
    out.sort(key=lambda l: l.code, reverse=True)
    return out[:limit]


def sample_with_heim(heim_bin: str) -> tuple[Sample, list[LangStat]]:
    proc = run([heim_bin, "--once", "--json", str(ROOT)])
    if proc.returncode != 0:
        raise RuntimeError(
            f"heim failed ({proc.returncode}): {proc.stderr.strip() or proc.stdout[:400]}"
        )
    data = json.loads(proc.stdout)
    loc = data.get("loc") or {}
    size = data.get("size") or {}
    git = data.get("git") or {}
    ts = int(data.get("collected_at_unix") or now_unix())
    sample = Sample(
        ts=ts,
        code=loc.get("code"),
        files=loc.get("files"),
        size=int(size.get("bytes") or 0),
        branch=git.get("branch"),
    )
    return sample, top_languages_from_loc(loc)


def load_samples(path: Path) -> list[Sample]:
    if not path.is_file():
        return []
    out: list[Sample] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            out.append(Sample.from_dict(json.loads(line)))
        except (json.JSONDecodeError, KeyError, TypeError, ValueError):
            continue
    out.sort(key=lambda s: s.ts)
    return out


def prune_samples(samples: list[Sample], now: int) -> list[Sample]:
    cutoff = now - MAX_AGE_SECS
    kept = [s for s in samples if s.ts >= cutoff]
    if len(kept) > MAX_SAMPLES:
        kept = kept[-MAX_SAMPLES:]
    return kept


def write_samples(path: Path, samples: list[Sample]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    body = "\n".join(json.dumps(s.to_dict(), separators=(",", ":")) for s in samples)
    if body:
        body += "\n"
    path.write_text(body, encoding="utf-8")


def baseline_for_window(
    samples: list[Sample], now: int, window_secs: int, cur: Sample
) -> Sample | None:
    """Pick the sample that anchors the start of a rolling window.

    Prefer the **newest sample at or before** ``now - window`` (true rolling Δ).
    If the repo is younger than the window, fall back to the **oldest** sample
    so long labels (1y, 6mo) still show growth-since-birth instead of \"building\".
    """
    others = [s for s in samples if s.ts != cur.ts or s is not cur]
    # Prefer identity by timestamp so re-built Sample objects still match.
    pool = [s for s in samples if not (s.ts == cur.ts and s.code == cur.code and s.size == cur.size)]
    if not pool:
        pool = [s for s in samples if s.ts < cur.ts]
    if not pool:
        return None

    cutoff = now - window_secs
    at_or_before = [s for s in pool if s.ts <= cutoff]
    if at_or_before:
        return max(at_or_before, key=lambda s: s.ts)

    # Window longer than available history → full-history baseline.
    return min(pool, key=lambda s: s.ts)


def code_delta(cur: Sample, old: Sample | None) -> int | None:
    if old is None or cur.code is None or old.code is None:
        return None
    return int(cur.code) - int(old.code)


def size_delta(cur: Sample, old: Sample | None) -> int | None:
    if old is None:
        return None
    return int(cur.size) - int(old.size)


def git_churn_since(window_secs: int) -> tuple[int, int] | None:
    """Sum insertions/deletions from git shortstat in the last window_secs."""
    since = f"{window_secs} seconds ago"
    proc = run(
        [
            "git",
            "log",
            f"--since={since}",
            "--pretty=tformat:",
            "--shortstat",
        ]
    )
    if proc.returncode != 0:
        return None
    ins = del_ = 0
    for line in proc.stdout.splitlines():
        # e.g. " 3 files changed, 120 insertions(+), 4 deletions(-)"
        m_ins = re.search(r"(\d+) insertion", line)
        m_del = re.search(r"(\d+) deletion", line)
        if m_ins:
            ins += int(m_ins.group(1))
        if m_del:
            del_ += int(m_del.group(1))
    return ins, del_


def compute_windows(
    samples: list[Sample], cur: Sample, now: int, with_git: bool
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for secs, short, human in WINDOWS:
        old = baseline_for_window(samples, now, secs, cur)
        ready = old is not None and old.ts < cur.ts
        c_delta = code_delta(cur, old) if ready else None
        s_delta = size_delta(cur, old) if ready else None
        # True rolling window only if baseline sits at/before cutoff.
        rolling = bool(old and old.ts <= now - secs)
        git_ins = git_del = None
        if with_git:
            churn = git_churn_since(secs)
            if churn is not None:
                git_ins, git_del = churn
        rows.append(
            {
                "window": short,
                "label": human,
                "window_secs": secs,
                "ready": ready,
                "rolling": rolling,
                "code": c_delta,
                "size_bytes": s_delta,
                "git_insertions": git_ins,
                "git_deletions": git_del,
                "baseline_ts": old.ts if old and ready else None,
            }
        )
    return rows


def badge_message(row: dict[str, Any]) -> tuple[str, str]:
    """Return (message, color) for a window badge.

    Prefer LOC sample history; fall back to git shortstat so badges are useful
    on day one before cron has accumulated baselines.
    """
    if row["ready"] and row["code"] is not None:
        return signed(row["code"]), badge_color(row["code"])
    ins, dele = row.get("git_insertions"), row.get("git_deletions")
    if ins is not None and dele is not None:
        net = int(ins) - int(dele)
        # Mark as git-sourced so readers don't confuse with LOC.
        return f"git {signed(net)}", badge_color(net)
    return "building", "lightgrey"


def render_badges(rows: list[dict[str, Any]], code: int | None) -> str:
    parts = []
    if code is not None:
        parts.append(
            f"![code {code:,}]({shields_badge('code', f'{code:,} LOC', 'informational')})"
        )
    for r in rows:
        msg, color = badge_message(r)
        label = f"Δ {r['window']}"
        parts.append(f"![{label}]({shields_badge(label, msg, color)})")
    return " ".join(parts)


def render_table(rows: list[dict[str, Any]]) -> str:
    lines = [
        "| Window | LOC Δ | Size Δ | Git +/− | Ready |",
        "|--------|------:|-------:|--------:|:-----:|",
    ]
    for r in rows:
        git = "—"
        if r["git_insertions"] is not None:
            git = f"+{r['git_insertions']:,}/−{r['git_deletions']:,}"
        if not r["ready"]:
            ready = "…"
        elif r.get("rolling"):
            ready = "yes"
        else:
            # Baseline is repo origin because the project is younger than the window.
            ready = "since birth"
        lines.append(
            f"| **{r['window']}** ({r['label']}) | `{signed(r['code'])}` | "
            f"`{signed_bytes(r['size_bytes'])}` | `{git}` | {ready} |"
        )
    return "\n".join(lines)


def lang_badge_color(rank: int) -> str:
    # Distinct-ish colors for the top slots (shields palette names).
    palette = (
        "blueviolet",
        "blue",
        "informational",
        "success",
        "yellowgreen",
        "orange",
        "lightgrey",
    )
    return palette[min(rank, len(palette) - 1)]


def render_language_badges(langs: list[LangStat]) -> str:
    parts = []
    for i, lang in enumerate(langs):
        label = lang.name
        msg = f"{lang.code:,}"
        parts.append(
            f"![{label} {msg}]({shields_badge(label, msg, lang_badge_color(i))})"
        )
    return " ".join(parts)


def render_languages_table(langs: list[LangStat]) -> str:
    if not langs:
        return "_No language stats yet._"
    lines = [
        f"| # | Language | Code | % of LOC |",
        f"|--:|----------|------:|--------:|",
    ]
    for i, lang in enumerate(langs, start=1):
        pct = f"{lang.pct:.1f}%"
        lines.append(
            f"| {i} | **{lang.name}** | `{lang.code:,}` | {pct} |"
        )
    return "\n".join(lines)


def render_block(
    cur: Sample,
    rows: list[dict[str, Any]],
    updated_ts: int,
    samples_n: int,
    languages: list[LangStat],
) -> str:
    badges = render_badges(rows, cur.code)
    table = render_table(rows)
    lang_badges = render_language_badges(languages)
    lang_table = render_languages_table(languages)
    code = f"{cur.code:,}" if cur.code is not None else "—"
    size = human_bytes(cur.size)
    files = f"{cur.files:,}" if cur.files is not None else "—"
    top_n = len(languages)
    return f"""{START_MARK}
<!-- Generated by scripts/metrics_readme.py — do not edit by hand. -->

### Project growth (heim metrics)

{badges}

| Now | |
|-----|--|
| **code** | {code} LOC |
| **files** | {files} |
| **size** | {size} |
| **updated** | {rfc3339(updated_ts)} UTC |
| **history** | {samples_n} samples in [`docs/metrics/`](docs/metrics/) |

#### Top {top_n} languages

{lang_badges}

{lang_table}

#### Time-window deltas

{table}

LOC/size Δ use committed samples in [`docs/metrics/samples.jsonl`](docs/metrics/samples.jsonl). If the repo is younger than a window, Ready shows **since birth** (full history). Git +/− is always from live `git log --shortstat`. Languages are the top {TOP_LANGUAGES} from the same sample as the TUI **languages** panel.

<sub>Auto-refreshed by [metrics.yml](.github/workflows/metrics.yml) (daily cron + manual). Re-run: <code>python3 scripts/metrics_readme.py</code></sub>

{END_MARK}"""


def upsert_readme(readme: Path, block: str) -> bool:
    text = readme.read_text(encoding="utf-8")
    if START_MARK in text and END_MARK in text:
        pattern = re.compile(
            re.escape(START_MARK) + r".*?" + re.escape(END_MARK),
            re.DOTALL,
        )
        new_text, n = pattern.subn(block.strip(), text, count=1)
        if n != 1:
            raise RuntimeError("failed to replace HEIM:METRICS block in README")
    else:
        # Insert after the hero / badges area — after the first thematic break following title.
        needle = "\n---\n\n## Motivation"
        if needle not in text:
            # fallback: append before license
            new_text = text.rstrip() + "\n\n" + block.strip() + "\n"
        else:
            insert = (
                "\n---\n\n"
                + block.strip()
                + "\n\n---\n\n## Motivation"
            )
            new_text = text.replace(needle, insert, 1)
    if new_text == text:
        return False
    readme.write_text(new_text, encoding="utf-8")
    return True


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--heim-bin", default=None, help="path to heim binary")
    ap.add_argument("--no-git", action="store_true", help="skip git shortstat churn")
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="print block to stdout; do not write files",
    )
    args = ap.parse_args()

    heim_bin = find_heim_bin(args.heim_bin)
    cur, languages = sample_with_heim(heim_bin)
    now = max(cur.ts, now_unix())

    samples = load_samples(SAMPLES_PATH)
    # Dedupe exact timestamps (re-runs), then append current sample.
    samples = [s for s in samples if s.ts != cur.ts]
    samples.append(cur)
    samples = ensure_origin_baseline(samples, cur)
    samples = prune_samples(samples, now)
    samples.sort(key=lambda s: s.ts)

    rows = compute_windows(samples, cur, now, with_git=not args.no_git)
    block = render_block(cur, rows, now, len(samples), languages)

    latest = {
        "schema": "heim.metrics.v1",
        "generated_at": rfc3339(now),
        "generated_at_unix": now,
        "current": cur.to_dict(),
        "languages": [l.to_dict() for l in languages],
        "languages_top_n": TOP_LANGUAGES,
        "windows": rows,
        "samples": len(samples),
        "samples_path": "docs/metrics/samples.jsonl",
    }

    if args.dry_run:
        print(block)
        print(json.dumps(latest, indent=2), file=sys.stderr)
        return 0

    METRICS_DIR.mkdir(parents=True, exist_ok=True)
    write_samples(SAMPLES_PATH, samples)
    LATEST_PATH.write_text(json.dumps(latest, indent=2) + "\n", encoding="utf-8")
    changed = upsert_readme(README_PATH, block)
    lang_names = ",".join(l.name for l in languages) or "-"
    print(
        f"metrics: samples={len(samples)} code={cur.code} langs=[{lang_names}] "
        f"readme_changed={changed} ready="
        + ",".join(r["window"] for r in rows if r["ready"])
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as e:
        print(f"metrics_readme: {e}", file=sys.stderr)
        raise SystemExit(1)
