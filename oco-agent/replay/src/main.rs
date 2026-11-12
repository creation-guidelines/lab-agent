//! What oco-agent would have done. Reads an old agent's log on standard input, line by line, and for
//! every job that agent started answers: would oco-agent have started it? Every start it agrees with
//! goes into a journal; every start it would have refused is counted, with the reason.
//!
//!     journalctl --user -u lab-agent -o short-iso | oco-agent-replay [FOLDER]
//!
//! With a FOLDER, the journal is kept there as tad tables and carries on from one reading to the next
//! (the shadow run reads the newest lines every few minutes). It exits with 3 if oco-agent would have
//! refused any start: the old agent did something the new one would not.

use oco_agent_core::deciding::{next, Act};
use oco_agent_core::journal::{state, Entry, Happened, InMemory, Journal, Moment, Outcome, State};
use oco_agent_core::leases::Leases;
use oco_agent_journal_in_tad::InTad;
use std::collections::BTreeMap;
use std::io::BufRead;

const DAY: Moment = 24 * 60 * 60;

/// The moment a log line was written, from a stamp like 2026-11-11T23:16:24+01:00.
fn moment(stamp: &str) -> Option<Moment> {
    let n = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hour, minute, second) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let ahead = match stamp.get(19..20)? {
        "+" => n(20..22)? * 3600 + n(23..25)? * 60,
        "-" => -(n(20..22)? * 3600 + n(23..25)? * 60),
        _ => 0,
    };
    // days since 1970-01-01 (Howard Hinnant's days_from_civil)
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    u64::try_from(days * 86_400 + hour * 3600 + minute * 60 + second - ahead).ok()
}

/// Why oco-agent would not have started a job, read from where it stands.
fn reason(stands: Option<&State>) -> &'static str {
    match stands {
        Some(State::Finished(_)) => "it had already finished",
        Some(State::Parked(_)) => "it was parked when the agent restarted",
        Some(State::Running { .. }) => "it was already running",
        Some(State::Expired) => "nobody had started it in time",
        _ => "the driver was silent or the agent was full",
    }
}

/// What one reading of the log found.
#[derive(Default, Debug)]
pub struct Report {
    pub restarts: u64,
    pub agreed: u64,
    pub ended: u64,
    pub parked: u64,
    pub refused: BTreeMap<&'static str, (u64, Vec<String>)>,
}

impl Report {
    pub fn refused_in_all(&self) -> u64 {
        self.refused.values().map(|r| r.0).sum()
    }
}

/// Reads the lines through oco-agent's rules, writing what it agrees with into `journal`. The run of
/// the agent carries on from the highest one the journal already holds, so readings join up.
pub fn replay(lines: impl Iterator<Item = String>, journal: &mut dyn Journal) -> Report {
    let mut report = Report::default();
    let mut run = journal
        .entries()
        .expect("the journal answers")
        .iter()
        .filter_map(|e| match e.what {
            Happened::Claimed { run } => Some(run),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    for line in lines {
        let Some(now) = line.split_whitespace().next().and_then(moment) else {
            continue;
        };
        // the driver is taken to be present throughout: this replay asks only about running once
        let drivers = Leases {
            heard: [("the old log".to_string(), now)].into(),
            lasts: DAY,
        };
        if line.contains("Started lab-agent") {
            run += 1;
            report.restarts += 1;
            for act in next(
                &journal.entries().expect("the journal answers"),
                &drivers,
                now,
                run,
                usize::MAX,
            ) {
                if let Act::Park(job, why) = act {
                    journal
                        .record(Entry {
                            job,
                            at: now,
                            what: Happened::Parked(why),
                        })
                        .expect("a running job is parked");
                    report.parked += 1;
                }
            }
        } else if let Some(rest) = line.split("lab-agent: starting ").nth(1) {
            // a reading that begins in the middle of a run of the agent calls that run 1, so that the
            // next restart it sees counts as a new run and parks what was left running
            if run == 0 {
                run = 1;
            }
            let job = rest.split_whitespace().next().unwrap_or("").to_string();
            let entries = journal.entries().expect("the journal answers");
            if state(&entries, &job).is_none() {
                journal
                    .record(Entry {
                        job: job.clone(),
                        at: now,
                        what: Happened::Submitted {
                            by: "the old log".into(),
                            start_by: now + DAY,
                        },
                    })
                    .expect("a new job is taken");
            }
            let entries = journal.entries().expect("the journal answers");
            if next(&entries, &drivers, now, run, usize::MAX).contains(&Act::Start(job.clone())) {
                journal
                    .record(Entry {
                        job,
                        at: now,
                        what: Happened::Claimed { run },
                    })
                    .expect("a waiting job is claimed");
                report.agreed += 1;
            } else {
                let slot = report
                    .refused
                    .entry(reason(state(&entries, &job).as_ref()))
                    .or_default();
                slot.0 += 1;
                if slot.1.len() < 3 {
                    slot.1.push(format!(
                        "{} {job}",
                        line.split_whitespace().next().unwrap_or("")
                    ));
                }
            }
        } else if let Some((before, how)) = line.split_once(" finished: ") {
            let job = before.rsplit(' ').next().unwrap_or("");
            if matches!(
                state(&journal.entries().expect("the journal answers"), job),
                Some(State::Running { .. })
            ) {
                let outcome = match how.trim() {
                    "ok" => Outcome::Done,
                    how if how.starts_with("timed") => Outcome::TimedOut,
                    _ => Outcome::Failed,
                };
                journal
                    .record(Entry {
                        job: job.to_string(),
                        at: now,
                        what: Happened::Finished(outcome),
                    })
                    .expect("a running job ends");
                report.ended += 1;
            }
        }
    }
    report
}

fn main() {
    let folder = std::env::args().nth(1);
    let mut journal: Box<dyn Journal> = match &folder {
        Some(f) => Box::new(InTad::open(f).expect("a folder to keep the journal in")),
        None => Box::new(InMemory::default()),
    };
    let lines = std::io::stdin().lock().lines().map_while(Result::ok);
    let report = replay(lines, journal.as_mut());
    println!(
        "the old agent started {} jobs, and restarted {} time(s)",
        report.agreed + report.refused_in_all(),
        report.restarts
    );
    println!("oco-agent would have started {} of them ({} ran to their end; {} it would have parked at a restart)", report.agreed, report.ended, report.parked);
    for (why, (count, examples)) in &report.refused {
        println!("oco-agent would have refused {count}: {why}");
        for e in examples {
            println!("    {e}");
        }
    }
    if let Some(f) = folder {
        println!("the journal is kept in {f}");
    }
    if report.refused_in_all() > 0 {
        std::process::exit(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> impl Iterator<Item = String> + '_ {
        text.lines().map(str::to_string)
    }

    #[test]
    fn a_stamp_is_a_moment() {
        assert_eq!(moment("2026-11-11T21:04:29+00:00"), Some(1_794_431_069));
        assert_eq!(
            moment("2026-11-11T22:04:29+01:00"),
            Some(1_794_431_069),
            "an hour ahead is the same moment"
        );
        assert_eq!(moment("not a stamp"), None);
    }

    #[test]
    fn readings_join_up_and_a_replay_in_a_later_one_is_caught() {
        let mut journal = InMemory::default();
        let first = replay(
            lines("2026-11-12T10:00:00+01:00 grig lab-agent[1]: lab-agent: starting job-a\n2026-11-12T10:01:00+01:00 grig lab-agent[1]: lab-agent: job-a finished: ok"),
            &mut journal,
        );
        assert_eq!(
            (first.agreed, first.ended, first.refused_in_all()),
            (1, 1, 0)
        );
        let second = replay(
            lines("2026-11-12T11:00:00+01:00 grig systemd[9]: Started lab-agent.\n2026-11-12T11:00:05+01:00 grig lab-agent[2]: lab-agent: starting job-a"),
            &mut journal,
        );
        assert_eq!(
            second.refused.get("it had already finished").map(|r| r.0),
            Some(1),
            "the later reading remembers job-a finished"
        );
    }

    #[test]
    fn a_reading_that_begins_mid_run_still_parks_at_the_next_restart() {
        let mut journal = InMemory::default();
        let report = replay(
            lines("2026-11-12T10:00:00+01:00 grig lab-agent[1]: lab-agent: starting job-b\n2026-11-12T10:05:00+01:00 grig systemd[9]: Started lab-agent.\n2026-11-12T10:05:05+01:00 grig lab-agent[2]: lab-agent: starting job-b"),
            &mut journal,
        );
        assert_eq!(report.parked, 1, "job-b was cut off by the restart");
        assert_eq!(
            report
                .refused
                .get("it was parked when the agent restarted")
                .map(|r| r.0),
            Some(1),
            "and starting it again unasked is refused"
        );
    }
}
