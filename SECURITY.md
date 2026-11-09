# Security policy

pane runs the `gh` CLI with your existing authentication and does not store or transmit tokens itself. It makes no network requests other than through `gh`, and has no telemetry.

## Supported versions

Only the latest release receives security fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's "Report a vulnerability" button on the Security tab of this repository, not in a public issue. Expect an initial reply within a week.

## Scope notes

- The repair action (`r`) runs `claude -p` with `--permission-mode auto` by default, on pull requests whose text and code may be written by someone else. CodeRabbit comments, which the `t` action feeds to Claude, count as that content too. Prompt injection through that content is a known risk of the feature; see "Security and privacy" in the README. Reports of ways to make pane itself pass untrusted content as instructions, or escape its worktree isolation, are in scope.
- Merges into a non-default branch are refused unless an open PR has that branch as its head (checked live), to avoid orphaning commits; `allow_merge_into` lists exceptions.
- Links from GitHub data are opened only when they are `http://` or `https://`.
