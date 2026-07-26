//! Theme, glyphs, and animation pacing for the TUI.
//!
//! Design principles:
//! 1. Named semantic color slots — no ad-hoc colors in widgets.
//! 2. Dark neutral base + Tokyo Night–style accents (RGB).
//! 3. Spinners are 1 column so layout never shifts while animating.
//! 4. Spinner frame held for N ticks (not advanced every draw).
//! 5. Pulse via sin² for breathing status icons.
//! 6. Focus uses a brighter border; selection uses highlight bg (not reverse video).

use ratatui::style::{Color, Modifier, Style};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

// ── Night palette (Tokyo Night–inspired accents) ───────────────────────────
mod palette {
    use super::*;
    pub const BG_STORM: Color = rgb(20, 20, 20); // #141414
    pub const FG: Color = rgb(225, 225, 225); // #e1e1e1
    pub const FG_DARK: Color = rgb(200, 200, 200); // #c8c8c8
    pub const GRAY_DIM: Color = rgb(88, 88, 88); // #585858
    pub const GRAY_BRIGHT: Color = rgb(120, 120, 120); // #787878
    pub const CYAN: Color = rgb(125, 207, 255); // #7dcfff
    pub const GREEN: Color = rgb(158, 206, 106); // #9ece6a
    pub const MAGENTA: Color = rgb(187, 154, 247); // #bb9af7
    pub const RED: Color = rgb(247, 118, 142); // #f7768e
    pub const BORDER: Color = rgb(50, 50, 55); // #323237
    pub const BORDER_ACTIVE: Color = rgb(80, 80, 88); // #505058
    pub const SELECTION: Color = rgb(44, 44, 44);
}
use palette::*;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub accent_running: Color,
    pub accent_success: Color,
    pub accent_error: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub gray_dim: Color,
    pub gray_bright: Color,
    pub border: Color,
    pub border_active: Color,
    pub selection_bg: Color,
    pub delta_up: Color,
    pub delta_down: Color,
    pub path: Color,
    pub bg_base: Color,
}

impl Theme {
    /// Default dark theme used by heim surfaces.
    pub const fn night() -> Self {
        Self {
            accent: CYAN,
            accent_running: MAGENTA,
            accent_success: GREEN,
            accent_error: RED,
            text_primary: FG,
            text_secondary: FG_DARK,
            gray_dim: GRAY_DIM,
            gray_bright: GRAY_BRIGHT,
            border: BORDER,
            border_active: BORDER_ACTIVE,
            selection_bg: SELECTION,
            delta_up: GREEN,
            delta_down: RED,
            path: rgb(255, 158, 100), // Tokyo Night orange (paths)
            bg_base: BG_STORM,
        }
    }

    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn title_running(&self) -> Style {
        Style::default()
            .fg(self.accent_running)
            .add_modifier(Modifier::BOLD)
    }

    pub fn border(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn border_focused(&self) -> Style {
        Style::default().fg(self.border_active)
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.gray_dim)
    }

    pub fn body(&self) -> Style {
        Style::default().fg(self.text_secondary)
    }

    pub fn bright(&self) -> Style {
        Style::default().fg(self.text_primary)
    }

    pub fn bold_body(&self) -> Style {
        Style::default()
            .fg(self.gray_bright)
            .add_modifier(Modifier::BOLD)
    }

    pub fn path_style(&self) -> Style {
        Style::default().fg(self.path)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::night()
    }
}

// ── Glyphs (always 1 column where animated) ────────────────────────────────

/// Target UI refresh rate (animations paced for this).
/// Event loop uses `1_000_000_000 / TARGET_FPS` ns (~8.33ms) per frame.
pub const TARGET_FPS: u64 = 120;

/// Braille spinner (`⠋⠙⠹⠸⠼⠴⠦⠧`) — 8 frames, 1 column each.
pub const BRAILLE_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

/// Hold each spinner frame this many ticks.
/// ~4 spinner steps/sec at 120fps → divisor 30 (~250ms per glyph).
pub const SPINNER_DIVISOR: u64 = TARGET_FPS / 4;

pub fn spinner_frame(tick: u64) -> &'static str {
    let frame = (tick / SPINNER_DIVISOR.max(1)) as usize % BRAILLE_SPINNER.len();
    BRAILLE_SPINNER[frame]
}

/// Live monitor breath: ○ ◎ ◉ ◎.
pub const MONITOR_FRAMES: &[&str] = &["○", "◎", "◉", "◎"];

/// ~1.6s full breath cycle through 4 frames @ 120fps.
pub const MONITOR_DIVISOR: u64 = TARGET_FPS * 2 / 5; // 48

pub fn monitor_icon(tick: u64) -> &'static str {
    let frame = (tick / MONITOR_DIVISOR.max(1)) as usize % MONITOR_FRAMES.len();
    MONITOR_FRAMES[frame]
}

/// Tool-header diamond bullet.
pub const BULLET_DIAMOND: &str = "◆";

/// Expandable / drill affordance.
pub const CHEVRON: &str = "›";

/// Left accent rail for focused panels.
pub const ACCENT_RAIL: &str = "┃";

/// Smooth 0..=1 pulse (sin²).
/// `speed` is radians per tick. At 120fps, `PULSE_SPEED` ≈ 2s cycle.
pub const PULSE_SPEED: f32 = 0.013; // π / (2s * 120fps) ≈ 0.0131

pub fn pulse_brightness(tick: u64, speed: f32) -> f32 {
    let t = tick as f32 * speed;
    let s = t.sin();
    s * s
}

/// Blend accent toward dim by pulse (for live status icon color).
pub fn pulse_color(tick: u64, bright: Color, dim: Color) -> Color {
    let p = pulse_brightness(tick, PULSE_SPEED);
    match (bright, dim) {
        (Color::Rgb(br, bg, bb), Color::Rgb(dr, dg, db)) => {
            let lerp =
                |a: u8, b: u8| -> u8 { (a as f32 + (b as f32 - a as f32) * (1.0 - p)) as u8 };
            // p=1 → bright, p=0 → dim
            Color::Rgb(lerp(dr, br), lerp(dg, bg), lerp(db, bb))
        }
        _ => bright,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_holds_divisor() {
        let a = spinner_frame(0);
        let b = spinner_frame(SPINNER_DIVISOR - 1);
        assert_eq!(a, b);
        let c = spinner_frame(SPINNER_DIVISOR);
        assert_ne!(a, c);
    }

    #[test]
    fn pulse_range() {
        for t in 0..200 {
            let p = pulse_brightness(t, 0.1);
            assert!((0.0..=1.0).contains(&p));
        }
    }

    #[test]
    fn glyphs_one_col() {
        for f in BRAILLE_SPINNER {
            assert_eq!(f.chars().count(), 1);
        }
        for f in MONITOR_FRAMES {
            assert_eq!(f.chars().count(), 1);
        }
    }
}
