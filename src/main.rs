mod config;
mod connect;
mod daemon;
mod github;
mod job;
mod monitor;

use anyhow::{bail, Result};
use argenv_resolve::*;

// lab-agent's invocation surface, declared once: one source for the parser, the generated
// --help, and (via `Contract::records()`) the published contract. Subcommand *selection*
// (connect/run/monitor/root-exec) is plain argv[1] dispatch - argenv models one flat
// invocation's flags, not a subcommand tree, so each subcommand would declare its own
// contract if it ever needs flags beyond this shared one.
contract! {
    /// Print extra diagnostic output.
    VERBOSE: bool = Input {
        key:     "verbose",
        ty:      Type::Bool,
        default: Some(false),
        env:     Some(Env::new("LAB_AGENT_VERBOSE")),
        arg:     Some(Arg { negatable: false, ..Arg::pair("verbose", 'v') }),
        summary: "Print extra diagnostic output",
        ..Input::EMPTY
    };
}

fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let sub = argv.get(1).map(String::as_str).unwrap_or("");

    // root-exec passes its own argv straight to exec(); it is not a set of typed options, it
    // *is* the command to run, so it bypasses the argenv model entirely.
    if sub == "root-exec" {
        return root_exec(&argv[2..]);
    }
    if sub == "--version" {
        println!("lab-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let rest: Vec<String> = argv.iter().skip(2).cloned().collect();
    let resolved = Invocation {
        args: &rest,
        env: &ProcessEnv,
    }
    .resolve(&Contract::records());
    let verbose = Contract::VERBOSE
        .get_from_or_default(&resolved)
        .unwrap_or(false);
    if verbose {
        std::env::set_var("LAB_AGENT_VERBOSE", "1");
    }

    match sub {
        "connect" => connect::run(),
        "run" => {
            let cfg = config::load()?;
            let token = config::load_token()?;
            daemon::run(cfg, token)
        }
        "monitor" => {
            let cfg = config::load()?;
            let token = config::load_token()?;
            monitor::run(cfg, token)
        }
        "" | "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("unknown subcommand '{other}'\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "lab-agent - pull-based job runner: polls a git repo for jobs, runs them on real \
         hardware, reports results through git\n\n\
         USAGE:\n    lab-agent <subcommand> [options]\n\n\
         SUBCOMMANDS:\n\
         \u{20}   connect    one-time interactive setup (repos, token, sudo, systemd service)\n\
         \u{20}   run        the daemon loop (normally started by the systemd service, not by hand)\n\
         \u{20}   monitor    a TUI listing ongoing and recent jobs\n\
         \u{20}   root-exec  -- <cmd...>   run a command as root; granted once via `connect`\n\n\
         OPTIONS:\n    -v, --verbose   extra diagnostic output (env: LAB_AGENT_VERBOSE)\n"
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
