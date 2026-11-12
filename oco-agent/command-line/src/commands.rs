//! What each command does, as plain functions: given where things are kept, who is asking and the
//! time, they act and answer in words. Nothing here knows about the command line.

use oco_agent_core::deciding::{next, Act};
use oco_agent_core::journal::{
    standing, Entry, Happened, Journal, Moment, Outcome, State, Trouble,
};
use oco_agent_core::leases::{Beats, Leases};

/// A span of seconds, the way a person says it.
fn span(seconds: u64) -> String {
    match seconds {
        s if s < 120 => format!("{s} s"),
        s if s < 7200 => format!("{} min", s / 60),
        s => format!("{} h", s / 3600),
    }
}

/// Where a job stands, in words.
fn in_words(stands: &State) -> String {
    match stands {
        State::Waiting { .. } => "waiting".into(),
        State::Running { run } => format!("running (run {run} of the agent)"),
        State::Finished(Outcome::Done) => "finished".into(),
        State::Finished(Outcome::Failed) => "failed".into(),
        State::Finished(Outcome::TimedOut) => "timed out".into(),
        State::Parked(_) => "parked: the agent restarted while doing it".into(),
        State::Expired => "expired: nobody started it in time".into(),
    }
}

/// A trouble, said plainly.
pub fn plainly(trouble: &Trouble) -> String {
    match trouble {
        Trouble::Refused { job, because } => format!("{job}: {because}"),
        Trouble::Unreachable { because } => format!("the journal cannot be reached - {because}"),
    }
}

pub fn beat(kept: &mut impl Beats, who: &str, now: Moment) -> Result<String, Trouble> {
    kept.beat(who, now)?;
    Ok(format!("{who} is here\n"))
}

pub fn submit(
    kept: &mut (impl Journal + Beats),
    job: &str,
    who: &str,
    now: Moment,
    start_within: u64,
) -> Result<String, Trouble> {
    kept.beat(who, now)?;
    kept.record(Entry {
        job: job.into(),
        at: now,
        what: Happened::Submitted {
            by: who.into(),
            start_by: now + start_within,
        },
    })?;
    Ok(format!(
        "{job}: asked for by {who}; it starts within {} or not at all\n",
        span(start_within)
    ))
}

pub fn release(
    kept: &mut (impl Journal + Beats),
    job: &str,
    who: &str,
    now: Moment,
    start_within: u64,
) -> Result<String, Trouble> {
    kept.beat(who, now)?;
    kept.record(Entry {
        job: job.into(),
        at: now,
        what: Happened::Released {
            start_by: now + start_within,
        },
    })?;
    Ok(format!("{job}: sent back to wait by {who}\n"))
}

fn leases(kept: &impl Beats, lasts: u64) -> Result<Leases, Trouble> {
    Ok(Leases {
        heard: kept.heard()?,
        lasts,
    })
}

pub fn where_things_stand(
    kept: &(impl Journal + Beats),
    now: Moment,
    lasts: u64,
) -> Result<String, Trouble> {
    let (entries, drivers) = (kept.entries()?, leases(kept, lasts)?);
    let mut text = String::new();
    for s in standing(&entries) {
        text.push_str(&format!("{}  ({})  {}\n", s.job, s.by, in_words(&s.stands)));
    }
    if text.is_empty() {
        text.push_str("no jobs yet\n");
    }
    let mut names: Vec<_> = drivers.heard.iter().collect();
    names.sort();
    for (who, at) in names {
        let ago = span(now.saturating_sub(*at));
        text.push_str(&if drivers.live(who, now) {
            format!("{who} is here (heard {ago} ago)\n")
        } else {
            format!("{who} is silent (last heard {ago} ago)\n")
        });
    }
    Ok(text)
}

pub fn what_next(
    kept: &(impl Journal + Beats),
    now: Moment,
    run: u64,
    at_once: usize,
    lasts: u64,
) -> Result<String, Trouble> {
    let (entries, drivers) = (kept.entries()?, leases(kept, lasts)?);
    let mut text = String::new();
    for act in next(&entries, &drivers, now, run, at_once) {
        text.push_str(&match act {
            Act::Start(job) => format!("would start {job}\n"),
            Act::Expire(job) => format!("would expire {job}: nobody started it in time\n"),
            Act::Park(job, _) => {
                format!("would park {job}: an earlier run of the agent left it running\n")
            }
        });
    }
    // why anything that could run is not about to
    for s in standing(&entries) {
        if let State::Waiting { start_by } = s.stands {
            if start_by >= now && !drivers.live(s.by, now) {
                let silent = drivers
                    .heard
                    .get(s.by)
                    .map(|at| format!("silent for {}", span(now.saturating_sub(*at))))
                    .unwrap_or_else(|| "never heard from".into());
                text.push_str(&format!(
                    "{} waits: its driver, {}, is {silent}\n",
                    s.job, s.by
                ));
            }
        }
    }
    if text.is_empty() {
        text.push_str("nothing to do\n");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oco_agent_journal_in_tad::InTad;

    fn kept(name: &str) -> InTad {
        let folder =
            std::env::temp_dir().join(format!("oco-agent-commands-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        InTad::open(folder).unwrap()
    }

    #[test]
    fn asking_is_a_beat_and_the_job_would_start() {
        let mut k = kept("asking");
        assert_eq!(
            submit(&mut k, "land the docs", "ann", 100, 3600).unwrap(),
            "land the docs: asked for by ann; it starts within 60 min or not at all\n"
        );
        assert_eq!(
            what_next(&k, 101, 1, 1, 600).unwrap(),
            "would start land the docs\n"
        );
        assert_eq!(
            where_things_stand(&k, 160, 600).unwrap(),
            "land the docs  (ann)  waiting\nann is here (heard 60 s ago)\n"
        );
    }

    #[test]
    fn a_silent_drivers_job_waits_and_says_why() {
        let mut k = kept("silent");
        submit(&mut k, "roll out", "bo", 0, 7200).unwrap();
        assert_eq!(
            what_next(&k, 1500, 1, 1, 600).unwrap(),
            "roll out waits: its driver, bo, is silent for 25 min\n"
        );
        beat(&mut k, "bo", 1501).unwrap();
        assert_eq!(
            what_next(&k, 1502, 1, 1, 600).unwrap(),
            "would start roll out\n"
        );
    }

    #[test]
    fn what_cannot_be_done_is_said_plainly() {
        let mut k = kept("plainly");
        submit(&mut k, "once", "ann", 0, 3600).unwrap();
        let again = submit(&mut k, "once", "ann", 1, 3600).unwrap_err();
        assert_eq!(
            plainly(&again),
            "once: it cannot be asked for again: it is waiting"
        );
        assert!(
            release(&mut k, "once", "ann", 2, 3600).is_err(),
            "only a parked job is sent back"
        );
        assert_eq!(
            what_next(&kept("empty"), 0, 1, 1, 600).unwrap(),
            "nothing to do\n"
        );
    }
}
