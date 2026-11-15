//! What a driver says with git alone. Some workspaces let a repository be pushed to and its API only be
//! read: there a file is said by adding it to a branch with a push. None of the repository's files are
//! kept here - the branch's tree is fetched without them, the one file is added to it, and the commit is
//! pushed; if the branch moved meanwhile, once more from where it then stands.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Why a file was not said.
#[derive(Debug, PartialEq, Eq)]
pub enum NotPushed {
    /// A file is there already, and only a new one was to be said.
    AlreadyThere,
    Because(String),
}

/// Adds the file `path`, holding `content`, to `branch` of the GitHub repository `repo`, as one commit
/// that says `message`. With `only_new`, a file already there is left as it is, and that is said.
pub fn push(
    repo: &str,
    branch: &str,
    path: &str,
    content: &[u8],
    message: &str,
    only_new: bool,
) -> Result<(), NotPushed> {
    let kept = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("oco-agent")
        .join(format!("{}.git", repo.replace('/', "--")));
    push_to(
        &format!("https://github.com/{repo}"),
        &kept,
        branch,
        path,
        content,
        message,
        only_new,
    )
}

/// The same, to the repository at `url`, with what git needs for it kept in the folder `kept`.
pub fn push_to(
    url: &str,
    kept: &Path,
    branch: &str,
    path: &str,
    content: &[u8],
    message: &str,
    only_new: bool,
) -> Result<(), NotPushed> {
    let because = NotPushed::Because;
    if !kept.join("HEAD").is_file() {
        std::fs::create_dir_all(kept).map_err(|e| because(e.to_string()))?;
        git(kept, &["init", "-q", "--bare"], None).map_err(because)?;
        git(kept, &["remote", "add", "origin", url], None).map_err(because)?;
    }
    // who commits: whoever git is told the driver is - and where it is told nothing, oco-agent itself
    let told = git(kept, &["config", "user.email"], None).is_ok_and(|email| !email.is_empty());
    let who: &[&str] = if told {
        &[]
    } else {
        &[
            "-c",
            "user.name=oco-agent",
            "-c",
            "user.email=oco-agent@localhost",
        ]
    };
    let mut last = String::new();
    for _ in 0..3 {
        // the branch as it now stands, without its files
        git(
            kept,
            &[
                "fetch",
                "-q",
                "--depth",
                "1",
                "--filter=blob:none",
                "origin",
                branch,
            ],
            None,
        )
        .map_err(because)?;
        if only_new
            && !git(kept, &["ls-tree", "FETCH_HEAD", "--", path], None)
                .map_err(because)?
                .is_empty()
        {
            return Err(NotPushed::AlreadyThere);
        }
        let blob = git(kept, &["hash-object", "-w", "--stdin"], Some(content)).map_err(because)?;
        git(kept, &["read-tree", "FETCH_HEAD"], None).map_err(because)?;
        git(
            kept,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{blob},{path}"),
            ],
            None,
        )
        .map_err(because)?;
        let tree = git(kept, &["write-tree", "--missing-ok"], None).map_err(because)?;
        let mut commit = who.to_vec();
        commit.extend(["commit-tree", &tree, "-p", "FETCH_HEAD", "-m", message]);
        let commit = git(kept, &commit, None).map_err(because)?;
        match git(
            kept,
            &[
                "push",
                "-q",
                "origin",
                &format!("{commit}:refs/heads/{branch}"),
            ],
            None,
        ) {
            Ok(_) => return Ok(()),
            // the branch moved while this was being made: once more, from where it now stands
            Err(why) => last = why,
        }
    }
    Err(because(format!("{path} could not be pushed: {last}")))
}

/// Runs git in `dir`, with `input` to read if any: what it printed, or what it said went wrong.
fn git(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<String, String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_INDEX_FILE", dir.join("said-index"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git could not be run: {e}"))?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(bytes).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        git(dir, args, None).unwrap_or_else(|why| panic!("git {args:?}: {why}"))
    }

    #[test]
    fn a_file_is_said_with_git_alone_and_a_new_one_only_once() {
        let root = std::env::temp_dir().join(format!("oco-agent-pushed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (there, seed, kept) = (
            root.join("there.git"),
            root.join("seed"),
            root.join("kept.git"),
        );
        std::fs::create_dir_all(&seed).unwrap();
        // a repository with one file on its main, as the one a driver speaks to
        run(&root, &["init", "-q", "--bare", "-b", "main", "there.git"]);
        run(&seed, &["init", "-q", "-b", "main"]);
        std::fs::write(seed.join("README"), "a repository\n").unwrap();
        run(&seed, &["add", "README"]);
        run(
            &seed,
            &[
                "-c",
                "user.name=someone",
                "-c",
                "user.email=someone@localhost",
                "commit",
                "-q",
                "-m",
                "first",
            ],
        );
        run(&seed, &["push", "-q", there.to_str().unwrap(), "main"]);
        let url = format!("file://{}", there.display());
        let say = |path: &str, text: &str, only_new: bool| {
            push_to(&url, &kept, "main", path, text.as_bytes(), "said", only_new)
        };

        assert_eq!(say("asked/a.sh", "echo a\n", true), Ok(()));
        assert_eq!(run(&there, &["show", "main:asked/a.sh"]), "echo a");
        assert_eq!(
            run(&there, &["show", "main:README"]),
            "a repository",
            "what was there stays"
        );
        assert_eq!(
            say("asked/a.sh", "echo again\n", true),
            Err(NotPushed::AlreadyThere),
            "what is asked is asked once"
        );
        assert_eq!(run(&there, &["show", "main:asked/a.sh"]), "echo a");
        // what is not only for new files is said again, over what was there
        assert_eq!(say("beats/ann.json", "1", false), Ok(()));
        assert_eq!(say("beats/ann.json", "2", false), Ok(()));
        assert_eq!(run(&there, &["show", "main:beats/ann.json"]), "2");
        assert_eq!(
            run(&there, &["rev-list", "--count", "main"]),
            "4",
            "each saying is one commit on what was there"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
