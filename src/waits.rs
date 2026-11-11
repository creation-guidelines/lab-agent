//! Every wait records itself (the author, 2026-11-11: waiting is a chore of its own, and it grows as
//! more is automated - it must be seen to be shrunk). Each wait the agent makes - `submit --wait` or
//! `wait` - appends one line to `waits.jsonl`, beside the board in the jobs repository: when it began,
//! how long it lasted, what was waited on (`--on`; "unlabelled" when not said, so skipping shows), for
//! which project (the board's most recently touched live task), the job, and how it ended. A gatherer
//! brings these into each project's data, beside its manual work: the two halves of flow efficiency.
use crate::board::Board;
use crate::config::Config;
use crate::github::Client;
use anyhow::{bail, Result};
use serde::Serialize;

pub const WAITS_PATH: &str = "waits.jsonl";

/// One wait, as recorded.
#[derive(Serialize, Debug, PartialEq)]
pub struct Wait {
    pub at: u64,
    pub seconds: u64,
    pub on: String,
    pub project: String,
    pub job: String,
    pub ended: String,
}

/// The project the agent works for now: the most recently touched live task's.
pub fn project_now(board: &Board) -> String {
    board
        .tasks
        .iter()
        .filter(|t| t.state.is_live())
        .max_by(|a, b| {
            a.touched
                .unwrap_or(a.since)
                .total_cmp(&b.touched.unwrap_or(b.since))
        })
        .and_then(|t| t.project.clone())
        .unwrap_or_default()
}

/// What a wait was on, as recorded: the label given, or "unlabelled".
pub fn label(on: Option<String>) -> String {
    on.map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unlabelled".into())
}

/// How a wait ended, from the job's exit status: done, failed, or gave up (the job still runs).
pub fn ended(result: &Result<i32>) -> String {
    match result {
        Ok(0) => "done",
        Ok(75) => "gave-up",
        Ok(_) => "failed",
        Err(_) => "error",
    }
    .into()
}

/// Appends one wait to the log beside the board; on a conflict (another write in between) reads again.
pub fn record(client: &Client, cfg: &Config, w: &Wait) -> Result<()> {
    let line = serde_json::to_string(w)? + "\n";
    for _ in 0..3 {
        let current = client.get_file(&cfg.owner, &cfg.jobs_repo, WAITS_PATH, &cfg.jobs_branch)?;
        let (mut body, sha) = match current {
            Some(f) => (f.content, Some(f.sha)),
            None => (Vec::new(), None),
        };
        body.extend_from_slice(line.as_bytes());
        match client.put_file(
            &cfg.owner,
            &cfg.jobs_repo,
            WAITS_PATH,
            &cfg.jobs_branch,
            &body,
            "waits: one more",
            sha.as_deref(),
        ) {
            Ok(_) => return Ok(()),
            Err(e) if e.to_string().contains("409") => continue,
            Err(e) => return Err(e),
        }
    }
    bail!("the waits log kept changing under us")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{State, Task};

    fn task(id: &str, project: &str, touched: f64, state: State) -> Task {
        Task {
            id: id.into(),
            text: id.into(),
            since: 1.0,
            touched: Some(touched),
            state,
            done_at: None,
            project: Some(project.into()),
        }
    }

    #[test]
    fn the_project_is_the_latest_live_tasks() {
        let b = Board {
            tasks: vec![
                task("a", "ocoeco", 10.0, State::Ongoing),
                task("b", "linux-phone-oco", 20.0, State::Ongoing),
                task("c", "old", 30.0, State::Done),
            ],
            ..Default::default()
        };
        assert_eq!(project_now(&b), "linux-phone-oco");
        assert_eq!(project_now(&Board::default()), "");
    }

    #[test]
    fn an_unlabelled_wait_says_so_and_endings_are_named() {
        assert_eq!(label(None), "unlabelled");
        assert_eq!(label(Some("  ".into())), "unlabelled");
        assert_eq!(label(Some("reboot".into())), "reboot");
        assert_eq!(ended(&Ok(0)), "done");
        assert_eq!(ended(&Ok(75)), "gave-up");
        assert_eq!(ended(&Ok(1)), "failed");
    }
}
