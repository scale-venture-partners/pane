//! A small terminal renderer for the GitHub-flavored Markdown that shows up in PR and issue text.

use crate::ui::DIM;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const CODE_FG: Color = Color::Indexed(180);
const CODE_BG: Color = Color::Indexed(237);
const LINK: Color = Color::Blue;

fn drop_html_comments(body: &str) -> String {
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
    out
}

/// Removes inline HTML tags such as `<br>` or `<summary>`, keeping their text.
fn strip_tags(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts_tag = c == '<'
            && chars
                .get(i + 1)
                .is_some_and(|n| n.is_ascii_alphabetic() || *n == '/');
        if starts_tag {
            if let Some(end) = chars[i..].iter().position(|&x| x == '>') {
                i += end + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

struct Inline {
    spans: Vec<Span<'static>>,
    buf: String,
    bold: bool,
    italic: bool,
    base: Style,
}

impl Inline {
    fn style(&self) -> Style {
        let mut s = self.base;
        if self.bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        s
    }

    fn flush(&mut self) {
        if !self.buf.is_empty() {
            let text = std::mem::take(&mut self.buf);
            self.spans.push(Span::styled(text, self.style()));
        }
    }

    fn push_styled(&mut self, text: String, style: Style) {
        self.flush();
        self.spans.push(Span::styled(text, style));
    }
}

fn find_from(chars: &[char], from: usize, pat: &[char]) -> Option<usize> {
    if pat.is_empty() || from >= chars.len() {
        return None;
    }
    (from..chars.len().saturating_sub(pat.len() - 1)).find(|&i| chars[i..i + pat.len()] == *pat)
}

/// Spans for one line of inline Markdown: code, bold, italic, links, images and bare URLs.
pub fn inline(text: &str, base: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let mut b = Inline {
        spans: vec![],
        buf: String::new(),
        bold: false,
        italic: false,
        base,
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '`' => {
                if let Some(end) = find_from(&chars, i + 1, &['`']) {
                    let code: String = chars[i + 1..end].iter().collect();
                    b.push_styled(format!(" {code} "), Style::new().fg(CODE_FG).bg(CODE_BG));
                    i = end + 1;
                    continue;
                }
            }
            '*' if next == Some('*') => {
                b.flush();
                b.bold = !b.bold;
                i += 2;
                continue;
            }
            '*' if b.italic || next.is_some_and(|n| !n.is_whitespace() && n != '*') => {
                b.flush();
                b.italic = !b.italic;
                i += 1;
                continue;
            }
            '!' if next == Some('[') => {
                if let Some((label, _, end)) = link_at(&chars, i + 1) {
                    b.push_styled(format!("▣ {label}"), Style::new().fg(DIM).italic());
                    i = end;
                    continue;
                }
            }
            '[' => {
                if let Some((label, _, end)) = link_at(&chars, i) {
                    let style = b.style().fg(LINK).add_modifier(Modifier::UNDERLINED);
                    b.push_styled(label, style);
                    i = end;
                    continue;
                }
            }
            'h' if is_url_start(&chars, i) => {
                let mut end = i;
                while end < chars.len()
                    && !chars[end].is_whitespace()
                    && !")>]".contains(chars[end])
                {
                    end += 1;
                }
                while end > i && ".,;:!?".contains(chars[end - 1]) {
                    end -= 1;
                }
                let url: String = chars[i..end].iter().collect();
                let style = b.style().fg(LINK).add_modifier(Modifier::UNDERLINED);
                b.push_styled(url, style);
                i = end;
                continue;
            }
            _ => {}
        }
        b.buf.push(c);
        i += 1;
    }
    b.flush();
    b.spans
}

fn is_url_start(chars: &[char], i: usize) -> bool {
    let at_boundary = i == 0 || chars[i - 1].is_whitespace() || "([<".contains(chars[i - 1]);
    let rest: String = chars[i..chars.len().min(i + 8)].iter().collect();
    at_boundary && (rest.starts_with("http://") || rest.starts_with("https://"))
}

/// Parses `[label](url)` starting at `chars[start] == '['`; returns label, url, index after `)`.
fn link_at(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let close = find_from(chars, start + 1, &[']'])?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = find_from(chars, close + 2, &[')'])?;
    Some((
        chars[start + 1..close].iter().collect(),
        chars[close + 2..end].iter().collect(),
        end + 1,
    ))
}

fn is_rule(t: &str) -> bool {
    let t = t.trim();
    t.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| t.chars().all(|c| m.starts_with(c) || c == ' '))
}

fn is_table_separator(t: &str) -> bool {
    t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn list_marker(t: &str) -> Option<(usize, String, &str)> {
    let indent = t.len() - t.trim_start().len();
    let rest = t.trim_start();
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = rest.strip_prefix(m) {
            return Some((indent, "•".to_string(), r));
        }
    }
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        if let Some(r) = rest[digits.len()..].strip_prefix(". ") {
            return Some((indent, format!("{digits}."), r));
        }
    }
    None
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&level) && line[level..].starts_with(' ') {
        Some((level, line[level..].trim()))
    } else {
        None
    }
}

pub fn render(body: &str, width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = vec![];
    let mut in_code = false;
    for raw in drop_html_comments(body).trim().lines() {
        let fence = raw.trim_start();
        if fence.starts_with("```") || fence.starts_with("~~~") {
            if in_code {
                out.push(Line::styled("╰─", DIM));
            } else {
                let lang = fence.trim_start_matches(['`', '~']).trim();
                out.push(Line::styled(format!("╭─ {lang}"), DIM));
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(Line::from(vec![
                Span::styled("│ ", DIM),
                Span::styled(raw.replace('\t', "    "), Style::new().fg(CODE_FG)),
            ]));
            continue;
        }
        let line = strip_tags(raw);
        let t = line.trim_end();
        if t.trim().is_empty() {
            if out.last().is_some_and(|l| !l.spans.is_empty()) {
                out.push(Line::raw(""));
            }
            continue;
        }
        out.push(block_line(t, width));
    }
    while out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    out
}

fn block_line(t: &str, width: usize) -> Line<'static> {
    if let Some((level, text)) = heading(t) {
        let color = match level {
            1 => Color::Yellow,
            2 => Color::Cyan,
            _ => Color::Reset,
        };
        let style = Style::new().bold().fg(color);
        let mut spans = vec![Span::styled("▍", Style::new().fg(color))];
        spans.push(Span::raw(" "));
        spans.extend(inline(text, style));
        return Line::from(spans);
    }
    if is_rule(t) {
        return Line::styled("─".repeat(width.min(40)), DIM);
    }
    if let Some(quote) = t.strip_prefix('>') {
        let mut spans = vec![Span::styled("▎ ", DIM)];
        spans.extend(inline(quote.trim_start(), Style::new().fg(DIM).italic()));
        return Line::from(spans);
    }
    if t.trim_start().starts_with('|') {
        return table_line(t, width);
    }
    if let Some((indent, marker, rest)) = list_marker(t) {
        let mut spans = vec![Span::raw(" ".repeat(indent))];
        let (rest, glyph) = if let Some(r) = rest.strip_prefix("[ ] ") {
            (r, Span::styled("☐ ", DIM))
        } else if let Some(r) = rest
            .strip_prefix("[x] ")
            .or_else(|| rest.strip_prefix("[X] "))
        {
            (r, Span::styled("☑ ", Color::Green))
        } else {
            (rest, Span::styled(format!("{marker} "), Color::Yellow))
        };
        spans.push(glyph);
        spans.extend(inline(rest, Style::new()));
        return Line::from(spans);
    }
    Line::from(inline(t, Style::new()))
}

fn table_line(t: &str, width: usize) -> Line<'static> {
    if is_table_separator(t) {
        return Line::styled("─".repeat(width.min(60)), DIM);
    }
    let cells: Vec<&str> = t.trim().trim_matches('|').split('|').collect();
    let mut spans = vec![];
    for (n, cell) in cells.iter().enumerate() {
        if n > 0 {
            spans.push(Span::styled(" │ ", DIM));
        }
        spans.extend(inline(cell.trim(), Style::new()));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn inline_styles() {
        let spans = inline("a **bold** and `code` and *it*", Style::new());
        let bold = spans.iter().find(|s| s.content == "bold").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let code = spans.iter().find(|s| s.content.contains("code")).unwrap();
        assert_eq!(code.style.bg, Some(CODE_BG));
        let it = spans.iter().find(|s| s.content == "it").unwrap();
        assert!(it.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn links_keep_label_and_drop_url() {
        let spans = inline("see [the docs](https://x.dev/a) now", Style::new());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "see the docs now");
        let link = spans.iter().find(|s| s.content == "the docs").unwrap();
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn bare_urls_are_underlined_without_trailing_punctuation() {
        let spans = inline("go to https://example.com/x.", Style::new());
        let url = spans
            .iter()
            .find(|s| s.content.starts_with("http"))
            .unwrap();
        assert_eq!(url.content, "https://example.com/x");
    }

    #[test]
    fn snake_case_and_math_are_untouched() {
        let l = inline("my_var_name and 2 * 3 * 4", Style::new());
        let joined: String = l.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "my_var_name and 2 * 3 * 4");
    }

    #[test]
    fn images_become_placeholders() {
        let spans = inline("![diagram](https://x/y.png)", Style::new());
        assert_eq!(spans[0].content, "▣ diagram");
    }

    #[test]
    fn unclosed_markers_stay_literal() {
        let spans = inline("a `b and [c", Style::new());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "a `b and [c");
    }

    #[test]
    fn comments_and_tags_are_removed() {
        let lines = render(
            "<!-- hidden -->\n<details><summary>More</summary>\nBody &amp; soul",
            80,
        );
        let all: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(all, ["More", "Body & soul"]);
    }

    #[test]
    fn lists_tasks_and_quotes() {
        let lines = render(
            "- one\n  - nested\n1. first\n- [ ] todo\n- [x] done\n> quoted",
            80,
        );
        let all: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(all[0], "• one");
        assert_eq!(all[1], "  • nested");
        assert_eq!(all[2], "1. first");
        assert_eq!(all[3], "☐ todo");
        assert_eq!(all[4], "☑ done");
        assert_eq!(all[5], "▎ quoted");
    }

    #[test]
    fn code_fences_get_a_gutter() {
        let lines = render("```rust\nlet x = 1;\n```", 80);
        let all: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(all, ["╭─ rust", "│ let x = 1;", "╰─"]);
    }

    #[test]
    fn headings_rules_and_tables() {
        let lines = render("## Title\n---\n| a | b |\n|---|---|\n| 1 | 2 |", 80);
        let all: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(all[0], "▍ Title");
        assert!(all[1].starts_with('─'));
        assert_eq!(all[2], "a │ b");
        assert!(all[3].starts_with('─'));
        assert_eq!(all[4], "1 │ 2");
    }

    #[test]
    fn blank_lines_collapse() {
        let lines = render("a\n\n\n\nb\n\n", 80);
        assert_eq!(lines.len(), 3);
    }
}
