use crate::config::{self, Config};
use crate::contract::Contract;
use crate::github::{Client, Fetched, Listed};
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

pub fn run(resolved: &Resolution) -> Result<()> {
    let count = Contract::JOBS_COUNT
        .get_from_or_default(resolved)
        .unwrap_or(20);
    let json = Contract::JSON
        .get_from_or_default(resolved)
        .unwrap_or(false);

    let rows = match source(resolved) {
        Source::Local(dir) => {
            if !json {
                eprintln!("(this machine's own state; --remote reads GitHub)");
            }
            local_rows(&dir, count as usize)
        }
        Source::Remote => {
            let (cfg, client) = (config::load()?, Client::new(config::load_token()?));
            fetch_jobs(&client, &cfg, count as usize)?
        }
    };

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

/// Where job state is read from.
pub enum Source {
    /// This machine's own state directory: no network, no token, and it works while GitHub is
    /// unreachable or refusing.
    Local(std::path::PathBuf),
    /// The results repo on GitHub: the way for anything that is not the machine running the agent.
    Remote,
}

/// Local state when this machine runs the agent (its state directory holds job records), unless
/// `--remote` asks for GitHub.
pub fn source(resolved: &Resolution) -> Source {
    let dir = crate::daemon::state_dir();
    let remote = Contract::REMOTE
        .get_from_or_default(resolved)
        .unwrap_or(false);
    if remote || !has_local_state(&dir) {
        Source::Remote
    } else {
        Source::Local(dir)
    }
}

fn is_record(name: &str) -> bool {
    name.ends_with(".status.json")
}

pub fn has_local_state(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_str().is_some_and(is_record))
        })
        .unwrap_or(false)
}

/// The `count` newest jobs the agent on this machine has recorded, newest first.
pub fn local_rows(dir: &std::path::Path, count: usize) -> Vec<JobRow> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".status.json")
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names.reverse();
    names.truncate(count);
    names
        .into_iter()
        .filter_map(|name| {
            let bytes = std::fs::read(dir.join(format!("{name}.status.json"))).ok()?;
            let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            Some(row_from_status(name, &v))
        })
        .collect()
}

/// Lists the `count` most recent jobs from the results repo with their status and timestamps.
/// Shared by the `jobs` subcommand and the `monitor` TUI, so both show the same data the same
/// way.
pub fn fetch_jobs(client: &Client, cfg: &Config, count: usize) -> Result<Vec<JobRow>> {
    fetch_jobs_cached(client, cfg, count, &mut JobsCache::default())
}

/// What a long-lived view (the monitor) remembers between refreshes. Every read is conditional: a
/// file that has not changed is answered "not modified", which GitHub does not count against the
/// hourly budget. A finished job never changes, so after the first load almost every refresh is
/// free - without this, a monitor re-reading 30 jobs every few seconds spends the whole budget
/// (shared with everything else the user does on GitHub) in minutes.
#[derive(Default)]
pub struct JobsCache {
    list_etag: Option<String>,
    names: Vec<String>,
    rows: std::collections::HashMap<String, (Option<String>, JobRow)>,
}

/// The `count` most recent jobs, newest first, reading only what changed since `cache` was filled.
pub fn fetch_jobs_cached(
    client: &Client,
    cfg: &Config,
    count: usize,
    cache: &mut JobsCache,
) -> Result<Vec<JobRow>> {
    match client.list_dir_cond(
        &cfg.owner,
        &cfg.results_repo,
        "jobs",
        &cfg.results_branch,
        cache.list_etag.as_deref(),
    )? {
        Listed::Changed { names, etag } => {
            cache.list_etag = etag;
            cache.names = names;
        }
        Listed::Unchanged => {}
        Listed::Missing => {
            cache.list_etag = None;
            cache.names.clear();
        }
    }
    let mut names = cache.names.clone();
    names.sort();
    names.reverse(); // newest first (job names start with a time)
    names.truncate(count);

    let mut rows = Vec::new();
    for name in names {
        let path = format!("jobs/{name}/status.json");
        let tag = cache.rows.get(&name).and_then(|(tag, _)| tag.clone());
        match client.get_file_cond(
            &cfg.owner,
            &cfg.results_repo,
            &path,
            &cfg.results_branch,
            tag.as_deref(),
        )? {
            Fetched::Changed { file, etag } => {
                let v: serde_json::Value =
                    serde_json::from_slice(&file.content).unwrap_or_default();
                let row = row_from_status(name.clone(), &v);
                cache.rows.insert(name, (etag, row.clone()));
                rows.push(row);
            }
            Fetched::Unchanged => {
                if let Some((_, row)) = cache.rows.get(&name) {
                    // Staleness depends on the clock, not on the file: recompute it.
                    let mut row = row.clone();
                    row.stale = is_stale(&row, now_unix());
                    rows.push(row);
                }
            }
            Fetched::Missing => {}
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
/// A time of day on this machine's clock (its timezone, as the system reports it now).
pub fn format_time(unix: f64) -> String {
    clock(unix, local_offset())
}

/// The time of day `offset` seconds east of UTC.
pub fn clock(unix: f64, offset: i64) -> String {
    let secs = (unix as i64 + offset).rem_euclid(86_400);
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

/// This machine's offset from UTC in seconds, asked of the system once.
fn local_offset() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as libc::time_t)
            .unwrap_or(0);
        // SAFETY: localtime_r only writes the tm it is given; both live for the whole call.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&now, &mut tm).is_null() {
                0
            } else {
                tm.tm_gmtoff as i64
            }
        }
    })
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

    use base64::Engine as _;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A fake GitHub with two finished jobs. It records every request and whether it was
    /// conditional (carried If-None-Match), and answers a conditional one with 304.
    fn fake_github(seen: Arc<Mutex<Vec<(String, bool)>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                // Read to the end of the headers: one read() is not one request.
                let (mut got, mut buf) = (Vec::new(), [0u8; 8192]);
                while !String::from_utf8_lossy(&got).contains("\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&got).to_string();
                let path = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split(' ')
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                let conditional = head
                    .lines()
                    .any(|l| l.to_ascii_lowercase().starts_with("if-none-match:"));
                seen.lock().unwrap().push((path.clone(), conditional));
                let reply = if conditional {
                    "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    let body = if path.contains("/status.json") {
                        let status = r#"{"status":"ok","exit_code":0,"started_at":100.0,"finished_at":105.0,"timeout":60}"#;
                        format!(
                            r#"{{"content":"{}","sha":"s"}}"#,
                            base64::engine::general_purpose::STANDARD.encode(status)
                        )
                    } else {
                        r#"[{"name":"1000-a"},{"name":"2000-b"}]"#.to_string()
                    };
                    format!(
                        "HTTP/1.1 200 OK\r\nETag: \"tag\"\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                stream.write_all(reply.as_bytes()).ok();
            }
        });
        format!("http://{addr}")
    }

    fn test_config() -> Config {
        Config {
            owner: "o".into(),
            jobs_repo: "j".into(),
            results_repo: "r".into(),
            jobs_branch: "main".into(),
            results_branch: "results".into(),
            max_parallel: 1,
            poll_seconds: 5,
        }
    }

    #[test]
    fn a_refresh_with_nothing_new_is_all_conditional_and_still_returns_the_rows() {
        // The property that keeps a monitor from spending GitHub's hourly budget: a 304 is free.
        let seen = Arc::new(Mutex::new(Vec::new()));
        let client = Client::with_base("token".into(), &fake_github(seen.clone()));
        let mut cache = JobsCache::default();

        let first = fetch_jobs_cached(&client, &test_config(), 30, &mut cache).unwrap();
        assert_eq!(
            first.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["2000-b", "1000-a"]
        );
        let after_first = seen.lock().unwrap().len();
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .all(|(_, conditional)| !conditional),
            "nothing to be conditional on yet"
        );

        let second = fetch_jobs_cached(&client, &test_config(), 30, &mut cache).unwrap();
        let calls = seen.lock().unwrap()[after_first..].to_vec();
        assert_eq!(calls.len(), 3, "the listing and two statuses");
        assert!(
            calls.iter().all(|(_, conditional)| *conditional),
            "every re-read must be conditional: {calls:?}"
        );
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].status, "ok");
        assert_eq!(second[0].timeout_secs, Some(60));
    }

    #[test]
    fn the_clock_shows_the_time_of_day_in_the_given_offset() {
        assert_eq!(clock(0.0, 0), "00:00:00");
        assert_eq!(
            clock(3600.0 * 25.0 + 61.0, 3600),
            "02:01:01",
            "an hour east of UTC"
        );
        assert_eq!(
            clock(1800.0, -3600),
            "23:30:00",
            "west of UTC, the day before"
        );
    }
}
