//! What the agent does next. Given the journal, the drivers' leases, the time and which run of the
//! agent this is, it names the acts to take and takes none itself.

use crate::journal::{standing, Entry, Moment, State, Why};
use crate::leases::Leases;

/// One thing the agent should now do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Start(String),
    Expire(String),
    Park(String, Why),
}

/// What the agent would do now, doing at most `at_once` jobs side by side.
pub fn next(
    entries: &[Entry],
    drivers: &Leases,
    now: Moment,
    run: u64,
    at_once: usize,
) -> Vec<Act> {
    let mut acts = Vec::new();
    let mut running = 0;
    let mut waiting = Vec::new();
    for s in standing(entries) {
        match s.stands {
            // a job an earlier run was doing when the agent stopped: set aside, never started again unasked
            State::Running { run: theirs } if theirs != run => {
                acts.push(Act::Park(s.job.to_string(), Why::TheAgentRestarted))
            }
            State::Running { .. } => running += 1,
            State::Waiting { start_by } if start_by < now => {
                acts.push(Act::Expire(s.job.to_string()))
            }
            State::Waiting { .. } => waiting.push(s),
            _ => {}
        }
    }
    // a job starts only while the driver who asked for it is still there; what is running reaches its end
    for s in waiting
        .into_iter()
        .filter(|s| drivers.live(s.by, now))
        .take(at_once.saturating_sub(running))
    {
        acts.push(Act::Start(s.job.to_string()));
    }
    acts
}
