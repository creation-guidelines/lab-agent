//! What each command does, as plain functions: given where things are kept, who is asking and the
//! time, they act and answer in words. Nothing here knows about the command line.

use oco_agent_core::deciding::{next, Act};
use oco_agent_core::journal::{
    standing, state, Entry, Happened, Journal, Moment, Outcome, State, Trouble,
};
use oco_agent_core::leases::{Beats, Leases};
use std::path::Path;

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

/// A job's name becomes a file's name, so it holds only what is safe in one.
fn a_safe_name(job: &str) -> Result<(), Trouble> {
    let fine = !job.is_empty()
        && job.len() <= 100
        && !job.starts_with('.')
        && job
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if fine {
        Ok(())
    } else {
        Err(Trouble::Refused { job: job.into(), because: "a job's name may hold only letters, digits, '-', '_' and '.', and not begin with '.'".into() })
    }
}

/// What a driver asks for.
pub struct Request<'a> {
    pub job: &'a str,
    pub who: &'a str,
    pub start_within: u64,
    pub may_run: u64,
    /// The script the job runs; kept as jobs/JOB.sh in the folder.
    pub script: Option<&'a str>,
}

pub fn beat(kept: &mut impl Beats, who: &str, now: Moment) -> Result<String, Trouble> {
    kept.beat(who, now)?;
    Ok(format!("{who} is here\n"))
}

pub fn submit(
    kept: &mut (impl Journal + Beats),
    folder: &Path,
    r: &Request,
    now: Moment,
) -> Result<String, Trouble> {
    a_safe_name(r.job)?;
    kept.beat(r.who, now)?;
    // the script is kept only for a job never asked for, so an old job's script is never overwritten
    if state(&kept.entries()?, r.job).is_none() {
        if let Some(script) = r.script {
            let jobs = folder.join("jobs");
            std::fs::create_dir_all(&jobs)
                .and_then(|()| std::fs::write(jobs.join(format!("{}.sh", r.job)), script))
                .map_err(|e| Trouble::Unreachable {
                    because: e.to_string(),
                })?;
        }
    }
    kept.record(Entry {
        job: r.job.into(),
        at: now,
        what: Happened::Submitted {
            by: r.who.into(),
            start_by: now + r.start_within,
            may_run: r.may_run,
        },
    })?;
    Ok(format!(
        "{}: asked for by {}; it starts within {} or not at all, and may run for {}\n",
        r.job,
        r.who,
        span(r.start_within),
        span(r.may_run)
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
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("oco-agent-commands-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    fn ask<'a>(job: &'a str, who: &'a str, start_within: u64) -> Request<'a> {
        Request {
            job,
            who,
            start_within,
            may_run: 1800,
            script: None,
        }
    }

    #[test]
    fn asking_is_a_beat_and_the_job_would_start() {
        let f = folder("asking");
        let mut k = InTad::open(&f).unwrap();
        assert_eq!(submit(&mut k, &f, &ask("land-the-docs", "ann", 3600), 100).unwrap(), "land-the-docs: asked for by ann; it starts within 60 min or not at all, and may run for 30 min\n");
        assert_eq!(
            what_next(&k, 101, 1, 1, 600).unwrap(),
            "would start land-the-docs\n"
        );
        assert_eq!(
            where_things_stand(&k, 160, 600).unwrap(),
            "land-the-docs  (ann)  waiting\nann is here (heard 60 s ago)\n"
        );
    }

    #[test]
    fn a_silent_drivers_job_waits_and_says_why() {
        let f = folder("silent");
        let mut k = InTad::open(&f).unwrap();
        submit(&mut k, &f, &ask("roll-out", "bo", 7200), 0).unwrap();
        assert_eq!(
            what_next(&k, 1500, 1, 1, 600).unwrap(),
            "roll-out waits: its driver, bo, is silent for 25 min\n"
        );
        beat(&mut k, "bo", 1501).unwrap();
        assert_eq!(
            what_next(&k, 1502, 1, 1, 600).unwrap(),
            "would start roll-out\n"
        );
    }

    #[test]
    fn what_cannot_be_done_is_said_plainly() {
        let f = folder("plainly");
        let mut k = InTad::open(&f).unwrap();
        submit(&mut k, &f, &ask("once", "ann", 3600), 0).unwrap();
        let again = submit(&mut k, &f, &ask("once", "ann", 3600), 1).unwrap_err();
        assert_eq!(
            plainly(&again),
            "once: it cannot be asked for again: it is waiting"
        );
        assert!(
            release(&mut k, "once", "ann", 2, 3600).is_err(),
            "only a parked job is sent back"
        );
        let e = folder("empty");
        assert_eq!(
            what_next(&InTad::open(&e).unwrap(), 0, 1, 1, 600).unwrap(),
            "nothing to do\n"
        );
    }

    #[test]
    fn a_name_that_is_not_safe_as_a_file_name_is_refused_and_nothing_is_written() {
        let f = folder("names");
        let mut k = InTad::open(&f).unwrap();
        for bad in ["../escape", "a/b", ".hidden", "with space", ""] {
            let e = submit(
                &mut k,
                &f,
                &Request {
                    script: Some("echo no"),
                    ..ask(bad, "ann", 60)
                },
                0,
            )
            .unwrap_err();
            assert!(
                plainly(&e).contains("may hold only"),
                "{bad}: {}",
                plainly(&e)
            );
        }
        assert!(!f.join("jobs").exists() && !f.parent().unwrap().join("escape.sh").exists());
    }

    #[test]
    fn a_second_asking_does_not_overwrite_the_first_ones_script() {
        let f = folder("script");
        let mut k = InTad::open(&f).unwrap();
        submit(
            &mut k,
            &f,
            &Request {
                script: Some("echo first"),
                ..ask("once", "ann", 60)
            },
            0,
        )
        .unwrap();
        assert!(submit(
            &mut k,
            &f,
            &Request {
                script: Some("echo second"),
                ..ask("once", "ann", 60)
            },
            1
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(f.join("jobs/once.sh")).unwrap(),
            "echo first"
        );
    }
}
