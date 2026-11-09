use crate::config::{self, Config};
use crate::github::Client;
use anyhow::{Context, Result};
use std::io::{self, Write};
use std::process::Command;

fn ask(prompt: &str, default: &str) -> Result<String> {
    print!("{prompt} [{default}]: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    })
}

/// Walks through one-time setup: who owns the jobs/results repos, a token
/// (asked for once, here, never as a CLI flag so it never lands in shell
/// history or `ps`), creates the repos if they don't exist yet, grants the
/// one sudoers line that lets job scripts run anything as root through
/// `lab-agent root-exec` without editing sudoers again later, and installs
/// + starts the systemd user service.
pub fn run() -> Result<()> {
    println!("lab-agent connect - one-time setup\n");

    let owner = ask(
        "GitHub user or org that will own the jobs/results repos",
        "",
    )?;
    anyhow::ensure!(!owner.is_empty(), "an owner is required");

    let project = ask(
        "Project name (used to name the repos, e.g. \"lab-agent\")",
        "lab-agent",
    )?;
    let jobs_repo = ask("Jobs repo name", &format!("{project}-jobs"))?;
    let results_repo = ask("Results repo name", &format!("{project}-results"))?;
    let max_parallel = ask("Max jobs running at once", "1")?.parse().unwrap_or(1);
    let poll_seconds = ask("Poll interval, seconds", "5")?.parse().unwrap_or(5);

    print!("GitHub token (repo scope, not echoed): ");
    io::stdout().flush()?;
    let token = rpassword::read_password().context("reading token")?;
    anyhow::ensure!(!token.is_empty(), "a token is required");

    let cfg = Config {
        owner: owner.clone(),
        jobs_repo: jobs_repo.clone(),
        results_repo: results_repo.clone(),
        jobs_branch: "main".to_string(),
        results_branch: "results".to_string(),
        max_parallel,
        poll_seconds,
    };

    let client = Client::new(token.clone());

    for (repo, branch) in [(&jobs_repo, "main"), (&results_repo, "results")] {
        if client.repo_exists(&owner, repo)? {
            println!("{owner}/{repo} already exists");
        } else {
            println!("creating {owner}/{repo} ...");
            client.create_repo(&owner, repo, true)?;
        }
        // Make sure the target branch exists by writing a README to it
        // (auto_init only creates the default branch).
        if branch != "main"
            && client
                .get_file(&owner, repo, "README.md", branch)?
                .is_none()
        {
            client.put_file(
                &owner,
                repo,
                "README.md",
                branch,
                format!("# {repo}\n\nResults branch for lab-agent.\n").as_bytes(),
                "chore: create results branch",
                None,
            )?;
        }
    }

    config::save(&cfg)?;
    config::save_token(&token)?;
    println!("config written to {}", config::config_path().display());

    setup_sudo()?;
    setup_systemd()?;

    println!(
        "\nready. Push a job to {owner}/{jobs_repo}'s jobs/ directory on {}.",
        cfg.jobs_branch
    );
    println!("Watch it:   journalctl --user -u lab-agent -f");
    println!("Monitor it: lab-agent monitor");
    println!(
        "Results:    https://github.com/{owner}/{results_repo}/tree/{}",
        cfg.results_branch
    );
    Ok(())
}

/// One sudoers line, covering the agent binary itself rather than an ever-
/// growing list of individual commands: job scripts that need root run
/// `sudo lab-agent root-exec -- <command...>` and this is the only grant
/// that's ever needed, for anything.
fn setup_sudo() -> Result<()> {
    let exe = std::env::current_exe()?;
    let user = std::env::var("USER").context("USER not set")?;
    let line = format!("{user} ALL=(root) NOPASSWD: {}\n", exe.display());
    let sudoers_path = "/etc/sudoers.d/lab-agent";

    let existing = std::fs::read_to_string(sudoers_path).unwrap_or_default();
    if existing.contains(exe.to_str().unwrap_or_default()) {
        println!("sudoers already grants root-exec on {}", exe.display());
        return Ok(());
    }

    println!(
        "granting NOPASSWD root-exec on {} (one sudo prompt now, never again):",
        exe.display()
    );
    let status = Command::new("sudo")
        .arg("tee")
        .arg("-a")
        .arg(sudoers_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.take().unwrap().write_all(line.as_bytes())?;
            child.wait()
        })?;
    anyhow::ensure!(status.success(), "writing sudoers entry failed");
    Ok(())
}

fn setup_systemd() -> Result<()> {
    let unit_dir = dirs_home()?.join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let exe = std::env::current_exe()?;
    let unit = format!(
        "[Unit]\nDescription=lab-agent: runs jobs pulled from git, reports progress through git\n\n\
         [Service]\nExecStart={exe} run\nRestart=on-failure\nRestartSec=5\nStartLimitIntervalSec=0\n\n\
         [Install]\nWantedBy=default.target\n",
        exe = exe.display()
    );
    std::fs::write(unit_dir.join("lab-agent.service"), unit)?;

    Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()?;
    Command::new("systemctl")
        .args(["--user", "enable", "--now", "lab-agent"])
        .status()?;
    let _ = Command::new("loginctl")
        .args(["enable-linger", &std::env::var("USER").unwrap_or_default()])
        .status();
    println!("systemd user service installed and started");
    Ok(())
}

fn dirs_home() -> Result<std::path::PathBuf> {
    Ok(std::path::PathBuf::from(std::env::var("HOME")?))
}
