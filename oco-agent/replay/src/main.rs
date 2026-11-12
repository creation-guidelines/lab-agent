//! What oco-agent would have done. Reads an old agent's log on standard input, line by line, and for
//! every job that agent started answers: would oco-agent have started it? Every start it agrees with
//! goes into a journal; every start it would have refused is counted, with the reason.
//!
//!     journalctl --user -u lab-agent -o short-iso | oco-agent-replay [FOLDER]
//!
//! With a FOLDER, the journal is kept there as tad tables, to read and to ask in SQL afterwards.

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

fn main() {
    let folder = std::env::args().nth(1);
    let mut journal: Box<dyn Journal> = match &folder {
        Some(f) => Box::new(InTad::open(f).expect("a folder to keep the journal in")),
        None => Box::new(InMemory::default()),
    };
    let (mut run, mut agreed, mut ended, mut parked) = (0u64, 0u64, 0u64, 0u64);
    let mut refused: BTreeMap<&'static str, (u64, Vec<String>)> = BTreeMap::new();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
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
                    parked += 1;
                }
            }
        } else if let Some(rest) = line.split("lab-agent: starting ").nth(1) {
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
            if next(&entries, &drivers, now, run.max(1), usize::MAX)
                .contains(&Act::Start(job.clone()))
            {
                journal
                    .record(Entry {
                        job,
                        at: now,
                        what: Happened::Claimed { run: run.max(1) },
                    })
                    .expect("a waiting job is claimed");
                agreed += 1;
            } else {
                let slot = refused
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
                ended += 1;
            }
        }
    }
    println!(
        "the old agent started {} jobs over {} run(s) of itself",
        agreed + refused.values().map(|r| r.0).sum::<u64>(),
        run.max(1)
    );
    println!("oco-agent would have started {agreed} of them ({ended} ran to their end; {parked} it would have parked at a restart)");
    for (why, (count, examples)) in &refused {
        println!("oco-agent would have refused {count}: {why}");
        for e in examples {
            println!("    {e}");
        }
    }
    if let Some(f) = folder {
        println!("the journal is kept in {f}");
    }
}

#[cfg(test)]
mod tests {
    use super::moment;

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
}
