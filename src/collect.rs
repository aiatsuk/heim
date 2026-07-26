//! Background collectors: disk size (dust|walk), cloc, git.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use walkdir::WalkDir;

use crate::fmt;

/// Directories skipped for size walk / dust and passed to cloc --exclude-dir.
/// Includes common JS/Rust/Flutter/iOS/Android build dumps so monorepos stay responsive.
pub const IGNORE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "vendor",
    ".tox",
    "coverage",
    ".cache",
    ".idea",
    ".vscode",
    "Pods",
    "DerivedData",
    ".turbo",
    ".gradle",
    "xcuserdata",
    ".heim",
    ".dart_tool",
    ".pub-cache",
    "out",
    ".firebase",
    ".dart-index",
    "xcshareddata",
    ".gradle",
    "ephemeral",
    // Heavy monorepo dumps (Swift/.build, AI agent caches, packaging)
    "artifacts",
    ".build",
    ".claude",
    ".swiftpm",
    "Carthage",
    "checkouts",
];

/// Soft timeouts so a 9GB monorepo cannot freeze the TUI forever.
const CLOC_TIMEOUT: Duration = Duration::from_secs(45);
const DUST_TIMEOUT: Duration = Duration::from_secs(20);
const GIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Commit log size: subject/hash only (no per-file numstat — that deadlocks + is huge).
const GIT_LOG_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeBackendKind {
    Auto,
    Dust,
    Walk,
}

impl SizeBackendKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "dust" => Some(Self::Dust),
            "walk" | "walkdir" => Some(Self::Walk),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeEngine {
    Dust,
    Walk,
}

impl SizeEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dust => "dust",
            Self::Walk => "walk",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LangStat {
    pub name: String,
    pub blank: u64,
    pub comment: u64,
    pub code: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LocStats {
    pub files: u64,
    pub blank: u64,
    pub comment: u64,
    pub code: u64,
    pub langs: Vec<LangStat>,
}

#[derive(Debug, Clone, Default)]
pub struct GitCommit {
    pub short: String,
    pub subject: String,
    pub author: String,
    pub ins: u64,
    pub del: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GitStats {
    pub branch: String,
    /// Working tree insertions (unstaged + staged).
    pub ins: u64,
    /// Working tree deletions (unstaged + staged).
    pub del: u64,
    /// Recent commits (newest first), compact.
    pub commits: Vec<GitCommit>,
}

#[derive(Debug, Clone)]
pub struct DirSize {
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub at: Instant,
    /// Wall clock for cross-session history / `.heim` store.
    pub wall: std::time::SystemTime,
    pub size_bytes: u64,
    pub top_dirs: Vec<DirSize>,
    pub size_engine: SizeEngine,
    pub loc: Option<LocStats>,
    pub loc_err: Option<String>,
    pub git: Option<GitStats>,
    pub git_err: Option<String>,
    pub duration: Duration,
}

fn dust_available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        Command::new("dust")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

pub fn resolve_engine(pref: SizeBackendKind) -> SizeEngine {
    match pref {
        SizeBackendKind::Dust => SizeEngine::Dust,
        SizeBackendKind::Walk => SizeEngine::Walk,
        SizeBackendKind::Auto => {
            if dust_available() {
                SizeEngine::Dust
            } else {
                SizeEngine::Walk
            }
        }
    }
}

pub fn collect(path: &Path, size_pref: SizeBackendKind) -> Sample {
    let t0 = Instant::now();
    let engine = resolve_engine(size_pref);
    let path_size = path.to_path_buf();
    let path_cloc = path.to_path_buf();
    let path_git = path.to_path_buf();

    // Parallel collectors — large monorepos must not serialize cloc+dust+git.
    let h_size = thread::spawn(move || measure_size(&path_size, engine));
    let h_cloc = thread::spawn(move || run_cloc(&path_cloc));
    let h_git = thread::spawn(move || run_git(&path_git));

    let (size_bytes, top_dirs, size_engine) = h_size.join().unwrap_or_else(|_| {
        let (b, t) = measure_size_walk(path, Some(20));
        (b, t, SizeEngine::Walk)
    });
    let (loc, loc_err) = match h_cloc
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("cloc worker panicked")))
    {
        Ok(l) => (Some(l), None),
        Err(e) => (None, Some(short_err(&e.to_string()))),
    };
    let (git, git_err) = match h_git
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("git worker panicked")))
    {
        Ok(Some(g)) => (Some(g), None),
        Ok(None) => (None, Some("not a git repository".into())),
        Err(e) => (None, Some(short_err(&e.to_string()))),
    };
    Sample {
        at: Instant::now(),
        wall: std::time::SystemTime::now(),
        size_bytes,
        top_dirs,
        size_engine,
        loc,
        loc_err,
        git,
        git_err,
        duration: t0.elapsed(),
    }
}

/// Run a command with a hard timeout.
///
/// **Critical:** stdout/stderr are drained on background threads while waiting.
/// Reading only after `try_wait` succeeds deadlocks when output exceeds the
/// OS pipe buffer (~64KiB) — which is exactly what `git log --numstat` does.
fn run_timed(mut cmd: Command, timeout: Duration) -> Result<std::process::Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("spawn")?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_h = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = stdout_pipe {
            let _ = r.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_h = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = stderr_pipe {
            let _ = r.read_to_end(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                // Join readers so threads don't leak; discard partial output.
                let _ = stdout_h.join();
                let _ = stderr_h.join();
                bail!("timed out after {}s", timeout.as_secs());
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = stdout_h.join();
                let _ = stderr_h.join();
                return Err(e.into());
            }
        }
    };

    let stdout = stdout_h.join().unwrap_or_default();
    let stderr = stderr_h.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn short_err(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(79).collect::<String>())
    } else {
        s.to_string()
    }
}

fn is_ignored(name: &str) -> bool {
    IGNORE_DIRS.contains(&name)
}

/// Fast children listing for drill-down — one-pass walk (never dust).
pub fn list_children_fast(root: &Path) -> (u64, Vec<DirSize>) {
    measure_size_walk(root, None)
}

/// Prefer dust when requested/available; fall back to walk on error.
pub fn measure_size(root: &Path, engine: SizeEngine) -> (u64, Vec<DirSize>, SizeEngine) {
    match engine {
        SizeEngine::Dust => match measure_size_dust(root) {
            Ok(v) => (v.0, v.1, SizeEngine::Dust),
            Err(_) => {
                let (b, t) = measure_size_walk(root, Some(20));
                (b, t, SizeEngine::Walk)
            }
        },
        SizeEngine::Walk => {
            let (b, t) = measure_size_walk(root, Some(20));
            (b, t, SizeEngine::Walk)
        }
    }
}

/// Single WalkDir pass: attribute every file to its first path component under `root`.
/// O(files) once — not O(children × files). Critical for monorepo drill-down speed.
pub fn measure_size_walk(root: &Path, limit: Option<usize>) -> (u64, Vec<DirSize>) {
    use std::collections::HashMap;

    let mut total = 0u64;
    let mut tops: HashMap<String, u64> = HashMap::new();

    // Ensure zero-byte top-level entries still appear.
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_ignored(&name) || name.starts_with('.') {
                continue;
            }
            tops.entry(name).or_insert(0);
        }
    }

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            e.file_name()
                .to_str()
                .map(|n| !is_ignored(n) && !n.starts_with('.'))
                .unwrap_or(true)
        });

    for ent in walker.flatten() {
        if ent.depth() == 0 {
            continue;
        }
        let Ok(meta) = ent.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let len = meta.len();
        total = total.saturating_add(len);
        if let Ok(rel) = ent.path().strip_prefix(root) {
            if let Some(comp) = rel.components().next() {
                let key = comp.as_os_str().to_string_lossy().into_owned();
                if !is_ignored(&key) && !key.starts_with('.') {
                    *tops.entry(key).or_default() += len;
                }
            }
        }
    }

    let mut top_dirs: Vec<DirSize> = tops
        .into_iter()
        .map(|(name, bytes)| DirSize { name, bytes })
        .collect();
    top_dirs.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    if let Some(n) = limit {
        top_dirs.truncate(n);
    }
    (total, top_dirs)
}

/// dust -d1 -b -c -s -i with shared ignores. Parse human sizes + names.
pub fn measure_size_dust(root: &Path) -> Result<(u64, Vec<DirSize>)> {
    let mut cmd = Command::new("dust");
    cmd.args([
        "-d", "1", "-n", "20", "-b", // no percent bars
        "-c", // no colors
        "-s", // apparent size
        "-i", // ignore hidden
    ]);
    for d in IGNORE_DIRS {
        cmd.arg("-X").arg(d);
    }
    cmd.arg(root);
    let out = run_timed(cmd, DUST_TIMEOUT).context("dust")?;
    if !out.status.success() {
        bail!(
            "dust failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let raw = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    parse_dust_output(&raw, root)
}

pub fn parse_dust_output(raw: &str, root: &Path) -> Result<(u64, Vec<DirSize>)> {
    let root_name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());

    let mut total = 0u64;
    let mut tops = Vec::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((bytes, name)) = parse_dust_line(line) else {
            continue;
        };
        let base = Path::new(&name)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(name);
        // total row: contains box "┴" or matches root name and is largest-ish
        let is_total = line.contains('┴') || base == root_name;
        if is_total {
            total = total.max(bytes);
            continue;
        }
        if is_ignored(&base) || base.starts_with('.') {
            continue;
        }
        tops.push(DirSize { name: base, bytes });
    }

    if total == 0 {
        total = tops.iter().map(|d| d.bytes).sum();
    }
    tops.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    // dedupe names keep first (largest)
    let mut seen = std::collections::HashSet::new();
    tops.retain(|d| seen.insert(d.name.clone()));
    tops.truncate(12);
    if total == 0 && tops.is_empty() {
        bail!("dust produced no parseable rows");
    }
    Ok((total, tops))
}

fn parse_dust_line(line: &str) -> Option<(u64, String)> {
    // " 538B   ┌── Cargo.toml" or "76K ┌─┴ heim"
    let line = line.trim();
    let mut parts = line.split_whitespace();
    let size_tok = parts.next()?;
    let bytes = fmt::parse_dust_size(size_tok)?;
    // remaining: tree glyphs + name
    let rest = parts.collect::<Vec<_>>().join(" ");
    if rest.is_empty() {
        return None;
    }
    // strip common dust tree prefixes
    let name = rest
        .trim_start_matches(|c: char| {
            matches!(c, '┌' | '├' | '└' | '─' | '┴' | '│' | '┬' | '┤' | '┼' | ' ')
                || c == '|'
                || c == '`'
                || c == '+'
                || c == '-'
        })
        .trim()
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some((bytes, name))
    }
}

#[derive(Debug, Deserialize)]
struct ClocJson {
    #[serde(rename = "SUM")]
    sum: Option<ClocSum>,
    #[serde(flatten)]
    rest: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ClocSum {
    blank: u64,
    comment: u64,
    code: u64,
    #[serde(rename = "nFiles")]
    n_files: u64,
}

fn run_cloc(path: &Path) -> Result<LocStats> {
    let exclude = IGNORE_DIRS.join(",");
    let mut cmd = Command::new("cloc");
    cmd.args(["--json", "--quiet", "--exclude-dir", &exclude])
        .arg(path);
    let out = run_timed(cmd, CLOC_TIMEOUT).context("cloc (is it installed?)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("cloc failed: {}", err.trim());
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let json = raw.find('{').map(|i| &raw[i..]).unwrap_or(&raw);
    let parsed: ClocJson = serde_json::from_str(json).context("parse cloc json")?;
    let sum = parsed.sum.context("cloc SUM missing")?;
    let mut langs = Vec::new();
    for (k, v) in parsed.rest {
        if k == "header" || k == "SUM" {
            continue;
        }
        let code = v.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
        let files = v
            .get("nFiles")
            .or_else(|| v.get("files"))
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let blank = v.get("blank").and_then(|c| c.as_u64()).unwrap_or(0);
        let comment = v.get("comment").and_then(|c| c.as_u64()).unwrap_or(0);
        if code > 0 || files > 0 {
            langs.push(LangStat {
                name: k,
                blank,
                comment,
                code,
            });
        }
    }
    langs.sort_by(|a, b| b.code.cmp(&a.code));
    langs.truncate(16);
    Ok(LocStats {
        files: sum.n_files,
        blank: sum.blank,
        comment: sum.comment,
        code: sum.code,
        langs,
    })
}

fn git_ok(path: &Path) -> bool {
    Command::new("git")
        .args(["-C"])
        .arg(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

fn git_out(path: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(path).args(args);
    let out = run_timed(cmd, GIT_TIMEOUT).context("git")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn numstat_sum(text: &str) -> (u64, u64, u64) {
    let mut ins = 0u64;
    let mut del = 0u64;
    let mut files = 0u64;
    for line in text.lines() {
        let mut p = line.split_whitespace();
        let a = p.next().unwrap_or("0");
        let b = p.next().unwrap_or("0");
        if a == "-" || b == "-" {
            files += 1;
            continue;
        }
        ins += a.parse().unwrap_or(0);
        del += b.parse().unwrap_or(0);
        files += 1;
    }
    (ins, del, files)
}

fn run_git(path: &Path) -> Result<Option<GitStats>> {
    if !git_ok(path) {
        return Ok(None);
    }
    let branch = git_out(path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|_| "HEAD".into())
        .trim()
        .to_string();

    // Working-tree stats: numstat is fine (usually << pipe buffer). On huge
    // dirty trees, shortstat is a fallback if numstat fails/times out.
    let (u_ins, u_del) = git_diff_stat(path, false);
    let (s_ins, s_del) = git_diff_stat(path, true);

    let commits = recent_commits(path, GIT_LOG_LIMIT);

    Ok(Some(GitStats {
        branch,
        ins: u_ins + s_ins,
        del: u_del + s_del,
        commits,
    }))
}

/// Unstaged (`cached=false`) or staged (`cached=true`) insert/delete counts.
fn git_diff_stat(path: &Path, cached: bool) -> (u64, u64) {
    let mut args: Vec<&str> = vec!["diff", "--numstat"];
    if cached {
        args = vec!["diff", "--cached", "--numstat"];
    }
    if let Ok(raw) = git_out(path, &args) {
        let (ins, del, _) = numstat_sum(&raw);
        return (ins, del);
    }
    // Fallback: one-line shortstat (tiny stdout).
    let args: &[&str] = if cached {
        &["diff", "--cached", "--shortstat"]
    } else {
        &["diff", "--shortstat"]
    };
    let raw = git_out(path, args).unwrap_or_default();
    parse_shortstat(&raw)
}

fn parse_shortstat(text: &str) -> (u64, u64) {
    // " 12 files changed, 34 insertions(+), 5 deletions(-)"
    let mut ins = 0u64;
    let mut del = 0u64;
    for part in text.split(',') {
        let p = part.trim();
        if let Some(n) = p
            .strip_suffix(" insertions(+)")
            .or_else(|| p.strip_suffix(" insertion(+)"))
        {
            ins = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = p
            .strip_suffix(" deletions(-)")
            .or_else(|| p.strip_suffix(" deletion(-)"))
        {
            del = n.trim().parse().unwrap_or(0);
        }
    }
    (ins, del)
}

/// Last N commits: hash / subject / author + shortstat (+/−).
/// Avoids per-file `--numstat` (hundreds of KB → pipe deadlock + slow).
fn recent_commits(path: &Path, n: usize) -> Vec<GitCommit> {
    // Record: RS short US subject US author, then optional shortstat line(s).
    let pretty = "--pretty=format:%x1e%h%x1f%s%x1f%an";
    let raw = match git_out(path, &["log", &format!("-{n}"), pretty, "--shortstat"]) {
        Ok(s) => s,
        Err(_) => {
            // Absolute fallback: subjects only, no +/-.
            git_out(path, &["log", &format!("-{n}"), pretty]).unwrap_or_default()
        }
    };

    let mut out = Vec::new();
    let mut cur: Option<GitCommit> = None;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix('\u{1e}') {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            let mut p = rest.split('\u{1f}');
            let short = p.next().unwrap_or("").to_string();
            let subject = p.next().unwrap_or("").to_string();
            let author = p.next().unwrap_or("").to_string();
            cur = Some(GitCommit {
                short,
                subject,
                author,
                ins: 0,
                del: 0,
            });
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        // shortstat line attached to current commit
        if let Some(c) = cur.as_mut() {
            if line.contains("insertion") || line.contains("deletion") || line.contains("changed") {
                let (ins, del) = parse_shortstat(line);
                c.ins = ins;
                c.del = del;
            }
        }
    }
    if let Some(c) = cur.take() {
        out.push(c);
    }
    out
}

pub fn path_label(p: &Path) -> String {
    p.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(p))
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn size_skips_ignored() {
        let dir = tempfile_dir();
        fs::write(dir.join("a.rs"), "fn main(){}").unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("node_modules/x.js"), "x".repeat(10_000)).unwrap();
        let (bytes, tops) = measure_size_walk(&dir, None);
        assert!(bytes < 1000, "ignored node_modules weight: {bytes}");
        assert!(tops.iter().any(|d| d.name == "a.rs"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_dust_fixture() {
        let raw = "\
 538B   ┌── Cargo.toml
1.3K   ├── README.md
 16K   ├── meetings
 21K   ├── Cargo.lock
 36K   ├── src
 76K ┌─┴ heim
";
        let (total, tops) = parse_dust_output(raw, Path::new("/tmp/heim")).unwrap();
        assert_eq!(total, 76 * 1024);
        assert_eq!(tops[0].name, "src");
        assert!(tops.iter().any(|d| d.name == "meetings"));
    }

    fn tempfile_dir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("heim-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn numstat() {
        let t = "10\t2\tfoo.rs\n3\t0\tbar.rs\n";
        assert_eq!(numstat_sum(t), (13, 2, 2));
    }

    #[test]
    fn shortstat_parse() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 20 insertions(+), 5 deletions(-)"),
            (20, 5)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 1 insertion(+)"), (1, 0));
        assert_eq!(parse_shortstat(" 2 files changed, 4 deletions(-)"), (0, 4));
    }

    #[test]
    fn parse_log_shortstat_stream() {
        // Same shape as `git log --pretty=format:%x1e… --shortstat`
        let raw = "\u{1e}abc1234\u{1f}feat: hello\u{1f}ann\
\n 2 files changed, 10 insertions(+), 3 deletions(-)\n\
\u{1e}def5678\u{1f}fix: world\u{1f}bob\n\
 1 file changed, 1 deletion(-)\n";
        let mut out = Vec::new();
        let mut cur: Option<GitCommit> = None;
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix('\u{1e}') {
                if let Some(c) = cur.take() {
                    out.push(c);
                }
                let mut p = rest.split('\u{1f}');
                cur = Some(GitCommit {
                    short: p.next().unwrap_or("").into(),
                    subject: p.next().unwrap_or("").into(),
                    author: p.next().unwrap_or("").into(),
                    ins: 0,
                    del: 0,
                });
                continue;
            }
            if let Some(c) = cur.as_mut() {
                if line.contains("changed") {
                    let (ins, del) = parse_shortstat(line);
                    c.ins = ins;
                    c.del = del;
                }
            }
        }
        if let Some(c) = cur.take() {
            out.push(c);
        }
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].short, "abc1234");
        assert_eq!(out[0].ins, 10);
        assert_eq!(out[0].del, 3);
        assert_eq!(out[1].del, 1);
    }
}
