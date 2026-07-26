//! heim — real-time project LOC / size / git TUI + JSON stats for AI agents.

mod app;
mod collect;
mod fmt;
mod report;
mod store;
mod theme;
mod ui;

use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use app::{App, Drag, Focus};
use collect::{DirSize, Sample, SizeBackendKind};

#[derive(Parser, Debug)]
#[command(
    name = "heim",
    about = "Stop vibe-code bloat: real-time LOC/size/git deltas + JSON agents can self-audit",
    long_about = "Live project control surface for AI coding sessions.\n\
                  \n\
                  TUI mode tracks languages, disk weight, git, and time-window \
                  deltas (how many lines landed in the last 5m–2h). Machine mode \
                  (`--once --json`) prints a detailed stats report agents can \
                  self-audit against, and always refreshes `<project>/.heim/stats.json`.",
    version
)]
struct Cli {
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    /// Refresh interval in seconds (TUI mode).
    #[arg(short, long, default_value_t = 60)]
    interval: u64,

    #[arg(long, default_value = "auto", value_parser = parse_size_backend)]
    size_backend: SizeBackendKind,

    /// Take one sample and exit (no TUI).
    #[arg(long)]
    once: bool,

    /// Emit machine-readable JSON (implies one-shot sample; no TUI).
    /// Also writes `<project>/.heim/stats.json` for agents.
    #[arg(long)]
    json: bool,

    /// Write JSON report to FILE (implies --json). Use `-` for stdout only.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: Option<PathBuf>,
}

fn parse_size_backend(s: &str) -> std::result::Result<SizeBackendKind, String> {
    SizeBackendKind::parse(s).ok_or_else(|| format!("expected auto|dust|walk, got {s}"))
}

enum Job {
    Full,
    WeightList(PathBuf),
}

enum Msg {
    Sample(Sample),
    Weight {
        path: PathBuf,
        total: u64,
        children: Vec<DirSize>,
    },
}

struct Worker {
    tx_job: Sender<Job>,
}

impl Worker {
    fn spawn(
        path: PathBuf,
        size_pref: SizeBackendKind,
        sample_tx: Sender<Msg>,
    ) -> (Self, thread::JoinHandle<()>) {
        let (tx_job, rx_job) = mpsc::channel::<Job>();
        let handle = thread::spawn(move || {
            while let Ok(job) = rx_job.recv() {
                let mut job = job;
                while let Ok(next) = rx_job.try_recv() {
                    job = next;
                }
                match job {
                    Job::Full => {
                        let s = collect::collect(&path, size_pref);
                        let _ = sample_tx.send(Msg::Sample(s));
                    }
                    Job::WeightList(p) => {
                        let (total, children) = collect::list_children_fast(&p);
                        let _ = sample_tx.send(Msg::Weight {
                            path: p,
                            total,
                            children,
                        });
                    }
                }
            }
        });
        (Self { tx_job }, handle)
    }

    fn request_full(&self) {
        let _ = self.tx_job.send(Job::Full);
    }

    fn request_weight(&self, path: PathBuf) {
        let _ = self.tx_job.send(Job::WeightList(path));
    }
}

pub struct CountingAlloc;
pub static ALLOCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BYTES.fetch_add(l.size() as u64, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        std::alloc::System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BYTES.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.realloc(p, l, n)
    }
}
#[global_allocator]
static GA: CountingAlloc = CountingAlloc;

fn main() {
    if let Err(e) = run() {
        eprintln!("heim: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let path = cli
        .path
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"))
        .canonicalize()
        .context("resolve path")?;
    if !path.is_dir() {
        bail!("{} is not a directory", path.display());
    }

    let want_json = cli.json || cli.output.is_some();
    if want_json || cli.once {
        if want_json {
            let report = report::collect_report(&path, cli.size_backend)?;
            let text = report.to_json_pretty()?;
            match cli.output.as_deref() {
                Some(p) if p.as_os_str() != "-" => {
                    report.write_to(p)?;
                    println!("{text}");
                    eprintln!("heim: wrote {}", p.display());
                    eprintln!(
                        "heim: also updated {}",
                        report::store_stats_path(&path).display()
                    );
                }
                _ => {
                    println!("{text}");
                    eprintln!(
                        "heim: updated {}",
                        report::store_stats_path(&path).display()
                    );
                }
            }
        } else {
            // Text one-shot: still refresh `.heim/stats.json` for agents.
            let mut app = App::new(path.clone(), cli.interval, cli.size_backend);
            let s = collect::collect(&path, cli.size_backend);
            app.apply_sample(s.clone());
            print_once(&path, &s);
            if let Some(mut st) = app.store.take() {
                st.end_session();
                std::mem::forget(st);
            }
        }
        return Ok(());
    }

    let mut app = App::new(path.clone(), cli.interval, cli.size_backend);
    let (sample_tx, sample_rx) = mpsc::channel();
    let (worker, _jh) = Worker::spawn(path, cli.size_backend, sample_tx);

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;

    app.refreshing = true;
    worker.request_full();

    // 120Hz frame budget for smooth animation pacing.
    let frame = Duration::from_nanos(1_000_000_000 / theme::TARGET_FPS);
    let res = event_loop(&mut term, &mut app, &worker, &sample_rx, frame);

    if let Some(st) = app.store.as_mut() {
        st.end_session();
    }
    if let Some(st) = app.store.take() {
        std::mem::forget(st);
    }

    disable_raw_mode()?;
    execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    term.show_cursor()?;
    res
}

fn print_once(path: &std::path::Path, s: &Sample) {
    println!("path:   {}", path.display());
    println!(
        "size:   {}  (via {})",
        fmt::human_bytes_short(s.size_bytes),
        s.size_engine.label()
    );
    if !s.top_dirs.is_empty() {
        println!("top:");
        for (i, d) in s.top_dirs.iter().enumerate() {
            let p = fmt::pct(d.bytes, s.size_bytes);
            println!(
                "  {:>2}. {:<16} {:>7}  {:4.0}%",
                i + 1,
                d.name,
                fmt::human_bytes_short(d.bytes),
                p
            );
        }
    }
    if let Some(loc) = &s.loc {
        println!(
            "loc:    code={} files={} blank={} comments={}",
            fmt::num(loc.code),
            fmt::num(loc.files),
            fmt::num(loc.blank),
            fmt::num(loc.comment)
        );
    } else if let Some(e) = &s.loc_err {
        println!("loc:    error: {e}");
    }
    if let Some(g) = &s.git {
        println!("git:    {}  +{}  -{}", g.branch, g.ins, g.del);
        for c in &g.commits {
            println!(
                "  {}  +{}/-{}  {}  ({})",
                c.short, c.ins, c.del, c.subject, c.author
            );
        }
    } else if let Some(e) = &s.git_err {
        println!("git:    {e}");
    }
    println!("took:   {:.2}s", s.duration.as_secs_f64());
}

fn contains(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x
        && col < r.x.saturating_add(r.width)
        && row >= r.y
        && row < r.y.saturating_add(r.height)
}

fn event_loop(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    worker: &Worker,
    sample_rx: &Receiver<Msg>,
    frame: Duration,
) -> Result<()> {
    let mut lang_vis = 8usize;
    let mut git_vis = 5usize;
    loop {
        let frame_start = Instant::now();

        loop {
            match sample_rx.try_recv() {
                Ok(Msg::Sample(s)) => app.apply_sample(s),
                Ok(Msg::Weight {
                    path,
                    total,
                    children,
                }) => {
                    if app.weight_cwd() == path {
                        app.apply_weight_listing(path, total, children);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => bail!("collector stopped"),
            }
        }

        if app.due() {
            app.refreshing = true;
            worker.request_full();
        }

        // The tick keeps advancing at 120Hz so animation pacing stays correct,
        // but we only repaint when the frame would actually differ: state/input
        // changed, or the quantized animation moved. Ratatui's double buffer
        // saves terminal *bytes*, not CPU — `swap_buffers` resets the incoming
        // buffer, so every `draw` re-renders all 1000+ lines of widget layout.
        app.bump_tick();
        let size = term.size()?;
        app.clamp_layout(size.height);
        let vis = match app.focus {
            Focus::Git => git_vis,
            _ => lang_vis,
        };
        app.ensure_sel_visible(vis);

        let anim = theme::anim_frame(app.tick);
        if app.dirty || anim != app.last_anim {
            app.last_anim = anim;
            app.dirty = false;
            term.draw(|f| {
                let (lv, gv) = ui::draw(f, app);
                lang_vis = lv;
                git_vis = gv;
            })?;
        }

        // Drain input for the rest of the frame budget (keeps UI responsive at 120Hz).
        let mut quit = false;
        loop {
            let remaining = frame.saturating_sub(frame_start.elapsed());
            if remaining.is_zero() {
                break;
            }
            if !event::poll(remaining)? {
                break;
            }
            // Any delivered event can move focus, selection, layout or scroll —
            // cheaper to repaint once than to prove which ones didn't.
            app.dirty = true;
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    let vis = match app.focus {
                        Focus::Git => git_vis,
                        _ => lang_vis,
                    };
                    match key.code {
                        KeyCode::Char('q') => {
                            quit = true;
                            break;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            quit = true;
                            break;
                        }
                        KeyCode::Char('r') => {
                            if !app.refreshing {
                                app.refreshing = true;
                                worker.request_full();
                            }
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => app.bump_interval(1),
                        KeyCode::Char('-') | KeyCode::Char('_') => app.bump_interval(-1),
                        KeyCode::Char('?') => app.help = !app.help,
                        KeyCode::Esc if app.help => app.help = false,
                        KeyCode::Tab => {
                            app.focus = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                app.focus.prev()
                            } else {
                                app.focus.next()
                            };
                        }
                        KeyCode::BackTab => app.focus = app.focus.prev(),
                        KeyCode::Down | KeyCode::Char('j') => app.move_sel(1, vis),
                        KeyCode::Up | KeyCode::Char('k') => app.move_sel(-1, vis),
                        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => {
                            if app.focus == Focus::Weight {
                                if let Some(p) = app.weight_enter() {
                                    worker.request_weight(p);
                                }
                            }
                        }
                        KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => {
                            if app.focus == Focus::Weight {
                                if let Some(p) = app.weight_up() {
                                    worker.request_weight(p);
                                }
                            }
                        }
                        KeyCode::PageDown => app.scroll_view(vis as i32, vis),
                        KeyCode::PageUp => app.scroll_view(-(vis as i32), vis),
                        KeyCode::Home => match app.focus {
                            Focus::Lang => {
                                app.lang_sel = 0;
                                app.lang_scroll = 0;
                            }
                            Focus::Weight => {
                                app.weight_sel = 0;
                                app.weight_scroll = 0;
                            }
                            Focus::Git => {
                                app.git_sel = 0;
                                app.git_scroll = 0;
                            }
                        },
                        KeyCode::End => {
                            match app.focus {
                                Focus::Lang => {
                                    let n = app.langs().len();
                                    if n > 0 {
                                        app.lang_sel = n - 1;
                                    }
                                }
                                Focus::Weight => {
                                    let n = app.weight_children.len();
                                    if n > 0 {
                                        app.weight_sel = n - 1;
                                    }
                                }
                                Focus::Git => {
                                    if let Some(g) = app.last.as_ref().and_then(|s| s.git.as_ref())
                                    {
                                        if !g.commits.is_empty() {
                                            app.git_sel = g.commits.len() - 1;
                                        }
                                    }
                                }
                            }
                            app.ensure_sel_visible(vis);
                        }
                        _ => {}
                    }
                }
                Event::Mouse(m) => {
                    let col = m.column;
                    let row = m.row;
                    let size = term.size()?;
                    app.clamp_layout(size.height);
                    let lang_v = lang_vis;
                    let git_v = git_vis;

                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Resize handles first
                            if contains(app.hit.v_split, col, row)
                                || (col
                                    >= app
                                        .hit
                                        .lang
                                        .x
                                        .saturating_add(app.hit.lang.width.saturating_sub(1))
                                    && col <= app.hit.weight.x
                                    && row >= app.hit.lang.y
                                    && row < app.hit.lang.y.saturating_add(app.hit.lang.height))
                            {
                                app.drag = Some(Drag::HSplit {
                                    start_col: col,
                                    start_pct: app.lang_pct,
                                });
                            } else if contains(app.hit.git_edge, col, row)
                                || (row == app.hit.git.y
                                    && col >= app.hit.git.x
                                    && col < app.hit.git.x.saturating_add(app.hit.git.width))
                            {
                                app.drag = Some(Drag::GitTop {
                                    start_row: row,
                                    start_h: app.git_h,
                                });
                            } else if contains(app.hit.lang, col, row) {
                                app.focus = Focus::Lang;
                                let inner_y = row.saturating_sub(app.hit.lang.y.saturating_add(2));
                                let idx = app.lang_scroll + inner_y as usize;
                                let n = app.langs().len();
                                if n > 0 {
                                    app.lang_sel = idx.min(n - 1);
                                    app.ensure_sel_visible(lang_v);
                                }
                            } else if contains(app.hit.weight, col, row) {
                                app.focus = Focus::Weight;
                                let inner_y =
                                    row.saturating_sub(app.hit.weight.y.saturating_add(2));
                                let idx = app.weight_scroll + inner_y as usize;
                                let n = app.weight_children.len();
                                if n > 0 {
                                    let idx = idx.min(n - 1);
                                    if app.weight_sel == idx {
                                        if let Some(p) = app.weight_enter() {
                                            worker.request_weight(p);
                                        }
                                    } else {
                                        app.weight_sel = idx;
                                        app.ensure_sel_visible(lang_v);
                                    }
                                }
                            } else if contains(app.hit.git, col, row) {
                                app.focus = Focus::Git;
                                let inner_y = row.saturating_sub(app.hit.git.y.saturating_add(2));
                                let idx = app.git_scroll + inner_y as usize;
                                let n = app
                                    .last
                                    .as_ref()
                                    .and_then(|s| s.git.as_ref())
                                    .map(|g| g.commits.len())
                                    .unwrap_or(0);
                                if n > 0 {
                                    app.git_sel = idx.min(n - 1);
                                    app.ensure_sel_visible(git_v);
                                }
                            }
                        }
                        MouseEventKind::Drag(MouseButton::Left) => match app.drag {
                            Some(Drag::HSplit {
                                start_col,
                                start_pct,
                            }) => {
                                let dx = col as i32 - start_col as i32;
                                // ~1% per column of total width
                                let w = size.width.max(1) as i32;
                                let dp = (dx * 100) / w;
                                app.lang_pct = (start_pct as i32 + dp).clamp(30, 70) as u16;
                            }
                            Some(Drag::GitTop { start_row, start_h }) => {
                                // Drag edge down → taller git; up → shorter
                                let dy = row as i32 - start_row as i32;
                                app.git_h = (start_h as i32 - dy).clamp(3, 30) as u16;
                                app.clamp_layout(size.height);
                            }
                            None => {}
                        },
                        MouseEventKind::Up(MouseButton::Left) => {
                            app.drag = None;
                        }
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                            let dir = if matches!(m.kind, MouseEventKind::ScrollDown) {
                                3
                            } else {
                                -3
                            };
                            if contains(app.hit.lang, col, row) {
                                app.focus = Focus::Lang;
                                app.scroll_view(dir, lang_v);
                            } else if contains(app.hit.weight, col, row) {
                                app.focus = Focus::Weight;
                                app.scroll_view(dir, lang_v);
                            } else if contains(app.hit.git, col, row) {
                                app.focus = Focus::Git;
                                app.scroll_view(dir, git_v);
                            } else {
                                let vis = match app.focus {
                                    Focus::Git => git_v,
                                    _ => lang_v,
                                };
                                app.scroll_view(dir, vis);
                            }
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            if contains(app.hit.weight, col, row) {
                                app.focus = Focus::Weight;
                                if let Some(p) = app.weight_up() {
                                    worker.request_weight(p);
                                }
                            }
                        }
                        MouseEventKind::Down(MouseButton::Middle)
                            if contains(app.hit.weight, col, row) =>
                        {
                            app.focus = Focus::Weight;
                            if let Some(p) = app.weight_enter() {
                                worker.request_weight(p);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Resize(_, h) => {
                    app.clamp_layout(h);
                }
                _ => {}
            }
        }
        if quit {
            break;
        }
    }
    Ok(())
}
