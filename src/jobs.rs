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
}

impl JobRow {
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
    let json = Contract::JOBS_JSON
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
            row.name, row.status, exit, started, elapsed
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
            rows.push(JobRow {
                name,
                status: v["status"].as_str().unwrap_or("?").to_string(),
                exit_code: v["exit_code"].as_i64().map(|c| c as i32),
                started_at: v["started_at"].as_f64(),
                finished_at: v["finished_at"].as_f64(),
            });
        }
    }
    Ok(rows)
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
