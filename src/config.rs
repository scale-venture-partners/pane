use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub refresh_secs: u64,
    pub merge_method: String,
    /// Every open item in these orgs is shown.
    pub orgs: Vec<String>,
    /// Individual `owner/name` repos shown in addition to `orgs`.
    /// With both lists empty, pane shows everything that involves you.
    pub repos: Vec<String>,
    /// Prompt sent to `claude -p` by the Claude action; editable before it runs.
    pub claude_prompt: String,
    /// Extra arguments for `claude`. `claude -p` has nobody to approve prompts, so the default
    /// is auto mode, which approves routine actions on its own.
    pub claude_args: Vec<String>,
    /// Run Claude in its own git worktree so your checkout is left alone.
    pub claude_worktree: bool,
    /// Directories searched for local checkouts of the repos you act on.
    pub repo_roots: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_secs: 90,
            merge_method: "squash".into(),
            orgs: vec![],
            repos: vec![],
            claude_prompt: "Rebase this PR on its base branch and fix any failing checks.".into(),
            claude_args: vec!["--permission-mode".into(), "auto".into()],
            claude_worktree: true,
            repo_roots: [
                "~/code",
                "~/src",
                "~/dev",
                "~/projects",
                "~/GitHub",
                "~/Documents/GitHub",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("pane").join("config.toml")
}

pub fn load() -> Result<Config> {
    let p = path();
    let cfg = match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", p.display())),
    };
    cfg.validate()?;
    Ok(cfg)
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.refresh_secs < 15 {
            bail!("refresh_secs must be at least 15 to stay within GitHub rate limits");
        }
        if !["squash", "merge", "rebase"].contains(&self.merge_method.as_str()) {
            bail!("merge_method must be squash, merge or rebase");
        }
        if self.claude_prompt.trim().is_empty() {
            bail!("claude_prompt must not be empty");
        }
        if let Some(bad) = self.repos.iter().find(|r| !r.contains('/')) {
            bail!("repos entries must be owner/name, got {bad:?}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn rejects_fast_refresh() {
        let c = Config {
            refresh_secs: 5,
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Config>("bogus = 1").is_err());
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let c: Config = toml::from_str("refresh_secs = 30").unwrap();
        assert_eq!(c.refresh_secs, 30);
        assert_eq!(c.merge_method, "squash");
    }

    #[test]
    fn claude_defaults() {
        let c = Config::default();
        assert!(c.claude_prompt.contains("Rebase"));
        assert!(c.claude_worktree);
        assert_eq!(c.claude_args, ["--permission-mode", "auto"]);
        assert!(!c.repo_roots.is_empty());
    }

    #[test]
    fn rejects_empty_claude_prompt() {
        let c = Config {
            claude_prompt: "  ".into(),
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_bad_merge_method() {
        let c = Config {
            merge_method: "yolo".into(),
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_repo_without_owner() {
        let c = Config {
            repos: vec!["just-a-name".into()],
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn parses_full_file() {
        let c: Config =
            toml::from_str("orgs = [\"acme\"]\nrepos = [\"acme/tool\"]\nmerge_method = \"rebase\"")
                .unwrap();
        c.validate().unwrap();
        assert_eq!(c.orgs, ["acme"]);
        assert_eq!(c.repos, ["acme/tool"]);
    }
}
