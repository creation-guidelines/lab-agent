//! Desktop popups beside ntfy, so a message reaches the user at the machine at once.
//!
//! KDE, GNOME, XFCE and the small daemons used with tiling window managers (mako, dunst, swaync)
//! all implement the freedesktop notification spec, which `notify-send` speaks. Where nothing
//! answers on the bus, a standalone window takes its place (zenity, else kdialog). Priorities
//! follow ntfy's: info is a normal popup, urgent is critical (it stays until dismissed).

use crate::notify::Level;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The freedesktop urgency for a level: only what needs the user pops up (urgent, as critical);
/// info goes to the phone and the board, and the inbox carries the user's own words.
pub fn urgency(level: Level) -> Option<&'static str> {
    match level {
        Level::Urgent => Some("critical"),
        Level::Info | Level::Inbox => None,
    }
}

/// The ways to show one popup, best first. The first is the notification service; the others are
/// standalone windows for sessions without one (an urgent one stays until closed).
pub fn attempts(level: Level, title: &str, body: &str) -> Vec<Vec<String>> {
    let Some(urgency) = urgency(level) else {
        return Vec::new();
    };
    let icon = if level == Level::Urgent {
        "dialog-warning"
    } else {
        "dialog-information"
    };
    let mut zenity = vec![
        "zenity", "--info", "--title", title, "--text", body, "--icon", icon,
    ];
    if level != Level::Urgent {
        zenity.extend(["--timeout", "30"]);
    }
    let owned = |v: Vec<&str>| v.into_iter().map(String::from).collect::<Vec<_>>();
    vec![
        owned(vec![
            "notify-send",
            "--app-name",
            "lab-agent",
            "--urgency",
            urgency,
            "--icon",
            icon,
            title,
            body,
        ]),
        owned(zenity),
        owned(vec!["kdialog", "--title", title, "--msgbox", body]),
    ]
}

/// Whether this process runs inside a graphical session (else there is no one to show it to).
fn graphical() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY", "XDG_CURRENT_DESKTOP"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

/// Shows a popup, best effort: the notification service is asked directly (it answers at once);
/// a standalone window is started detached, so a short-lived caller never waits for it.
pub fn popup(level: Level, title: &str, body: &str) {
    if !graphical() {
        return;
    }
    let mut tries = attempts(level, title, body).into_iter();
    let quiet = |argv: &[String]| {
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        c
    };
    if let Some(service) = tries.next() {
        match quiet(&service).status() {
            Ok(s) if s.success() => return,
            Ok(s) => eprintln!(
                "lab-agent: the notification service refused a popup ({s}); opening a window"
            ),
            // no notify-send here: a standalone window is the way
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // it started but its exit could not be read: it was almost surely shown
            Err(e) => {
                eprintln!("lab-agent: could not read notify-send's exit ({e}); assuming the popup was shown");
                return;
            }
        }
    }
    // no notification service: the first standalone window that starts, collected once closed
    for window in tries {
        if let Ok(mut child) = quiet(&window).spawn() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return;
        }
    }
}

/// The command that asks with buttons through the notification service: it stays until clicked and
/// prints the name of the button pressed (here its index), nothing if dismissed.
pub fn ask_argv(question: &str, choices: &[String]) -> Vec<String> {
    let mut argv: Vec<String> = [
        "notify-send",
        "--app-name",
        "lab-agent",
        "--urgency",
        "critical",
        "--icon",
        "dialog-question",
        "--wait",
        "--expire-time",
        "0",
    ]
    .map(String::from)
    .to_vec();
    argv.extend(
        choices
            .iter()
            .enumerate()
            .map(|(i, c)| format!("--action={i}={c}")),
    );
    argv.extend(["lab-agent asks".to_string(), question.to_string()]);
    argv
}

/// The choice named by what notify-send printed (the pressed button's index).
pub fn chosen(printed: &str, choices: &[String]) -> Option<String> {
    printed
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|i| choices.get(i).cloned())
}

/// Runs `argv`, waiting at most `within`: its exit code and what it printed, or None on timeout
/// (it is then stopped) or when it cannot start.
fn run_for(argv: &[String], within: Duration) -> Option<(i32, String)> {
    use std::io::Read;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + within;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let mut out = String::new();
            let _ = child.stdout.take()?.read_to_string(&mut out);
            return Some((status.code().unwrap_or(-1), out));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Asks on the desktop and waits for a click: the notification service's buttons first, a zenity
/// window where there is none. None: dismissed, or no answer in time.
pub fn ask(question: &str, choices: &[String], within: Duration) -> Option<String> {
    let started = Instant::now();
    match run_for(&ask_argv(question, choices), within) {
        Some((0, printed)) => return chosen(&printed, choices),
        None if started.elapsed() >= within => return None, // asked, no click in time
        _ => {} // no notification service: a window instead
    }
    let secs = within.as_secs().max(1).to_string();
    let base = |kind: &str| -> Vec<String> {
        [
            "zenity",
            kind,
            "--title",
            "lab-agent asks",
            "--text",
            question,
            "--timeout",
            &secs,
        ]
        .map(String::from)
        .to_vec()
    };
    if choices.len() <= 2 {
        let mut argv = base("--question");
        argv.push(format!("--ok-label={}", choices[0]));
        if let Some(second) = choices.get(1) {
            argv.push(format!("--cancel-label={second}"));
        }
        return match run_for(&argv, within)?.0 {
            0 => Some(choices[0].clone()),
            1 => choices.get(1).cloned(),
            _ => None,
        };
    }
    let mut argv = base("--list");
    argv.extend(["--column".to_string(), "choice".to_string()]);
    argv.extend(choices.iter().cloned());
    let (code, printed) = run_for(&argv, within)?;
    let printed = printed.trim().to_string();
    (code == 0 && choices.contains(&printed)).then_some(printed)
}

/// `lab-agent ask QUESTION [--choices A,B] [--within SECONDS]`: prints the button pressed (exit 0),
/// or "no answer" (exit 2). The question goes into the conversation, the answer into the inbox.
pub fn ask_cli(resolved: &argenv_resolve::Resolution) -> anyhow::Result<i32> {
    use crate::contract::Contract;
    use argenv_resolve::*;
    let question = Contract::ASK_QUESTION
        .get_from_or_default(resolved)
        .ok_or_else(|| anyhow::anyhow!("usage: lab-agent ask \"QUESTION\" [--choices A,B]"))?;
    let choices: Vec<String> = Contract::ASK_CHOICES
        .get_from_or_default(resolved)
        .unwrap_or_else(|| "OK".into())
        .split(',')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    anyhow::ensure!(!choices.is_empty(), "give at least one choice");
    let within = Duration::from_secs(
        Contract::WITHIN
            .get_from_or_default(resolved)
            .unwrap_or(300),
    );
    crate::notify::record_sent(Level::Urgent, "asks", &question);
    let answer = ask(&question, &choices, within);
    if let (Some(a), Some(cfg)) = (&answer, crate::notify::load()) {
        let _ = crate::notify::Notifier::new(cfg).send(
            Level::Inbox,
            "you",
            &format!("[clicked {a}] {question}"),
        );
    }
    Ok(match answer {
        Some(a) => {
            println!("{a}");
            0
        }
        None => {
            println!("no answer");
            2
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priorities_follow_the_channels() {
        assert_eq!(urgency(Level::Info), None, "info does not pop up");
        assert_eq!(urgency(Level::Urgent), Some("critical"));
        assert_eq!(
            urgency(Level::Inbox),
            None,
            "the user's own messages do not pop up"
        );
        assert!(attempts(Level::Inbox, "t", "b").is_empty());
    }

    #[test]
    fn the_service_comes_first_then_standalone_windows() {
        let a = attempts(Level::Urgent, "Listen now", "three beeps");
        let tools: Vec<&str> = a.iter().map(|v| v[0].as_str()).collect();
        assert_eq!(tools, ["notify-send", "zenity", "kdialog"]);
        assert!(a[0].windows(2).any(|w| w == ["--urgency", "critical"]));
        assert!(a[0].ends_with(&["Listen now".to_string(), "three beeps".to_string()]));
        assert!(
            !a[1].contains(&"--timeout".to_string()),
            "an urgent window stays until closed"
        );
    }

    #[test]
    fn asking_offers_each_choice_as_a_button_and_reads_back_the_one_pressed() {
        let choices = vec!["Go".to_string(), "Not yet".to_string()];
        let argv = ask_argv("Ready?", &choices);
        assert!(
            argv.contains(&"--wait".to_string())
                && argv.contains(&"--action=1=Not yet".to_string())
        );
        assert_eq!(argv.last().map(String::as_str), Some("Ready?"));
        assert_eq!(chosen("1\n", &choices), Some("Not yet".into()));
        assert_eq!(chosen("", &choices), None, "dismissed");
        assert_eq!(chosen("7", &choices), None);
    }
}
