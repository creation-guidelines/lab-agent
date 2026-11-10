use anyhow::{Context, Result};
use std::process::Command;

use crate::contract::Contract;
use argenv_resolve::*;

/// Updates the `lab-agent` package via pacman. The running agent notices the new program and
/// switches to it by itself as soon as no job runs - so an update never cuts a job off. Meant to
/// be callable from a pushed job (so a remote caller - a chat session, a script - can trigger an
/// update without a human at the keyboard), as well as by hand.
///
/// Updates only this one package (`pacman -Sy` then `-S lab-agent`, not a full `-Syu`), since a
/// full system upgrade is a much bigger, separate decision than "get the newest lab-agent".
/// `root-exec` is used for the pacman calls (this needs root); the service restart runs as the
/// invoking user, since `systemctl --user` only makes sense there.
///
pub fn run(resolved: &Resolution) -> Result<()> {
    let exe = std::env::current_exe().context("finding my own path")?;
    let within = Contract::WITHIN.get_from_or_default(resolved);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(within.unwrap_or(0));
    loop {
        println!("lab-agent self-update: syncing package databases");
        let sync = Command::new("sudo")
            .args([
                "-n",
                exe.to_str().unwrap(),
                "root-exec",
                "--",
                "pacman",
                "-Sy",
                "--noconfirm",
            ])
            .status()?;
        anyhow::ensure!(sync.success(), "pacman -Sy failed");
        if within.is_none()
            || offers_another(
                &pacman(&["-Q", "lab-agent"]),
                &pacman(&["-Si", "lab-agent"]),
            )
        {
            break;
        }
        if std::time::Instant::now() >= deadline {
            println!(
                "lab-agent self-update: no newer lab-agent within {}s - nothing installed",
                within.unwrap_or(0)
            );
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    println!("lab-agent self-update: installing the newest lab-agent");
    let install = Command::new("sudo")
        .args([
            "-n",
            exe.to_str().unwrap(),
            "root-exec",
            "--",
            "pacman",
            "-S",
            "--noconfirm",
            "lab-agent",
        ])
        .status()?;
    anyhow::ensure!(install.success(), "pacman -S lab-agent failed");

    println!("lab-agent self-update: installed - the running agent switches to it as soon as no job runs");
    println!("lab-agent self-update: done");
    Ok(())
}

/// A pacman query's output, or nothing.
fn pacman(args: &[&str]) -> String {
    Command::new("pacman")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Whether the repository offers a lab-agent other than the installed one: `pacman -Q` says
/// "lab-agent 0.1.28-1", `pacman -Si` has a "Version : 0.1.29-1" line.
fn offers_another(installed: &str, offered: &str) -> bool {
    let have = installed.split_whitespace().nth(1);
    let repo = offered
        .lines()
        .find(|l| l.starts_with("Version"))
        .and_then(|l| l.split(':').nth(1))
        .map(str::trim);
    matches!((have, repo), (Some(h), Some(r)) if h != r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_package_is_one_whose_version_differs_from_the_installed() {
        let si = "Repository      : lab-agent\nName            : lab-agent\nVersion         : 0.1.29-1\n";
        assert!(offers_another("lab-agent 0.1.28-1\n", si));
        assert!(
            !offers_another("lab-agent 0.1.29-1\n", si),
            "the same version"
        );
        assert!(!offers_another("", si), "not installed: nothing to compare");
        assert!(
            !offers_another("lab-agent 0.1.28-1\n", ""),
            "not in the repository"
        );
    }
}
