//! Compact interactive dashboard: monitor, ranks, git — responsive, no dupe noise.

use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{opt_delta_bytes, opt_delta_i64, App, Focus};
use crate::fmt;
use crate::theme::{self, Theme, BULLET_DIAMOND};

fn dstyle(th: &Theme, v: Option<i64>) -> Style {
    match v {
        Some(n) if n > 0 => Style::default().fg(th.delta_up),
        Some(n) if n < 0 => Style::default().fg(th.delta_down),
        _ => th.dim(),
    }
}

fn panel<'a>(th: &Theme, title_txt: &'a str, focused: bool, running: bool) -> Block<'a> {
    let title_style = if running {
        th.title_running()
    } else if focused {
        th.title()
    } else {
        th.dim().add_modifier(Modifier::BOLD)
    };
    let border = if running {
        Style::default().fg(th.accent_running)
    } else if focused {
        th.border_focused()
    } else {
        th.border()
    };
    let title = if focused {
        format!(" {} {title_txt} ", theme::ACCENT_RAIL)
    } else {
        format!(" {title_txt} ")
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Span::styled(title, title_style))
}

fn dim_label(th: &Theme, s: &str) -> Span<'static> {
    Span::styled(s.to_string(), th.dim())
}

fn sel_style(th: &Theme, selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(th.text_primary)
            .bg(th.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        th.bright()
    }
}

/// Soften primary text when panel is unfocused.
fn body_for(th: &Theme, focused: bool) -> Style {
    if focused {
        th.bold_body()
    } else {
        th.body()
    }
}

/// Compact percent label: tiny non-zero shares show as `<1%` instead of `0%`.
fn pct_label(part: u64, whole: u64) -> String {
    let p = fmt::pct(part, whole);
    if part > 0 && p < 0.5 {
        " <1%".into()
    } else {
        format!("{p:3.0}%")
    }
}

/// Returns (lang_visible_rows, git_visible_rows).
pub fn draw(f: &mut Frame, app: &mut App) -> (usize, usize) {
    let th = Theme::default();
    let area = f.area();
    app.clamp_layout(area.height);

    let git_h = app.effective_git_h(area.height);
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // monitor: title + 3 lines
            Constraint::Min(4),    // ranks (main workspace)
            Constraint::Length(git_h),
            Constraint::Length(1), // footer
        ])
        .split(area);

    app.hit.monitor = root[0];

    draw_header(f, root[0], app, &th);
    let lang_vis = draw_ranks(f, root[1], app, &th);
    draw_git(f, root[2], app, &th);
    app.hit.git = root[2];
    app.hit.git_edge = Rect {
        x: root[2].x,
        y: root[2].y,
        width: root[2].width,
        height: 1,
    };
    let git_vis = root[2].height.saturating_sub(3) as usize;
    draw_footer(f, root[3], app, &th);

    if app.help {
        draw_help(f, centered(area, 72, 18), &th);
    }
    (lang_vis.max(1), git_vis.max(1))
}

// ── Monitor ────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, app: &App, th: &Theme) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let (code, files, blank, cmt) = app
        .last
        .as_ref()
        .and_then(|s| s.loc.as_ref())
        .map(|l| (Some(l.code), Some(l.files), Some(l.blank), Some(l.comment)))
        .unwrap_or((None, None, None, None));
    let size = app.last.as_ref().map(|s| s.size_bytes);
    let loc_err = app.last.as_ref().and_then(|s| s.loc_err.clone());

    let (status_icon, phase_style) = if app.refreshing {
        (
            theme::spinner_frame(app.tick),
            Style::default().fg(th.accent_running),
        )
    } else {
        (
            theme::monitor_icon(app.tick),
            Style::default().fg(theme::pulse_color(app.tick, th.accent_success, th.gray_dim)),
        )
    };

    // Line 0: name · path · every Ns · age · phase
    let name = app.project_name();
    let age = app.update_age_value();
    let every = fmt::hum_interval(app.interval.as_secs());
    let meta = match age.as_str() {
        "now" => format!("every {every} · just now"),
        "—" | "…" => format!("every {every} · {age}"),
        other => format!("every {every} · {other} ago"),
    };
    // Reserve ~ right side for meta + phase (~ 22–28 cols)
    let right_budget = meta.chars().count() + 12;
    let path_budget = inner_w
        .saturating_sub(name.chars().count() + 2 + right_budget)
        .max(8);
    let line0 = Line::from(vec![
        Span::styled(
            name,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            fmt::truncate_middle(&app.path_label, path_budget),
            th.path_style().add_modifier(Modifier::DIM),
        ),
        Span::raw("  "),
        Span::styled(meta, th.dim()),
        Span::raw("  "),
        Span::styled(format!("{status_icon} "), phase_style),
        Span::styled(app.phase_label().to_string(), phase_style),
    ]);

    // Line 1: totals only (source of truth). Adaptive density by width.
    let line1 = if let Some(err) = loc_err {
        Line::from(vec![
            dim_label(th, "code "),
            Span::styled(err, Style::default().fg(th.accent_error)),
            dim_label(th, "  size "),
            Span::styled(
                size.map(fmt::human_bytes_short)
                    .unwrap_or_else(|| "—".into()),
                th.bright().add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        let mut spans = vec![
            dim_label(th, "code "),
            Span::styled(
                code.map(fmt::num).unwrap_or_else(|| "—".into()),
                th.bright().add_modifier(Modifier::BOLD),
            ),
            dim_label(th, "  files "),
            Span::styled(files.map(fmt::num).unwrap_or_else(|| "—".into()), th.body()),
        ];
        // blank/comments only when there is room
        if inner_w >= 72 {
            spans.push(dim_label(th, "  blank "));
            spans.push(Span::styled(
                blank.map(fmt::num).unwrap_or_else(|| "—".into()),
                th.dim(),
            ));
        }
        if inner_w >= 88 {
            spans.push(dim_label(th, "  comments "));
            spans.push(Span::styled(
                cmt.map(fmt::num).unwrap_or_else(|| "—".into()),
                th.dim(),
            ));
        }
        spans.push(dim_label(th, "  size "));
        spans.push(Span::styled(
            size.map(fmt::human_bytes_short)
                .unwrap_or_else(|| "—".into()),
            th.bright().add_modifier(Modifier::BOLD),
        ));
        Line::from(spans)
    };

    // Line 2: Δcode windows (5m…2h) + secondary insight chips.
    let line2 = if app.last.is_none() {
        Line::from(vec![
            Span::styled(format!("{BULLET_DIAMOND} "), Style::default().fg(th.accent)),
            Span::styled(
                if app.refreshing {
                    "collecting first sample…"
                } else {
                    "waiting…"
                },
                th.dim(),
            ),
        ])
    } else {
        draw_insight_line(app, th, inner_w)
    };

    let title = if app.refreshing {
        format!("monitor {}", theme::spinner_frame(app.tick))
    } else {
        "monitor".into()
    };
    f.render_widget(
        Paragraph::new(vec![line0, line1, line2]).block(panel(th, &title, false, app.refreshing)),
        area,
    );
}

/// Monitor insight: code-line window deltas first, then secondary chips.
fn draw_insight_line(app: &App, th: &Theme, inner_w: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        format!("{BULLET_DIAMOND} "),
        Style::default().fg(th.accent),
    )];

    // How many windows fit: each ~ "10m +1234 " ≈ 10–12 cols + label.
    let n_win = if inner_w < 56 {
        3 // 5m 10m 30m
    } else if inner_w < 72 {
        4 // +1h
    } else {
        5 // +2h
    };

    spans.push(Span::styled("Δ ".to_string(), th.dim()));
    let windows = app.code_window_deltas();
    for (i, (label, delta)) in windows.into_iter().take(n_win).enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ".to_string(), th.dim()));
        }
        spans.push(Span::styled(format!("{label} "), th.dim()));
        let (txt, st) = match delta {
            None => ("—".to_string(), th.dim()),
            Some(0) => ("·".to_string(), th.dim()),
            Some(n) => (
                fmt::signed_i64(n),
                dstyle(th, Some(n)).add_modifier(Modifier::BOLD),
            ),
        };
        spans.push(Span::styled(txt, st));
    }

    // Secondary chips if room remains (rough budget).
    let used_est = 2 + 2 + n_win * 10; // diamond + "Δ " + windows
    if used_est + 12 < inner_w {
        let chips = app.insight_chips();
        if !chips.is_empty() {
            let room = inner_w.saturating_sub(used_est + 3);
            let joined = chips.join(" · ");
            let tail = fmt::truncate_middle(&joined, room);
            if !tail.is_empty() {
                spans.push(Span::styled(" · ".to_string(), th.dim()));
                spans.push(Span::styled(tail, th.body()));
            }
        }
    }

    Line::from(spans)
}

// ── Ranks ──────────────────────────────────────────────────────────────────

fn draw_ranks(f: &mut Frame, area: Rect, app: &mut App, th: &Theme) -> usize {
    let left = app.lang_pct.clamp(30, 70);
    let right = 100 - left;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left),
            Constraint::Length(1),
            Constraint::Percentage(right),
        ])
        .split(area);

    app.hit.lang = cols[0];
    app.hit.v_split = cols[1];
    app.hit.weight = cols[2];

    // Visible resize rail
    let rail = if matches!(app.drag, Some(crate::app::Drag::HSplit { .. })) {
        Style::default().fg(th.accent)
    } else {
        th.dim()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "┊".repeat(cols[1].height.max(1) as usize),
            rail,
        ))),
        cols[1],
    );

    let vis = draw_lang_table(f, cols[0], app, th);
    draw_weight_table(f, cols[2], app, th, vis);
    vis
}

const NAME_MIN: usize = 14;

#[derive(Clone, Copy)]
struct FlexCols {
    name_w: usize,
    show_blank: bool,
    show_comments: bool,
    show_delta: bool,
}

/// Budget columns: fixed metrics first, leftover → name/path (no % bars).
fn layout_lang(inner_w: u16, show_delta: bool) -> FlexCols {
    // Always: `# sp name sp code sp % [sp Δ]`
    // Optional: blank, comments when wide.
    let mut fixed = 2 + 1 + 1 + 8 + 1 + 4; // # sp … sp code sp %
    if show_delta {
        fixed += 1 + 5; // sp Δ
    }
    let w = inner_w as usize;
    let show_comments = w >= fixed + NAME_MIN + 1 + 8;
    if show_comments {
        fixed += 1 + 8;
    }
    let show_blank = w >= fixed + NAME_MIN + 1 + 5;
    if show_blank {
        fixed += 1 + 5;
    }

    let name_w = w.saturating_sub(fixed).max(1);
    FlexCols {
        name_w,
        show_blank,
        show_comments,
        show_delta,
    }
}

fn layout_weight(inner_w: u16, show_delta: bool) -> FlexCols {
    // `# sp › name sp size sp % [sp Δ]`
    let mut fixed = 2 + 1 + 1 + 1 + 7 + 1 + 4;
    if show_delta {
        fixed += 1 + 6;
    }
    let name_w = (inner_w as usize).saturating_sub(fixed).max(1);
    FlexCols {
        name_w,
        show_blank: false,
        show_comments: false,
        show_delta,
    }
}

fn header_row(th: &Theme, cols: &[(&str, usize)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (name, w)) in cols.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let label = if name.is_empty() {
            " ".repeat(*w)
        } else if name.chars().count() > *w {
            fmt::truncate_middle(name, *w)
        } else {
            fmt::pad_right(name, *w)
        };
        spans.push(Span::styled(label, th.dim().add_modifier(Modifier::BOLD)));
    }
    Line::from(spans)
}

fn draw_lang_table(f: &mut Frame, area: Rect, app: &App, th: &Theme) -> usize {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let vis = inner.height.saturating_sub(1) as usize;
    let total_code = app
        .last
        .as_ref()
        .and_then(|s| s.loc.as_ref())
        .map(|l| l.code)
        .unwrap_or(0);
    let lay = layout_lang(inner.width, app.has_lang_deltas());
    let focused = app.focus == Focus::Lang;
    let langs = app.langs();
    let n = langs.len();

    let mut header_cols: Vec<(&str, usize)> = vec![("#", 2), ("language", lay.name_w), ("code", 8)];
    if lay.show_blank {
        header_cols.push(("blank", 5));
    }
    if lay.show_comments {
        header_cols.push(("comments", 8));
    }
    header_cols.push(("%", 4));
    if lay.show_delta {
        header_cols.push(("Δ", 5));
    }
    let mut lines = vec![header_row(th, &header_cols)];

    if app.last.is_none() {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", theme::spinner_frame(app.tick)),
                Style::default().fg(th.accent_running),
            ),
            Span::styled("collecting languages…", th.dim()),
        ]));
    } else if n == 0 {
        lines.push(Line::from(Span::styled("  no languages", th.dim())));
    } else {
        let end = (app.lang_scroll + vis).min(n);
        for (i, lang) in langs.iter().enumerate().take(end).skip(app.lang_scroll) {
            let d = app
                .lang_delta_session(&lang.name)
                .or_else(|| app.lang_delta_interval(&lang.name));
            let selected = focused && i == app.lang_sel;
            let mut spans = vec![
                Span::styled(
                    fmt::pad_left(&(i + 1).to_string(), 2),
                    if selected { th.bright() } else { th.dim() },
                ),
                Span::raw(" "),
                Span::styled(
                    fmt::pad_right(&fmt::truncate_middle(&lang.name, lay.name_w), lay.name_w),
                    sel_style(th, selected),
                ),
                Span::raw(" "),
                Span::styled(
                    fmt::pad_left(&fmt::num(lang.code), 8),
                    body_for(th, focused || selected),
                ),
            ];
            if lay.show_blank {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    fmt::pad_left(&fmt::num(lang.blank), 5),
                    th.dim(),
                ));
            }
            if lay.show_comments {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    fmt::pad_left(&fmt::num(lang.comment), 8),
                    th.dim(),
                ));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                fmt::pad_left(&pct_label(lang.code, total_code), 4),
                th.dim(),
            ));
            if lay.show_delta {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    fmt::pad_left(&opt_delta_i64(d), 5),
                    dstyle(th, d),
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    let scroll = if n > vis.max(1) {
        format!(
            "  {}-{}/{}",
            app.lang_scroll + 1,
            (app.lang_scroll + vis).min(n),
            n
        )
    } else {
        String::new()
    };
    // Title: no redundant "LOC · N lines" when monitor already has code total
    let title = if n > 0 {
        format!("languages · {n}{scroll}")
    } else {
        format!("languages{scroll}")
    };
    f.render_widget(
        Paragraph::new(lines).block(panel(th, &title, focused, false)),
        area,
    );
    vis.max(1)
}

fn draw_weight_table(f: &mut Frame, area: Rect, app: &App, th: &Theme, vis: usize) {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let total = if app.weight_stack.is_empty() {
        app.last
            .as_ref()
            .map(|s| s.size_bytes)
            .unwrap_or(app.weight_total)
    } else {
        app.weight_total
    };
    let engine = app.size_engine_label();
    let lay = layout_weight(inner.width, app.has_dir_deltas());
    let focused = app.focus == Focus::Weight;
    let children = &app.weight_children;
    let n = children.len();

    let mut header_cols: Vec<(&str, usize)> = vec![
        ("#", 2),
        ("", 1),
        ("path", lay.name_w),
        ("size", 7),
        ("%", 4),
    ];
    if lay.show_delta {
        header_cols.push(("Δ", 6));
    }
    let mut lines = vec![header_row(th, &header_cols)];

    if app.weight_loading {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", theme::spinner_frame(app.tick.wrapping_add(2))),
                Style::default().fg(th.accent_running),
            ),
            Span::styled("loading…", th.dim()),
        ]));
    } else if app.last.is_none() && app.weight_stack.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", theme::spinner_frame(app.tick.wrapping_add(2))),
                Style::default().fg(th.accent_running),
            ),
            Span::styled("waiting for sample…", th.dim()),
        ]));
    } else if n == 0 {
        lines.push(Line::from(Span::styled("  empty", th.dim())));
    } else {
        let end = (app.weight_scroll + vis).min(n);
        for (i, d) in children
            .iter()
            .enumerate()
            .take(end)
            .skip(app.weight_scroll)
        {
            let selected = focused && i == app.weight_sel;
            let is_dir = app.weight_cwd().join(&d.name).is_dir();
            let mark = if is_dir { theme::CHEVRON } else { " " };
            let dlt = if app.weight_stack.is_empty() {
                app.dir_delta_session(&d.name)
                    .or_else(|| app.dir_delta_interval(&d.name))
            } else {
                None
            };
            let mut spans = vec![
                Span::styled(
                    fmt::pad_left(&(i + 1).to_string(), 2),
                    if selected { th.bright() } else { th.dim() },
                ),
                Span::raw(" "),
                Span::styled(
                    mark.to_string(),
                    if is_dir {
                        Style::default().fg(th.accent)
                    } else {
                        th.dim()
                    },
                ),
                Span::styled(
                    fmt::pad_right(&fmt::truncate_middle(&d.name, lay.name_w), lay.name_w),
                    sel_style(th, selected),
                ),
                Span::raw(" "),
                Span::styled(
                    fmt::pad_left(&fmt::human_bytes_short(d.bytes), 7),
                    body_for(th, focused || selected),
                ),
                Span::raw(" "),
                Span::styled(
                    fmt::pad_left(&pct_label(d.bytes, total.max(1)), 4),
                    th.dim(),
                ),
            ];
            if lay.show_delta {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    fmt::pad_left(&opt_delta_bytes(dlt), 6),
                    dstyle(th, dlt),
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    let scroll = if n > vis.max(1) {
        format!(
            "  {}-{}/{}",
            app.weight_scroll + 1,
            (app.weight_scroll + vis).min(n),
            n
        )
    } else {
        String::new()
    };
    let crumb = app.weight_breadcrumb();
    // Prefer path breadcrumb + size; engine is secondary
    let title = if app.weight_stack.is_empty() {
        format!(
            "weight · {} · {engine}{scroll}",
            fmt::human_bytes_short(total)
        )
    } else {
        format!(
            "weight · {crumb} · {}{scroll}",
            fmt::human_bytes_short(total)
        )
    };
    f.render_widget(
        Paragraph::new(lines).block(panel(th, &title, focused, app.weight_loading)),
        area,
    );
}

// ── Git ────────────────────────────────────────────────────────────────────

/// Colored working-tree +/- (or dim `clean`).
fn working_tree_spans(th: &Theme, ins: u64, del: u64) -> Vec<Span<'static>> {
    if ins == 0 && del == 0 {
        return vec![Span::styled("clean".to_string(), th.dim())];
    }
    vec![
        Span::styled(
            format!("+{ins}"),
            Style::default()
                .fg(th.delta_up)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("−{del}"),
            Style::default()
                .fg(th.delta_down)
                .add_modifier(Modifier::BOLD),
        ),
    ]
}

fn draw_git(f: &mut Frame, area: Rect, app: &mut App, th: &Theme) {
    app.hit.git = area;
    app.hit.git_edge = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let focused = app.focus == Focus::Git;
    let vis = area.height.saturating_sub(3) as usize;
    let compact = area.height <= 4;

    let (title, lines) = match app.last.as_ref() {
        None => (
            "git".to_string(),
            vec![Line::from(vec![
                Span::styled(
                    format!("{} ", theme::spinner_frame(app.tick.wrapping_add(5))),
                    Style::default().fg(th.accent_running),
                ),
                Span::styled("collecting…", th.dim()),
            ])],
        ),
        Some(s) => {
            if let Some(g) = &s.git {
                let n = g.commits.len();
                let tree_spans = working_tree_spans(th, g.ins, g.del);

                if n == 0 || compact {
                    // Title carries branch; body is tree status only (no name repeat).
                    let title = format!("git · {}", g.branch);
                    let mut line = tree_spans;
                    if n == 0 {
                        line.push(dim_label(th, "  ·  no commits in log"));
                    } else {
                        line.push(dim_label(th, &format!("  · {n} commits")));
                    }
                    (title, vec![Line::from(line)])
                } else {
                    let scroll_hint = if n > vis.max(1) {
                        format!(
                            "  {}-{}/{}",
                            app.git_scroll + 1,
                            (app.git_scroll + vis).min(n),
                            n
                        )
                    } else {
                        String::new()
                    };
                    let title = format!("git · {} · {n}{scroll_hint}", g.branch);
                    let mut head = vec![Span::styled("working tree ".to_string(), th.dim())];
                    head.extend(working_tree_spans(th, g.ins, g.del));
                    let mut v = vec![Line::from(head)];

                    let end = (app.git_scroll + vis).min(n);
                    for i in app.git_scroll..end {
                        let c = &g.commits[i];
                        let selected = focused && i == app.git_sel;
                        // Responsive: drop author when narrow
                        let show_author = area.width >= 70;
                        let show_stat = area.width >= 48;
                        let author_w = if show_author { 12 } else { 0 };
                        let stat_w = if show_stat { 12 } else { 0 };
                        let fixed = 8 + 1 + stat_w + if show_author { 2 + author_w } else { 0 };
                        let subj_w = (area.width as usize).saturating_sub(fixed + 2).max(8);
                        let subj = fmt::truncate_middle(&c.subject, subj_w);
                        let mut row = vec![
                            Span::styled(
                                format!("{} ", c.short),
                                if selected {
                                    Style::default()
                                        .fg(th.accent)
                                        .bg(th.selection_bg)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(th.accent)
                                },
                            ),
                            Span::styled(
                                subj,
                                if selected {
                                    sel_style(th, true)
                                } else {
                                    th.body()
                                },
                            ),
                        ];
                        if show_stat && (c.ins > 0 || c.del > 0) {
                            row.push(Span::raw("  "));
                            row.push(Span::styled(
                                format!("+{}", c.ins),
                                Style::default()
                                    .fg(th.delta_up)
                                    .add_modifier(Modifier::BOLD),
                            ));
                            row.push(Span::raw(" "));
                            row.push(Span::styled(
                                format!("-{}", c.del),
                                Style::default()
                                    .fg(th.delta_down)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                        if show_author {
                            row.push(Span::raw("  "));
                            row.push(Span::styled(
                                fmt::truncate_middle(&c.author, author_w),
                                th.dim(),
                            ));
                        }
                        v.push(Line::from(row));
                    }
                    (title, v)
                }
            } else {
                let err = s.git_err.clone().unwrap_or_else(|| "unavailable".into());
                (
                    "git".to_string(),
                    vec![Line::from(Span::styled(err, th.dim()))],
                )
            }
        }
    };

    f.render_widget(
        Paragraph::new(lines).block(panel(th, &title, focused, false)),
        area,
    );
}

// ── Chrome ─────────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, area: Rect, app: &App, th: &Theme) {
    let focus = app.focus.label();
    let w = area.width as usize;
    // Progressive disclosure by terminal width
    let mut spans = vec![
        Span::styled(" q", Style::default().fg(th.accent)),
        Span::styled("uit", th.dim()),
        Span::styled("  r", Style::default().fg(th.accent)),
        Span::styled("efresh", th.dim()),
        Span::styled("  +/-", Style::default().fg(th.accent)),
        Span::styled(" interval", th.dim()),
        Span::styled("  tab", Style::default().fg(th.accent)),
        Span::styled(format!(":{focus}"), th.dim()),
    ];
    if w >= 70 {
        spans.extend([
            Span::styled("  j/k", Style::default().fg(th.accent)),
            Span::styled(" select", th.dim()),
            Span::styled("  wheel", Style::default().fg(th.accent)),
            Span::styled(" scroll", th.dim()),
        ]);
    }
    if w >= 95 {
        spans.extend([
            Span::styled("  drag", Style::default().fg(th.accent)),
            Span::styled(" resize", th.dim()),
            Span::styled("  enter", Style::default().fg(th.accent)),
            Span::styled(" open", th.dim()),
            Span::styled("  bs", Style::default().fg(th.accent)),
            Span::styled(" up", th.dim()),
        ]);
    }
    spans.extend([
        Span::styled("  ?", Style::default().fg(th.accent)),
        Span::styled(" help", th.dim()),
    ]);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help(f: &mut Frame, area: Rect, th: &Theme) {
    let text = "\
heim — compact project monitor\n\
\n\
  q / r / +/-     quit · refresh · interval\n\
  Tab             focus languages → weight → git\n\
  j/k             move selection (keeps row in view)\n\
  mouse wheel     scroll the list viewport\n\
  click           focus + select\n\
  drag ┊ / git top  resize panels\n\
  enter / dbl-click open weight dir\n\
  backspace / right-click  parent dir\n\
\n\
monitor  name · path · every Ns · age · live\n\
         totals once · Δ code over 5m/10m/30m/1h/2h\n\
weight   path-first · numeric % · › folders · cache\n\
git      auto-collapses when empty · height fits log";
    f.render_widget(
        Paragraph::new(text).style(th.bright()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(th.accent))
                .title(Span::styled(" help ", th.title()))
                .style(Style::default().bg(th.bg_base)),
        ),
        area,
    );
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::collect::{
        DirSize, GitStats, LangStat, LocStats, Sample, SizeBackendKind, SizeEngine,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime};

    fn sample_fixture() -> Sample {
        Sample {
            at: Instant::now(),
            wall: SystemTime::now(),
            size_bytes: 7_000_000_000,
            top_dirs: vec![
                DirSize {
                    name: "docs".into(),
                    bytes: 600_000,
                },
                DirSize {
                    name: "src".into(),
                    bytes: 400_000,
                },
                DirSize {
                    name: "very_long_directory_name_for_path_column".into(),
                    bytes: 100_000,
                },
            ],
            size_engine: SizeEngine::Dust,
            loc: Some(LocStats {
                files: 861,
                blank: 18_943,
                comment: 13_129,
                code: 121_958,
                langs: vec![
                    LangStat {
                        name: "Dart".into(),
                        blank: 1,
                        comment: 1,
                        code: 91_330,
                    },
                    LangStat {
                        name: "Markdown".into(),
                        blank: 1,
                        comment: 1,
                        code: 11_933,
                    },
                    LangStat {
                        name: "JSON".into(),
                        blank: 0,
                        comment: 0,
                        code: 5_048,
                    },
                ],
            }),
            loc_err: None,
            git: Some(GitStats {
                branch: "feature/ios-appmetrica-attribution".into(),
                ins: 0,
                del: 0,
                commits: vec![],
            }),
            git_err: None,
            duration: Duration::from_millis(800),
        }
    }

    fn render_at(w: u16, h: u16) -> String {
        let mut app = App::new(PathBuf::from("/tmp/frontend"), 10, SizeBackendKind::Auto);
        app.apply_sample(sample_fixture());
        app.refreshing = false;
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            let _ = draw(f, &mut app);
        })
        .unwrap();
        let buf = term.backend().buffer();
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn layout_wide_no_dupe_totals_and_collapsed_git() {
        let s = render_at(120, 30);
        // Insight should not restate full code total line as "code 121,958 lines"
        assert!(
            !s.contains("code 121,958 lines"),
            "summary must not restate code total: {s}"
        );
        // Totals appear in metrics row
        assert!(
            s.contains("121,958") || s.contains("121958"),
            "expected code total in buffer:\n{s}"
        );
        // Git collapsed empty state
        assert!(
            s.contains("no commits") || s.contains("clean"),
            "expected git empty/clean state:\n{s}"
        );
        // Path-ish long name should appear (or truncated with ellipsis)
        assert!(
            s.contains("very_long") || s.contains("…") || s.contains("directory"),
            "path column should surface long names:\n{s}"
        );
    }

    #[test]
    fn layout_narrow_still_renders() {
        let s = render_at(60, 20);
        assert!(s.contains("frontend") || s.contains("monitor"), "{s}");
        assert!(s.contains("languages") || s.contains("weight"), "{s}");
    }

    #[test]
    fn flex_name_grows_with_width() {
        let narrow = layout_weight(40, false);
        assert!(narrow.name_w >= 8, "name_w={}", narrow.name_w);
        let wide = layout_weight(100, false);
        assert!(wide.name_w > narrow.name_w, "wide name should grow");
    }

    #[test]
    fn bench_draw() {
        if std::env::var("HEIM_BENCH").is_err() {
            return;
        }
        let root = PathBuf::from(std::env::var("HEIM_BENCH_DIR").unwrap_or("/tmp".into()));
        let mut app = App::new(root.clone(), 60, SizeBackendKind::Walk);
        let mut s = sample_fixture();
        // realistic: 20 weight children that exist on disk, 16 langs, 100 commits
        let mut kids = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten().take(20) {
                kids.push(DirSize {
                    name: e.file_name().to_string_lossy().into_owned(),
                    bytes: 1234,
                });
            }
        }
        s.top_dirs = kids;
        if let Some(l) = s.loc.as_mut() {
            l.langs = (0..16)
                .map(|i| LangStat {
                    name: format!("Lang{i}"),
                    blank: 10,
                    comment: 20,
                    code: 1000 - i,
                })
                .collect();
        }
        if let Some(g) = s.git.as_mut() {
            g.commits = (0..100)
                .map(|i| crate::collect::GitCommit {
                    short: format!("abc{i:04}"),
                    subject: format!("feat: some reasonably long commit subject number {i}"),
                    author: "developer".into(),
                    ins: 12,
                    del: 3,
                })
                .collect();
            g.ins = 40;
            g.del = 12;
        }
        app.apply_sample(s);
        // history size from env
        let hist: usize = std::env::var("HEIM_HIST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        app.history.clear();
        for i in 0..hist {
            let mut h = sample_fixture();
            h.wall = SystemTime::now() - Duration::from_secs((hist - i) as u64 * 10);
            h.git = None;
            h.top_dirs = vec![];
            app.history.push_back(h);
        }
        app.refreshing = false;
        let backend = TestBackend::new(120, 40);
        let mut term = Terminal::new(backend).unwrap();
        // warm
        for _ in 0..5 {
            term.draw(|f| {
                let _ = draw(f, &mut app);
            })
            .unwrap();
        }
        let n = 200;
        let t0 = Instant::now();
        for _ in 0..n {
            app.bump_tick();
            term.draw(|f| {
                let _ = draw(f, &mut app);
            })
            .unwrap();
        }
        let per = t0.elapsed() / n;
        eprintln!("draw: {per:?}/frame  (hist={hist})");
        use std::sync::atomic::Ordering::Relaxed;
        let a0 = crate::ALLOCS.load(Relaxed);
        let b0 = crate::BYTES.load(Relaxed);
        for _ in 0..10 {
            app.bump_tick();
            term.draw(|f| {
                let _ = draw(f, &mut app);
            })
            .unwrap();
        }
        eprintln!(
            "allocs/frame: {}  bytes/frame: {}",
            (crate::ALLOCS.load(Relaxed) - a0) / 10,
            (crate::BYTES.load(Relaxed) - b0) / 10
        );

        // isolate: window deltas only
        let t1 = Instant::now();
        for _ in 0..n {
            std::hint::black_box(app.code_window_deltas());
        }
        eprintln!(
            "code_window_deltas: {:?}/call (hist={hist})",
            t1.elapsed() / n
        );

        // isolate: is_dir stats for 20 rows
        let t2 = Instant::now();
        for _ in 0..n {
            for d in &app.weight_children {
                std::hint::black_box(app.weight_cwd().join(&d.name).is_dir());
            }
        }
        eprintln!(
            "is_dir x{} rows: {:?}/frame",
            app.weight_children.len(),
            t2.elapsed() / n
        );
    }

    #[test]
    fn dump_layouts_for_qa() {
        if std::env::var("HEIM_DUMP").is_err() {
            return;
        }
        for (w, h) in [(120u16, 28u16), (80, 22), (60, 18)] {
            println!("\n======== empty-git {w}x{h} ========");
            print!("{}", render_at(w, h));
        }
        // With commits: git panel should expand
        let mut app = App::new(PathBuf::from("/tmp/frontend"), 10, SizeBackendKind::Auto);
        let mut s = sample_fixture();
        if let Some(g) = s.git.as_mut() {
            g.commits = vec![
                crate::collect::GitCommit {
                    short: "a1b2c3d".into(),
                    subject: "feat: add login retry backoff".into(),
                    author: "dev".into(),
                    ins: 120,
                    del: 14,
                },
                crate::collect::GitCommit {
                    short: "d4e5f6a".into(),
                    subject: "fix: handle empty auth token".into(),
                    author: "dev".into(),
                    ins: 20,
                    del: 3,
                },
            ];
            g.ins = 12;
            g.del = 2;
        }
        app.apply_sample(s);
        let backend = TestBackend::new(100, 28);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            let _ = draw(f, &mut app);
        })
        .unwrap();
        let buf = term.backend().buffer();
        println!("\n======== with-commits 100x28 ========");
        for y in 0..28u16 {
            for x in 0..100u16 {
                print!("{}", buf[(x, y)].symbol());
            }
            println!();
        }
    }
}
