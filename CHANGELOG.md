# Changelog

All notable changes are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

First public version.

### Added
- Live dashboard of open PRs and issues, grouped by repo, refreshed in the background, with a first-load screen that shows sign-in and query progress.
- Stacked PRs are nested under the PR they target, with tree connectors.
- Per-item signals: CI state, review state, draft, label chips in GitHub's colors, and a marker for items that changed since you last looked.
- PR details: badges, branch, size and check progress bars, reviews, clickable checks, Markdown descriptions and comments, and a diff with a file summary, line numbers, tinted rows and `]`/`[` file jumping.
- Issue details with Markdown rendering and comments.
- Actions: open in browser, copy URL, comment, approve, merge, close, toggle draft. Writes ask for confirmation.
- Merged or closed items stay on screen, marked, until you navigate away.
- Repair action (`r`): runs `claude -p` in the background in your local clone, in an isolated git worktree, with an editable default prompt. The run's progress, reply, log path and any leftover worktree show in the PR's details. At most one run per PR.
- CodeRabbit: an orange `CR n` chip for PRs with unresolved CodeRabbit threads, a CodeRabbit section in PR details with clickable threads, and `t` to have Claude work through them (shares the one-run-per-PR slot with repair).
- Text filter (matches labels too), an "only mine" filter, mouse support.
- `--demo` mode with fictional data, and `--check` to list what would be shown.
- Optional config file for orgs, repos, refresh interval, merge method and the repair action.
