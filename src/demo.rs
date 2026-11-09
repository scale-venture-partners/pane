//! Fictional data for `pane --demo`: lets anyone try the UI, and powers the README screenshots.

use crate::app::{ClaudeRun, RunState};
use crate::diff;
use crate::gh::{parse_hex, Check, Ci, Comment, Detail, Item, Kind, Label, Snapshot};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use std::time::{Duration as StdDuration, Instant};

const ORG: &str = "acme";

fn label(name: &str, color: &str) -> Label {
    Label {
        name: name.into(),
        rgb: parse_hex(color),
    }
}

struct Spec {
    repo: &'static str,
    number: u64,
    title: &'static str,
    author: &'static str,
    minutes_ago: i64,
    draft: bool,
    review: Option<&'static str>,
    ci: Ci,
    labels: Vec<Label>,
}

fn pr(s: Spec) -> Item {
    item(Kind::Pr, s)
}

fn issue(s: Spec) -> Item {
    item(Kind::Issue, s)
}

fn item(kind: Kind, s: Spec) -> Item {
    let path = if kind == Kind::Pr { "pull" } else { "issues" };
    let tag = if kind == Kind::Pr { "pr" } else { "issue" };
    Item {
        id: format!("demo-{tag}-{}-{}", s.repo, s.number),
        kind,
        number: s.number,
        title: s.title.into(),
        url: format!("https://github.com/{ORG}/{}/{path}/{}", s.repo, s.number),
        repo: s.repo.into(),
        repo_full: format!("{ORG}/{}", s.repo),
        author: s.author.into(),
        updated_at: Utc::now() - Duration::minutes(s.minutes_ago),
        draft: s.draft,
        review: s.review.map(str::to_string),
        ci: s.ci,
        gone: None,
        labels: s.labels,
        head: if kind == Kind::Pr && s.number == 482 {
            "mira/upload-rate-limit".to_string()
        } else if kind == Kind::Pr {
            format!(
                "{}/change-{}",
                s.author.split('-').next().unwrap_or("dev"),
                s.number
            )
        } else {
            String::new()
        },
        base: if kind == Kind::Pr {
            "main".into()
        } else {
            String::new()
        },
    }
}

/// Makes PR `child` target the head branch of PR `parent`, like a stacked PR.
fn stack_on(prs: &mut [Item], child: u64, parent: u64) {
    let head = prs
        .iter()
        .find(|p| p.number == parent)
        .map(|p| p.head.clone())
        .unwrap_or_default();
    if let Some(c) = prs.iter_mut().find(|p| p.number == child) {
        c.base = head;
    }
}

pub fn snapshot() -> Snapshot {
    let feature = || label("feature", "0e8a16");
    let bug = || label("bug", "d73a4a");
    let mut prs = vec![
        pr(Spec {
            repo: "lumen-api",
            number: 482,
            title: "Add per-key rate limiting to the upload API",
            author: "mira-k",
            minutes_ago: 12,
            draft: false,
            review: Some("REVIEW_REQUIRED"),
            ci: Ci::Failure,
            labels: vec![
                feature(),
                label("api", "1d76db"),
                label("needs-review", "fbca04"),
            ],
        }),
        pr(Spec {
            repo: "lumen-api",
            number: 479,
            title: "Fix pagination cursor overflow on large result sets",
            author: "jonas-b",
            minutes_ago: 190,
            draft: false,
            review: Some("APPROVED"),
            ci: Ci::Success,
            labels: vec![bug()],
        }),
        pr(Spec {
            repo: "lumen-api",
            number: 471,
            title: "Streaming exports for large workspaces",
            author: "priya-n",
            minutes_ago: 60 * 26,
            draft: true,
            review: None,
            ci: Ci::Pending,
            labels: vec![label("wip", "ededed"), feature()],
        }),
        pr(Spec {
            repo: "lumen-api",
            number: 468,
            title: "Bump pydantic from 2.8.2 to 2.9.0",
            author: "dependabot[bot]",
            minutes_ago: 60 * 50,
            draft: false,
            review: None,
            ci: Ci::Success,
            labels: vec![label("dependencies", "0366d6")],
        }),
        pr(Spec {
            repo: "lumen-web",
            number: 1207,
            title: "Redesign the settings page",
            author: "liam-o",
            minutes_ago: 300,
            draft: false,
            review: Some("CHANGES_REQUESTED"),
            ci: Ci::Failure,
            labels: vec![label("design", "c5def5"), label("ui", "bfdadc")],
        }),
        pr(Spec {
            repo: "lumen-web",
            number: 1203,
            title: "Keyboard shortcuts for the command palette",
            author: "sofia-r",
            minutes_ago: 60 * 27,
            draft: false,
            review: Some("APPROVED"),
            ci: Ci::Success,
            labels: vec![feature()],
        }),
        pr(Spec {
            repo: "lumen-web",
            number: 1198,
            title: "Fix flash of unstyled content on hydration",
            author: "jonas-b",
            minutes_ago: 60 * 70,
            draft: false,
            review: Some("REVIEW_REQUIRED"),
            ci: Ci::Pending,
            labels: vec![bug()],
        }),
        pr(Spec {
            repo: "harbor",
            number: 96,
            title: "Support multi-arch image builds",
            author: "mira-k",
            minutes_ago: 420,
            draft: false,
            review: Some("APPROVED"),
            ci: Ci::Success,
            labels: vec![label("build", "5319e7")],
        }),
        pr(Spec {
            repo: "harbor",
            number: 93,
            title: "Cache layer metadata between runs",
            author: "priya-n",
            minutes_ago: 60 * 48,
            draft: false,
            review: Some("REVIEW_REQUIRED"),
            ci: Ci::Failure,
            labels: vec![label("performance", "f9d0c4")],
        }),
        pr(Spec {
            repo: "harbor",
            number: 90,
            title: "Drop Node 16 support",
            author: "liam-o",
            minutes_ago: 60 * 24 * 6,
            draft: false,
            review: None,
            ci: Ci::Success,
            labels: vec![label("breaking", "b60205"), label("chore", "cfd3d7")],
        }),
        pr(Spec {
            repo: "docs",
            number: 311,
            title: "Document the new upload limits",
            author: "sofia-r",
            minutes_ago: 640,
            draft: true,
            review: None,
            ci: Ci::Success,
            labels: vec![label("documentation", "0075ca")],
        }),
        pr(Spec {
            repo: "docs",
            number: 308,
            title: "Reorganize the getting-started guide",
            author: "mira-k",
            minutes_ago: 60 * 24 * 4,
            draft: false,
            review: Some("APPROVED"),
            ci: Ci::Success,
            labels: vec![label("documentation", "0075ca")],
        }),
        pr(Spec {
            repo: "lumen-api",
            number: 484,
            title: "Expose remaining quota in a response header",
            author: "mira-k",
            minutes_ago: 25,
            draft: false,
            review: Some("REVIEW_REQUIRED"),
            ci: Ci::Pending,
            labels: vec![feature(), label("api", "1d76db")],
        }),
        pr(Spec {
            repo: "lumen-api",
            number: 487,
            title: "Per-plan limit overrides",
            author: "mira-k",
            minutes_ago: 40,
            draft: true,
            review: None,
            ci: Ci::None,
            labels: vec![feature()],
        }),
        pr(Spec {
            repo: "lumen-web",
            number: 1209,
            title: "Settings: add a dark-mode toggle",
            author: "liam-o",
            minutes_ago: 240,
            draft: false,
            review: None,
            ci: Ci::Success,
            labels: vec![label("ui", "bfdadc")],
        }),
    ];
    stack_on(&mut prs, 484, 482);
    stack_on(&mut prs, 487, 484);
    stack_on(&mut prs, 1209, 1207);
    let issues = vec![
        issue(Spec {
            repo: "lumen-api",
            number: 475,
            title: "Uploads over 5 GB time out behind some proxies",
            author: "jonas-b",
            minutes_ago: 95,
            draft: false,
            review: None,
            ci: Ci::None,
            labels: vec![bug(), label("help wanted", "008672")],
        }),
        issue(Spec {
            repo: "lumen-api",
            number: 470,
            title: "Support ETag on list endpoints",
            author: "priya-n",
            minutes_ago: 60 * 30,
            draft: false,
            review: None,
            ci: Ci::None,
            labels: vec![label("enhancement", "a2eeef"), label("api", "1d76db")],
        }),
        issue(Spec {
            repo: "lumen-web",
            number: 1211,
            title: "Command palette steals focus from text inputs",
            author: "sofia-r",
            minutes_ago: 240,
            draft: false,
            review: None,
            ci: Ci::None,
            labels: vec![bug()],
        }),
        issue(Spec {
            repo: "lumen-web",
            number: 1190,
            title: "Dark mode: low contrast on disabled buttons",
            author: "liam-o",
            minutes_ago: 60 * 24 * 5,
            draft: false,
            review: None,
            ci: Ci::None,
            labels: vec![
                bug(),
                label("design", "c5def5"),
                label("good first issue", "7057ff"),
            ],
        }),
        issue(Spec {
            repo: "harbor",
            number: 94,
            title: "Builds fail on Apple silicon runners with Rosetta disabled",
            author: "mira-k",
            minutes_ago: 60 * 9,
            draft: false,
            review: None,
            ci: Ci::None,
            labels: vec![bug()],
        }),
        issue(Spec {
            repo: "harbor",
            number: 88,
            title: "Document registry authentication setup",
            author: "priya-n",
            minutes_ago: 60 * 24 * 8,
            draft: false,
            review: None,
            ci: Ci::None,
            labels: vec![
                label("documentation", "0075ca"),
                label("good first issue", "7057ff"),
            ],
        }),
        issue(Spec {
            repo: "docs",
            number: 305,
            title: "Search ignores text inside code blocks",
            author: "sofia-r",
            minutes_ago: 60 * 24 * 2,
            draft: false,
            review: None,
            ci: Ci::None,
            labels: vec![label("enhancement", "a2eeef")],
        }),
    ];
    Snapshot {
        prs,
        issues,
        rate_remaining: Some(4987),
    }
}

/// One hunk with correct header counts; each line starts with ' ', '+' or '-'.
fn hunk(old_start: u32, new_start: u32, context: &str, lines: &[&str]) -> String {
    let old = lines.iter().filter(|l| !l.starts_with('+')).count();
    let new = lines.iter().filter(|l| !l.starts_with('-')).count();
    let mut out = format!("@@ -{old_start},{old} +{new_start},{new} @@ {context}\n");
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    out
}

fn file_diff(path: &str, is_new: bool, hunks: &[String]) -> String {
    let mut out = format!("diff --git a/{path} b/{path}\n");
    if is_new {
        out.push_str("new file mode 100644\nindex 0000000..4f2c1a9\n--- /dev/null\n");
    } else {
        out.push_str("index 83a1c2e..b72f0d4 100644\n");
        out.push_str(&format!("--- a/{path}\n"));
    }
    out.push_str(&format!("+++ b/{path}\n"));
    for h in hunks {
        out.push_str(h);
    }
    out
}

fn hero_diff() -> String {
    let limits = file_diff(
        "lumen/limits.py",
        true,
        &[hunk(
            0,
            1,
            "",
            &[
                "+\"\"\"Token-bucket rate limiting for upload endpoints.\"\"\"",
                "+from __future__ import annotations",
                "+",
                "+import time",
                "+from dataclasses import dataclass",
                "+",
                "+from redis.asyncio import Redis",
                "+",
                "+",
                "+@dataclass(frozen=True)",
                "+class Limit:",
                "+    requests_per_minute: int = 60",
                "+    bytes_per_hour: int = 2 * 1024**3",
                "+",
                "+",
                "+class RateLimitExceeded(Exception):",
                "+    def __init__(self, retry_after: float) -> None:",
                "+        super().__init__(f\"rate limit exceeded, retry in {retry_after:.0f}s\")",
                "+        self.retry_after = retry_after",
                "+",
                "+",
                "+class RateLimiter:",
                "+    \"\"\"Per-key token bucket backed by Redis.\"\"\"",
                "+",
                "+    def __init__(self, redis: Redis, limit: Limit | None = None, jitter: float = 0.1) -> None:",
                "+        self._redis = redis",
                "+        self._limit = limit or Limit()",
                "+        self._jitter = jitter",
                "+",
                "+    async def check(self, key: str) -> None:",
                "+        now = time.time()",
                "+        bucket = f\"rl:{key}:{int(now // 60)}\"",
                "+        used = await self._redis.incr(bucket)",
                "+        if used == 1:",
                "+            await self._redis.expire(bucket, 120)",
                "+        if used > self._limit.requests_per_minute:",
                "+            raise RateLimitExceeded((60 - now % 60) * (1 + self._jitter))",
            ],
        )],
    );
    let upload = file_diff(
        "lumen/routes/upload.py",
        false,
        &[
            hunk(
                1,
                1,
                "",
                &[
                    " from fastapi import APIRouter, Depends, HTTPException, UploadFile",
                    "+from fastapi.responses import JSONResponse",
                    " ",
                    " from lumen.auth import current_key",
                    "+from lumen.limits import RateLimitExceeded, RateLimiter, get_limiter",
                    " from lumen.storage import store_blob",
                    " ",
                    " router = APIRouter(prefix=\"/v1\")",
                ],
            ),
            hunk(
                24,
                26,
                "router = APIRouter(prefix=\"/v1\")",
                &[
                    " @router.post(\"/uploads\")",
                    "-async def create_upload(file: UploadFile, key: str = Depends(current_key)):",
                    "-    blob = await store_blob(file)",
                    "-    return {\"id\": blob.id, \"size\": blob.size}",
                    "+async def create_upload(",
                    "+    file: UploadFile,",
                    "+    key: str = Depends(current_key),",
                    "+    limiter: RateLimiter = Depends(get_limiter),",
                    "+):",
                    "+    try:",
                    "+        await limiter.check(key)",
                    "+    except RateLimitExceeded as exc:",
                    "+        return JSONResponse(",
                    "+            {\"error\": \"rate_limited\"},",
                    "+            status_code=429,",
                    "+            headers={\"Retry-After\": str(int(exc.retry_after))},",
                    "+        )",
                    "+    blob = await store_blob(file)",
                    "+    return {\"id\": blob.id, \"size\": blob.size}",
                ],
            ),
        ],
    );
    let tests = file_diff(
        "tests/test_limits.py",
        true,
        &[hunk(
            0,
            1,
            "",
            &[
                "+import pytest",
                "+",
                "+from lumen.limits import Limit, RateLimitExceeded, RateLimiter",
                "+",
                "+",
                "+@pytest.mark.asyncio",
                "+async def test_blocks_after_the_limit(fake_redis):",
                "+    limiter = RateLimiter(fake_redis, Limit(requests_per_minute=2))",
                "+    await limiter.check(\"key-1\")",
                "+    await limiter.check(\"key-1\")",
                "+    with pytest.raises(RateLimitExceeded):",
                "+        await limiter.check(\"key-1\")",
                "+",
                "+",
                "+@pytest.mark.asyncio",
                "+async def test_keys_are_independent(fake_redis):",
                "+    limiter = RateLimiter(fake_redis, Limit(requests_per_minute=1))",
                "+    await limiter.check(\"key-1\")",
                "+    await limiter.check(\"key-2\")",
            ],
        )],
    );
    let docs = file_diff(
        "docs/upload-limits.md",
        true,
        &[hunk(
            0,
            1,
            "",
            &[
                "+# Upload limits",
                "+",
                "+Each API key may make **60 upload requests per minute**.",
                "+Requests over the limit receive `429 Too Many Requests` and a",
                "+`Retry-After` header, in seconds.",
            ],
        )],
    );
    [limits, upload, tests, docs].concat()
}

fn generic_diff(title: &str) -> String {
    file_diff(
        "CHANGELOG.md",
        false,
        &[hunk(
            1,
            1,
            "",
            &[
                " # Changelog",
                " ",
                &format!("+- {title}"),
                " ## Unreleased",
            ],
        )],
    )
}

const ISSUE_BODY: &str = "\
Uploads larger than about 5 GB fail with a `504` when the client sits behind an HTTP proxy with a 60 second idle timeout.

### Steps to reproduce

1. Upload a 6 GB file with `lumen upload big.bin`
2. Wait for the proxy's idle timeout
3. Observe the `504 Gateway Timeout`

### Expected

The upload completes, or fails with a clear, actionable error.

```
$ lumen upload big.bin
error: 504 Gateway Timeout (after 61s)
```";

const HERO_BODY: &str = "\
## What

Adds a per-API-key token bucket in front of `POST /v1/uploads`, so one noisy client can no longer starve everyone else.

## How

- New `RateLimiter` in `lumen/limits.py` (token bucket, Redis-backed)
- Defaults to **60 requests/min** per key
- Exceeding the limit returns `429` with a `Retry-After` header
- Limits are configurable per plan via `LUMEN_UPLOAD_LIMITS`

## Testing

- [x] Unit tests for bucket refill and burst handling
- [x] Load test at 5k req/s against a local Redis
- [ ] Verify `Retry-After` against the Python and JS SDKs

> Follow-up: expose remaining quota in a response header (see #470).

See the [design doc](https://example.com/design/upload-limits) for the full rationale.";

fn checks(ci: Ci, hero: bool) -> Vec<Check> {
    let names: &[&str] = if hero {
        &[
            "lint",
            "type-check",
            "unit-tests",
            "integration-tests",
            "build-image",
            "security-scan",
            "migrations-check",
            "openapi-diff",
            "docs-build",
            "license-check",
            "deploy-preview",
        ]
    } else {
        &["lint", "type-check", "unit-tests", "build", "security-scan"]
    };
    names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let state = match (ci, *name) {
                (_, "integration-tests") if hero => Ci::Failure,
                (_, "deploy-preview") if hero => Ci::Pending,
                (Ci::Failure, _) if n == 2 => Ci::Failure,
                (Ci::Pending, _) if n == names.len() - 1 => Ci::Pending,
                _ => Ci::Success,
            };
            Check {
                name: format!("ci/circleci: {name}"),
                ci: state,
                url: Some(format!("https://ci.example.com/jobs/{name}")),
            }
        })
        .collect()
}

pub fn details(snap: &Snapshot) -> HashMap<String, Detail> {
    let mut out = HashMap::new();
    for i in snap.prs.iter().chain(snap.issues.iter()) {
        let hero = i.number == 482;
        let is_pr = i.kind == Kind::Pr;
        let d = Detail {
            state: "OPEN".into(),
            created_at: (i.updated_at - Duration::hours(if hero { 30 } else { 70 })).to_rfc3339(),
            body: if hero {
                HERO_BODY.into()
            } else if i.number == 475 {
                ISSUE_BODY.into()
            } else if is_pr {
                format!("{}.\n\n- [x] Tests added\n- [ ] Docs updated", i.title)
            } else {
                format!("{}\n\nSteps to reproduce are in the thread below.", i.title)
            },
            head: i.head.clone(),
            base: i.base.clone(),
            mergeable: "MERGEABLE".into(),
            merge_state: if i.ci == Ci::Failure {
                "UNSTABLE"
            } else {
                "CLEAN"
            }
            .into(),
            additions: 0,
            deletions: 0,
            files: 0,
            labels: i.labels.clone(),
            assignees: if is_pr {
                vec![]
            } else {
                vec![i.author.clone()]
            },
            requested: if hero { vec!["sofia-r".into()] } else { vec![] },
            reviews: if hero {
                vec![
                    ("jonas-b".into(), "APPROVED".into()),
                    ("priya-n".into(), "COMMENTED".into()),
                ]
            } else {
                vec![]
            },
            checks: if is_pr { checks(i.ci, hero) } else { vec![] },
            comments: if i.number == 475 {
                vec![Comment {
                    author: "mira-k".into(),
                    body: "Reproduced with nginx `proxy_read_timeout 60s`. Chunked uploads with a keep-alive ping should avoid it.".into(),
                    created_at: (Utc::now() - Duration::hours(3)).to_rfc3339(),
                }]
            } else if hero {
                vec![
                    Comment {
                        author: "priya-n".into(),
                        body: "Looks good overall. Could we jitter `Retry-After` so clients don't all retry in the same instant?".into(),
                        created_at: (Utc::now() - Duration::hours(5)).to_rfc3339(),
                    },
                    Comment {
                        author: "mira-k".into(),
                        body: "Good call, added `jitter=0.1` in the latest push.".into(),
                        created_at: (Utc::now() - Duration::minutes(40)).to_rfc3339(),
                    },
                ]
            } else {
                vec![]
            },
            fetched: Instant::now(),
        };
        out.insert(i.id.clone(), d);
    }
    out
}

fn ago(secs: u64) -> Instant {
    let now = Instant::now();
    now.checked_sub(StdDuration::from_secs(secs)).unwrap_or(now)
}

/// A finished Claude repair on one PR and a running one on another, keyed by item id.
pub fn runs(snap: &Snapshot) -> HashMap<String, ClaudeRun> {
    let id = |n: u64| {
        snap.prs
            .iter()
            .find(|i| i.number == n)
            .map(|i| i.id.clone())
            .unwrap_or_default()
    };
    let prompt = "Rebase this PR on its base branch and fix any failing checks.";
    let mut out = HashMap::new();
    out.insert(
        id(1207),
        ClaudeRun {
            repo: "lumen-web".into(),
            number: 1207,
            prompt: prompt.into(),
            started: ago(260),
            finished: Some(ago(70)),
            state: RunState::Done(0),
            log: "~/.local/state/pane/claude/lumen-web-pr1207-1794245349.log".into(),
            kept_worktree: None,
            summary: Some(
                "Rebased `liam/settings-redesign` onto `main` (two conflicts in `SettingsPage.tsx`; kept both sets of changes) and fixed the failing **unit-tests** check:\n\n- `SettingsForm` called `useRouter` outside a provider, so I wrapped it in the test harness\n- updated 3 snapshots for the new layout\n\nAll 5 checks pass locally. Pushed with `--force-with-lease`.".into(),
            ),
        },
    );
    out.insert(
        id(471),
        ClaudeRun {
            repo: "lumen-api".into(),
            number: 471,
            prompt: prompt.into(),
            started: ago(75),
            finished: None,
            state: RunState::Running,
            log: "~/.local/state/pane/claude/lumen-api-pr471-1794245401.log".into(),
            kept_worktree: None,
            summary: None,
        },
    );
    out
}

/// Diffs keyed by item id; also fills in each detail's size counts from its diff.
pub fn diffs(snap: &Snapshot, details: &mut HashMap<String, Detail>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for i in snap.prs.iter() {
        let text = if i.number == 482 {
            hero_diff()
        } else {
            generic_diff(&i.title)
        };
        let parsed = diff::parse(&text);
        if let Some(d) = details.get_mut(&i.id) {
            d.additions = parsed.files.iter().map(|f| f.adds as u64).sum();
            d.deletions = parsed.files.iter().map(|f| f.dels as u64).sum();
            d.files = parsed.files.len() as u64;
        }
        out.insert(i.id.clone(), text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_has_stacked_prs() {
        let snap = snapshot();
        let by_number = |n: u64| snap.prs.iter().find(|p| p.number == n).unwrap();
        assert_eq!(by_number(484).base, by_number(482).head);
        assert_eq!(by_number(487).base, by_number(484).head);
        assert_eq!(by_number(1209).base, by_number(1207).head);
        assert_eq!(by_number(482).base, "main");
    }

    #[test]
    fn demo_runs_cover_one_finished_and_one_running() {
        let snap = snapshot();
        let runs = runs(&snap);
        assert_eq!(runs.len(), 2);
        let states: Vec<bool> = runs
            .values()
            .map(|r| matches!(r.state, RunState::Running))
            .collect();
        assert!(states.contains(&true) && states.contains(&false));
        assert!(runs.keys().all(|k| !k.is_empty()));
    }

    #[test]
    fn hunk_headers_count_lines() {
        let h = hunk(5, 5, "ctx", &[" a", "-b", "+c", "+d"]);
        assert!(h.starts_with("@@ -5,2 +5,3 @@ ctx\n"));
    }

    #[test]
    fn hero_diff_parses_cleanly() {
        let d = diff::parse(&hero_diff());
        assert_eq!(d.files.len(), 4);
        for f in &d.files {
            let parsed: usize = f.hunks.iter().map(|h| h.lines.len()).sum();
            assert!(parsed > 0, "{} has no parsed lines", f.new);
        }
        assert_eq!(d.files[1].hunks.len(), 2);
        assert_eq!(d.files[1].dels, 3);
    }

    #[test]
    fn every_item_has_detail_and_prs_have_diffs() {
        let snap = snapshot();
        let mut det = details(&snap);
        let dif = diffs(&snap, &mut det);
        for i in snap.prs.iter().chain(snap.issues.iter()) {
            assert!(det.contains_key(&i.id), "{}", i.title);
        }
        for i in &snap.prs {
            assert!(dif.contains_key(&i.id));
        }
        let hero = det
            .values()
            .find(|d| d.head == "mira/upload-rate-limit")
            .unwrap();
        assert_eq!(hero.files, 4);
        assert!(hero.additions > 50);
    }

    #[test]
    fn demo_data_uses_only_fictional_names() {
        let snap = snapshot();
        for i in snap.prs.iter().chain(snap.issues.iter()) {
            assert!(i.url.contains("/acme/"));
        }
    }
}
