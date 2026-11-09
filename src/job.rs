use anyhow::Result;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Runs when its turn comes, after everything queued before it.
    Queue,
    /// Replaces whatever is currently running (killed, process group and
    /// all) and jumps the queue. For "stop what you're doing, do this now".
    Preempt,
}

/// How long a job gets to clean up (a `trap ... TERM`: unmount, remove temporary files, release
/// hardware) between SIGTERM and SIGKILL.
#[cfg(not(test))]
const GRACE: Duration = Duration::from_secs(10);
#[cfg(test)]
const GRACE: Duration = Duration::from_secs(1);

/// Stops a job's whole process group: SIGTERM first, so a script's cleanup runs, then SIGKILL for
/// whatever is still in the group once the grace period is over or the script has exited. The
/// group-wide kill is what cures a hung child (`fastboot reboot`) that outlives its script; the
/// grace period is what stops a mounted image or a half-written file being left behind when a job
/// times out or is preempted.
fn stop(child: &mut Child, pgid: Pid) {
    let _ = killpg(pgid, Signal::SIGTERM);
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline && !matches!(child.try_wait(), Ok(Some(_))) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = killpg(pgid, Signal::SIGKILL);
    let _ = child.wait();
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
            stop(&mut child, pgid);
            return Ok(JobResult {
                exit_code: None,
                timed_out: true,
                preempted: false,
            });
        }
        if should_stop() {
            stop(&mut child, pgid);
            return Ok(JobResult {
                exit_code: None,
                timed_out: false,
                preempted: true,
            });
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lab-agent-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_job_that_times_out_gets_to_clean_up() {
        let dir = scratch("cleanup");
        let marker = dir.join("cleaned");
        // `wait` lets the trap run as soon as TERM arrives, as a script that mounts an image
        // and waits on a long step would.
        let script = format!(
            "trap 'echo cleaned > {}; exit 0' TERM\nsleep 30 &\nwait\n",
            marker.display()
        );
        let r = run(&script, Duration::from_secs(1), &dir.join("log"), || false).unwrap();
        assert!(r.timed_out);
        assert!(marker.exists(), "the TERM trap must run before any SIGKILL");
    }

    #[test]
    fn a_preempted_job_gets_to_clean_up_too() {
        let dir = scratch("preempt");
        let marker = dir.join("cleaned");
        let script = format!(
            "trap 'echo cleaned > {}; exit 0' TERM\nsleep 30 &\nwait\n",
            marker.display()
        );
        let mut polls = 0;
        let r = run(&script, Duration::from_secs(60), &dir.join("log"), || {
            polls += 1;
            polls > 3
        })
        .unwrap();
        assert!(r.preempted);
        assert!(marker.exists());
    }

    #[test]
    fn a_job_that_ignores_term_is_still_killed_after_the_grace_period() {
        let dir = scratch("stubborn");
        let started = Instant::now();
        let r = run(
            "trap '' TERM\nsleep 30 &\nwait\n",
            Duration::from_secs(1),
            &dir.join("log"),
            || false,
        )
        .unwrap();
        assert!(r.timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "SIGKILL must follow the grace period"
        );
    }
}
