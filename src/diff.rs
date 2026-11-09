//! Parses unified diffs and renders them with line numbers, tinted rows and file header bars.

use crate::ui::{rgb_color, DIM};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Added,
    Deleted,
    Renamed,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DLine {
    pub kind: Kind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Hunk {
    pub range: String,
    pub context: String,
    pub lines: Vec<DLine>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub old: String,
    pub new: String,
    pub status: Status,
    pub binary: bool,
    pub adds: usize,
    pub dels: usize,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
    /// Set when the fetched diff was cut short.
    pub note: Option<String>,
}

const TRUNCATION_MARK: &str = "… diff truncated";

fn strip_prefix_path(p: &str) -> String {
    p.strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p)
        .to_string()
}

fn parse_count(s: &str) -> (u32, u32) {
    match s.split_once(',') {
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(1)),
        None => (s.parse().unwrap_or(0), 1),
    }
}

/// Parses `@@ -a,b +c,d @@ context` into (old start, old len, new start, new len, range text, context).
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32, String, String)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, ctx) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let (os, ol) = parse_count(old);
    let (ns, nl) = parse_count(new);
    Some((
        os,
        ol,
        ns,
        nl,
        format!("-{old} +{new}"),
        ctx.trim().to_string(),
    ))
}

pub fn parse(text: &str) -> Diff {
    let mut diff = Diff::default();
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.last().is_some_and(|l| l.starts_with(TRUNCATION_MARK)) {
        diff.note = lines.pop().map(str::to_string);
    }

    let mut remaining = (0u32, 0u32);
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for line in lines {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let (old, new) = rest
                .split_once(" b/")
                .map(|(a, b)| (strip_prefix_path(a), b.to_string()))
                .unwrap_or_else(|| (rest.to_string(), rest.to_string()));
            diff.files.push(FileDiff {
                old,
                new,
                status: Status::Modified,
                binary: false,
                adds: 0,
                dels: 0,
                hunks: vec![],
            });
            remaining = (0, 0);
            continue;
        }
        let Some(file) = diff.files.last_mut() else {
            continue;
        };

        if remaining != (0, 0) {
            let (kind, old, new, text) = match line.chars().next() {
                Some('+') => (Kind::Add, None, Some(new_no), &line[1..]),
                Some('-') => (Kind::Del, Some(old_no), None, &line[1..]),
                Some('\\') => continue,
                _ => (
                    Kind::Context,
                    Some(old_no),
                    Some(new_no),
                    line.get(1..).unwrap_or(""),
                ),
            };
            match kind {
                Kind::Add => {
                    new_no += 1;
                    remaining.1 = remaining.1.saturating_sub(1);
                    file.adds += 1;
                }
                Kind::Del => {
                    old_no += 1;
                    remaining.0 = remaining.0.saturating_sub(1);
                    file.dels += 1;
                }
                Kind::Context => {
                    old_no += 1;
                    new_no += 1;
                    remaining.0 = remaining.0.saturating_sub(1);
                    remaining.1 = remaining.1.saturating_sub(1);
                }
            }
            if let Some(h) = file.hunks.last_mut() {
                h.lines.push(DLine {
                    kind,
                    old,
                    new,
                    text: text.to_string(),
                });
            }
            continue;
        }

        if let Some((os, ol, ns, nl, range, context)) = parse_hunk_header(line) {
            old_no = os;
            new_no = ns;
            remaining = (ol, nl);
            file.hunks.push(Hunk {
                range,
                context,
                lines: vec![],
            });
        } else if line.starts_with("new file mode") {
            file.status = Status::Added;
        } else if line.starts_with("deleted file mode") {
            file.status = Status::Deleted;
        } else if let Some(p) = line.strip_prefix("rename from ") {
            file.status = Status::Renamed;
            file.old = p.to_string();
        } else if let Some(p) = line.strip_prefix("rename to ") {
            file.new = p.to_string();
        } else if line.starts_with("Binary files") || line.starts_with("GIT binary patch") {
            file.binary = true;
        }
    }
    diff
}

fn status_style(s: Status) -> (&'static str, Color) {
    match s {
        Status::Added => ("A", Color::Green),
        Status::Deleted => ("D", Color::Red),
        Status::Renamed => ("R", Color::Cyan),
        Status::Modified => ("M", Color::Yellow),
    }
}

fn fit(text: &str, max: usize) -> String {
    let text = text.replace('\t', "    ");
    if text.chars().count() <= max {
        return text;
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('›');
    out
}

fn span_width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn pad_to(mut spans: Vec<Span<'static>>, width: usize, bg: Option<Color>) -> Line<'static> {
    let w = span_width(&spans);
    if w < width {
        let style = bg.map_or(Style::new(), |b| Style::new().bg(b));
        spans.push(Span::styled(" ".repeat(width - w), style));
    }
    Line::from(spans)
}

fn mini_bar(adds: usize, dels: usize) -> Vec<Span<'static>> {
    let total = adds + dels;
    let cells = 5usize;
    let green = (adds * cells + total / 2).checked_div(total).unwrap_or(0);
    let red = if total == 0 { 0 } else { cells - green };
    vec![
        Span::styled("■".repeat(green), Color::Green),
        Span::styled("■".repeat(red), Color::Red),
        Span::styled("■".repeat(cells - green - red), DIM),
    ]
}

pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    /// Index into `lines` of each file's header bar.
    pub file_starts: Vec<usize>,
}

fn file_title(f: &FileDiff) -> String {
    if f.status == Status::Renamed && f.old != f.new {
        format!("{} → {}", f.old, f.new)
    } else {
        f.new.clone()
    }
}

pub fn render(diff: &Diff, width: usize) -> Rendered {
    let width = width.max(24);
    let bar_bg = rgb_color((38, 42, 54));
    let hunk_bg = rgb_color((28, 34, 48));
    let add_bg = rgb_color((18, 56, 32));
    let del_bg = rgb_color((66, 24, 30));
    let row_fg = Color::Indexed(253);
    let num_w = diff
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.lines.iter())
        .flat_map(|l| [l.old, l.new])
        .flatten()
        .max()
        .map_or(3, |m| m.to_string().len().max(3));
    let gutter = num_w * 2 + 4;

    let mut lines: Vec<Line<'static>> = vec![];
    let mut file_starts = vec![];

    for f in &diff.files {
        let (letter, color) = status_style(f.status);
        let mut spans = vec![
            Span::styled(
                format!(" {letter} "),
                Style::new().fg(Color::Black).bg(color).bold(),
            ),
            Span::styled(
                format!(" {}", fit(&file_title(f), width.saturating_sub(24))),
                Style::new().bold().bg(bar_bg),
            ),
        ];
        let stats = format!("+{} −{} ", f.adds, f.dels);
        let used = span_width(&spans) + stats.chars().count();
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used)),
            Style::new().bg(bar_bg),
        ));
        spans.push(Span::styled(
            format!("+{} ", f.adds),
            Style::new().fg(Color::Green).bg(bar_bg),
        ));
        spans.push(Span::styled(
            format!("−{} ", f.dels),
            Style::new().fg(Color::Red).bg(bar_bg),
        ));
        file_starts.push(lines.len());
        lines.push(Line::from(spans));

        if f.binary {
            lines.push(Line::styled("  binary file not shown", DIM));
        } else if f.hunks.is_empty() {
            let msg = match f.status {
                Status::Renamed => "  renamed without changes",
                _ => "  no textual changes",
            };
            lines.push(Line::styled(msg, DIM));
        }

        for h in &f.hunks {
            let mut spans = vec![
                Span::styled(" ⋯ ".to_string(), Style::new().fg(DIM).bg(hunk_bg)),
                Span::styled(
                    format!("@@ {} @@", h.range),
                    Style::new().fg(Color::Cyan).bg(hunk_bg),
                ),
            ];
            if !h.context.is_empty() {
                let room = width.saturating_sub(span_width(&spans) + 2);
                spans.push(Span::styled(
                    format!("  {}", fit(&h.context, room)),
                    Style::new()
                        .fg(DIM)
                        .bg(hunk_bg)
                        .add_modifier(Modifier::ITALIC),
                ));
            }
            lines.push(pad_to(spans, width, Some(hunk_bg)));

            for l in &h.lines {
                let num = |n: Option<u32>| match n {
                    Some(n) => format!("{n:>num_w$}"),
                    None => " ".repeat(num_w),
                };
                let (marker, bg, mcolor) = match l.kind {
                    Kind::Add => ('+', Some(add_bg), Color::Green),
                    Kind::Del => ('-', Some(del_bg), Color::Red),
                    Kind::Context => (' ', None, DIM),
                };
                let mut gutter_style = Style::new().fg(DIM);
                let mut text_style = Style::new();
                if let Some(bg) = bg {
                    gutter_style = gutter_style.bg(bg);
                    text_style = text_style.fg(row_fg).bg(bg);
                }
                let spans = vec![
                    Span::styled(format!(" {} {} ", num(l.old), num(l.new)), gutter_style),
                    Span::styled(format!("{marker} "), gutter_style.fg(mcolor).bold()),
                    Span::styled(fit(&l.text, width.saturating_sub(gutter + 2)), text_style),
                ];
                lines.push(pad_to(spans, width, bg));
            }
        }
        lines.push(Line::raw(""));
    }
    if let Some(note) = &diff.note {
        lines.push(Line::styled(note.clone(), Style::new().fg(Color::Yellow)));
    }
    Rendered { lines, file_starts }
}

/// One row per file: status letter, path, counts and a proportion bar.
pub fn summary(diff: &Diff, width: usize) -> Vec<Line<'static>> {
    let longest = diff
        .files
        .iter()
        .map(|f| file_title(f).chars().count())
        .max()
        .unwrap_or(0);
    let name_w = longest.min(width.saturating_sub(24)).max(8);
    diff.files
        .iter()
        .map(|f| {
            let (letter, color) = status_style(f.status);
            let mut spans = vec![
                Span::styled(letter.to_string(), Style::new().fg(color).bold()),
                Span::raw(format!(" {:<name_w$} ", fit(&file_title(f), name_w))),
                Span::styled(format!("+{:<4}", f.adds), Color::Green),
                Span::styled(format!("−{:<4} ", f.dels), Color::Red),
            ];
            spans.extend(mini_bar(f.adds, f.dels));
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,4 +10,5 @@ fn main() {
 keep
-old line
+new line
+another
 tail
\\ No newline at end of file
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..333
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
--- starts with dashes
diff --git a/img.png b/img.png
Binary files a/img.png and b/img.png differ
diff --git a/old.rs b/renamed.rs
similarity index 100%
rename from old.rs
rename to renamed.rs
";

    #[test]
    fn parses_files_statuses_and_counts() {
        let d = parse(SAMPLE);
        let names: Vec<_> = d.files.iter().map(|f| f.new.as_str()).collect();
        assert_eq!(
            names,
            ["src/a.rs", "new.txt", "gone.txt", "img.png", "renamed.rs"]
        );
        assert_eq!(d.files[0].status, Status::Modified);
        assert_eq!((d.files[0].adds, d.files[0].dels), (2, 1));
        assert_eq!(d.files[1].status, Status::Added);
        assert_eq!(d.files[2].status, Status::Deleted);
        assert!(d.files[3].binary);
        assert_eq!(d.files[4].status, Status::Renamed);
        assert_eq!(d.files[4].old, "old.rs");
    }

    #[test]
    fn tracks_line_numbers() {
        let d = parse(SAMPLE);
        let lines = &d.files[0].hunks[0].lines;
        assert_eq!(
            lines[0],
            DLine {
                kind: Kind::Context,
                old: Some(10),
                new: Some(10),
                text: "keep".into()
            }
        );
        assert_eq!(
            (lines[1].kind, lines[1].old, lines[1].new),
            (Kind::Del, Some(11), None)
        );
        assert_eq!(
            (lines[2].kind, lines[2].old, lines[2].new),
            (Kind::Add, None, Some(11))
        );
        assert_eq!(lines[4].new, Some(13));
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn content_that_looks_like_a_header_stays_content() {
        let d = parse(SAMPLE);
        let gone = &d.files[2].hunks[0].lines;
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].kind, Kind::Del);
        assert_eq!(gone[0].text, "-- starts with dashes");
    }

    #[test]
    fn truncation_note_is_split_off() {
        let d = parse("diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n… diff truncated, 5 more lines");
        assert_eq!(d.files[0].hunks[0].lines.len(), 2);
        assert!(d.note.unwrap().contains("5 more lines"));
    }

    #[test]
    fn empty_input_is_empty_diff() {
        assert!(parse("").files.is_empty());
    }

    #[test]
    fn render_never_exceeds_width() {
        let d = parse(&format!(
            "diff --git a/x b/x\n@@ -1 +1 @@ fn {}\n-{}\n+{}\n",
            "ctx".repeat(40),
            "a".repeat(300),
            "é".repeat(300)
        ));
        for width in [30usize, 60, 100] {
            let r = render(&d, width);
            for l in &r.lines {
                assert!(l.width() <= width, "width {width}: {}", l.width());
            }
        }
    }

    #[test]
    fn added_and_removed_rows_are_tinted_full_width() {
        let r = render(&parse(SAMPLE), 60);
        let add = r
            .lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content == "new line"))
            .unwrap();
        assert_eq!(add.width(), 60);
        assert!(add.spans.iter().any(|s| s.style.bg.is_some()));
        let ctx = r
            .lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content == "keep"))
            .unwrap();
        assert!(ctx.spans.iter().all(|s| s.style.bg.is_none()));
    }

    #[test]
    fn file_starts_point_at_header_bars() {
        let d = parse(SAMPLE);
        let r = render(&d, 80);
        assert_eq!(r.file_starts.len(), d.files.len());
        let first: String = r.lines[r.file_starts[0]]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(first.contains("src/a.rs"));
        assert!(first.contains("+2"));
    }

    #[test]
    fn summary_has_one_row_per_file() {
        let d = parse(SAMPLE);
        assert_eq!(summary(&d, 80).len(), 5);
    }

    #[test]
    fn mini_bar_always_five_cells() {
        for (a, d) in [(0, 0), (1, 0), (0, 9), (3, 3), (100, 1)] {
            let n: usize = mini_bar(a, d)
                .iter()
                .map(|s| s.content.chars().count())
                .sum();
            assert_eq!(n, 5, "{a}/{d}");
        }
    }
}
