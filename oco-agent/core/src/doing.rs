//! Doing the work: the agent's hands, and the step that keeps them honest. Deciding says what to do;
//! this carries it out through `Hands` - whatever actually runs a job - and writes every start, end,
//! timeout and parking into the journal as it happens, so the journal is always the truth about what
//! ran. A job is claimed in the journal before it starts: if the claim is refused, it does not start.

use crate::deciding::{next, Act};
use crate::journal::{Entry, Happened, Journal, Moment, Outcome, Trouble};
use crate::leases::{Beats, Leases};
use std::collections::BTreeMap;

/// Whatever actually runs a job: processes on this machine today, something else later.
pub trait Hands {
    /// What the hands hold on to while a job runs.
    type Holding;
    /// Starts `job`; says why not if it cannot.
    fn start(&mut self, job: &str) -> Result<Self::Holding, String>;
    /// How the job ended, if it has.
    fn look(&mut self, holding: &mut Self::Holding) -> Option<Outcome>;
    /// Stops the job, and everything it started.
    fn stop(&mut self, holding: Self::Holding);
    /// Makes sure nothing of `job` still runs - for a job an earlier run of the agent left behind.
    fn clear(&mut self, job: &str);
}

/// One run of the agent, doing work.
pub struct Doing<H: Hands> {
    pub hands: H,
    /// Which run of the agent this is: any number no earlier run used.
    pub run: u64,
    pub at_once: usize,
    /// How long a beat keeps a driver's lease alive, in seconds.
    pub lasts: u64,
    running: BTreeMap<String, (H::Holding, Moment)>,
    /// Ends not yet written, because the journal could not be reached; written at the next step.
    ending: Vec<(String, Outcome, Moment)>,
}

/// How long `job` may run, as its driver asked.
fn may_run(entries: &[Entry], job: &str) -> u64 {
    entries
        .iter()
        .find_map(|e| match &e.what {
            Happened::Submitted { may_run, .. } if e.job == job => Some(*may_run),
            _ => None,
        })
        .unwrap_or(0)
}

fn ended(job: &str, outcome: Outcome) -> String {
    match outcome {
        Outcome::Done => format!("{job} finished"),
        Outcome::Failed => format!("{job} failed"),
        Outcome::TimedOut => format!("{job} ran out of time and was stopped"),
    }
}

impl<H: Hands> Doing<H> {
    pub fn new(hands: H, run: u64, at_once: usize, lasts: u64) -> Self {
        Doing {
            hands,
            run,
            at_once,
            lasts,
            running: BTreeMap::new(),
            ending: Vec::new(),
        }
    }

    /// The jobs this run is doing now.
    pub fn running(&self) -> Vec<&str> {
        self.running.keys().map(String::as_str).collect()
    }

    /// One step: what ended or overstayed is written down, then what deciding says is done.
    /// Returns what happened, in words.
    pub fn step(
        &mut self,
        journal: &mut impl Journal,
        beats: &impl Beats,
        now: Moment,
    ) -> Result<Vec<String>, Trouble> {
        let mut said = Vec::new();
        let jobs: Vec<String> = self.running.keys().cloned().collect();
        for job in jobs {
            let Some((mut holding, until)) = self.running.remove(&job) else {
                continue;
            };
            let outcome = match self.hands.look(&mut holding) {
                Some(outcome) => outcome,
                None if now > until => {
                    self.hands.stop(holding);
                    Outcome::TimedOut
                }
                None => {
                    self.running.insert(job, (holding, until));
                    continue;
                }
            };
            self.ending.push((job, outcome, now));
        }
        self.write_ends(journal, &mut said)?;

        let entries = journal.entries()?;
        let drivers = Leases {
            heard: beats.heard()?,
            lasts: self.lasts,
        };
        for act in next(&entries, &drivers, now, self.run, self.at_once) {
            match act {
                Act::Start(job) => {
                    // claimed before it starts: if another took it, or the journal cannot be reached, it does not start
                    if journal
                        .record(Entry {
                            job: job.clone(),
                            at: now,
                            what: Happened::Claimed { run: self.run },
                        })
                        .is_err()
                    {
                        continue;
                    }
                    match self.hands.start(&job) {
                        Ok(holding) => {
                            said.push(format!("{job} started"));
                            let until = now + may_run(&entries, &job);
                            self.running.insert(job, (holding, until));
                        }
                        Err(why) => {
                            said.push(format!("{job} could not start: {why}"));
                            self.ending.push((job, Outcome::Failed, now));
                        }
                    }
                }
                Act::Expire(job) => {
                    journal.record(Entry {
                        job: job.clone(),
                        at: now,
                        what: Happened::Expired,
                    })?;
                    said.push(format!("{job} expired: nobody started it in time"));
                }
                Act::Park(job, why) => {
                    self.hands.clear(&job);
                    journal.record(Entry {
                        job: job.clone(),
                        at: now,
                        what: Happened::Parked(why),
                    })?;
                    said.push(format!(
                        "{job} parked: an earlier run of the agent left it running"
                    ));
                }
            }
        }
        self.write_ends(journal, &mut said)?;
        Ok(said)
    }

    fn write_ends(
        &mut self,
        journal: &mut impl Journal,
        said: &mut Vec<String>,
    ) -> Result<(), Trouble> {
        while let Some((job, outcome, at)) = self.ending.first().cloned() {
            journal.record(Entry {
                job: job.clone(),
                at,
                what: Happened::Finished(outcome),
            })?;
            said.push(ended(&job, outcome));
            self.ending.remove(0);
        }
        Ok(())
    }
}
