# pane

[![CI](https://github.com/scale-venture-partners/pane/actions/workflows/ci.yml/badge.svg)](https://github.com/scale-venture-partners/pane/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A live terminal dashboard for your open GitHub pull requests and issues. Leave it running in a terminal pane all day; it polls GitHub at a polite rate and keeps the list current.

- PRs and issues in two tabs, grouped by repo, most recently active repo first.
- PR details are laid out as a card: state, review, CI and merge badges, branch chips, a `+/−` size bar, a check progress bar, rendered Markdown descriptions and comments (headings, lists, tasks, code blocks, links), and a diff with a file summary, line-number gutters and tinted added/removed rows.
- Per PR at a glance: CI state `●`, review state `✓ ✗ ○`, draft `◇`.
- Labels show as chips in GitHub's own colors, in rows (when there is room) and in details; `/` filtering matches label names.
- `◆` marks items that changed since you last looked at them.
- Click or press Enter for details: description, branch, diff size, mergeability, reviews, every check (click one to open it), recent comments, and the full diff at the bottom.
- Merge or close an item and it stays on screen marked `MERGED` or `CLOSED` until you move off it, then disappears.
- Basic actions without leaving the terminal: open in browser, copy URL, comment, approve, merge, close, toggle draft. Anything that writes asks `y/N` first.
- Filter by text, show only your own items, full mouse support.

## Install

Requires the [GitHub CLI](https://cli.github.com) (`gh`), signed in with `gh auth login`. pane uses it for authentication and never handles tokens itself.

```sh
cargo install --git https://github.com/scale-venture-partners/pane
pane
```

Or from a clone: `cargo install --path .`

Works on macOS and Linux. Label colors use truecolor when `COLORTERM` says so and fall back to the nearest 256-color shade otherwise. Opening links uses `open` or `xdg-open`; copying uses `pbcopy`, `wl-copy`, `xclip` or `xsel`, whichever is found.

## Usage

With no config, pane shows open PRs and issues that involve you, plus PRs awaiting your review. Press `?` for all keys.

| Key | Action |
| --- | --- |
| `↑` `↓` `j` `k` | move |
| `Enter` `l` / `Esc` `h` | open / close details |
| `Tab` `1` `2` | switch PRs and issues |
| `/` | filter by text |
| `m` | only my items |
| `r` | refresh now |
| `o` `y` | open in browser, copy URL |
| `f` | open the first failing check |
| `d` | jump to the diff (PR details) |
| `]` `[` | next / previous file in the diff |
| `a` `M` `x` `D` | approve, merge, close, toggle draft (each confirms) |
| `c` | comment |
| `q` | quit |

## Configuration

Optional file at `~/.config/pane/config.toml` (or `$XDG_CONFIG_HOME/pane/config.toml`). See [`config.example.toml`](config.example.toml).

| Key | Default | Meaning |
| --- | --- | --- |
| `refresh_secs` | `90` | poll interval; minimum 15 |
| `merge_method` | `"squash"` | `squash`, `merge` or `rebase` |
| `orgs` | `[]` | show every open item in these orgs |
| `repos` | `[]` | extra `owner/name` repos to show |

When both `orgs` and `repos` are empty, pane shows items that involve you.

`pane --check` prints the URL of every item pane would show, then exits. Handy for verifying a config.

## Rate limits

Each refresh issues a few GraphQL search queries (one per org, one per ~4 repos, twice for PRs and issues). At the default interval that is well under GitHub's hourly allowance; the header shows how many calls remain.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
