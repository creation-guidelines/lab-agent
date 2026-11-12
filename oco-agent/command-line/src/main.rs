//! oco-agent: the commons' system for coordinating work. This file only wires: it reads the declared
//! command line, opens the folder where the journal, the beats and the jobs are kept - or the repository
//! work is asked through and spoken back to - and hands over.

mod commands;
mod contract;
mod processes;
mod speaking;

use argenv_resolve::*;
use contract::{Command, Contract};
use oco_agent_core::doing::Doing;
use oco_agent_core::journal::{state, Entry, Journal, Moment, Trouble};
use oco_agent_core::leases::Beats;
use oco_agent_github::GitHub;
use oco_agent_journal_in_tad::InTad;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The repository named, with the token from OCO_AGENT_GITHUB_TOKEN, else lab-agent's token file.
fn repository(repo: &str) -> Result<GitHub, Trouble> {
    let token = match std::env::var("OCO_AGENT_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    {
        Some(token) => token,
        None => {
            let file = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".config/lab-agent/token");
            std::fs::read_to_string(&file).map_err(|e| Trouble::Unreachable {
                because: format!(
                    "no GitHub token: set OCO_AGENT_GITHUB_TOKEN, or keep one in {} ({e})",
                    file.display()
                ),
            })?
        }
    };
    Ok(GitHub::new(repo, token.trim().to_string()))
}

/// The journal a repository speaks, seen through a copy read as tad tables, with the repository's beats.
struct Seen {
    entries: Vec<Entry>,
    remote: GitHub,
}

impl Seen {
    fn of(remote: GitHub) -> Result<Seen, Trouble> {
        let entries = spoken_journal(&remote)?;
        Ok(Seen { entries, remote })
    }
}

fn spoken_journal(remote: &GitHub) -> Result<Vec<Entry>, Trouble> {
    let copy = std::env::temp_dir().join(format!("oco-agent-seen-{}", std::process::id()));
    let seen = InTad::open(&copy)?;
    let text = remote.spoken("oco-agent/journal.json")?.unwrap_or_default();
    std::fs::write(copy.join("data/journal.json"), text).map_err(|e| Trouble::Unreachable {
        because: e.to_string(),
    })?;
    seen.entries()
}

impl Journal for Seen {
    fn entries(&self) -> Result<Vec<Entry>, Trouble> {
        Ok(self.entries.clone())
    }

    fn record(&mut self, entry: Entry) -> Result<(), Trouble> {
        Err(Trouble::Refused {
            job: entry.job,
            because: "what a repository spoke is read, not written to".into(),
        })
    }
}

impl Beats for Seen {
    fn heard(&self) -> Result<HashMap<String, Moment>, Trouble> {
        self.remote.heard()
    }

    fn beat(&mut self, driver: &str, at: Moment) -> Result<(), Trouble> {
        self.remote.beat(driver, at)
    }
}

/// A job's output: from the folder, or as the repository spoke it.
fn log_of(job: &str, folder: &Path, via: Option<&GitHub>) -> Result<String, Trouble> {
    match via {
        Some(remote) => Ok(remote
            .spoken(&format!("oco-agent/logs/{job}.log"))?
            .unwrap_or_default()),
        None => Ok(
            std::fs::read_to_string(folder.join("logs").join(format!("{job}.log")))
                .unwrap_or_default(),
        ),
    }
}

/// Waits for `job` to end, beating while it waits; its output, its end in words, and an exit status.
fn wait(
    job: &str,
    who: &str,
    folder: &Path,
    mut via: Option<GitHub>,
    within: u64,
) -> Result<(String, i32), Trouble> {
    let began = seconds();
    let mut last_beat = 0;
    loop {
        let now = seconds();
        let entries = match via.as_mut() {
            Some(remote) => {
                if now.saturating_sub(last_beat) >= 60 {
                    remote.beat(who, now)?;
                    last_beat = now;
                }
                spoken_journal(remote)?
            }
            None => {
                let mut kept = InTad::open(folder)?;
                if now.saturating_sub(last_beat) >= 60 {
                    kept.beat(who, now)?;
                    last_beat = now;
                }
                kept.entries()?
            }
        };
        let stands = state(&entries, job);
        if let Some((words, status)) = stands.as_ref().and_then(commands::ending) {
            return Ok((
                format!("{}{job} {words}\n", log_of(job, folder, via.as_ref())?),
                status,
            ));
        }
        if now.saturating_sub(began) >= within {
            return Ok((
                format!(
                    "{job} has not ended within {within} s: it is {}\n",
                    commands::standing_in_words(stands.as_ref())
                ),
                2,
            ));
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// The runner's loop: one step every `every` seconds, taking work in every `listen_every` and speaking
/// what happened back to the repository, saying what happened.
fn run(
    folder: &Path,
    at_once: usize,
    lasts: u64,
    every: u64,
    listen: Option<(GitHub, u64)>,
) -> Result<(), Trouble> {
    let mut journal = InTad::open(folder)?;
    let beats = InTad::open(folder)?;
    // any number no earlier run used: the moment it began, to the millisecond
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    let mut doing = Doing::new(
        processes::Processes {
            folder: folder.to_path_buf(),
        },
        run,
        at_once,
        lasts,
    );
    let mut speaking = speaking::Speaking::default();
    let mut passed_over = HashSet::new();
    let mut last_taken_in = 0;
    let mut short_of_budget = false;
    match &listen {
        Some((remote, _)) => println!("oco-agent: run {run}, doing at most {at_once} job(s) at once, kept in {}, listening and speaking to {}", folder.display(), remote.repo),
        None => println!("oco-agent: run {run}, doing at most {at_once} job(s) at once, kept in {}", folder.display()),
    }
    loop {
        let now = seconds();
        if let Some((remote, listen_every)) = &listen {
            // the hour's GitHub budget is shared by everything on the account: when it runs low, take in less often
            let low = remote.budget_left().is_some_and(|left| left < 500);
            if low != short_of_budget {
                short_of_budget = low;
                match low {
                    true => println!(
                        "GitHub's budget is low ({} left this hour): taking work in every 2 min",
                        remote.budget_left().unwrap_or(0)
                    ),
                    false => {
                        println!("GitHub's budget is back: taking work in every {listen_every} s")
                    }
                }
            }
            let wait = if low {
                (*listen_every).max(120)
            } else {
                *listen_every
            };
            if now.saturating_sub(last_taken_in) >= wait {
                match commands::take_in(&mut journal, folder, remote, &mut passed_over, now) {
                    Ok(said) => said.iter().for_each(|s| println!("{s}")),
                    Err(trouble) => {
                        eprintln!("oco-agent: taking in: {}", commands::plainly(&trouble))
                    }
                }
                last_taken_in = now;
            }
        }
        match doing.step(&mut journal, &beats, seconds()) {
            Ok(said) => said.iter().for_each(|s| println!("{s}")),
            Err(trouble) => eprintln!("oco-agent: {}", commands::plainly(&trouble)),
        }
        if let Some((remote, _)) = &listen {
            if let Err(trouble) = speaking.speak(folder, remote, &doing.running(), seconds()) {
                eprintln!("oco-agent: speaking: {}", commands::plainly(&trouble));
            }
        }
        std::thread::sleep(Duration::from_secs(every.max(1)));
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let Some(command) = argv.get(1).and_then(|token| Command::parse(token)) else {
        eprint!("{}", contract::usage());
        std::process::exit(2);
    };
    let resolved = Contract::parse_and_lint(OnProblems::FailOnError);
    let folder = Contract::FOLDER
        .get_from_or_default(&resolved)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/share/oco-agent")
        });
    let who = Contract::DRIVER
        .get_from_or_default(&resolved)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "someone".into());
    let lasts = Contract::BEAT_LASTS
        .get_from_or_default(&resolved)
        .unwrap_or(600);
    let within = Contract::START_WITHIN
        .get_from_or_default(&resolved)
        .unwrap_or(3600);
    let at_once = Contract::AT_ONCE
        .get_from_or_default(&resolved)
        .unwrap_or(1) as usize;
    let job = Contract::JOB
        .get_from_or_default(&resolved)
        .unwrap_or_default();
    let via = Contract::VIA.get_from_or_default(&resolved);
    let now = seconds();

    let script = || -> Result<Option<String>, Trouble> {
        match Contract::FILE.get_from_or_default(&resolved) {
            Some(file) => {
                std::fs::read_to_string(&file)
                    .map(Some)
                    .map_err(|e| Trouble::Unreachable {
                        because: format!("{file}: {e}"),
                    })
            }
            None => Ok(None),
        }
    };
    let remote = || via.as_deref().map(repository).transpose();
    let mut status = 0;
    let answer = match (command, via.as_deref()) {
        (Command::Run, _) => {
            let listen = match Contract::LISTEN.get_from_or_default(&resolved) {
                Some(repo) => repository(&repo).map(|remote| {
                    Some((
                        remote,
                        Contract::LISTEN_EVERY
                            .get_from_or_default(&resolved)
                            .unwrap_or(15),
                    ))
                }),
                None => Ok(None),
            };
            listen
                .and_then(|listen| {
                    run(
                        &folder,
                        at_once,
                        lasts,
                        Contract::EVERY.get_from_or_default(&resolved).unwrap_or(2),
                        listen,
                    )
                })
                .map(|()| String::new())
        }
        (Command::Log, _) => remote().and_then(|r| log_of(&job, &folder, r.as_ref())),
        (Command::Wait, _) => remote()
            .and_then(|r| {
                wait(
                    &job,
                    &who,
                    &folder,
                    r,
                    Contract::WITHIN
                        .get_from_or_default(&resolved)
                        .unwrap_or(600),
                )
            })
            .map(|(text, s)| {
                status = s;
                text
            }),
        (Command::Beat, Some(repo)) => repository(repo).and_then(|mut remote| {
            remote
                .beat(&who, now)
                .map(|()| format!("{who} is here, through {repo}\n"))
        }),
        (Command::Submit, Some(repo)) => repository(repo).and_then(|mut remote| {
            let script = script()?;
            let may_run = Contract::MAY_RUN
                .get_from_or_default(&resolved)
                .unwrap_or(1800);
            commands::submit_via(
                &mut remote,
                &commands::Request {
                    job: &job,
                    who: &who,
                    start_within: within,
                    may_run,
                    script: script.as_deref(),
                },
                now,
            )
        }),
        (Command::Standing, Some(repo)) => repository(repo)
            .and_then(Seen::of)
            .and_then(|seen| commands::where_things_stand(&seen, now, lasts)),
        _ => InTad::open(&folder).and_then(|mut kept| match command {
            Command::Beat => commands::beat(&mut kept, &who, now),
            Command::Submit => {
                let script = script()?;
                let may_run = Contract::MAY_RUN
                    .get_from_or_default(&resolved)
                    .unwrap_or(1800);
                commands::submit(
                    &mut kept,
                    &folder,
                    &commands::Request {
                        job: &job,
                        who: &who,
                        start_within: within,
                        may_run,
                        script: script.as_deref(),
                    },
                    now,
                )
            }
            Command::Release => commands::release(&mut kept, &job, &who, now, within),
            Command::Standing => commands::where_things_stand(&kept, now, lasts),
            Command::Next => commands::what_next(
                &kept,
                now,
                Contract::RUN.get_from_or_default(&resolved).unwrap_or(1),
                at_once,
                lasts,
            ),
            Command::Run | Command::Log | Command::Wait => unreachable!("handled above"),
        }),
    };
    match answer {
        Ok(text) => print!("{text}"),
        Err(trouble) => {
            eprintln!("oco-agent: {}", commands::plainly(&trouble));
            std::process::exit(1);
        }
    }
    std::process::exit(status);
}
