//! Command-line arguments.

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    /// Start the dashboard; `demo` uses fictional data and never touches the network.
    Run {
        demo: bool,
    },
    /// Print the URL of every item that would be shown, then exit.
    Check,
    Help,
    Version,
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut cli = Cli::Run { demo: false };
    let mut seen_mode = false;
    for arg in args {
        let next = match arg.as_str() {
            "-h" | "--help" => Cli::Help,
            "-V" | "--version" => Cli::Version,
            "--check" => Cli::Check,
            "--demo" => Cli::Run { demo: true },
            other => return Err(format!("unknown argument `{other}`")),
        };
        if seen_mode && !matches!(next, Cli::Help | Cli::Version) {
            return Err("only one of --demo and --check may be given".into());
        }
        if matches!(next, Cli::Help | Cli::Version) {
            return Ok(next);
        }
        seen_mode = true;
        cli = next;
    }
    Ok(cli)
}

pub fn usage(config_path: &str) -> String {
    format!(
        "pane: live terminal dashboard for open GitHub PRs and issues\n\n\
         usage: pane [--demo | --check]\n\n\
         \x20 --demo       run on fictional data; no network or GitHub account needed\n\
         \x20 --check      print the URL of every open PR and issue pane would show, then exit\n\
         \x20 -V, --version\n\
         \x20 -h, --help\n\n\
         config: {config_path}\n\
         keys:   press ? inside the app"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Cli, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_args_runs_the_dashboard() {
        assert_eq!(p(&[]), Ok(Cli::Run { demo: false }));
    }

    #[test]
    fn flags_select_modes() {
        assert_eq!(p(&["--demo"]), Ok(Cli::Run { demo: true }));
        assert_eq!(p(&["--check"]), Ok(Cli::Check));
        assert_eq!(p(&["--version"]), Ok(Cli::Version));
        assert_eq!(p(&["-V"]), Ok(Cli::Version));
        assert_eq!(p(&["-h"]), Ok(Cli::Help));
    }

    #[test]
    fn help_and_version_win_over_other_flags() {
        assert_eq!(p(&["--demo", "--help"]), Ok(Cli::Help));
        assert_eq!(p(&["--check", "-V"]), Ok(Cli::Version));
    }

    #[test]
    fn unknown_and_conflicting_flags_are_errors() {
        assert!(p(&["--bogus"]).unwrap_err().contains("--bogus"));
        assert!(p(&["--demo", "--check"]).is_err());
        assert!(p(&["positional"]).is_err());
    }

    #[test]
    fn usage_mentions_every_flag() {
        let u = usage("/x/config.toml");
        for flag in ["--demo", "--check", "--version", "--help", "/x/config.toml"] {
            assert!(u.contains(flag), "{flag}");
        }
    }
}
