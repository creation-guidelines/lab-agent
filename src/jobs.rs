use crate::config::Config;
use crate::contract::Contract;
use crate::github::Client;
use anyhow::Result;
use argenv_resolve::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct JobRow {
    pub name: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub started_at: Option<f64>,
    pub finished_at: Option<f64>,
    /// The job's own time limit, when it was recorded.
    pub timeout_secs: Option<u64>,
    /// "running" for longer than it can be: its result was never recorded (the agent died, or
    /// could not push). It is not a live job, whatever the status says.
    pub stale: bool,
}

impl JobRow {
    /// The status to show: a stale "running" is not shown as running.
    pub fn display_status(&self) -> &str {
        if self.stale {
            "stale"
        } else {
            &self.status
        }
    }

    /// Seconds the job has been running (still going) or took (finished). `None` for a job with
    /// no `started_at` yet (a result pushed before this field existed).
    pub fn elapsed_secs(&self) -> Option<f64> {
        let started = self.started_at?;
        let end = self.finished_at.unwrap_or_else(now_unix);
        Some((end - started).max(0.0))
    }
}

fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn run(cfg: Config, token: String, resolved: &Resolution) -> Result<()> {
    let count = Contract::JOBS_COUNT
        .get_from_or_default(resolved)
        .unwrap_or(20);
    let json = Contract::JSON
        .get_from_or_default(resolved)
        .unwrap_or(false);

    let client = Client::new(token);
    let rows = fetch_jobs(&client, &cfg, count as usize)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!(
        "{:<40} {:<12} {:<6} {:<9} ELAPSED",
        "JOB", "STATUS", "EXIT", "STARTED"
    );
    for row in &rows {
        let started = row
            .started_at
            .map(self::format_time)
            .unwrap_or_else(|| "-".to_string());
        let elapsed = row
            .elapsed_secs()
            .map(self::format_duration)
            .unwrap_or_else(|| "-".to_string());
        let exit = row
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<40} {:<12} {:<6} {:<9} {}",
            row.name,
            row.display_status(),
            exit,
            started,
            elapsed
        );
    }
    for row in rows.iter().filter(|r| r.stale) {
        println!(
            "! {} says running but is stale: past its {}s limit with no result recorded",
            row.name,
            row.timeout_secs
                .map_or("unknown".to_string(), |t| t.to_string())
        );
    }
    Ok(())
}

/// Lists the `count` most recent jobs from the results repo with their status and timestamps.
/// Shared by the `jobs` subcommand and the `monitor` TUI, so both show the same data the same
/// way.
pub fn fetch_jobs(client: &Client, cfg: &Config, count: usize) -> Result<Vec<JobRow>> {
    let mut job_dirs =
        client.list_dir(&cfg.owner, &cfg.results_repo, "jobs", &cfg.results_branch)?;
    job_dirs.sort();
    job_dirs.reverse(); // newest first (job names are numbered)
    job_dirs.truncate(count);

    let mut rows = Vec::new();
    for name in job_dirs {
        let path = format!("jobs/{name}/status.json");
        if let Some(f) =
            client.get_file(&cfg.owner, &cfg.results_repo, &path, &cfg.results_branch)?
        {
            let v: serde_json::Value = serde_json::from_slice(&f.content).unwrap_or_default();
            rows.push(row_from_status(name, &v));
        }
    }
    Ok(rows)
}

/// A job's row from its `status.json`. Results written by the earlier (Python) agent carry
/// `running`/`ok` booleans instead of a `status` word; read them too, so history still lists.
pub fn row_from_status(name: String, v: &serde_json::Value) -> JobRow {
    let status = match v["status"].as_str() {
        Some(s) => s.to_string(),
        None if v["running"].as_bool() == Some(true) => "running".to_string(),
        None if v["ok"].as_bool() == Some(true) => "ok".to_string(),
        None if v["ok"].as_bool() == Some(false) => "failed".to_string(),
        None => "?".to_string(),
    };
    let mut row = JobRow {
        name,
        status,
        exit_code: v["exit_code"].as_i64().map(|c| c as i32),
        started_at: v["started_at"].as_f64(),
        finished_at: v["finished_at"].as_f64(),
        timeout_secs: v["timeout"].as_u64(),
        stale: false,
    };
    row.stale = is_stale(&row, now_unix());
    row
}

/// Slack after the time limit for the job's cleanup (SIGTERM and its grace period) and for the
/// result to be pushed.
const STALE_GRACE: f64 = 120.0;
/// A "running" job with no recorded time limit is stale after this long.
const STALE_UNKNOWN_TIMEOUT: f64 = 3600.0;

/// Whether a job still marked running has outlived its time limit plus slack.
pub fn is_stale(row: &JobRow, now: f64) -> bool {
    let Some(started) = row.started_at else {
        return false;
    };
    let limit = row.timeout_secs.map_or(STALE_UNKNOWN_TIMEOUT, |t| t as f64);
    row.status == "running" && now > started + limit + STALE_GRACE
}

/// A plain HH:MM:SS in UTC. No chrono/time dependency needed for something this simple; shared
/// by the `jobs` subcommand and the `monitor` TUI so both show times the same way.
pub fn format_time(unix: f64) -> String {
    let secs = unix as i64;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

pub fn format_duration(secs: f64) -> String {
    let secs = secs as u64;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: &str, started: Option<f64>, timeout: Option<u64>) -> JobRow {
        JobRow {
            name: "j".into(),
            status: status.into(),
            exit_code: None,
            started_at: started,
            finished_at: None,
            timeout_secs: timeout,
            stale: false,
        }
    }

    #[test]
    fn a_running_job_past_its_limit_is_stale() {
        assert!(is_stale(
            &row("running", Some(1000.0), Some(180)),
            1000.0 + 180.0 + 121.0
        ));
    }

    #[test]
    fn a_running_job_within_its_limit_is_not() {
        assert!(!is_stale(
            &row("running", Some(1000.0), Some(180)),
            1000.0 + 180.0 + 60.0
        ));
    }

    #[test]
    fn a_finished_job_is_never_stale() {
        assert!(!is_stale(&row("ok", Some(1000.0), Some(10)), 1_000_000.0));
    }

    #[test]
    fn an_unknown_limit_means_an_hour_and_slack() {
        assert!(!is_stale(&row("running", Some(0.0), None), 3600.0 + 100.0));
        assert!(is_stale(&row("running", Some(0.0), None), 3600.0 + 121.0));
    }

    #[test]
    fn a_stale_row_is_shown_as_stale() {
        let mut r = row("running", Some(0.0), Some(1));
        r.stale = true;
        assert_eq!(r.display_status(), "stale");
    }
}
