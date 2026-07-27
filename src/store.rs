//! Private per-project store under `{project}/.heim/`.
//!
//! Layout (never intended for VCS):
//! ```text
//! .heim/
//!   .gitignore      # ignore all store contents
//!   README          # short privacy note
//!   sessions.jsonl  # session start/end events
//!   samples.jsonl   # compact metric samples (append-only, rotated)
//!   stats.json      # latest full report, refreshed on every sample
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::collect::{LocStats, Sample, SizeEngine};

pub const DIR_NAME: &str = ".heim";
/// Retain roughly enough for 1d history windows + headroom.
pub const RETAIN: Duration = Duration::from_secs(48 * 3600);
const MAX_SAMPLE_LINES: usize = 50_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSample {
    /// Unix seconds (UTC).
    pub ts: u64,
    pub code: Option<u64>,
    pub files: Option<u64>,
    pub blank: Option<u64>,
    pub comment: Option<u64>,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ins: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub del: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    Start {
        id: String,
        ts: u64,
        pid: u32,
        interval_secs: u64,
        path: String,
    },
    End {
        id: String,
        ts: u64,
        samples: u64,
    },
}

/// Create `{project}/.heim/` and its privacy files without opening a session.
///
/// Callers that only need the directory to exist must use this instead of
/// `Store::open`: a `Store` owns a session lifecycle, and dropping a throwaway
/// one used to append a session `end` for a session that never started.
pub fn ensure_dir(project: &Path) -> Result<PathBuf> {
    let root = project.join(DIR_NAME);
    fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    ensure_privacy_files(&root)?;
    Ok(root)
}

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    pub session_id: String,
    samples_written: u64,
    /// `begin_session` wrote a `start` event.
    began: bool,
    /// `end_session` already wrote the matching `end` event.
    ended: bool,
}

impl Store {
    pub fn open(project: &Path) -> Result<Self> {
        let root = ensure_dir(project)?;
        let session_id = new_session_id();
        Ok(Self {
            root,
            session_id,
            samples_written: 0,
            began: false,
            ended: false,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn begin_session(&mut self, project: &Path, interval_secs: u64) -> Result<()> {
        let ev = SessionEvent::Start {
            id: self.session_id.clone(),
            ts: now_ts(),
            pid: std::process::id(),
            interval_secs,
            path: project.display().to_string(),
        };
        append_jsonl(&self.root.join("sessions.jsonl"), &ev)?;
        self.began = true;
        Ok(())
    }

    /// Write the session `end` event. Idempotent, and a no-op for a store whose
    /// session never began.
    ///
    /// Both guards are load-bearing: `Drop` also calls this, so without them an
    /// explicit `end_session()` followed by the drop wrote the event twice, and
    /// every `Store` opened just to create the directory wrote an orphan `end`.
    /// The live store had accumulated 177 `end` events against 81 `start`s.
    pub fn end_session(&mut self) {
        if !self.began || self.ended {
            return;
        }
        self.ended = true;
        let ev = SessionEvent::End {
            id: self.session_id.clone(),
            ts: now_ts(),
            samples: self.samples_written,
        };
        let _ = append_jsonl(&self.root.join("sessions.jsonl"), &ev);
    }

    pub fn record_sample(&mut self, s: &Sample) -> Result<()> {
        let row = StoredSample::from_sample(s);
        append_jsonl(&self.root.join("samples.jsonl"), &row)?;
        self.samples_written += 1;
        // cheap periodic rotate every 64 writes
        if self.samples_written.is_multiple_of(64) {
            let _ = self.compact_samples();
        }
        Ok(())
    }

    /// Load samples within retain window, oldest first.
    pub fn load_recent(&self) -> Result<Vec<StoredSample>> {
        let path = self.root.join("samples.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let cutoff = now_ts().saturating_sub(RETAIN.as_secs());
        let file = File::open(&path).with_context(|| format!("open {}", path.display()))?;
        let mut out = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(s) = serde_json::from_str::<StoredSample>(line) {
                if s.ts >= cutoff {
                    out.push(s);
                }
            }
        }
        out.sort_by_key(|s| s.ts);
        Ok(out)
    }

    fn compact_samples(&self) -> Result<()> {
        let path = self.root.join("samples.jsonl");
        if !path.exists() {
            return Ok(());
        }
        let mut rows = self.load_recent()?;
        if rows.len() > MAX_SAMPLE_LINES {
            rows = rows.split_off(rows.len() - MAX_SAMPLE_LINES);
        }
        let tmp = self.root.join("samples.jsonl.tmp");
        {
            // Buffered: `to_writer` on a bare File emits a syscall per JSON
            // token, so compacting the 50k-row cap cost millions of them.
            let mut f = std::io::BufWriter::new(File::create(&tmp)?);
            for r in &rows {
                serde_json::to_writer(&mut f, r)?;
                f.write_all(b"\n")?;
            }
            let f = f.into_inner().map_err(|e| e.into_error())?;
            f.sync_all()?;
        }
        fs::rename(tmp, path)?;
        Ok(())
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.end_session();
    }
}

impl StoredSample {
    pub fn from_sample(s: &Sample) -> Self {
        let (code, files, blank, comment) = s
            .loc
            .as_ref()
            .map(|l| (Some(l.code), Some(l.files), Some(l.blank), Some(l.comment)))
            .unwrap_or((None, None, None, None));
        let (branch, ins, del) = s
            .git
            .as_ref()
            .map(|g| (Some(g.branch.clone()), Some(g.ins), Some(g.del)))
            .unwrap_or((None, None, None));
        Self {
            ts: system_to_ts(s.wall),
            code,
            files,
            blank,
            comment,
            size: s.size_bytes,
            branch,
            ins,
            del,
            engine: Some(s.size_engine.label().into()),
        }
    }

    /// Rebuild a lightweight Sample for history windows (no top_dirs / lang breakdown).
    pub fn to_sample(&self) -> Sample {
        let wall = ts_to_system(self.ts);
        let at = wall_to_instant(wall);
        let loc = self.code.map(|code| LocStats {
            files: self.files.unwrap_or(0),
            blank: self.blank.unwrap_or(0),
            comment: self.comment.unwrap_or(0),
            code,
            langs: vec![],
        });
        Sample {
            at,
            wall,
            size_bytes: self.size,
            top_dirs: vec![],
            size_engine: match self.engine.as_deref() {
                Some("dust") => SizeEngine::Dust,
                _ => SizeEngine::Walk,
            },
            loc,
            loc_err: None,
            git: None,
            git_err: None,
            duration: Duration::ZERO,
        }
    }
}

fn ensure_privacy_files(root: &Path) -> Result<()> {
    let gi = root.join(".gitignore");
    if !gi.exists() {
        fs::write(
            &gi,
            "# heim private store — do not commit\n*\n!.gitignore\n!README\n",
        )?;
    }
    let readme = root.join("README");
    if !readme.exists() {
        fs::write(
            &readme,
            "heim private local store\n\n\
             sessions.jsonl — session start/end events\n\
             samples.jsonl  — metric samples for cross-session history\n\
             stats.json     — latest full report (for AI agents / scripts)\n\n\
             This directory is local-only. Do not commit it.\n",
        )?;
    }
    Ok(())
}

/// Append one JSON record plus newline as a **single** `write_all`.
///
/// `O_APPEND` makes an individual write atomic, not a token stream. Handing the
/// raw `File` to `serde_json::to_writer` emitted ~66 syscalls per record, so two
/// heim processes on one project interleaved mid-record and produced lines like
/// `{"{ts""ts:"1784809746:,1784809746"…`. `load_recent` silently drops
/// unparseable lines, so the history just quietly lost samples: a store driven by
/// concurrent writers measured **15% of records corrupt**.
///
/// Concurrent writers are a documented workflow — the README tells agents to run
/// `heim --once --json .` while the TUI is live.
fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("append {}", path.display()))?;
    let mut buf = serde_json::to_vec(value)?;
    buf.push(b'\n');
    f.write_all(&buf)?;
    Ok(())
}

fn now_ts() -> u64 {
    system_to_ts(SystemTime::now())
}

fn system_to_ts(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn ts_to_system(ts: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(ts)
}

/// Map a wall time to Instant relative to now (clamped — never before process Instant origin).
fn wall_to_instant(wall: SystemTime) -> Instant {
    let now_i = Instant::now();
    let now_w = SystemTime::now();
    match now_w.duration_since(wall) {
        Ok(ago) => now_i.checked_sub(ago).unwrap_or(now_i),
        Err(e) => now_i + e.duration(),
    }
}

fn new_session_id() -> String {
    // compact sortable id: unix_secs-pid
    format!("{}-{}", now_ts(), std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{DirSize, SizeEngine};
    use std::time::Instant;

    fn count_events(dir: &Path) -> (usize, usize) {
        let p = dir.join(DIR_NAME).join("sessions.jsonl");
        let Ok(raw) = fs::read_to_string(&p) else {
            return (0, 0);
        };
        let mut start = 0;
        let mut end = 0;
        for line in raw.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<SessionEvent>(line) {
                Ok(SessionEvent::Start { .. }) => start += 1,
                Ok(SessionEvent::End { .. }) => end += 1,
                Err(_) => {}
            }
        }
        (start, end)
    }

    /// Every `start` gets exactly one `end`, no matter how the store is closed.
    ///
    /// Regression: `end_session()` was not idempotent and `Drop` called it too,
    /// and `write_store_stats` opened a throwaway `Store` on every sample whose
    /// drop appended an orphan `end`. The real store had 177 ends to 81 starts.
    #[test]
    fn session_events_are_balanced() {
        let dir = std::env::temp_dir().join(format!("heim-sessions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // A store that begins, is ended explicitly, then also dropped.
        {
            let mut st = Store::open(&dir).unwrap();
            st.begin_session(&dir, 10).unwrap();
            st.end_session();
            st.end_session(); // idempotent
        } // Drop calls end_session again
        assert_eq!(
            count_events(&dir),
            (1, 1),
            "explicit end + drop double-wrote"
        );

        // Directory-only preparation must not touch the session log at all.
        for _ in 0..5 {
            ensure_dir(&dir).unwrap();
        }
        assert_eq!(
            count_events(&dir),
            (1, 1),
            "ensure_dir wrote session events"
        );

        // A store that never begins must not write an `end` when dropped.
        {
            let _st = Store::open(&dir).unwrap();
        }
        assert_eq!(
            count_events(&dir),
            (1, 1),
            "unbegun store wrote an orphan end"
        );

        // A second real session appends exactly one balanced pair.
        {
            let mut st = Store::open(&dir).unwrap();
            st.begin_session(&dir, 10).unwrap();
        } // ended by Drop only
        assert_eq!(count_events(&dir), (2, 2));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrip_store() {
        let dir = std::env::temp_dir().join(format!("heim-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut store = Store::open(&dir).unwrap();
        store.begin_session(&dir, 10).unwrap();
        let s = Sample {
            at: Instant::now(),
            wall: SystemTime::now(),
            size_bytes: 1234,
            top_dirs: vec![DirSize {
                name: "src".into(),
                bytes: 100,
            }],
            size_engine: SizeEngine::Walk,
            loc: Some(LocStats {
                files: 2,
                blank: 1,
                comment: 0,
                code: 50,
                langs: vec![],
            }),
            loc_err: None,
            git: None,
            git_err: None,
            duration: Duration::from_millis(5),
        };
        store.record_sample(&s).unwrap();
        let loaded = store.load_recent().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].code, Some(50));
        assert_eq!(loaded[0].size, 1234);
        assert!(dir.join(DIR_NAME).join("samples.jsonl").exists());
        assert!(dir.join(DIR_NAME).join(".gitignore").exists());
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }
}
