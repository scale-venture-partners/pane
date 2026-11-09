# Contributing

Thanks for helping. Bug reports and pull requests are welcome.

## Setup

You need stable Rust and the GitHub CLI (`gh auth login`) to run the app. Tests do not touch the network.

```sh
cargo run            # run against your real GitHub account
cargo run -- --check # list what would be shown, no UI
```

## Before opening a PR

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same three commands on Linux and macOS.

## Guidelines

- Keep changes focused; one concern per PR.
- Add a test with each behavior change. Rendering is tested with ratatui's `TestBackend`, input with synthetic key events, and GitHub responses with JSON fixtures.
- Failures should be visible to the user, not silently swallowed or papered over with a fallback.
- Never run write actions (merge, close, approve) against real repos while testing. Use a throwaway repo.
- Comments explain a constraint the code relies on, not what changed.

## Reporting security issues

See [SECURITY.md](SECURITY.md).
