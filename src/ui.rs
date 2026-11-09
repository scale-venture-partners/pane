use crate::app::{App, Mode, Tab};
use crate::gh::{Ci, Detail, Item, Kind};
use chrono::Utc;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

const DIM: Color = Color::Indexed(245);
const SEL_BG: Color = Color::Indexed(238);
const WIDE_MIN: u16 = 110;

pub fn age(item_time: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> String {
    let secs = (now - item_time).num_seconds().max(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=172_799 => format!("{}h", secs / 3600),
        _ if secs < 86_400 * 100 => format!("{}d", secs / 86_400),
        _ => format!("{}mo", secs / (86_400 * 30)),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn ci_span(ci: Ci) -> Span<'static> {
    match ci {
        Ci::Success => Span::styled("●", Color::Green),
        Ci::Failure => Span::styled("●", Color::Red),
        Ci::Pending => Span::styled("●", Color::Yellow),
        Ci::None => Span::styled("·", DIM),
    }
}

fn review_span(i: &Item) -> Span<'static> {
    if i.draft {
        return Span::styled("◇", DIM);
    }
    match i.review.as_deref() {
        Some("APPROVED") => Span::styled("✓", Color::Green),
        Some("CHANGES_REQUESTED") => Span::styled("✗", Color::Red),
        Some("REVIEW_REQUIRED") => Span::styled("○", Color::Yellow),
        _ => Span::raw(" "),
    }
}

fn gone_color(state: &str) -> Color {
    match state {
        "MERGED" => Color::Magenta,
        "CLOSED" => Color::Red,
        _ => Color::Yellow,
    }
}

fn item_line(i: &Item, fresh: bool, width: usize, show_author: bool) -> Line<'static> {
    let mut spans = vec![if fresh {
        Span::styled("◆", Color::Magenta)
    } else {
        Span::raw(" ")
    }];
    if let Some(state) = &i.gone {
        spans.push(Span::styled(format!("{state} "), gone_color(state)));
    } else if i.kind == Kind::Pr {
        spans.push(ci_span(i.ci));
        spans.push(Span::raw(" "));
        spans.push(review_span(i));
        spans.push(Span::raw(" "));
    }
    let num = format!("#{:<5}", i.number);
    let right = if show_author {
        format!("  {} · {:>4}", i.author, age(i.updated_at, Utc::now()))
    } else {
        format!("  {:>4}", age(i.updated_at, Utc::now()))
    };
    let used: usize = spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>()
        + num.chars().count()
        + right.chars().count();
    let title = truncate(&i.title, width.saturating_sub(used + 1));
    let pad = width.saturating_sub(used + title.chars().count());
    spans.push(Span::styled(num, Color::Blue));
    let title_style = if i.draft || i.gone.is_some() {
        Style::new().fg(DIM).italic()
    } else {
        Style::new()
    };
    spans.push(Span::styled(title, title_style));
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right, DIM));
    Line::from(spans)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.wide = area.width >= WIDE_MIN;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(f, app, header);

    if app.detail_open && !app.wide {
        draw_detail(f, app, body);
    } else if app.detail_open {
        let [l, r] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Min(40)]).areas(body);
        draw_list(f, app, l);
        draw_detail(f, app, r);
    } else {
        draw_list(f, app, body);
    }
    draw_footer(f, app, footer);
    if matches!(app.mode, Mode::Help) {
        draw_help(f, area);
    }
}

fn draw_header(f: &mut Frame, app: &mut App, area: Rect) {
    let open = |v: &[Item]| v.iter().filter(|i| i.gone.is_none()).count();
    let tabs = [
        (Tab::Prs, "PRs", open(&app.prs)),
        (Tab::Issues, "Issues", open(&app.issues)),
    ];
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x + 1;
    for (n, (tab, name, count)) in tabs.iter().enumerate() {
        let label = format!(" {name} {count} ");
        let w = label.chars().count() as u16;
        app.tab_areas[n] = Rect {
            x,
            y: area.y,
            width: w,
            height: 1,
        };
        let style = if app.tab == *tab {
            Style::new().bold().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::new().fg(DIM)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    if app.mine {
        spans.push(Span::styled("mine ", Color::Cyan));
    }
    if !app.filter.is_empty() {
        spans.push(Span::styled(format!("/{} ", app.filter), Color::Cyan));
    }

    let sync = if app.loading {
        Span::styled("refreshing…", Color::Yellow)
    } else if let Some(err) = &app.fetch_error {
        Span::styled(format!("fetch failed: {}", truncate(err, 60)), Color::Red)
    } else if let Some(t) = app.last_ok {
        Span::styled(format!("updated {}s ago", t.elapsed().as_secs()), DIM)
    } else {
        Span::styled(format!("{} loading…", app.spinner()), Color::Yellow)
    };
    let mut right = vec![sync];
    if app.fetch_error.is_some() {
        if let Some(t) = app.last_ok {
            right.push(Span::styled(
                format!(" (data {}s old)", t.elapsed().as_secs()),
                DIM,
            ));
        }
    }
    if let Some(r) = app.rate {
        right.push(Span::styled(format!(" · api {r}"), DIM));
    }
    right.push(Span::raw(" "));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        area,
    );
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(DIM);
    let inner = block.inner(area);
    app.list_area = inner;
    let width = inner.width as usize;
    let show_author = width >= 60;

    let (mut rows, mut ids): (Vec<ListItem>, Vec<Option<String>>) = (vec![], vec![]);
    let mut sel_row = None;
    let mut last_repo = String::new();
    let visible = app.visible();
    let mut group_counts: std::collections::HashMap<&str, usize> = Default::default();
    for i in &visible {
        if i.gone.is_none() {
            *group_counts.entry(i.repo_full.as_str()).or_default() += 1;
        }
        group_counts.entry(i.repo_full.as_str()).or_default();
    }
    for i in &visible {
        if i.repo_full != last_repo {
            last_repo = i.repo_full.clone();
            let head = Line::from(vec![
                Span::styled(
                    format!(" {}", i.repo),
                    Style::new().bold().fg(Color::Yellow),
                ),
                Span::styled(format!("  {}", group_counts[i.repo_full.as_str()]), DIM),
            ]);
            rows.push(ListItem::new(head));
            ids.push(None);
        }
        if app.selected.as_deref() == Some(i.id.as_str()) {
            sel_row = Some(rows.len());
        }
        let fresh = app.fresh.contains(&i.id);
        rows.push(ListItem::new(item_line(i, fresh, width, show_author)));
        ids.push(Some(i.id.clone()));
    }
    app.row_ids = ids;
    app.list_state.select(sel_row);

    if rows.is_empty() {
        if app.last_ok.is_none() {
            draw_loading(f, app, area, block);
        } else {
            f.render_widget(
                Paragraph::new("nothing open")
                    .style(Style::new().fg(DIM))
                    .alignment(Alignment::Center)
                    .block(block),
                area,
            );
        }
        return;
    }
    let list = List::new(rows)
        .block(block)
        .highlight_style(Style::new().bg(SEL_BG).bold());
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn one_line(s: &str, max: usize) -> String {
    truncate(&s.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

fn loading_lines(app: &App) -> Vec<Line<'static>> {
    let spin = app.spinner();
    let mut out = vec![Line::styled(
        "Loading your pull requests and issues",
        Style::new().bold(),
    )];
    out.push(Line::raw(""));
    out.push(match (&app.viewer, &app.viewer_err) {
        (Some(v), _) => Line::from(vec![
            Span::styled("✓ ", Color::Green),
            Span::raw(format!("Signed in to GitHub as @{v}")),
        ]),
        (None, Some(e)) => Line::from(vec![
            Span::styled("✗ ", Color::Red),
            Span::styled(
                format!("gh is not signed in: {}", one_line(e, 48)),
                Color::Red,
            ),
        ]),
        (None, None) => Line::from(vec![
            Span::styled(format!("{spin} "), Color::Yellow),
            Span::raw("Checking GitHub sign-in via gh"),
        ]),
    });
    out.push(match (&app.fetch_error, app.progress) {
        (Some(_), _) if app.viewer_err.is_some() => Line::raw(""),
        (Some(e), _) => Line::from(vec![
            Span::styled("✗ ", Color::Red),
            Span::styled(
                format!("Could not load from GitHub: {}", one_line(e, 48)),
                Color::Red,
            ),
        ]),
        (None, Some((done, total))) => Line::from(vec![
            Span::styled(format!("{spin} "), Color::Yellow),
            Span::raw(format!(
                "Fetching from GitHub ({done}/{total} searches done)"
            )),
        ]),
        (None, None) => Line::from(vec![
            Span::styled(format!("{spin} "), Color::Yellow),
            Span::raw("Starting"),
        ]),
    });
    out.push(Line::raw(""));
    out.push(if app.fetch_error.is_some() {
        Line::styled(
            "Check `gh auth status`. pane retries automatically, or press r.",
            DIM,
        )
    } else {
        Line::styled(
            "The first load takes a few seconds; after that pane refreshes in the background.",
            DIM,
        )
    });
    out
}

fn draw_loading(f: &mut Frame, app: &App, area: Rect, block: Block<'_>) {
    let lines = loading_lines(app);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let widest = lines.iter().map(Line::width).max().unwrap_or(0) as u16;
    let width = widest.min(inner.width).min(80);
    let top = inner.height.saturating_sub(lines.len() as u16) / 2;
    let rect = Rect {
        x: inner.x + (inner.width - width) / 2,
        y: inner.y + top,
        width,
        height: inner.height - top,
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rect);
}

fn badge(text: String, color: Color) -> Span<'static> {
    Span::styled(format!(" {text} "), Style::new().fg(Color::Black).bg(color))
}

fn clean_body(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body.replace('\r', "");
    while let Some(s) = rest.find("<!--") {
        out.push_str(&rest[..s]);
        match rest[s..].find("-->") {
            Some(e) => rest = rest[s + e + 3..].to_string(),
            None => {
                rest.clear();
                break;
            }
        }
    }
    out.push_str(&rest);
    out.trim().to_string()
}

fn body_lines(body: &str) -> Vec<Line<'static>> {
    let mut in_code = false;
    clean_body(body)
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("```") {
                in_code = !in_code;
                return Line::styled(l.to_string(), DIM);
            }
            if in_code {
                Line::styled(l.to_string(), DIM)
            } else if l.starts_with('#') {
                Line::styled(
                    l.trim_start_matches('#').trim().to_string(),
                    Style::new().bold(),
                )
            } else {
                Line::raw(l.to_string())
            }
        })
        .collect()
}

fn section(title: &str) -> Line<'static> {
    Line::styled(title.to_string(), Style::new().bold().fg(Color::Cyan))
}

struct DetailView {
    text: Text<'static>,
    check_lines: Vec<(usize, String)>,
    diff_line: Option<usize>,
}

pub fn diff_lines(diff: &str) -> Vec<Line<'static>> {
    diff.lines()
        .map(|l| {
            let style = if l.starts_with("diff --git") {
                Style::new().bold().fg(Color::Yellow)
            } else if l.starts_with("+++") || l.starts_with("---") || l.starts_with("index ") {
                Style::new().fg(DIM)
            } else if l.starts_with("@@") {
                Style::new().fg(Color::Cyan)
            } else if l.starts_with('+') {
                Style::new().fg(Color::Green)
            } else if l.starts_with('-') {
                Style::new().fg(Color::Red)
            } else {
                Style::new()
            };
            Line::styled(l.to_string(), style)
        })
        .collect()
}

fn detail_text(item: &Item, d: &Detail, diff: Option<&str>, diff_err: Option<&str>) -> DetailView {
    let mut check_lines = vec![];
    let mut diff_line = None;
    let mut t: Vec<Line> = vec![
        Line::styled(item.title.clone(), Style::new().bold()),
        Line::raw(""),
    ];

    let mut meta = vec![Span::styled(format!("@{}", item.author), Color::Blue)];
    meta.push(Span::styled(
        match age(item.updated_at, Utc::now()).as_str() {
            "now" => " · updated just now".to_string(),
            a => format!(" · updated {a} ago"),
        },
        DIM,
    ));
    t.push(Line::from(meta));

    if let Some(state) = &item.gone {
        t.push(Line::raw(""));
        t.push(Line::from(badge(state.clone(), gone_color(state))));
    }

    if item.kind == Kind::Pr {
        t.push(Line::from(vec![
            Span::styled(format!("{} → {}", d.head, d.base), Color::Cyan),
            Span::styled(format!("  +{}", d.additions), Color::Green),
            Span::styled(format!(" −{}", d.deletions), Color::Red),
            Span::styled(format!("  {} files", d.files), DIM),
        ]));
        let mut badges = vec![];
        if item.draft {
            badges.push(badge("DRAFT".into(), Color::Indexed(245)));
        }
        if let Some(r) = &item.review {
            let c = match r.as_str() {
                "APPROVED" => Color::Green,
                "CHANGES_REQUESTED" => Color::Red,
                _ => Color::Yellow,
            };
            badges.push(badge(r.replace('_', " "), c));
        }
        let (mc, ml) = match d.mergeable.as_str() {
            "MERGEABLE" => (Color::Green, "MERGEABLE".to_string()),
            "CONFLICTING" => (Color::Red, "CONFLICTS".to_string()),
            _ => (Color::Yellow, "MERGE UNKNOWN".to_string()),
        };
        if item.gone.is_none() {
            badges.push(badge(ml, mc));
            if !d.merge_state.is_empty() && d.merge_state != "CLEAN" && d.merge_state != "UNKNOWN" {
                badges.push(badge(d.merge_state.clone(), Color::Yellow));
            }
        }
        let mut line = vec![];
        for b in badges {
            line.push(b);
            line.push(Span::raw(" "));
        }
        t.push(Line::raw(""));
        t.push(Line::from(line));
    }

    if !d.labels.is_empty() {
        t.push(Line::from(vec![
            Span::styled("labels  ", DIM),
            Span::raw(d.labels.join(", ")),
        ]));
    }
    if !d.assignees.is_empty() {
        t.push(Line::from(vec![
            Span::styled("assigned  ", DIM),
            Span::raw(d.assignees.join(", ")),
        ]));
    }

    if item.kind == Kind::Pr {
        if !d.requested.is_empty() || !d.reviews.is_empty() {
            t.push(Line::raw(""));
            t.push(section("Reviews"));
            for (who, state) in &d.reviews {
                let (sym, c) = match state.as_str() {
                    "APPROVED" => ("✓", Color::Green),
                    "CHANGES_REQUESTED" => ("✗", Color::Red),
                    _ => ("•", DIM),
                };
                t.push(Line::from(vec![
                    Span::styled(format!("{sym} "), c),
                    Span::raw(format!("{who} ")),
                    Span::styled(state.to_lowercase().replace('_', " "), DIM),
                ]));
            }
            for who in &d.requested {
                t.push(Line::from(vec![
                    Span::styled("○ ", Color::Yellow),
                    Span::raw(format!("{who} ")),
                    Span::styled("requested", DIM),
                ]));
            }
        }

        t.push(Line::raw(""));
        let passing = d.checks.iter().filter(|c| c.ci == Ci::Success).count();
        t.push(section(&format!(
            "Checks  {passing}/{} passing",
            d.checks.len()
        )));
        if d.checks.is_empty() {
            t.push(Line::styled("no checks reported", DIM));
        }
        let mut checks: Vec<_> = d.checks.iter().collect();
        checks.sort_by_key(|c| match c.ci {
            Ci::Failure => 0,
            Ci::Pending => 1,
            _ => 2,
        });
        for c in checks {
            let name_style = match c.url {
                Some(_) => Style::new().underlined(),
                None => Style::new(),
            };
            if let Some(u) = &c.url {
                check_lines.push((t.len(), u.clone()));
            }
            t.push(Line::from(vec![
                ci_span(c.ci),
                Span::styled(format!(" {}", c.name), name_style),
            ]));
        }
    }

    t.push(Line::raw(""));
    t.push(section("Description"));
    let body = body_lines(&d.body);
    if body.is_empty() {
        t.push(Line::styled("no description", DIM));
    }
    t.extend(body);

    if !d.comments.is_empty() {
        t.push(Line::raw(""));
        t.push(section("Recent comments"));
        for c in &d.comments {
            t.push(Line::raw(""));
            t.push(Line::from(vec![
                Span::styled(format!("@{}", c.author), Color::Blue),
                Span::styled(format!("  {}", c.created_at.get(..10).unwrap_or("")), DIM),
            ]));
            t.extend(body_lines(&c.body));
        }
    }
    if item.kind == Kind::Pr {
        t.push(Line::raw(""));
        diff_line = Some(t.len());
        t.push(section(&format!(
            "Diff  +{} −{} · {} files",
            d.additions, d.deletions, d.files
        )));
        match (diff, diff_err) {
            (Some(""), _) => t.push(Line::styled("empty diff", DIM)),
            (Some(text), _) => t.extend(diff_lines(text)),
            (None, Some(e)) => t.push(Line::styled(
                format!("failed to load diff: {e}"),
                Color::Red,
            )),
            (None, None) => t.push(Line::styled("loading diff…", Color::Yellow)),
        }
    }
    DetailView {
        text: Text::from(t),
        check_lines,
        diff_line,
    }
}

fn rows_before(lines: &[Line<'static>], upto: usize, width: u16) -> u16 {
    if upto == 0 {
        return 0;
    }
    let p = Paragraph::new(Text::from(lines[..upto].to_vec())).wrap(Wrap { trim: false });
    p.line_count(width) as u16
}

fn draw_detail(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(item) = app.selected_item() else {
        f.render_widget(Block::bordered().border_type(BorderType::Rounded), area);
        return;
    };
    let kind = if item.kind == Kind::Pr { "PR" } else { "Issue" };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Color::Yellow)
        .title(Line::from(vec![
            Span::styled(format!(" {kind} "), Style::new().bold()),
            Span::styled(format!("{}#{} ", item.repo_full, item.number), Color::Blue),
        ]));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.check_hits.clear();

    let view = if let Some(d) = app.details.get(&item.id) {
        let diff = app.diffs.get(&item.id).map(|(_, t)| t.as_str());
        let err = app.diff_err.get(&item.id).map(String::as_str);
        detail_text(&item, d, diff, err)
    } else if let Some(e) = app.detail_err.get(&item.id) {
        DetailView {
            text: Text::from(vec![Line::styled(
                format!("failed to load: {e}"),
                Color::Red,
            )]),
            check_lines: vec![],
            diff_line: None,
        }
    } else {
        DetailView {
            text: Text::styled(format!("{} loading…", app.spinner()), DIM),
            check_lines: vec![],
            diff_line: None,
        }
    };

    let total = Paragraph::new(view.text.clone())
        .wrap(Wrap { trim: false })
        .line_count(inner.width) as u16;
    app.detail_scroll = app.detail_scroll.min(total.saturating_sub(inner.height));
    app.diff_row = view
        .diff_line
        .map_or(0, |l| rows_before(&view.text.lines, l, inner.width));

    for (line, url) in &view.check_lines {
        let row = rows_before(&view.text.lines, *line, inner.width);
        if row >= app.detail_scroll && row - app.detail_scroll < inner.height {
            let rect = Rect {
                x: inner.x,
                y: inner.y + row - app.detail_scroll,
                width: inner.width,
                height: 1,
            };
            app.check_hits.push((rect, url.clone()));
        }
    }

    let p = Paragraph::new(view.text)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0));
    f.render_widget(p, inner);
}

fn keys(pairs: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut v = vec![];
    for (k, d) in pairs {
        v.push(Span::styled(
            format!(" {k}"),
            Style::new().bold().fg(Color::Yellow),
        ));
        v.push(Span::styled(format!(" {d} "), DIM));
    }
    v
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let line = match &app.mode {
        Mode::Filter => Line::from(vec![
            Span::styled(" /", Color::Cyan),
            Span::raw(app.filter.clone()),
            Span::styled("█", DIM),
            Span::styled("   enter keep · esc clear", DIM),
        ]),
        Mode::Comment => Line::from(vec![
            Span::styled(" comment: ", Color::Cyan),
            Span::raw(app.input.clone()),
            Span::styled("█", DIM),
            Span::styled("   enter send · esc cancel", DIM),
        ]),
        Mode::Confirm(op) => {
            let what = match op {
                crate::gh::Op::Approve => "Approve".to_string(),
                crate::gh::Op::Merge(m) => format!("Merge ({m})"),
                crate::gh::Op::Close => "Close".to_string(),
                crate::gh::Op::Ready => "Mark ready for review".to_string(),
                crate::gh::Op::Draft => "Convert to draft".to_string(),
                crate::gh::Op::Comment(_) => "Comment".to_string(),
            };
            let target = app
                .selected_item()
                .map(|i| format!("{}#{}", i.repo, i.number))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(
                    format!(" {what} {target}? "),
                    Style::new().bold().fg(Color::Yellow),
                ),
                Span::styled("y/N", DIM),
            ])
        }
        _ => match &app.status {
            Some(s) => Line::styled(
                format!(" {}", truncate(&s.text, area.width as usize - 2)),
                if s.error { Color::Red } else { Color::Green },
            ),
            None => {
                let mut p = vec![
                    ("↑↓", "move"),
                    ("enter", "open"),
                    ("o", "browser"),
                    ("/", "filter"),
                    ("m", "mine"),
                ];
                if app.tab == Tab::Prs {
                    p.extend([("a", "approve"), ("M", "merge")]);
                }
                p.extend([("c", "comment"), ("?", "help")]);
                Line::from(keys(&p))
            }
        },
    };
    f.render_widget(Paragraph::new(line), area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let rows = [
        ("↑↓ j k", "move selection"),
        ("g G", "first / last"),
        ("enter l →", "open details"),
        ("esc h ←", "close details / clear filter"),
        ("space b", "scroll details (page)"),
        ("tab 1 2", "switch PRs / issues"),
        ("/", "filter by text"),
        ("m", "only my items"),
        ("r", "refresh now"),
        ("o", "open in browser"),
        ("y", "copy URL"),
        ("f", "open first failing check"),
        ("click", "a check to open it"),
        ("d", "jump to diff (PR details)"),
        ("a", "approve PR (confirm)"),
        ("M", "merge PR (confirm)"),
        ("D", "toggle draft (confirm)"),
        ("x", "close (confirm)"),
        ("c", "comment"),
        ("q", "quit"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!(" {k:<10}"), Style::new().bold().fg(Color::Yellow)),
                Span::raw(*d),
            ])
        })
        .collect();
    let h = (lines.len() as u16 + 2).min(area.height);
    let w = 48.min(area.width);
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Color::Yellow)
                .title(" keys "),
        ),
        r,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::gh::Snapshot;
    use chrono::Duration;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    fn sample(n: u64, repo: &str, ci: Ci) -> Item {
        Item {
            id: format!("id{n}"),
            kind: Kind::Pr,
            number: n,
            title: format!("Add feature number {n}"),
            url: format!("https://github.com/o/{repo}/pull/{n}"),
            repo: repo.into(),
            repo_full: format!("o/{repo}"),
            author: "will".into(),
            updated_at: Utc::now() - Duration::hours(n as i64),
            draft: n == 3,
            review: Some("APPROVED".into()),
            ci,
            gone: None,
        }
    }

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app_with_items() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(tokio::sync::Notify::new()));
        app.update(crate::app::Msg::List(Ok(Snapshot {
            prs: vec![
                sample(1, "tool", Ci::Success),
                sample(2, "tool", Ci::Failure),
                sample(3, "engine", Ci::Pending),
            ],
            issues: vec![],
            rate_remaining: Some(4900),
        })));
        app
    }

    #[test]
    fn age_buckets() {
        let now = Utc::now();
        assert_eq!(age(now - Duration::seconds(30), now), "now");
        assert_eq!(age(now - Duration::minutes(5), now), "5m");
        assert_eq!(age(now - Duration::hours(5), now), "5h");
        assert_eq!(age(now - Duration::days(4), now), "4d");
    }

    #[tokio::test]
    async fn list_renders_groups_and_rows() {
        let mut app = app_with_items();
        let out = render(&mut app, 90, 12);
        assert!(out.contains("tool"));
        assert!(out.contains("engine"));
        assert!(out.contains("#2"));
        assert!(out.contains("PRs 3"));
    }

    #[tokio::test]
    async fn narrow_detail_replaces_list() {
        let mut app = app_with_items();
        app.detail_open = true;
        let out = render(&mut app, 80, 12);
        assert!(out.contains("PR"));
        assert!(out.contains("loading"));
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    #[test]
    fn body_comments_are_stripped() {
        assert_eq!(clean_body("a <!-- hidden --> b\r\n"), "a  b");
        assert_eq!(clean_body("x <!-- never closed"), "x");
    }

    #[tokio::test]
    async fn wide_layout_shows_list_and_detail() {
        let mut app = app_with_items();
        app.detail_open = true;
        let out = render(&mut app, 130, 14);
        assert!(out.contains("engine"));
        assert!(out.contains("loading"));
    }

    #[tokio::test]
    async fn help_overlay_lists_keys() {
        let mut app = app_with_items();
        app.mode = Mode::Help;
        let out = render(&mut app, 100, 30);
        assert!(out.contains("approve PR"));
    }

    fn detail_with_checks() -> crate::gh::Detail {
        use crate::gh::{Check, Detail};
        Detail {
            state: "OPEN".into(),
            body: "hello".into(),
            head: "feature".into(),
            base: "main".into(),
            mergeable: "MERGEABLE".into(),
            merge_state: "CLEAN".into(),
            additions: 3,
            deletions: 1,
            files: 1,
            labels: vec![],
            assignees: vec![],
            requested: vec![],
            reviews: vec![],
            checks: vec![
                Check {
                    name: "lint".into(),
                    ci: Ci::Failure,
                    url: Some("https://ci/lint".into()),
                },
                Check {
                    name: "test".into(),
                    ci: Ci::Success,
                    url: Some("https://ci/test".into()),
                },
                Check {
                    name: "no-link".into(),
                    ci: Ci::Success,
                    url: None,
                },
            ],
            comments: vec![],
            fetched: std::time::Instant::now(),
        }
    }

    fn app_with_detail(diff: &str) -> App {
        let mut app = app_with_items();
        app.selected = Some("id1".into());
        app.detail_open = true;
        app.details.insert("id1".into(), detail_with_checks());
        app.diffs
            .insert("id1".into(), (std::time::Instant::now(), diff.into()));
        app
    }

    #[test]
    fn diff_lines_are_colored_by_kind() {
        let lines = diff_lines("diff --git a/x b/x\n@@ -1 +1 @@\n-old\n+new\n ctx");
        let colors: Vec<_> = lines.iter().map(|l| l.style.fg).collect();
        assert_eq!(
            colors,
            [
                Some(Color::Yellow),
                Some(Color::Cyan),
                Some(Color::Red),
                Some(Color::Green),
                None
            ]
        );
    }

    #[tokio::test]
    async fn checks_with_urls_are_clickable() {
        let mut app = app_with_detail("+x");
        let out = render(&mut app, 130, 40);
        assert!(out.contains("lint"));
        assert_eq!(app.check_hits.len(), 2);
        let (rect, url) = app.check_hits[0].clone();
        assert_eq!(url, "https://ci/lint");
        let pos = ratatui::layout::Position {
            x: rect.x + 2,
            y: rect.y,
        };
        assert_eq!(app.check_url_at(pos).as_deref(), Some("https://ci/lint"));
        let row_text: String = out.lines().nth(rect.y as usize).unwrap().to_string();
        assert!(row_text.contains("lint"), "hit row was: {row_text}");
    }

    #[tokio::test]
    async fn diff_is_at_the_bottom_and_reachable_by_scrolling() {
        let diff: String = (0..80).map(|n| format!("+line {n}\n")).collect();
        let mut app = app_with_detail(&diff);
        let top = render(&mut app, 130, 30);
        assert!(!top.contains("+line 79"));
        assert!(app.diff_row > 0);
        app.detail_scroll = u16::MAX;
        let bottom = render(&mut app, 130, 30);
        assert!(bottom.contains("+line 79"));
    }

    #[tokio::test]
    async fn clicks_follow_scrolling() {
        let diff: String = (0..80).map(|n| format!("+line {n}\n")).collect();
        let mut app = app_with_detail(&diff);
        render(&mut app, 130, 30);
        assert_eq!(app.check_hits.len(), 2);
        app.detail_scroll = app.diff_row;
        render(&mut app, 130, 30);
        assert!(app.check_hits.is_empty());
    }

    #[tokio::test]
    async fn diff_error_is_shown() {
        let mut app = app_with_detail("");
        app.diffs.clear();
        app.diff_err.insert("id1".into(), "too large".into());
        app.detail_scroll = u16::MAX;
        let out = render(&mut app, 130, 30);
        assert!(out.contains("failed to load diff: too large"));
    }

    fn fresh_app() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(Config::default(), tx, Arc::new(tokio::sync::Notify::new()))
    }

    #[tokio::test]
    async fn first_load_shows_loading_screen() {
        let mut app = fresh_app();
        let out = render(&mut app, 100, 20);
        assert!(out.contains("Loading your pull requests and issues"));
        assert!(out.contains("Checking GitHub sign-in"));
    }

    #[tokio::test]
    async fn loading_screen_reports_progress_and_auth() {
        let mut app = fresh_app();
        app.update(crate::app::Msg::Viewer(Ok("octocat".into())));
        app.update(crate::app::Msg::Progress(3, 8));
        let out = render(&mut app, 100, 20);
        assert!(out.contains("Signed in to GitHub as @octocat"));
        assert!(out.contains("3/8 searches done"));
    }

    #[tokio::test]
    async fn loading_screen_explains_fetch_failure() {
        let mut app = fresh_app();
        app.update(crate::app::Msg::Viewer(Ok("octocat".into())));
        app.update(crate::app::Msg::List(Err("HTTP 401".into())));
        let out = render(&mut app, 100, 20);
        assert!(out.contains("Could not load from GitHub: HTTP 401"));
        assert!(out.contains("gh auth status"));
    }

    #[tokio::test]
    async fn loading_screen_explains_missing_sign_in() {
        let mut app = fresh_app();
        app.update(crate::app::Msg::List(Err("HTTP 401".into())));
        app.update(crate::app::Msg::Viewer(Err("not logged in".into())));
        let out = render(&mut app, 100, 20);
        assert!(out.contains("gh is not signed in: not logged in"));
        assert!(!out.contains("Could not load"));
    }

    #[tokio::test]
    async fn spinner_advances_with_ticks() {
        let mut app = fresh_app();
        let a = app.spinner();
        app.update(crate::app::Msg::Tick);
        assert_ne!(a, app.spinner());
    }
}
