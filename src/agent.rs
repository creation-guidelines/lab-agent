//! The agent-facing verbs: `submit`, `wait`, `logs`. They are lab-agent's API for another program
//! (an AI agent, a CI job, a script): push a script, wait for it, read its output - all through
//! the same GitHub repos the daemon already watches, so nothing listens on a port and nothing
//! needs the lab machine to be reachable.
//!
//! The contract (JSON shapes and the exit statuses below) is documented in AGENTS.md; agents rely
//! on it, so change it deliberately.

use crate::config::{self, Config};
use crate::contract::Contract;
use crate::daemon;
use crate::github::{Client, Fetched};
use crate::job::Mode;
use crate::jobs::{self, JobRow};
use anyhow::{anyhow, ensure, Context, Result};
use argenv_resolve::*;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Exit statuses of `wait` and `submit --wait`. 0 is a job that succeeded; a failed job exits with
/// its own non-zero status (1 if it has none).
pub mod exit {
    /// `--within` elapsed; the job may still finish. Same as `EX_TEMPFAIL`.
    pub const STILL_RUNNING: i32 = 75;
    /// The job hit its own time limit. Same as `timeout(1)`.
    pub const TIMED_OUT: i32 = 124;
    /// The job was cancelled (preempted by another job) or cut off by an agent restart.
    pub const CANCELLED: i32 = 125;
    /// GitHub's hourly request budget is used up (it is per user, shared by every token). Nothing
    /// is lost: a submitted job still runs, and `wait` can be run again once the budget returns.
    /// Same as `EX_UNAVAILABLE`.
    pub const RATE_LIMITED: i32 = 69;
}

pub fn exit_code(row: &JobRow) -> i32 {
    match row.status.as_str() {
        "ok" => 0,
        "timed_out" => exit::TIMED_OUT,
        "preempted" | "interrupted" => exit::CANCELLED,
        _ => match row.exit_code {
            Some(code) if code != 0 => code,
            _ => 1,
        },
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A short, safe word for a job id: lowercase letters and digits, single dashes, at most 40.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    let out = out.trim_matches('-');
    if out.is_empty() {
        "job".to_string()
    } else {
        out.to_string()
    }
}

fn mode_word(mode: Mode) -> &'static str {
    match mode {
        Mode::Queue => "queue",
        Mode::Preempt => "preempt",
    }
}

/// The script with exactly one `# lab:` header, after the shebang when there is one.
fn with_header(script: &str, mode: Mode, timeout: Duration) -> String {
    let header = format!(
        "# lab: mode={} timeout={}",
        mode_word(mode),
        timeout.as_secs()
    );
    let mut lines: Vec<&str> = script
        .lines()
        .filter(|l| !l.trim_start().starts_with("# lab:"))
        .collect();
    let at = usize::from(lines.first().is_some_and(|l| l.starts_with("#!")));
    lines.insert(at, &header);
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn read_script(file: Option<&str>) -> Result<String> {
    match file {
        Some(f) if f != "-" => std::fs::read_to_string(f).with_context(|| format!("reading {f}")),
        _ => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            Ok(text)
        }
    }
}

fn client() -> Result<(Config, Client)> {
    Ok((config::load()?, Client::new(config::load_token()?)))
}

/// `lab-agent submit [FILE]`: push a script as a job. Prints the job id (or JSON), and with
/// `--wait` waits for it and returns its status.
pub fn submit(resolved: &Resolution) -> Result<i32> {
    let (cfg, client) = client()?;
    let file = Contract::SUBMIT_FILE.get_from_or_default(resolved);
    let script = read_script(file.as_deref())?;
    ensure!(!script.trim().is_empty(), "the script is empty");

    let (header_mode, header_timeout) = daemon::parse_header(&script);
    let mode = match Contract::SUBMIT_MODE
        .get_from_or_default(resolved)
        .as_deref()
    {
        Some("preempt") => Mode::Preempt,
        Some(_) => Mode::Queue,
        None => header_mode,
    };
    let timeout = Contract::SUBMIT_TIMEOUT
        .get_from_or_default(resolved)
        .map(Duration::from_secs)
        .unwrap_or(header_timeout);
    let name_source = Contract::SUBMIT_NAME
        .get_from_or_default(resolved)
        .or_else(|| {
            file.as_deref()
                .filter(|f| *f != "-")
                .and_then(|f| Path::new(f).file_stem())
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let name = format!("{}-{}", unix_secs(), slug(&name_source));

    client.put_file(
        &cfg.owner,
        &cfg.jobs_repo,
        &format!("jobs/{name}.sh"),
        &cfg.jobs_branch,
        with_header(&script, mode, timeout).as_bytes(),
        &format!("job: {name}"),
        None,
    )?;

    let json = Contract::JSON
        .get_from_or_default(resolved)
        .unwrap_or(false);
    if !Contract::SUBMIT_WAIT
        .get_from_or_default(resolved)
        .unwrap_or(false)
    {
        if json {
            println!(
                "{}",
                serde_json::json!({ "name": name, "file": format!("jobs/{name}.sh") })
            );
        } else {
            println!("{name}");
        }
        return Ok(0);
    }
    let within = Contract::WITHIN
        .get_from_or_default(resolved)
        .map(Duration::from_secs)
        .unwrap_or(timeout + Duration::from_secs(180));
    wait_for(&client, &cfg, &name, within, json)
}

/// `lab-agent wait JOB`: wait for a job, printing its output as it arrives.
pub fn wait(resolved: &Resolution) -> Result<i32> {
    let (cfg, client) = client()?;
    let job = Contract::JOB
        .get_from_or_default(resolved)
        .ok_or_else(|| anyhow!("usage: lab-agent wait JOB"))?;
    let within = Contract::WITHIN
        .get_from_or_default(resolved)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600));
    let json = Contract::JSON
        .get_from_or_default(resolved)
        .unwrap_or(false);
    wait_for(&client, &cfg, &job, within, json)
}

/// `lab-agent logs JOB`: the job's output so far.
pub fn logs(resolved: &Resolution) -> Result<i32> {
    let (cfg, client) = client()?;
    let job = Contract::JOB
        .get_from_or_default(resolved)
        .ok_or_else(|| anyhow!("usage: lab-agent logs JOB"))?;
    let job = job.trim_end_matches(".sh");
    let path = format!("jobs/{job}/log/0000.txt");
    match client.get_file(&cfg.owner, &cfg.results_repo, &path, &cfg.results_branch)? {
        Some(file) => {
            print!("{}", String::from_utf8_lossy(&file.content));
            Ok(0)
        }
        None => {
            eprintln!("lab-agent: no output for {job} yet");
            Ok(1)
        }
    }
}

/// Polls the job's result until it finishes or `within` elapses. Reads are conditional (ETag), so
/// polling every second costs almost nothing against the rate limit.
fn wait_for(
    client: &Client,
    cfg: &Config,
    name: &str,
    within: Duration,
    json: bool,
) -> Result<i32> {
    let name = name.trim_end_matches(".sh");
    let status_path = format!("jobs/{name}/status.json");
    let log_path = format!("jobs/{name}/log/0000.txt");
    let (mut status_tag, mut log_tag): (Option<String>, Option<String>) = (None, None);
    let mut row: Option<JobRow> = None;
    let (mut log, mut shown) = (String::new(), 0usize);
    let start = Instant::now();

    loop {
        if let Fetched::Changed { file, etag } = client.get_file_cond(
            &cfg.owner,
            &cfg.results_repo,
            &status_path,
            &cfg.results_branch,
            status_tag.as_deref(),
        )? {
            status_tag = etag;
            let v: serde_json::Value = serde_json::from_slice(&file.content).unwrap_or_default();
            row = Some(jobs::row_from_status(name.to_string(), &v));
        }
        // The log is read after the status: the daemon pushes the final log before the final
        // status, so once the status says finished, this read is the last word.
        if row.is_some() {
            if let Fetched::Changed { file, etag } = client.get_file_cond(
                &cfg.owner,
                &cfg.results_repo,
                &log_path,
                &cfg.results_branch,
                log_tag.as_deref(),
            )? {
                log_tag = etag;
                log = String::from_utf8_lossy(&file.content).into_owned();
                if !json {
                    if let Some(new) = log.get(shown..) {
                        print!("{new}");
                        std::io::stdout().flush().ok();
                    }
                    shown = log.len();
                }
            }
        }
        if row.as_ref().is_some_and(|r| r.status != "running") {
            break;
        }
        if start.elapsed() >= within {
            if row.is_none() {
                eprintln!(
                    "lab-agent: {name} was not picked up within {}s - is the agent running?",
                    within.as_secs()
                );
            } else {
                eprintln!(
                    "lab-agent: {name} did not finish within {}s (it keeps running)",
                    within.as_secs()
                );
            }
            return Ok(exit::STILL_RUNNING);
        }
        let pause = if start.elapsed() < Duration::from_secs(20) {
            1
        } else {
            3
        };
        std::thread::sleep(Duration::from_secs(pause));
    }

    let row = row.expect("a finished job has a row");
    let code = exit_code(&row);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "name": row.name,
                "status": row.status,
                "exit_code": row.exit_code,
                "started_at": row.started_at,
                "finished_at": row.finished_at,
                "elapsed_secs": row.elapsed_secs(),
                "exit": code,
                "log": log,
            }))?
        );
    } else {
        let took = row
            .elapsed_secs()
            .map(jobs::format_duration)
            .unwrap_or_else(|| "-".into());
        eprintln!("lab-agent: {name} {} (exit {code}) in {took}", row.status);
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: &str, exit_code: Option<i32>) -> JobRow {
        JobRow {
            name: "j".into(),
            status: status.into(),
            exit_code,
            started_at: None,
            finished_at: None,
        }
    }

    #[test]
    fn exit_statuses_are_stable() {
        assert_eq!(exit_code(&row("ok", Some(0))), 0);
        assert_eq!(exit_code(&row("failed", Some(2))), 2);
        assert_eq!(exit_code(&row("failed", None)), 1);
        assert_eq!(exit_code(&row("failed", Some(0))), 1);
        assert_eq!(exit_code(&row("timed_out", None)), 124);
        assert_eq!(exit_code(&row("preempted", None)), 125);
        assert_eq!(exit_code(&row("interrupted", None)), 125);
    }

    #[test]
    fn a_slug_is_short_and_safe() {
        assert_eq!(slug("Build the Kernel!"), "build-the-kernel");
        assert_eq!(slug("--  --"), "job");
        assert_eq!(slug(""), "job");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert!(slug(&"a".repeat(100)).len() <= 40);
    }

    #[test]
    fn the_header_goes_after_the_shebang_and_replaces_an_old_one() {
        let out = with_header(
            "#!/bin/sh\n# lab: mode=queue timeout=5\necho hi\n",
            Mode::Preempt,
            Duration::from_secs(90),
        );
        assert_eq!(out, "#!/bin/sh\n# lab: mode=preempt timeout=90\necho hi\n");
    }

    #[test]
    fn the_header_goes_first_without_a_shebang() {
        let out = with_header("echo hi", Mode::Queue, Duration::from_secs(300));
        assert_eq!(out, "# lab: mode=queue timeout=300\necho hi\n");
    }
}
