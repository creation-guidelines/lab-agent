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

fn after(what: &Happened) -> State {
    match what {
        Happened::Submitted { start_by, .. } | Happened::Released { start_by } => State::Waiting {
            start_by: *start_by,
        },
        Happened::Claimed { run } => State::Running { run: *run },
        Happened::Finished(outcome) => State::Finished(*outcome),
        Happened::Parked(why) => State::Parked(*why),
        Happened::Expired => State::Expired,
    }
}

/// Where `job` stands; nothing, if the journal has never heard of it.
pub fn state(entries: &[Entry], job: &str) -> Option<State> {
    entries
        .iter()
        .rev()
        .find(|e| e.job == job)
        .map(|e| after(&e.what))
}

/// Where one job stands, and which driver asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing<'a> {
    pub job: &'a str,
    pub by: &'a str,
    pub stands: State,
}

/// Where every job stands, in the order they were first submitted - the journal read once.
pub fn standing(entries: &[Entry]) -> Vec<Standing<'_>> {
    let mut order: Vec<Standing> = Vec::new();
    let mut place: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for e in entries {
        match place.get(e.job.as_str()) {
            Some(&i) => order[i].stands = after(&e.what),
            None => {
                let by = match &e.what {
                    Happened::Submitted { by, .. } => by.as_str(),
                    _ => "",
                };
                place.insert(&e.job, order.len());
                order.push(Standing {
                    job: &e.job,
                    by,
                    stands: after(&e.what),
                });
            }
        }
    }
    order
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

/// Why the journal would not take an entry, or could not answer at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The entry may not follow what is already written for its job.
    Refused { job: String, because: String },
    /// Wherever the journal is kept could not be read or written.
    Unreachable { because: String },
}

/// Says no to an entry that may not follow what is written - the one rule every journal applies
/// before it writes, wherever it is kept.
pub fn refuse_unless_it_may_follow(entries: &[Entry], entry: &Entry) -> Result<(), Trouble> {
    let stands = state(entries, &entry.job);
    if may_follow(stands.as_ref(), &entry.what) {
        Ok(())
    } else {
        Err(Trouble::Refused {
            job: entry.job.clone(),
            because: format!("{:?} cannot follow {:?}", entry.what, stands),
        })
    }
}

/// Where the journal is kept. Whatever keeps it writes an entry only if it may follow what is already
/// there, one writer at a time - that is what lets two agents reach for the same job and only one take it.
pub trait Journal {
    fn entries(&self) -> Result<Vec<Entry>, Trouble>;
    fn record(&mut self, entry: Entry) -> Result<(), Trouble>;
}

/// A journal kept in memory: for trying the rules, and the plainest thing that passes the measure.
#[derive(Default)]
pub struct InMemory(Vec<Entry>);

impl Journal for InMemory {
    fn entries(&self) -> Result<Vec<Entry>, Trouble> {
        Ok(self.0.clone())
    }

    fn record(&mut self, entry: Entry) -> Result<(), Trouble> {
        refuse_unless_it_may_follow(&self.0, &entry)?;
        self.0.push(entry);
        Ok(())
    }
}
