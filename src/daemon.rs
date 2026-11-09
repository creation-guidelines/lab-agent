use crate::config::Config;
use crate::github::Client;
use crate::job::{self, Mode};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Reads the `# lab: mode=... timeout=...` header line a job script may
/// carry. Defaults: queue, 300s.
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

struct Shared {
    client: Client,
    cfg: Config,
    /// Name of the newest preempt-mode job seen, once one exists; a running
    /// job compares itself against this to know when to stop early.
    newest_preempt: Mutex<Option<String>>,
    running: AtomicUsize,
}

/// The main loop: poll the jobs repo, dispatch unclaimed jobs, push
/// results. Polling and dispatch run independently of job *execution* - a
/// job that hangs blocks only itself, never the poll loop, which is the bug
/// this rewrite exists to fix (a leftover `fastboot reboot` child wedged
/// the whole previous agent because nothing else was watching for new work
/// while it waited).
pub fn run(cfg: Config, token: String) -> Result<()> {
    let shared = Arc::new(Shared {
        client: Client::new(token),
        cfg: cfg.clone(),
        newest_preempt: Mutex::new(None),
        running: AtomicUsize::new(0),
    });

    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir)?;
    let mut claimed: HashMap<String, ()> = HashMap::new();

    println!(
        "lab-agent: polling {}/{} every {}s (max {} job(s) at once)",
        cfg.owner, cfg.jobs_repo, cfg.poll_seconds, cfg.max_parallel
    );

    loop {
        match poll_once(&shared, &mut claimed, &state_dir) {
            Ok(()) => {}
            Err(e) => eprintln!("lab-agent: poll error (will retry): {e}"),
        }
        std::thread::sleep(Duration::from_secs(cfg.poll_seconds));
    }
}

fn poll_once(
    shared: &Arc<Shared>,
    claimed: &mut HashMap<String, ()>,
    state_dir: &Path,
) -> Result<()> {
    let names = shared.client.list_dir(
        &shared.cfg.owner,
        &shared.cfg.jobs_repo,
        "jobs",
        &shared.cfg.jobs_branch,
    )?;

    let mut names: Vec<String> = names.into_iter().filter(|n| n.ends_with(".sh")).collect();
    names.sort(); // job filenames are numbered, so this is run order

    // Track the newest preempt-mode job so any currently running job can see it.
    if let Some(newest) = names.iter().rev().find(|n| !claimed.contains_key(*n)) {
        let file = shared.client.get_file(
            &shared.cfg.owner,
            &shared.cfg.jobs_repo,
            &format!("jobs/{newest}"),
            &shared.cfg.jobs_branch,
        )?;
        if let Some(f) = file {
            let script = String::from_utf8_lossy(&f.content).to_string();
            let (mode, _) = parse_header(&script);
            if mode == Mode::Preempt {
                *shared.newest_preempt.lock().unwrap() = Some(newest.clone());
            }
        }
    }

    for name in names {
        if claimed.contains_key(&name) {
            continue;
        }
        if shared.running.load(Ordering::SeqCst) >= shared.cfg.max_parallel {
            break; // at capacity; the rest wait for the next poll
        }
        claimed.insert(name.clone(), ());

        let shared = Arc::clone(shared);
        let state_dir: PathBuf = state_dir.to_path_buf();
        shared.running.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            if let Err(e) = run_one(&shared, &name, &state_dir) {
                eprintln!("lab-agent: job {name} failed to run: {e}");
            }
            shared.running.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}

fn run_one(shared: &Shared, name: &str, state_dir: &Path) -> Result<()> {
    let job_name = name.trim_end_matches(".sh");
    let file = shared.client.get_file(
        &shared.cfg.owner,
        &shared.cfg.jobs_repo,
        &format!("jobs/{name}"),
        &shared.cfg.jobs_branch,
    )?;
    let Some(file) = file else { return Ok(()) };
    let script = String::from_utf8_lossy(&file.content).to_string();
    let (mode, timeout) = parse_header(&script);

    println!(
        "lab-agent: starting {job_name} (mode={mode:?}, timeout={}s)",
        timeout.as_secs()
    );
    let started_at = now_unix();
    push_status(shared, job_name, "running", None, started_at, None)?;

    // A running job is cancelled early only by a *different, newer* preempt-mode job - never by
    // itself, and never just because some other ordinary queued job exists.
    let own_filename = format!("{job_name}.sh");
    let log_path = state_dir.join(format!("{job_name}.log"));
    let result = job::run(&script, timeout, &log_path, || {
        shared
            .newest_preempt
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|newest| newest != own_filename)
    })?;

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
    push_result(shared, job_name, status, result.exit_code, started_at, &log)?;
    Ok(())
}

/// Seconds since the Unix epoch, as a float (fractional seconds), matching how `status.json`
/// has always recorded timestamps in this project.
fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[allow(clippy::too_many_arguments)]
fn push_status(
    shared: &Shared,
    job_name: &str,
    status: &str,
    exit_code: Option<i32>,
    started_at: f64,
    finished_at: Option<f64>,
) -> Result<()> {
    let path = format!("jobs/{job_name}/status.json");
    let body = serde_json::json!({
        "name": job_name,
        "status": status,
        "exit_code": exit_code,
        "started_at": started_at,
        "finished_at": finished_at,
    });
    let existing = shared.client.get_file(
        &shared.cfg.owner,
        &shared.cfg.results_repo,
        &path,
        &shared.cfg.results_branch,
    )?;
    shared.client.put_file(
        &shared.cfg.owner,
        &shared.cfg.results_repo,
        &path,
        &shared.cfg.results_branch,
        serde_json::to_vec_pretty(&body)?.as_slice(),
        &format!("status: {job_name} {status}"),
        existing.map(|f| f.sha).as_deref(),
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_result(
    shared: &Shared,
    job_name: &str,
    status: &str,
    exit_code: Option<i32>,
    started_at: f64,
    log: &str,
) -> Result<()> {
    push_status(
        shared,
        job_name,
        status,
        exit_code,
        started_at,
        Some(now_unix()),
    )?;
    let log_path = format!("jobs/{job_name}/log/0000.txt");
    let existing = shared.client.get_file(
        &shared.cfg.owner,
        &shared.cfg.results_repo,
        &log_path,
        &shared.cfg.results_branch,
    )?;
    shared.client.put_file(
        &shared.cfg.owner,
        &shared.cfg.results_repo,
        &log_path,
        &shared.cfg.results_branch,
        log.as_bytes(),
        &format!("log: {job_name}"),
        existing.map(|f| f.sha).as_deref(),
    )?;
    Ok(())
}

pub fn state_dir() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".local/state"));
    base.join("lab-agent")
}
