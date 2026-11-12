//! What happened to each job, in order. The journal is the only truth about a job: nothing is ever
//! inferred from what is missing, and what may be written next depends only on what is written.

/// A moment, in seconds.
pub type Moment = u64;

/// How a job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Failed,
    TimedOut,
}

/// Why a job was set aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The agent stopped while it was doing the job; nobody knows how far it got.
    TheAgentRestarted,
}

/// One thing that happened to a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Happened {
    /// A driver asked for it; it is started by `start_by` or not at all.
    Submitted {
        by: String,
        start_by: Moment,
    },
    /// A run of the agent took it. A run's number only ever grows, so a later run outranks an earlier one.
    Claimed {
        run: u64,
    },
    Finished(Outcome),
    Parked(Why),
    /// A driver sent a parked job back to wait.
    Released {
        start_by: Moment,
    },
    /// Nobody started it in time.
    Expired,
}

/// One line of the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub job: String,
    pub at: Moment,
    pub what: Happened,
}

/// Where a job stands, read from its entries alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Waiting { start_by: Moment },
    Running { run: u64 },
    Finished(Outcome),
    Parked(Why),
    Expired,
}

/// Where `job` stands; nothing, if the journal has never heard of it.
pub fn state(entries: &[Entry], job: &str) -> Option<State> {
    entries
        .iter()
        .rev()
        .find(|e| e.job == job)
        .map(|e| match &e.what {
            Happened::Submitted { start_by, .. } | Happened::Released { start_by } => {
                State::Waiting {
                    start_by: *start_by,
                }
            }
            Happened::Claimed { run } => State::Running { run: *run },
            Happened::Finished(outcome) => State::Finished(*outcome),
            Happened::Parked(why) => State::Parked(*why),
            Happened::Expired => State::Expired,
        })
}

/// Every job the journal knows, in the order they were first submitted.
pub fn jobs(entries: &[Entry]) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for e in entries {
        if !seen.contains(&e.job.as_str()) {
            seen.push(&e.job);
        }
    }
    seen
}

/// Whether `what` may be written after where a job stands. This is where "run once" lives: a job is
/// claimed only while it waits, so one that finished, expired or was parked can never start unasked.
pub fn may_follow(stands: Option<&State>, what: &Happened) -> bool {
    matches!(
        (stands, what),
        (None, Happened::Submitted { .. })
            | (
                Some(State::Waiting { .. }),
                Happened::Claimed { .. } | Happened::Expired
            )
            | (
                Some(State::Running { .. }),
                Happened::Finished(_) | Happened::Parked(_)
            )
            | (Some(State::Parked(_)), Happened::Released { .. })
    )
}

/// An entry the journal would not take, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub job: String,
    pub because: String,
}

/// Where the journal is kept. Whatever keeps it writes an entry only if it may follow what is already
/// there - that one rule is what lets two agents reach for the same job and only one take it.
pub trait Journal {
    fn entries(&self) -> Vec<Entry>;
    fn record(&mut self, entry: Entry) -> Result<(), Refused>;
}

/// A journal kept in memory: for trying the rules, and the measure every real one must match.
#[derive(Default)]
pub struct InMemory(Vec<Entry>);

impl Journal for InMemory {
    fn entries(&self) -> Vec<Entry> {
        self.0.clone()
    }

    fn record(&mut self, entry: Entry) -> Result<(), Refused> {
        let stands = state(&self.0, &entry.job);
        if !may_follow(stands.as_ref(), &entry.what) {
            return Err(Refused {
                job: entry.job,
                because: format!("{:?} cannot follow {:?}", entry.what, stands),
            });
        }
        self.0.push(entry);
        Ok(())
    }
}
