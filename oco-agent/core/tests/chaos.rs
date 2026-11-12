//! Every way it was broken on 2026-11-11, and a few more, tried on purpose. Each test names what was
//! done to the agent and says what must happen; none needs a machine, a network or a clock.

use oco_agent_core::deciding::{next, Act};
use oco_agent_core::journal::{
    state, Entry, Happened, InMemory, Journal, Moment, Outcome, State, Why,
};
use oco_agent_core::leases::Lease;

const MINUTE: Moment = 60;
const HOUR: Moment = 60 * MINUTE;

fn submit(journal: &mut InMemory, job: &str, at: Moment) {
    journal
        .record(Entry {
            job: job.into(),
            at,
            what: Happened::Submitted {
                by: "a driver".into(),
                start_by: at + HOUR,
            },
        })
        .unwrap();
}

fn finish(journal: &mut InMemory, job: &str, at: Moment) {
    journal
        .record(Entry {
            job: job.into(),
            at,
            what: Happened::Finished(Outcome::Done),
        })
        .unwrap();
}

/// The driver beat at `at`, and a beat lasts ten minutes.
fn heard(at: Moment) -> Lease {
    Lease {
        last_beat: Some(at),
        lasts: 10 * MINUTE,
    }
}

/// Does what `next` says, as the agent would; returns the jobs it started.
fn carry_out(
    journal: &mut InMemory,
    driver: &Lease,
    now: Moment,
    run: u64,
    at_once: usize,
) -> Vec<String> {
    let mut started = Vec::new();
    for act in next(&journal.entries(), driver, now, run, at_once) {
        match act {
            Act::Start(job) => {
                if journal
                    .record(Entry {
                        job: job.clone(),
                        at: now,
                        what: Happened::Claimed { run },
                    })
                    .is_ok()
                {
                    started.push(job);
                }
            }
            Act::Expire(job) => journal
                .record(Entry {
                    job,
                    at: now,
                    what: Happened::Expired,
                })
                .unwrap(),
            Act::Park(job, why) => journal
                .record(Entry {
                    job,
                    at: now,
                    what: Happened::Parked(why),
                })
                .unwrap(),
        }
    }
    started
}

#[test]
fn a_restart_never_replays_finished_jobs() {
    // 2026-11-11, 23:16: after a reboot the agent ran the afternoon's finished jobs again.
    let mut journal = InMemory::default();
    for (i, job) in ["one", "two", "three"].into_iter().enumerate() {
        submit(&mut journal, job, i as Moment);
        assert_eq!(carry_out(&mut journal, &heard(10), 10, 1, 1), vec![job]);
        finish(&mut journal, job, 20);
    }
    assert!(
        carry_out(&mut journal, &heard(100), 100, 2, 1).is_empty(),
        "a new run of the agent starts nothing that finished"
    );
}

#[test]
fn a_thousand_finished_jobs_and_fifty_at_once_still_start_nothing() {
    // 2026-11-11, 23:34: the agent was reconnected with fifty jobs at once and replayed them side by side.
    let mut journal = InMemory::default();
    for i in 0..1000u64 {
        let job = format!("job-{i}");
        submit(&mut journal, &job, i);
        carry_out(&mut journal, &heard(i), i, 1, 1);
        finish(&mut journal, &job, i);
    }
    assert!(carry_out(&mut journal, &heard(2000), 2000, 2, 50).is_empty());
}

#[test]
fn a_job_cut_off_by_a_restart_is_parked_not_run_again() {
    let mut journal = InMemory::default();
    submit(&mut journal, "half done", 0);
    assert_eq!(
        carry_out(&mut journal, &heard(1), 1, 1, 1),
        vec!["half done"]
    );
    // the agent dies here and comes back as run 2
    assert!(
        carry_out(&mut journal, &heard(5), 5, 2, 1).is_empty(),
        "it does not start the job again"
    );
    assert_eq!(
        state(&journal.entries(), "half done"),
        Some(State::Parked(Why::TheAgentRestarted))
    );
    assert!(
        carry_out(&mut journal, &heard(6), 6, 2, 1).is_empty(),
        "and it stays parked"
    );
    // only a driver sends it back
    journal
        .record(Entry {
            job: "half done".into(),
            at: 7,
            what: Happened::Released { start_by: 7 + HOUR },
        })
        .unwrap();
    assert_eq!(
        carry_out(&mut journal, &heard(8), 8, 2, 1),
        vec!["half done"]
    );
}

#[test]
fn a_silent_driver_starts_nothing_and_a_returning_one_carries_on() {
    // the author, 2026-11-12: with no beat from its driver, the agent stops and postpones.
    let mut journal = InMemory::default();
    submit(&mut journal, "waiting", 0);
    let silent_since = heard(0);
    assert!(
        carry_out(&mut journal, &silent_since, 11 * MINUTE, 1, 1).is_empty(),
        "eleven minutes after the last beat nothing starts"
    );
    assert!(
        carry_out(
            &mut journal,
            &Lease {
                last_beat: None,
                lasts: 10 * MINUTE
            },
            1,
            1,
            1
        )
        .is_empty(),
        "nor with a driver never heard from"
    );
    assert_eq!(
        carry_out(&mut journal, &heard(12 * MINUTE), 12 * MINUTE, 1, 1),
        vec!["waiting"],
        "the driver is back: the work carries on"
    );
}

#[test]
fn work_nobody_started_in_time_expires_instead_of_waiting_forever() {
    let mut journal = InMemory::default();
    submit(&mut journal, "stale", 0);
    assert!(
        carry_out(&mut journal, &heard(2 * HOUR), 2 * HOUR, 1, 1).is_empty(),
        "two hours on, it is not started"
    );
    assert_eq!(state(&journal.entries(), "stale"), Some(State::Expired));
    assert!(
        carry_out(&mut journal, &heard(3 * HOUR), 3 * HOUR, 1, 1).is_empty(),
        "and never will be"
    );
}

#[test]
fn two_agents_reaching_for_one_job_cannot_both_take_it() {
    let mut journal = InMemory::default();
    submit(&mut journal, "wanted", 0);
    assert!(journal
        .record(Entry {
            job: "wanted".into(),
            at: 1,
            what: Happened::Claimed { run: 1 }
        })
        .is_ok());
    assert!(
        journal
            .record(Entry {
                job: "wanted".into(),
                at: 1,
                what: Happened::Claimed { run: 2 }
            })
            .is_err(),
        "the journal takes one claim"
    );
}

#[test]
fn what_is_finished_expired_or_unknown_cannot_be_claimed() {
    let mut journal = InMemory::default();
    assert!(journal
        .record(Entry {
            job: "never submitted".into(),
            at: 0,
            what: Happened::Claimed { run: 1 }
        })
        .is_err());
    submit(&mut journal, "done", 0);
    carry_out(&mut journal, &heard(0), 0, 1, 1);
    finish(&mut journal, "done", 1);
    assert!(
        journal
            .record(Entry {
                job: "done".into(),
                at: 2,
                what: Happened::Claimed { run: 1 }
            })
            .is_err(),
        "run once, by construction"
    );
    assert!(
        journal
            .record(Entry {
                job: "done".into(),
                at: 2,
                what: Happened::Finished(Outcome::Failed)
            })
            .is_err(),
        "and it ends once"
    );
}

#[test]
fn no_more_run_side_by_side_than_allowed() {
    let mut journal = InMemory::default();
    for job in ["a", "b", "c"] {
        submit(&mut journal, job, 0);
    }
    assert_eq!(
        carry_out(&mut journal, &heard(1), 1, 1, 2),
        vec!["a", "b"],
        "two at once, in the order they were asked for"
    );
    assert!(
        carry_out(&mut journal, &heard(2), 2, 1, 2).is_empty(),
        "the third waits"
    );
    finish(&mut journal, "a", 3);
    assert_eq!(carry_out(&mut journal, &heard(4), 4, 1, 2), vec!["c"]);
}
