use anyhow::{Context, Result};
use std::process::Command;

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
pub fn run() -> Result<()> {
    let exe = std::env::current_exe().context("finding my own path")?;

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
