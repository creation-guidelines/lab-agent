//! Notifications on the user's phone through [ntfy](https://ntfy.sh), or a server of their own.
//!
//! Two channels, so the phone can treat them differently: **info** is progress (a normal
//! notification) and **urgent** means the user is needed (set it to maximum priority in the app
//! so it rings through Do Not Disturb).
//!
//! Every user gets their own channels: `notify-setup` makes two random topic names (120 bits each)
//! on their machine. The names are the secret - on ntfy.sh anyone who knows a topic can read it -
//! so they live only in `notify.toml` (mode 0600) beside the config, never in a flag, a log, an
//! error message or a repo. Messages are sent from the machine that runs the agent, so whoever
//! asks for a notification (an AI agent, a script) never needs the secret.

use crate::contract::{
    Contract, ENV_NTFY_INBOX_TOPIC, ENV_NTFY_INFO_TOPIC, ENV_NTFY_SERVER, ENV_NTFY_TOKEN,
    ENV_NTFY_URGENT_TOPIC,
};
use anyhow::{anyhow, ensure, Context, Result};
use argenv_resolve::*;
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// Which channel a message goes to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Progress: a normal notification.
    Info,
    /// The user is needed: something failed, or a decision is waiting.
    Urgent,
    /// The conversation channel: to the agent, from the phone or this machine.
    Inbox,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NotifyConfig {
    pub server: String,
    pub info_topic: String,
    pub urgent_topic: String,
    /// Where the user writes to the agent (from the ntfy app). Older setups have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_topic: Option<String>,
    /// For a server that needs a login. ntfy.sh with random topics needs none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

pub const DEFAULT_SERVER: &str = "https://ntfy.sh";

pub fn notify_path() -> PathBuf {
    crate::config::config_path().with_file_name("notify.toml")
}

/// The environment wins (a sandbox or CI with no file), then `notify.toml`. `None` means
/// notifications are not set up here, which is not an error.
pub fn load() -> Option<NotifyConfig> {
    if let Ok(info) = std::env::var(ENV_NTFY_INFO_TOPIC) {
        return Some(NotifyConfig {
            server: std::env::var(ENV_NTFY_SERVER).unwrap_or_else(|_| DEFAULT_SERVER.into()),
            urgent_topic: std::env::var(ENV_NTFY_URGENT_TOPIC).unwrap_or_else(|_| info.clone()),
            info_topic: info,
            inbox_topic: std::env::var(ENV_NTFY_INBOX_TOPIC)
                .ok()
                .filter(|t| !t.is_empty()),
            token: std::env::var(ENV_NTFY_TOKEN).ok().filter(|t| !t.is_empty()),
        });
    }
    load_file()
}

fn load_file() -> Option<NotifyConfig> {
    toml::from_str(&std::fs::read_to_string(notify_path()).ok()?).ok()
}

fn save(cfg: &NotifyConfig) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = notify_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("writing {}", path.display()))?;
    file.write_all(toml::to_string_pretty(cfg)?.as_bytes())?;
    // mode() applies only when the file is created: make an existing one private too.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Sends messages. Cheap to build; holds no connection.
#[derive(Clone)]
pub struct Notifier {
    cfg: NotifyConfig,
    agent: ureq::Agent,
}

impl Notifier {
    pub fn new(cfg: NotifyConfig) -> Self {
        Notifier {
            cfg,
            agent: ureq::AgentBuilder::new().build(),
        }
    }

    /// Sends `message` to the channel for `level`. An error never contains the topic.
    pub fn send(&self, level: Level, title: &str, message: &str) -> Result<()> {
        let (topic, priority, tags) = match level {
            Level::Info => (&self.cfg.info_topic, "3", "robot"),
            Level::Urgent => (&self.cfg.urgent_topic, "5", "rotating_light"),
            Level::Inbox => (
                self.cfg.inbox_topic.as_ref().ok_or_else(|| {
                    anyhow!("there is no inbox channel here yet: run `lab-agent notify-setup`")
                })?,
                "3",
                "speech_balloon",
            ),
        };
        let url = format!("{}/{topic}", self.cfg.server.trim_end_matches('/'));
        let mut request = self
            .agent
            .post(&url)
            .timeout(Duration::from_secs(10))
            .set("Title", &ascii(title))
            .set("Priority", priority)
            .set("Tags", tags);
        if let Some(token) = &self.cfg.token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        request.send_string(message).map_err(|e| {
            let why = match &e {
                ureq::Error::Status(code, _) => format!("the server answered {code}"),
                ureq::Error::Transport(t) => t.kind().to_string(),
            };
            anyhow!("could not send a notification on the {level:?} channel: {why}")
        })?;
        Ok(())
    }
}

/// A link the ntfy Android app opens itself, subscribing to the topic (an `https://` link opens the
/// web app instead: Android cannot hand those to an app reliably).
fn deep_link(server: &str, topic: &str, display: &str) -> String {
    let server = server.trim_end_matches('/');
    let (host, secure) = match server.strip_prefix("https://") {
        Some(host) => (host, true),
        None => (server.strip_prefix("http://").unwrap_or(server), false),
    };
    let display: String = display
        .chars()
        .map(|c| match c {
            ' ' => "+".to_string(),
            c if c.is_ascii_alphanumeric() || "-_.".contains(c) => c.to_string(),
            c => c.to_string().bytes().map(|b| format!("%{b:02X}")).collect(),
        })
        .collect();
    let mut link = format!("ntfy://{host}/{topic}?display={display}");
    if !secure {
        link.push_str("&secure=false");
    }
    link
}

/// `text` as a QR code drawn with block characters, to scan with the phone's camera.
fn qr(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(
        // Drawn for a dark terminal, the way qrencode does it: a block is a *light* module, so on a
        // dark background the code has the polarity a phone camera expects.
        code.render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .quiet_zone(true)
            .build(),
    )
}

/// One message the user sent to the inbox channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    pub id: String,
    pub time: i64,
    pub text: String,
}

/// New messages on the inbox channel since message `since` (all cached ones when `None`), oldest
/// first. ntfy answers with one JSON object per line; only `message` events carry text.
pub fn poll_inbox(cfg: &NotifyConfig, since: Option<&str>) -> Result<Vec<Incoming>> {
    let Some(topic) = &cfg.inbox_topic else {
        return Ok(Vec::new());
    };
    let url = format!(
        "{}/{topic}/json?poll=1&since={}",
        cfg.server.trim_end_matches('/'),
        since.unwrap_or("all")
    );
    let mut request = ureq::get(&url).timeout(Duration::from_secs(10));
    if let Some(token) = &cfg.token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let body = request
        .call()
        .map_err(|e| {
            anyhow!(
                "could not read the inbox channel: {}",
                match &e {
                    ureq::Error::Status(code, _) => format!("the server answered {code}"),
                    ureq::Error::Transport(t) => t.kind().to_string(),
                }
            )
        })?
        .into_string()?;
    Ok(parse_ndjson(&body))
}

fn parse_ndjson(body: &str) -> Vec<Incoming> {
    body.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v["event"] == "message")
        .filter_map(|v| {
            Some(Incoming {
                id: v["id"].as_str()?.to_string(),
                time: v["time"].as_i64().unwrap_or(0),
                text: v["message"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// HTTP headers carry ASCII: anything else in a title is shown as `?` rather than failing.
fn ascii(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

/// A random topic name: 120 bits from the system, as 24 base32 letters, after a readable prefix.
fn random_topic(kind: &str) -> Result<String> {
    let mut bytes = [0u8; 15];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let (mut acc, mut bits, mut out) = (0u32, 0u32, String::new());
    for byte in bytes {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    Ok(format!("labagent-{kind}-{out}"))
}

/// `lab-agent notify [--urgent] [--title T] [MESSAGE]`: send now; the message may come on stdin.
pub fn send_cli(resolved: &Resolution) -> Result<()> {
    let cfg = load().ok_or_else(|| {
        anyhow!("notifications are not set up on this machine: run `lab-agent notify-setup`")
    })?;
    let message = match Contract::NOTIFY_MESSAGE.get_from_or_default(resolved) {
        Some(m) => m,
        None => std::io::read_to_string(std::io::stdin())?,
    };
    ensure!(!message.trim().is_empty(), "nothing to send");
    let urgent = Contract::NOTIFY_URGENT
        .get_from_or_default(resolved)
        .unwrap_or(false);
    let title = Contract::NOTIFY_TITLE
        .get_from_or_default(resolved)
        .unwrap_or_else(|| "lab-agent".to_string());
    let level = if urgent { Level::Urgent } else { Level::Info };
    Notifier::new(cfg).send(level, &title, message.trim())
}

/// `lab-agent notify-setup`, also offered at the end of `connect`: makes this user's two channels,
/// keeps them private, sends a test to each and says what to do on the phone. Re-running it keeps
/// the existing channels unless asked not to. No restart is needed: the agent reads the file each
/// time it sends.
pub fn setup() -> Result<()> {
    let interactive = std::io::stdin().is_terminal();
    let existing = load_file();
    println!(
        "\nPhone notifications through ntfy: an `info` channel for progress, and an `urgent` one \
         for when you are needed.\n"
    );
    let server_default = existing
        .as_ref()
        .map_or(DEFAULT_SERVER.to_string(), |c| c.server.clone());
    let server = if interactive {
        crate::connect::ask("ntfy server", &server_default)?
    } else {
        server_default
    };
    let server = server.trim().trim_end_matches('/').to_string();
    let keep = match &existing {
        Some(c) if c.server == server => {
            !interactive
                || crate::connect::ask("keep your existing channels? (y/n)", "y")?
                    .to_lowercase()
                    .starts_with('y')
        }
        _ => false,
    };
    let (info_topic, urgent_topic, kept_inbox) = match (&existing, keep) {
        (Some(c), true) => (
            c.info_topic.clone(),
            c.urgent_topic.clone(),
            c.inbox_topic.clone(),
        ),
        _ => (random_topic("info")?, random_topic("urgent")?, None),
    };
    // The inbox is newer than the other two: an existing setup gets one added, keeping the rest.
    let inbox_topic = Some(match kept_inbox {
        Some(t) => t,
        None => random_topic("inbox")?,
    });
    let token = if interactive && server != DEFAULT_SERVER {
        let t = rpassword::prompt_password("access token for that server (blank: none): ")?;
        (!t.trim().is_empty()).then(|| t.trim().to_string())
    } else {
        existing.as_ref().and_then(|c| c.token.clone())
    };
    let cfg = NotifyConfig {
        server,
        info_topic,
        urgent_topic,
        inbox_topic,
        token,
    };
    save(&cfg)?;
    println!(
        "saved to {} (readable by you only)",
        notify_path().display()
    );

    let notifier = Notifier::new(cfg.clone());
    for (level, name) in [(Level::Info, "info"), (Level::Urgent, "urgent")] {
        match notifier.send(
            level,
            &format!("lab-agent: the {name} channel works"),
            "If you can read this on your phone, this channel is set up.",
        ) {
            Ok(()) => println!("test message sent to the {name} channel"),
            Err(e) => println!("could not send the test to the {name} channel: {e:#}"),
        }
    }
    println!(
        "\nOn your phone (ntfy app installed): scan each code with the camera, or open the link on"
    );
    println!("the phone - the app subscribes by itself. Do not use the + button: it suggests a new name.");
    let inbox = cfg.inbox_topic.clone().unwrap_or_default();
    for (name, topic) in [
        ("info", &cfg.info_topic),
        ("urgent", &cfg.urgent_topic),
        ("inbox", &inbox),
    ] {
        let link = deep_link(&cfg.server, topic, &format!("lab-agent {name}"));
        println!(
            "\n{name}:\n{}{link}",
            qr(&link).map(|q| q + "\n").unwrap_or_default()
        );
    }
    println!(
        "\nThe inbox is how you talk to the agent while it works: open \"lab-agent inbox\" in the app and\n\
         type. It reads new messages at natural pauses, and you get a receipt on the info channel.\n\
         \nTo let urgent messages ring through Do Not Disturb, in Android: Settings > Apps > ntfy >\n\
         Notifications > the \"Max priority\" category > allow it to override Do Not Disturb.\n\n\
         These two names are the secret: anyone who knows one can read it, so keep them to yourself.\n\
         On the public ntfy.sh, messages are not encrypted; for that, use your own ntfy server or one\n\
         with access tokens.\n\nSend from a script or a job:\n  lab-agent notify \"text\"\n  \
         lab-agent notify --urgent \"text\"\n  lab-agent submit --notify job.sh   (tells you how the job ended)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_topic_is_long_random_and_url_safe() {
        let a = random_topic("info").unwrap();
        let b = random_topic("info").unwrap();
        assert_ne!(a, b);
        let random = a
            .strip_prefix("labagent-info-")
            .expect("the readable prefix");
        assert_eq!(random.len(), 24, "120 bits as base32");
        assert!(random
            .bytes()
            .all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c)));
    }

    #[test]
    fn the_phone_link_opens_the_app_not_the_web() {
        assert_eq!(
            deep_link("https://ntfy.sh/", "labagent-info-x", "lab-agent info"),
            "ntfy://ntfy.sh/labagent-info-x?display=lab-agent+info"
        );
        assert_eq!(
            deep_link("http://ntfy.home:8080", "t", "a&b"),
            "ntfy://ntfy.home:8080/t?display=a%26b&secure=false"
        );
    }

    #[test]
    fn the_printed_code_scans_back_to_the_link_on_a_dark_terminal() {
        let link = "ntfy://ntfy.sh/labagent-info-abcdefghijklmnopqrstuvwx?display=lab-agent+info";
        let printed = qr(link).unwrap();
        // What a dark terminal shows: a block character is lit, a space stays dark, and each
        // character is two pixels high (top half, bottom half). Scale it up and decode it the
        // way a camera would.
        let rows: Vec<Vec<char>> = printed.lines().map(|l| l.chars().collect()).collect();
        let (w, h, scale) = (rows[0].len(), rows.len() * 2, 6);
        let lit = |x: usize, y: usize| -> bool {
            match rows[y / 2].get(x).copied().unwrap_or(' ') {
                '\u{2588}' => true,
                '\u{2580}' => y % 2 == 0,
                '\u{2584}' => y % 2 == 1,
                _ => false,
            }
        };
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w * scale, h * scale, |x, y| {
            if lit(x / scale, y / scale) {
                255
            } else {
                0
            }
        });
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "exactly one code must be found");
        let (_, content) = grids[0].decode().expect("the code decodes");
        assert_eq!(content, link);
    }

    #[test]
    fn only_message_events_are_messages() {
        let body = concat!(
            r#"{"id":"o1","time":1,"event":"open","topic":"t"}"#,
            "\n",
            r#"{"id":"m1","time":10,"event":"message","topic":"t","message":"stop the flash"}"#,
            "\n",
            r#"{"id":"k1","time":11,"event":"keepalive","topic":"t"}"#,
            "\n",
            r#"{"id":"m2","time":12,"event":"message","topic":"t","message":"ok go on"}"#,
            "\n"
        );
        let got = parse_ndjson(body);
        assert_eq!(
            got.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["stop the flash", "ok go on"]
        );
        assert_eq!(got[0].id, "m1");
    }

    #[test]
    fn an_old_setup_without_an_inbox_still_loads() {
        let old = "server = \"https://ntfy.sh\"\ninfo_topic = \"a\"\nurgent_topic = \"b\"\n";
        let cfg: NotifyConfig = toml::from_str(old).unwrap();
        assert_eq!(cfg.inbox_topic, None);
    }

    #[test]
    fn an_inbox_message_goes_to_the_inbox_topic() {
        let (server, seen) = capture();
        let n = Notifier::new(NotifyConfig {
            server,
            info_topic: "t-info".into(),
            urgent_topic: "t-urgent".into(),
            inbox_topic: Some("t-inbox".into()),
            token: None,
        });
        n.send(Level::Inbox, "you", "take a break").unwrap();
        let req = seen.lock().unwrap().clone();
        assert!(req.starts_with("POST /t-inbox "), "{req}");
        assert!(req.ends_with("take a break"));
    }

    #[test]
    fn without_an_inbox_channel_sending_says_how_to_add_one() {
        let n = Notifier::new(NotifyConfig {
            server: "http://127.0.0.1:1".into(),
            info_topic: "a".into(),
            urgent_topic: "b".into(),
            inbox_topic: None,
            token: None,
        });
        let err = n.send(Level::Inbox, "you", "hi").unwrap_err().to_string();
        assert!(err.contains("notify-setup"), "{err}");
    }

    #[test]
    fn a_title_becomes_header_safe() {
        assert_eq!(ascii("build ok: Привіт\n"), "build ok: ???????");
    }

    /// Serves one request and hands back what it received.
    fn capture() -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let out = seen.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // One read() is not one request: the body can arrive in a later packet (it does on
            // CI). Read to the end of the headers, then to the length they declare.
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if got.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            *out.lock().unwrap() = String::from_utf8_lossy(&got).to_string();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .ok();
        });
        (format!("http://{addr}"), seen)
    }

    #[test]
    fn urgent_goes_to_the_urgent_topic_at_top_priority() {
        let (server, seen) = capture();
        let n = Notifier::new(NotifyConfig {
            server,
            info_topic: "t-info".into(),
            urgent_topic: "t-urgent".into(),
            inbox_topic: None,
            token: Some("secret-token".into()),
        });
        n.send(Level::Urgent, "phone stuck", "come and look")
            .unwrap();
        let req = seen.lock().unwrap().clone();
        assert!(req.starts_with("POST /t-urgent "), "{req}");
        let lower = req.to_lowercase();
        assert!(
            lower.contains("\r\npriority: 5\r\n") && lower.contains("\r\ntitle: phone stuck\r\n"),
            "{req}"
        );
        assert!(lower.contains("authorization: bearer secret-token"));
        assert!(req.ends_with("come and look"));
    }

    #[test]
    fn a_failure_never_names_the_topic() {
        let n = Notifier::new(NotifyConfig {
            server: "http://127.0.0.1:1".into(), // nothing listens there
            info_topic: "labagent-info-topsecret".into(),
            urgent_topic: "x".into(),
            inbox_topic: None,
            token: None,
        });
        let err = n.send(Level::Info, "t", "m").unwrap_err().to_string();
        assert!(!err.contains("topsecret"), "{err}");
    }
}
