//! oco-agent: the commons' system for coordinating work. This file only wires: it reads the declared
//! command line, opens the folder where the journal and the beats are kept, and hands over.

mod commands;
mod contract;

use argenv_resolve::*;
use contract::{Command, Contract};
use oco_agent_journal_in_tad::InTad;
use std::path::PathBuf;

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
    let job = Contract::JOB
        .get_from_or_default(&resolved)
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let answer = InTad::open(&folder).and_then(|mut kept| match command {
        Command::Beat => commands::beat(&mut kept, &who, now),
        Command::Submit => commands::submit(&mut kept, &job, &who, now, within),
        Command::Release => commands::release(&mut kept, &job, &who, now, within),
        Command::Standing => commands::where_things_stand(&kept, now, lasts),
        Command::Next => commands::what_next(
            &kept,
            now,
            Contract::RUN.get_from_or_default(&resolved).unwrap_or(1),
            Contract::AT_ONCE
                .get_from_or_default(&resolved)
                .unwrap_or(1) as usize,
            lasts,
        ),
    });
    match answer {
        Ok(text) => print!("{text}"),
        Err(trouble) => {
            eprintln!("oco-agent: {}", commands::plainly(&trouble));
            std::process::exit(1);
        }
    }
}
