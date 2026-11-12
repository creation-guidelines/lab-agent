//! The voice on a phone, through ntfy. What is said to a driver who has a channel on this machine is
//! posted to their topic - urgent words to the urgent one, which the phone lets ring through - and
//! what they answer on their own topic is heard. The topics' names are the secret: they live only in
//! ~/.config/oco-agent/channels.toml (a private file) on this machine, never in a flag, a log or a
//! repository, so whoever says something never needs them. Only this file knows ntfy.

use oco_agent_core::saying::{Said, Voice};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// How one driver is reached, and heard.
#[derive(serde::Deserialize, Clone)]
struct Channel {
    server: String,
    info: String,
    urgent: String,
    hears: Option<String>,
}

/// Something a driver said on their own topic.
pub struct Heard {
    pub id: String,
    pub at: u64,
    pub text: String,
}

pub struct Ntfy {
    channels: HashMap<String, Channel>,
    agent: ureq::Agent,
}

/// The channels file: OCO_AGENT_CHANNELS, else ~/.config/oco-agent/channels.toml.
fn file() -> Option<PathBuf> {
    match std::env::var_os("OCO_AGENT_CHANNELS") {
        Some(path) => Some(path.into()),
        None => {
            Some(PathBuf::from(std::env::var_os("HOME")?).join(".config/oco-agent/channels.toml"))
        }
    }
}

/// The channels as written, a table for each driver: server, info, urgent and, if they answer, hears.
fn read(text: &str) -> Result<HashMap<String, Channel>, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

/// A title the server takes: header values are plain ASCII.
fn plain(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

impl Ntfy {
    /// The voice, if this machine has channels; says why not if the file cannot be read.
    pub fn open() -> Result<Option<Ntfy>, String> {
        let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) else {
            return Ok(None);
        };
        let channels = read(&text)?;
        Ok(Some(Ntfy {
            channels,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(15))
                .build(),
        }))
    }

    /// The drivers who answer on a topic of their own.
    pub fn who_answer(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .channels
            .iter()
            .filter(|(_, c)| c.hears.is_some())
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// What `driver` said on their own topic since `since` (a message's id, or a moment in seconds).
    pub fn hear(&self, driver: &str, since: &str) -> Result<Vec<Heard>, String> {
        let Some(channel) = self.channels.get(driver) else {
            return Ok(Vec::new());
        };
        let Some(topic) = &channel.hears else {
            return Ok(Vec::new());
        };
        let url = format!(
            "{}/{topic}/json?poll=1&since={since}",
            channel.server.trim_end_matches('/')
        );
        let body = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())?;
        Ok(body.lines().filter_map(heard).collect())
    }
}

/// One line of the server's answer, if it is a message.
fn heard(line: &str) -> Option<Heard> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v["event"].as_str() != Some("message") {
        return None;
    }
    Some(Heard {
        id: v["id"].as_str()?.to_string(),
        at: v["time"].as_u64()?,
        text: v["message"].as_str()?.to_string(),
    })
}

impl Voice for Ntfy {
    fn reaches(&self, driver: &str) -> bool {
        self.channels.contains_key(driver)
    }

    fn tell(&mut self, said: &Said) -> Result<(), String> {
        let channel = self.channels.get(&said.to).ok_or("no channel")?;
        let (topic, priority) = if said.urgent {
            (&channel.urgent, "urgent")
        } else {
            (&channel.info, "default")
        };
        let text: String = said.text.chars().take(3000).collect();
        self.agent
            .post(&format!("{}/{topic}", channel.server.trim_end_matches('/')))
            .set("Title", &plain(&said.from))
            .set("Priority", priority)
            .send_string(&text)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_read_a_table_for_each_driver() {
        let channels = read("[ann]\nserver = \"https://example.org\"\ninfo = \"a\"\nurgent = \"b\"\nhears = \"c\"\n\n[bo]\nserver = \"https://example.org\"\ninfo = \"d\"\nurgent = \"e\"\n").unwrap();
        let voice = Ntfy {
            channels,
            agent: ureq::agent(),
        };
        assert!(voice.reaches("ann") && voice.reaches("bo") && !voice.reaches("claude"));
        assert_eq!(
            voice.who_answer(),
            ["ann"],
            "bo has no topic of their own to answer on"
        );
    }

    #[test]
    fn only_messages_are_heard_and_a_title_is_plain() {
        assert!(heard(r#"{"id":"x1","time":10,"event":"open","topic":"t"}"#).is_none());
        let h =
            heard(r#"{"id":"x2","time":11,"event":"message","topic":"t","message":"go"}"#).unwrap();
        assert_eq!((h.id.as_str(), h.at, h.text.as_str()), ("x2", 11, "go"));
        assert_eq!(plain("Володимир says"), "????????? says");
    }
}
