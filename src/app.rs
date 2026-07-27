//! Application state: samples, history windows, deltas, optional `.heim` store.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::collect::{self, DirSize, LangStat, Sample, SizeBackendKind, SizeEngine, WeightMode};
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
    /// Whether samples are written to disk at all (store + `stats.json`).
    /// False for pure-render tests, which must not touch the filesystem.
    pub persist: bool,
    /// Samples loaded from disk at session start (for status).
    pub loaded_from_store: usize,
    /// Frame counter for spinners / accent animation (paced for 120Hz UI).
    pub tick: u64,
    /// Something changed that the current frame does not reflect — redraw.
    /// Set by state updates, input, and resize; cleared after a draw.
    pub dirty: bool,
    /// Animation state of the last drawn frame; a change forces a redraw.
    pub last_anim: crate::theme::AnimFrame,
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
    /// Size (bytes) vs code (LOC) for the weight panel + drill-down.
    pub weight_mode: WeightMode,
    /// Cached weight listings per path and metric mode.
    pub weight_cache: HashMap<(PathBuf, WeightMode), (u64, Vec<DirSize>)>,
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
            persist: true,
            loaded_from_store,
            tick: 0,
            dirty: true,
            last_anim: crate::theme::AnimFrame::default(),
            focus: Focus::Lang,
            lang_sel: 0,
            lang_scroll: 0,
            weight_sel: 0,
            weight_scroll: 0,
            weight_stack: Vec::new(),
            weight_children: Vec::new(),
            weight_total: 0,
            weight_loading: false,
            weight_mode: WeightMode::Size,
            weight_cache: HashMap::new(),
            git_sel: 0,
            git_scroll: 0,
            lang_pct: 54,
            git_h: 12,
            drag: None,
            hit: PanelHits::default(),
        }
    }

    /// An app that never touches the filesystem: no `.heim/` store, no
    /// `stats.json` write on `apply_sample`.
    ///
    /// For render tests. They used to build a real `App` on a hardcoded
    /// `/tmp/frontend`, so running the suite opened a session and appended
    /// samples to a store shared by every concurrent test process — that store
    /// is where the 15% interleaved-write corruption was first measured.
    #[cfg(test)]
    pub fn without_store(path: PathBuf, interval_secs: u64, size_pref: SizeBackendKind) -> Self {
        let path_label = collect::path_label(&path);
        Self {
            path,
            path_label,
            interval: Duration::from_secs(interval_secs.clamp(1, 300)),
            size_pref,
            baseline: None,
            prev: None,
            last: None,
            history: VecDeque::new(),
            refreshing: false,
            help: false,
            status: "collecting…".into(),
            last_sample_at: None,
            store: None,
            persist: false,
            loaded_from_store: 0,
            tick: 0,
            dirty: true,
            last_anim: crate::theme::AnimFrame::default(),
            focus: Focus::Lang,
            lang_sel: 0,
            lang_scroll: 0,
            weight_sel: 0,
            weight_scroll: 0,
            weight_stack: Vec::new(),
            weight_children: Vec::new(),
            weight_total: 0,
            weight_loading: false,
            weight_mode: WeightMode::Size,
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

    pub fn apply_weight_listing(
        &mut self,
        path: PathBuf,
        mode: WeightMode,
        total: u64,
        children: Vec<DirSize>,
    ) {
        self.dirty = true;
        self.weight_cache
            .insert((path, mode), (total, children.clone()));
        // Only paint if this listing matches the mode the user is looking at.
        if mode != self.weight_mode {
            return;
        }
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
    pub fn try_weight_from_cache(&mut self, path: &Path) -> bool {
        let key = (path.to_path_buf(), self.weight_mode);
        if let Some((total, children)) = self.weight_cache.get(&key).cloned() {
            self.dirty = true;
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

    /// Toggle size ↔ code for the weight panel. Returns a path to fetch when
    /// the listing is not cached for the new mode.
    pub fn toggle_weight_mode(&mut self) -> Option<PathBuf> {
        self.dirty = true;
        self.weight_mode = self.weight_mode.toggle();
        self.weight_sel = 0;
        self.weight_scroll = 0;
        let cwd = self.weight_cwd();
        if self.try_weight_from_cache(&cwd) {
            self.status = format!("weight · {}", self.weight_mode.label());
            return None;
        }
        self.weight_loading = true;
        self.weight_children.clear();
        self.weight_total = 0;
        self.status = format!("weight · {} · loading…", self.weight_mode.label());
        Some(cwd)
    }

    /// Keep selection inside the viewport after j/k (selection-driven).
    /// Clamp one list's selection and viewport to its own row budget.
    fn clamp_view(n: usize, sel: &mut usize, scroll: &mut usize, visible: usize) {
        if n == 0 {
            *sel = 0;
            *scroll = 0;
            return;
        }
        let vis = visible.max(1);
        *sel = (*sel).min(n - 1);
        if *sel < *scroll {
            *scroll = *sel;
        }
        if *sel >= *scroll + vis {
            *scroll = *sel + 1 - vis;
        }
        // Downward clamp. Without it a widening panel or a shrinking list left
        // `scroll` stranded past the end and the head rows stayed hidden with
        // no way to scroll back — the upward guards above never lower it.
        *scroll = (*scroll).min(n.saturating_sub(vis));
    }

    /// Keep each selection inside its own viewport.
    ///
    /// Takes two budgets because the panels have different heights: languages
    /// and weight share the ranks row, git has its own. Passing one budget for
    /// all three meant focusing the short git panel rewrote `lang_scroll` and
    /// `weight_scroll` with git's height — the top languages silently vanished,
    /// the scroll hint is gated on `n > vis` so it did not even render, and the
    /// clamp was one-directional so tabbing back never repaired it.
    pub fn ensure_sel_visible(&mut self, list_vis: usize, git_vis: usize) {
        let nlang = self.langs().len();
        let (mut lang_sel, mut lang_scroll) = (self.lang_sel, self.lang_scroll);
        Self::clamp_view(nlang, &mut lang_sel, &mut lang_scroll, list_vis);
        self.lang_sel = lang_sel;
        self.lang_scroll = lang_scroll;

        let nw = self.weight_children.len();
        let (mut w_sel, mut w_scroll) = (self.weight_sel, self.weight_scroll);
        Self::clamp_view(nw, &mut w_sel, &mut w_scroll, list_vis);
        self.weight_sel = w_sel;
        self.weight_scroll = w_scroll;

        let ng = self
            .last
            .as_ref()
            .and_then(|s| s.git.as_ref())
            .map(|g| g.commits.len())
            .unwrap_or(0);
        let (mut g_sel, mut g_scroll) = (self.git_sel, self.git_scroll);
        Self::clamp_view(ng, &mut g_sel, &mut g_scroll, git_vis);
        self.git_sel = g_sel;
        self.git_scroll = g_scroll;
    }

    pub fn move_sel(&mut self, delta: i32, list_vis: usize, git_vis: usize) {
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
        self.ensure_sel_visible(list_vis, git_vis);
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
            //
            // On a very short terminal `max_git` floors at 3, below the natural
            // minimum of 5. `clamp(5, max_git)` would then be an inverted range
            // and panic, so apply the ceiling last instead.
            let natural = n.saturating_add(3).max(5).min(max_git);
            preferred.min(natural).max(3).min(max_git)
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

    /// Dir deltas only make sense at weight root in **size** mode
    /// (session/interval vs sample `top_dirs` bytes).
    pub fn has_dir_deltas(&self) -> bool {
        if self.weight_mode != WeightMode::Size {
            return false;
        }
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
        self.dirty = true;
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
        // Root size listing follows full samples when not drilled in.
        // Code mode is refreshed separately (async tokei per-child listing).
        if self.weight_stack.is_empty() {
            self.weight_cache.insert(
                (self.path.clone(), WeightMode::Size),
                (s.size_bytes, s.top_dirs.clone()),
            );
            if self.weight_mode == WeightMode::Size {
                self.weight_children = s.top_dirs.clone();
                self.weight_total = s.size_bytes;
            }
        }
        if let Some(g) = &s.git {
            if self.git_sel >= g.commits.len() && !g.commits.is_empty() {
                self.git_sel = 0;
            }
        }
        self.last_sample_at = Some(s.at);
        self.last = Some(s.clone());

        // History is read only for `wall`, `loc.code` and `size_bytes`
        // (`window_pair`, `report::history_meta`). Retaining whole samples kept
        // up to `GIT_LOG_LIMIT` commits, the churn vector, `top_dirs` and the
        // per-language breakdown alive for all 10k retained entries — ~17.8 KB
        // each. This mirrors `store::StoredSample::to_sample`, so live and
        // disk-loaded rows stay interchangeable.
        //
        // Assignment, not `.clear()`: clearing retains the Vec's capacity, which
        // is most of the footprint.
        let mut slim = s;
        slim.top_dirs = Vec::new();
        slim.git = None;
        slim.git_err = None;
        if let Some(l) = slim.loc.as_mut() {
            l.langs = Vec::new();
        }
        self.history.push_back(slim);
        while self.history.len() > HISTORY_CAP {
            self.history.pop_front();
        }
        self.refreshing = false;
        // Keep a machine-readable snapshot for agents (`heim --once --json` / `.heim/stats.json`).
        if self.persist {
            let report = crate::report::Report::from_app(self);
            let _ = crate::report::write_store_stats(&self.path, &report);
        }
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
        self.window_pair(window).is_some()
    }

    /// Code-line delta vs oldest sample still within `window` (wall clock, cross-session).
    pub fn window_code_delta(&self, window: Duration) -> Option<i64> {
        let (cur, old) = self.window_pair(window)?;
        match (&cur.loc, &old.loc) {
            (Some(c), Some(o)) => Some(c.code as i64 - o.code as i64),
            _ => None,
        }
    }

    /// Disk size delta vs oldest sample still within `window`.
    pub fn window_size_delta(&self, window: Duration) -> Option<i64> {
        let (cur, old) = self.window_pair(window)?;
        Some(cur.size_bytes as i64 - old.size_bytes as i64)
    }

    /// Current sample + oldest sample still inside the wall-clock window.
    /// Current sample + the baseline it should be compared against, or `None`
    /// when this window cannot be answered honestly.
    ///
    /// Two independent guards, both load-bearing:
    ///
    /// 1. History must actually **span** the window, otherwise a "2h" delta gets
    ///    measured over whatever shorter slice happens to exist and over-claims.
    /// 2. The baseline must **predate** the current sample. After heim has been
    ///    off for a while, the only sample inside the window is the one just
    ///    taken, and `cur - cur == 0` is indistinguishable from "nothing was
    ///    written" — the single most misleading answer heim can give an agent
    ///    that is asking precisely that question.
    fn window_pair(&self, window: Duration) -> Option<(&Sample, &Sample)> {
        let cur = self.last.as_ref()?;
        let now = SystemTime::now();

        let oldest = self.history.front()?;
        if now.duration_since(oldest.wall).unwrap_or(Duration::ZERO) < window {
            return None;
        }

        let old = self
            .history
            .iter()
            .find(|s| now.duration_since(s.wall).unwrap_or(Duration::MAX) <= window)?;
        if old.wall >= cur.wall {
            return None;
        }
        Some((cur, old))
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

    /// Focusing the short git panel must not rewrite the language viewport.
    ///
    /// Regression: one row budget was applied to all three lists, so tabbing to
    /// git (≈4 rows) scrolled the 24-row languages panel as if it were 4 rows
    /// tall. The top languages vanished, the `x-y/n` hint is gated on `n > vis`
    /// so it did not render, and the clamp was upward-only so tabbing back never
    /// restored them.
    #[test]
    fn focusing_git_does_not_scroll_the_language_panel() {
        let dir = std::env::temp_dir().join(format!("heim-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(dir.clone(), 10, SizeBackendKind::Walk);

        let mut s = sample(100, 1000, Duration::ZERO);
        if let Some(loc) = s.loc.as_mut() {
            loc.langs = (0..16)
                .map(|i| LangStat {
                    name: format!("lang{i}"),
                    blank: 0,
                    comment: 0,
                    code: 100 - i,
                })
                .collect();
        }
        s.git = Some(crate::collect::GitStats {
            branch: "main".into(),
            ins: 0,
            del: 0,
            commits: vec![crate::collect::GitCommit::default(); 20],
            churn: vec![],
        });
        app.last = Some(s);

        // Wide languages panel (24 rows), short git panel (4 rows).
        app.lang_sel = 10;
        app.lang_scroll = 0;
        app.ensure_sel_visible(24, 4);
        assert_eq!(
            app.lang_scroll, 0,
            "git's 4-row budget must not scroll a 24-row language panel"
        );

        // Git's own selection still gets clamped to git's budget.
        app.git_sel = 19;
        app.ensure_sel_visible(24, 4);
        assert_eq!(app.git_scroll, 16, "git list must scroll within its 4 rows");

        // Downward clamp: a panel that grows re-reveals the head rows.
        app.lang_sel = 0;
        app.lang_scroll = 12;
        app.ensure_sel_visible(24, 4);
        assert_eq!(app.lang_scroll, 0, "widened panel must not strand scroll");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `effective_git_h` used to build an inverted `clamp(5, max_git)` range and
    /// panic outright whenever the terminal was shorter than 15 rows.
    #[test]
    fn git_height_survives_short_terminals() {
        let dir = std::env::temp_dir().join(format!("heim-short-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(dir.clone(), 10, SizeBackendKind::Walk);

        let mut s = sample(100, 1000, Duration::ZERO);
        s.git = Some(crate::collect::GitStats {
            branch: "main".into(),
            ins: 0,
            del: 0,
            commits: vec![crate::collect::GitCommit::default(); 4],
            churn: vec![],
        });
        app.last = Some(s);

        for term_h in 0u16..40 {
            app.clamp_layout(term_h);
            let h = app.effective_git_h(term_h);
            assert!(h >= 3, "term_h={term_h} gave git_h={h}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A gap in history must read as "no data", never as "no growth".
    ///
    /// Regression: `window_ready` only compared the total span from the oldest
    /// sample, so after heim had been off for days a fresh start reported
    /// `ready = true` with `code = 0` for every window — the baseline the search
    /// found was the current sample itself, and `cur - cur` is 0. An agent
    /// asking "how many lines landed in the last 2h" was told "none".
    #[test]
    fn stale_history_is_not_ready_and_reports_no_delta() {
        let dir = std::env::temp_dir().join(format!("heim-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(dir.clone(), 10, SizeBackendKind::Walk);
        app.history.clear();
        // heim was off for days: the only sample inside the 2h window is the
        // one just taken, so there is no baseline to measure against.
        let old = sample(1000, 10_000, Duration::from_secs(5 * 24 * 3600));
        let mut cur = sample(1000, 10_000, Duration::ZERO);
        cur.wall = SystemTime::now();

        app.history.push_back(old);
        app.history.push_back(cur.clone());
        app.last = Some(cur);

        let two_h = Duration::from_secs(2 * 3600);
        assert_eq!(
            app.window_code_delta(two_h),
            None,
            "a gap in history must read as no data, not as zero growth"
        );
        assert!(
            !app.window_ready(two_h),
            "a window whose only in-range sample is `cur` is not ready"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
