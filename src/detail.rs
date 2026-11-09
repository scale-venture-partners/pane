//! Builds the right-hand detail view for a PR or issue as styled lines.

use crate::app::{ClaudeRun, RunState};
use crate::diff;
use crate::gh::{Ci, Detail, Item, Kind};
use crate::markdown;
use crate::ui::{age, badge, chip, ci_span, gone_color, DIM, RABBIT};
use chrono::{DateTime, Utc};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};

pub struct DetailView {
    pub text: Text<'static>,
    pub check_lines: Vec<(usize, String)>,
    pub diff_line: Option<usize>,
    pub file_lines: Vec<usize>,
}

impl DetailView {
    pub fn plain(text: Text<'static>) -> Self {
        Self {
            text,
            check_lines: vec![],
            diff_line: None,
            file_lines: vec![],
        }
    }
}

fn ago(iso: &str) -> String {
    match DateTime::parse_from_rfc3339(iso) {
        Ok(t) => {
            let a = age(t.with_timezone(&Utc), Utc::now());
            if a == "now" {
                "just now".into()
            } else {
                format!("{a} ago")
            }
        }
        Err(_) => String::new(),
    }
}

/// `── Title  extra ─────────` filling `width`.
fn section(title: &str, extra: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut spans = vec![
        Span::styled("── ", DIM),
        Span::styled(title.to_string(), Style::new().bold().fg(Color::Cyan)),
    ];
    if !extra.is_empty() {
        spans.push(Span::raw("  "));
        spans.extend(extra);
    }
    spans.push(Span::raw(" "));
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::styled("─".repeat(width.saturating_sub(used)), DIM));
    Line::from(spans)
}

/// Splits `cells` into per-part cell counts that sum to `cells` exactly.
pub fn proportions(parts: &[usize], cells: usize) -> Vec<usize> {
    let total: usize = parts.iter().sum();
    if total == 0 {
        return vec![0; parts.len()];
    }
    let mut out = vec![];
    let (mut cum, mut prev) = (0usize, 0usize);
    for p in parts {
        cum += p;
        let edge = (cum * cells + total / 2) / total;
        out.push(edge - prev);
        prev = edge;
    }
    out
}

fn bar(parts: &[(usize, Color)], cells: usize, glyph: &str) -> Vec<Span<'static>> {
    let counts = proportions(&parts.iter().map(|p| p.0).collect::<Vec<_>>(), cells);
    parts
        .iter()
        .zip(counts)
        .map(|((_, c), n)| Span::styled(glyph.repeat(n), *c))
        .collect()
}

fn state_badge(item: &Item, d: &Detail) -> Span<'static> {
    let state = item.gone.clone().unwrap_or_else(|| d.state.clone());
    match state.as_str() {
        "OPEN" if item.draft => badge("DRAFT".into(), Color::Indexed(245)),
        "OPEN" => badge("OPEN".into(), Color::Green),
        other => badge(other.to_string(), gone_color(other)),
    }
}

fn check_counts(d: &Detail) -> (usize, usize, usize) {
    let count = |c: Ci| d.checks.iter().filter(|x| x.ci == c).count();
    (count(Ci::Success), count(Ci::Failure), count(Ci::Pending))
}

fn badges(item: &Item, d: &Detail) -> Line<'static> {
    let mut b = vec![state_badge(item, d)];
    if item.kind == Kind::Pr {
        if let Some(r) = &item.review {
            let c = match r.as_str() {
                "APPROVED" => Color::Green,
                "CHANGES_REQUESTED" => Color::Red,
                _ => Color::Yellow,
            };
            b.push(badge(r.replace('_', " "), c));
        }
        if let Some(r) = item.rabbit.filter(|r| r.unresolved > 0) {
            b.push(badge(format!("CodeRabbit {} open", r.unresolved), RABBIT));
        }
        if item.gone.is_none() {
            let (pass, fail, pending) = check_counts(d);
            if fail > 0 {
                b.push(badge(format!("✗ {fail} failing"), Color::Red));
            } else if pending > 0 {
                b.push(badge(format!("● {pending} running"), Color::Yellow));
            } else if pass > 0 {
                b.push(badge("✓ checks passed".into(), Color::Green));
            }
            match d.mergeable.as_str() {
                "CONFLICTING" => b.push(badge("CONFLICTS".into(), Color::Red)),
                "MERGEABLE" => {}
                _ => b.push(badge("MERGE UNKNOWN".into(), Color::Yellow)),
            }
            if !["", "CLEAN", "UNKNOWN", "UNSTABLE"].contains(&d.merge_state.as_str()) {
                b.push(badge(d.merge_state.clone(), Color::Yellow));
            }
        }
    }
    let mut spans = vec![];
    for x in b {
        spans.push(x);
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

fn branch_line(d: &Detail) -> Line<'static> {
    let chip = |s: &str| {
        Span::styled(
            format!(" {s} "),
            Style::new().fg(Color::Cyan).bg(Color::Indexed(237)),
        )
    };
    Line::from(vec![chip(&d.head), Span::styled(" → ", DIM), chip(&d.base)])
}

fn stats_line(d: &Detail) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("+{}", d.additions), Color::Green),
        Span::raw(" "),
        Span::styled(format!("−{}", d.deletions), Color::Red),
        Span::raw("  "),
    ];
    spans.extend(bar(
        &[
            (d.additions as usize, Color::Green),
            (d.deletions as usize, Color::Red),
        ],
        10,
        "■",
    ));
    spans.push(Span::styled(
        format!("  {} file{}", d.files, if d.files == 1 { "" } else { "s" }),
        DIM,
    ));
    Line::from(spans)
}

const REPLY_LINES: usize = 14;

fn short_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

/// The PR's Claude run: progress while it runs, Claude's reply and where to find the rest after.
fn claude_section(run: &ClaudeRun, spinner: &str, width: usize) -> Vec<Line<'static>> {
    let attention = run.kept_worktree.is_some();
    let status = match &run.state {
        RunState::Running => Span::styled(
            format!(
                "{spinner} running {}",
                short_duration(run.started.elapsed().as_secs())
            ),
            Color::Magenta,
        ),
        RunState::Done(0) if attention => {
            Span::styled("! done, work left in a worktree", Color::Yellow)
        }
        RunState::Done(0) => Span::styled("✓ done", Color::Green),
        RunState::Done(c) => Span::styled(format!("✗ exited {c}"), Color::Red),
        RunState::Failed(_) => Span::styled("✗ could not run", Color::Red),
    };
    let mut extra = vec![status];
    if let Some(done) = run.finished {
        extra.push(Span::styled(
            format!("  {} ago", short_duration(done.elapsed().as_secs())),
            DIM,
        ));
    }
    let mut lines = vec![section("Claude", extra, width)];
    lines.push(Line::from(vec![
        Span::styled("› ", Color::Magenta),
        Span::styled(run.prompt.clone(), Style::new().fg(DIM).italic()),
    ]));
    match &run.state {
        RunState::Running => {}
        RunState::Failed(e) => lines.push(Line::styled(e.clone(), Color::Red)),
        RunState::Done(_) => {
            lines.push(Line::raw(""));
            match &run.summary {
                Some(text) => {
                    let md = markdown::render(text, width);
                    let cut = md.len() > REPLY_LINES;
                    lines.extend(md.into_iter().take(REPLY_LINES));
                    if cut {
                        lines.push(Line::styled("… rest of the reply is in the log", DIM));
                    }
                }
                None => lines.push(Line::styled("claude printed nothing", DIM)),
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(format!("log: {}", run.log.display()), DIM));
    if let Some(p) = &run.kept_worktree {
        lines.push(Line::styled(
            format!("worktree kept: {}", p.display()),
            Color::Yellow,
        ));
    }
    lines
}

fn severity_color(severity: &str) -> Color {
    match severity.to_ascii_lowercase().as_str() {
        "critical" => Color::Red,
        "major" => Color::Indexed(203),
        "minor" => Color::Yellow,
        _ => DIM,
    }
}

const RESOLVED_SHOWN: usize = 3;

/// CodeRabbit's review threads: unresolved first, each clickable to its comment on GitHub.
fn rabbit_section(
    d: &Detail,
    width: usize,
    t: &mut Vec<Line<'static>>,
    clickable: &mut Vec<(usize, String)>,
) {
    let open = d.rabbit_threads.iter().filter(|x| !x.resolved).count();
    let status = if open > 0 {
        Span::styled(format!("{open} open"), RABBIT)
    } else {
        Span::styled("all resolved", Color::Green)
    };
    t.push(section(
        "CodeRabbit",
        vec![
            status,
            Span::styled(format!(" of {}", d.rabbit_threads.len()), DIM),
        ],
        width,
    ));
    if open > 0 {
        t.push(Line::styled(
            "press t to have Claude work through them",
            DIM,
        ));
    }
    let (unresolved, resolved): (Vec<_>, Vec<_>) =
        d.rabbit_threads.iter().partition(|x| !x.resolved);
    let hidden = resolved.len().saturating_sub(RESOLVED_SHOWN);
    for th in unresolved
        .into_iter()
        .chain(resolved.into_iter().take(RESOLVED_SHOWN))
    {
        let (glyph, glyph_color) = if th.resolved {
            ("✓", Color::Green)
        } else {
            ("○", RABBIT)
        };
        let where_ = match th.line {
            Some(l) => format!("{}:{l}", th.path),
            None => th.path.clone(),
        };
        let mut spans = vec![
            Span::styled(format!("{glyph} "), glyph_color),
            Span::styled(
                format!("{} ", th.severity),
                Style::new().fg(severity_color(&th.severity)).bold(),
            ),
        ];
        let title_style = if th.resolved {
            Style::new().fg(DIM)
        } else {
            Style::new()
        };
        spans.push(Span::styled(th.title.clone(), title_style));
        spans.push(Span::styled(format!("  {where_}"), DIM));
        if th.outdated {
            spans.push(Span::styled(" (outdated)", DIM));
        }
        if !th.url.is_empty() {
            clickable.push((t.len(), th.url.clone()));
        }
        t.push(Line::from(spans));
    }
    if hidden > 0 {
        t.push(Line::styled(format!("+{hidden} more resolved"), DIM));
    }
}

pub fn build(
    item: &Item,
    d: &Detail,
    diff_text: Option<&str>,
    diff_err: Option<&str>,
    width: usize,
    claude: Option<(&ClaudeRun, &str)>,
) -> DetailView {
    let mut t: Vec<Line<'static>> = vec![];
    let mut check_lines = vec![];
    let mut diff_line = None;
    let mut file_lines = vec![];

    t.push(Line::styled(item.title.clone(), Style::new().bold()));
    t.push(Line::raw(""));
    t.push(badges(item, d));
    t.push(Line::raw(""));
    if let Some((run, spinner)) = claude {
        t.extend(claude_section(run, spinner, width));
        t.push(Line::raw(""));
    }

    let mut meta = vec![Span::styled(format!("@{}", item.author), Color::Blue)];
    let opened = ago(&d.created_at);
    if !opened.is_empty() {
        meta.push(Span::styled(format!("  opened {opened}"), DIM));
    }
    meta.push(Span::styled(
        format!("  ·  updated {}", ago(&item.updated_at.to_rfc3339())),
        DIM,
    ));
    t.push(Line::from(meta));

    if item.kind == Kind::Pr {
        t.push(branch_line(d));
        t.push(stats_line(d));
    }
    if !d.labels.is_empty() {
        let mut spans = vec![];
        for l in &d.labels {
            spans.push(chip(l, 40));
            spans.push(Span::raw(" "));
        }
        t.push(Line::raw(""));
        t.push(Line::from(spans));
    }
    if !d.assignees.is_empty() {
        t.push(Line::from(vec![
            Span::styled("assigned ", DIM),
            Span::raw(d.assignees.join(", ")),
        ]));
    }

    if item.kind == Kind::Pr {
        if !d.requested.is_empty() || !d.reviews.is_empty() {
            t.push(Line::raw(""));
            t.push(section("Reviews", vec![], width));
            for (who, state) in &d.reviews {
                let (sym, c) = match state.as_str() {
                    "APPROVED" => ("✓", Color::Green),
                    "CHANGES_REQUESTED" => ("✗", Color::Red),
                    _ => ("•", DIM),
                };
                t.push(Line::from(vec![
                    Span::styled(format!("{sym} "), c),
                    Span::raw(format!("{who}  ")),
                    Span::styled(state.to_lowercase().replace('_', " "), DIM),
                ]));
            }
            for who in &d.requested {
                t.push(Line::from(vec![
                    Span::styled("○ ", Color::Yellow),
                    Span::raw(format!("{who}  ")),
                    Span::styled("awaiting review", DIM),
                ]));
            }
        }

        if !d.rabbit_threads.is_empty() {
            t.push(Line::raw(""));
            rabbit_section(d, width, &mut t, &mut check_lines);
        }

        let (pass, fail, pending) = check_counts(d);
        let mut extra = bar(
            &[
                (pass, Color::Green),
                (fail, Color::Red),
                (pending, Color::Yellow),
            ],
            16,
            "━",
        );
        extra.push(Span::styled(format!(" {pass}/{}", d.checks.len()), DIM));
        t.push(Line::raw(""));
        t.push(section(
            "Checks",
            if d.checks.is_empty() { vec![] } else { extra },
            width,
        ));
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
            let name = match c.url {
                Some(_) => Style::new().underlined(),
                None => Style::new(),
            };
            if let Some(u) = &c.url {
                check_lines.push((t.len(), u.clone()));
            }
            t.push(Line::from(vec![
                ci_span(c.ci),
                Span::styled(format!(" {}", c.name), name),
            ]));
        }
    }

    t.push(Line::raw(""));
    t.push(section("Description", vec![], width));
    let body = markdown::render(&d.body, width);
    if body.is_empty() {
        t.push(Line::styled("no description", DIM));
    }
    t.extend(body);

    if !d.comments.is_empty() {
        t.push(Line::raw(""));
        t.push(section(
            "Comments",
            vec![Span::styled(format!("{}", d.comments.len()), DIM)],
            width,
        ));
        for c in &d.comments {
            t.push(Line::raw(""));
            t.push(Line::from(vec![
                Span::styled("▎ ", Color::Blue),
                Span::styled(
                    format!("@{}", c.author),
                    Style::new().bold().fg(Color::Blue),
                ),
                Span::styled(format!("  {}", ago(&c.created_at)), DIM),
            ]));
            for l in markdown::render(&c.body, width.saturating_sub(2)) {
                let mut spans = vec![Span::styled("▎ ", Color::Indexed(238))];
                spans.extend(l.spans);
                t.push(Line::from(spans));
            }
        }
    }

    if item.kind == Kind::Pr {
        t.push(Line::raw(""));
        diff_line = Some(t.len());
        let parsed = diff_text.map(diff::parse);
        let extra = vec![Span::styled(
            format!("+{} −{} · {} files", d.additions, d.deletions, d.files),
            DIM,
        )];
        t.push(section("Diff", extra, width));
        match (&parsed, diff_err) {
            (Some(p), _) if p.files.is_empty() && p.note.is_none() => {
                t.push(Line::styled("empty diff", DIM))
            }
            (Some(p), _) => {
                t.extend(diff::summary(p, width));
                t.push(Line::raw(""));
                let base = t.len();
                let r = diff::render(p, width);
                file_lines = r.file_starts.iter().map(|s| base + s).collect();
                t.extend(r.lines);
            }
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
        file_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proportions_always_sum_to_cells() {
        for parts in [
            vec![1, 1],
            vec![9, 1],
            vec![0, 5],
            vec![3, 3, 3],
            vec![1, 0, 0],
        ] {
            let p = proportions(&parts, 10);
            assert_eq!(p.iter().sum::<usize>(), 10, "{parts:?}");
        }
        assert_eq!(proportions(&[0, 0], 10), vec![0, 0]);
    }

    #[test]
    fn ago_formats_relative_time() {
        assert_eq!(ago("not a date"), "");
        assert_eq!(ago(&Utc::now().to_rfc3339()), "just now");
        let old = (Utc::now() - chrono::Duration::days(3)).to_rfc3339();
        assert_eq!(ago(&old), "3d ago");
    }
}
