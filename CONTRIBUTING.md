# Contributing

Thanks for helping. Bug reports and pull requests are welcome.

## Setup

You need Rust 1.89 or newer (stable is fine) and `git`. Running against real data also needs the GitHub CLI (`gh auth login`). The tests do not touch the network; they use `git` and a few Unix commands, so run them on macOS or Linux.

```sh
cargo run -- --demo   # the UI on fictional data, no network
cargo run             # against your real GitHub account
cargo run -- --check  # list what would be shown, no UI
```

## Before opening a PR

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs these on Linux and macOS, builds the docs with warnings denied, and runs the tests on the minimum supported Rust version (the `rust-version` in `Cargo.toml`). A weekly job audits dependencies.

## Project layout

| File | Responsibility |
| --- | --- |
| `src/main.rs` | startup, terminal setup, background tasks, the event loop |
| `src/cli.rs` | command-line flags |
| `src/config.rs` | the optional config file and its validation |
| `src/gh.rs` | everything that talks to GitHub: GraphQL queries, response parsing, write actions |
| `src/app.rs` | application state and input handling (keys, mouse, modes) |
| `src/ui.rs` | the list, header, footer and overlays |
| `src/detail.rs` | the PR and issue detail view |
| `src/markdown.rs` | Markdown to styled terminal lines |
| `src/diff.rs` | unified diff parsing and rendering |
| `src/claude.rs` | the repair action: finding the clone, running `claude -p`, worktree cleanup |
| `src/demo.rs` | fictional data for `--demo` and the screenshots |

State lives in `App`; `App::update` is the only place that changes it, and rendering reads it. Background work (refreshes, detail fetches, actions, Claude runs) reports back through messages.

## Guidelines

- Keep changes focused; one concern per PR.
- Add a test with each behavior change. Rendering is tested with ratatui's `TestBackend`, input with synthetic key events, GitHub responses with JSON fixtures, and git behavior with throwaway repositories.
- Failures should be visible to the user, not silently swallowed or papered over with a fallback.
- Every command is a single lowercase letter, and upper case behaves the same. Don't add commands that differ only by case.
- Never run write actions (merge, close, approve, repair) against real repos while testing. Use a throwaway repo.
- Comments explain a constraint the code relies on, not what changed.
- User-facing changes get a line in `CHANGELOG.md` under Unreleased, and the README stays in sync with the keys and config.

## Screenshots

The README screenshots come from `pane --demo`. After a visual change, run `scripts/screenshots.sh` (needs `tmux`, `python3` and Google Chrome) and commit the updated images.

## Releasing

Move the Unreleased entries in `CHANGELOG.md` under a `## [x.y.z] - YYYY-MM-DD` heading, bump `version` in `Cargo.toml`, merge, then push a `vx.y.z` tag. The release workflow builds binaries for macOS and Linux and publishes the changelog text as the release notes.

## Reporting security issues

See [SECURITY.md](SECURITY.md).
