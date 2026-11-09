use crate::config::Config;
use crate::gh::{self, ActionReq, Detail, Item, Op, Snapshot};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc::UnboundedSender, Notify};

const DETAIL_STALE: Duration = Duration::from_secs(45);

/// Messages are short-lived and sent one at a time, so variant size does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Msg {
    Term(Event),
    Tick,
    Loading,
    Progress(usize, usize),
    List(Result<Snapshot, String>),
    Viewer(Result<String, String>),
    Detail(String, Result<Detail, String>),
    Diff(String, Result<String, String>),
    ActionDone(Result<String, String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Prs,
    Issues,
}

pub enum Mode {
    Normal,
    Filter,
    Comment,
    Confirm(Op),
    Help,
}

pub struct Status {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct App {
    pub cfg: Config,
    pub viewer: Option<String>,
    pub viewer_err: Option<String>,
    pub progress: Option<(usize, usize)>,
    pub frame: usize,
    pub prs: Vec<Item>,
    pub issues: Vec<Item>,
    pub tab: Tab,
    pub selected: Option<String>,
    pub list_state: ListState,
    pub row_ids: Vec<Option<String>>,
    pub detail_open: bool,
    pub detail_scroll: u16,
    pub details: HashMap<String, Detail>,
    pub detail_err: HashMap<String, String>,
    pub diffs: HashMap<String, (Instant, String)>,
    pub diff_err: HashMap<String, String>,
    inflight: HashSet<String>,
    diff_inflight: HashSet<String>,
    pub check_hits: Vec<(Rect, String)>,
    pub diff_row: u16,
    /// The selected item after it left the open list; kept on screen until you move off it.
    pub ghost: Option<Item>,
    pub fresh: HashSet<String>,
    seen: HashMap<String, chrono::DateTime<chrono::Utc>>,
    pub last_ok: Option<Instant>,
    pub fetch_error: Option<String>,
    pub loading: bool,
    pub rate: Option<u64>,
    pub status: Option<Status>,
    pub mode: Mode,
    pub input: String,
    pub filter: String,
    pub mine: bool,
    pub quit: bool,
    pub wide: bool,
    pub list_area: Rect,
    pub tab_areas: [Rect; 2],
    tx: UnboundedSender<Msg>,
    refresh: Arc<Notify>,
}

pub fn group_order(items: &[Item]) -> Vec<usize> {
    let mut latest: HashMap<&str, chrono::DateTime<chrono::Utc>> = HashMap::new();
    for i in items {
        let e = latest.entry(i.repo_full.as_str()).or_insert(i.updated_at);
        if i.updated_at > *e {
            *e = i.updated_at;
        }
    }
    let mut idx: Vec<usize> = (0..items.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (&items[a], &items[b]);
        latest[y.repo_full.as_str()]
            .cmp(&latest[x.repo_full.as_str()])
            .then_with(|| x.repo_full.cmp(&y.repo_full))
            .then_with(|| y.updated_at.cmp(&x.updated_at))
    });
    idx
}

impl App {
    pub fn new(cfg: Config, tx: UnboundedSender<Msg>, refresh: Arc<Notify>) -> Self {
        Self {
            cfg,
            viewer: None,
            viewer_err: None,
            progress: None,
            frame: 0,
            prs: vec![],
            issues: vec![],
            tab: Tab::Prs,
            selected: None,
            list_state: ListState::default(),
            row_ids: vec![],
            detail_open: false,
            detail_scroll: 0,
            details: HashMap::new(),
            detail_err: HashMap::new(),
            diffs: HashMap::new(),
            diff_err: HashMap::new(),
            inflight: HashSet::new(),
            diff_inflight: HashSet::new(),
            check_hits: vec![],
            diff_row: 0,
            ghost: None,
            fresh: HashSet::new(),
            seen: HashMap::new(),
            last_ok: None,
            fetch_error: None,
            loading: true,
            rate: None,
            status: None,
            mode: Mode::Normal,
            input: String::new(),
            filter: String::new(),
            mine: false,
            quit: false,
            wide: true,
            list_area: Rect::default(),
            tab_areas: [Rect::default(); 2],
            tx,
            refresh,
        }
    }

    pub fn spinner(&self) -> &'static str {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[self.frame % FRAMES.len()]
    }

    pub fn items(&self) -> &[Item] {
        match self.tab {
            Tab::Prs => &self.prs,
            Tab::Issues => &self.issues,
        }
    }

    pub fn matches(&self, i: &Item) -> bool {
        if self.mine && self.viewer.as_deref() != Some(i.author.as_str()) {
            return false;
        }
        if self.filter.is_empty() {
            return true;
        }
        let labels: Vec<&str> = i.labels.iter().map(|l| l.name.as_str()).collect();
        let hay = format!(
            "{} {} {} #{} {}",
            i.title,
            i.repo_full,
            i.author,
            i.number,
            labels.join(" ")
        )
        .to_lowercase();
        self.filter
            .to_lowercase()
            .split_whitespace()
            .all(|w| hay.contains(w))
    }

    /// Visible items of the current tab in display order.
    pub fn visible(&self) -> Vec<&Item> {
        let items = self.items();
        group_order(items)
            .into_iter()
            .map(|i| &items[i])
            .filter(|i| self.matches(i))
            .collect()
    }

    pub fn selected_item(&self) -> Option<Item> {
        let id = self.selected.as_ref()?;
        self.visible().into_iter().find(|i| &i.id == id).cloned()
    }

    fn select(&mut self, id: Option<String>) {
        self.drop_ghost_unless(id.as_deref());
        if let Some(id) = &id {
            self.fresh.remove(id);
        }
        self.selected = id;
        self.detail_scroll = 0;
        self.ensure_detail();
    }

    fn move_selection(&mut self, delta: isize) {
        let ids: Vec<String> = self.visible().iter().map(|i| i.id.clone()).collect();
        if ids.is_empty() {
            self.selected = None;
            return;
        }
        let cur = self
            .selected
            .as_ref()
            .and_then(|s| ids.iter().position(|i| i == s));
        let next = match cur {
            Some(c) => (c as isize + delta).clamp(0, ids.len() as isize - 1) as usize,
            None => 0,
        };
        self.select(Some(ids[next].clone()));
    }

    pub fn ensure_selection(&mut self) {
        let ids: Vec<String> = self.visible().iter().map(|i| i.id.clone()).collect();
        let valid = self.selected.as_ref().is_some_and(|s| ids.contains(s));
        if !valid {
            self.selected = ids.first().cloned();
            self.drop_ghost_unless(self.selected.clone().as_deref());
            self.ensure_detail();
        }
    }

    pub fn ghost_selected(&self) -> bool {
        match (&self.ghost, &self.selected) {
            (Some(g), Some(id)) => &g.id == id,
            _ => false,
        }
    }

    fn drop_ghost_unless(&mut self, keep: Option<&str>) {
        let Some(g) = &self.ghost else { return };
        if keep == Some(g.id.as_str()) {
            return;
        }
        let id = g.id.clone();
        self.ghost = None;
        self.prs.retain(|i| i.id != id);
        self.issues.retain(|i| i.id != id);
    }

    fn spawn_detail(&mut self, item: &Item) {
        if !self.inflight.insert(item.id.clone()) {
            return;
        }
        let (tx, item) = (self.tx.clone(), item.clone());
        tokio::spawn(async move {
            let r = gh::fetch_detail(&item).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Detail(item.id, r));
        });
    }

    fn ensure_detail(&mut self) {
        if !self.detail_open && !self.ghost_selected() {
            return;
        }
        let Some(item) = self.selected_item() else {
            return;
        };
        let stale = self
            .details
            .get(&item.id)
            .is_none_or(|d| d.fetched.elapsed() > DETAIL_STALE);
        if stale {
            self.spawn_detail(&item);
        }
        let diff_stale = self
            .diffs
            .get(&item.id)
            .is_none_or(|(t, _)| t.elapsed() > DETAIL_STALE);
        if self.detail_open
            && item.kind == gh::Kind::Pr
            && diff_stale
            && self.diff_inflight.insert(item.id.clone())
        {
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let r = gh::fetch_diff(&item).await.map_err(|e| format!("{e:#}"));
                let _ = tx.send(Msg::Diff(item.id, r));
            });
        }
    }

    fn label_ghost(&mut self, id: &str, state: &str) {
        if self.ghost.as_ref().is_none_or(|g| g.id != id) {
            return;
        }
        let label = if state == "OPEN" { "UNLISTED" } else { state }.to_string();
        for i in self.prs.iter_mut().chain(self.issues.iter_mut()) {
            if i.id == id {
                i.gone = Some(label.clone());
            }
        }
        if let Some(g) = self.ghost.as_mut() {
            g.gone = Some(label);
        }
    }

    pub fn check_url_at(&self, pos: Position) -> Option<String> {
        self.check_hits
            .iter()
            .find(|(r, _)| r.contains(pos))
            .map(|(_, u)| u.clone())
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    fn switch_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.drop_ghost_unless(None);
            self.tab = tab;
            self.selected = None;
            self.detail_open = false;
            self.ensure_selection();
        }
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Term(Event::Key(k)) if k.kind == KeyEventKind::Press => self.on_key(k),
            Msg::Term(Event::Mouse(m)) => self.on_mouse(m),
            Msg::Term(_) | Msg::Tick => {
                self.frame = self.frame.wrapping_add(1);
                if self
                    .status
                    .as_ref()
                    .is_some_and(|s| s.at.elapsed() > Duration::from_secs(8))
                {
                    self.status = None;
                }
            }
            Msg::Loading => {
                self.loading = true;
                self.progress = None;
            }
            Msg::Progress(done, total) => self.progress = Some((done, total)),
            Msg::List(r) => self.on_list(r),
            Msg::Viewer(Ok(v)) => self.viewer = Some(v),
            Msg::Viewer(Err(e)) => self.viewer_err = Some(e),
            Msg::Detail(id, r) => {
                self.inflight.remove(&id);
                match r {
                    Ok(d) => {
                        self.detail_err.remove(&id);
                        self.label_ghost(&id, &d.state);
                        self.details.insert(id, d);
                    }
                    Err(e) => {
                        self.detail_err.insert(id, e);
                    }
                }
            }
            Msg::Diff(id, r) => {
                self.diff_inflight.remove(&id);
                match r {
                    Ok(d) => {
                        self.diff_err.remove(&id);
                        self.diffs.insert(id, (Instant::now(), d));
                    }
                    Err(e) => {
                        self.diff_err.insert(id, e);
                    }
                }
            }
            Msg::ActionDone(Ok(out)) => {
                self.details.clear();
                self.diffs.clear();
                self.set_status(
                    if out.is_empty() {
                        "done".to_string()
                    } else {
                        out
                    },
                    false,
                );
                self.ensure_detail();
                self.refresh.notify_one();
            }
            Msg::ActionDone(Err(e)) => self.set_status(e, true),
        }
    }

    fn on_list(&mut self, r: Result<Snapshot, String>) {
        self.loading = false;
        match r {
            Ok(snap) => {
                let first = self.last_ok.is_none();
                for i in snap.prs.iter().chain(snap.issues.iter()) {
                    let changed = self.seen.get(&i.id).is_none_or(|t| *t != i.updated_at);
                    if changed && !first {
                        self.fresh.insert(i.id.clone());
                    }
                    self.seen.insert(i.id.clone(), i.updated_at);
                }
                let prev = self.selected_item();
                self.prs = snap.prs;
                self.issues = snap.issues;
                self.keep_ghost(prev);
                self.rate = snap.rate_remaining;
                self.last_ok = Some(Instant::now());
                self.fetch_error = None;
                self.ensure_selection();
                if let Some(id) = &self.selected {
                    self.details.remove(id);
                    self.diffs.remove(id);
                }
                self.ensure_detail();
            }
            Err(e) => self.fetch_error = Some(e),
        }
    }

    /// Keeps the selected item on screen if the refresh dropped it from the open list.
    fn keep_ghost(&mut self, prev: Option<Item>) {
        self.ghost = None;
        let Some(mut item) = prev else { return };
        if self
            .prs
            .iter()
            .chain(self.issues.iter())
            .any(|i| i.id == item.id)
        {
            return;
        }
        let first_time = item.gone.is_none();
        item.gone.get_or_insert_with(|| "…".to_string());
        match item.kind {
            gh::Kind::Pr => self.prs.push(item.clone()),
            gh::Kind::Issue => self.issues.push(item.clone()),
        }
        self.ghost = Some(item.clone());
        if first_time {
            self.details.remove(&item.id);
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let pos = Position {
            x: m.column,
            y: m.row,
        };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(url) = self.check_url_at(pos) {
                    open_url(&url);
                } else if self.tab_areas[0].contains(pos) {
                    self.switch_tab(Tab::Prs);
                } else if self.tab_areas[1].contains(pos) {
                    self.switch_tab(Tab::Issues);
                } else if self.list_area.contains(pos) && (self.wide || !self.detail_open) {
                    let row = self.list_state.offset() + (pos.y - self.list_area.y) as usize;
                    if let Some(Some(id)) = self.row_ids.get(row).cloned() {
                        let again = self.selected.as_deref() == Some(id.as_str());
                        self.select(Some(id));
                        if self.wide || again {
                            self.open_detail();
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown => self.scroll(pos, 3),
            MouseEventKind::ScrollUp => self.scroll(pos, -3),
            _ => {}
        }
    }

    fn scroll(&mut self, pos: Position, delta: i32) {
        if self.detail_open && (!self.wide || !self.list_area.contains(pos)) {
            self.detail_scroll = (self.detail_scroll as i32 + delta).max(0) as u16;
        } else {
            self.move_selection(delta.signum() as isize);
        }
    }

    fn open_detail(&mut self) {
        if self.selected.is_some() {
            self.detail_open = true;
            self.detail_scroll = 0;
            self.ensure_detail();
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match &self.mode {
            Mode::Normal => self.on_key_normal(k),
            Mode::Filter => self.on_key_filter(k),
            Mode::Comment => self.on_key_comment(k),
            Mode::Confirm(_) => self.on_key_confirm(k),
            Mode::Help => self.mode = Mode::Normal,
        }
    }

    fn on_key_filter(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Esc => {
                self.filter.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) => self.filter.push(c),
            _ => {}
        }
        self.ensure_selection();
    }

    fn on_key_comment(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.input.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let body = std::mem::take(&mut self.input);
                self.mode = Mode::Normal;
                if !body.trim().is_empty() {
                    self.run(Op::Comment(body));
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
    }

    fn on_key_confirm(&mut self, k: KeyEvent) {
        let Mode::Confirm(op) = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        if matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            self.run(op);
        }
    }

    fn run(&mut self, op: Op) {
        let Some(item) = self.selected_item() else {
            return;
        };
        let req = ActionReq {
            kind: item.kind,
            repo_full: item.repo_full.clone(),
            number: item.number,
            op,
        };
        self.set_status(format!("running on {}#{}…", item.repo, item.number), false);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = gh::run_action(&req).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::ActionDone(r));
        });
    }

    fn confirm(&mut self, op: Op) {
        if self.ghost_selected() {
            self.set_status("no longer open: nothing to do", true);
            return;
        }
        if self.selected.is_some() {
            self.mode = Mode::Confirm(op);
        }
    }

    fn on_key_normal(&mut self, k: KeyEvent) {
        let is_pr = self.tab == Tab::Prs;
        let page = (self.list_area.height as isize).max(4) - 2;
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Tab | KeyCode::BackTab => {
                self.switch_tab(if is_pr { Tab::Issues } else { Tab::Prs })
            }
            KeyCode::Char('1') => self.switch_tab(Tab::Prs),
            KeyCode::Char('2') => self.switch_tab(Tab::Issues),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_selection(isize::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => self.move_selection(isize::MAX / 2),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                if self.detail_open {
                    self.detail_scroll = self.detail_scroll.saturating_add(10);
                } else {
                    self.move_selection(page);
                }
            }
            KeyCode::PageUp | KeyCode::Char('b') => {
                if self.detail_open {
                    self.detail_scroll = self.detail_scroll.saturating_sub(10);
                } else {
                    self.move_selection(-page);
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.open_detail(),
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                if self.detail_open {
                    self.detail_open = false;
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.ensure_selection();
                }
            }
            KeyCode::Char('r') => {
                self.loading = true;
                self.refresh.notify_one();
            }
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('m') => {
                self.mine = !self.mine;
                self.ensure_selection();
            }
            KeyCode::Char('o') => {
                if let Some(i) = self.selected_item() {
                    open_url(&i.url);
                }
            }
            KeyCode::Char('y') => {
                if let Some(i) = self.selected_item() {
                    match copy(&i.url) {
                        Ok(()) => self.set_status("copied URL", false),
                        Err(e) => self.set_status(format!("copy failed: {e}"), true),
                    }
                }
            }
            KeyCode::Char('f') => self.open_failing_check(),
            KeyCode::Char('d') if is_pr && self.detail_open => self.detail_scroll = self.diff_row,
            KeyCode::Char('c') => {
                if self.ghost_selected() {
                    self.set_status("no longer open: nothing to do", true);
                } else if self.selected.is_some() {
                    self.input.clear();
                    self.mode = Mode::Comment;
                }
            }
            KeyCode::Char('x') => self.confirm(Op::Close),
            KeyCode::Char('a') if is_pr => self.confirm(Op::Approve),
            KeyCode::Char('M') if is_pr => {
                let m = self.cfg.merge_method.clone();
                self.confirm(Op::Merge(m))
            }
            KeyCode::Char('D') if is_pr => {
                let op = match self.selected_item() {
                    Some(i) if i.draft => Op::Ready,
                    _ => Op::Draft,
                };
                self.confirm(op)
            }
            _ => {}
        }
    }

    fn open_failing_check(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Some(d) = self.details.get(&id) else {
            self.set_status("open the PR first (Enter) to load checks", true);
            return;
        };
        let pick = d
            .checks
            .iter()
            .find(|c| c.ci == gh::Ci::Failure && c.url.is_some())
            .or_else(|| {
                d.checks
                    .iter()
                    .find(|c| c.ci == gh::Ci::Pending && c.url.is_some())
            });
        match pick.and_then(|c| c.url.clone()) {
            Some(u) => open_url(&u),
            None => self.set_status("no failing or pending check with a URL", false),
        }
    }
}

fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

const CLIPBOARDS: &[(&str, &[&str])] = &[
    ("pbcopy", &[]),
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

fn copy(text: &str) -> std::io::Result<()> {
    for (prog, args) in CLIPBOARDS {
        let spawned = Command::new(prog)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = spawned else { continue };
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(text.as_bytes())?;
        }
        drop(child.stdin.take());
        child.wait()?;
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no clipboard tool found (pbcopy, wl-copy, xclip, xsel)",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::{Ci, Kind};
    use chrono::{TimeZone, Utc};

    fn item(repo: &str, n: u64, day: u32) -> Item {
        Item {
            id: format!("{repo}{n}"),
            kind: Kind::Pr,
            number: n,
            title: format!("title {n}"),
            url: format!("https://github.com/o/{repo}/pull/{n}"),
            repo: repo.into(),
            repo_full: format!("o/{repo}"),
            author: "will".into(),
            updated_at: Utc.with_ymd_and_hms(2026, 11, day, 0, 0, 0).unwrap(),
            draft: false,
            review: None,
            ci: Ci::None,
            gone: None,
            labels: vec![],
        }
    }

    #[test]
    fn groups_by_most_recent_repo_then_item() {
        let items = vec![
            item("a", 1, 1),
            item("b", 2, 5),
            item("a", 3, 9),
            item("b", 4, 6),
        ];
        let order: Vec<u64> = group_order(&items)
            .into_iter()
            .map(|i| items[i].number)
            .collect();
        assert_eq!(order, [3, 1, 4, 2]);
    }

    #[test]
    fn filter_matches_all_words() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(Notify::new()));
        let i = item("tool", 12, 1);
        app.filter = "tool title".into();
        assert!(app.matches(&i));
        app.filter = "tool nope".into();
        assert!(!app.matches(&i));
    }

    #[test]
    fn filter_matches_label_names() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(Notify::new()));
        let mut i = item("tool", 1, 1);
        i.labels = vec![crate::gh::Label {
            name: "needs-review".into(),
            rgb: (0, 0, 0),
        }];
        app.filter = "needs-review".into();
        assert!(app.matches(&i));
        app.filter = "blocked".into();
        assert!(!app.matches(&i));
    }

    #[test]
    fn mine_filter_needs_viewer() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(Notify::new()));
        let i = item("a", 1, 1);
        app.mine = true;
        assert!(!app.matches(&i));
        app.viewer = Some("will".into());
        assert!(app.matches(&i));
    }

    fn loaded_app() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(Notify::new()));
        let snap = Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2), item("b", 3, 3)],
            issues: vec![],
            rate_remaining: None,
        };
        app.update(Msg::List(Ok(snap)));
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.update(Msg::Term(Event::Key(KeyEvent::new(
            code,
            KeyModifiers::NONE,
        ))));
    }

    #[tokio::test]
    async fn first_item_selected_after_load() {
        let app = loaded_app();
        assert_eq!(app.selected.as_deref(), Some("b3"));
    }

    #[tokio::test]
    async fn j_k_move_and_clamp() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected.as_deref(), Some("a2"));
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.selected.as_deref(), Some("a1"));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected.as_deref(), Some("a1"));
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.selected.as_deref(), Some("b3"));
    }

    #[tokio::test]
    async fn enter_opens_and_esc_closes_detail() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Enter);
        assert!(app.detail_open);
        press(&mut app, KeyCode::Esc);
        assert!(!app.detail_open);
    }

    #[tokio::test]
    async fn tab_switches_and_clears_selection() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Tab);
        assert!(app.tab == Tab::Issues);
        assert_eq!(app.selected, None);
    }

    #[tokio::test]
    async fn writes_require_confirmation() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('M'));
        assert!(matches!(app.mode, Mode::Confirm(Op::Merge(_))));
        press(&mut app, KeyCode::Char('n'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.status.is_none());
    }

    #[tokio::test]
    async fn approve_ignored_on_issues_tab() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('a'));
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[tokio::test]
    async fn filter_narrows_and_esc_restores() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('b'));
        assert_eq!(app.visible().len(), 1);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visible().len(), 3);
    }

    #[tokio::test]
    async fn changed_items_are_marked_fresh_after_first_load() {
        let mut app = loaded_app();
        assert!(app.fresh.is_empty());
        let mut changed = item("a", 1, 20);
        changed.id = "a1".into();
        app.update(Msg::List(Ok(Snapshot {
            prs: vec![changed, item("a", 2, 2), item("b", 3, 3)],
            issues: vec![],
            rate_remaining: None,
        })));
        assert!(app.fresh.contains("a1"));
        assert!(!app.fresh.contains("a2"));
    }

    #[tokio::test]
    async fn failed_refresh_keeps_old_data() {
        let mut app = loaded_app();
        app.update(Msg::List(Err("boom".into())));
        assert_eq!(app.prs.len(), 3);
        assert_eq!(app.fetch_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn merged_selected_item_stays_until_navigating_away() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected.as_deref(), Some("a2"));
        let snap = Snapshot {
            prs: vec![item("a", 1, 1), item("b", 3, 3)],
            issues: vec![],
            rate_remaining: None,
        };
        app.update(Msg::List(Ok(snap)));
        assert_eq!(app.selected.as_deref(), Some("a2"));
        assert!(app.ghost_selected());
        assert_eq!(app.visible().len(), 3);
        app.label_ghost("a2", "MERGED");
        let shown = app.selected_item().unwrap();
        assert_eq!(shown.gone.as_deref(), Some("MERGED"));

        let again = Snapshot {
            prs: vec![item("a", 1, 1), item("b", 3, 3)],
            issues: vec![],
            rate_remaining: None,
        };
        app.update(Msg::List(Ok(again)));
        assert!(app.ghost_selected());
        assert_eq!(app.selected_item().unwrap().gone.as_deref(), Some("MERGED"));

        press(&mut app, KeyCode::Char('k'));
        assert!(app.ghost.is_none());
        assert_eq!(app.visible().len(), 2);
    }

    #[tokio::test]
    async fn ghost_is_dropped_on_tab_switch() {
        let mut app = loaded_app();
        app.update(Msg::List(Ok(Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2)],
            issues: vec![],
            rate_remaining: None,
        })));
        assert!(app.ghost_selected());
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert!(app.ghost.is_none());
        assert_eq!(app.prs.len(), 2);
    }

    #[tokio::test]
    async fn writes_blocked_on_ghost() {
        let mut app = loaded_app();
        app.update(Msg::List(Ok(Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2)],
            issues: vec![],
            rate_remaining: None,
        })));
        press(&mut app, KeyCode::Char('M'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.status.as_ref().is_some_and(|s| s.error));
    }

    #[tokio::test]
    async fn reopened_item_is_no_longer_a_ghost() {
        let mut app = loaded_app();
        let without = Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2)],
            issues: vec![],
            rate_remaining: None,
        };
        app.update(Msg::List(Ok(without)));
        assert!(app.ghost.is_some());
        let back = Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2), item("b", 3, 3)],
            issues: vec![],
            rate_remaining: None,
        };
        app.update(Msg::List(Ok(back)));
        assert!(app.ghost.is_none());
    }
}
