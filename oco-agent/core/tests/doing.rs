//! The runner's step, tried with stand-in hands: no process, no machine, no clock.

use oco_agent_core::doing::{Doing, Hands};
use oco_agent_core::journal::{state, Entry, Happened, InMemory, Journal, Outcome, State, Why};
use oco_agent_core::leases::{self, Beats};
use std::collections::BTreeMap;

/// Hands that pretend: each job ends after so many looks, or never; some cannot start.
#[derive(Default)]
struct StandIn {
    ends_after: BTreeMap<String, u32>,
    cannot: Vec<String>,
    started: Vec<String>,
    stopped: Vec<String>,
    cleared: Vec<String>,
}

impl Hands for StandIn {
    type Holding = (String, u32);

    fn start(&mut self, job: &str) -> Result<Self::Holding, String> {
        if self.cannot.iter().any(|j| j == job) {
            return Err("there is no script".into());
        }
        self.started.push(job.into());
        Ok((job.into(), 0))
    }

    fn look(&mut self, holding: &mut Self::Holding) -> Option<Outcome> {
        holding.1 += 1;
        match self.ends_after.get(&holding.0) {
            Some(n) if holding.1 >= *n => Some(Outcome::Done),
            _ => None,
        }
    }

    fn stop(&mut self, holding: Self::Holding) {
        self.stopped.push(holding.0);
    }

    fn clear(&mut self, job: &str) {
        self.cleared.push(job.into());
    }
}

fn ask(journal: &mut InMemory, job: &str, may_run: u64) {
    journal
        .record(Entry {
            job: job.into(),
            at: 0,
            what: Happened::Submitted {
                by: "ann".into(),
                start_by: 1000,
                may_run,
            },
        })
        .unwrap();
}

fn ann_here() -> leases::InMemory {
    let mut beats = leases::InMemory::default();
    beats.beat("ann", 0).unwrap();
    beats
}

fn stands(journal: &InMemory, job: &str) -> Option<State> {
    state(&journal.entries().unwrap(), job)
}

#[test]
fn a_job_runs_and_its_end_is_written() {
    let (mut journal, beats) = (InMemory::default(), ann_here());
    ask(&mut journal, "a", 100);
    let mut doing = Doing::new(
        StandIn {
            ends_after: [("a".to_string(), 1)].into(),
            ..Default::default()
        },
        1,
        1,
        600,
    );
    assert_eq!(
        doing.step(&mut journal, &beats, 1).unwrap(),
        vec!["a started"]
    );
    assert_eq!(
        stands(&journal, "a"),
        Some(State::Running { run: 1 }),
        "claimed before it started"
    );
    assert_eq!(
        doing.step(&mut journal, &beats, 2).unwrap(),
        vec!["a finished"]
    );
    assert_eq!(stands(&journal, "a"), Some(State::Finished(Outcome::Done)));
    assert!(doing.running().is_empty());
}

#[test]
fn a_job_that_overstays_is_stopped_and_written_as_timed_out() {
    let (mut journal, beats) = (InMemory::default(), ann_here());
    ask(&mut journal, "slow", 5);
    let mut doing = Doing::new(StandIn::default(), 1, 1, 600);
    doing.step(&mut journal, &beats, 1).unwrap();
    assert!(
        doing.step(&mut journal, &beats, 3).unwrap().is_empty(),
        "within its time it runs on"
    );
    assert_eq!(
        doing.step(&mut journal, &beats, 7).unwrap(),
        vec!["slow ran out of time and was stopped"]
    );
    assert_eq!(doing.hands.stopped, vec!["slow"]);
    assert_eq!(
        stands(&journal, "slow"),
        Some(State::Finished(Outcome::TimedOut))
    );
}

#[test]
fn a_job_that_cannot_start_fails_once_and_is_not_tried_again() {
    let (mut journal, beats) = (InMemory::default(), ann_here());
    ask(&mut journal, "broken", 100);
    let mut doing = Doing::new(
        StandIn {
            cannot: vec!["broken".into()],
            ..Default::default()
        },
        1,
        1,
        600,
    );
    assert_eq!(
        doing.step(&mut journal, &beats, 1).unwrap(),
        vec![
            "broken could not start: there is no script",
            "broken failed"
        ]
    );
    assert_eq!(
        stands(&journal, "broken"),
        Some(State::Finished(Outcome::Failed))
    );
    assert!(doing.step(&mut journal, &beats, 2).unwrap().is_empty());
}

#[test]
fn a_new_run_parks_and_clears_what_a_dead_one_left_running() {
    let (mut journal, beats) = (InMemory::default(), ann_here());
    ask(&mut journal, "left behind", 100);
    let mut first = Doing::new(StandIn::default(), 1, 1, 600);
    first.step(&mut journal, &beats, 1).unwrap();
    drop(first); // the agent dies here
    let mut second = Doing::new(StandIn::default(), 2, 1, 600);
    assert_eq!(
        second.step(&mut journal, &beats, 5).unwrap(),
        vec!["left behind parked: an earlier run of the agent left it running"]
    );
    assert_eq!(
        second.hands.cleared,
        vec!["left behind"],
        "what it left is cleared away"
    );
    assert!(
        second.hands.started.is_empty(),
        "and it is not started again"
    );
    assert_eq!(
        stands(&journal, "left behind"),
        Some(State::Parked(Why::TheAgentRestarted))
    );
}

#[test]
fn no_more_run_side_by_side_than_allowed_and_the_next_starts_when_one_ends() {
    let (mut journal, beats) = (InMemory::default(), ann_here());
    for job in ["a", "b", "c"] {
        ask(&mut journal, job, 100);
    }
    let mut doing = Doing::new(
        StandIn {
            ends_after: [("a".to_string(), 1)].into(),
            ..Default::default()
        },
        1,
        2,
        600,
    );
    assert_eq!(
        doing.step(&mut journal, &beats, 1).unwrap(),
        vec!["a started", "b started"]
    );
    assert_eq!(
        doing.step(&mut journal, &beats, 2).unwrap(),
        vec!["a finished", "c started"]
    );
}

#[test]
fn nothing_starts_for_a_driver_never_heard_from() {
    let mut journal = InMemory::default();
    ask(&mut journal, "a", 100);
    let mut doing = Doing::new(StandIn::default(), 1, 1, 600);
    assert!(doing
        .step(&mut journal, &leases::InMemory::default(), 1)
        .unwrap()
        .is_empty());
}
