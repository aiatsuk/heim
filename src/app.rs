//! Application state: samples, history windows, deltas, optional `.heim` store.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::collect::{self, DirSize, LangStat, Sample, SizeBackendKind, SizeEngine};
use crate::fmt;
use crate::store::Store;

const HISTORY_CAP: usize = 10_000;

/// Code-line delta windows (wall-clock, cross-session via `.heim` history).
pub const CODE_DELTA_WINDOWS: &[(u64, &str)] = &[
    (5 * 60, "5m"),
    (10 * 60, "10m"),
    (30 * 60, "30m"),
    (60 * 60, "1h"),
    (2 * 60 * 60, "2h"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Lang,
    Weight,
    Git,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Self::Lang => Self::Weight,
            Self::Weight => Self::Git,
            Self::Git => Self::Lang,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Lang => Self::Git,
            Self::Weight => Self::Lang,
            Self::Git => Self::Weight,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lang => "languages",
            Self::Weight => "weight",
            Self::Git => "git",
        }
    }
}

pub struct App {
    pub path: PathBuf,
    pub path_label: String,
    pub interval: Duration,
    pub size_pref: SizeBackendKind,
    pub baseline: Option<Sample>,
    pub prev: Option<Sample>,
    pub last: Option<Sample>,
    pub history: VecDeque<Sample>,
    pub refreshing: bool,
    pub help: bool,
    pub status: String,
    /// When the last successful sample landed — "upd Xs" and auto-refresh schedule.
    pub last_sample_at: Option<Instant>,
    /// Private project store (`.heim/`); None if open failed.
    pub store: Option<Store>,
    /// Samples loaded from disk at session start (for status).
    pub loaded_from_store: usize,
    /// Frame counter for spinners / accent animation (paced for 120Hz UI).
    pub tick: u64,
    /// Keyboard focus for scroll / drill-down.
    pub focus: Focus,
    pub lang_sel: usize,
    pub lang_scroll: usize,
    pub weight_sel: usize,
    pub weight_scroll: usize,
    /// Path stack under project root for weight drill-down (empty = root).
    pub weight_stack: Vec<String>,
    pub weight_children: Vec<DirSize>,
    pub weight_total: u64,
    pub weight_loading: bool,
    /// Cached weight listings for instant drill-down (path → total + children).
    pub weight_cache: HashMap<PathBuf, (u64, Vec<DirSize>)>,
    /// Selected commit index in git panel (0 = newest).
    pub git_sel: usize,
    pub git_scroll: usize,
    /// Horizontal split: languages panel width % (30–70).
    pub lang_pct: u16,
    /// Git panel height in rows (including borders).
    pub git_h: u16,
    /// Active mouse drag for resize.
    pub drag: Option<Drag>,
    /// Last drawn panel rects for mouse hit-testing.
    pub hit: PanelHits,
}

#[derive(Debug, Clone, Copy)]
pub enum Drag {
    /// Vertical divider between languages and weight.
    HSplit { start_col: u16, start_pct: u16 },
    /// Horizontal top edge of git panel.
    GitTop { start_row: u16, start_h: u16 },
}

#[derive(Debug, Clone, Default)]
pub struct PanelHits {
    pub lang: ratatui::layout::Rect,
    pub weight: ratatui::layout::Rect,
    pub git: ratatui::layout::Rect,
    pub monitor: ratatui::layout::Rect,
    /// 1-col strip between lang and weight.
    pub v_split: ratatui::layout::Rect,
    /// 1-row strip on top of git for height drag.
    pub git_edge: ratatui::layout::Rect,
}

impl App {
    pub fn new(path: PathBuf, interval_secs: u64, size_pref: SizeBackendKind) -> Self {
        let path_label = collect::path_label(&path);
        let mut store = match Store::open(&path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("heim: .heim store unavailable: {e:#}");
                None
            }
        };

        let mut history = VecDeque::new();
        let mut loaded_from_store = 0;
        if let Some(st) = store.as_mut() {
            if let Ok(rows) = st.load_recent() {
                loaded_from_store = rows.len();
                for row in rows {
                    history.push_back(row.to_sample());
                }
                while history.len() > HISTORY_CAP {
                    history.pop_front();
                }
            }
            let _ = st.begin_session(&path, interval_secs);
        }

        let status = if loaded_from_store > 0 {
            format!("loaded {loaded_from_store} samples · collecting…")
        } else {
            "collecting…".into()
        };

        Self {
            path,
            path_label,
            interval: Duration::from_secs(interval_secs.clamp(1, 300)),
            size_pref,
            baseline: None,
            prev: None,
            last: None,
            history,
            refreshing: false,
            help: false,
            status,
            last_sample_at: None,
            store,
            loaded_from_store,
            tick: 0,
            focus: Focus::Lang,
            lang_sel: 0,
            lang_scroll: 0,
            weight_sel: 0,
            weight_scroll: 0,
            weight_stack: Vec::new(),
            weight_children: Vec::new(),
            weight_total: 0,
            weight_loading: false,
            weight_cache: HashMap::new(),
            git_sel: 0,
            git_scroll: 0,
            lang_pct: 54,
            git_h: 12,
            drag: None,
            hit: PanelHits::default(),
        }
    }

    pub fn project_name(&self) -> String {
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path_label.clone())
    }

    pub fn bump_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Scroll the focused list viewport (does not move selection) — natural wheel feel.
    pub fn scroll_view(&mut self, delta: i32, visible: usize) {
        let vis = visible.max(1);
        match self.focus {
            Focus::Lang => {
                let n = self.langs().len();
                let max = n.saturating_sub(vis);
                let next = self.lang_scroll as i32 + delta;
                self.lang_scroll = next.clamp(0, max as i32) as usize;
            }
            Focus::Weight => {
                let n = self.weight_children.len();
                let max = n.saturating_sub(vis);
                let next = self.weight_scroll as i32 + delta;
                self.weight_scroll = next.clamp(0, max as i32) as usize;
            }
            Focus::Git => {
                let n = self
                    .last
                    .as_ref()
                    .and_then(|s| s.git.as_ref())
                    .map(|g| g.commits.len())
                    .unwrap_or(0);
                let max = n.saturating_sub(vis);
                let next = self.git_scroll as i32 + delta;
                self.git_scroll = next.clamp(0, max as i32) as usize;
            }
        }
    }

    pub fn weight_cwd(&self) -> PathBuf {
        let mut p = self.path.clone();
        for seg in &self.weight_stack {
            p.push(seg);
        }
        p
    }

    pub fn weight_breadcrumb(&self) -> String {
        if self.weight_stack.is_empty() {
            ".".into()
        } else {
            self.weight_stack.join("/")
        }
    }

    pub fn apply_weight_listing(&mut self, path: PathBuf, total: u64, children: Vec<DirSize>) {
        self.weight_cache.insert(path, (total, children.clone()));
        self.weight_total = total;
        self.weight_children = children;
        self.weight_loading = false;
        if self.weight_sel >= self.weight_children.len() && !self.weight_children.is_empty() {
            self.weight_sel = self.weight_children.len() - 1;
        }
        if self.weight_children.is_empty() {
            self.weight_sel = 0;
        }
    }

    /// Apply listing from cache if present. Returns true if served from cache.
    pub fn try_weight_from_cache(&mut self, path: &PathBuf) -> bool {
        if let Some((total, children)) = self.weight_cache.get(path).cloned() {
            self.weight_total = total;
            self.weight_children = children;
            self.weight_loading = false;
            self.weight_sel = 0;
            self.weight_scroll = 0;
            true
        } else {
            false
        }
    }

    /// Keep selection inside the viewport after j/k (selection-driven).
    pub fn ensure_sel_visible(&mut self, visible: usize) {
        let vis = visible.max(1);
        let nlang = self.langs().len();
        if nlang > 0 {
            self.lang_sel = self.lang_sel.min(nlang - 1);
            if self.lang_sel < self.lang_scroll {
                self.lang_scroll = self.lang_sel;
            }
            if self.lang_sel >= self.lang_scroll + vis {
                self.lang_scroll = self.lang_sel + 1 - vis;
            }
        }
        let nw = self.weight_children.len();
        if nw > 0 {
            self.weight_sel = self.weight_sel.min(nw - 1);
            if self.weight_sel < self.weight_scroll {
                self.weight_scroll = self.weight_sel;
            }
            if self.weight_sel >= self.weight_scroll + vis {
                self.weight_scroll = self.weight_sel + 1 - vis;
            }
        }
        let ng = self
            .last
            .as_ref()
            .and_then(|s| s.git.as_ref())
            .map(|g| g.commits.len())
            .unwrap_or(0);
        // git panel uses its own visible height — approximate with vis
        if ng > 0 {
            self.git_sel = self.git_sel.min(ng - 1);
            if self.git_sel < self.git_scroll {
                self.git_scroll = self.git_sel;
            }
            if self.git_sel >= self.git_scroll + vis {
                self.git_scroll = self.git_sel + 1 - vis;
            }
        }
    }

    pub fn move_sel(&mut self, delta: i32, visible: usize) {
        match self.focus {
            Focus::Lang => {
                let n = self.langs().len();
                if n == 0 {
                    return;
                }
                let cur = self.lang_sel as i32 + delta;
                self.lang_sel = cur.clamp(0, n as i32 - 1) as usize;
            }
            Focus::Weight => {
                let n = self.weight_children.len();
                if n == 0 {
                    return;
                }
                let cur = self.weight_sel as i32 + delta;
                self.weight_sel = cur.clamp(0, n as i32 - 1) as usize;
            }
            Focus::Git => {
                let n = self
                    .last
                    .as_ref()
                    .and_then(|s| s.git.as_ref())
                    .map(|g| g.commits.len())
                    .unwrap_or(0);
                if n == 0 {
                    return;
                }
                let cur = self.git_sel as i32 + delta;
                self.git_sel = cur.clamp(0, n as i32 - 1) as usize;
            }
        }
        self.ensure_sel_visible(visible);
    }

    pub fn clamp_layout(&mut self, term_h: u16) {
        self.lang_pct = self.lang_pct.clamp(30, 70);
        // Leave room for monitor(5)+ranks(min4)+footer(1); git may collapse to 3.
        let max_git = term_h.saturating_sub(10).max(3);
        self.git_h = self.git_h.clamp(3, max_git);
    }

    /// Preferred git panel height for this frame (auto-collapse when empty).
    pub fn effective_git_h(&self, term_h: u16) -> u16 {
        let max_git = term_h.saturating_sub(10).max(3);
        let preferred = self.git_h.clamp(3, max_git);
        let n = self
            .last
            .as_ref()
            .and_then(|s| s.git.as_ref())
            .map(|g| g.commits.len() as u16)
            .unwrap_or(0);
        if n > 0 {
            // borders(2) + working-tree line(1) + commits — don't leave a hollow panel
            // when the log is short; `git_h` is an upper cap (user drag).
            let natural = n.saturating_add(3).clamp(5, max_git);
            preferred.min(natural).max(5).min(max_git)
        } else {
            // Empty / error / loading: single status line.
            3.min(max_git)
        }
    }

    /// Any non-zero language delta worth showing a Δ column for.
    pub fn has_lang_deltas(&self) -> bool {
        self.langs().iter().any(|l| {
            nonzero_delta(
                self.lang_delta_session(&l.name)
                    .or_else(|| self.lang_delta_interval(&l.name)),
            )
        })
    }

    /// Dir deltas only make sense at weight root (session/interval vs top_dirs).
    pub fn has_dir_deltas(&self) -> bool {
        if !self.weight_stack.is_empty() {
            return false;
        }
        self.weight_children.iter().any(|d| {
            nonzero_delta(
                self.dir_delta_session(&d.name)
                    .or_else(|| self.dir_delta_interval(&d.name)),
            )
        })
    }

    /// Code-line deltas for each standard window: `(label, delta)`.
    /// `None` = not enough history yet for that window.
    pub fn code_window_deltas(&self) -> Vec<(&'static str, Option<i64>)> {
        CODE_DELTA_WINDOWS
            .iter()
            .map(|&(secs, label)| (label, self.window_code_delta(Duration::from_secs(secs))))
            .collect()
    }

    /// Secondary insight chips (no totals, no window strip — that is drawn separately).
    pub fn insight_chips(&self) -> Vec<String> {
        let mut chips = Vec::new();
        let Some(cur) = self.last.as_ref() else {
            return chips;
        };

        if let Some(loc) = &cur.loc {
            if let Some(top) = loc.langs.first() {
                chips.push(format!(
                    "top {} {:.0}%",
                    top.name,
                    fmt::pct(top.code, loc.code)
                ));
            }
            let n = loc.langs.len();
            if n > 0 {
                chips.push(format!("{n} lang{}", if n == 1 { "" } else { "s" }));
            }
        }

        chips.push(format!("via {}", cur.size_engine.label()));

        if let Some(d) = self.size_delta_session() {
            if d != 0 {
                chips.push(format!("size {}", fmt::signed_bytes(d)));
            }
        }

        // Git lives in its own panel — only surface errors up here.
        if cur.git.is_none() {
            if let Some(e) = &cur.git_err {
                chips.push(format!("git {e}"));
            }
        }

        if cur.duration.as_secs_f64() >= 0.5 {
            chips.push(format!("took {:.1}s", cur.duration.as_secs_f64()));
        }

        if self.loaded_from_store > 0 {
            chips.push(format!("{} stored", self.loaded_from_store));
        }

        chips
    }

    /// Enter selected weight entry if it is a directory.
    /// Returns `Some(path)` needing async fetch, or `None` if cache hit / not a dir.
    pub fn weight_enter(&mut self) -> Option<PathBuf> {
        let ent = self.weight_children.get(self.weight_sel)?;
        let name = ent.name.clone();
        let p = self.weight_cwd().join(&name);
        if !p.is_dir() {
            return None;
        }
        self.weight_stack.push(name);
        self.weight_sel = 0;
        self.weight_scroll = 0;
        let cwd = self.weight_cwd();
        if self.try_weight_from_cache(&cwd) {
            return None;
        }
        self.weight_loading = true;
        Some(cwd)
    }

    /// Go up one weight level.
    pub fn weight_up(&mut self) -> Option<PathBuf> {
        if self.weight_stack.is_empty() {
            return None;
        }
        self.weight_stack.pop();
        self.weight_sel = 0;
        self.weight_scroll = 0;
        let cwd = self.weight_cwd();
        if self.try_weight_from_cache(&cwd) {
            return None;
        }
        self.weight_loading = true;
        Some(cwd)
    }

    /// Short phase label for status strip.
    pub fn phase_label(&self) -> &'static str {
        if self.refreshing {
            "collecting"
        } else if self.last.is_none() {
            "starting"
        } else {
            "live"
        }
    }

    /// Value only (no "upd" prefix): `now`, `3s`, `—`, `…`.
    pub fn update_age_value(&self) -> String {
        match self.last_sample_at {
            None if self.refreshing => "…".into(),
            None => "—".into(),
            Some(t) => fmt::hum_dur_age(t.elapsed()),
        }
    }

    pub fn size_engine_label(&self) -> &'static str {
        self.last
            .as_ref()
            .map(|s| s.size_engine.label())
            .unwrap_or_else(|| match collect::resolve_engine(self.size_pref) {
                SizeEngine::Dust => "dust",
                SizeEngine::Walk => "walk",
            })
    }

    pub fn apply_sample(&mut self, s: Sample) {
        self.prev = self.last.take();
        // Session baseline = first sample of *this* process, not disk history.
        if self.baseline.is_none() {
            self.baseline = Some(s.clone());
        }
        self.status = format!("{:.2}s", s.duration.as_secs_f64());
        if let Some(st) = self.store.as_mut() {
            if let Err(e) = st.record_sample(&s) {
                self.status = format!("{:.2}s · store err", s.duration.as_secs_f64());
                let _ = e;
            }
        }
        // Root weight listing follows full samples only when not drilled in.
        if self.weight_stack.is_empty() {
            self.weight_children = s.top_dirs.clone();
            self.weight_total = s.size_bytes;
            self.weight_cache
                .insert(self.path.clone(), (s.size_bytes, s.top_dirs.clone()));
        }
        if let Some(g) = &s.git {
            if self.git_sel >= g.commits.len() && !g.commits.is_empty() {
                self.git_sel = 0;
            }
        }
        self.last_sample_at = Some(s.at);
        self.last = Some(s.clone());
        self.history.push_back(s);
        while self.history.len() > HISTORY_CAP {
            self.history.pop_front();
        }
        self.refreshing = false;
    }

    pub fn due(&self) -> bool {
        if self.refreshing {
            return false;
        }
        match self.last_sample_at {
            None => true,
            Some(t) => t.elapsed() >= self.interval,
        }
    }

    pub fn bump_interval(&mut self, dir: i32) {
        let cur = self.interval.as_secs() as i64;
        let next = (cur + dir as i64).clamp(1, 300) as u64;
        self.interval = Duration::from_secs(next);
    }

    pub fn size_delta_session(&self) -> Option<i64> {
        let a = self.last.as_ref()?.size_bytes as i64;
        let b = self.baseline.as_ref()?.size_bytes as i64;
        Some(a - b)
    }

    pub fn lang_delta_session(&self, name: &str) -> Option<i64> {
        let cur = lang_code(self.last.as_ref()?.loc.as_ref()?, name)?;
        // Some(series) = baseline has loc (missing name → 0); None = no baseline loc.
        let base_series = self
            .baseline
            .as_ref()
            .and_then(|s| s.loc.as_ref())
            .map(|l| lang_code(l, name));
        Some(session_named_delta(cur, base_series))
    }

    pub fn lang_delta_interval(&self, name: &str) -> Option<i64> {
        interval_named_delta(
            lang_code(self.last.as_ref()?.loc.as_ref()?, name),
            self.prev
                .as_ref()
                .and_then(|s| s.loc.as_ref())
                .and_then(|l| lang_code(l, name)),
        )
    }

    pub fn dir_delta_session(&self, name: &str) -> Option<i64> {
        let cur = dir_bytes(self.last.as_ref()?, name)?;
        let base_series = self.baseline.as_ref().map(|s| dir_bytes(s, name));
        Some(session_named_delta(cur, base_series))
    }

    pub fn dir_delta_interval(&self, name: &str) -> Option<i64> {
        interval_named_delta(
            dir_bytes(self.last.as_ref()?, name),
            self.prev.as_ref().and_then(|s| dir_bytes(s, name)),
        )
    }

    /// Enough wall-clock history to evaluate this window.
    pub fn window_ready(&self, window: Duration) -> bool {
        let Some(oldest) = self.history.front() else {
            return false;
        };
        let Ok(span) = SystemTime::now().duration_since(oldest.wall) else {
            return false;
        };
        span >= window && self.last.is_some()
    }

    /// Code-line delta vs oldest sample still within `window` (wall clock, cross-session).
    pub fn window_code_delta(&self, window: Duration) -> Option<i64> {
        let cur = self.last.as_ref()?;
        if !self.window_ready(window) {
            return None;
        }
        let now = SystemTime::now();
        let old = self
            .history
            .iter()
            .find(|s| now.duration_since(s.wall).unwrap_or(Duration::MAX) <= window)?;
        match (&cur.loc, &old.loc) {
            (Some(c), Some(o)) => Some(c.code as i64 - o.code as i64),
            _ => None,
        }
    }

    pub fn langs(&self) -> &[LangStat] {
        self.last
            .as_ref()
            .and_then(|s| s.loc.as_ref())
            .map(|l| l.langs.as_slice())
            .unwrap_or(&[])
    }
}

fn lang_code(loc: &collect::LocStats, name: &str) -> Option<u64> {
    loc.langs.iter().find(|l| l.name == name).map(|l| l.code)
}

fn dir_bytes(s: &Sample, name: &str) -> Option<u64> {
    s.top_dirs.iter().find(|d| d.name == name).map(|d| d.bytes)
}

/// Session delta.
/// - `base_series = None` → no baseline series (treat as `cur`, delta 0)
/// - `base_series = Some(None)` → series exists, name missing (treat as 0)
/// - `base_series = Some(Some(v))` → baseline value `v`
fn session_named_delta(cur: u64, base_series: Option<Option<u64>>) -> i64 {
    let base = match base_series {
        None => cur,
        Some(v) => v.unwrap_or(0),
    };
    cur as i64 - base as i64
}

/// Interval delta: needs both current and previous values.
fn interval_named_delta(cur: Option<u64>, prev: Option<u64>) -> Option<i64> {
    Some(cur? as i64 - prev? as i64)
}

fn nonzero_delta(v: Option<i64>) -> bool {
    matches!(v, Some(n) if n != 0)
}

fn format_opt_delta(v: Option<i64>, signed: impl FnOnce(i64) -> String) -> String {
    match v {
        None => "—".into(),
        Some(0) => "·".into(),
        Some(n) => signed(n),
    }
}

pub fn opt_delta_i64(v: Option<i64>) -> String {
    format_opt_delta(v, fmt::signed_i64)
}

pub fn opt_delta_bytes(v: Option<i64>) -> String {
    format_opt_delta(v, fmt::signed_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{LocStats, Sample, SizeEngine};

    fn sample(code: u64, size: u64, ago: Duration) -> Sample {
        let wall = SystemTime::now() - ago;
        Sample {
            at: Instant::now().checked_sub(ago).unwrap_or_else(Instant::now),
            wall,
            size_bytes: size,
            top_dirs: vec![],
            size_engine: SizeEngine::Walk,
            loc: Some(LocStats {
                files: 1,
                blank: 0,
                comment: 0,
                code,
                langs: vec![LangStat {
                    name: "Rust".into(),
                    blank: 0,
                    comment: 0,
                    code,
                }],
            }),
            loc_err: None,
            git: None,
            git_err: None,
            duration: Duration::from_millis(1),
        }
    }

    #[test]
    fn update_age_resets_on_sample() {
        let dir = std::env::temp_dir().join(format!("heim-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(dir.clone(), 10, SizeBackendKind::Walk);
        assert_eq!(app.update_age_value(), "—");
        app.refreshing = true;
        assert_eq!(app.update_age_value(), "…");
        app.apply_sample(sample(100, 1000, Duration::ZERO));
        let v = app.update_age_value();
        assert!(v == "now" || v.ends_with('s'), "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_needs_span() {
        let dir = std::env::temp_dir().join(format!("heim-app2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(dir.clone(), 10, SizeBackendKind::Walk);
        // clear any loaded
        app.history.clear();
        app.apply_sample(sample(100, 1000, Duration::ZERO));
        assert!(app.window_code_delta(Duration::from_secs(300)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_delta_code() {
        let dir = std::env::temp_dir().join(format!("heim-app3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(dir.clone(), 10, SizeBackendKind::Walk);
        app.history.clear();
        app.baseline = None;
        app.last = None;
        app.prev = None;

        let mut s1 = sample(100, 1000, Duration::from_secs(200));
        s1.at = Instant::now() - Duration::from_secs(200);
        app.apply_sample(s1);
        // fake older wall for span: re-push history with correct walls
        app.history.clear();
        let mut old = sample(100, 1000, Duration::from_secs(400));
        old.wall = SystemTime::now() - Duration::from_secs(400);
        let mut mid = sample(100, 1000, Duration::from_secs(200));
        mid.wall = SystemTime::now() - Duration::from_secs(200);
        let mut cur = sample(130, 2000, Duration::ZERO);
        cur.wall = SystemTime::now();
        app.history.push_back(old);
        app.history.push_back(mid.clone());
        app.baseline = Some(mid);
        app.last = Some(cur.clone());
        app.history.push_back(cur);

        assert_eq!(app.window_code_delta(Duration::from_secs(300)), Some(30));

        // 5m window (300s) ready; 10m (600s) not ready with only 400s span
        let deltas = app.code_window_deltas();
        assert_eq!(deltas.len(), 5);
        assert_eq!(deltas[0].0, "5m");
        assert_eq!(deltas[0].1, Some(30)); // oldest within 5m → mid@200s, +30
        assert_eq!(deltas[1].1, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
