use anyhow::Result;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Runs when its turn comes, after everything queued before it.
    Queue,
    /// Replaces whatever is currently running (killed, process group and
    /// all) and jumps the queue. For "stop what you're doing, do this now".
    Preempt,
}

pub struct JobResult {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub preempted: bool,
}

/// Runs one job script in its own process group, with a hard-enforced
/// timeout: on expiry, the *whole group* gets SIGKILL, not just the direct
/// child. This is the fix for the one real failure mode this tool had
/// before: a child the script spawned (e.g. `fastboot reboot` outliving the
/// script around it) left running forever, wedging the whole agent because
/// nothing was watching the group, only the immediate PID.
///
/// `should_stop` is polled periodically; when it returns true (a newer
/// preempt-mode job has shown up) the group is killed early and the result
/// is marked `preempted` rather than `timed_out`.
pub fn run(
    script: &str,
    timeout: Duration,
    log_path: &std::path::Path,
    mut should_stop: impl FnMut() -> bool,
) -> Result<JobResult> {
    let log_file = File::create(log_path)?;
    let log_file_err = log_file.try_clone()?;

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err));

    // New session/process group, so the whole tree can be killed as one
    // unit via killpg - the thing a plain Child::kill() cannot do.
    unsafe {
        cmd.pre_exec(|| {
            nix::unistd::setsid().map_err(std::io::Error::other)?;
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;
    let pgid = Pid::from_raw(child.id() as i32);
    let start = Instant::now();

    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(JobResult {
                exit_code: status.code(),
                timed_out: false,
                preempted: false,
            });
        }
        if start.elapsed() >= timeout {
            let _ = killpg(pgid, Signal::SIGKILL);
            let _ = child.wait();
            return Ok(JobResult {
                exit_code: None,
                timed_out: true,
                preempted: false,
            });
        }
        if should_stop() {
            let _ = killpg(pgid, Signal::SIGKILL);
            let _ = child.wait();
            return Ok(JobResult {
                exit_code: None,
                timed_out: false,
                preempted: true,
            });
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}
