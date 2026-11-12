//! oco-agent: the commons' system for coordinating work. This file only wires: it reads the declared
//! command line, opens the folder where the journal, the beats and the jobs are kept, and hands over.

mod commands;
mod contract;
mod processes;

use argenv_resolve::*;
use contract::{Command, Contract};
use oco_agent_core::doing::Doing;
use oco_agent_core::journal::Trouble;
use oco_agent_journal_in_tad::InTad;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The runner's loop: one step every `every` seconds, saying what happened.
fn run(folder: &Path, at_once: usize, lasts: u64, every: u64) -> Result<(), Trouble> {
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
    println!(
        "oco-agent: run {run}, doing at most {at_once} job(s) at once, kept in {}",
        folder.display()
    );
    loop {
        match doing.step(&mut journal, &beats, seconds()) {
            Ok(said) => said.iter().for_each(|s| println!("{s}")),
            Err(trouble) => eprintln!("oco-agent: {}", commands::plainly(&trouble)),
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
    let now = seconds();

    let answer = if command == Command::Run {
        run(
            &folder,
            at_once,
            lasts,
            Contract::EVERY.get_from_or_default(&resolved).unwrap_or(2),
        )
        .map(|()| String::new())
    } else {
        InTad::open(&folder).and_then(|mut kept| match command {
            Command::Beat => commands::beat(&mut kept, &who, now),
            Command::Submit => {
                let script =
                    match Contract::FILE.get_from_or_default(&resolved) {
                        Some(file) => Some(std::fs::read_to_string(&file).map_err(|e| {
                            Trouble::Unreachable {
                                because: format!("{file}: {e}"),
                            }
                        })?),
                        None => None,
                    };
                let request = commands::Request {
                    job: &job,
                    who: &who,
                    start_within: within,
                    may_run: Contract::MAY_RUN
                        .get_from_or_default(&resolved)
                        .unwrap_or(1800),
                    script: script.as_deref(),
                };
                commands::submit(&mut kept, &folder, &request, now)
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
            Command::Run => unreachable!("handled above"),
        })
    };
    match answer {
        Ok(text) => print!("{text}"),
        Err(trouble) => {
            eprintln!("oco-agent: {}", commands::plainly(&trouble));
            std::process::exit(1);
        }
    }
}
