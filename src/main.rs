mod agent;
mod board;
mod board_tui;
mod completions;
mod config;
mod connect;
mod contract;
mod daemon;
mod desktop;
mod github;
mod job;
mod jobs;
mod monitor;
mod notify;
mod self_update;

use anyhow::{bail, Result};
use argenv_resolve::*;
use contract::{Command, Contract};

fn main() {
    if let Err(e) = run() {
        // A spent GitHub budget is an expected state, not a crash: say so plainly, with a status
        // callers can act on, instead of a stack of error text.
        let limited = e.downcast_ref::<github::RateLimited>().is_some();
        eprintln!(
            "{}",
            if limited {
                format!("lab-agent: {e:#}")
            } else {
                format!("Error: {e:?}")
            }
        );
        std::process::exit(if limited {
            agent::exit::RATE_LIMITED
        } else {
            1
        });
    }
}

fn run() -> Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let sub = argv.get(1).map(String::as_str).unwrap_or("");

    if matches!(sub, "--version" | "-V") {
        println!("lab-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if matches!(sub, "" | "help" | "--help" | "-h") {
        print_help();
        return Ok(());
    }
    let Some(command) = Command::parse(sub) else {
        eprintln!("unknown subcommand '{sub}'\n");
        print_help();
        std::process::exit(2);
    };

    // root-exec hands its own argv straight to exec(); it is not a set of typed options, it *is*
    // the command to run, so it bypasses the argenv model entirely.
    if command == Command::RootExec {
        return root_exec(&argv[2..]);
    }

    // One call: resolve argv and env against the contract, lint it (misspelled flags, flags given
    // to a subcommand they do not belong to) and exit 1 with a clear message on an error.
    let resolved = Contract::parse_and_lint(OnProblems::FailOnError);
    if Contract::VERBOSE
        .get_from_or_default(&resolved)
        .unwrap_or(false)
    {
        std::env::set_var("LAB_AGENT_VERBOSE", "1");
    }

    match command {
        Command::Connect => connect::run(),
        Command::Run => daemon::run(config::load()?, config::load_token()?),
        Command::Monitor => monitor::run(&resolved),
        Command::Jobs => jobs::run(&resolved),
        Command::Submit => std::process::exit(agent::submit(&resolved)?),
        Command::Wait => std::process::exit(agent::wait(&resolved)?),
        Command::Logs => std::process::exit(agent::logs(&resolved)?),
        Command::Notify => notify::send_cli(&resolved),
        Command::Ask => std::process::exit(desktop::ask_cli(&resolved)?),
        Command::NotifySetup => notify::setup(),
        Command::Board => board::board_cli(&resolved),
        Command::Inbox => board::inbox_cli(&resolved),
        Command::SelfUpdate => self_update::run(),
        Command::Completions => completions::completions(&resolved),
        Command::Man => completions::man(),
        Command::RootExec => unreachable!("handled above"),
    }
}

/// The subcommand list comes from the contract, so help cannot drift from what is accepted.
fn print_help() {
    println!(
        "lab-agent - pull-based job runner: polls a git repo for jobs, runs them on real \
         hardware, reports results through git\n\nUSAGE:\n    lab-agent <subcommand> [options]\n\n\
         SUBCOMMANDS:"
    );
    for c in Command::ALL {
        println!("    {:<12} {}", c.token(), c.summary());
    }
    println!(
        "\nOPTIONS:\n    -v, --verbose   extra diagnostic output (env: LAB_AGENT_VERBOSE)\n    \
         -V, --version   print the version"
    );
}

/// The one thing job scripts use for anything privileged: `sudo lab-agent root-exec -- <cmd>`.
/// `connect` grants exactly one sudoers line, on this binary, once - no per-command sudoers
/// edits, ever again.
fn root_exec(args: &[String]) -> Result<()> {
    let args: Vec<&String> = args.iter().skip_while(|a| a.as_str() == "--").collect();
    let Some((cmd, rest)) = args.split_first() else {
        bail!("usage: lab-agent root-exec -- <command> [args...]");
    };
    let status = std::process::Command::new(cmd).args(rest).status()?;
    std::process::exit(status.code().unwrap_or(1));
}
