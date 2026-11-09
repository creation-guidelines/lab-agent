use crate::config::Config;
use crate::github::{Client, Listed, RateLimited};
use crate::job::{self, Mode};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Reads the `# lab: mode=... timeout=...` header line a job script may carry. Defaults: queue,
/// 300s.
pub(crate) fn parse_header(script: &str) -> (Mode, Duration) {
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

/// Whether the job's `# lab:` header asks for a phone notification of how it ended.
pub(crate) fn wants_notify(script: &str) -> bool {
    script
        .lines()
        .find(|l| l.trim_start().starts_with("# lab:"))
        .is_some_and(|l| {
            l.split_whitespace()
                .any(|t| matches!(t, "notify=yes" | "notify=true" | "notify=1"))
        })
}

/// Tells the user on the phone how a job that asked for it ended: info if it succeeded, urgent
/// otherwise, with its last lines. Never fails or delays the job's result.
fn notify_outcome(job: &str, done: &Done, log: &str) {
    let Some(cfg) = crate::notify::load() else {
        return;
    };
    let took = crate::jobs::format_duration((done.finished_at - done.started_at).max(0.0));
    let how = done
        .exit_code
        .map_or(done.status.clone(), |c| format!("exit {c}"));
    let tail: Vec<&str> = log
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(6)
        .collect();
    let mut body = format!("{how} after {took}");
    if !tail.is_empty() {
        body.push_str("\n\n");
        body.push_str(&tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    let body: String = body.chars().take(900).collect();
    let level = if done.status == "ok" {
        crate::notify::Level::Info
    } else {
        crate::notify::Level::Urgent
    };
    let title = format!("{job}: {}", done.status);
    if let Err(e) = crate::notify::Notifier::new(cfg).send(level, &title, &body) {
        eprintln!("lab-agent: {e:#}");
    }
}

/// How often a running job's output so far is pushed: often at first, when someone is likely
/// watching it start, then less and less. GitHub caps content writes at about 500 an hour, and an
/// hour-long build pushing every 8 seconds would come close on its own.
fn partial_log_every(running_for: Duration) -> Duration {
    match running_for.as_secs() {
        0..=119 => Duration::from_secs(8),
        120..=599 => Duration::from_secs(30),
        _ => Duration::from_secs(60),
    }
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

/// What one poll remembers for the next, so that an idle poll costs nothing.
#[derive(Default)]
struct PollState {
    /// The ETag of the last listing of the jobs directory: GitHub answers "not modified" for free.
    etag: Option<String>,
    /// That listing, reused when nothing changed.
    names: Vec<String>,
    /// Jobs found but not started because the agent was full, with their scripts, so waiting for
    /// room costs no requests either.
    waiting: HashMap<String, Pending>,
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
    /// Jobs whose outcome is being pushed right now, so the retry pass does not push them twice.
    in_flight: Mutex<HashSet<String>>,
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
        in_flight: Mutex::new(HashSet::new()),
    });

    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir)?;
    prune_state(&state_dir, KEEP_LOCAL);
    let mut claimed = already_handled(&shared, &state_dir)?;

    println!(
        "lab-agent {}: polling {}/{} every {}s (max {} job(s) at once); {} job(s) already have results",
        env!("CARGO_PKG_VERSION"),
        cfg.owner,
        cfg.jobs_repo,
        cfg.poll_seconds,
        cfg.max_parallel,
        claimed.len()
    );

    {
        let shared = Arc::clone(&shared);
        let state_dir = state_dir.clone();
        std::thread::spawn(move || watch_board_and_inbox(&shared, &state_dir));
    }

    let mut state = PollState::default();
    loop {
        flush_unsent(&shared, &state_dir);
        let pause = match poll_once(&shared, &mut claimed, &state_dir, &mut state) {
            Ok(()) => Duration::from_secs(cfg.poll_seconds),
            // A spent budget is not a transient error: asking again only spends the next hour's too.
            Err(e) => match e.downcast_ref::<RateLimited>() {
                Some(limited) => {
                    eprintln!("lab-agent: {limited}; not polling until then");
                    limited.wait().min(Duration::from_secs(3600))
                }
                None => {
                    eprintln!("lab-agent: poll error (will retry): {e:#}");
                    Duration::from_secs(cfg.poll_seconds)
                }
            },
        };
        std::thread::sleep(pause);
    }
}

/// Every job that already has a result. Any that says `running` was cut off by a restart.
fn already_handled(shared: &Shared, state_dir: &Path) -> Result<HashSet<String>> {
    let cfg = &shared.cfg;
    let dirs =
        shared
            .client
            .list_dir(&cfg.owner, &cfg.results_repo, "jobs", &cfg.results_branch)?;
    // Only recent jobs can be "running": reading every old result on every start costs a request
    // each (hundreds), for nothing.
    let mut recent = dirs.clone();
    recent.sort();
    recent.reverse();
    for name in recent.iter().take(RECENT_TO_CHECK) {
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
            // A finished job whose result is still waiting to be pushed is not an interrupted one:
            // the retry pass has its real outcome.
            if done_path(state_dir, name).exists() {
                continue;
            }
            let started = v["started_at"].as_f64().unwrap_or_else(now_unix);
            println!(
                "lab-agent: {name} was running when the agent stopped: recording it as interrupted"
            );
            let timeout = v["timeout"].as_u64().unwrap_or(0);
            record_local(
                state_dir,
                name,
                "interrupted",
                None,
                started,
                Some(now_unix()),
                timeout,
            );
            let _ = push_status(
                shared,
                name,
                "interrupted",
                None,
                started,
                Some(now_unix()),
                timeout,
            );
        }
    }
    Ok(dirs.into_iter().collect())
}

fn poll_once(
    shared: &Arc<Shared>,
    claimed: &mut HashSet<String>,
    state_dir: &Path,
    state: &mut PollState,
) -> Result<()> {
    let cfg = &shared.cfg;
    match shared.client.list_dir_cond(
        &cfg.owner,
        &cfg.jobs_repo,
        "jobs",
        &cfg.jobs_branch,
        state.etag.as_deref(),
    )? {
        Listed::Changed { names, etag } => {
            state.etag = etag;
            state.names = names;
        }
        Listed::Unchanged => {}
        Listed::Missing => {
            state.etag = None;
            state.names.clear();
        }
    }
    let names = &state.names;
    state.waiting.retain(|file, _| names.contains(file));

    let mut files: Vec<String> = state
        .names
        .iter()
        .filter(|f| f.ends_with(".sh") && !claimed.contains(f.trim_end_matches(".sh")))
        .cloned()
        .collect();
    files.sort(); // job filenames are numbered, so this is run order
    if files.is_empty() {
        return Ok(());
    }

    let mut pending = Vec::new();
    for file in files {
        if let Some(waiting) = state.waiting.remove(&file) {
            pending.push(waiting);
            continue;
        }
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
    for &i in &plan.start {
        let p = &pending[i];
        claimed.insert(p.name().to_string());
        spawn(shared, p, state_dir);
    }
    for (i, p) in pending.into_iter().enumerate() {
        if !plan.start.contains(&i) {
            state.waiting.insert(p.file.clone(), p);
        }
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
    record_local(
        state_dir,
        job_name,
        "running",
        None,
        started_at,
        None,
        timeout.as_secs(),
    );
    // A failed status push must not stop the job itself from running.
    if let Err(e) = push_status(
        shared,
        job_name,
        "running",
        None,
        started_at,
        None,
        timeout.as_secs(),
    ) {
        eprintln!("lab-agent: could not record {job_name} as running: {e:#}");
    }

    let log_path = state_dir.join(format!("{job_name}.log"));
    // While the job runs, push what it has printed so far every few seconds, so whoever waits on
    // it (`lab-agent wait`, `lab-agent logs`) sees a long build as it happens, not all at the end.
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let begun = std::time::Instant::now();
            let tick = Duration::from_millis(400);
            let (mut sent, mut since) = (0, Duration::ZERO);
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(tick);
                since += tick;
                if since < partial_log_every(begun.elapsed()) {
                    continue;
                }
                since = Duration::ZERO;
                let text = read_log(&log_path);
                if text.len() != sent {
                    sent = text.len();
                    if let Err(e) = push_log(shared, job_name, &text) {
                        eprintln!("lab-agent: partial log of {job_name}: {e:#}");
                    }
                }
            }
        });
        let result = job::run(script, timeout, &log_path, || cancel.load(Ordering::SeqCst));
        done.store(true, Ordering::SeqCst);
        result
    })?;

    let log = read_log(&log_path);
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
    let done = Done {
        status: status.to_string(),
        exit_code: result.exit_code,
        started_at,
        finished_at: now_unix(),
        timeout_secs: timeout.as_secs(),
    };
    record_local(
        state_dir,
        job_name,
        &done.status,
        done.exit_code,
        done.started_at,
        Some(done.finished_at),
        done.timeout_secs,
    );
    // The outcome goes to disk first and to GitHub second: it must survive a failed push, a
    // spent budget and a restart. The marker stays until GitHub has the result.
    lock(&shared.in_flight).insert(job_name.to_string());
    if let Err(e) = write_done(state_dir, job_name, &done) {
        eprintln!("lab-agent: could not keep {job_name}'s outcome on disk: {e:#}");
    }
    let pushed = push_done(shared, job_name, &done, &log);
    if pushed.is_ok() {
        let _ = std::fs::remove_file(done_path(state_dir, job_name));
    }
    lock(&shared.in_flight).remove(job_name);
    if wants_notify(script) {
        notify_outcome(job_name, &done, &log);
    }
    pushed
}

/// Runs `f` as the only writer to the results branch, trying again a few times: a conflict is
/// usually another commit landing a moment earlier.
fn with_retry<T>(shared: &Shared, what: &str, f: impl Fn() -> Result<T>) -> Result<T> {
    let _only_writer = lock(&shared.push_lock);
    let (mut attempt, mut limit_waits) = (0u64, 0);
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                // A result must not be lost because the hourly budget ran out: wait for it to
                // return (a couple of times at most) without using up an attempt.
                if let Some(limited) = e.downcast_ref::<RateLimited>() {
                    if limit_waits < 2 {
                        limit_waits += 1;
                        let wait = limited.wait().min(Duration::from_secs(3900));
                        eprintln!(
                            "lab-agent: {what}: {limited}; waiting {}s to push it",
                            wait.as_secs()
                        );
                        std::thread::sleep(wait);
                        continue;
                    }
                }
                attempt += 1;
                if attempt >= 4 {
                    return Err(e.context(format!("pushing {what}")));
                }
                std::thread::sleep(Duration::from_millis(400 * attempt));
            }
        }
    }
}

/// Seconds since the Unix epoch, as a float (fractional seconds), matching how `status.json`
/// has always recorded timestamps in this project.
fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// A job's `status.json`: the same document goes to GitHub and, next to the job's log, to this
/// machine's own state directory, so a viewer on the machine needs no network at all.
fn status_json(
    job_name: &str,
    status: &str,
    exit_code: Option<i32>,
    started_at: f64,
    finished_at: Option<f64>,
    timeout_secs: u64,
) -> serde_json::Value {
    serde_json::json!({
        "name": job_name,
        "status": status,
        "exit_code": exit_code,
        "started_at": started_at,
        "finished_at": finished_at,
        "timeout": timeout_secs,
    })
}

/// Keeps `<job>.status.json` in the state directory current (written atomically). Best effort: a
/// failure here must never stop a job or its push.
fn record_local(
    state_dir: &Path,
    job_name: &str,
    status: &str,
    exit_code: Option<i32>,
    started_at: f64,
    finished_at: Option<f64>,
    timeout_secs: u64,
) {
    let body = status_json(
        job_name,
        status,
        exit_code,
        started_at,
        finished_at,
        timeout_secs,
    );
    let path = state_dir.join(format!("{job_name}.status.json"));
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(&body)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(Into::into))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(Into::into));
    if let Err(e) = written {
        eprintln!("lab-agent: could not record {job_name} locally: {e:#}");
    }
}

/// Keeps the newest `keep` jobs' local files (status and log) and removes older ones, so the
/// state directory does not grow for ever. Outcome markers of unsent results are never removed.
fn prune_state(state_dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(state_dir) else {
        return;
    };
    let mut jobs: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".status.json")
                .map(str::to_string)
        })
        .collect();
    jobs.sort();
    jobs.reverse(); // newest first (job names start with a time)
    for old in jobs.into_iter().skip(keep) {
        if done_path(state_dir, &old).exists() {
            continue;
        }
        let _ = std::fs::remove_file(state_dir.join(format!("{old}.status.json")));
        let _ = std::fs::remove_file(state_dir.join(format!("{old}.log")));
    }
}

fn push_status(
    shared: &Shared,
    job_name: &str,
    status: &str,
    exit_code: Option<i32>,
    started_at: f64,
    finished_at: Option<f64>,
    timeout_secs: u64,
) -> Result<()> {
    let cfg = &shared.cfg;
    let path = format!("jobs/{job_name}/status.json");
    let body = serde_json::to_vec_pretty(&status_json(
        job_name,
        status,
        exit_code,
        started_at,
        finished_at,
        timeout_secs,
    ))?;
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

/// How often the board and the inbox are looked at. Both reads are free when nothing changed (a
/// conditional GitHub read; ntfy is not GitHub).
const BOARD_EVERY: Duration = Duration::from_secs(30);

/// Keeps the user told while the agent works, from this machine, so nobody spends tokens on it:
/// the open tasks every `every_minutes`, a note when they are all done, a receipt when inbox
/// messages are read, each new reply; and the user's new inbox messages filed for the agent.
fn watch_board_and_inbox(shared: &Shared, state_dir: &Path) {
    use crate::board::{self, Board, Inbox};
    use crate::notify::{Level, Notifier};
    let cfg = &shared.cfg;
    let cursor_path = state_dir.join("inbox.cursor");
    let sent_path = state_dir.join("board.replies-sent");
    let mut cursor = std::fs::read_to_string(&cursor_path)
        .ok()
        .map(|s| s.trim().to_string());
    let mut sent: HashSet<String> = std::fs::read_to_string(&sent_path)
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();
    let (mut etag, mut board, mut seen_once) = (None::<String>, Board::default(), false);
    let inbox_local = state_dir.join(board::INBOX_PATH);
    // the conversation so far, so the board's live view on this machine has it without network
    if let Ok(Some(f)) = shared.client.get_file(
        &cfg.owner,
        &cfg.results_repo,
        board::INBOX_PATH,
        &cfg.results_branch,
    ) {
        let _ = std::fs::write(&inbox_local, &f.content);
    }
    let mut last_digest = std::time::Instant::now() - Duration::from_secs(24 * 3600);
    loop {
        std::thread::sleep(BOARD_EVERY);
        let Some(ncfg) = crate::notify::load() else {
            continue; // notifications are not set up on this machine
        };
        let notifier = Notifier::new(ncfg.clone());
        // 1. the board (a free 304 when unchanged)
        match shared.client.get_file_cond(
            &cfg.owner,
            &cfg.jobs_repo,
            board::BOARD_PATH,
            &cfg.jobs_branch,
            etag.as_deref(),
        ) {
            Ok(crate::github::Fetched::Changed { file, etag: tag }) => {
                let new = board::parse_board(&file.content);
                if seen_once && board.live() > 0 && new.live() == 0 {
                    let _ = notifier.send(
                        Level::Info,
                        "lab-agent: all tasks done",
                        "Nothing is open on the board.",
                    );
                }
                if seen_once
                    && new.inbox_read_upto.is_some()
                    && new.inbox_read_upto != board.inbox_read_upto
                {
                    let _ = notifier.send(Level::Info, "lab-agent: read", "Your message was read.");
                }
                for reply in &new.replies {
                    if sent.insert(reply.id.clone()) {
                        if seen_once {
                            let _ = notifier.send(Level::Info, "Claude", &reply.text);
                        }
                        let _ = std::fs::write(
                            &sent_path,
                            sent.iter().cloned().collect::<Vec<_>>().join("\n"),
                        );
                    }
                }
                if new.live() > board.live() || !seen_once {
                    last_digest = std::time::Instant::now() - Duration::from_secs(24 * 3600);
                    // report a new task soon
                }
                // a local copy, so the board's live view on this machine needs no network
                let local = state_dir.join(board::BOARD_PATH);
                let tmp = local.with_extension("json.tmp");
                if std::fs::write(&tmp, &file.content)
                    .and_then(|()| std::fs::rename(&tmp, &local))
                    .is_err()
                {
                    eprintln!("lab-agent: could not keep a local copy of the board");
                }
                board = new;
                etag = tag;
                seen_once = true;
            }
            Ok(_) => {}
            Err(e) => eprintln!("lab-agent: board: {e:#}"),
        }
        // 2. the digest, while anything is open
        let every = Duration::from_secs(board.every_minutes.max(1) * 60);
        if last_digest.elapsed() >= every {
            if let Some(text) = board::digest(&board, now_unix()) {
                if notifier
                    .send(Level::Info, "lab-agent: what I'm doing", &text)
                    .is_ok()
                {
                    last_digest = std::time::Instant::now();
                }
            }
        }
        // 3. the user's new inbox messages, filed in the results repo for the agent
        match crate::notify::poll_inbox(&ncfg, cursor.as_deref()) {
            Ok(new) if !new.is_empty() => {
                let filed = with_retry(shared, "inbox", || {
                    let current = shared.client.get_file(
                        &cfg.owner,
                        &cfg.results_repo,
                        board::INBOX_PATH,
                        &cfg.results_branch,
                    )?;
                    let (mut inbox, sha) = match current {
                        Some(f) => (board::parse_inbox(&f.content), Some(f.sha)),
                        None => (Inbox::default(), None),
                    };
                    for m in &new {
                        if !inbox.messages.iter().any(|x| x.id == m.id) {
                            inbox.messages.push(board::Message {
                                id: m.id.clone(),
                                time: m.time,
                                text: m.text.clone(),
                            });
                        }
                    }
                    let keep = inbox.messages.len().saturating_sub(100);
                    inbox.messages.drain(..keep);
                    shared.client.put_file(
                        &cfg.owner,
                        &cfg.results_repo,
                        board::INBOX_PATH,
                        &cfg.results_branch,
                        &serde_json::to_vec_pretty(&inbox)?,
                        "inbox: new message",
                        sha.as_deref(),
                    )?;
                    let _ = std::fs::write(&inbox_local, serde_json::to_vec_pretty(&inbox)?);
                    Ok(())
                });
                match filed {
                    Ok(()) => {
                        cursor = new.last().map(|m| m.id.clone());
                        if let Some(c) = &cursor {
                            let _ = std::fs::write(&cursor_path, c);
                        }
                        println!("lab-agent: filed {} inbox message(s)", new.len());
                    }
                    Err(e) => eprintln!("lab-agent: inbox: {e:#}"),
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("lab-agent: {e:#}"),
        }
    }
}

/// How many jobs' local files (status and log) the agent keeps.
const KEEP_LOCAL: usize = 200;
/// How many of the newest results a starting agent reads to find jobs that were cut off.
const RECENT_TO_CHECK: usize = 20;
/// The contents API serves files up to 1 MB; a longer log keeps its tail.
const MAX_PUSHED_LOG: usize = 900_000;

/// The job's output, whatever bytes it printed (a build can print non-UTF-8).
fn read_log(path: &Path) -> String {
    std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

fn tail_for_push(text: &str) -> String {
    if text.len() <= MAX_PUSHED_LOG {
        return text.to_string();
    }
    let mut cut = text.len() - MAX_PUSHED_LOG;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    format!("[... {cut} earlier bytes not shown ...]\n{}", &text[cut..])
}

fn push_log(shared: &Shared, job_name: &str, text: &str) -> Result<()> {
    let cfg = &shared.cfg;
    let path = format!("jobs/{job_name}/log/0000.txt");
    let body = tail_for_push(text);
    with_retry(shared, &format!("{job_name} log"), || {
        let existing =
            shared
                .client
                .get_file(&cfg.owner, &cfg.results_repo, &path, &cfg.results_branch)?;
        shared.client.put_file(
            &cfg.owner,
            &cfg.results_repo,
            &path,
            &cfg.results_branch,
            body.as_bytes(),
            &format!("log: {job_name}"),
            existing.map(|f| f.sha).as_deref(),
        )?;
        Ok(())
    })
}

/// A job's outcome, kept on disk until GitHub has it.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct Done {
    status: String,
    exit_code: Option<i32>,
    started_at: f64,
    finished_at: f64,
    timeout_secs: u64,
}

fn done_path(state_dir: &Path, job: &str) -> PathBuf {
    state_dir.join(format!("{job}.done"))
}

/// Writes the marker atomically (a half-written one would be worse than none).
fn write_done(state_dir: &Path, job: &str, done: &Done) -> Result<()> {
    let path = done_path(state_dir, job);
    let tmp = path.with_extension("done.tmp");
    std::fs::write(&tmp, serde_json::to_vec(done)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn read_done(path: &Path) -> Option<Done> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn push_done(shared: &Shared, job_name: &str, done: &Done, log: &str) -> Result<()> {
    // The log first: a result whose status says "finished" must already have its final log.
    push_log(shared, job_name, log).context("the log")?;
    push_status(
        shared,
        job_name,
        &done.status,
        done.exit_code,
        done.started_at,
        Some(done.finished_at),
        done.timeout_secs,
    )
}

/// Pushes every outcome that is still only on disk: a job finished while GitHub was unreachable or
/// the budget was spent. Run before every poll, so a result is never stuck as "running".
fn flush_unsent(shared: &Shared, state_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(state_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(job) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".done"))
        else {
            continue;
        };
        if lock(&shared.in_flight).contains(job) {
            continue;
        }
        let Some(done) = read_done(&path) else {
            eprintln!("lab-agent: {job}: unreadable outcome marker, leaving it");
            continue;
        };
        let log = read_log(&state_dir.join(format!("{job}.log")));
        match push_done(shared, job, &done, &log) {
            Ok(()) => {
                let _ = std::fs::remove_file(&path);
                println!("lab-agent: {job}: pushed the result that could not be pushed earlier");
            }
            Err(e) => {
                eprintln!("lab-agent: {job}: still cannot push its result: {e:#}");
                break; // the same cause will stop the rest too; try again next poll
            }
        }
    }
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

    #[test]
    fn an_outcome_survives_on_disk_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("lab-agent-done-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let done = Done {
            status: "ok".into(),
            exit_code: Some(0),
            started_at: 10.5,
            finished_at: 20.25,
            timeout_secs: 300,
        };
        write_done(&dir, "1-job", &done).unwrap();
        assert_eq!(read_done(&done_path(&dir, "1-job")), Some(done));
        assert!(
            !dir.join("1-job.done.tmp").exists(),
            "the temporary file must be renamed away"
        );
    }

    #[test]
    fn a_torn_marker_reads_as_nothing_not_as_a_result() {
        let dir = std::env::temp_dir().join(format!("lab-agent-torn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(done_path(&dir, "2-job"), b"{\"status\":\"o").unwrap();
        assert_eq!(read_done(&done_path(&dir, "2-job")), None);
    }

    #[test]
    fn a_local_record_reads_back_as_a_job_row() {
        let dir = std::env::temp_dir().join(format!("lab-agent-local-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        record_local(&dir, "100-a", "ok", Some(0), 1.0, Some(3.0), 60);
        let rows = crate::jobs::local_rows(&dir, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (
                rows[0].name.as_str(),
                rows[0].status.as_str(),
                rows[0].timeout_secs
            ),
            ("100-a", "ok", Some(60))
        );
        assert!(
            !dir.join("100-a.status.json.tmp").exists() && !dir.join("100-a.json.tmp").exists()
        );
    }

    #[test]
    fn pruning_keeps_the_newest_and_never_an_unsent_result() {
        let dir = std::env::temp_dir().join(format!("lab-agent-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["1-a", "2-b", "3-c"] {
            record_local(&dir, n, "ok", Some(0), 1.0, Some(2.0), 60);
            std::fs::write(dir.join(format!("{n}.log")), b"x").unwrap();
        }
        std::fs::write(done_path(&dir, "1-a"), b"{}").unwrap(); // 1-a is still waiting to be pushed
        prune_state(&dir, 1);
        assert!(dir.join("3-c.status.json").exists(), "the newest stays");
        assert!(
            !dir.join("2-b.status.json").exists() && !dir.join("2-b.log").exists(),
            "an old one goes"
        );
        assert!(
            dir.join("1-a.status.json").exists(),
            "an unsent result is never pruned"
        );
    }

    #[test]
    fn output_is_pushed_less_often_the_longer_a_job_runs() {
        assert_eq!(
            partial_log_every(Duration::from_secs(10)),
            Duration::from_secs(8)
        );
        assert_eq!(
            partial_log_every(Duration::from_secs(300)),
            Duration::from_secs(30)
        );
        assert_eq!(
            partial_log_every(Duration::from_secs(3600)),
            Duration::from_secs(60)
        );
    }
}
