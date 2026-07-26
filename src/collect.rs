//! Background collectors: disk size (dust|walk), LOC, git.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use ignore::{WalkBuilder, WalkState};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::fmt;

/// Directories skipped for size walk / dust and passed to tokei as excludes.
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
const DUST_TIMEOUT: Duration = Duration::from_secs(20);
const GIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Availability probes (`dust --version`, `git rev-parse`) — must answer fast.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
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

/// `auto` resolves to the in-process walk.
///
/// Measured on a ~4k-file / 80MB Rust tree: the parallel walk takes ~17ms, the
/// `dust` subprocess ~920ms — 53x slower, because heim pays a process spawn, a
/// pipe drain and ~34 `-X` arguments to get a number it can compute itself.
/// `dust` stays available via `--size-backend dust`; it does hardlink dedup the
/// walk does not, so the two report slightly different totals.
pub fn resolve_engine(pref: SizeBackendKind) -> SizeEngine {
    match pref {
        SizeBackendKind::Dust => SizeEngine::Dust,
        SizeBackendKind::Walk | SizeBackendKind::Auto => SizeEngine::Walk,
    }
}

pub fn collect(path: &Path, size_pref: SizeBackendKind) -> Sample {
    let t0 = Instant::now();
    let engine = resolve_engine(size_pref);
    let path_size = path.to_path_buf();
    let path_loc = path.to_path_buf();
    let path_git = path.to_path_buf();

    // Parallel collectors — sample latency is the slowest one, not their sum.
    let h_size = thread::spawn(move || timed("size", || measure_size(&path_size, engine)));
    let h_loc = thread::spawn(move || timed("loc", || run_loc(&path_loc)));
    let h_git = thread::spawn(move || timed("git", || run_git(&path_git)));

    let (size_bytes, top_dirs, size_engine) = h_size.join().unwrap_or_else(|_| {
        let (b, t) = measure_size_walk(path, Some(20));
        (b, t, SizeEngine::Walk)
    });
    let (loc, loc_err) = match h_loc
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("loc worker panicked")))
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
    let tspawn = Instant::now();
    let mut child = cmd.spawn().context("spawn")?;
    let spawn_took = tspawn.elapsed();

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
    // Adaptive backoff: a flat 20ms poll added up to 20ms of pure sleep to every
    // command, and the git collector runs five of them. Start tight for the
    // common fast case, then relax so a slow command does not spin.
    let mut poll = Duration::from_millis(1);
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
            Ok(None) => {
                thread::sleep(poll);
                poll = (poll * 2).min(Duration::from_millis(20));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = stdout_h.join();
                let _ = stderr_h.join();
                return Err(e.into());
            }
        }
    };

    let waited = start.elapsed();
    let stdout = stdout_h.join().unwrap_or_default();
    let stderr = stderr_h.join().unwrap_or_default();
    if std::env::var_os("HEIM_TRACE").is_some() {
        eprintln!(
            "heim[trace]   cmd {:?}: spawn={spawn_took:.2?} wait={waited:.2?} drain={:.2?}",
            cmd.get_args().take(4).collect::<Vec<_>>(),
            start.elapsed() - waited
        );
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Per-collector timing, printed when `HEIM_TRACE=1`.
///
/// Sample latency is the slowest collector, not their sum, so a regression in
/// one is invisible in the total until it becomes the max. Keep this cheap and
/// always available — guessing which collector dominates is how heim ended up
/// shipping a 45s `cloc` timeout nobody had measured.
fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    if std::env::var_os("HEIM_TRACE").is_none() {
        return f();
    }
    let t0 = Instant::now();
    let out = f();
    eprintln!("heim[trace] {label:>5}: {:>8.2?}", t0.elapsed());
    out
}

fn short_err(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(79).collect::<String>())
    } else {
        s.to_string()
    }
}

/// `IGNORE_DIRS` as a hash set.
///
/// The linear `[&str]::contains` this replaces ran once per directory entry and
/// again per file, so a 100k-entry tree spent millions of string comparisons
/// just deciding what to skip.
fn ignore_set() -> &'static FxHashSet<&'static str> {
    static SET: OnceLock<FxHashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| IGNORE_DIRS.iter().copied().collect())
}

fn is_ignored(name: &str) -> bool {
    ignore_set().contains(name)
}

/// Skip rule shared by the size walk and its `filter_entry` prune.
fn skip_name(name: &str) -> bool {
    is_ignored(name) || name.starts_with('.')
}

/// Worker count for the parallel walk. Capped — this is syscall-bound work and
/// returns flatten out well before core count on any real tree.
fn walk_threads() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 12)
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

/// Per-thread size accumulator.
///
/// Each walker thread owns one and merges it into the shared result exactly
/// once, on drop, when its visitor is torn down. Nothing is shared or locked on
/// the per-file hot path — the pattern `ignore` documents for `WalkParallel`.
struct SizeAcc {
    total: u64,
    tops: FxHashMap<String, u64>,
    sink: mpsc::Sender<(u64, FxHashMap<String, u64>)>,
}

impl Drop for SizeAcc {
    fn drop(&mut self) {
        let _ = self.sink.send((self.total, std::mem::take(&mut self.tops)));
    }
}

/// One parallel pass: attribute every file to its first path component under `root`.
/// O(files) once — not O(children × files). Critical for monorepo drill-down speed.
///
/// Uses `ignore` purely as a work-stealing walker: `standard_filters(false)` keeps
/// heim's own [`IGNORE_DIRS`] + dotfile rules authoritative and does not consult
/// `.gitignore`, so reported sizes stay comparable to previous releases.
pub fn measure_size_walk(root: &Path, limit: Option<usize>) -> (u64, Vec<DirSize>) {
    let mut tops: FxHashMap<String, u64> = FxHashMap::default();

    // Ensure zero-byte top-level entries still appear.
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if skip_name(&name) {
                continue;
            }
            tops.entry(name).or_insert(0);
        }
    }

    let (tx, rx) = mpsc::channel::<(u64, FxHashMap<String, u64>)>();
    let root_owned = root.to_path_buf();

    WalkBuilder::new(root)
        .standard_filters(false)
        .follow_links(false)
        .threads(walk_threads())
        .filter_entry(|e| {
            e.depth() == 0
                || e.file_name()
                    .to_str()
                    .map(|n| !skip_name(n))
                    .unwrap_or(true)
        })
        .build_parallel()
        .run(|| {
            let mut acc = SizeAcc {
                total: 0,
                tops: FxHashMap::default(),
                sink: tx.clone(),
            };
            let root = root_owned.clone();
            Box::new(move |res| {
                let Ok(ent) = res else {
                    return WalkState::Continue;
                };
                if ent.depth() == 0 {
                    return WalkState::Continue;
                }
                // `file_type()` comes from readdir — cheaper than stat'ing dirs.
                if !ent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    return WalkState::Continue;
                }
                let Ok(meta) = ent.metadata() else {
                    return WalkState::Continue;
                };
                let len = meta.len();
                acc.total = acc.total.saturating_add(len);
                if let Ok(rel) = ent.path().strip_prefix(&root) {
                    if let Some(comp) = rel.components().next() {
                        let key = comp.as_os_str().to_string_lossy();
                        if !skip_name(&key) {
                            // Only allocate the key the first time we see it.
                            match acc.tops.get_mut(key.as_ref()) {
                                Some(v) => *v = v.saturating_add(len),
                                None => {
                                    acc.tops.insert(key.into_owned(), len);
                                }
                            }
                        }
                    }
                }
                WalkState::Continue
            })
        });
    // Drop our handle so `rx` terminates once every worker has merged.
    drop(tx);

    let mut total = 0u64;
    for (part_total, part) in rx {
        total = total.saturating_add(part_total);
        for (k, v) in part {
            let e = tops.entry(k).or_default();
            *e = e.saturating_add(v);
        }
    }

    let mut top_dirs: Vec<DirSize> = tops
        .into_iter()
        .map(|(name, bytes)| DirSize { name, bytes })
        .collect();
    top_dirs.sort_by_key(|b| std::cmp::Reverse(b.bytes));
    if let Some(n) = limit {
        top_dirs.truncate(n);
    }
    (total, top_dirs)
}

/// Escape a literal directory name for use inside a regex alternation.
fn regex_escape(s: &str) -> String {
    const META: &[char] = &[
        '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$', '\\', '/', '-',
    ];
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if META.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// [`IGNORE_DIRS`] as a path regex for dust's `--invert-filter`.
///
/// dust's `-X/--ignore-directory` takes a full **path**, not a directory name,
/// so the previous `-X build -X node_modules …` invocation excluded nothing that
/// wasn't a top-level match: on a Flutter monorepo it reported **7.0G** where the
/// real non-build total was **11M**. `-v/--invert-filter` matches a regex against
/// whole file paths, which is what heim actually wants. The surrounding slashes
/// keep it to whole path components, so a *file* named `build` still counts.
fn dust_ignore_regex() -> &'static str {
    static RE: OnceLock<String> = OnceLock::new();
    RE.get_or_init(|| {
        let alts: Vec<String> = IGNORE_DIRS.iter().map(|d| regex_escape(d)).collect();
        format!("/({})/", alts.join("|"))
    })
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
    cmd.arg("-v").arg(dust_ignore_regex());
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
    tops.sort_by_key(|b| std::cmp::Reverse(b.bytes));
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

/// Count lines in-process with `tokei`.
///
/// Replaces shelling out to `cloc`, which dominated every sample: on a ~4k-file
/// Rust tree `cloc` took ~8.5s of a 9.4s sample (a Perl process that re-walked
/// the whole tree a second time), and it silently lost *all* LOC stats whenever
/// it appended its "files took longer than expected" warning after the JSON —
/// trailing bytes that `serde_json` rejects.
///
/// Unlike `cloc`, `tokei` honours `.gitignore`, so generated-but-tracked-as-
/// ignored files no longer inflate the count. Totals therefore differ slightly
/// from pre-0.2 releases.
fn run_loc(path: &Path) -> Result<LocStats> {
    use tokei::{Config, Languages};

    let config = Config::default();
    let mut languages = Languages::new();
    languages.get_statistics(&[path], IGNORE_DIRS, &config);

    let mut langs: Vec<LangStat> = languages
        .iter()
        .filter_map(|(ty, lang)| {
            let sum = lang.summarise();
            if sum.code == 0 && sum.reports.is_empty() {
                return None;
            }
            Some(LangStat {
                name: ty.name().to_string(),
                blank: sum.blanks as u64,
                comment: sum.comments as u64,
                code: sum.code as u64,
            })
        })
        .collect();

    let total = languages.total();
    let files = languages
        .values()
        .map(|l| l.summarise().reports.len() as u64)
        .sum();

    if files == 0 {
        bail!("no countable source files found");
    }

    langs.sort_by_key(|b| std::cmp::Reverse(b.code));
    langs.truncate(16);

    Ok(LocStats {
        files,
        blank: total.blanks as u64,
        comment: total.comments as u64,
        code: total.code as u64,
        langs,
    })
}

fn git_ok(path: &Path) -> bool {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(path)
        .args(["rev-parse", "--is-inside-work-tree"]);
    // Bounded: a bare `.output()` here can block the collector forever — e.g. on
    // a stale network mount or a repo whose index is locked.
    run_timed(cmd, PROBE_TIMEOUT)
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
    if !timed("probe", || git_ok(path)) {
        return Ok(None);
    }
    // Four independent invocations. Run them concurrently: in series they cost
    // the sum of four spawn+wait round-trips, and `git log --shortstat` over a
    // large history dominates the rest.
    let (p_branch, p_unstaged) = (path.to_path_buf(), path.to_path_buf());
    let (p_staged, p_log) = (path.to_path_buf(), path.to_path_buf());

    let h_branch = thread::spawn(move || {
        timed("branch", || {
            git_out(&p_branch, &["rev-parse", "--abbrev-ref", "HEAD"])
        })
        .unwrap_or_else(|_| "HEAD".into())
        .trim()
        .to_string()
    });
    // Working-tree stats: numstat is fine (usually << pipe buffer). On huge
    // dirty trees, shortstat is a fallback if numstat fails/times out.
    let h_unstaged = thread::spawn(move || timed("unstg", || git_diff_stat(&p_unstaged, false)));
    let h_staged = thread::spawn(move || timed("stged", || git_diff_stat(&p_staged, true)));
    let h_log = thread::spawn(move || timed("log", || recent_commits(&p_log, GIT_LOG_LIMIT)));

    let branch = h_branch.join().unwrap_or_else(|_| "HEAD".into());
    let (u_ins, u_del) = h_unstaged.join().unwrap_or((0, 0));
    let (s_ins, s_del) = h_staged.join().unwrap_or((0, 0));
    let commits = h_log.join().unwrap_or_default();

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

/// One commit's wall time + shortstat churn (for windowed deltas in JSON reports).
#[derive(Debug, Clone, Copy)]
pub struct CommitChurn {
    pub unix_ts: u64,
    pub insertions: u64,
    pub deletions: u64,
}

/// Commits since `max_age` ago with insertions/deletions (newest first).
/// Used to fill `deltas[].insertions` / `deltas[].deletions` with one git call.
pub fn git_commit_churn_since(path: &Path, max_age: Duration) -> Vec<CommitChurn> {
    if !git_ok(path) {
        return Vec::new();
    }
    let since = format!("--since={} seconds ago", max_age.as_secs().max(1));
    // RS + unix timestamp, then optional shortstat line(s).
    let pretty = "--pretty=format:%x1e%ct";
    let raw = match git_out(path, &["log", &since, pretty, "--shortstat"]) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    parse_commit_churn_stream(&raw)
}

/// Parse `git log --pretty=format:%x1e%ct --shortstat` output into per-commit churn.
pub fn parse_commit_churn_stream(raw: &str) -> Vec<CommitChurn> {
    let mut out = Vec::new();
    let mut cur_ts: Option<u64> = None;
    let mut cur_ins = 0u64;
    let mut cur_del = 0u64;

    let flush = |out: &mut Vec<CommitChurn>, ts: &mut Option<u64>, ins: &mut u64, del: &mut u64| {
        if let Some(t) = ts.take() {
            out.push(CommitChurn {
                unix_ts: t,
                insertions: *ins,
                deletions: *del,
            });
        }
        *ins = 0;
        *del = 0;
    };

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix('\u{1e}') {
            flush(&mut out, &mut cur_ts, &mut cur_ins, &mut cur_del);
            let rest = rest.trim();
            // Usually just a timestamp; sometimes shortstat shares the line.
            let mut parts = rest.splitn(2, char::is_whitespace);
            cur_ts = parts.next().and_then(|t| t.parse().ok());
            if let Some(tail) = parts.next() {
                let tail = tail.trim();
                if tail.contains("insertion")
                    || tail.contains("deletion")
                    || tail.contains("changed")
                {
                    let (ins, del) = parse_shortstat(tail);
                    cur_ins = ins;
                    cur_del = del;
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        if line.contains("insertion") || line.contains("deletion") || line.contains("changed") {
            let (ins, del) = parse_shortstat(line);
            cur_ins = ins;
            cur_del = del;
        }
    }
    flush(&mut out, &mut cur_ts, &mut cur_ins, &mut cur_del);
    out
}

/// Sum commit insertions/deletions with `unix_ts >= now - window_secs`.
pub fn sum_churn_in_window(churn: &[CommitChurn], window_secs: u64, now_unix: u64) -> (u64, u64) {
    let cutoff = now_unix.saturating_sub(window_secs);
    let mut ins = 0u64;
    let mut del = 0u64;
    for c in churn {
        if c.unix_ts >= cutoff {
            ins = ins.saturating_add(c.insertions);
            del = del.saturating_add(c.deletions);
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

    /// dust's `-X` matches whole paths, not names, so heim's old
    /// `-X build -X node_modules …` never pruned a nested build dir.
    #[test]
    fn dust_regex_matches_nested_components() {
        let re = dust_ignore_regex();
        assert!(re.starts_with("/("), "{re}");
        assert!(re.ends_with(")/"), "{re}");
        // Dots must be escaped or `.git` would match `xgit`, `1git`, ...
        assert!(re.contains(r"\.git|"), "{re}");
        assert!(re.contains(r"\.dart_tool"), "{re}");
        assert!(re.contains("node_modules"), "{re}");
        // Every ignore dir is represented.
        for d in IGNORE_DIRS {
            assert!(re.contains(&regex_escape(d)), "missing {d} in {re}");
        }
    }

    #[test]
    fn regex_escape_escapes_metachars() {
        assert_eq!(regex_escape(".git"), r"\.git");
        assert_eq!(regex_escape("node_modules"), "node_modules");
        assert_eq!(regex_escape(".pub-cache"), r"\.pub\-cache");
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

    #[test]
    fn parse_commit_churn_and_window_sum() {
        let t = "\u{1e}1000\n 2 files changed, 10 insertions(+), 3 deletions(-)\n\
\u{1e}2000\n 1 file changed, 7 insertions(+)\n\
\u{1e}3000\n 1 file changed, 2 deletions(-)\n";
        let churn = parse_commit_churn_stream(t);
        assert_eq!(churn.len(), 3);
        assert_eq!(churn[0].unix_ts, 1000);
        assert_eq!(churn[0].insertions, 10);
        assert_eq!(churn[0].deletions, 3);
        assert_eq!(churn[1].insertions, 7);
        assert_eq!(churn[2].deletions, 2);
        // window ending at 3000 of length 1000 → ts >= 2000
        let (ins, del) = sum_churn_in_window(&churn, 1000, 3000);
        assert_eq!(ins, 7);
        assert_eq!(del, 2);
        // full span
        let (ins, del) = sum_churn_in_window(&churn, 3000, 3000);
        assert_eq!(ins, 17);
        assert_eq!(del, 5);
    }
}
