use crate::config::Config;
use crate::github::Client;
use crate::job::{self, Mode};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Reads the `# lab: mode=... timeout=...` header line a job script may carry. Defaults: queue,
/// 300s.
fn parse_header(script: &str) -> (Mode, Duration) {
    let mut mode = Mode::Queue;
    let mut timeout = Duration::from_secs(300);
    if let Some(line) = script
        .lines()
        .find(|l| l.trim_start().starts_with("# lab:"))
    {
        for tok in line.split_whitespace() {
            if let Some(v) = tok.strip_prefix("mode=") {
                if v == "preempt" {
                    mode = Mode::Preempt;
                }
            } else if let Some(v) = tok.strip_prefix("timeout=") {
                if let Ok(secs) = v.parse() {
                    timeout = Duration::from_secs(secs);
                }
            }
        }
    }
    (mode, timeout)
}

/// A job that has been found and not yet started.
struct Pending {
    file: String,
    script: String,
    mode: Mode,
    timeout: Duration,
}

impl Pending {
    fn name(&self) -> &str {
        self.file.trim_end_matches(".sh")
    }
}

/// What to do with the jobs waiting right now.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    /// Cancel everything that is running now (a preempt job is starting).
    cancel_running: bool,
    /// Indices into the waiting list, in the order to start them.
    start: Vec<usize>,
}

/// Decides which waiting jobs start. A preempt job always starts, first, and cancels what is
/// running; queue jobs start in order while there is room under `max_parallel`. Pure, so the
/// scheduling rules are tested without a network or a process.
fn plan(modes: &[Mode], running: usize, max_parallel: usize) -> Plan {
    let preempts: Vec<usize> = (0..modes.len())
        .filter(|&i| modes[i] == Mode::Preempt)
        .collect();
    let cancel_running = !preempts.is_empty();
    let mut running = if cancel_running { 0 } else { running };
    running += preempts.len();
    let mut start = preempts;
    for (i, mode) in modes.iter().enumerate() {
        if *mode == Mode::Queue && running < max_parallel {
            start.push(i);
            running += 1;
        }
    }
    Plan {
        cancel_running,
        start,
    }
}

struct Shared {
    client: Client,
    cfg: Config,
    /// The jobs running now, by file name, each with the flag that cancels it. A preempt job sets
    /// the flag of exactly the jobs running at that moment - there is no global "newest preempt"
    /// to go stale and kill every later job.
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// The results branch takes one commit at a time: concurrent writers conflict (HTTP 409).
    push_lock: Mutex<()>,
}

/// A poisoned lock means another thread panicked, not that the data is wrong; keep going.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The main loop: poll the jobs repo, dispatch unclaimed jobs, push results. Polling and dispatch
/// run independently of job *execution* - a job that hangs blocks only itself, never the poll
/// loop.
///
/// What counts as already handled is read from the results repo, not remembered: a job with a
/// result is never run again, so a restart (an update, a crash, a reboot) cannot re-run history.
/// A job found `running` after a restart was cut off by it, and is recorded as `interrupted`.
pub fn run(cfg: Config, token: String) -> Result<()> {
    let shared = Arc::new(Shared {
        client: Client::new(token),
        cfg: cfg.clone(),
        running: Mutex::new(HashMap::new()),
        push_lock: Mutex::new(()),
    });

    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir)?;
    let mut claimed = already_handled(&shared)?;

    println!(
        "lab-agent {}: polling {}/{} every {}s (max {} job(s) at once); {} job(s) already have results",
        env!("CARGO_PKG_VERSION"),
        cfg.owner,
        cfg.jobs_repo,
        cfg.poll_seconds,
        cfg.max_parallel,
        claimed.len()
    );

    loop {
        if let Err(e) = poll_once(&shared, &mut claimed, &state_dir) {
            eprintln!("lab-agent: poll error (will retry): {e:#}");
        }
        std::thread::sleep(Duration::from_secs(cfg.poll_seconds));
    }
}

/// Every job that already has a result. Any that says `running` was cut off by a restart.
fn already_handled(shared: &Shared) -> Result<HashSet<String>> {
    let cfg = &shared.cfg;
    let dirs =
        shared
            .client
            .list_dir(&cfg.owner, &cfg.results_repo, "jobs", &cfg.results_branch)?;
    for name in &dirs {
        let path = format!("jobs/{name}/status.json");
        let Some(f) =
            shared
                .client
                .get_file(&cfg.owner, &cfg.results_repo, &path, &cfg.results_branch)?
        else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_slice(&f.content).unwrap_or_default();
        let running =
            v["status"].as_str() == Some("running") || v["running"].as_bool() == Some(true);
        if running {
            let started = v["started_at"].as_f64().unwrap_or_else(now_unix);
            println!(
                "lab-agent: {name} was running when the agent stopped: recording it as interrupted"
            );
            let _ = push_status(shared, name, "interrupted", None, started, Some(now_unix()));
        }
    }
    Ok(dirs.into_iter().collect())
}

fn poll_once(shared: &Arc<Shared>, claimed: &mut HashSet<String>, state_dir: &Path) -> Result<()> {
    let cfg = &shared.cfg;
    let mut files = shared
        .client
        .list_dir(&cfg.owner, &cfg.jobs_repo, "jobs", &cfg.jobs_branch)?;
    files.retain(|f| f.ends_with(".sh") && !claimed.contains(f.trim_end_matches(".sh")));
    files.sort(); // job filenames are numbered, so this is run order
    if files.is_empty() {
        return Ok(());
    }

    let mut pending = Vec::new();
    for file in files {
        let path = format!("jobs/{file}");
        let Some(f) =
            shared
                .client
                .get_file(&cfg.owner, &cfg.jobs_repo, &path, &cfg.jobs_branch)?
        else {
            continue;
        };
        let script = String::from_utf8_lossy(&f.content).to_string();
        let (mode, timeout) = parse_header(&script);
        pending.push(Pending {
            file,
            script,
            mode,
            timeout,
        });
    }

    let modes: Vec<Mode> = pending.iter().map(|p| p.mode).collect();
    let running_now = lock(&shared.running).len();
    let plan = plan(&modes, running_now, cfg.max_parallel);
    if plan.cancel_running {
        for flag in lock(&shared.running).values() {
            flag.store(true, Ordering::SeqCst);
        }
    }
    for i in plan.start {
        let p = &pending[i];
        claimed.insert(p.name().to_string());
        spawn(shared, p, state_dir);
    }
    Ok(())
}

fn spawn(shared: &Arc<Shared>, p: &Pending, state_dir: &Path) {
    let cancel = Arc::new(AtomicBool::new(false));
    lock(&shared.running).insert(p.file.clone(), Arc::clone(&cancel));
    let shared = Arc::clone(shared);
    let state_dir: PathBuf = state_dir.to_path_buf();
    let (file, script, mode, timeout) = (p.file.clone(), p.script.clone(), p.mode, p.timeout);
    std::thread::spawn(move || {
        if let Err(e) = run_one(&shared, &file, &script, mode, timeout, &cancel, &state_dir) {
            eprintln!("lab-agent: job {file} failed to run: {e:#}");
        }
        lock(&shared.running).remove(&file);
    });
}

fn run_one(
    shared: &Shared,
    file: &str,
    script: &str,
    mode: Mode,
    timeout: Duration,
    cancel: &AtomicBool,
    state_dir: &Path,
) -> Result<()> {
    let job_name = file.trim_end_matches(".sh");
    println!(
        "lab-agent: starting {job_name} (mode={mode:?}, timeout={}s)",
        timeout.as_secs()
    );
    // A job that removes the state directory must not stop the next one from logging.
    std::fs::create_dir_all(state_dir)?;
    let started_at = now_unix();
    // A failed status push must not stop the job itself from running.
    if let Err(e) = push_status(shared, job_name, "running", None, started_at, None) {
        eprintln!("lab-agent: could not record {job_name} as running: {e:#}");
    }

    let log_path = state_dir.join(format!("{job_name}.log"));
    let result = job::run(script, timeout, &log_path, || cancel.load(Ordering::SeqCst))?;

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let status = if result.preempted {
        "preempted"
    } else if result.timed_out {
        "timed_out"
    } else if result.exit_code == Some(0) {
        "ok"
    } else {
        "failed"
    };
    println!("lab-agent: {job_name} finished: {status}");
    push_result(shared, job_name, status, result.exit_code, started_at, &log)
}

/// Runs `f` as the only writer to the results branch, trying again a few times: a conflict is
/// usually another commit landing a moment earlier.
fn with_retry<T>(shared: &Shared, what: &str, f: impl Fn() -> Result<T>) -> Result<T> {
    let _only_writer = lock(&shared.push_lock);
    let mut last = None;
    for attempt in 0..4u64 {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(400 * (attempt + 1)));
            }
        }
    }
    Err(last
        .expect("at least one attempt")
        .context(format!("pushing {what}")))
}

/// Seconds since the Unix epoch, as a float (fractional seconds), matching how `status.json`
/// has always recorded timestamps in this project.
fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn push_status(
    shared: &Shared,
    job_name: &str,
    status: &str,
    exit_code: Option<i32>,
    started_at: f64,
    finished_at: Option<f64>,
) -> Result<()> {
    let cfg = &shared.cfg;
    let path = format!("jobs/{job_name}/status.json");
    let body = serde_json::to_vec_pretty(&serde_json::json!({
        "name": job_name,
        "status": status,
        "exit_code": exit_code,
        "started_at": started_at,
        "finished_at": finished_at,
    }))?;
    with_retry(shared, &format!("{job_name} status"), || {
        let existing =
            shared
                .client
                .get_file(&cfg.owner, &cfg.results_repo, &path, &cfg.results_branch)?;
        shared.client.put_file(
            &cfg.owner,
            &cfg.results_repo,
            &path,
            &cfg.results_branch,
            &body,
            &format!("status: {job_name} {status}"),
            existing.map(|f| f.sha).as_deref(),
        )?;
        Ok(())
    })
}

fn push_result(
    shared: &Shared,
    job_name: &str,
    status: &str,
    exit_code: Option<i32>,
    started_at: f64,
    log: &str,
) -> Result<()> {
    let cfg = &shared.cfg;
    // The log first: a result whose status says "finished" must already have its log.
    let log_path = format!("jobs/{job_name}/log/0000.txt");
    with_retry(shared, &format!("{job_name} log"), || {
        let existing = shared.client.get_file(
            &cfg.owner,
            &cfg.results_repo,
            &log_path,
            &cfg.results_branch,
        )?;
        shared.client.put_file(
            &cfg.owner,
            &cfg.results_repo,
            &log_path,
            &cfg.results_branch,
            log.as_bytes(),
            &format!("log: {job_name}"),
            existing.map(|f| f.sha).as_deref(),
        )?;
        Ok(())
    })
    .context("the log")?;
    push_status(
        shared,
        job_name,
        status,
        exit_code,
        started_at,
        Some(now_unix()),
    )
}

pub fn state_dir() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".local/state"));
    base.join("lab-agent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use Mode::{Preempt, Queue};

    #[test]
    fn the_header_defaults_to_queue_and_five_minutes() {
        assert_eq!(parse_header("echo hi"), (Queue, Duration::from_secs(300)));
    }

    #[test]
    fn the_header_sets_mode_and_timeout() {
        let (mode, timeout) = parse_header("#!/bin/sh\n# lab: mode=preempt timeout=45\necho");
        assert_eq!((mode, timeout), (Preempt, Duration::from_secs(45)));
    }

    #[test]
    fn queue_jobs_start_in_order_up_to_the_limit() {
        let p = plan(&[Queue, Queue, Queue], 0, 2);
        assert_eq!(
            p,
            Plan {
                cancel_running: false,
                start: vec![0, 1]
            }
        );
    }

    #[test]
    fn queue_jobs_wait_when_the_agent_is_full() {
        let p = plan(&[Queue], 1, 1);
        assert_eq!(
            p,
            Plan {
                cancel_running: false,
                start: vec![]
            }
        );
    }

    #[test]
    fn a_preempt_job_cancels_what_runs_and_goes_first() {
        let p = plan(&[Queue, Preempt], 3, 1);
        assert_eq!(
            p,
            Plan {
                cancel_running: true,
                start: vec![1]
            }
        );
    }

    #[test]
    fn a_later_queue_job_is_not_cancelled_by_an_earlier_preempt_job() {
        // The bug this replaces: a global "newest preempt" killed every job after it. Now a
        // preempt job's effect ends once it has cancelled what was running.
        let p = plan(&[Queue], 0, 1);
        assert!(!p.cancel_running);
        assert_eq!(p.start, vec![0]);
    }
}
