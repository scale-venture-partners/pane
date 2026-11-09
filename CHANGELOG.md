# Changelog

All notable changes are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Live PR and issue dashboard with detail pane, filter, and mouse support.
- Actions: open, copy URL, comment, approve, merge, close, toggle draft.
- Clickable checks and a scrollable diff at the bottom of PR details.
- Items that merge or close stay visible, marked `MERGED` or `CLOSED`, until you navigate away.
- First-load screen with spinner, sign-in check, query progress, and a clear message when GitHub can't be reached.
- Label chips in GitHub colors on rows and in details; text filter matches labels.
- Redesigned PR details: badge row, branch chips, size and check progress bars, Markdown rendering, and a diff view with file summary, line numbers, tinted rows and `]`/`[` file jumping.
- `--demo` mode with fictional data, and README screenshots generated from it.
- Claude action: press `C` on a PR to run `claude -p` in the background in the local clone, in an isolated worktree, with an editable default prompt, a run list, and logs.
- Configurable orgs, repos, refresh interval and merge method.
