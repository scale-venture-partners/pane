use crate::app::{App, Mode, Tab};
use crate::detail::{self, DetailView};
use crate::gh::{Ci, Item, Kind, Label};
use chrono::Utc;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

pub(crate) const DIM: Color = Color::Indexed(245);
const SEL_BG: Color = Color::Indexed(238);
const WIDE_MIN: u16 = 110;
const LABEL_MAX: usize = 16;

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

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(crate) fn ci_span(ci: Ci) -> Span<'static> {
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

pub(crate) fn gone_color(state: &str) -> Color {
    match state {
        "MERGED" => Color::Magenta,
        "CLOSED" => Color::Red,
        _ => Color::Yellow,
    }
}

fn truecolor() -> bool {
    static CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        matches!(
            std::env::var("COLORTERM").as_deref(),
            Ok("truecolor") | Ok("24bit")
        )
    })
}

/// Nearest entry in the xterm 6x6x6 color cube.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let level = |c: u8| ((c as u16 * 5 + 127) / 255) as u8;
    16 + 36 * level(r) + 6 * level(g) + level(b)
}

pub(crate) fn rgb_color((r, g, b): (u8, u8, u8)) -> Color {
    if truecolor() {
        Color::Rgb(r, g, b)
    } else {
        Color::Indexed(nearest_256(r, g, b))
    }
}

/// Black or white text, whichever is more readable on the label color.
pub fn label_fg((r, g, b): (u8, u8, u8)) -> Color {
    let lum = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    if lum > 140.0 {
        Color::Black
    } else {
        Color::White
    }
}

fn chip_text(l: &Label, max: usize) -> String {
    format!(" {} ", truncate(&l.name, max))
}

pub(crate) fn chip(l: &Label, max: usize) -> Span<'static> {
    Span::styled(
        chip_text(l, max),
        Style::new().fg(label_fg(l.rgb)).bg(rgb_color(l.rgb)),
    )
}

/// Chips that fit in `budget` columns, plus how many were left out.
fn fit_chips(labels: &[Label], budget: usize) -> (Vec<Span<'static>>, usize) {
    let mut out = vec![];
    let mut used = 0;
    for (n, l) in labels.iter().enumerate() {
        let w = chip_text(l, LABEL_MAX).chars().count() + 1;
        let more = labels.len() - n - 1;
        let reserve = if more > 0 { 4 } else { 0 };
        if used + w + reserve > budget {
            return (out, labels.len() - n);
        }
        out.push(chip(l, LABEL_MAX));
        out.push(Span::raw(" "));
        used += w;
    }
    (out, 0)
}

fn item_line(
    i: &Item,
    fresh: bool,
    width: usize,
    show_author: bool,
    busy: Option<&'static str>,
) -> Line<'static> {
    let mut spans = vec![if let Some(glyph) = busy {
        Span::styled(glyph, Color::Magenta)
    } else if fresh {
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
    let avail = width.saturating_sub(used + 1);
    let budget = if width >= 70 { (avail / 3).min(40) } else { 0 };
    let (mut chips, hidden) = fit_chips(&i.labels, budget);
    if hidden > 0 {
        chips.push(Span::styled(format!("+{hidden}"), DIM));
    }
    let chips_w: usize = chips.iter().map(|c| c.content.chars().count()).sum();
    let title_room = avail.saturating_sub(chips_w);
    let title = truncate(&i.title, title_room);
    let pad = width.saturating_sub(used + title.chars().count() + chips_w);
    spans.push(Span::styled(num, Color::Blue));
    let title_style = if i.draft || i.gone.is_some() {
        Style::new().fg(DIM).italic()
    } else {
        Style::new()
    };
    spans.push(Span::styled(title, title_style));
    if !chips.is_empty() {
        spans.push(Span::raw(" "));
    }
    spans.extend(chips);
    spans.push(Span::raw(
        " ".repeat(pad.saturating_sub(usize::from(chips_w > 0))),
    ));
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
    let mut right = vec![];
    let running = app.running_count();
    if running > 0 {
        right.push(Span::styled(
            format!("{} {running} claude   ", app.spinner()),
            Color::Magenta,
        ));
    }
    right.push(sync);
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
        let busy = app.is_busy(&i.id).then(|| app.spinner());
        rows.push(ListItem::new(item_line(i, fresh, width, show_author, busy)));
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

pub(crate) fn badge(text: String, color: Color) -> Span<'static> {
    Span::styled(format!(" {text} "), Style::new().fg(Color::Black).bg(color))
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
        detail::build(
            &item,
            d,
            diff,
            err,
            inner.width as usize,
            app.runs.get(&item.id).map(|r| (r, app.spinner())),
        )
    } else if let Some(e) = app.detail_err.get(&item.id) {
        DetailView::plain(Text::from(vec![Line::styled(
            format!("failed to load: {e}"),
            Color::Red,
        )]))
    } else {
        DetailView::plain(Text::styled(format!("{} loading…", app.spinner()), DIM))
    };

    // Everything before the diff may wrap; diff rows are pre-fitted to one row each, so
    // offsets inside it are plain line arithmetic rather than a wrap pass over thousands of lines.
    let lines = &view.text.lines;
    let (total, diff_row) = match view.diff_line {
        Some(dl) => {
            let before = rows_before(lines, dl, inner.width);
            (before + (lines.len() - dl) as u16, before)
        }
        None => (
            Paragraph::new(view.text.clone())
                .wrap(Wrap { trim: false })
                .line_count(inner.width) as u16,
            0,
        ),
    };
    app.detail_scroll = app.detail_scroll.min(total.saturating_sub(inner.height));
    app.diff_row = diff_row;
    app.file_rows = match view.diff_line {
        Some(dl) => view
            .file_lines
            .iter()
            .map(|l| diff_row + (*l - dl) as u16)
            .collect(),
        None => vec![],
    };

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
        Mode::Claude => {
            let room = (area.width as usize).saturating_sub(34);
            let shown = if app.input.chars().count() > room {
                let skip = app.input.chars().count() - room;
                format!("…{}", app.input.chars().skip(skip + 1).collect::<String>())
            } else {
                app.input.clone()
            };
            Line::from(vec![
                Span::styled(" claude› ", Style::new().bold().fg(Color::Magenta)),
                Span::raw(shown),
                Span::styled("█", DIM),
                Span::styled("   enter run · esc cancel", DIM),
            ])
        }
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
                    ("i", "mine"),
                ];
                if app.tab == Tab::Prs {
                    p.extend([("a", "approve"), ("m", "merge"), ("r", "repair")]);
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
        ("home end", "first / last"),
        ("enter l →", "open details"),
        ("esc h ←", "close details / clear filter"),
        ("space b", "scroll details (page)"),
        ("tab 1 2", "switch PRs / issues"),
        ("/", "filter by text"),
        ("i", "only my items"),
        ("u  F5", "update now"),
        ("o", "open in browser"),
        ("y", "copy URL"),
        ("f", "open first failing check"),
        ("click", "a check to open it"),
        ("d", "jump to diff (PR details)"),
        ("] [", "next / previous file in diff"),
        ("a", "approve PR (confirm)"),
        ("m", "merge PR (confirm)"),
        ("w", "toggle draft (confirm)"),
        ("x", "close (confirm)"),
        ("c", "comment"),
        ("r", "repair: run Claude on the PR"),
        ("q", "quit"),
        ("", "letters are not case sensitive"),
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
    use crate::app::RunState;
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
            labels: vec![],
            head: String::new(),
            base: String::new(),
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
        assert!(out.contains("repair"));
        assert!(!out.contains("C  "));
    }

    fn detail_with_checks() -> crate::gh::Detail {
        use crate::gh::{Check, Detail};
        Detail {
            state: "OPEN".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
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

    fn sample_diff(adds: usize) -> String {
        let mut d =
            format!("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,0 +1,{adds} @@\n");
        for n in 0..adds {
            d.push_str(&format!("+line {n}\n"));
        }
        d
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

    #[tokio::test]
    async fn checks_with_urls_are_clickable() {
        let mut app = app_with_detail(&sample_diff(1));
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
        let diff = sample_diff(80);
        let mut app = app_with_detail(&diff);
        let top = render(&mut app, 130, 30);
        assert!(!top.contains("line 79"));
        assert!(app.diff_row > 0);
        app.detail_scroll = u16::MAX;
        let bottom = render(&mut app, 130, 30);
        assert!(bottom.contains("line 79"));
    }

    #[tokio::test]
    async fn clicks_follow_scrolling() {
        let diff = sample_diff(80);
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

    fn labeled(n: u64, names: &[&str]) -> Item {
        let mut i = sample(n, "tool", Ci::Success);
        i.labels = names
            .iter()
            .map(|n| Label {
                name: n.to_string(),
                rgb: (0xd7, 0x3a, 0x4a),
            })
            .collect();
        i
    }

    #[test]
    fn nearest_256_maps_corners() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
    }

    #[test]
    fn label_text_contrasts_with_background() {
        assert_eq!(label_fg((255, 255, 0)), Color::Black);
        assert_eq!(label_fg((0, 0, 128)), Color::White);
    }

    #[test]
    fn chips_that_do_not_fit_are_counted() {
        let labels: Vec<Label> = ["alpha", "beta", "gamma", "delta"]
            .iter()
            .map(|n| Label {
                name: n.to_string(),
                rgb: (1, 2, 3),
            })
            .collect();
        let (spans, hidden) = fit_chips(&labels, 20);
        assert!(hidden > 0);
        assert!(spans.len() / 2 + hidden == 4);
        let (all, none) = fit_chips(&labels, 200);
        assert_eq!(none, 0);
        assert_eq!(all.len(), 8);
        assert_eq!(fit_chips(&labels, 0).0.len(), 0);
    }

    #[test]
    fn row_shows_labels_without_exceeding_width() {
        let item = labeled(1, &["bug", "needs-review", "a-very-long-label-name-indeed"]);
        for width in [60usize, 80, 120] {
            let line = item_line(&item, false, width, true, None);
            assert!(line.width() <= width, "width {width}: {}", line.width());
        }
        let wide = item_line(&item, false, 120, true, None);
        let text: String = wide.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("bug"));
        assert!(text.contains("Add feature number 1"));
    }

    #[test]
    fn narrow_rows_skip_labels() {
        let line = item_line(&labeled(1, &["bug"]), false, 50, true, None);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(!text.contains("bug"));
    }

    #[tokio::test]
    async fn detail_shows_label_chips() {
        let mut app = app_with_detail(&sample_diff(1));
        app.details.get_mut("id1").unwrap().labels = vec![Label {
            name: "needs-review".into(),
            rgb: (1, 2, 3),
        }];
        let out = render(&mut app, 130, 30);
        assert!(out.contains("needs-review"));
    }

    #[tokio::test]
    async fn claude_prompt_line_shows_the_editable_prompt() {
        let mut app = app_with_items();
        app.mode = Mode::Claude;
        app.input = "rebase and fix checks".into();
        let out = render(&mut app, 120, 20);
        assert!(out.contains("claude› rebase and fix checks"));
        assert!(out.contains("enter run"));
    }

    #[tokio::test]
    async fn long_prompts_keep_the_cursor_end_visible() {
        let mut app = app_with_items();
        app.mode = Mode::Claude;
        app.input = format!("{} THE-END", "word ".repeat(60));
        let out = render(&mut app, 80, 20);
        assert!(out.contains("THE-END"));
    }

    fn run_for(prompt: &str, state: RunState) -> crate::app::ClaudeRun {
        crate::app::ClaudeRun {
            repo: "tool".into(),
            number: 1,
            prompt: prompt.into(),
            started: std::time::Instant::now(),
            finished: None,
            state,
            log: "/tmp/run.log".into(),
            kept_worktree: None,
            summary: None,
        }
    }

    #[tokio::test]
    async fn running_claude_shows_at_the_top_of_the_pr_view() {
        let mut app = app_with_detail(&sample_diff(2));
        app.runs.insert(
            "id1".into(),
            run_for("rebase and fix checks", RunState::Running),
        );
        let out = render(&mut app, 130, 40);
        assert!(out.contains("Claude"));
        assert!(out.contains("running"));
        assert!(out.contains("› rebase and fix checks"));
        assert!(out.contains("1 claude"), "header counts running jobs");
        let claude_at = out.find("Claude").unwrap();
        let checks_at = out.find("Checks").unwrap();
        assert!(
            claude_at < checks_at,
            "Claude section sits above the checks"
        );
    }

    #[tokio::test]
    async fn finished_claude_shows_its_reply_log_and_leftover_work() {
        let mut app = app_with_detail(&sample_diff(2));
        let mut run = run_for("rebase", RunState::Done(0));
        run.finished = Some(std::time::Instant::now());
        run.summary = Some(
            "The rebase is stopped partway with one conflict in `CHANGELOG.md`.\n\n- keep both sections".into(),
        );
        run.kept_worktree = Some("/repo/.claude/worktrees/w".into());
        app.runs.insert("id1".into(), run);
        let out = render(&mut app, 130, 50);
        assert!(out.contains("done, work left in a worktree"));
        assert!(out.contains("The rebase is stopped partway"));
        assert!(
            out.contains("• keep both sections"),
            "reply is rendered as markdown"
        );
        assert!(out.contains("log: /tmp/run.log"));
        assert!(out.contains("worktree kept: /repo/.claude/worktrees/w"));
    }

    #[tokio::test]
    async fn failed_and_failed_exit_states_are_explained() {
        let mut app = app_with_detail(&sample_diff(2));
        app.runs.insert(
            "id1".into(),
            run_for("x", RunState::Failed("no local clone of o/r found".into())),
        );
        let out = render(&mut app, 130, 40);
        assert!(out.contains("could not run"));
        assert!(out.contains("no local clone of o/r found"));

        app.runs
            .insert("id1".into(), run_for("x", RunState::Done(3)));
        let out = render(&mut app, 130, 40);
        assert!(out.contains("exited 3"));
        assert!(out.contains("claude printed nothing"));
    }

    #[tokio::test]
    async fn long_replies_are_cut_with_a_pointer_to_the_log() {
        let mut app = app_with_detail(&sample_diff(2));
        let mut run = run_for("x", RunState::Done(0));
        run.summary = Some((0..60).map(|n| format!("line number {n}\n\n")).collect());
        app.runs.insert("id1".into(), run);
        app.detail_scroll = 0;
        let out = render(&mut app, 130, 60);
        assert!(out.contains("rest of the reply is in the log"));
        assert!(!out.contains("line number 59"));
    }

    #[tokio::test]
    async fn other_prs_do_not_show_the_section() {
        let mut app = app_with_detail(&sample_diff(2));
        app.runs
            .insert("some-other-pr".into(), run_for("x", RunState::Running));
        let out = render(&mut app, 130, 40);
        assert!(!out.contains("── Claude"));
    }

    #[tokio::test]
    async fn help_has_no_runs_list_key() {
        let mut app = app_with_items();
        app.mode = Mode::Help;
        let out = render(&mut app, 100, 30);
        assert!(!out.contains("list Claude runs"));
    }
}
