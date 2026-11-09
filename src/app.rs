use crate::claude;
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
    ClaudeDone(String, Result<claude::Outcome, String>),
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
    /// Editing the prompt for the Claude action.
    Claude,
    Help,
}

pub enum RunState {
    Running,
    Done(i32),
    Failed(String),
}

/// The Claude run for one PR. A PR has at most one at a time; a new run replaces the old result.
pub struct ClaudeRun {
    pub repo: String,
    pub number: u64,
    pub prompt: String,
    pub started: Instant,
    pub finished: Option<Instant>,
    pub state: RunState,
    pub log: std::path::PathBuf,
    pub kept_worktree: Option<std::path::PathBuf>,
    pub summary: Option<String>,
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
    /// Fake data, no network: see `pane --demo`.
    pub demo: bool,
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
    pub file_rows: Vec<u16>,
    /// The selected item after it left the open list; kept on screen until you move off it.
    pub ghost: Option<Item>,
    /// Keyed by item id.
    pub runs: HashMap<String, ClaudeRun>,
    pub fresh: HashSet<String>,
    seen: HashMap<String, chrono::DateTime<chrono::Utc>>,
    pub last_ok: Option<Instant>,
    /// When the most recent fetch began; the next scheduled one is `refresh_secs` later.
    pub last_fetch_started: Option<Instant>,
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
            demo: false,
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
            file_rows: vec![],
            ghost: None,
            runs: HashMap::new(),
            fresh: HashSet::new(),
            seen: HashMap::new(),
            last_ok: None,
            last_fetch_started: None,
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

    pub fn load_demo(&mut self) {
        let snap = crate::demo::snapshot();
        let mut details = crate::demo::details(&snap);
        let diffs = crate::demo::diffs(&snap, &mut details);
        let snap_for_runs = crate::gh::Snapshot {
            prs: snap.prs.clone(),
            issues: vec![],
            rate_remaining: None,
        };
        self.demo = true;
        self.viewer = Some("mira-k".into());
        self.rate = snap.rate_remaining;
        self.prs = snap.prs;
        self.issues = snap.issues;
        self.runs = crate::demo::runs(&snap_for_runs);
        self.details = details;
        let now = Instant::now();
        self.diffs = diffs.into_iter().map(|(k, v)| (k, (now, v))).collect();
        self.last_ok = Some(now);
        self.loading = false;
        self.ensure_selection();
    }

    pub fn running_count(&self) -> usize {
        self.runs
            .values()
            .filter(|r| matches!(r.state, RunState::Running))
            .count()
    }

    pub fn is_busy(&self, item_id: &str) -> bool {
        self.runs
            .get(item_id)
            .is_some_and(|r| matches!(r.state, RunState::Running))
    }

    /// Opens the prompt editor prefilled with `prompt`; Enter then runs it on the selected PR.
    fn edit_claude_prompt(&mut self, prompt: String) {
        if self.ghost_selected() {
            self.set_status("no longer open: nothing to do", true);
        } else if self.selected.is_some() {
            self.input = prompt;
            self.mode = Mode::Claude;
        }
    }

    fn start_claude(&mut self, task: String) {
        if self.demo {
            self.set_status("demo mode: Claude is disabled", true);
            return;
        }
        let Some(item) = self.selected_item() else {
            return;
        };
        if self.is_busy(&item.id) {
            self.set_status("claude is already running on this PR", true);
            return;
        }
        let log = claude::log_path(&item);
        self.runs.insert(
            item.id.clone(),
            ClaudeRun {
                repo: item.repo.clone(),
                number: item.number,
                prompt: task.clone(),
                started: Instant::now(),
                finished: None,
                state: RunState::Running,
                log: log.clone(),
                kept_worktree: None,
                summary: None,
            },
        );
        self.open_detail();
        let (tx, cfg, id) = (self.tx.clone(), self.cfg.clone(), item.id.clone());
        tokio::spawn(async move {
            let r = claude::run(cfg, item, task, log)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::ClaudeDone(id, r));
        });
    }

    fn on_claude_done(&mut self, item_id: String, result: Result<claude::Outcome, String>) {
        let Some(run) = self.runs.get_mut(&item_id) else {
            return;
        };
        run.finished = Some(Instant::now());
        let what = format!("{}#{}", run.repo, run.number);
        match result {
            Ok(out) => {
                run.state = RunState::Done(out.code);
                run.kept_worktree = out.kept_worktree.clone();
                run.summary = out.summary.clone();
                let head = out
                    .summary
                    .as_deref()
                    .and_then(|s| s.lines().next())
                    .map(|l| format!(": {}", crate::ui::truncate(l, 110)))
                    .unwrap_or_default();
                if out.code == 0 {
                    self.set_status(format!("claude done on {what}{head}"), false);
                    self.refresh.notify_one();
                } else {
                    let code = out.code;
                    self.set_status(format!("claude exited {code} on {what}{head}"), true);
                }
            }
            Err(e) => {
                run.state = RunState::Failed(e.clone());
                self.set_status(format!("claude could not run on {what}: {e}"), true);
            }
        }
    }

    /// Time until the next scheduled refresh; `None` when none is scheduled (demo mode, or the
    /// first fetch has not started).
    pub fn next_refresh_in(&self) -> Option<Duration> {
        if self.demo {
            return None;
        }
        let due = self.last_fetch_started? + Duration::from_secs(self.cfg.refresh_secs);
        Some(due.saturating_duration_since(Instant::now()))
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

    /// Visible items of the current tab in display order, with stacked PRs nested under their
    /// base PR. A PR whose base PR is hidden by a filter is shown at the top level.
    pub fn visible_rows(&self) -> Vec<(&Item, String)> {
        let items = self.items();
        let shown: Vec<&Item> = group_order(items)
            .into_iter()
            .map(|i| &items[i])
            .filter(|i| self.matches(i))
            .collect();
        crate::stack::arrange(&shown)
            .into_iter()
            .map(|p| (shown[p.index], p.prefix))
            .collect()
    }

    pub fn visible(&self) -> Vec<&Item> {
        self.visible_rows().into_iter().map(|(i, _)| i).collect()
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
        if self.demo {
            return;
        }
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
        if !self.demo
            && self.detail_open
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

    /// Returns whether the screen needs redrawing.
    pub fn update(&mut self, msg: Msg) -> bool {
        let tick = matches!(msg, Msg::Tick);
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
                self.last_fetch_started = Some(Instant::now());
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
            Msg::ClaudeDone(id, r) => self.on_claude_done(id, r),
        }
        !tick || self.tick_needs_redraw()
    }

    /// Ticks drive the spinner and the "updated Ns ago" label; skip redraws when nothing moves.
    fn tick_needs_redraw(&self) -> bool {
        self.last_ok.is_none()
            || self.loading
            || !self.inflight.is_empty()
            || !self.diff_inflight.is_empty()
            || self.running_count() > 0
            || self.frame.is_multiple_of(10)
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
                    self.open_link(&url);
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
            Mode::Claude => self.on_key_claude(k),
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

    fn on_key_claude(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.input.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let task = std::mem::take(&mut self.input);
                self.mode = Mode::Normal;
                if !task.trim().is_empty() {
                    self.start_claude(task);
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
        if self.demo {
            self.set_status("demo mode: actions are disabled", true);
            return;
        }
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
        // Commands are case-insensitive so Caps Lock or Shift never changes what a key does.
        let code = match k.code {
            KeyCode::Char(c) if c.is_ascii_alphabetic() => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Tab | KeyCode::BackTab => {
                self.switch_tab(if is_pr { Tab::Issues } else { Tab::Prs })
            }
            KeyCode::Char('1') => self.switch_tab(Tab::Prs),
            KeyCode::Char('2') => self.switch_tab(Tab::Issues),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Home => self.move_selection(isize::MIN / 2),
            KeyCode::End => self.move_selection(isize::MAX / 2),
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
            KeyCode::Char('u') | KeyCode::F(5) if !self.demo => {
                self.loading = true;
                self.refresh.notify_one();
            }
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('i') => {
                if self.viewer.is_none() && !self.mine {
                    self.set_status("still looking up your GitHub login", true);
                } else {
                    self.mine = !self.mine;
                    self.ensure_selection();
                }
            }
            KeyCode::Char('o') => {
                if let Some(i) = self.selected_item() {
                    self.open_link(&i.url);
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
            KeyCode::Char(']') if is_pr && self.detail_open => {
                if let Some(r) = self.file_rows.iter().find(|r| **r > self.detail_scroll) {
                    self.detail_scroll = *r;
                }
            }
            KeyCode::Char('[') if is_pr && self.detail_open => {
                if let Some(r) = self
                    .file_rows
                    .iter()
                    .rev()
                    .find(|r| **r < self.detail_scroll)
                {
                    self.detail_scroll = *r;
                }
            }
            KeyCode::Char('c') => {
                if self.ghost_selected() {
                    self.set_status("no longer open: nothing to do", true);
                } else if self.selected.is_some() {
                    self.input.clear();
                    self.mode = Mode::Comment;
                }
            }
            KeyCode::Char('r') if is_pr => self.edit_claude_prompt(self.cfg.claude_prompt.clone()),
            KeyCode::Char('t') if is_pr => {
                let open = self
                    .selected_item()
                    .and_then(|i| i.rabbit)
                    .is_some_and(|r| r.unresolved > 0);
                if open {
                    self.edit_claude_prompt(self.cfg.coderabbit_prompt.clone());
                } else {
                    self.set_status("no unresolved CodeRabbit comments on this PR", true);
                }
            }
            KeyCode::Char('x') => self.confirm(Op::Close),
            KeyCode::Char('a') if is_pr => self.confirm(Op::Approve),
            KeyCode::Char('m') if is_pr => {
                let m = self.cfg.merge_method.clone();
                self.confirm(Op::Merge(m))
            }
            KeyCode::Char('w') if is_pr => {
                let op = match self.selected_item() {
                    Some(i) if i.draft => Op::Ready,
                    _ => Op::Draft,
                };
                self.confirm(op)
            }
            _ => {}
        }
    }

    fn open_link(&mut self, url: &str) {
        if let Err(e) = open_url(url) {
            self.set_status(e, true);
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
            Some(u) => self.open_link(&u),
            None => self.set_status("no failing or pending check with a URL", false),
        }
    }
}

/// Only web links are opened: check and PR URLs come from GitHub's API and must not be able
/// to launch other URL schemes through the system opener.
pub fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn open_url(url: &str) -> Result<(), String> {
    if !is_web_url(url) {
        return Err(format!("refusing to open a non-web link: {url}"));
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(url.trim())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the link with {opener}: {e}"))
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
            head: String::new(),
            base: String::new(),
            rabbit: None,
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
        press(&mut app, KeyCode::End);
        assert_eq!(app.selected.as_deref(), Some("a1"));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected.as_deref(), Some("a1"));
        press(&mut app, KeyCode::Home);
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
        press(&mut app, KeyCode::Char('m'));
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
        press(&mut app, KeyCode::Char('m'));
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

    #[tokio::test]
    async fn claude_key_prefills_prompt_and_esc_cancels() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('r'));
        assert!(matches!(app.mode, Mode::Claude));
        assert_eq!(app.input, Config::default().claude_prompt);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.runs.is_empty());
    }

    #[tokio::test]
    async fn claude_prompt_is_editable() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('r'));
        for _ in 0..app.input.len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(app.input, "hi");
    }

    #[tokio::test]
    async fn claude_is_pr_only_and_blocked_in_demo_and_on_ghosts() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('r'));
        assert!(matches!(app.mode, Mode::Normal));

        let mut app = loaded_app();
        app.demo = true;
        press(&mut app, KeyCode::Char('r'));
        assert!(
            matches!(app.mode, Mode::Claude),
            "demo mode still shows the prompt editor"
        );
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.status.as_ref().is_some_and(|s| s.error));
        assert!(app.runs.is_empty());

        let mut app = loaded_app();
        app.update(Msg::List(Ok(Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2)],
            issues: vec![],
            rate_remaining: None,
        })));
        assert!(app.ghost_selected());
        press(&mut app, KeyCode::Char('r'));
        assert!(matches!(app.mode, Mode::Normal));
    }

    fn no_checkout_app() -> (App, tokio::sync::mpsc::UnboundedReceiver<Msg>) {
        let cfg = Config {
            repo_roots: vec![],
            ..Config::default()
        };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(cfg, tx, Arc::new(Notify::new()));
        app.update(Msg::List(Ok(Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2)],
            issues: vec![],
            rate_remaining: None,
        })));
        (app, rx)
    }

    #[tokio::test]
    async fn starting_claude_records_one_run_per_pr_and_opens_details() {
        let (mut app, mut rx) = no_checkout_app();
        assert!(!app.detail_open);
        app.start_claude("do it".into());
        assert!(app.detail_open, "the run is shown in the PR view");
        assert_eq!(app.running_count(), 1);
        assert!(app.is_busy("a2"));
        assert_eq!(app.runs["a2"].prompt, "do it");

        app.start_claude("again".into());
        assert_eq!(app.runs.len(), 1);
        assert_eq!(app.runs["a2"].prompt, "do it", "second start is refused");
        assert!(app.status.as_ref().is_some_and(|s| s.error));

        let Some(Msg::ClaudeDone(id, r)) = rx.recv().await else {
            panic!("no completion")
        };
        assert_eq!(id, "a2");
        assert!(
            r.is_err(),
            "no repo_roots, so the checkout lookup must fail"
        );
        app.update(Msg::ClaudeDone(id, r));
        assert!(!app.is_busy("a2"));
        assert!(matches!(app.runs["a2"].state, RunState::Failed(_)));
        assert!(app.runs["a2"].finished.is_some());
    }

    #[tokio::test]
    async fn different_prs_can_run_at_once_and_a_finished_pr_can_rerun() {
        let (mut app, _rx) = no_checkout_app();
        app.start_claude("first".into());
        press(&mut app, KeyCode::Char('j'));
        app.start_claude("second".into());
        assert_eq!(app.running_count(), 2);

        app.update(Msg::ClaudeDone(
            "a2".into(),
            Ok(claude::Outcome {
                code: 0,
                summary: None,
                kept_worktree: None,
            }),
        ));
        assert_eq!(app.running_count(), 1);
        press(&mut app, KeyCode::Char('k'));
        app.start_claude("third".into());
        assert_eq!(
            app.runs["a2"].prompt, "third",
            "a new run replaces the old result"
        );
        assert!(app.runs["a2"].summary.is_none());
    }

    #[tokio::test]
    async fn completion_states_set_status_and_store_results() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(Notify::new()));
        let run = || ClaudeRun {
            repo: "r".into(),
            number: 1,
            prompt: "p".into(),
            started: Instant::now(),
            finished: None,
            state: RunState::Running,
            log: "/tmp/x.log".into(),
            kept_worktree: None,
            summary: None,
        };
        app.runs.insert("x".into(), run());
        app.runs.insert("y".into(), run());
        app.update(Msg::ClaudeDone(
            "x".into(),
            Ok(claude::Outcome {
                code: 0,
                summary: Some("All checks fixed.\nDetails follow.".into()),
                kept_worktree: None,
            }),
        ));
        assert!(app.status.as_ref().is_some_and(|s| !s.error
            && s.text.contains("All checks fixed.")
            && !s.text.contains("Details")));
        assert_eq!(
            app.runs["x"].summary.as_deref(),
            Some("All checks fixed.\nDetails follow.")
        );
        app.update(Msg::ClaudeDone(
            "y".into(),
            Ok(claude::Outcome {
                code: 2,
                summary: None,
                kept_worktree: Some("/tmp/wt".into()),
            }),
        ));
        assert!(app
            .status
            .as_ref()
            .is_some_and(|s| s.error && s.text.contains("exited 2")));
        assert_eq!(
            app.runs["y"].kept_worktree.as_deref(),
            Some(std::path::Path::new("/tmp/wt"))
        );
    }

    #[tokio::test]
    async fn there_is_no_runs_list_key() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('t'));
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[tokio::test]
    async fn commands_are_case_insensitive() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('J'));
        assert_eq!(app.selected.as_deref(), Some("a2"));
        press(&mut app, KeyCode::Char('R'));
        assert!(matches!(app.mode, Mode::Claude));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('M'));
        assert!(matches!(app.mode, Mode::Confirm(Op::Merge(_))));
    }

    #[tokio::test]
    async fn text_entry_stays_case_sensitive() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('A'));
        assert_eq!(app.filter, "A");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('c'));
        press(&mut app, KeyCode::Char('Q'));
        assert_eq!(app.input, "Q");
    }

    #[tokio::test]
    async fn every_command_key_has_one_meaning() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('i'));
        assert!(!app.mine, "no login known yet");
        app.viewer = Some("will".into());
        press(&mut app, KeyCode::Char('i'));
        assert!(app.mine);
        press(&mut app, KeyCode::Char('w'));
        assert!(matches!(app.mode, Mode::Confirm(Op::Draft | Op::Ready)));
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Char('c'));
        assert!(matches!(app.mode, Mode::Comment));
    }

    #[tokio::test]
    async fn home_and_end_jump_to_the_ends() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::End);
        assert_eq!(app.selected.as_deref(), Some("a1"));
        press(&mut app, KeyCode::Home);
        assert_eq!(app.selected.as_deref(), Some("b3"));
    }

    #[test]
    fn only_web_urls_are_opened() {
        assert!(is_web_url("https://example.com/x"));
        assert!(is_web_url("  HTTP://example.com"));
        for bad in [
            "file:///etc/passwd",
            "ssh://host",
            "slack://open",
            "javascript:alert(1)",
            "",
            "example.com",
        ] {
            assert!(!is_web_url(bad), "{bad}");
            assert!(open_url(bad).is_err(), "{bad}");
        }
    }

    fn stacked_app() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(Notify::new()));
        let mut root = item("r", 1, 9);
        root.head = "root".into();
        root.base = "main".into();
        let mut mid = item("r", 2, 8);
        mid.head = "mid".into();
        mid.base = "root".into();
        let mut top = item("r", 3, 7);
        top.head = "top".into();
        top.base = "mid".into();
        let mut lone = item("r", 4, 1);
        lone.head = "lone".into();
        lone.base = "main".into();
        app.update(Msg::List(Ok(Snapshot {
            prs: vec![lone, top, root, mid],
            issues: vec![],
            rate_remaining: None,
        })));
        app
    }

    #[tokio::test]
    async fn stacked_prs_nest_and_navigation_follows_the_tree() {
        let mut app = stacked_app();
        let rows: Vec<(u64, String)> = app
            .visible_rows()
            .into_iter()
            .map(|(i, p)| (i.number, p))
            .collect();
        assert_eq!(
            rows,
            [
                (1, "".to_string()),
                (2, "└─".to_string()),
                (3, "  └─".to_string()),
                (4, "".to_string())
            ]
        );
        assert_eq!(app.selected.as_deref(), Some("r1"));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected.as_deref(), Some("r2"));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected.as_deref(), Some("r3"));
    }

    #[tokio::test]
    async fn filtering_out_the_base_pr_promotes_its_children() {
        let mut app = stacked_app();
        app.filter = "title 2".into();
        let rows = app.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "");
        app.filter = "title".into();
        app.mine = true;
        app.viewer = Some("nobody".into());
        assert!(app.visible_rows().is_empty());
    }

    fn rabbit_app(unresolved: usize) -> App {
        let mut app = loaded_app();
        for pr in app.prs.iter_mut() {
            pr.rabbit = Some(crate::gh::Rabbit {
                unresolved,
                total: unresolved.max(1),
                capped: false,
            });
        }
        app
    }

    #[tokio::test]
    async fn t_opens_the_coderabbit_prompt_when_threads_are_open() {
        let mut app = rabbit_app(2);
        press(&mut app, KeyCode::Char('t'));
        assert!(matches!(app.mode, Mode::Claude));
        assert_eq!(app.input, Config::default().coderabbit_prompt);
        assert!(app.input.contains("CodeRabbit"));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('R'));
        assert_eq!(
            app.input,
            Config::default().claude_prompt,
            "r keeps its own prompt"
        );
    }

    #[tokio::test]
    async fn t_explains_when_there_is_nothing_to_resolve() {
        for mut app in [rabbit_app(0), loaded_app()] {
            press(&mut app, KeyCode::Char('t'));
            assert!(matches!(app.mode, Mode::Normal));
            assert!(app
                .status
                .as_ref()
                .is_some_and(|s| s.error && s.text.contains("CodeRabbit")));
        }
    }

    #[tokio::test]
    async fn t_is_pr_only_and_ignores_merged_items() {
        let mut app = rabbit_app(1);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('t'));
        assert!(matches!(app.mode, Mode::Normal));

        let mut app = rabbit_app(1);
        app.update(Msg::List(Ok(Snapshot {
            prs: vec![item("a", 1, 1), item("a", 2, 2)],
            issues: vec![],
            rate_remaining: None,
        })));
        assert!(app.ghost_selected());
        press(&mut app, KeyCode::Char('t'));
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[tokio::test]
    async fn coderabbit_runs_share_the_one_run_per_pr_slot() {
        let (mut app, _rx) = no_checkout_app();
        app.prs[0].rabbit = Some(crate::gh::Rabbit {
            unresolved: 1,
            total: 1,
            capped: false,
        });
        app.prs[1].rabbit = app.prs[0].rabbit;
        app.start_claude("repair".into());
        assert!(app.is_busy("a2"));
        press(&mut app, KeyCode::Char('t'));
        assert!(
            matches!(app.mode, Mode::Claude),
            "the editor can open while busy"
        );
        app.start_claude(Config::default().coderabbit_prompt);
        assert_eq!(app.runs["a2"].prompt, "repair", "a second run is refused");
        assert!(app.status.as_ref().is_some_and(|s| s.error));
    }

    #[tokio::test]
    async fn next_refresh_counts_down_from_the_last_fetch_start() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx, Arc::new(Notify::new()));
        assert_eq!(
            app.next_refresh_in(),
            None,
            "nothing scheduled before the first fetch"
        );
        app.update(Msg::Loading);
        let next = app.next_refresh_in().unwrap();
        assert!(
            next <= Duration::from_secs(90) && next > Duration::from_secs(85),
            "{next:?}"
        );
        app.last_fetch_started = Some(Instant::now() - Duration::from_secs(100));
        assert_eq!(
            app.next_refresh_in(),
            Some(Duration::ZERO),
            "overdue clamps to zero"
        );
        app.demo = true;
        assert_eq!(app.next_refresh_in(), None);
    }
}
