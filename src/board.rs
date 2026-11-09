//! Talking while working, without interrupting the work.
//!
//! **The board** (`board.json` in the jobs repo) holds what the agent is doing, in a sentence per
//! task. The machine running the agent sends it to the user's `info` channel every few minutes
//! while any task is open - regular, and free for whoever is working: one small write per change,
//! no tokens spent on timing.
//!
//! **The inbox** is a third ntfy channel the user writes to from the ntfy app. The machine files
//! new messages in `inbox.json` in the results repo, where `lab-agent inbox` reads them at a
//! natural pause. Marking them read sends the user a receipt; a short reply goes to `info`.

use crate::config::{self, Config};
use crate::contract::Contract;
use crate::github::{Client, Fetched};
use anyhow::{bail, Result};
use argenv_resolve::*;
use serde::{Deserialize, Serialize};

pub const BOARD_PATH: &str = "board.json";
pub const INBOX_PATH: &str = "inbox.json";

fn five() -> u64 {
    5
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Board {
    /// How often the machine sends the open tasks to the info channel.
    #[serde(default = "five")]
    pub every_minutes: u64,
    #[serde(default)]
    pub tasks: Vec<Task>,
    /// The id of the newest inbox message that has been read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_read_upto: Option<String>,
    /// Short replies for the machine to send to the info channel, newest last.
    #[serde(default)]
    pub replies: Vec<Reply>,
}

impl Default for Board {
    fn default() -> Self {
        Board {
            every_minutes: five(),
            tasks: Vec::new(),
            inbox_read_upto: None,
            replies: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub text: String,
    pub since: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reply {
    pub id: String,
    pub text: String,
    pub at: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Inbox {
    #[serde(default)]
    pub messages: Vec<Message>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub time: i64,
    pub text: String,
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// The digest of the open tasks, or `None` when there is nothing to report.
pub fn digest(board: &Board, at: f64) -> Option<String> {
    if board.tasks.is_empty() {
        return None;
    }
    let lines: Vec<String> = board
        .tasks
        .iter()
        .map(|t| {
            let mins = ((at - t.since).max(0.0) / 60.0).round() as u64;
            format!("- {} ({mins} min)", t.text)
        })
        .collect();
    Some(lines.join("\n"))
}

/// Messages newer than the read marker, oldest first.
pub fn unread<'a>(inbox: &'a Inbox, read_upto: Option<&str>) -> &'a [Message] {
    match read_upto.and_then(|id| inbox.messages.iter().position(|m| m.id == id)) {
        Some(i) => &inbox.messages[i + 1..],
        None if read_upto.is_some() && !inbox.messages.is_empty() => {
            // The marker scrolled out of the kept window: everything kept is newer.
            &inbox.messages[..]
        }
        None => &inbox.messages[..],
    }
}

pub fn parse_board(bytes: &[u8]) -> Board {
    serde_json::from_slice(bytes).unwrap_or_default()
}

pub fn parse_inbox(bytes: &[u8]) -> Inbox {
    serde_json::from_slice(bytes).unwrap_or_default()
}

/// Reads the board, applies `change`, writes it back; on a conflict (someone wrote in between)
/// reads again and re-applies.
fn update_board(client: &Client, cfg: &Config, change: impl Fn(&mut Board)) -> Result<Board> {
    for _ in 0..3 {
        let current = client.get_file(&cfg.owner, &cfg.jobs_repo, BOARD_PATH, &cfg.jobs_branch)?;
        let (mut board, sha) = match current {
            Some(f) => (parse_board(&f.content), Some(f.sha)),
            None => (Board::default(), None),
        };
        change(&mut board);
        let body = serde_json::to_vec_pretty(&board)?;
        match client.put_file(
            &cfg.owner,
            &cfg.jobs_repo,
            BOARD_PATH,
            &cfg.jobs_branch,
            &body,
            "board: update",
            sha.as_deref(),
        ) {
            Ok(_) => return Ok(board),
            Err(e) if e.to_string().contains("409") => continue,
            Err(e) => return Err(e),
        }
    }
    bail!("the board kept changing under us; try again")
}

fn client() -> Result<(Config, Client)> {
    Ok((config::load()?, Client::new(config::load_token()?)))
}

/// `lab-agent board [show|set ID TEXT|done ID|every MINUTES|clear]`
pub fn board_cli(resolved: &Resolution) -> Result<()> {
    let (cfg, client) = client()?;
    let action = Contract::BOARD_ACTION
        .get_from_or_default(resolved)
        .unwrap_or_else(|| "show".into());
    let id = Contract::BOARD_ID.get_from_or_default(resolved);
    let text = Contract::BOARD_TEXT.get_from_or_default(resolved);
    let board = match action.as_str() {
        "set" => {
            let (Some(id), Some(text)) = (id, text) else {
                bail!("usage: lab-agent board set ID \"what is being done\"")
            };
            update_board(&client, &cfg, |b| {
                match b.tasks.iter_mut().find(|t| t.id == id) {
                    Some(t) => t.text = text.clone(),
                    None => b.tasks.push(Task {
                        id: id.clone(),
                        text: text.clone(),
                        since: now(),
                    }),
                }
            })?
        }
        "done" => {
            let Some(id) = id else {
                bail!("usage: lab-agent board done ID")
            };
            update_board(&client, &cfg, |b| b.tasks.retain(|t| t.id != id))?
        }
        "every" => {
            let minutes: u64 = id
                .and_then(|m| m.parse().ok())
                .filter(|m| *m > 0)
                .ok_or_else(|| {
                    anyhow::anyhow!("usage: lab-agent board every MINUTES (a whole number above 0)")
                })?;
            update_board(&client, &cfg, |b| b.every_minutes = minutes)?
        }
        "clear" => update_board(&client, &cfg, |b| b.tasks.clear())?,
        _ => client
            .get_file(&cfg.owner, &cfg.jobs_repo, BOARD_PATH, &cfg.jobs_branch)?
            .map(|f| parse_board(&f.content))
            .unwrap_or_default(),
    };
    if Contract::JSON
        .get_from_or_default(resolved)
        .unwrap_or(false)
    {
        println!("{}", serde_json::to_string_pretty(&board)?);
    } else {
        println!(
            "sent to the info channel every {} min while tasks are open",
            board.every_minutes
        );
        match digest(&board, now()) {
            Some(d) => println!("{d}"),
            None => println!("(no open tasks)"),
        }
    }
    Ok(())
}

/// `lab-agent inbox [--ack] [--reply TEXT] [--json]`: the user's unread messages.
pub fn inbox_cli(resolved: &Resolution) -> Result<()> {
    let (cfg, client) = client()?;
    let inbox = match client.get_file_cond(
        &cfg.owner,
        &cfg.results_repo,
        INBOX_PATH,
        &cfg.results_branch,
        None,
    )? {
        Fetched::Changed { file, .. } => parse_inbox(&file.content),
        _ => Inbox::default(),
    };
    let board = client
        .get_file(&cfg.owner, &cfg.jobs_repo, BOARD_PATH, &cfg.jobs_branch)?
        .map(|f| parse_board(&f.content))
        .unwrap_or_default();
    let fresh = unread(&inbox, board.inbox_read_upto.as_deref()).to_vec();
    if Contract::JSON
        .get_from_or_default(resolved)
        .unwrap_or(false)
    {
        println!("{}", serde_json::to_string_pretty(&fresh)?);
    } else if fresh.is_empty() {
        println!("(no unread messages)");
    } else {
        for m in &fresh {
            println!("[{}] {}", crate::jobs::format_time(m.time as f64), m.text);
        }
    }
    let ack = Contract::INBOX_ACK
        .get_from_or_default(resolved)
        .unwrap_or(false);
    let reply = Contract::INBOX_REPLY.get_from_or_default(resolved);
    if ack || reply.is_some() {
        let last = fresh.last().map(|m| m.id.clone());
        update_board(&client, &cfg, |b| {
            if ack {
                if let Some(id) = &last {
                    b.inbox_read_upto = Some(id.clone());
                }
            }
            if let Some(text) = &reply {
                b.replies.push(Reply {
                    id: format!("{}", (now() * 1000.0) as u64),
                    text: text.clone(),
                    at: now(),
                });
                let keep = b.replies.len().saturating_sub(20);
                b.replies.drain(..keep);
            }
        })?;
        if ack && last.is_some() {
            eprintln!("marked read; the user gets a receipt");
        }
        if reply.is_some() {
            eprintln!("reply queued for the info channel");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str) -> Message {
        Message {
            id: id.into(),
            time: 0,
            text: id.into(),
        }
    }

    #[test]
    fn the_digest_lists_open_tasks_with_their_age() {
        let mut b = Board::default();
        assert_eq!(digest(&b, 0.0), None);
        b.tasks.push(Task {
            id: "a".into(),
            text: "fixing audio start-up".into(),
            since: 0.0,
        });
        b.tasks.push(Task {
            id: "b".into(),
            text: "writing it up upstream".into(),
            since: 540.0,
        });
        assert_eq!(
            digest(&b, 600.0).unwrap(),
            "- fixing audio start-up (10 min)\n- writing it up upstream (1 min)"
        );
    }

    #[test]
    fn unread_is_what_came_after_the_marker() {
        let inbox = Inbox {
            messages: vec![msg("1"), msg("2"), msg("3")],
        };
        assert_eq!(unread(&inbox, None).len(), 3);
        assert_eq!(unread(&inbox, Some("2")), &[msg("3")]);
        assert!(unread(&inbox, Some("3")).is_empty());
        assert_eq!(
            unread(&inbox, Some("gone")).len(),
            3,
            "a marker that scrolled out means all kept are newer"
        );
    }

    #[test]
    fn an_empty_or_old_board_reads_with_defaults() {
        assert_eq!(parse_board(b"{}"), Board::default());
        assert_eq!(parse_board(b"not json").every_minutes, 5);
        assert_eq!(parse_board(br#"{"every_minutes":10}"#).every_minutes, 10);
    }
}
