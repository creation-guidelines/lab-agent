//! What the agent does next. Given the journal, the driver's lease, the time and which run of the
//! agent this is, it names the acts to take and takes none itself.

use crate::journal::{standing, Entry, Moment, State, Why};
use crate::leases::Lease;

/// One thing the agent should now do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Start(String),
    Expire(String),
    Park(String, Why),
}

/// What the agent would do now, doing at most `at_once` jobs side by side.
pub fn next(entries: &[Entry], driver: &Lease, now: Moment, run: u64, at_once: usize) -> Vec<Act> {
    let mut acts = Vec::new();
    let mut running = 0;
    let mut waiting = Vec::new();
    for (job, stands) in standing(entries) {
        match Some(stands) {
            // a job an earlier run was doing when the agent stopped: set aside, never started again unasked
            Some(State::Running { run: theirs }) if theirs != run => {
                acts.push(Act::Park(job.to_string(), Why::TheAgentRestarted))
            }
            Some(State::Running { .. }) => running += 1,
            Some(State::Waiting { start_by }) if start_by < now => {
                acts.push(Act::Expire(job.to_string()))
            }
            Some(State::Waiting { .. }) => waiting.push(job),
            _ => {}
        }
    }
    // with no word from the driver nothing new starts; what is running reaches its end
    if driver.live(now) {
        for job in waiting.into_iter().take(at_once.saturating_sub(running)) {
            acts.push(Act::Start(job.to_string()));
        }
    }
    acts
}
