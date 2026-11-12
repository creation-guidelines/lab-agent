//! oco-agent on GitHub: work is asked for as files in a repository (under `oco-agent/jobs`), and
//! drivers' beats are kept as the repository's Actions variables, so a beat writes no commit. Only
//! this crate knows GitHub; the token travels in a header, never in an address.

pub mod asking;

pub use asking::Asking;
use base64::Engine;
use oco_agent_core::journal::{Moment, Trouble};
use oco_agent_core::leases::Beats;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Where asked-for work lives in the repository.
pub const ASKED: &str = "oco-agent/jobs";
/// The branch what happened is written to, so the history of asking, on main, stays clean.
pub const SPOKEN: &str = "spoken";
const BEAT: &str = "OCO_AGENT_BEAT_";
const API: &str = "https://api.github.com";

/// One repository, as oco-agent uses it. Listings are asked for conditionally: GitHub answers "not
/// modified" without charging the budget - 5,000 requests an hour, shared by everything that uses the
/// same account - so taking work in costs next to nothing while nothing changes.
pub struct GitHub {
    pub repo: String,
    branch: String,
    token: String,
    agent: ureq::Agent,
    /// Each listing's last answer, by its fingerprint (ETag).
    known: RefCell<HashMap<String, (String, Value)>>,
    /// How much of the hour's budget GitHub last said was left.
    left: Cell<Option<u64>>,
    /// Each spoken file's last known sha, so writing it needs no read first.
    shas: RefCell<HashMap<String, String>>,
    /// Each spoken file's last answer, by its fingerprint.
    texts: RefCell<HashMap<String, (String, String)>>,
    /// Whether the spoken branch is known to exist.
    has_spoken: Cell<bool>,
    /// Listings GitHub said do not exist, and when: not asked again for two minutes (a "not found" is
    /// charged to the budget, and carries no fingerprint to ask conditionally with).
    missing: RefCell<HashMap<String, Instant>>,
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
            known: RefCell::default(),
            left: Cell::default(),
            missing: RefCell::default(),
            shas: RefCell::default(),
            texts: RefCell::default(),
            has_spoken: Cell::default(),
        }
    }

    fn call(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(method, &format!("{API}/repos/{}/{path}", self.repo))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
    }

    /// How much of the hour's budget GitHub last said was left.
    pub fn budget_left(&self) -> Option<u64> {
        self.left.get()
    }

    fn note_budget(&self, r: &ureq::Response) {
        if let Some(left) = r
            .header("X-RateLimit-Remaining")
            .and_then(|v| v.parse().ok())
        {
            self.left.set(Some(left));
        }
    }

    /// A listing, asked for only if it changed since the last answer; nothing, if there is none.
    fn listing(&self, path: &str) -> Result<Option<Value>, Trouble> {
        if self
            .missing
            .borrow()
            .get(path)
            .is_some_and(|since| since.elapsed() < Duration::from_secs(120))
        {
            return Ok(None);
        }
        let mut request = self.call("GET", path);
        if let Some((etag, _)) = self.known.borrow().get(path) {
            request = request.set("If-None-Match", etag);
        }
        match request.call() {
            Ok(r) if r.status() == 304 => {
                self.note_budget(&r);
                Ok(self.known.borrow().get(path).map(|(_, v)| v.clone()))
            }
            Ok(r) => {
                self.note_budget(&r);
                let etag = r.header("ETag").map(str::to_string);
                let v: Value = r.into_json().map_err(unreachable)?;
                if let Some(etag) = etag {
                    self.known
                        .borrow_mut()
                        .insert(path.to_string(), (etag, v.clone()));
                }
                Ok(Some(v))
            }
            Err(ureq::Error::Status(404, _)) => {
                self.missing
                    .borrow_mut()
                    .insert(path.to_string(), Instant::now());
                Ok(None)
            }
            Err(e) => Err(said(e)),
        }
    }

    /// The names of every job asked for.
    pub fn asked(&self) -> Result<Vec<String>, Trouble> {
        let listing = self
            .listing(&format!("contents/{ASKED}?ref={}", self.branch))?
            .unwrap_or(Value::Null);
        Ok(listing
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["name"].as_str()?.strip_suffix(".sh").map(str::to_string))
            .collect())
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
    /// Makes sure the spoken branch exists, starting it from main if it does not.
    fn spoken_branch(&self) -> Result<(), Trouble> {
        if self.has_spoken.get() {
            return Ok(());
        }
        match self.call("GET", &format!("git/ref/heads/{SPOKEN}")).call() {
            Ok(_) => {}
            Err(ureq::Error::Status(404, _)) => {
                let main: Value = self
                    .call("GET", &format!("git/ref/heads/{}", self.branch))
                    .call()
                    .map_err(said)?
                    .into_json()
                    .map_err(unreachable)?;
                let sha = main["object"]["sha"]
                    .as_str()
                    .ok_or_else(|| unreachable("the main branch has no commit"))?;
                match self
                    .call("POST", "git/refs")
                    .send_json(json!({ "ref": format!("refs/heads/{SPOKEN}"), "sha": sha }))
                {
                    Ok(_) | Err(ureq::Error::Status(422, _)) => {}
                    Err(e) => return Err(said(e)),
                }
            }
            Err(e) => return Err(said(e)),
        }
        self.has_spoken.set(true);
        Ok(())
    }

    fn sha_of(&self, path: &str) -> Result<Option<String>, Trouble> {
        match self
            .call("GET", &format!("contents/{path}?ref={SPOKEN}"))
            .call()
        {
            Ok(r) => Ok(r.into_json::<Value>().map_err(unreachable)?["sha"]
                .as_str()
                .map(str::to_string)),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(said(e)),
        }
    }

    /// Writes `content` at `path` on the spoken branch: one commit.
    pub fn speak(&self, path: &str, content: &[u8], message: &str) -> Result<(), Trouble> {
        self.spoken_branch()?;
        for attempt in 0..2 {
            let known = self.shas.borrow().get(path).cloned();
            let sha = match known {
                Some(sha) if attempt == 0 => Some(sha),
                _ => self.sha_of(path)?,
            };
            let mut body = json!({ "message": message, "content": base64::engine::general_purpose::STANDARD.encode(content), "branch": SPOKEN });
            if let Some(sha) = sha {
                body["sha"] = json!(sha);
            }
            match self
                .call("PUT", &format!("contents/{path}"))
                .send_json(body)
            {
                Ok(r) => {
                    let written: Value = r.into_json().map_err(unreachable)?;
                    if let Some(sha) = written["content"]["sha"].as_str() {
                        self.shas
                            .borrow_mut()
                            .insert(path.to_string(), sha.to_string());
                    }
                    return Ok(());
                }
                // the file changed under us: read its sha and try once more
                Err(ureq::Error::Status(409 | 422, _)) if attempt == 0 => continue,
                Err(e) => return Err(said(e)),
            }
        }
        Err(unreachable(format!("{path} could not be written")))
    }

    /// What was spoken at `path`, asked for only if it changed since the last answer; nothing, if nothing was.
    pub fn spoken(&self, path: &str) -> Result<Option<String>, Trouble> {
        let key = format!("contents/{path}?ref={SPOKEN}");
        let mut request = self
            .call("GET", &key)
            .set("Accept", "application/vnd.github.raw");
        if let Some((etag, _)) = self.texts.borrow().get(&key) {
            request = request.set("If-None-Match", etag);
        }
        match request.call() {
            Ok(r) if r.status() == 304 => {
                self.note_budget(&r);
                Ok(self.texts.borrow().get(&key).map(|(_, text)| text.clone()))
            }
            Ok(r) => {
                self.note_budget(&r);
                let etag = r.header("ETag").map(str::to_string);
                let text = r.into_string().map_err(unreachable)?;
                if let Some(etag) = etag {
                    self.texts.borrow_mut().insert(key, (etag, text.clone()));
                }
                Ok(Some(text))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
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
        let listing = self
            .listing("actions/variables?per_page=30")?
            .unwrap_or(Value::Null);
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
