use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

/// A small, deliberately plain GitHub REST client: just the handful of calls
/// lab-agent needs (read/write a file, list a directory, check/create a
/// repo). No generated SDK, no async runtime - one struct, one blocking
/// client, kept simple on purpose.
pub struct Client {
    token: String,
    agent: ureq::Agent,
}

#[derive(Debug, Clone)]
pub struct FileContent {
    pub content: Vec<u8>,
    pub sha: String,
}

impl Client {
    pub fn new(token: String) -> Self {
        Client {
            token,
            agent: ureq::AgentBuilder::new().build(),
        }
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        req.set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "lab-agent")
    }

    pub fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}");
        match self.auth(self.agent.get(&url)).call() {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(e) => Err(anyhow!("checking repo {owner}/{repo}: {e}")),
        }
    }

    pub fn create_repo(&self, owner: &str, repo: &str, private: bool) -> Result<()> {
        // Works for both user-owned and org-owned targets.
        let who = self.whoami()?;
        let url = if who == owner {
            "https://api.github.com/user/repos".to_string()
        } else {
            format!("https://api.github.com/orgs/{owner}/repos")
        };
        let body = json!({ "name": repo, "private": private, "auto_init": true });
        self.auth(self.agent.post(&url)).send_json(body)?;
        Ok(())
    }

    fn whoami(&self) -> Result<String> {
        let resp: Value = self
            .auth(self.agent.get("https://api.github.com/user"))
            .call()?
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
        let url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={ref}");
        match self.auth(self.agent.get(&url)).call() {
            Ok(resp) => {
                let v: Value = resp.into_json()?;
                let b64 = v["content"].as_str().unwrap_or_default();
                let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                let content = STANDARD.decode(clean)?;
                let sha = v["sha"].as_str().unwrap_or_default().to_string();
                Ok(Some(FileContent { content, sha }))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(anyhow!("reading {owner}/{repo}:{path}@{ref}: {e}")),
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
        let url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={ref}");
        match self.auth(self.agent.get(&url)).call() {
            Ok(resp) => {
                let v: Value = resp.into_json()?;
                let names = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| e["name"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(names)
            }
            Err(ureq::Error::Status(404, _)) => Ok(vec![]),
            Err(e) => Err(anyhow!("listing {owner}/{repo}:{path}@{ref}: {e}")),
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
        let url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{path}");
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
            .send_json(body)?
            .into_json()?;
        Ok(resp["content"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}
