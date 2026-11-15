//! oco-agent on GitHub: work is asked for as files in a repository (under `oco-agent/jobs`), and
//! drivers' beats are kept as the repository's Actions variables, so a beat writes no commit. Only
//! this crate knows GitHub; the token travels in a header, never in an address.
//!
//! What a driver needs is to read one repository and to put a file in it. Where its workspace lets the
//! repository be pushed to and its API only be read, both of what a driver says - what it asks, and that
//! it is still here - are said with git alone (`pushed`): the asking as its file, the beat as a small
//! file beside the asked work, which the lab hears as it hears the variables.

pub mod asking;
pub mod pushed;

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
/// Where a driver who speaks with git alone says it is still here: one small file for each, beside the
/// asked work.
pub const BEATS: &str = "oco-agent/beats";
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
    /// When the repository's Actions were last found closed from here: a beat is then said with git, and
    /// the variables are not asked for again for a while.
    no_variables: Cell<Option<Instant>>,
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

/// Whether the request itself was refused - by GitHub, or by the way out to it - and not for a spent
/// budget: a spent budget returns by itself, a refusal does not, and what was to be said goes another way.
fn refused(code: u16, response: &ureq::Response) -> bool {
    code == 403 && response.header("X-RateLimit-Remaining") != Some("0")
}

/// A job of that name was asked for before: it is not asked for again.
fn asked_already(job: &str) -> Trouble {
    Trouble::Refused {
        job: job.into(),
        because: "it cannot be asked for again: it was asked for already".into(),
    }
}

/// The file that keeps the beat of a driver who speaks with git alone.
fn beat_file(driver: &str) -> String {
    let name: String = driver
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{BEATS}/{name}.json")
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
        // a driver's workspace may have its own way out - a proxy named by the environment, which may
        // also be where the credential for the repository is attached: then that way is taken
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .try_proxy_from_env(true)
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
            no_variables: Cell::default(),
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
            // a way out that passes only a repository's files refuses its Actions: there is nothing to
            // list there from here, for a while
            Err(ureq::Error::Status(code, r))
                if refused(code, &r) && path.starts_with("actions/") =>
            {
                self.no_variables.set(Some(Instant::now()));
                Ok(None)
            }
            Err(e) => Err(said(e)),
        }
    }

    /// Whether the repository's Actions were lately found closed from here.
    fn variables_closed(&self) -> bool {
        self.no_variables
            .get()
            .is_some_and(|since| since.elapsed() < Duration::from_secs(600))
    }

    /// Says a driver's beat as a variable - no commit; false where the repository's Actions are closed
    /// from here, and the beat is to be said another way.
    fn beat_as_variable(&self, driver: &str, at: Moment, value: &str) -> Result<bool, Trouble> {
        if self.variables_closed() {
            return Ok(false);
        }
        let name = variable(driver);
        let written = match self
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
                    return Ok(true);
                }
                self.call("PATCH", &format!("actions/variables/{name}"))
                    .send_json(json!({ "name": name, "value": value }))
            }
            Err(ureq::Error::Status(404, _)) => self
                .call("POST", "actions/variables")
                .send_json(json!({ "name": name, "value": value })),
            Err(e) => Err(e),
        };
        match written {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(code, r)) if refused(code, &r) => {
                self.no_variables.set(Some(Instant::now()));
                Ok(false)
            }
            Err(e) => Err(said(e)),
        }
    }

    /// The names of every job asked for, read from the git tree of the folder: unlike the contents
    /// listing, which silently stops at 1,000 entries (how lab-agent came to replay finished jobs), the
    /// tree says when it is cut short - and then nothing is taken in until it is whole.
    pub fn asked(&self) -> Result<Vec<String>, Trouble> {
        let listing = self
            .listing(&format!("git/trees/{}:{ASKED}", self.branch))?
            .unwrap_or(Value::Null);
        if listing["truncated"].as_bool() == Some(true) {
            return Err(unreachable("GitHub's listing of the asked work is cut short; nothing is taken in until it is whole"));
        }
        Ok(listing["tree"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["path"].as_str()?.strip_suffix(".sh").map(str::to_string))
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
            Err(ureq::Error::Status(422, _)) => Err(asked_already(&asking.job)),
            // the repository's API takes no write from here: the asking is said with git alone
            Err(ureq::Error::Status(code, r)) if refused(code, &r) => pushed::push(
                &self.repo,
                &self.branch,
                &format!("{ASKED}/{}.sh", asking.job),
                asking.to_text().as_bytes(),
                &format!("asked: {} (by {})", asking.job, asking.by),
                true,
            )
            .map_err(|not| match not {
                pushed::NotPushed::AlreadyThere => asked_already(&asking.job),
                pushed::NotPushed::Because(why) => unreachable(why),
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
        self.text_at(path, SPOKEN)
    }

    /// The file at `path` on `branch`, asked for only if it changed since the last answer; nothing, if
    /// there is none.
    fn text_at(&self, path: &str, branch: &str) -> Result<Option<String>, Trouble> {
        let key = format!("contents/{path}?ref={branch}");
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
    /// The beats, wherever drivers said them: as the repository's variables, and as the small files of
    /// drivers who speak with git alone. Of two beats of one driver, the later is the one heard.
    fn heard(&self) -> Result<HashMap<String, Moment>, Trouble> {
        let mut said: Vec<(String, Moment)> = Vec::new();
        if !self.variables_closed() {
            let listing = self
                .listing("actions/variables?per_page=30")?
                .unwrap_or(Value::Null);
            said.extend(
                listing["variables"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|v| v["name"].as_str().is_some_and(|n| n.starts_with(BEAT)))
                    .filter_map(|v| beat_of(v["value"].as_str()?)),
            );
        }
        let files = self
            .listing(&format!("git/trees/{}:{BEATS}", self.branch))?
            .unwrap_or(Value::Null);
        for file in files["tree"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["path"].as_str())
        {
            let text = self.text_at(&format!("{BEATS}/{file}"), &self.branch)?;
            said.extend(text.as_deref().and_then(beat_of));
        }
        let mut heard: HashMap<String, Moment> = HashMap::new();
        for (driver, at) in said {
            let last = heard.entry(driver).or_insert(at);
            *last = (*last).max(at);
        }
        Ok(heard)
    }

    /// Says that `driver` is here: as a variable where the repository's Actions can be written from here,
    /// and otherwise with git alone, as the driver's small file beside the asked work.
    fn beat(&mut self, driver: &str, at: Moment) -> Result<(), Trouble> {
        let value = json!({ "driver": driver, "at": at }).to_string();
        if self.beat_as_variable(driver, at, &value)? {
            return Ok(());
        }
        let path = beat_file(driver);
        // a beat never goes back: an older one than what is known is not written
        if self
            .text_at(&path, &self.branch)?
            .as_deref()
            .and_then(beat_of)
            .is_some_and(|(_, last)| last >= at)
        {
            return Ok(());
        }
        pushed::push(
            &self.repo,
            &self.branch,
            &path,
            value.as_bytes(),
            &format!("{driver} is here"),
            false,
        )
        .map_err(|not| match not {
            pushed::NotPushed::AlreadyThere => unreachable(format!("{path} could not be written")),
            pushed::NotPushed::Because(why) => unreachable(why),
        })
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
    fn a_drivers_name_becomes_a_files_name_beside_the_asked_work() {
        assert_eq!(beat_file("claude"), "oco-agent/beats/claude.json");
        assert_eq!(beat_file("ann-lee.2"), "oco-agent/beats/ann-lee.2.json");
        assert_eq!(beat_file("a b/c"), "oco-agent/beats/a_b_c.json");
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
