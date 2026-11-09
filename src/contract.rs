//! lab-agent's invocation surface, declared once with argenv.
//!
//! This file is the single source of truth for subcommands, flags and environment variables:
//! parsing, the subcommand list in `--help`, shell completions and the man page all derive from
//! these declarations (see `completions.rs`), so none of them can disagree with what the
//! program accepts.

use argenv_resolve::*;

/// A client with no config file (an agent, a CI job) is configured by these - never by a flag, so
/// the token cannot land in shell history or a process listing.
pub const ENV_TOKEN: &str = "LAB_AGENT_TOKEN";
pub const ENV_OWNER: &str = "LAB_AGENT_OWNER";
pub const ENV_JOBS_REPO: &str = "LAB_AGENT_JOBS_REPO";
pub const ENV_RESULTS_REPO: &str = "LAB_AGENT_RESULTS_REPO";
pub const ENV_API_URL: &str = "LAB_AGENT_API_URL";

/// Every subcommand. Inputs scoped to one are checked by the compiler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Connect,
    Run,
    Monitor,
    Jobs,
    Submit,
    Wait,
    Logs,
    SelfUpdate,
    RootExec,
    Completions,
    Man,
}

impl Command {
    pub const ALL: &'static [Command] = &[
        Command::Connect,
        Command::Run,
        Command::Monitor,
        Command::Jobs,
        Command::Submit,
        Command::Wait,
        Command::Logs,
        Command::SelfUpdate,
        Command::RootExec,
        Command::Completions,
        Command::Man,
    ];

    /// How it is typed.
    pub const fn token(self) -> &'static str {
        match self {
            Command::Connect => "connect",
            Command::Run => "run",
            Command::Monitor => "monitor",
            Command::Jobs => "jobs",
            Command::Submit => "submit",
            Command::Wait => "wait",
            Command::Logs => "logs",
            Command::SelfUpdate => "self-update",
            Command::RootExec => "root-exec",
            Command::Completions => "completions",
            Command::Man => "man",
        }
    }

    /// One line for the command list in `--help`, completions and the man page.
    pub const fn summary(self) -> &'static str {
        match self {
            Command::Connect => {
                "One-time interactive setup: repos, token, sudo grant, systemd service"
            }
            Command::Run => "The daemon loop (normally started by the systemd service)",
            Command::Monitor => "A live TUI of ongoing and recent jobs",
            Command::Jobs => "List recent jobs, as a table or JSON, for scripting",
            Command::Submit => "Push a script as a job; with --wait, wait and print its output",
            Command::Wait => "Wait for a job to finish: prints its output, exits with its status",
            Command::Logs => "Print a job's output (so far, if it is still running)",
            Command::SelfUpdate => "Update lab-agent through pacman and restart the service",
            Command::RootExec => "Run a command as root: root-exec -- COMMAND [ARGS...]",
            Command::Completions => "Print shell completions (bash, zsh, fish, nushell)",
            Command::Man => "Print the man page",
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

    /// How many of the most recent jobs `jobs` shows.
    JOBS_COUNT: u64, Command = Input {
        key:         "count",
        ty:          Type::Uint,
        default:     Some(20),
        subcommands: &[Command::Jobs],
        env:         Some(Env::new("LAB_AGENT_JOBS_COUNT")),
        arg:         Some(Arg { negatable: false, ..Arg::pair("count", 'n') }),
        summary:     "How many of the most recent jobs to show",
        ..Input::EMPTY
    };

    /// Machine-readable output.
    JSON: bool, Command = Input {
        key:         "json",
        ty:          Type::Bool,
        default:     Some(false),
        subcommands: &[Command::Jobs, Command::Submit, Command::Wait],
        env:         Some(Env::new("LAB_AGENT_JSON")),
        arg:         Some(Arg { negatable: false, ..Arg::long("json") }),
        summary:     "Print JSON instead of text",
        ..Input::EMPTY
    };

    /// Which shell `completions` prints for.
    COMPLETIONS_SHELL: String, Command = Input {
        key:         "shell",
        ty:          Type::Enum,
        allowed:     &["bash", "zsh", "fish", "nushell"],
        subcommands: &[Command::Completions],
        arg:         Some(Arg { value_name: "SHELL", ..Arg::positional(0) }),
        summary:     "bash, zsh, fish or nushell",
        ..Input::EMPTY
    };

    /// The script `submit` pushes (`-` or nothing: standard input).
    SUBMIT_FILE: String, Command = Input {
        key:         "file",
        ty:          Type::Path,
        subcommands: &[Command::Submit],
        arg:         Some(Arg { value_name: "FILE", ..Arg::positional(0) }),
        summary:     "The script to run (default: standard input)",
        ..Input::EMPTY
    };

    /// A short name for the job, which becomes part of its id.
    SUBMIT_NAME: String, Command = Input {
        key:         "name",
        ty:          Type::String,
        subcommands: &[Command::Submit],
        arg:         Some(Arg { value_name: "NAME", ..Arg::long("name") }),
        summary:     "A short name for the job (default: the file's name)",
        ..Input::EMPTY
    };

    /// queue: run when its turn comes. preempt: cancel what is running and go first.
    SUBMIT_MODE: String, Command = Input {
        key:         "mode",
        ty:          Type::Enum,
        allowed:     &["queue", "preempt"],
        subcommands: &[Command::Submit],
        arg:         Some(Arg { value_name: "MODE", ..Arg::long("mode") }),
        summary:     "queue (default) or preempt (cancel what is running, go first)",
        ..Input::EMPTY
    };

    /// The job's own time limit, after which its whole process group is killed.
    SUBMIT_TIMEOUT: u64, Command = Input {
        key:         "timeout",
        ty:          Type::Uint,
        subcommands: &[Command::Submit],
        arg:         Some(Arg { value_name: "SECONDS", ..Arg::long("timeout") }),
        summary:     "The job's own time limit in seconds (default: 300, or its # lab: header)",
        ..Input::EMPTY
    };

    /// Wait for the job and print its output.
    SUBMIT_WAIT: bool, Command = Input {
        key:         "wait",
        ty:          Type::Bool,
        default:     Some(false),
        subcommands: &[Command::Submit],
        arg:         Some(Arg { negatable: false, ..Arg::pair("wait", 'w') }),
        summary:     "Wait for the job, print its output, exit with its status",
        ..Input::EMPTY
    };

    /// How long to wait before giving up (the job keeps running).
    WITHIN: u64, Command = Input {
        key:         "within",
        ty:          Type::Uint,
        subcommands: &[Command::Submit, Command::Wait],
        arg:         Some(Arg { value_name: "SECONDS", ..Arg::long("within") }),
        summary:     "Stop waiting after this many seconds; the job keeps running",
        ..Input::EMPTY
    };

    /// The job to wait for or read.
    JOB: String, Command = Input {
        key:         "job",
        ty:          Type::String,
        subcommands: &[Command::Wait, Command::Logs],
        arg:         Some(Arg { value_name: "JOB", ..Arg::positional(0) }),
        summary:     "The job's id, as `submit` printed it",
        ..Input::EMPTY
    };

    /// GitHub token for a client with no config file.
    TOKEN: String = Input {
        key:     "token",
        ty:      Type::String,
        env:     Some(Env::new(ENV_TOKEN)),
        summary: "GitHub token (repo scope) for a client with no config file; never a flag",
        ..Input::EMPTY
    };

    /// GitHub user or org owning the jobs and results repos, for a client with no config file.
    OWNER: String = Input {
        key:     "owner",
        ty:      Type::String,
        env:     Some(Env::new(ENV_OWNER)),
        summary: "Owner of the jobs/results repos, for a client with no config file",
        ..Input::EMPTY
    };

    /// The jobs repo, for a client with no config file.
    JOBS_REPO: String = Input {
        key:     "jobs_repo",
        ty:      Type::String,
        env:     Some(Env::new(ENV_JOBS_REPO)),
        summary: "The jobs repo (default: lab-agent-jobs)",
        ..Input::EMPTY
    };

    /// The GitHub API root, for GitHub Enterprise.
    API_URL: String = Input {
        key:     "api_url",
        ty:      Type::String,
        env:     Some(Env::new(ENV_API_URL)),
        summary: "The GitHub API root, for GitHub Enterprise (default: https://api.github.com)",
        ..Input::EMPTY
    };

    /// The results repo, for a client with no config file.
    RESULTS_REPO: String = Input {
        key:     "results_repo",
        ty:      Type::String,
        env:     Some(Env::new(ENV_RESULTS_REPO)),
        summary: "The results repo (default: lab-agent-results)",
        ..Input::EMPTY
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_declarations_are_sound() {
        assert_eq!(Contract::problems(), Vec::<String>::new());
    }

    #[test]
    fn every_command_round_trips_through_its_token() {
        for c in Command::ALL {
            assert_eq!(Command::parse(c.token()), Some(*c));
        }
    }

    fn resolve(args: &[&str]) -> Resolution {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        Invocation {
            args: &args,
            env: &EmptyEnv,
        }
        .resolve(&Contract::records())
    }

    struct EmptyEnv;
    impl EnvSource for EmptyEnv {
        fn get(&self, _: &str) -> Option<String> {
            None
        }
        fn names(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn jobs_flags_resolve_for_jobs() {
        let r = resolve(&["jobs", "-n", "5", "--json"]);
        assert_eq!(Contract::JOBS_COUNT.get_from_or_default(&r), Some(5));
        assert_eq!(Contract::JSON.get_from_or_default(&r), Some(true));
    }

    #[test]
    fn a_jobs_flag_given_to_another_subcommand_is_a_lint_finding() {
        let r = resolve(&["monitor", "--json"]);
        assert!(!lint(&Contract::records(), &r).is_empty());
        let r = resolve(&["jobs", "--json"]);
        assert!(lint(&Contract::records(), &r).is_empty());
    }

    #[test]
    fn a_misspelled_flag_is_a_lint_finding() {
        let r = resolve(&["jobs", "--josn"]);
        assert!(!lint(&Contract::records(), &r).is_empty());
    }

    #[test]
    fn the_completions_shell_is_the_positional_after_the_subcommand() {
        let r = resolve(&["completions", "zsh"]);
        assert_eq!(
            Contract::COMPLETIONS_SHELL.get_from_or_default(&r),
            Some("zsh".to_string())
        );
    }

    #[test]
    fn submit_flags_resolve() {
        let r = resolve(&[
            "submit",
            "--wait",
            "--mode",
            "preempt",
            "--timeout",
            "30",
            "--name",
            "x",
            "s.sh",
        ]);
        assert_eq!(Contract::SUBMIT_WAIT.get_from_or_default(&r), Some(true));
        assert_eq!(
            Contract::SUBMIT_MODE.get_from_or_default(&r),
            Some("preempt".to_string())
        );
        assert_eq!(Contract::SUBMIT_TIMEOUT.get_from_or_default(&r), Some(30));
        assert_eq!(
            Contract::SUBMIT_FILE.get_from_or_default(&r),
            Some("s.sh".to_string())
        );
        assert!(lint(&Contract::records(), &r).is_empty());
    }

    #[test]
    fn wait_takes_the_job_as_its_positional() {
        let r = resolve(&["wait", "--within", "60", "1700000000-build"]);
        assert_eq!(
            Contract::JOB.get_from_or_default(&r),
            Some("1700000000-build".to_string())
        );
        assert_eq!(Contract::WITHIN.get_from_or_default(&r), Some(60));
        assert!(lint(&Contract::records(), &r).is_empty());
    }

    #[test]
    fn submit_only_flags_are_findings_elsewhere() {
        assert!(!lint(&Contract::records(), &resolve(&["monitor", "--wait"])).is_empty());
        assert!(!lint(
            &Contract::records(),
            &resolve(&["wait", "--mode", "queue", "x"])
        )
        .is_empty());
    }

    #[test]
    fn a_bad_mode_is_a_finding() {
        let r = resolve(&["submit", "--mode", "sideways"]);
        assert!(!lint(&Contract::records(), &r).is_empty());
    }
}
