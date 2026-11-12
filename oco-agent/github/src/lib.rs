//! oco-agent on GitHub: work is asked for as files in a repository (under `oco-agent/jobs`), and
//! drivers' beats are kept as the repository's Actions variables, so a beat writes no commit. Only
//! this crate knows GitHub; the token travels in a header, never in an address.

pub mod asking;

pub use asking::Asking;
use base64::Engine;
use oco_agent_core::journal::{Moment, Trouble};
use oco_agent_core::leases::Beats;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// Where asked-for work lives in the repository.
pub const ASKED: &str = "oco-agent/jobs";
const BEAT: &str = "OCO_AGENT_BEAT_";
const API: &str = "https://api.github.com";

/// One repository, as oco-agent uses it.
pub struct GitHub {
    pub repo: String,
    branch: String,
    token: String,
    agent: ureq::Agent,
}

fn unreachable(because: impl std::fmt::Display) -> Trouble {
    Trouble::Unreachable {
        because: because.to_string(),
    }
}

/// What GitHub answered, when it was not what was hoped for.
fn said(e: ureq::Error) -> Trouble {
    match e {
        ureq::Error::Status(code, response) => {
            let message = response
                .into_json::<Value>()
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .unwrap_or_default();
            unreachable(format!("GitHub answered {code}: {message}"))
        }
        other => unreachable(format!("GitHub could not be reached: {other}")),
    }
}

/// The variable that keeps a driver's beat.
fn variable(driver: &str) -> String {
    let name: String = driver
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{BEAT}{name}")
}

impl GitHub {
    pub fn new(repo: &str, token: String) -> GitHub {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .build();
        GitHub {
            repo: repo.to_string(),
            branch: "main".into(),
            token,
            agent,
        }
    }

    fn call(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(method, &format!("{API}/repos/{}/{path}", self.repo))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
    }

    /// The names of every job asked for.
    pub fn asked(&self) -> Result<Vec<String>, Trouble> {
        match self
            .call("GET", &format!("contents/{ASKED}?ref={}", self.branch))
            .call()
        {
            Ok(r) => {
                let listing: Value = r.into_json().map_err(unreachable)?;
                Ok(listing
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f["name"].as_str()?.strip_suffix(".sh").map(str::to_string))
                    .collect())
            }
            Err(ureq::Error::Status(404, _)) => Ok(Vec::new()),
            Err(e) => Err(said(e)),
        }
    }

    /// The job asked for as `job`.
    pub fn asking(&self, job: &str) -> Result<Asking, Trouble> {
        let text = self
            .call(
                "GET",
                &format!("contents/{ASKED}/{job}.sh?ref={}", self.branch),
            )
            .set("Accept", "application/vnd.github.raw")
            .call()
            .map_err(said)?
            .into_string()
            .map_err(unreachable)?;
        Asking::read(job, &text).map_err(|because| Trouble::Refused {
            job: job.into(),
            because,
        })
    }

    /// Asks for a job by writing its file; refused if a job of that name was ever asked for.
    pub fn ask(&self, asking: &Asking) -> Result<(), Trouble> {
        let content = base64::engine::general_purpose::STANDARD.encode(asking.to_text());
        let body = json!({ "message": format!("asked: {} (by {})", asking.job, asking.by), "content": content, "branch": self.branch });
        match self
            .call("PUT", &format!("contents/{ASKED}/{}.sh", asking.job))
            .send_json(body)
        {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(422, _)) => Err(Trouble::Refused {
                job: asking.job.clone(),
                because: "it cannot be asked for again: it was asked for already".into(),
            }),
            Err(e) => Err(said(e)),
        }
    }
}

fn beat_of(value: &str) -> Option<(String, Moment)> {
    let v: Value = serde_json::from_str(value).ok()?;
    Some((v["driver"].as_str()?.to_string(), v["at"].as_u64()?))
}

impl Beats for GitHub {
    fn heard(&self) -> Result<HashMap<String, Moment>, Trouble> {
        let listing: Value = self
            .call("GET", "actions/variables?per_page=30")
            .call()
            .map_err(said)?
            .into_json()
            .map_err(unreachable)?;
        Ok(listing["variables"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["name"].as_str().is_some_and(|n| n.starts_with(BEAT)))
            .filter_map(|v| beat_of(v["value"].as_str()?))
            .collect())
    }

    fn beat(&mut self, driver: &str, at: Moment) -> Result<(), Trouble> {
        let name = variable(driver);
        let value = json!({ "driver": driver, "at": at }).to_string();
        match self
            .call("GET", &format!("actions/variables/{name}"))
            .call()
        {
            Ok(r) => {
                let known: Value = r.into_json().map_err(unreachable)?;
                // a beat never goes back: an older one than what is known is not written
                if known["value"]
                    .as_str()
                    .and_then(beat_of)
                    .is_some_and(|(_, last)| last >= at)
                {
                    return Ok(());
                }
                self.call("PATCH", &format!("actions/variables/{name}"))
                    .send_json(json!({ "name": name, "value": value }))
                    .map(|_| ())
                    .map_err(said)
            }
            Err(ureq::Error::Status(404, _)) => self
                .call("POST", "actions/variables")
                .send_json(json!({ "name": name, "value": value }))
                .map(|_| ())
                .map_err(said),
            Err(e) => Err(said(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drivers_name_becomes_a_variables_name() {
        assert_eq!(variable("claude"), "OCO_AGENT_BEAT_CLAUDE");
        assert_eq!(variable("ann-lee.2"), "OCO_AGENT_BEAT_ANN_LEE_2");
    }

    #[test]
    fn a_beat_reads_back_from_its_value() {
        assert_eq!(
            beat_of(r#"{"driver":"claude","at":42}"#),
            Some(("claude".into(), 42))
        );
        assert_eq!(beat_of("not a beat"), None);
    }
}
