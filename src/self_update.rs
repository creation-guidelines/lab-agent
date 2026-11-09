use anyhow::{Context, Result};
use std::process::Command;

/// Updates the `lab-agent` package via pacman and restarts the service to pick it up. Meant to
/// be callable from a pushed job (so a remote caller - a chat session, a script - can trigger an
/// update without a human at the keyboard), as well as by hand.
///
/// Updates only this one package (`pacman -Sy` then `-S lab-agent`, not a full `-Syu`), since a
/// full system upgrade is a much bigger, separate decision than "get the newest lab-agent".
/// `root-exec` is used for the pacman calls (this needs root); the service restart runs as the
/// invoking user, since `systemctl --user` only makes sense there.
///
/// Note for anyone calling this as a subprocess of the very daemon it restarts (a job script
/// does exactly this): the restart kills the process running this command too, partway through
/// its own cleanup - that is expected, not a bug. The update and the restart have both already
/// been issued by the time that happens.
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

    println!("lab-agent self-update: restarting the service");
    let restart = Command::new("systemctl")
        .args(["--user", "restart", "lab-agent"])
        .status()?;
    anyhow::ensure!(
        restart.success(),
        "systemctl --user restart lab-agent failed"
    );

    println!("lab-agent self-update: done");
    Ok(())
}
