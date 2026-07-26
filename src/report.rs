//! Machine-readable project stats for agents / scripts (JSON).

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::app::{App, CODE_DELTA_WINDOWS};
use crate::collect::{self, Sample, SizeBackendKind};
use crate::fmt;
use crate::store::{self, Store};

/// Extra windows for agent reports (beyond the TUI strip).
const AGENT_EXTRA_WINDOWS: &[(u64, &str)] = &[
    (4 * 60 * 60, "4h"),
    (8 * 60 * 60, "8h"),
    (24 * 60 * 60, "1d"),
];

/// Default snapshot path under the private store (AI agents can read this file).
pub const STATS_FILE: &str = "stats.json";

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub version: &'static str,
    pub purpose: &'static str,
    pub path: String,
    pub collected_at: String,
    pub collected_at_unix: u64,
    pub took_secs: f64,
    pub loc: Option<LocReport>,
    pub loc_error: Option<String>,
    pub size: SizeReport,
    pub git: Option<GitReport>,
    pub git_error: Option<String>,
    /// Deltas over wall-clock windows (uses live + `.heim` history when available).
    pub deltas: Vec<WindowDeltaReport>,
    pub session: SessionReport,
    pub history: HistoryReport,
    pub hints: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LocReport {
    pub code: u64,
    pub files: u64,
    pub blank: u64,
    pub comment: u64,
    pub languages: Vec<LangReport>,
}

#[derive(Debug, Serialize)]
pub struct LangReport {
    pub name: String,
    pub code: u64,
    pub blank: u64,
    pub comment: u64,
    pub pct: f64,
}

#[derive(Debug, Serialize)]
pub struct SizeReport {
    pub bytes: u64,
    pub human: String,
    pub engine: String,
    pub top: Vec<DirReport>,
}

#[derive(Debug, Serialize)]
pub struct DirReport {
    pub name: String,
    pub bytes: u64,
    pub human: String,
    pub pct: f64,
}

#[derive(Debug, Serialize)]
pub struct GitReport {
    pub branch: String,
    pub working_tree_insertions: u64,
    pub working_tree_deletions: u64,
    pub recent_commits: Vec<CommitReport>,
}

#[derive(Debug, Serialize)]
pub struct CommitReport {
    pub short: String,
    pub subject: String,
    pub author: String,
    pub insertions: u64,
    pub deletions: u64,
}

#[derive(Debug, Serialize)]
pub struct WindowDeltaReport {
    pub window: String,
    pub window_secs: u64,
    /// `true` when enough heim sample history exists for `code` / `size_bytes`.
    pub ready: bool,
    pub code: Option<i64>,
    pub size_bytes: Option<i64>,
    /// Git commit insertions in this wall-clock window (sum of shortstats).
    pub insertions: u64,
    /// Git commit deletions in this wall-clock window (sum of shortstats).
    pub deletions: u64,
}

#[derive(Debug, Serialize)]
pub struct SessionReport {
    pub code_delta: Option<i64>,
    pub size_bytes_delta: Option<i64>,
    pub samples_loaded_from_store: usize,
}

#[derive(Debug, Serialize)]
pub struct HistoryReport {
    pub samples: usize,
    pub span_secs: Option<u64>,
    pub oldest_unix: Option<u64>,
    pub newest_unix: Option<u64>,
    pub store_dir: Option<String>,
}

impl Report {
    pub fn from_app(app: &App) -> Self {
        let path = app.path.display().to_string();
        let Some(s) = app.last.as_ref() else {
            return empty_report(path, app);
        };

        let collected_at_unix = system_to_unix(s.wall);
        let loc = s.loc.as_ref().map(|l| LocReport {
            code: l.code,
            files: l.files,
            blank: l.blank,
            comment: l.comment,
            languages: l
                .langs
                .iter()
                .map(|lang| LangReport {
                    name: lang.name.clone(),
                    code: lang.code,
                    blank: lang.blank,
                    comment: lang.comment,
                    pct: round2(fmt::pct(lang.code, l.code)),
                })
                .collect(),
        });

        let size = SizeReport {
            bytes: s.size_bytes,
            human: fmt::human_bytes_short(s.size_bytes),
            engine: s.size_engine.label().into(),
            top: s
                .top_dirs
                .iter()
                .map(|d| DirReport {
                    name: d.name.clone(),
                    bytes: d.bytes,
                    human: fmt::human_bytes_short(d.bytes),
                    pct: round2(fmt::pct(d.bytes, s.size_bytes)),
                })
                .collect(),
        };

        let git = s.git.as_ref().map(|g| GitReport {
            branch: g.branch.clone(),
            working_tree_insertions: g.ins,
            working_tree_deletions: g.del,
            recent_commits: g
                .commits
                .iter()
                .map(|c| CommitReport {
                    short: c.short.clone(),
                    subject: c.subject.clone(),
                    author: c.author.clone(),
                    insertions: c.ins,
                    deletions: c.del,
                })
                .collect(),
        });

        // One git log for the longest report window; bucket into each delta.
        let max_window_secs = report_windows().map(|(s, _)| s).max().unwrap_or(0);
        let churn =
            collect::git_commit_churn_since(&app.path, Duration::from_secs(max_window_secs));
        let now_unix = system_to_unix(SystemTime::now());

        let deltas: Vec<WindowDeltaReport> = report_windows()
            .map(|(secs, label)| {
                let window = Duration::from_secs(secs);
                let ready = app.window_ready(window);
                let (code, size_bytes) = if ready {
                    (app.window_code_delta(window), app.window_size_delta(window))
                } else {
                    (None, None)
                };
                let (insertions, deletions) = collect::sum_churn_in_window(&churn, secs, now_unix);
                WindowDeltaReport {
                    window: label.to_string(),
                    window_secs: secs,
                    ready,
                    code,
                    size_bytes,
                    insertions,
                    deletions,
                }
            })
            .collect();

        let (span_secs, oldest_unix, newest_unix) = history_meta(&app.history);
        let store_dir = app.store.as_ref().map(|st| st.root().display().to_string());

        let mut hints = Vec::new();
        hints.push(
            "Compare deltas[].code and deltas[].insertions/deletions for window \"2h\" \
             (cloc net LOC vs git commit churn)."
                .into(),
        );
        hints.push(
            "Re-run `heim --once --json .` or read <project>/.heim/stats.json after monitoring."
                .into(),
        );
        if let Some(d) = deltas.iter().find(|d| d.window == "2h") {
            if let Some(code) = d.code {
                if code > 500 {
                    hints.push(format!(
                        "code +{code} over 2h — review generated files for cleanup"
                    ));
                } else if code < 0 {
                    hints.push(format!("code {code} over 2h — net lines removed"));
                }
            } else if !d.ready {
                hints.push(
                    "2h window not ready yet — keep heim running or re-sample later for history"
                        .into(),
                );
            }
        }

        Report {
            schema: "heim.stats.v1",
            version: env!("CARGO_PKG_VERSION"),
            purpose: "AI / agent project metrics: LOC, size, git, and time-window deltas",
            path,
            collected_at: unix_to_rfc3339(collected_at_unix),
            collected_at_unix,
            took_secs: s.duration.as_secs_f64(),
            loc,
            loc_error: s.loc_err.clone(),
            size,
            git,
            git_error: s.git_err.clone(),
            deltas,
            session: SessionReport {
                code_delta: session_code_delta(app),
                size_bytes_delta: app.size_delta_session(),
                samples_loaded_from_store: app.loaded_from_store,
            },
            history: HistoryReport {
                samples: app.history.len(),
                span_secs,
                oldest_unix,
                newest_unix,
                store_dir,
            },
            hints,
        }
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn write_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create parent {}", parent.display()))?;
            }
        }
        let tmp = path.with_extension("json.tmp");
        {
            let mut f =
                fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
            f.write_all(self.to_json_pretty()?.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))?;
        Ok(())
    }
}

/// Collect one sample, merge `.heim` history, build report.
///
/// Also refreshes `<project>/.heim/stats.json` via [`App::apply_sample`].
pub fn collect_report(path: &Path, size_pref: SizeBackendKind) -> Result<Report> {
    let mut app = App::new(path.to_path_buf(), 60, size_pref);
    let sample = crate::collect::collect(path, size_pref);
    // Records into `.heim/samples.jsonl` and writes `stats.json`.
    app.apply_sample(sample);
    let report = Report::from_app(&app);
    if let Some(st) = app.store.as_mut() {
        st.end_session();
        std::mem::forget(app.store.take());
    }
    Ok(report)
}

pub fn store_stats_path(project: &Path) -> PathBuf {
    project.join(store::DIR_NAME).join(STATS_FILE)
}

pub fn write_store_stats(project: &Path, report: &Report) -> Result<()> {
    let dir = project.join(store::DIR_NAME);
    fs::create_dir_all(&dir)?;
    // Ensure privacy files exist.
    let _ = Store::open(project);
    report.write_to(&store_stats_path(project))
}

fn empty_report(path: String, app: &App) -> Report {
    Report {
        schema: "heim.stats.v1",
        version: env!("CARGO_PKG_VERSION"),
        purpose: "AI / agent project metrics: LOC, size, git, and time-window deltas",
        path,
        collected_at: unix_to_rfc3339(now_unix()),
        collected_at_unix: now_unix(),
        took_secs: 0.0,
        loc: None,
        loc_error: Some("no sample yet".into()),
        size: SizeReport {
            bytes: 0,
            human: "0B".into(),
            engine: "—".into(),
            top: vec![],
        },
        git: None,
        git_error: None,
        deltas: vec![],
        session: SessionReport {
            code_delta: None,
            size_bytes_delta: None,
            samples_loaded_from_store: app.loaded_from_store,
        },
        history: HistoryReport {
            samples: app.history.len(),
            span_secs: None,
            oldest_unix: None,
            newest_unix: None,
            store_dir: app.store.as_ref().map(|s| s.root().display().to_string()),
        },
        hints: vec!["No sample collected yet.".into()],
    }
}

fn report_windows() -> impl Iterator<Item = (u64, &'static str)> {
    CODE_DELTA_WINDOWS
        .iter()
        .copied()
        .chain(AGENT_EXTRA_WINDOWS.iter().copied())
}

fn session_code_delta(app: &App) -> Option<i64> {
    let a = app.last.as_ref()?.loc.as_ref()?.code as i64;
    let b = app.baseline.as_ref()?.loc.as_ref()?.code as i64;
    Some(a - b)
}

fn history_meta(history: &VecDeque<Sample>) -> (Option<u64>, Option<u64>, Option<u64>) {
    let Some(oldest) = history.front() else {
        return (None, None, None);
    };
    let Some(newest) = history.back() else {
        return (None, None, None);
    };
    let oldest_unix = system_to_unix(oldest.wall);
    let newest_unix = system_to_unix(newest.wall);
    (
        Some(newest_unix.saturating_sub(oldest_unix)),
        Some(oldest_unix),
        Some(newest_unix),
    )
}

fn system_to_unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_unix() -> u64 {
    system_to_unix(SystemTime::now())
}

fn unix_to_rfc3339(ts: u64) -> String {
    const DAY: u64 = 86_400;
    const DAYS_IN_MONTH: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut days = ts / DAY;
    let time = ts % DAY;
    let hour = time / 3600;
    let min = (time % 3600) / 60;
    let sec = time % 60;
    let mut year = 1970u64;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if days >= diy {
            days -= diy;
            year += 1;
        } else {
            break;
        }
    }
    let leap = is_leap(year);
    let mut month = 1u64;
    for (i, &dim) in DAYS_IN_MONTH.iter().enumerate() {
        let dim = if i == 1 && leap { 29 } else { dim };
        if days >= dim {
            days -= dim;
            month += 1;
        } else {
            break;
        }
    }
    let day = days + 1;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{LangStat, LocStats, SizeEngine};
    use std::time::Instant;

    #[test]
    fn report_serializes_and_has_schema() {
        let dir = std::env::temp_dir().join(format!("heim-report-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(dir.clone(), 10, SizeBackendKind::Walk);
        app.history.clear();
        let sample = Sample {
            at: Instant::now(),
            wall: SystemTime::now(),
            size_bytes: 4096,
            top_dirs: vec![],
            size_engine: SizeEngine::Walk,
            loc: Some(LocStats {
                files: 2,
                blank: 1,
                comment: 1,
                code: 100,
                langs: vec![LangStat {
                    name: "Rust".into(),
                    blank: 1,
                    comment: 1,
                    code: 100,
                }],
            }),
            loc_err: None,
            git: None,
            git_err: Some("not a git repository".into()),
            duration: Duration::from_millis(12),
        };
        app.apply_sample(sample);
        let r = Report::from_app(&app);
        assert_eq!(r.schema, "heim.stats.v1");
        assert_eq!(r.loc.as_ref().unwrap().code, 100);
        let json = r.to_json_pretty().unwrap();
        assert!(json.contains("\"schema\": \"heim.stats.v1\""));
        assert!(json.contains("deltas"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc3339_epoch() {
        assert_eq!(unix_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_rfc3339(1), "1970-01-01T00:00:01Z");
    }
}
