use crate::config::Config;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;
use tokio::process::Command;

const MAX_PAGES: usize = 5;
const SCOPE_BUDGET: usize = 160;
const RABBIT_QUALIFIER: &str = "reviewed-by:app/coderabbitai";
const RABBIT_BATCH: usize = 50;
const MAX_DIFF_LINES: usize = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pr,
    Issue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    pub rgb: (u8, u8, u8),
}

const NEUTRAL_LABEL: (u8, u8, u8) = (128, 128, 128);

/// Parses GitHub's six-digit label color; anything else becomes neutral gray.
pub fn parse_hex(s: &str) -> (u8, u8, u8) {
    let s = s.trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return NEUTRAL_LABEL;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16);
    match (byte(0), byte(2), byte(4)) {
        (Ok(r), Ok(g), Ok(b)) => (r, g, b),
        _ => NEUTRAL_LABEL,
    }
}

fn parse_labels(v: &Value, path: &str) -> Vec<Label> {
    nodes(v, path)
        .iter()
        .map(|l| Label {
            name: text(l, "/name"),
            rgb: parse_hex(&text(l, "/color")),
        })
        .collect()
}

/// Review threads CodeRabbit opened on a PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rabbit {
    pub unresolved: usize,
    pub total: usize,
    /// More threads exist than were fetched, so the counts are lower bounds.
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RabbitThread {
    pub path: String,
    pub line: Option<u64>,
    pub resolved: bool,
    pub outdated: bool,
    pub severity: String,
    pub title: String,
    pub url: String,
}

pub fn is_coderabbit(login: &str) -> bool {
    login.to_ascii_lowercase().starts_with("coderabbitai")
}

fn strip_markup(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    out.replace("**", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Severity and a one-line title from a CodeRabbit review comment. Comments open with a header
/// like `Major · Stability & Availability · Heavy lift`, often followed by a bold title line.
pub fn summarize_comment(body: &str) -> (String, String) {
    let mut lines = body
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("<!--"));
    let header = strip_markup(lines.next().unwrap_or(""));
    let mut parts = header.split(" · ");
    let severity = parts.next().unwrap_or("").trim().to_string();
    let category = parts.next().unwrap_or("").trim().to_string();
    // The title is the first plain line outside any collapsed <details> block, which holds
    // CodeRabbit's analysis and script output.
    let mut depth = 0usize;
    let mut title = None;
    for line in lines {
        let t = line.trim_start();
        let opens = t.matches("<details").count();
        let closes = t.matches("</details>").count();
        depth = (depth + opens).saturating_sub(closes);
        if opens > 0 || closes > 0 || depth > 0 || t.starts_with('<') || t.starts_with("---") {
            continue;
        }
        let clean = strip_markup(t);
        if !clean.is_empty() {
            title = Some(clean);
            break;
        }
    }
    (severity, title.unwrap_or(category))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ci {
    None,
    Pending,
    Success,
    Failure,
}

impl Ci {
    pub fn from_state(s: &str) -> Ci {
        match s {
            "SUCCESS" => Ci::Success,
            "FAILURE" | "ERROR" => Ci::Failure,
            "PENDING" | "EXPECTED" => Ci::Pending,
            _ => Ci::None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub repo: String,
    pub repo_full: String,
    pub author: String,
    pub updated_at: DateTime<Utc>,
    pub draft: bool,
    pub review: Option<String>,
    pub ci: Ci,
    /// Set once an item has left the open list: `MERGED`, `CLOSED`, or `UNLISTED`.
    pub gone: Option<String>,
    pub labels: Vec<Label>,
    /// PR head and base branch names; empty for issues.
    pub head: String,
    pub base: String,
    /// Comment count; fetched for issues only, 0 for PRs.
    pub comments: u64,
    /// The repo's default branch; empty if GitHub did not say.
    pub default_branch: String,
    /// Set for PRs CodeRabbit has reviewed.
    pub rabbit: Option<Rabbit>,
}

#[derive(Debug, Default)]
pub struct Snapshot {
    pub prs: Vec<Item>,
    pub issues: Vec<Item>,
    pub rate_remaining: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub ci: Ci,
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct Detail {
    pub state: String,
    pub created_at: String,
    pub body: String,
    pub head: String,
    pub base: String,
    pub mergeable: String,
    pub merge_state: String,
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    pub labels: Vec<Label>,
    pub assignees: Vec<String>,
    pub requested: Vec<String>,
    pub reviews: Vec<(String, String)>,
    pub checks: Vec<Check>,
    pub comments: Vec<Comment>,
    pub rabbit_threads: Vec<RabbitThread>,
    pub fetched: Instant,
}

#[derive(Debug, Clone)]
pub enum Op {
    Approve,
    Comment(String),
    Merge(String),
    Close,
    Ready,
    Draft,
}

#[derive(Debug, Clone)]
pub struct ActionReq {
    pub kind: Kind,
    pub repo_full: String,
    pub number: u64,
    pub op: Op,
    /// For merges into a non-default branch: the base branch must be the head of an open PR
    /// (checked live) or the merge is refused.
    pub merge_guard: Option<MergeGuard>,
}

#[derive(Debug, Clone)]
pub struct MergeGuard {
    pub base: String,
    pub default_branch: String,
}

async fn graphql(query: &str, vars: &[(&str, &str)]) -> Result<Value> {
    let mut cmd = Command::new("gh");
    cmd.args(["api", "graphql", "-f"])
        .arg(format!("query={query}"));
    for (k, v) in vars {
        cmd.arg("-f").arg(format!("{k}={v}"));
    }
    let out = cmd.output().await.context("running gh")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let body = String::from_utf8_lossy(&out.stdout);
        bail!("gh api graphql failed: {} {}", err.trim(), body.trim());
    }
    let v: Value = serde_json::from_slice(&out.stdout).context("parsing gh output")?;
    if let Some(errs) = v.get("errors") {
        bail!("graphql: {errs}");
    }
    Ok(v)
}

pub async fn viewer() -> Result<String> {
    let out = Command::new("gh")
        .args(["api", "user", "-q", ".login"])
        .output()
        .await
        .context("running gh")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Search scopes: each org, plus repos chunked to fit GitHub's 256-char query limit.
/// With no orgs or repos configured, falls back to items involving `login`.
pub fn scopes(cfg: &Config, login: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = cfg.orgs.iter().map(|o| format!("org:{o}")).collect();
    let mut cur = String::new();
    for r in &cfg.repos {
        let q = format!("repo:{r}");
        if !cur.is_empty() && cur.len() + 1 + q.len() > SCOPE_BUDGET {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&q);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        if let Some(l) = login {
            out.push(format!("involves:{l}"));
            out.push(format!("review-requested:{l}"));
        }
    }
    out
}

const LIST_QUERY: &str = r#"
query($q:String!,$after:String){
  rateLimit{remaining}
  search(query:$q,type:ISSUE,first:100,after:$after){
    pageInfo{hasNextPage endCursor}
    nodes{
      __typename
      ... on PullRequest{
        id number title url updatedAt isDraft reviewDecision headRefName baseRefName
        labels(first:8){nodes{name color}}
        author{login} repository{name nameWithOwner defaultBranchRef{name}}
        commits(last:1){nodes{commit{statusCheckRollup{state}}}}
      }
      ... on Issue{
        id number title url updatedAt
        comments{totalCount}
        labels(first:8){nodes{name color}}
        author{login} repository{name nameWithOwner defaultBranchRef{name}}
      }
    }
  }
}"#;

#[derive(Deserialize)]
struct ListResp {
    data: ListData,
}
#[derive(Deserialize)]
struct ListData {
    #[serde(rename = "rateLimit")]
    rate_limit: RateLimit,
    search: Search,
}
#[derive(Deserialize)]
struct RateLimit {
    remaining: u64,
}
#[derive(Deserialize)]
struct Search {
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    id: String,
    number: u64,
    title: String,
    url: String,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    is_draft: bool,
    review_decision: Option<String>,
    author: Option<Login>,
    repository: Repo,
    commits: Option<Value>,
    labels: Option<Value>,
    head_ref_name: Option<String>,
    base_ref_name: Option<String>,
    comments: Option<Count>,
}

#[derive(Deserialize)]
struct Count {
    #[serde(rename = "totalCount")]
    total_count: u64,
}
#[derive(Deserialize)]
struct Login {
    login: String,
}
#[derive(Deserialize)]
struct Repo {
    name: String,
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    #[serde(rename = "defaultBranchRef")]
    default_branch_ref: Option<BranchRef>,
}

#[derive(Deserialize)]
struct BranchRef {
    name: String,
}

impl Node {
    fn into_item(self, kind: Kind) -> Item {
        let labels = self
            .labels
            .as_ref()
            .map(|l| parse_labels(l, "/nodes"))
            .unwrap_or_default();
        let ci = self
            .commits
            .as_ref()
            .and_then(|c| c.pointer("/nodes/0/commit/statusCheckRollup/state"))
            .and_then(Value::as_str)
            .map(Ci::from_state)
            .unwrap_or(Ci::None);
        Item {
            id: self.id,
            kind,
            number: self.number,
            title: self.title,
            url: self.url,
            repo: self.repository.name,
            repo_full: self.repository.name_with_owner,
            author: self
                .author
                .map(|a| a.login)
                .unwrap_or_else(|| "ghost".into()),
            updated_at: self.updated_at,
            draft: self.is_draft,
            review: self.review_decision,
            ci,
            gone: None,
            labels,
            head: self.head_ref_name.unwrap_or_default(),
            base: self.base_ref_name.unwrap_or_default(),
            comments: self.comments.map_or(0, |c| c.total_count),
            default_branch: self
                .repository
                .default_branch_ref
                .map(|b| b.name)
                .unwrap_or_default(),
            rabbit: None,
        }
    }
}

async fn search(scope: &str, kind: Kind, extra: &str) -> Result<(Vec<Item>, u64)> {
    if kind == Kind::Issue && scope.starts_with("review-requested:") {
        return Ok((vec![], u64::MAX));
    }
    let word = if kind == Kind::Pr { "pr" } else { "issue" };
    let q = format!("{scope} is:{word} is:open archived:false {extra} sort:updated-desc");
    let mut items = Vec::new();
    let mut after = String::new();
    let mut remaining = u64::MAX;
    for _ in 0..MAX_PAGES {
        let mut vars = vec![("q", q.as_str())];
        if !after.is_empty() {
            vars.push(("after", after.as_str()));
        }
        let v = graphql(LIST_QUERY, &vars).await?;
        let resp: ListResp = serde_json::from_value(v).context("decoding search page")?;
        remaining = remaining.min(resp.data.rate_limit.remaining);
        items.extend(
            resp.data
                .search
                .nodes
                .into_iter()
                .map(|n| n.into_item(kind)),
        );
        if !resp.data.search.page_info.has_next {
            break;
        }
        match resp.data.search.page_info.end_cursor {
            Some(c) => after = c,
            None => break,
        }
    }
    Ok((items, remaining))
}

/// `on_progress(done, total)` is called as each search query finishes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Prs,
    Issues,
    /// PRs CodeRabbit has reviewed; only their ids are used.
    Rabbit,
}

const RABBIT_THREADS: &str = r#"
query($ids:[ID!]!){ nodes(ids:$ids){ ... on PullRequest{
  id
  reviewThreads(first:100){ totalCount nodes{
    isResolved
    comments(first:1){nodes{author{login}}}
  }}
}}}"#;

/// A PR id and its CodeRabbit thread counts from one `nodes(ids:)` result entry.
fn rabbit_from_node(n: &Value) -> Option<(String, Rabbit)> {
    let id = text(n, "/id");
    if id.is_empty() {
        return None;
    }
    let threads = nodes(n, "/reviewThreads/nodes");
    let mine: Vec<&Value> = threads
        .iter()
        .filter(|t| is_coderabbit(&text(t, "/comments/nodes/0/author/login")))
        .collect();
    let unresolved = mine
        .iter()
        .filter(|t| t.pointer("/isResolved").and_then(Value::as_bool) != Some(true))
        .count();
    Some((
        id,
        Rabbit {
            unresolved,
            total: mine.len(),
            capped: num(n, "/reviewThreads/totalCount") > threads.len() as u64,
        },
    ))
}

/// CodeRabbit thread counts for the given PR ids.
async fn rabbit_counts(ids: &[String]) -> Result<HashMap<String, Rabbit>> {
    let mut out = HashMap::new();
    for chunk in ids.chunks(RABBIT_BATCH) {
        let mut cmd_vars: Vec<(String, String)> = vec![];
        for id in chunk {
            cmd_vars.push(("ids[]".to_string(), id.clone()));
        }
        let vars: Vec<(&str, &str)> = cmd_vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let v = graphql(RABBIT_THREADS, &vars).await?;
        for n in nodes(&v, "/data/nodes") {
            if let Some((id, rabbit)) = rabbit_from_node(n) {
                out.insert(id, rabbit);
            }
        }
    }
    Ok(out)
}

/// `on_progress(done, total)` is called as each search query finishes.
pub async fn fetch_all(cfg: &Config, on_progress: impl Fn(usize, usize)) -> Result<Snapshot> {
    let login = if cfg.orgs.is_empty() && cfg.repos.is_empty() {
        Some(viewer().await?)
    } else {
        None
    };
    fetch_scopes(&scopes(cfg, login.as_deref()), on_progress).await
}

/// Everything open in one repo (`owner/name`): used to refresh just that repo after an action.
pub async fn fetch_repo(repo_full: &str) -> Result<Snapshot> {
    fetch_scopes(&[format!("repo:{repo_full}")], |_, _| {}).await
}

async fn fetch_scopes(scopes: &[String], on_progress: impl Fn(usize, usize)) -> Result<Snapshot> {
    let jobs = scopes.iter().flat_map(|s| {
        [
            (Pass::Prs, Kind::Pr, ""),
            (Pass::Issues, Kind::Issue, ""),
            (Pass::Rabbit, Kind::Pr, RABBIT_QUALIFIER),
        ]
        .into_iter()
        .map(move |(pass, kind, extra)| async move {
            search(s, kind, extra).await.map(|r| (pass, r))
        })
    });
    let mut pending: FuturesUnordered<_> = jobs.collect();
    let total = pending.len();
    on_progress(0, total);
    let mut results = Vec::with_capacity(total);
    while let Some(r) = pending.next().await {
        results.push(r?);
        on_progress(results.len(), total);
    }
    let mut snap = Snapshot::default();
    let mut rabbit_ids: Vec<String> = vec![];
    let mut rate = u64::MAX;
    for (pass, (items, remaining)) in results {
        rate = rate.min(remaining);
        match pass {
            Pass::Prs => snap.prs.extend(items),
            Pass::Issues => snap.issues.extend(items),
            Pass::Rabbit => rabbit_ids.extend(items.into_iter().map(|i| i.id)),
        }
    }
    for v in [&mut snap.prs, &mut snap.issues] {
        v.sort_by(|a, b| a.url.cmp(&b.url));
        v.dedup_by(|a, b| a.url == b.url);
    }
    rabbit_ids.sort();
    rabbit_ids.dedup();
    rabbit_ids.retain(|id| snap.prs.iter().any(|p| &p.id == id));
    let counts = rabbit_counts(&rabbit_ids).await?;
    for pr in &mut snap.prs {
        pr.rabbit = counts.get(&pr.id).copied();
    }
    snap.rate_remaining = (rate != u64::MAX).then_some(rate);
    Ok(snap)
}

const PR_DETAIL: &str = r#"
query($id:ID!){ node(id:$id){ ... on PullRequest{
  state createdAt body headRefName baseRefName mergeable mergeStateStatus additions deletions changedFiles
  labels(first:20){nodes{name color}}
  assignees(first:10){nodes{login}}
  reviewRequests(first:10){nodes{requestedReviewer{... on User{login} ... on Team{name}}}}
  latestReviews(first:10){nodes{author{login} state}}
  comments(last:5){nodes{author{login} body createdAt}}
  reviewThreads(first:100){nodes{
    isResolved isOutdated path line
    comments(first:1){nodes{url body author{login}}}
  }}
  commits(last:1){nodes{commit{statusCheckRollup{contexts(first:60){nodes{
    __typename
    ... on CheckRun{name status conclusion detailsUrl}
    ... on StatusContext{context state targetUrl}
  }}}}}}
}}}"#;

const ISSUE_DETAIL: &str = r#"
query($id:ID!){ node(id:$id){ ... on Issue{
  state createdAt body
  labels(first:20){nodes{name color}}
  assignees(first:10){nodes{login}}
  comments(last:5){nodes{author{login} body createdAt}}
}}}"#;

fn text(v: &Value, p: &str) -> String {
    v.pointer(p)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn num(v: &Value, p: &str) -> u64 {
    v.pointer(p).and_then(Value::as_u64).unwrap_or(0)
}

fn nodes<'a>(v: &'a Value, p: &str) -> &'a [Value] {
    v.pointer(p)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn rabbit_threads(n: &Value) -> Vec<RabbitThread> {
    nodes(n, "/reviewThreads/nodes")
        .iter()
        .filter(|t| is_coderabbit(&text(t, "/comments/nodes/0/author/login")))
        .map(|t| {
            let (severity, title) = summarize_comment(&text(t, "/comments/nodes/0/body"));
            RabbitThread {
                path: text(t, "/path"),
                line: t.pointer("/line").and_then(Value::as_u64),
                resolved: t.pointer("/isResolved").and_then(Value::as_bool) == Some(true),
                outdated: t.pointer("/isOutdated").and_then(Value::as_bool) == Some(true),
                severity,
                title,
                url: text(t, "/comments/nodes/0/url"),
            }
        })
        .collect()
}

pub fn check_from_run(status: &str, conclusion: &str) -> Ci {
    if status != "COMPLETED" {
        return Ci::Pending;
    }
    match conclusion {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => Ci::Success,
        _ => Ci::Failure,
    }
}

pub async fn fetch_detail(item: &Item) -> Result<Detail> {
    let q = if item.kind == Kind::Pr {
        PR_DETAIL
    } else {
        ISSUE_DETAIL
    };
    let v = graphql(q, &[("id", item.id.as_str())]).await?;
    let n = v
        .pointer("/data/node")
        .context("node missing in response")?;

    let checks = nodes(
        n,
        "/commits/nodes/0/commit/statusCheckRollup/contexts/nodes",
    )
    .iter()
    .map(|c| match text(c, "/__typename").as_str() {
        "CheckRun" => Check {
            name: text(c, "/name"),
            ci: check_from_run(&text(c, "/status"), &text(c, "/conclusion")),
            url: Some(text(c, "/detailsUrl")).filter(|u| !u.is_empty()),
        },
        _ => Check {
            name: text(c, "/context"),
            ci: Ci::from_state(&text(c, "/state")),
            url: Some(text(c, "/targetUrl")).filter(|u| !u.is_empty()),
        },
    })
    .collect();

    let requested = nodes(n, "/reviewRequests/nodes")
        .iter()
        .map(|r| {
            let login = text(r, "/requestedReviewer/login");
            if login.is_empty() {
                text(r, "/requestedReviewer/name")
            } else {
                login
            }
        })
        .collect();

    Ok(Detail {
        state: text(n, "/state"),
        created_at: text(n, "/createdAt"),
        body: text(n, "/body"),
        head: text(n, "/headRefName"),
        base: text(n, "/baseRefName"),
        mergeable: text(n, "/mergeable"),
        merge_state: text(n, "/mergeStateStatus"),
        additions: num(n, "/additions"),
        deletions: num(n, "/deletions"),
        files: num(n, "/changedFiles"),
        labels: parse_labels(n, "/labels/nodes"),
        assignees: nodes(n, "/assignees/nodes")
            .iter()
            .map(|a| text(a, "/login"))
            .collect(),
        requested,
        reviews: nodes(n, "/latestReviews/nodes")
            .iter()
            .map(|r| (text(r, "/author/login"), text(r, "/state")))
            .collect(),
        checks,
        rabbit_threads: rabbit_threads(n),
        comments: nodes(n, "/comments/nodes")
            .iter()
            .map(|c| Comment {
                author: text(c, "/author/login"),
                body: text(c, "/body"),
                created_at: text(c, "/createdAt"),
            })
            .collect(),
        fetched: Instant::now(),
    })
}

/// Keeps the first `max` lines and appends a notice when the diff is longer.
pub fn cap_diff(diff: &str, max: usize) -> String {
    let total = diff.lines().count();
    if total <= max {
        return diff.to_string();
    }
    let mut out: String = diff.lines().take(max).collect::<Vec<_>>().join("\n");
    out.push_str(&format!(
        "\n… diff truncated, {} more lines (press o to view all on GitHub)",
        total - max
    ));
    out
}

pub async fn fetch_diff(item: &Item) -> Result<String> {
    let out = Command::new("gh")
        .args([
            "pr",
            "diff",
            &item.number.to_string(),
            "-R",
            &item.repo_full,
        ])
        .args(["--color", "never"])
        .output()
        .await
        .context("running gh")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(cap_diff(
        &String::from_utf8_lossy(&out.stdout),
        MAX_DIFF_LINES,
    ))
}

pub fn action_args(req: &ActionReq) -> Vec<String> {
    let sub = if req.kind == Kind::Pr { "pr" } else { "issue" };
    let n = req.number.to_string();
    let mut a: Vec<String> = match &req.op {
        Op::Approve => vec!["pr", "review", &n, "--approve"],
        Op::Comment(_) => vec![sub, "comment", &n],
        Op::Merge(_) => vec!["pr", "merge", &n],
        Op::Close => vec![sub, "close", &n],
        Op::Ready | Op::Draft => vec!["pr", "ready", &n],
    }
    .into_iter()
    .map(String::from)
    .collect();
    a.extend(["-R".into(), req.repo_full.clone()]);
    match &req.op {
        Op::Comment(body) => a.extend(["--body".into(), body.clone()]),
        Op::Merge(method) => a.push(format!("--{method}")),
        Op::Draft => a.push("--undo".into()),
        _ => {}
    }
    a
}

const OPEN_PR_WITH_HEAD: &str = r#"
query($owner:String!,$name:String!,$head:String!){
  repository(owner:$owner,name:$name){
    pullRequests(first:1,states:OPEN,headRefName:$head){nodes{number}}
  }
}"#;

/// Number of an open PR whose head branch is `head`, checked live against GitHub.
pub async fn open_pr_with_head(repo_full: &str, head: &str) -> Result<Option<u64>> {
    let (owner, name) = repo_full
        .split_once('/')
        .context("repo must be owner/name")?;
    let v = graphql(
        OPEN_PR_WITH_HEAD,
        &[("owner", owner), ("name", name), ("head", head)],
    )
    .await?;
    Ok(first_pr_number(&v))
}

fn first_pr_number(v: &Value) -> Option<u64> {
    nodes(v, "/data/repository/pullRequests/nodes")
        .first()
        .map(|n| num(n, "/number"))
}

pub fn merge_refusal(base: &str, default_branch: &str) -> String {
    let target = if default_branch.is_empty() {
        "the default branch"
    } else {
        default_branch
    };
    format!(
        "can't merge into `{base}`: no open PR has it as its head (commits would be orphaned); \
         retarget to {target} first"
    )
}

pub async fn run_action(req: &ActionReq) -> Result<String> {
    if let (Op::Merge(_), Some(guard)) = (&req.op, &req.merge_guard) {
        let base = &guard.base;
        match open_pr_with_head(&req.repo_full, base).await {
            Ok(Some(_)) => {}
            Ok(None) => bail!("{}", merge_refusal(base, &guard.default_branch)),
            Err(e) => bail!("couldn't verify that `{base}` has an open PR, so not merging: {e:#}"),
        }
    }
    let out = Command::new("gh")
        .args(action_args(req))
        .output()
        .await
        .context("running gh")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn many_repos() -> Config {
        Config {
            orgs: vec!["acme".into()],
            repos: (0..40)
                .map(|n| format!("example-org/service-number-{n}"))
                .collect(),
            ..Config::default()
        }
    }

    #[test]
    fn scopes_fit_query_limit() {
        let s = scopes(&many_repos(), None);
        assert!(s.len() > 2);
        assert_eq!(s[0], "org:acme");
        for scope in &s {
            let q = format!("{scope} is:issue is:open archived:false sort:updated-desc");
            assert!(q.len() <= 256, "{} chars: {q}", q.len());
        }
    }

    #[test]
    fn rabbit_query_fits_the_search_length_limit() {
        for scope in scopes(&many_repos(), None) {
            let q = format!(
                "{scope} is:pr is:open archived:false {RABBIT_QUALIFIER} sort:updated-desc"
            );
            assert!(q.len() <= 256, "{} chars: {q}", q.len());
        }
    }

    #[test]
    fn coderabbit_logins_match_bot_forms_only() {
        assert!(is_coderabbit("coderabbitai"));
        assert!(is_coderabbit("coderabbitai[bot]"));
        assert!(is_coderabbit("CodeRabbitAI"));
        assert!(!is_coderabbit("octocat"));
        assert!(!is_coderabbit(""));
    }

    #[test]
    fn comment_summary_reads_header_and_bold_title() {
        let body = "<!-- coderabbit-ai-generated:v1 -->\n\n**<picture><img src=\"x.svg\" alt=\"\"></picture> Minor** · **Functional Correctness** · **Quick win**\n\n**Add a visible fallback link.**\n\n`window.open` runs late.";
        assert_eq!(
            summarize_comment(body),
            (
                "Minor".to_string(),
                "Add a visible fallback link.".to_string()
            )
        );
    }

    #[test]
    fn comment_summary_skips_collapsed_analysis_blocks() {
        let body = "<!-- coderabbit-ai-generated:v1 -->\n\n**<picture><img src=\"x.svg\"></picture> Minor** · **Security & Privacy** · **Quick win**\n\n<details>\n<summary>🧩 Analysis chain</summary>\n\nScript executed:\n\n```bash\nrg foo\n```\n\n---\n\n</details>\n\n**Validate the redirect target.**\n\nOpen redirects are risky.";
        assert_eq!(
            summarize_comment(body),
            (
                "Minor".to_string(),
                "Validate the redirect target.".to_string()
            )
        );
    }

    #[test]
    fn comment_summary_handles_nested_details() {
        let body = "**Major** · **Cat** · **x**\n<details>\n<details>\ninner\n</details>\nstill inside\n</details>\nReal title";
        assert_eq!(
            summarize_comment(body),
            ("Major".to_string(), "Real title".to_string())
        );
    }

    #[test]
    fn comment_summary_falls_back_to_the_category() {
        let body = "<!-- coderabbit-ai-generated:v1 -->\n\n**<picture><img src=\"x.svg\"></picture> Major** · **Stability & Availability** · **Heavy lift**\n\n<details>\n<summary>Review details</summary>\n</details>";
        assert_eq!(
            summarize_comment(body),
            ("Major".to_string(), "Stability & Availability".to_string())
        );
        assert_eq!(summarize_comment(""), (String::new(), String::new()));
    }

    #[test]
    fn rabbit_counts_only_coderabbit_threads() {
        let n = serde_json::json!({
            "id": "PR_1",
            "reviewThreads": { "totalCount": 4, "nodes": [
                {"isResolved": false, "comments": {"nodes": [{"author": {"login": "coderabbitai"}}]}},
                {"isResolved": true,  "comments": {"nodes": [{"author": {"login": "coderabbitai"}}]}},
                {"isResolved": false, "comments": {"nodes": [{"author": {"login": "human"}}]}},
                {"isResolved": false, "comments": {"nodes": [{"author": {"login": "coderabbitai"}}]}}
            ]}
        });
        let (id, r) = rabbit_from_node(&n).unwrap();
        assert_eq!(id, "PR_1");
        assert_eq!((r.unresolved, r.total, r.capped), (2, 3, false));
        assert!(rabbit_from_node(&serde_json::json!({})).is_none());
    }

    #[test]
    fn rabbit_counts_flag_truncated_thread_lists() {
        let n = serde_json::json!({
            "id": "PR_2",
            "reviewThreads": { "totalCount": 150, "nodes": [
                {"isResolved": false, "comments": {"nodes": [{"author": {"login": "coderabbitai"}}]}}
            ]}
        });
        assert!(rabbit_from_node(&n).unwrap().1.capped);
    }

    #[test]
    fn detail_threads_keep_location_state_and_url() {
        let n = serde_json::json!({
            "reviewThreads": { "nodes": [
                {"isResolved": false, "isOutdated": true, "path": "a.rs", "line": 12,
                 "comments": {"nodes": [{"url": "https://x/1", "author": {"login": "coderabbitai"},
                   "body": "**<img alt=\"\"> Major** · **Security**\n\n**Sanitize the path.**"}]}},
                {"isResolved": false, "path": "b.rs", "line": null,
                 "comments": {"nodes": [{"url": "https://x/2", "author": {"login": "human"}, "body": "hi"}]}}
            ]}
        });
        let threads = rabbit_threads(&n);
        assert_eq!(threads.len(), 1);
        let t = &threads[0];
        assert_eq!(
            (t.path.as_str(), t.line, t.outdated, t.resolved),
            ("a.rs", Some(12), true, false)
        );
        assert_eq!(
            (t.severity.as_str(), t.title.as_str(), t.url.as_str()),
            ("Major", "Sanitize the path.", "https://x/1")
        );
    }

    #[test]
    fn scopes_cover_every_repo() {
        let cfg = many_repos();
        let joined = scopes(&cfg, None).join(" ");
        for r in &cfg.repos {
            assert!(joined.contains(&format!("repo:{r}")));
        }
    }

    #[test]
    fn empty_config_uses_involvement() {
        let s = scopes(&Config::default(), Some("octocat"));
        assert_eq!(s, ["involves:octocat", "review-requested:octocat"]);
    }

    #[test]
    fn configured_scopes_ignore_login() {
        let s = scopes(&many_repos(), Some("octocat"));
        assert!(s.iter().all(|x| !x.contains("octocat")));
    }

    #[test]
    fn list_response_maps_to_items() {
        let json = serde_json::json!({
            "data": {
                "rateLimit": {"remaining": 4000},
                "search": {
                    "pageInfo": {"hasNextPage": false, "endCursor": null},
                    "nodes": [{
                        "__typename": "PullRequest",
                        "id": "PR_1", "number": 7, "title": "Add thing",
                        "url": "https://github.com/acme/tool/pull/7",
                        "updatedAt": "2026-01-02T03:04:05Z",
                        "isDraft": true, "reviewDecision": "APPROVED",
                        "author": {"login": "octocat"},
                        "repository": {"name": "tool", "nameWithOwner": "acme/tool"},
                        "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]}
                    }, {
                        "__typename": "Issue",
                        "id": "I_2", "number": 8, "title": "Bug",
                        "url": "https://github.com/acme/tool/issues/8",
                        "updatedAt": "2026-01-02T03:04:05Z",
                        "author": null,
                        "repository": {"name": "tool", "nameWithOwner": "acme/tool"}
                    }]
                }
            }
        });
        let resp: ListResp = serde_json::from_value(json).unwrap();
        let mut nodes = resp.data.search.nodes.into_iter();
        let pr = nodes.next().unwrap().into_item(Kind::Pr);
        assert!(pr.draft);
        assert_eq!(pr.ci, Ci::Failure);
        assert_eq!(pr.review.as_deref(), Some("APPROVED"));
        let issue = nodes.next().unwrap().into_item(Kind::Issue);
        assert_eq!(issue.author, "ghost");
        assert_eq!(issue.ci, Ci::None);
    }

    #[test]
    fn rollup_states() {
        assert_eq!(Ci::from_state("SUCCESS"), Ci::Success);
        assert_eq!(Ci::from_state("ERROR"), Ci::Failure);
        assert_eq!(Ci::from_state("EXPECTED"), Ci::Pending);
        assert_eq!(Ci::from_state("???"), Ci::None);
    }

    #[test]
    fn comment_and_close_args() {
        let req = ActionReq {
            kind: Kind::Issue,
            repo_full: "o/r".into(),
            number: 3,
            op: Op::Comment("hi".into()),
            merge_guard: None,
        };
        assert_eq!(
            action_args(&req),
            ["issue", "comment", "3", "-R", "o/r", "--body", "hi"]
        );
        let req = ActionReq {
            op: Op::Close,
            ..req
        };
        assert_eq!(action_args(&req), ["issue", "close", "3", "-R", "o/r"]);
    }

    #[test]
    fn cap_diff_truncates_with_notice() {
        let diff = "a\nb\nc\nd";
        assert_eq!(cap_diff(diff, 10), diff);
        let capped = cap_diff(diff, 2);
        assert!(capped.starts_with("a\nb\n"));
        assert!(capped.contains("2 more lines"));
    }

    #[test]
    fn check_run_mapping() {
        assert_eq!(check_from_run("IN_PROGRESS", ""), Ci::Pending);
        assert_eq!(check_from_run("COMPLETED", "SKIPPED"), Ci::Success);
        assert_eq!(check_from_run("COMPLETED", "TIMED_OUT"), Ci::Failure);
    }

    #[test]
    fn merge_args() {
        let req = ActionReq {
            kind: Kind::Pr,
            repo_full: "o/r".into(),
            number: 7,
            op: Op::Merge("squash".into()),
            merge_guard: None,
        };
        assert_eq!(
            action_args(&req),
            ["pr", "merge", "7", "-R", "o/r", "--squash"]
        );
    }

    #[test]
    fn draft_args_undo() {
        let req = ActionReq {
            kind: Kind::Pr,
            repo_full: "o/r".into(),
            number: 7,
            op: Op::Draft,
            merge_guard: None,
        };
        assert_eq!(
            action_args(&req),
            ["pr", "ready", "7", "-R", "o/r", "--undo"]
        );
    }

    #[test]
    fn hex_colors_parse() {
        assert_eq!(parse_hex("d73a4a"), (0xd7, 0x3a, 0x4a));
        assert_eq!(parse_hex("#000000"), (0, 0, 0));
        assert_eq!(parse_hex("nope"), NEUTRAL_LABEL);
        assert_eq!(parse_hex("zzzzzz"), NEUTRAL_LABEL);
        assert_eq!(parse_hex("ééé"), NEUTRAL_LABEL);
    }

    #[test]
    fn labels_come_through_the_list_response() {
        let json = serde_json::json!({
            "id": "I_1", "number": 1, "title": "t", "url": "u",
            "updatedAt": "2026-01-02T03:04:05Z",
            "author": {"login": "a"},
            "repository": {"name": "r", "nameWithOwner": "o/r"},
            "labels": {"nodes": [{"name": "bug", "color": "d73a4a"}]}
        });
        let node: Node = serde_json::from_value(json).unwrap();
        let item = node.into_item(Kind::Issue);
        assert_eq!(
            item.labels,
            [Label {
                name: "bug".into(),
                rgb: (0xd7, 0x3a, 0x4a)
            }]
        );
    }

    #[test]
    fn open_pr_lookup_reads_the_first_node() {
        let none = serde_json::json!({"data": {"repository": {"pullRequests": {"nodes": []}}}});
        assert_eq!(first_pr_number(&none), None);
        let one = serde_json::json!({"data": {"repository": {"pullRequests": {"nodes": [{"number": 7}]}}}});
        assert_eq!(first_pr_number(&one), Some(7));
        assert_eq!(first_pr_number(&serde_json::json!({})), None);
    }

    #[test]
    fn refusal_names_the_branch_and_the_fix() {
        let m = merge_refusal("feat/a", "develop");
        assert!(
            m.contains("`feat/a`")
                && m.contains("orphaned")
                && m.contains("retarget to develop first")
        );
        assert!(merge_refusal("x", "").contains("the default branch"));
    }

    #[test]
    fn list_nodes_carry_the_default_branch() {
        let json = serde_json::json!({
            "id": "PR_1", "number": 1, "title": "t", "url": "u",
            "updatedAt": "2026-01-02T03:04:05Z",
            "repository": {"name": "r", "nameWithOwner": "o/r", "defaultBranchRef": {"name": "develop"}}
        });
        let item = serde_json::from_value::<Node>(json)
            .unwrap()
            .into_item(Kind::Pr);
        assert_eq!(item.default_branch, "develop");
        let bare = serde_json::json!({
            "id": "PR_2", "number": 2, "title": "t", "url": "u",
            "updatedAt": "2026-01-02T03:04:05Z",
            "repository": {"name": "r", "nameWithOwner": "o/r"}
        });
        assert_eq!(
            serde_json::from_value::<Node>(bare)
                .unwrap()
                .into_item(Kind::Pr)
                .default_branch,
            ""
        );
    }

    #[test]
    fn issue_nodes_carry_a_comment_count() {
        let json = serde_json::json!({
            "id": "I_1", "number": 4, "title": "t", "url": "u",
            "updatedAt": "2026-01-02T03:04:05Z",
            "comments": {"totalCount": 7},
            "repository": {"name": "r", "nameWithOwner": "o/r"}
        });
        let issue = serde_json::from_value::<Node>(json)
            .unwrap()
            .into_item(Kind::Issue);
        assert_eq!(issue.comments, 7);
        let none = serde_json::json!({
            "id": "PR_1", "number": 5, "title": "t", "url": "u",
            "updatedAt": "2026-01-02T03:04:05Z",
            "repository": {"name": "r", "nameWithOwner": "o/r"}
        });
        assert_eq!(
            serde_json::from_value::<Node>(none)
                .unwrap()
                .into_item(Kind::Pr)
                .comments,
            0
        );
    }
}
