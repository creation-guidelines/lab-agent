use crate::contract::{ENV_JOBS_REPO, ENV_OWNER, ENV_RESULTS_REPO, ENV_TOKEN};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// Everything lab-agent needs to run, for one project. Project-agnostic: the
/// same binary and config shape work for any hardware-in-the-loop setup, not
/// just one device. Lives at ~/.config/lab-agent/config.toml.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    /// GitHub user or org that owns the jobs/results repos.
    pub owner: String,
    /// Repo holding job scripts to pull, e.g. "lab-agent-jobs".
    pub jobs_repo: String,
    /// Repo results get pushed to, e.g. "lab-agent-results".
    pub results_repo: String,
    /// Branch jobs are read from.
    #[serde(default = "default_jobs_branch")]
    pub jobs_branch: String,
    /// Branch results are pushed to.
    #[serde(default = "default_results_branch")]
    pub results_branch: String,
    /// How many jobs may run at once. 1 is the safe default for a single
    /// piece of hardware; raise it for independent/parallel-safe work.
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    /// How often to poll for new jobs, in seconds.
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
}

fn default_jobs_branch() -> String {
    "main".to_string()
}
fn default_results_branch() -> String {
    "results".to_string()
}
fn default_max_parallel() -> usize {
    1
}
fn default_poll_seconds() -> u64 {
    5
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("HOME not set");
            PathBuf::from(home).join(".config")
        });
    base.join("lab-agent")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn token_path() -> PathBuf {
    config_dir().join("token")
}

pub fn load() -> Result<Config> {
    // A client with no config file (an agent, a CI job) names the repos in its environment: the
    // one way to configure it without a file and without putting anything on a command line.
    if let Ok(owner) = std::env::var(ENV_OWNER) {
        let var =
            |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
        return Ok(Config {
            owner,
            jobs_repo: var(ENV_JOBS_REPO, "lab-agent-jobs"),
            results_repo: var(ENV_RESULTS_REPO, "lab-agent-results"),
            jobs_branch: default_jobs_branch(),
            results_branch: default_results_branch(),
            max_parallel: default_max_parallel(),
            poll_seconds: default_poll_seconds(),
        });
    }
    let path = config_path();
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "no config at {} - run `lab-agent connect` first",
            path.display()
        )
    })?;
    Ok(toml::from_str(&text)?)
}

pub fn save(cfg: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let text = toml::to_string_pretty(cfg)?;
    fs::write(config_path(), text)?;
    Ok(())
}

/// Writes the token to its own file, mode 0600, never into config.toml and
/// never accepted as a CLI argument (so it can't leak into shell history or
/// process listings).
pub fn save_token(token: &str) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let path = token_path();
    fs::write(&path, token)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn load_token() -> Result<String> {
    if let Ok(token) = std::env::var(ENV_TOKEN) {
        let token = token.trim();
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }
    let path = token_path();
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "no token at {} - run `lab-agent connect` first",
            path.display()
        )
    })?;
    Ok(text.trim().to_string())
}
