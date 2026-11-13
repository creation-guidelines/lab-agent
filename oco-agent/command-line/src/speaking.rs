//! Speaking: what happened goes back to the repository work was asked through - the journal and each
//! job's log, on its `spoken` branch, with the mark that says what this record is - written only when
//! something changed: the journal at most every
//! ten seconds, a running job's log at most every minute, a finished job's log at once. Logs go before
//! the journal, so a driver who reads that a job ended finds its log there too. Only jobs this run has
//! done are spoken, so a restarted runner does not pour out old logs.

use oco_agent_core::journal::Trouble;
use oco_agent_github::GitHub;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The most of a log that is spoken: its end.
const LOG_TAIL: usize = 512 * 1024;

#[derive(Default)]
pub struct Speaking {
    /// What the record says it is (its mark), as last spoken.
    mark: Option<String>,
    journal: Option<String>,
    journal_at: u64,
    /// Jobs this run has seen running, until their last words are spoken.
    doing: HashSet<String>,
    /// Each log's spoken length, and when.
    logs: HashMap<String, (u64, u64)>,
}

impl Speaking {
    pub fn speak(
        &mut self,
        folder: &Path,
        remote: &GitHub,
        running: &[&str],
        now: u64,
    ) -> Result<(), Trouble> {
        let unreachable = |e: std::io::Error| Trouble::Unreachable {
            because: e.to_string(),
        };
        self.doing.extend(running.iter().map(|j| j.to_string()));
        let jobs: Vec<String> = self.doing.iter().cloned().collect();
        for job in jobs {
            let file = folder.join("logs").join(format!("{job}.log"));
            let still = running.contains(&job.as_str());
            let length = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            let (spoken, at) = self.logs.get(&job).copied().unwrap_or((u64::MAX, 0));
            if length != spoken && (!still || now.saturating_sub(at) >= 60) {
                let bytes = std::fs::read(&file).unwrap_or_default();
                remote.speak(
                    &format!("oco-agent/logs/{job}.log"),
                    &bytes[bytes.len().saturating_sub(LOG_TAIL)..],
                    &format!("log: {job}"),
                )?;
                self.logs.insert(job.clone(), (length, now));
            }
            if !still {
                self.doing.remove(&job);
            }
        }
        // what the record is travels with it: its mark, spoken whenever it is not what was last spoken -
        // so that a copy brought elsewhere is this place, and is not named for wherever it arrives
        if let Ok(mark) = std::fs::read_to_string(folder.join("oco.toml")) {
            if self.mark.as_deref() != Some(mark.as_str()) {
                remote.speak("oco-agent/oco.toml", mark.as_bytes(), "what this record is")?;
                self.mark = Some(mark);
            }
        }
        let text = match std::fs::read_to_string(folder.join("data/journal.json")) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(unreachable(e)),
        };
        if self.journal.as_deref() != Some(text.as_str())
            && now.saturating_sub(self.journal_at) >= 10
        {
            remote.speak("oco-agent/journal.json", text.as_bytes(), "the journal")?;
            self.journal = Some(text);
            self.journal_at = now;
        }
        Ok(())
    }
}
