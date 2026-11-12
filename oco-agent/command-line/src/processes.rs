//! The hands on this machine: each job runs as a process of its own, in a working folder of its own,
//! its output in a log of its own. Every process a job starts carries the job's name in its environment
//! (OCO_AGENT_JOB), so ending a job - when it finishes, overstays, or was left by a run that died -
//! finds everything it started, its children's children included. A job owns what it starts.

use oco_agent_core::doing::Hands;
use oco_agent_core::journal::Outcome;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The mark every process of a job carries.
const MARK: &str = "OCO_AGENT_JOB";

pub struct Processes {
    pub folder: PathBuf,
}

impl Processes {
    pub fn script(&self, job: &str) -> PathBuf {
        self.folder.join("jobs").join(format!("{job}.sh"))
    }

    pub fn log(&self, job: &str) -> PathBuf {
        self.folder.join("logs").join(format!("{job}.log"))
    }

    fn work(&self, job: &str) -> PathBuf {
        self.folder.join("work").join(job)
    }
}

/// Every process on this machine carrying `job`'s mark.
pub fn marked(job: &str) -> Vec<u32> {
    let mark = format!("{MARK}={job}\0");
    let inner = format!("\0{mark}");
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|e| {
            let pid = e.file_name().to_str()?.parse::<u32>().ok()?;
            let env = std::fs::read(e.path().join("environ")).ok()?;
            let carries = env.starts_with(mark.as_bytes())
                || env.windows(inner.len()).any(|w| w == inner.as_bytes());
            (carries && pid != std::process::id()).then_some(pid)
        })
        .collect()
}

fn signal(pids: &[u32], which: &str) {
    if !pids.is_empty() {
        let _ = Command::new("kill")
            .arg(format!("-{which}"))
            .args(pids.iter().map(u32::to_string))
            .stderr(Stdio::null())
            .status();
    }
}

/// Ends everything carrying `job`'s mark: asked to stop first, then made to.
fn end_all(job: &str) {
    signal(&marked(job), "TERM");
    for _ in 0..50 {
        if marked(job).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    signal(&marked(job), "KILL");
}

impl Hands for Processes {
    type Holding = (String, Child);

    fn start(&mut self, job: &str) -> Result<Self::Holding, String> {
        let script = self.script(job);
        if !script.is_file() {
            return Err(format!("there is no script at {}", script.display()));
        }
        let work = self.work(job);
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(self.folder.join("logs")).map_err(|e| e.to_string())?;
        let out = std::fs::File::create(self.log(job)).map_err(|e| e.to_string())?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let child = Command::new("sh")
            .arg(&script)
            .current_dir(&work)
            .env(MARK, job)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok((job.to_string(), child))
    }

    fn look(&mut self, holding: &mut Self::Holding) -> Option<Outcome> {
        let outcome = match holding.1.try_wait() {
            Ok(Some(status)) if status.success() => Outcome::Done,
            Ok(Some(_)) | Err(_) => Outcome::Failed,
            Ok(None) => return None,
        };
        // what the job started and left running ends with it
        end_all(&holding.0);
        Some(outcome)
    }

    fn stop(&mut self, mut holding: Self::Holding) {
        end_all(&holding.0);
        let _ = holding.1.kill();
        let _ = holding.1.wait();
    }

    fn clear(&mut self, job: &str) {
        end_all(job);
    }
}
