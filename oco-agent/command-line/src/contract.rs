//! oco-agent's command line, declared once with argenv: every command, flag and environment variable
//! it accepts, and what each means. Parsing and help both come from these declarations, so they
//! cannot disagree with what the program accepts.

use argenv_resolve::*;

/// Every command. Inputs scoped to one are checked by the compiler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Beat,
    Submit,
    Release,
    Standing,
    Next,
    Run,
}

impl Command {
    pub const ALL: &'static [Command] = &[
        Command::Beat,
        Command::Submit,
        Command::Release,
        Command::Standing,
        Command::Next,
        Command::Run,
    ];

    /// How it is typed.
    pub const fn token(self) -> &'static str {
        match self {
            Command::Beat => "beat",
            Command::Submit => "submit",
            Command::Release => "release",
            Command::Standing => "standing",
            Command::Next => "next",
            Command::Run => "run",
        }
    }

    /// One line for the command list in the help.
    pub const fn summary(self) -> &'static str {
        match self {
            Command::Beat => "Say you are here: your waiting work may start while you are",
            Command::Submit => "Ask for a job: submit JOB --file SCRIPT (asking is itself a beat)",
            Command::Release => "Send a parked job back to wait: release JOB",
            Command::Standing => "Where every job stands, and which drivers are here",
            Command::Next => "What the agent would do now, and why anything waits - it names the acts and takes none",
            Command::Run => "Do the work: start what may start, write down how each job ends, never run one twice",
        }
    }

    pub fn parse(token: &str) -> Option<Command> {
        Command::ALL.iter().copied().find(|c| c.token() == token)
    }
}

impl serde::Serialize for Command {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.token())
    }
}

/// The help: every command with its one line.
pub fn usage() -> String {
    let mut text = String::from("oco-agent: the commons' system for coordinating work\n\nusage: oco-agent COMMAND [--folder FOLDER] [--as NAME]\n\n");
    for c in Command::ALL {
        text.push_str(&format!("  {:<9} {}\n", c.token(), c.summary()));
    }
    text
}

contract! {
    /// Where the journal, the beats and the jobs are kept.
    FOLDER: String = Input {
        key:     "folder",
        ty:      Type::String,
        env:     Some(Env::new("OCO_AGENT_FOLDER")),
        arg:     Some(Arg { value_name: "FOLDER", ..Arg::long("folder") }),
        summary: "Where the journal, the beats and the jobs are kept (default: ~/.local/share/oco-agent)",
        ..Input::EMPTY
    };

    /// Who is driving.
    DRIVER: String = Input {
        key:     "as",
        ty:      Type::String,
        env:     Some(Env::new("OCO_AGENT_AS")),
        arg:     Some(Arg { value_name: "NAME", ..Arg::long("as") }),
        summary: "Who is driving - a person or a machine (default: your user name)",
        ..Input::EMPTY
    };

    /// How long a beat keeps a driver's lease alive.
    BEAT_LASTS: u64 = Input {
        key:     "beat_lasts",
        ty:      Type::Uint,
        default: Some(600),
        env:     Some(Env::new("OCO_AGENT_BEAT_LASTS")),
        arg:     Some(Arg { value_name: "SECONDS", ..Arg::long("beat-lasts") }),
        summary: "How long a beat keeps a driver's lease alive, in seconds",
        ..Input::EMPTY
    };

    /// The job a command is about.
    JOB: String, Command = Input {
        key:         "job",
        ty:          Type::String,
        subcommands: &[Command::Submit, Command::Release],
        arg:         Some(Arg { value_name: "JOB", ..Arg::positional(0) }),
        summary:     "The job's name: letters, digits, '-', '_' and '.'",
        ..Input::EMPTY
    };

    /// The script a job runs.
    FILE: String, Command = Input {
        key:         "file",
        ty:          Type::String,
        subcommands: &[Command::Submit],
        arg:         Some(Arg { value_name: "SCRIPT", ..Arg::long("file") }),
        summary:     "The script the job runs (sh)",
        ..Input::EMPTY
    };

    /// How long a job may wait to be started.
    START_WITHIN: u64, Command = Input {
        key:         "start_within",
        ty:          Type::Uint,
        default:     Some(3600),
        subcommands: &[Command::Submit, Command::Release],
        arg:         Some(Arg { value_name: "SECONDS", ..Arg::long("start-within") }),
        summary:     "Start the job within this many seconds, or not at all",
        ..Input::EMPTY
    };

    /// How long a job may run.
    MAY_RUN: u64, Command = Input {
        key:         "may_run",
        ty:          Type::Uint,
        default:     Some(1800),
        subcommands: &[Command::Submit],
        arg:         Some(Arg { value_name: "SECONDS", ..Arg::long("may-run") }),
        summary:     "How long the job may run before it is stopped, in seconds",
        ..Input::EMPTY
    };

    /// How many jobs may run side by side.
    AT_ONCE: u64, Command = Input {
        key:         "at_once",
        ty:          Type::Uint,
        default:     Some(1),
        subcommands: &[Command::Next, Command::Run],
        arg:         Some(Arg { value_name: "N", ..Arg::long("at-once") }),
        summary:     "How many jobs may run side by side",
        ..Input::EMPTY
    };

    /// Which run of the agent is asking.
    RUN: u64, Command = Input {
        key:         "run",
        ty:          Type::Uint,
        default:     Some(1),
        subcommands: &[Command::Next],
        arg:         Some(Arg { value_name: "N", ..Arg::long("run") }),
        summary:     "Which run of the agent this is (a later run parks what an earlier one left running)",
        ..Input::EMPTY
    };

    /// How often the runner looks.
    EVERY: u64, Command = Input {
        key:         "every",
        ty:          Type::Uint,
        default:     Some(2),
        subcommands: &[Command::Run],
        arg:         Some(Arg { value_name: "SECONDS", ..Arg::long("every") }),
        summary:     "How often the runner looks at the journal and its jobs, in seconds",
        ..Input::EMPTY
    };
}
