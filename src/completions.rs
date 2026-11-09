//! Tab completion and the man page. Neither is written by hand: both come from the contract
//! (`contract.rs`) through argenv's generator, so they cannot disagree with what the program
//! accepts. This file only says what argenv cannot know - the program's name, one-line
//! description, exit statuses - and passes the records through.

use crate::contract::{Command, Contract};
use anyhow::{anyhow, Result};
use argenv_resolve::generate::{self, Program, Shell};
use argenv_resolve::*;

fn commands() -> Vec<generate::Command<'static>> {
    Command::ALL
        .iter()
        .map(|c| generate::Command {
            name: c.token(),
            summary: c.summary(),
        })
        .collect()
}

fn program<'a>(commands: &'a [generate::Command<'a>], date: &'a str) -> Program<'a> {
    Program {
        name: "lab-agent",
        version: env!("CARGO_PKG_VERSION"),
        about: "pull-based job runner for hardware-in-the-loop work",
        commands,
        exit_status: &[
            ("0", "done"),
            ("1", "a job or setup step failed"),
            ("2", "the invocation is wrong"),
        ],
        see_also: &["systemd.service(5)", "sudoers(5)"],
        date,
    }
}

/// `lab-agent completions SHELL`: the completion script for `SHELL` on stdout.
pub fn completions(resolved: &Resolution) -> Result<()> {
    let token = Contract::COMPLETIONS_SHELL
        .get_from_or_default(resolved)
        .ok_or_else(|| anyhow!("usage: lab-agent completions bash|zsh|fish|nushell"))?;
    let shell = Shell::parse(&token)
        .ok_or_else(|| anyhow!("unknown shell '{token}': expected bash, zsh, fish or nushell"))?;
    let commands = commands();
    let text = generate::completions(shell, &program(&commands, ""), &Contract::records())
        .map_err(|e| anyhow!("{e:?}"))?;
    print!("{text}");
    Ok(())
}

/// `lab-agent man`: the man page on stdout. Dated by `SOURCE_DATE_EPOCH` when set, so a build is
/// reproducible.
pub fn man() -> Result<()> {
    let date = man_date(
        std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|s| s.parse().ok()),
    );
    let commands = commands();
    let text = generate::man_page(&program(&commands, &date), &Contract::records())
        .map_err(|e| anyhow!("{e:?}"))?;
    print!("{text}");
    Ok(())
}

/// `YYYY-MM-DD` for a Unix time (default: now), by the civil-from-days algorithm - no date crate
/// for one formatted line.
fn man_date(epoch: Option<u64>) -> String {
    let secs = epoch.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    });
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shell_generates_and_mentions_every_subcommand() {
        let commands = commands();
        for shell in Shell::ALL {
            let text = generate::completions(shell, &program(&commands, ""), &Contract::records())
                .expect("completions generate");
            for c in Command::ALL {
                assert!(
                    text.contains(c.token()),
                    "{shell:?} completions miss {}",
                    c.token()
                );
            }
        }
    }

    #[test]
    fn the_man_page_generates() {
        let commands = commands();
        let text = generate::man_page(&program(&commands, "2026-01-01"), &Contract::records())
            .expect("man page generates");
        assert!(text.contains("lab\\-agent"), "man pages escape hyphens");
    }

    #[test]
    fn the_date_is_the_epochs_own() {
        assert_eq!(man_date(Some(0)), "1970-01-01");
        assert_eq!(man_date(Some(1_794_217_435)), "2026-11-09");
    }
}
