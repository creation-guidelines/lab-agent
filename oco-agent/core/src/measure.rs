//! The checks every journal must pass, wherever it is kept: every way the old agent was broken on
//! 2026-11-11, and a few more, done on purpose. A journal kept in memory passes them; so must one kept
//! in tad tables, or anywhere else - that is what makes replacing where the journal lives safe.
//! Each check is given a fresh, empty journal and panics, saying what failed.

use crate::deciding::{next, Act};
use crate::journal::{state, Entry, Happened, Journal, Moment, Outcome, State, Why};
use crate::leases::{Beats, Leases};

const MINUTE: Moment = 60;
const HOUR: Moment = 60 * MINUTE;

fn submit(journal: &mut impl Journal, job: &str, at: Moment) {
    journal
        .record(Entry {
            job: job.into(),
            at,
            what: Happened::Submitted {
                by: "a driver".into(),
                start_by: at + HOUR,
            },
        })
        .expect("a new job is taken");
}

fn finish(journal: &mut impl Journal, job: &str, at: Moment) {
    journal
        .record(Entry {
            job: job.into(),
            at,
            what: Happened::Finished(Outcome::Done),
        })
        .expect("a running job ends");
}

fn stands(journal: &impl Journal, job: &str) -> Option<State> {
    state(&journal.entries().expect("the journal answers"), job)
}

/// "a driver" - who submits every job in these checks unless said otherwise - beat at `at`; a beat lasts ten minutes.
fn heard(at: Moment) -> Leases {
    Leases {
        heard: [("a driver".to_string(), at)].into(),
        lasts: 10 * MINUTE,
    }
}

/// Does what `next` says, as the agent would; returns the jobs it started.
fn carry_out(
    journal: &mut impl Journal,
    drivers: &Leases,
    now: Moment,
    run: u64,
    at_once: usize,
) -> Vec<String> {
    let mut started = Vec::new();
    for act in next(
        &journal.entries().expect("the journal answers"),
        drivers,
        now,
        run,
        at_once,
    ) {
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
                .expect("a waiting job expires"),
            Act::Park(job, why) => journal
                .record(Entry {
                    job,
                    at: now,
                    what: Happened::Parked(why),
                })
                .expect("a running job is parked"),
        }
    }
    started
}

/// 2026-11-11, 23:16: after a reboot the agent ran the afternoon's finished jobs again.
pub fn a_restart_never_replays_finished_jobs(journal: &mut impl Journal) {
    for (i, job) in ["one", "two", "three"].into_iter().enumerate() {
        submit(journal, job, i as Moment);
        assert_eq!(carry_out(journal, &heard(10), 10, 1, 1), vec![job]);
        finish(journal, job, 20);
    }
    assert!(
        carry_out(journal, &heard(100), 100, 2, 1).is_empty(),
        "a new run of the agent starts nothing that finished"
    );
}

/// 2026-11-11, 23:34: the agent was reconnected with fifty jobs at once and replayed them side by side.
pub fn many_finished_jobs_and_fifty_at_once_still_start_nothing(
    journal: &mut impl Journal,
    many: u64,
) {
    for i in 0..many {
        let job = format!("job-{i}");
        submit(journal, &job, i);
        carry_out(journal, &heard(i), i, 1, 1);
        finish(journal, &job, i);
    }
    assert!(carry_out(journal, &heard(many + HOUR), many + HOUR, 2, 50).is_empty());
}

/// The agent dies in the middle of a job and comes back.
pub fn a_job_cut_off_by_a_restart_is_parked_not_run_again(journal: &mut impl Journal) {
    submit(journal, "half done", 0);
    assert_eq!(carry_out(journal, &heard(1), 1, 1, 1), vec!["half done"]);
    assert!(
        carry_out(journal, &heard(5), 5, 2, 1).is_empty(),
        "the next run does not start the job again"
    );
    assert_eq!(
        stands(journal, "half done"),
        Some(State::Parked(Why::TheAgentRestarted))
    );
    assert!(
        carry_out(journal, &heard(6), 6, 2, 1).is_empty(),
        "and it stays parked"
    );
    journal
        .record(Entry {
            job: "half done".into(),
            at: 7,
            what: Happened::Released { start_by: 7 + HOUR },
        })
        .expect("a driver sends it back");
    assert_eq!(carry_out(journal, &heard(8), 8, 2, 1), vec!["half done"]);
}

/// The author, 2026-11-12: with no beat from its driver, the agent stops and postpones.
pub fn a_silent_driver_starts_nothing_and_a_returning_one_carries_on(journal: &mut impl Journal) {
    submit(journal, "waiting", 0);
    assert!(
        carry_out(journal, &heard(0), 11 * MINUTE, 1, 1).is_empty(),
        "eleven minutes after the last beat nothing starts"
    );
    assert!(
        carry_out(
            journal,
            &Leases {
                heard: Default::default(),
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
        carry_out(journal, &heard(12 * MINUTE), 12 * MINUTE, 1, 1),
        vec!["waiting"],
        "the driver is back: the work carries on"
    );
}

/// A job nobody starts in time.
pub fn work_nobody_started_in_time_expires_instead_of_waiting_forever(journal: &mut impl Journal) {
    submit(journal, "stale", 0);
    assert!(
        carry_out(journal, &heard(2 * HOUR), 2 * HOUR, 1, 1).is_empty(),
        "two hours on, it is not started"
    );
    assert_eq!(stands(journal, "stale"), Some(State::Expired));
    assert!(
        carry_out(journal, &heard(3 * HOUR), 3 * HOUR, 1, 1).is_empty(),
        "and never will be"
    );
}

/// Two agents take the same job at the same moment.
pub fn two_agents_reaching_for_one_job_cannot_both_take_it(journal: &mut impl Journal) {
    submit(journal, "wanted", 0);
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

/// Run once, by construction.
pub fn what_is_finished_expired_or_unknown_cannot_be_claimed(journal: &mut impl Journal) {
    assert!(journal
        .record(Entry {
            job: "never submitted".into(),
            at: 0,
            what: Happened::Claimed { run: 1 }
        })
        .is_err());
    submit(journal, "done", 0);
    carry_out(journal, &heard(0), 0, 1, 1);
    finish(journal, "done", 1);
    assert!(
        journal
            .record(Entry {
                job: "done".into(),
                at: 2,
                what: Happened::Claimed { run: 1 }
            })
            .is_err(),
        "a finished job is not claimed again"
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

/// More work than may run side by side.
pub fn no_more_run_side_by_side_than_allowed(journal: &mut impl Journal) {
    for job in ["a", "b", "c"] {
        submit(journal, job, 0);
    }
    assert_eq!(
        carry_out(journal, &heard(1), 1, 1, 2),
        vec!["a", "b"],
        "two at once, in the order they were asked for"
    );
    assert!(
        carry_out(journal, &heard(2), 2, 1, 2).is_empty(),
        "the third waits"
    );
    finish(journal, "a", 3);
    assert_eq!(carry_out(journal, &heard(4), 4, 1, 2), vec!["c"]);
}

/// What was written is what is read back, in order - nothing lost, nothing reordered.
pub fn it_gives_back_what_it_was_given(journal: &mut impl Journal) {
    let entries = vec![
        Entry {
            job: "a job with a name, in Кириллица".into(),
            at: 1,
            what: Happened::Submitted {
                by: "the author".into(),
                start_by: 9,
            },
        },
        Entry {
            job: "a job with a name, in Кириллица".into(),
            at: 2,
            what: Happened::Claimed { run: 7 },
        },
        Entry {
            job: "a job with a name, in Кириллица".into(),
            at: 3,
            what: Happened::Parked(Why::TheAgentRestarted),
        },
        Entry {
            job: "a job with a name, in Кириллица".into(),
            at: 4,
            what: Happened::Released { start_by: 99 },
        },
        Entry {
            job: "a job with a name, in Кириллица".into(),
            at: 5,
            what: Happened::Claimed { run: 8 },
        },
        Entry {
            job: "a job with a name, in Кириллица".into(),
            at: 6,
            what: Happened::Finished(Outcome::TimedOut),
        },
        Entry {
            job: "another".into(),
            at: 7,
            what: Happened::Submitted {
                by: "a machine".into(),
                start_by: 8,
            },
        },
        Entry {
            job: "another".into(),
            at: 9,
            what: Happened::Expired,
        },
    ];
    for e in &entries {
        journal.record(e.clone()).expect("each may follow the last");
    }
    assert_eq!(journal.entries().expect("the journal answers"), entries);
}

/// Two drivers ask for work; only one of them is still there.
pub fn a_job_waits_for_its_own_driver(journal: &mut impl Journal) {
    for (job, by) in [("ann's", "ann"), ("bo's", "bo")] {
        journal
            .record(Entry {
                job: job.into(),
                at: 0,
                what: Happened::Submitted {
                    by: by.into(),
                    start_by: HOUR,
                },
            })
            .expect("a new job is taken");
    }
    let only_ann = Leases {
        heard: [("ann".to_string(), 5)].into(),
        lasts: 10 * MINUTE,
    };
    assert_eq!(
        carry_out(journal, &only_ann, 5, 1, 5),
        vec!["ann's"],
        "bo is silent: bo's job waits, ann's runs"
    );
    let both = Leases {
        heard: [("ann".to_string(), 6), ("bo".to_string(), 6)].into(),
        lasts: 10 * MINUTE,
    };
    assert_eq!(
        carry_out(journal, &both, 6, 1, 5),
        vec!["bo's"],
        "bo is back: bo's job runs"
    );
}

/// What is known of each driver is the latest beat, and it never goes back.
pub fn beats_are_remembered_and_never_go_back(beats: &mut impl Beats) {
    assert!(
        beats.heard().expect("the beats answer").is_empty(),
        "nobody has been heard yet"
    );
    beats.beat("ann", 10).expect("a beat is taken");
    beats.beat("bo", 7).expect("a beat is taken");
    beats.beat("ann", 4).expect("an old beat is not an error");
    let heard = beats.heard().expect("the beats answer");
    assert_eq!(
        (heard.get("ann"), heard.get("bo")),
        (Some(&10), Some(&7)),
        "an older beat does not replace a newer one"
    );
    beats.beat("ann", 11).expect("a beat is taken");
    assert_eq!(
        beats.heard().expect("the beats answer").get("ann"),
        Some(&11)
    );
}
