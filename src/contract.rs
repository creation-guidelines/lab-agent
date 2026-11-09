//! lab-agent's invocation surface, declared once with argenv.
//!
//! This file is the single source of truth for subcommands, flags and environment variables:
//! parsing, the subcommand list in `--help`, shell completions and the man page all derive from
//! these declarations (see `completions.rs`), so none of them can disagree with what the
//! program accepts.

use argenv_resolve::*;

/// Every subcommand. Inputs scoped to one are checked by the compiler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Connect,
    Run,
    Monitor,
    Jobs,
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

    /// Machine-readable output for `jobs`.
    JOBS_JSON: bool, Command = Input {
        key:         "json",
        ty:          Type::Bool,
        default:     Some(false),
        subcommands: &[Command::Jobs],
        env:         Some(Env::new("LAB_AGENT_JOBS_JSON")),
        arg:         Some(Arg { negatable: false, ..Arg::long("json") }),
        summary:     "Print JSON instead of a table",
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
        assert_eq!(Contract::JOBS_JSON.get_from_or_default(&r), Some(true));
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
}
