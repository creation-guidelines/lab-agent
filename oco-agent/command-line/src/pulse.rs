//! The pulse: oco-agent's own sign of life, sent to a dead man's switch outside the machine (such as
//! Healthchecks) after each healthy turn of its loop - at most every five minutes. It comes from the
//! working loop itself, not from a timer, so it stops when the machine, its network, or the agent
//! stalls; the switch then alerts the author. A pulse is the agent's; a beat is a driver's.

use std::path::PathBuf;
use std::time::Duration;

/// Where the pulse goes: OCO_AGENT_PULSE_URL, else the first line of ~/.config/oco-agent/pulse-url
/// (a private file); no pulse if neither is set.
pub fn address() -> Option<String> {
    if let Some(url) = std::env::var("OCO_AGENT_PULSE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
    {
        return Some(url.trim().to_string());
    }
    let file = PathBuf::from(std::env::var_os("HOME")?).join(".config/oco-agent/pulse-url");
    std::fs::read_to_string(file)
        .ok()?
        .lines()
        .next()
        .map(str::trim)
        .filter(|u| u.starts_with("https://"))
        .map(str::to_string)
}

pub struct Pulse {
    url: String,
    agent: ureq::Agent,
    last: u64,
}

impl Pulse {
    pub fn new(url: String) -> Pulse {
        Pulse {
            url,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build(),
            last: 0,
        }
    }

    /// Sends the pulse if five minutes have passed since the last; says why if it could not.
    pub fn send(&mut self, now: u64) -> Option<String> {
        if now.saturating_sub(self.last) < 300 {
            return None;
        }
        match self.agent.get(&self.url).call() {
            Ok(_) => {
                self.last = now;
                None
            }
            Err(e) => Some(format!("the pulse could not be sent: {e}")),
        }
    }
}
