use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

/// A small, deliberately plain GitHub REST client: just the handful of calls
/// lab-agent needs (read/write a file, list a directory, check/create a
/// repo). No generated SDK, no async runtime - one struct, one blocking
/// client, kept simple on purpose.
pub struct Client {
    token: String,
    /// The API root: <https://api.github.com>, or a GitHub Enterprise server's.
    base: String,
    agent: ureq::Agent,
}

#[derive(Debug, Clone)]
pub struct FileContent {
    pub content: Vec<u8>,
    pub sha: String,
}

/// GitHub refused because the hourly request budget is spent. The budget is per *user*: every
/// token of the same user draws on it, so a busy session elsewhere can starve this program.
#[derive(Debug)]
pub struct RateLimited {
    /// Unix time at which the budget returns.
    pub reset: u64,
}

impl RateLimited {
    /// How long to wait for the budget to return (a couple of seconds of slack).
    pub fn wait(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.reset.saturating_sub(unix_now()) + 2)
    }
}

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GitHub's hourly API budget is used up (it is per user, shared by every token); it returns in {} min",
            self.wait().as_secs().div_ceil(60)
        )
    }
}

impl std::error::Error for RateLimited {}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// An HTTP failure as an error: a spent budget becomes [`RateLimited`] (so callers can wait for
/// it instead of retrying into a wall); anything else keeps GitHub's own status text.
fn fail(what: &str, e: ureq::Error) -> anyhow::Error {
    if let ureq::Error::Status(code, resp) = &e {
        let spent = resp.header("x-ratelimit-remaining") == Some("0")
            || resp.header("retry-after").is_some();
        if matches!(*code, 403 | 429) && spent {
            let retry_after = resp
                .header("retry-after")
                .and_then(|v| v.parse::<u64>().ok());
            let reset = resp
                .header("x-ratelimit-reset")
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| unix_now() + retry_after.unwrap_or(60));
            return anyhow::Error::new(RateLimited { reset });
        }
    }
    anyhow!("{what}: {e}")
}

/// The answer to a conditional directory listing.
#[derive(Debug)]
pub enum Listed {
    /// There is no such directory.
    Missing,
    /// It is the same as when the ETag was issued.
    Unchanged,
    /// It is new, or changed; `etag` is for the next conditional listing.
    Changed {
        names: Vec<String>,
        etag: Option<String>,
    },
}

/// The answer to a conditional read.
#[derive(Debug)]
pub enum Fetched {
    /// There is no such file.
    Missing,
    /// It is the same as when the ETag was issued.
    Unchanged,
    /// It is new, or changed; `etag` is for the next conditional read.
    Changed {
        file: FileContent,
        etag: Option<String>,
    },
}

impl Client {
    /// A client for GitHub, or for a GitHub Enterprise server when `LAB_AGENT_API_URL` is set.
    pub fn new(token: String) -> Self {
        let base = std::env::var(crate::contract::ENV_API_URL)
            .unwrap_or_else(|_| "https://api.github.com".to_string());
        Client::with_base(token, &base)
    }

    pub fn with_base(token: String, base: &str) -> Self {
        Client {
            token,
            base: base.trim_end_matches('/').to_string(),
            agent: ureq::AgentBuilder::new().build(),
        }
    }

    fn api(&self, tail: &str) -> String {
        format!("{}{tail}", self.base)
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        req.set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "lab-agent")
    }

    pub fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool> {
        let url = self.api(&format!("/repos/{owner}/{repo}"));
        match self.auth(self.agent.get(&url)).call() {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(e) => Err(fail(&format!("checking repo {owner}/{repo}"), e)),
        }
    }

    pub fn create_repo(&self, owner: &str, repo: &str, private: bool) -> Result<()> {
        // Works for both user-owned and org-owned targets.
        let who = self.whoami()?;
        let url = if who == owner {
            self.api("/user/repos")
        } else {
            self.api(&format!("/orgs/{owner}/repos"))
        };
        let body = json!({ "name": repo, "private": private, "auto_init": true });
        self.auth(self.agent.post(&url))
            .send_json(body)
            .map_err(|e| fail("writing to GitHub", e))?;
        Ok(())
    }

    fn whoami(&self) -> Result<String> {
        let resp: Value = self
            .auth(self.agent.get(&self.api("/user")))
            .call()
            .map_err(|e| fail("calling GitHub", e))?
            .into_json()?;
        Ok(resp["login"].as_str().unwrap_or_default().to_string())
    }

    /// Reads one file's content and sha at a ref. None if it doesn't exist.
    pub fn get_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        r#ref: &str,
    ) -> Result<Option<FileContent>> {
        match self.get_file_cond(owner, repo, path, r#ref, None)? {
            Fetched::Changed { file, .. } => Ok(Some(file)),
            Fetched::Missing | Fetched::Unchanged => Ok(None),
        }
    }

    /// [`Client::get_file`], sending the ETag of an earlier read. GitHub answers "not modified"
    /// without a body, and a conditional request answered that way does not count against the
    /// rate limit - which is what makes polling a job's result every second affordable.
    pub fn get_file_cond(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        r#ref: &str,
        etag: Option<&str>,
    ) -> Result<Fetched> {
        let url = self.api(&format!("/repos/{owner}/{repo}/contents/{path}?ref={ref}"));
        let mut req = self.auth(self.agent.get(&url));
        if let Some(tag) = etag {
            req = req.set("If-None-Match", tag);
        }
        match req.call() {
            // ureq reports only 4xx/5xx as errors: a 304 is an Ok response with no body.
            Ok(resp) if resp.status() == 304 => Ok(Fetched::Unchanged),
            Ok(resp) => {
                let etag = resp.header("ETag").map(str::to_string);
                let v: Value = resp.into_json()?;
                let b64 = v["content"].as_str().unwrap_or_default();
                let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                let content = STANDARD.decode(clean)?;
                let sha = v["sha"].as_str().unwrap_or_default().to_string();
                Ok(Fetched::Changed {
                    file: FileContent { content, sha },
                    etag,
                })
            }
            Err(ureq::Error::Status(304, _)) => Ok(Fetched::Unchanged),
            Err(ureq::Error::Status(404, _)) => Ok(Fetched::Missing),
            Err(e) => Err(fail(&format!("reading {owner}/{repo}:{path}@{ref}"), e)),
        }
    }

    /// Lists a directory's entries (name only) at a ref. Empty if missing.
    pub fn list_dir(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        r#ref: &str,
    ) -> Result<Vec<String>> {
        match self.list_dir_cond(owner, repo, path, r#ref, None)? {
            Listed::Changed { names, .. } => Ok(names),
            Listed::Missing | Listed::Unchanged => Ok(vec![]),
        }
    }

    /// [`Client::list_dir`] with an ETag from an earlier listing: when nothing changed GitHub
    /// answers 304 and the request is free against the rate limit, so a daemon can poll all day
    /// without spending the budget it shares with everything else the user does on GitHub.
    pub fn list_dir_cond(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        r#ref: &str,
        etag: Option<&str>,
    ) -> Result<Listed> {
        let url = self.api(&format!("/repos/{owner}/{repo}/contents/{path}?ref={ref}"));
        let mut req = self.auth(self.agent.get(&url));
        if let Some(tag) = etag {
            req = req.set("If-None-Match", tag);
        }
        match req.call() {
            Ok(resp) if resp.status() == 304 => Ok(Listed::Unchanged),
            Ok(resp) => {
                let etag = resp.header("ETag").map(str::to_string);
                let v: Value = resp.into_json()?;
                let names = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| e["name"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Listed::Changed { names, etag })
            }
            Err(ureq::Error::Status(404, _)) => Ok(Listed::Missing),
            Err(e) => Err(fail(&format!("listing {owner}/{repo}:{path}@{ref}"), e)),
        }
    }

    /// Creates or updates one file. Pass the current sha to update, None to
    /// create. Returns the new sha.
    #[allow(clippy::too_many_arguments)]
    pub fn put_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        r#ref: &str,
        content: &[u8],
        message: &str,
        sha: Option<&str>,
    ) -> Result<String> {
        let url = self.api(&format!("/repos/{owner}/{repo}/contents/{path}"));
        let mut body = json!({
            "message": message,
            "content": STANDARD.encode(content),
            "branch": r#ref,
        });
        if let Some(s) = sha {
            body["sha"] = json!(s);
        }
        let resp: Value = self
            .auth(self.agent.put(&url))
            .send_json(body)
            .map_err(|e| fail("writing to GitHub", e))?
            .into_json()?;
        Ok(resp["content"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serves the given raw HTTP responses, one per connection and in order, on a local port.
    fn serve(responses: Vec<String>) -> Client {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        Client::with_base("token".into(), &format!("http://{addr}"))
    }

    fn json(status: &str, body: &str, extra: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn a_304_is_unchanged_not_a_json_error() {
        // The bug this pins: ureq returns a 304 as Ok with an empty body, and parsing that as JSON
        // failed with "EOF while parsing a value" on the first conditional re-read.
        let client = serve(vec![json("304 Not Modified", "", "ETag: \"abc\"\r\n")]);
        let got = client
            .get_file_cond("o", "r", "p", "main", Some("\"abc\""))
            .unwrap();
        assert!(matches!(got, Fetched::Unchanged), "{got:?}");
    }

    #[test]
    fn a_404_is_missing() {
        let client = serve(vec![json(
            "404 Not Found",
            "{\"message\":\"Not Found\"}",
            "",
        )]);
        let got = client.get_file_cond("o", "r", "p", "main", None).unwrap();
        assert!(matches!(got, Fetched::Missing), "{got:?}");
    }

    #[test]
    fn a_200_carries_the_content_and_the_etag() {
        let body = "{\"content\":\"aGk=\\n\",\"sha\":\"s1\"}";
        let client = serve(vec![json("200 OK", body, "ETag: \"v1\"\r\n")]);
        match client.get_file_cond("o", "r", "p", "main", None).unwrap() {
            Fetched::Changed { file, etag } => {
                assert_eq!(file.content, b"hi");
                assert_eq!(file.sha, "s1");
                assert_eq!(etag.as_deref(), Some("\"v1\""));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_spent_budget_is_a_rate_limit_with_its_reset_time() {
        let client = serve(vec![json(
            "403 Forbidden",
            "{\"message\":\"API rate limit exceeded\"}",
            "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: 4102444800\r\n",
        )]);
        let err = client
            .get_file_cond("o", "r", "p", "main", None)
            .unwrap_err();
        let limited = err
            .downcast_ref::<RateLimited>()
            .expect("a RateLimited error");
        assert_eq!(limited.reset, 4_102_444_800);
    }

    #[test]
    fn a_403_with_budget_left_is_just_an_error() {
        let client = serve(vec![json(
            "403 Forbidden",
            "{\"message\":\"Resource not accessible\"}",
            "x-ratelimit-remaining: 4999\r\n",
        )]);
        let err = client
            .get_file_cond("o", "r", "p", "main", None)
            .unwrap_err();
        assert!(err.downcast_ref::<RateLimited>().is_none(), "{err:#}");
    }

    #[test]
    fn an_unchanged_listing_is_free() {
        let client = serve(vec![json("304 Not Modified", "", "ETag: \"d1\"\r\n")]);
        let got = client
            .list_dir_cond("o", "r", "jobs", "main", Some("\"d1\""))
            .unwrap();
        assert!(matches!(got, Listed::Unchanged), "{got:?}");
    }

    #[test]
    fn a_changed_listing_has_names_and_an_etag() {
        let body = "[{\"name\":\"1-a.sh\"},{\"name\":\"archive\"}]";
        let client = serve(vec![json("200 OK", body, "ETag: \"d2\"\r\n")]);
        match client
            .list_dir_cond("o", "r", "jobs", "main", None)
            .unwrap()
        {
            Listed::Changed { names, etag } => {
                assert_eq!(names, vec!["1-a.sh", "archive"]);
                assert_eq!(etag.as_deref(), Some("\"d2\""));
            }
            other => panic!("{other:?}"),
        }
    }
}
