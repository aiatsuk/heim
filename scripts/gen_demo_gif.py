#!/usr/bin/env python3
"""Generate docs/assets/demo.gif — animated heim TUI with jumping Δ.

Usage:
  python3 scripts/gen_demo_gif.py
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs" / "assets" / "demo.gif"

# Catppuccin Mocha-ish
BG = (30, 30, 46)
BAR = (17, 17, 27)
BORDER = (69, 71, 90)
FG = (205, 214, 244)
DIM = (108, 112, 134)
GREEN = (166, 227, 161)
PEACH = (250, 179, 135)
RED = (243, 139, 168)
BLUE = (137, 180, 250)
MAUVE = (203, 166, 247)
YELLOW = (249, 226, 175)
SKY = (137, 220, 235)
TEAL = (148, 226, 213)

W, H = 920, 420
PAD = 16


def font(size: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    for path in (
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Mono.ttf",
        "/Library/Fonts/SF-Mono-Regular.otf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ):
        try:
            return ImageFont.truetype(path, size=size)
        except OSError:
            continue
    return ImageFont.load_default()


def fmt_signed(n: int) -> str:
    if n >= 1000:
        return f"+{n / 1000:.1f}k".replace(".0k", "k")
    if n > 0:
        return f"+{n}"
    return str(n)


def delta_color(n: int) -> tuple[int, int, int]:
    if n >= 2000:
        return RED
    if n >= 800:
        return PEACH
    if n > 0:
        return GREEN
    return DIM


def draw_frame(
    code: int,
    files: int,
    size_label: str,
    d5: int,
    d10: int,
    d30: int,
    d1h: int,
    d2h: int,
    rust: int,
    md: int,
    ts: int,
    wt_ins: int,
    wt_del: int,
    commit_line: str,
    status: str,
    alert: str | None,
) -> Image.Image:
    img = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(img)
    mono = font(13)
    mono_sm = font(12)
    mono_lg = font(14)

    # Title bar
    d.rounded_rectangle((0, 0, W, 36), radius=0, fill=BAR)
    for i, c in enumerate([(243, 139, 168), (249, 226, 175), (166, 227, 161)]):
        d.ellipse((14 + i * 20, 12, 26 + i * 20, 24), fill=c)
    title = "heim — stop vibe-code bloat"
    tw = d.textlength(title, font=mono_sm)
    d.text(((W - tw) / 2, 11), title, fill=DIM, font=mono_sm)

    # Monitor panel
    y0 = 48
    d.rounded_rectangle((PAD, y0, W - PAD, y0 + 100), radius=6, outline=BORDER, width=1)
    d.text((PAD + 12, y0 + 10), "┌ monitor", fill=MAUVE, font=mono)
    d.text(
        (PAD + 12, y0 + 32),
        "demo-app  ~/code/demo-app  every 10s · just now  ",
        fill=FG,
        font=mono,
    )
    d.text((PAD + 430, y0 + 32), "○ live", fill=GREEN, font=mono)

    # totals
    x = PAD + 12
    y = y0 + 54
    d.text((x, y), "code ", fill=FG, font=mono)
    x += d.textlength("code ", font=mono)
    d.text((x, y), f"{code:,}", fill=YELLOW, font=mono)
    x += d.textlength(f"{code:,}", font=mono)
    d.text(
        (x, y),
        f"  files {files}  blank —  comments —  size {size_label}",
        fill=DIM,
        font=mono,
    )

    # deltas
    x = PAD + 12
    y = y0 + 76
    d.text((x, y), "◆ Δ ", fill=SKY, font=mono)
    x += d.textlength("◆ Δ ", font=mono)
    for label, val in (
        ("5m ", d5),
        ("10m ", d10),
        ("30m ", d30),
        ("1h ", d1h),
        ("2h ", d2h),
    ):
        d.text((x, y), label, fill=SKY, font=mono)
        x += d.textlength(label, font=mono)
        s = fmt_signed(val)
        d.text((x, y), s, fill=delta_color(val), font=mono)
        x += d.textlength(s, font=mono) + 10
    d.text((x, y), "· top Rust 71%", fill=DIM, font=mono)

    # Languages + weight
    y1 = 160
    mid = W // 2
    d.rounded_rectangle((PAD, y1, mid - 6, y1 + 140), radius=6, outline=BORDER, width=1)
    d.rounded_rectangle((mid + 6, y1, W - PAD, y1 + 140), radius=6, outline=BORDER, width=1)

    d.text((PAD + 12, y1 + 10), "languages", fill=MAUVE, font=mono)
    d.text((PAD + 12, y1 + 34), "#  language     code      %", fill=DIM, font=mono)
    total_lang = max(rust + md + ts, 1)
    rows = [
        (1, "Rust", rust, rust * 100 / total_lang),
        (2, "Markdown", md, md * 100 / total_lang),
        (3, "TypeScript", ts, ts * 100 / total_lang),
    ]
    for i, (n, name, c, pct) in enumerate(rows):
        d.text(
            (PAD + 12, y1 + 56 + i * 20),
            f"{n}  {name:<11} {c:>6,}  {pct:5.0f}%",
            fill=FG,
            font=mono,
        )
    d.text(
        (PAD + 12, y1 + 118),
        f"   session Δ        {fmt_signed(d2h)}",
        fill=GREEN if d2h < 2000 else RED,
        font=mono,
    )

    d.text((mid + 18, y1 + 10), "weight", fill=MAUVE, font=mono)
    d.text((mid + 18, y1 + 34), "#  path                 size     %", fill=DIM, font=mono)
    d.text((mid + 18, y1 + 56), "1  › src                3.1M    74%", fill=FG, font=mono)
    gen_size = "120K" if d2h < 1500 else ("420K" if d2h < 3500 else "890K")
    gen_pct = 4 if d2h < 1500 else (10 if d2h < 3500 else 18)
    gen_col = FG if d2h < 2000 else PEACH
    d.text(
        (mid + 18, y1 + 76),
        f"2  › generated          {gen_size:>4}   {gen_pct:2}%",
        fill=gen_col,
        font=mono,
    )
    d.text((mid + 18, y1 + 96), "3  › target          …ignored…", fill=DIM, font=mono)
    d.text((mid + 18, y1 + 118), "   drill-down ready · Enter", fill=TEAL, font=mono)

    # Git
    y2 = 312
    d.rounded_rectangle((PAD, y2, W - PAD, y2 + 68), radius=6, outline=BORDER, width=1)
    d.text((PAD + 12, y2 + 8), "git", fill=MAUVE, font=mono)
    d.text((PAD + 12, y2 + 28), "feature/agent", fill=BLUE, font=mono)
    d.text((PAD + 140, y2 + 28), f"+{wt_ins}", fill=GREEN, font=mono)
    d.text((PAD + 190, y2 + 28), f"-{wt_del}", fill=RED, font=mono)
    d.text((PAD + 12, y2 + 48), commit_line, fill=DIM, font=mono)

    # Footer / status
    d.text((PAD, H - 22), status, fill=DIM, font=mono_sm)
    if alert:
        aw = d.textlength(alert, font=mono_sm)
        d.rounded_rectangle(
            (W - PAD - aw - 16, H - 28, W - PAD, H - 8),
            radius=4,
            fill=(60, 30, 40),
        )
        d.text((W - PAD - aw - 8, H - 24), alert, fill=RED, font=mono_sm)

    # Subtle top caption
    d.text((PAD, 38), "AI coding session · live sample", fill=DIM, font=font(10))

    return img


def main() -> None:
    # Keyframes: agent dumps code, then human/agent cleans up.
    story = [
        # t0 — quiet project
        dict(
            code=4200,
            files=42,
            size_label="1.1M",
            d5=0,
            d10=12,
            d30=40,
            d1h=80,
            d2h=120,
            rust=3000,
            md=400,
            ts=400,
            wt_ins=4,
            wt_del=1,
            commit_line="c0ffee1  +12/-2  chore: tiny fix",
            status="q quit  r refresh  ·  agent: heim --once --json .",
            alert=None,
            hold=8,
        ),
        # agent scaffolding
        dict(
            code=6100,
            files=58,
            size_label="1.8M",
            d5=180,
            d10=420,
            d30=900,
            d1h=1200,
            d2h=1900,
            rust=4100,
            md=500,
            ts=900,
            wt_ins=220,
            wt_del=8,
            commit_line="a1b2c3d  +1.9k/-3  feat: scaffold generated modules",
            status="agent writing…  Δ climbing",
            alert=None,
            hold=6,
        ),
        # vibe-code peak
        dict(
            code=12480,
            files=86,
            size_label="4.2M",
            d5=920,
            d10=1800,
            d30=4100,
            d1h=6200,
            d2h=8280,
            rust=8900,
            md=1200,
            ts=980,
            wt_ins=42,
            wt_del=8,
            commit_line="a1b2c3d  +4.8k/-12  feat: more generated helpers",
            status="too much code — self-audit",
            alert="ALERT: +8.2k / 2h",
            hold=10,
        ),
        # agent self-audit JSON beat
        dict(
            code=12480,
            files=86,
            size_label="4.2M",
            d5=920,
            d10=1800,
            d30=4100,
            d1h=6200,
            d2h=8280,
            rust=8900,
            md=1200,
            ts=980,
            wt_ins=42,
            wt_del=8,
            commit_line="heim --once --json  →  hints: ALERT: code +8280 over 2h",
            status="agent runs heim --once --json .",
            alert="trim generated/",
            hold=8,
        ),
        # after cleanup
        dict(
            code=6400,
            files=51,
            size_label="1.6M",
            d5=-800,
            d10=-400,
            d30=800,
            d1h=1400,
            d2h=2200,
            rust=4800,
            md=600,
            ts=500,
            wt_ins=0,
            wt_del=0,
            commit_line="d4e5f60  +40/-6.1k  refactor: drop unused scaffolds",
            status="cleaned · session under control",
            alert=None,
            hold=12,
        ),
    ]

    # Interpolate between keyframes for smoother motion
    frames: list[Image.Image] = []
    durations: list[int] = []

    def lerp(a: int, b: int, t: float) -> int:
        return int(round(a + (b - a) * t))

    keys = list(story)
    for i, kf in enumerate(keys):
        frames.append(draw_frame(**{k: v for k, v in kf.items() if k != "hold"}))
        durations.append(kf["hold"] * 80)

        if i + 1 < len(keys):
            nxt = keys[i + 1]
            for step in range(1, 5):
                t = step / 5
                mid = {
                    "code": lerp(kf["code"], nxt["code"], t),
                    "files": lerp(kf["files"], nxt["files"], t),
                    "size_label": nxt["size_label"] if t > 0.5 else kf["size_label"],
                    "d5": lerp(kf["d5"], nxt["d5"], t),
                    "d10": lerp(kf["d10"], nxt["d10"], t),
                    "d30": lerp(kf["d30"], nxt["d30"], t),
                    "d1h": lerp(kf["d1h"], nxt["d1h"], t),
                    "d2h": lerp(kf["d2h"], nxt["d2h"], t),
                    "rust": lerp(kf["rust"], nxt["rust"], t),
                    "md": lerp(kf["md"], nxt["md"], t),
                    "ts": lerp(kf["ts"], nxt["ts"], t),
                    "wt_ins": lerp(kf["wt_ins"], nxt["wt_ins"], t),
                    "wt_del": lerp(kf["wt_del"], nxt["wt_del"], t),
                    "commit_line": nxt["commit_line"] if t > 0.5 else kf["commit_line"],
                    "status": nxt["status"] if t > 0.4 else kf["status"],
                    "alert": nxt["alert"] if t > 0.6 else kf["alert"],
                }
                frames.append(draw_frame(**mid))
                durations.append(90)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    frames[0].save(
        OUT,
        save_all=True,
        append_images=frames[1:],
        duration=durations,
        loop=0,
        optimize=True,
    )
    print(f"wrote {OUT} ({OUT.stat().st_size // 1024} KiB, {len(frames)} frames)")


if __name__ == "__main__":
    main()
