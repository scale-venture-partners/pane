use crate::config::Config;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::time::Instant;
use tokio::process::Command;

const MAX_PAGES: usize = 5;
const SCOPE_BUDGET: usize = 190;
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
        id number title url updatedAt isDraft reviewDecision
        labels(first:8){nodes{name color}}
        author{login} repository{name nameWithOwner}
        commits(last:1){nodes{commit{statusCheckRollup{state}}}}
      }
      ... on Issue{
        id number title url updatedAt
        labels(first:8){nodes{name color}}
        author{login} repository{name nameWithOwner}
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
        }
    }
}

async fn search(scope: &str, kind: Kind) -> Result<(Vec<Item>, u64)> {
    if kind == Kind::Issue && scope.starts_with("review-requested:") {
        return Ok((vec![], u64::MAX));
    }
    let word = if kind == Kind::Pr { "pr" } else { "issue" };
    let q = format!("{scope} is:{word} is:open archived:false sort:updated-desc");
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
pub async fn fetch_all(cfg: &Config, on_progress: impl Fn(usize, usize)) -> Result<Snapshot> {
    let login = if cfg.orgs.is_empty() && cfg.repos.is_empty() {
        Some(viewer().await?)
    } else {
        None
    };
    let scopes = scopes(cfg, login.as_deref());
    let jobs = scopes.iter().flat_map(|s| {
        [Kind::Pr, Kind::Issue]
            .into_iter()
            .map(move |k| async move { search(s, k).await.map(|r| (k, r)) })
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
    let mut rate = u64::MAX;
    for (kind, (items, remaining)) in results {
        rate = rate.min(remaining);
        match kind {
            Kind::Pr => snap.prs.extend(items),
            Kind::Issue => snap.issues.extend(items),
        }
    }
    for v in [&mut snap.prs, &mut snap.issues] {
        v.sort_by(|a, b| a.url.cmp(&b.url));
        v.dedup_by(|a, b| a.url == b.url);
    }
    snap.rate_remaining = (rate != u64::MAX).then_some(rate);
    Ok(snap)
}

const PR_DETAIL: &str = r#"
query($id:ID!){ node(id:$id){ ... on PullRequest{
  state body headRefName baseRefName mergeable mergeStateStatus additions deletions changedFiles
  labels(first:20){nodes{name color}}
  assignees(first:10){nodes{login}}
  reviewRequests(first:10){nodes{requestedReviewer{... on User{login} ... on Team{name}}}}
  latestReviews(first:10){nodes{author{login} state}}
  comments(last:5){nodes{author{login} body createdAt}}
  commits(last:1){nodes{commit{statusCheckRollup{contexts(first:60){nodes{
    __typename
    ... on CheckRun{name status conclusion detailsUrl}
    ... on StatusContext{context state targetUrl}
  }}}}}}
}}}"#;

const ISSUE_DETAIL: &str = r#"
query($id:ID!){ node(id:$id){ ... on Issue{
  state body
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

pub async fn run_action(req: &ActionReq) -> Result<String> {
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
}
