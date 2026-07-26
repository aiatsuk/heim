//! Private per-project store under `{project}/.heim/`.
//!
//! Layout (never intended for VCS):
//! ```text
//! .heim/
//!   .gitignore      # ignore all store contents
//!   README          # short privacy note
//!   sessions.jsonl  # session start/end events
//!   samples.jsonl   # compact metric samples (append-only, rotated)
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    pub session_id: String,
    samples_written: u64,
}

impl Store {
    pub fn open(project: &Path) -> Result<Self> {
        let root = project.join(DIR_NAME);
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        ensure_privacy_files(&root)?;
        let session_id = new_session_id();
        Ok(Self {
            root,
            session_id,
            samples_written: 0,
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
        Ok(())
    }

    pub fn end_session(&mut self) {
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
        if self.samples_written % 64 == 0 {
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
            let mut f = File::create(&tmp)?;
            for r in &rows {
                serde_json::to_writer(&mut f, r)?;
                f.write_all(b"\n")?;
            }
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

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("append {}", path.display()))?;
    serde_json::to_writer(&mut f, value)?;
    f.write_all(b"\n")?;
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
    use std::time::Instant;
    let now_i = Instant::now();
    let now_w = SystemTime::now();
    match now_w.duration_since(wall) {
        Ok(ago) => now_i.checked_sub(ago).unwrap_or(now_i),
        Err(e) => now_i + e.duration(),
    }
}

// Instant used only in wall_to_instant
use std::time::Instant;

fn new_session_id() -> String {
    // compact sortable id: unix_secs-pid
    format!("{}-{}", now_ts(), std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{DirSize, SizeEngine};
    use std::time::Instant;

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
