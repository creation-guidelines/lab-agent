//! Desktop popups beside ntfy, so a message reaches the user at the machine at once.
//!
//! KDE, GNOME, XFCE and the small daemons used with tiling window managers (mako, dunst, swaync)
//! all implement the freedesktop notification spec, which `notify-send` speaks. Where nothing
//! answers on the bus, a standalone window takes its place (zenity, else kdialog). Priorities
//! follow ntfy's: info is a normal popup, urgent is critical (it stays until dismissed).

use crate::notify::Level;
use std::process::{Command, Stdio};

/// The freedesktop urgency for a level; the inbox carries the user's own words, so no popup.
pub fn urgency(level: Level) -> Option<&'static str> {
    match level {
        Level::Info => Some("normal"),
        Level::Urgent => Some("critical"),
        Level::Inbox => None,
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
        if quiet(&service).status().is_ok_and(|s| s.success()) {
            return;
        }
    }
    // no notification service answered: the first standalone window that starts
    for window in tries {
        if quiet(&window).spawn().is_ok() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priorities_follow_the_channels() {
        assert_eq!(urgency(Level::Info), Some("normal"));
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
        assert!(attempts(Level::Info, "t", "b")[1].contains(&"--timeout".to_string()));
    }
}
