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

/// Where a task stands. Older boards have no state: their tasks are ongoing.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Ongoing,
    /// Waiting for the user.
    Waiting,
    Paused,
    /// Not started yet: shown after the active work, not in digests.
    Planned,
    Done,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Ongoing => "ongoing",
            State::Waiting => "waiting for you",
            State::Paused => "paused",
            State::Planned => "planned",
            State::Done => "done",
        }
    }
    /// Live tasks are the active ones, reported in digests; planned and done ones are not.
    pub fn is_live(self) -> bool {
        matches!(self, State::Ongoing | State::Waiting | State::Paused)
    }
    /// Display order: what needs the user first, finished last.
    fn rank(self) -> u8 {
        match self {
            State::Waiting => 0,
            State::Ongoing => 1,
            State::Paused => 2,
            State::Planned => 3,
            State::Done => 4,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub text: String,
    pub since: f64,
    #[serde(default)]
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<f64>,
    /// Which project the task belongs to (none: shown under "all" only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// How long a finished task stays on the board, and how many finished tasks at most.
const KEEP_DONE_SECS: f64 = 24.0 * 3600.0;
const KEEP_DONE: usize = 15;

impl Board {
    pub fn live(&self) -> usize {
        self.tasks.iter().filter(|t| t.state.is_live()).count()
    }

    /// The projects tasks belong to, sorted.
    pub fn projects(&self) -> Vec<String> {
        let mut p: Vec<String> = self
            .tasks
            .iter()
            .filter_map(|t| t.project.clone())
            .collect();
        p.sort();
        p.dedup();
        p
    }

    /// The tasks in display order: waiting for the user, ongoing, paused, then the latest done.
    pub fn ordered(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.iter().collect();
        tasks.sort_by(|a, b| {
            a.state.rank().cmp(&b.state.rank()).then(
                b.done_at
                    .unwrap_or(0.0)
                    .partial_cmp(&a.done_at.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        tasks
    }
}

/// One change to the board without a project (the tests' shorthand for [`apply_in`]).
#[cfg(test)]
pub fn apply(
    board: &mut Board,
    action: &str,
    id: Option<&str>,
    text: Option<&str>,
    at: f64,
) -> Result<()> {
    apply_in(board, action, id, text, None, at)
}

/// One change to the board, applied the same way by the CLI and in tests; `project` is the one a new
/// or updated task belongs to.
pub fn apply_in(
    board: &mut Board,
    action: &str,
    id: Option<&str>,
    text: Option<&str>,
    project: Option<&str>,
    at: f64,
) -> Result<()> {
    let need_id = || id.ok_or_else(|| anyhow::anyhow!("usage: lab-agent board {action} ID"));
    match action {
        "set" | "plan" => {
            let state = if action == "plan" {
                State::Planned
            } else {
                State::Ongoing
            };
            let (id, text) = (
                need_id()?,
                text.ok_or_else(|| {
                    anyhow::anyhow!("usage: lab-agent board {action} ID \"what is to be done\"")
                })?,
            );
            match board.tasks.iter_mut().find(|t| t.id == id) {
                Some(t) => {
                    t.text = text.to_string();
                    if t.state == State::Done
                        || (t.state == State::Planned && state == State::Ongoing)
                    {
                        t.since = at;
                    }
                    t.state = state;
                    t.done_at = None;
                    if let Some(p) = project {
                        t.project = Some(p.to_string());
                    }
                }
                None => board.tasks.push(Task {
                    id: id.into(),
                    text: text.into(),
                    since: at,
                    state,
                    done_at: None,
                    project: project.map(String::from),
                }),
            }
        }
        "wait" | "pause" | "resume" | "done" => {
            let id = need_id()?;
            let t = board
                .tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| anyhow::anyhow!("no task {id} on the board"))?;
            if action == "resume" && t.state == State::Planned {
                t.since = at;
            }
            t.state = match action {
                "wait" => State::Waiting,
                "pause" => State::Paused,
                "resume" => State::Ongoing,
                _ => State::Done,
            };
            t.done_at = (t.state == State::Done).then_some(at);
        }
        "every" => {
            board.every_minutes = id
                .and_then(|m| m.parse().ok())
                .filter(|m: &u64| *m > 0)
                .ok_or_else(|| {
                    anyhow::anyhow!("usage: lab-agent board every MINUTES (a whole number above 0)")
                })?;
        }
        "clear" => board.tasks.clear(),
        _ => bail!("unknown board action {action}"),
    }
    // Finished tasks fade: kept a day, at most the newest fifteen.
    let mut done: Vec<f64> = board.tasks.iter().filter_map(|t| t.done_at).collect();
    done.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let cutoff = done
        .get(KEEP_DONE - 1)
        .copied()
        .unwrap_or(f64::MIN)
        .max(at - KEEP_DONE_SECS);
    board
        .tasks
        .retain(|t| t.done_at.is_none_or(|d| d >= cutoff));
    Ok(())
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
    let live: Vec<&Task> = board
        .ordered()
        .into_iter()
        .filter(|t| t.state.is_live())
        .collect();
    if live.is_empty() {
        return None;
    }
    let lines: Vec<String> = live
        .iter()
        .map(|t| {
            let mins = ((at - t.since).max(0.0) / 60.0).round() as u64;
            match t.state {
                State::Ongoing => format!("- {} ({mins} min)", t.text),
                state => format!("- [{}] {} ({mins} min)", state.word(), t.text),
            }
        })
        .collect();
    Some(lines.join(
        "
",
    ))
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

/// The title the agent's replies are sent with.
pub const REPLY_TITLE: &str = "Claude";

/// Who said a line of the conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    User,
    Agent,
}

/// One line of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub at: f64,
    pub who: Who,
    pub text: String,
    /// For the user's messages: whether the agent has read it.
    pub read: bool,
}

/// The user's messages and the agent's replies, oldest first. With `sent` (this machine's log of
/// everything the agent sent: heads-ups, questions, replies) the agent's side comes from it;
/// without it (read from elsewhere), from the replies on the board.
pub fn conversation_with(
    inbox: &Inbox,
    board: &Board,
    sent: Option<&[crate::notify::Sent]>,
) -> Vec<Line> {
    let upto = board
        .inbox_read_upto
        .as_deref()
        .and_then(|id| inbox.messages.iter().position(|m| m.id == id));
    let mut lines: Vec<Line> = inbox
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| Line {
            at: m.time as f64,
            who: Who::User,
            text: m.text.clone(),
            read: upto.is_some_and(|u| i <= u),
        })
        .chain(board.replies.iter().map(|r| Line {
            at: r.at,
            who: Who::Agent,
            text: r.text.clone(),
            read: true,
        }))
        // this machine's log adds what is not a reply (heads-ups, questions); replies come from
        // the board, which has them all, also from before the log existed
        .chain(
            sent.unwrap_or_default()
                .iter()
                .filter(|s| s.title != REPLY_TITLE)
                .map(|s| Line {
                    at: s.at,
                    who: Who::Agent,
                    text: if s.urgent {
                        format!("[urgent] {}: {}", s.title, s.text)
                    } else {
                        format!("{}: {}", s.title, s.text)
                    },
                    read: true,
                }),
        )
        .collect();
    lines.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
    lines
}

pub fn parse_inbox(bytes: &[u8]) -> Inbox {
    serde_json::from_slice(bytes).unwrap_or_default()
}

/// Reads the board, applies `change`, writes it back; on a conflict (someone wrote in between)
/// reads again and re-applies.
fn update_board(
    client: &Client,
    cfg: &Config,
    change: impl Fn(&mut Board) -> Result<()>,
) -> Result<Board> {
    for _ in 0..3 {
        let current = client.get_file(&cfg.owner, &cfg.jobs_repo, BOARD_PATH, &cfg.jobs_branch)?;
        let (mut board, sha) = match current {
            Some(f) => (parse_board(&f.content), Some(f.sha)),
            None => (Board::default(), None),
        };
        change(&mut board)?; // a change that cannot apply writes nothing
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

/// `lab-agent board [show|set ID TEXT|wait ID|pause ID|resume ID|done ID|every MINUTES|clear]`.
/// With no action in a terminal it is a live view; piped, it prints once.
pub fn board_cli(resolved: &Resolution) -> Result<()> {
    let action = Contract::BOARD_ACTION.get_from_or_default(resolved);
    let json = Contract::JSON
        .get_from_or_default(resolved)
        .unwrap_or(false);
    if action.is_none() && !json && std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return crate::board_tui::run(resolved);
    }
    let action = action.unwrap_or_else(|| "show".into());
    let board = if action == "show" {
        read_board(resolved)?
    } else {
        let (cfg, client) = client()?;
        let id = Contract::BOARD_ID.get_from_or_default(resolved);
        let text = Contract::BOARD_TEXT.get_from_or_default(resolved);
        let project = Contract::BOARD_PROJECT.get_from_or_default(resolved);
        update_board(&client, &cfg, |b| {
            apply_in(
                b,
                &action,
                id.as_deref(),
                text.as_deref(),
                project.as_deref(),
                now(),
            )
        })?
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&board)?);
    } else {
        println!(
            "live tasks go to the info channel every {} min",
            board.every_minutes
        );
        for t in board.ordered() {
            let project = t
                .project
                .as_deref()
                .map(|p| format!("[{p}] "))
                .unwrap_or_default();
            println!("  {:<16} {:<10} {project}{}", t.state.word(), t.id, t.text);
        }
        if board.tasks.is_empty() {
            println!("  (empty)");
        }
    }
    Ok(())
}

/// The board, from this machine's copy when the agent runs here (no network), else from GitHub.
pub fn read_board(resolved: &Resolution) -> Result<Board> {
    let local = crate::daemon::state_dir().join(BOARD_PATH);
    let remote = Contract::REMOTE
        .get_from_or_default(resolved)
        .unwrap_or(false);
    if !remote {
        if let Ok(bytes) = std::fs::read(&local) {
            return Ok(parse_board(&bytes));
        }
    }
    let (cfg, client) = client()?;
    Ok(client
        .get_file(&cfg.owner, &cfg.jobs_repo, BOARD_PATH, &cfg.jobs_branch)?
        .map(|f| parse_board(&f.content))
        .unwrap_or_default())
}

/// `lab-agent inbox "text"` sends to the agent (the same channel the phone writes to);
/// `lab-agent inbox [--ack] [--reply TEXT] [--json]` shows the unread messages.
pub fn inbox_cli(resolved: &Resolution) -> Result<()> {
    if let Some(text) = Contract::INBOX_TEXT.get_from_or_default(resolved) {
        return send_to_agent(&text);
    }
    inbox_read(resolved)
}

/// Sends a message to the agent through the inbox channel, from this machine.
pub fn send_to_agent(text: &str) -> Result<()> {
    let cfg = crate::notify::load().ok_or_else(|| {
        anyhow::anyhow!("notifications are not set up here: run `lab-agent notify-setup`")
    })?;
    crate::notify::Notifier::new(cfg).send(crate::notify::Level::Inbox, "you", text.trim())?;
    println!("sent; the agent reads it at its next pause");
    Ok(())
}

fn inbox_read(resolved: &Resolution) -> Result<()> {
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
            Ok(())
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
            state: State::Ongoing,
            done_at: None,
            project: None,
        });
        b.tasks.push(Task {
            id: "b".into(),
            text: "writing it up upstream".into(),
            since: 540.0,
            state: State::Ongoing,
            done_at: None,
            project: None,
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

    #[test]
    fn states_move_as_asked_and_done_is_not_reported() {
        let mut b = Board::default();
        apply(&mut b, "set", Some("a"), Some("audio"), 0.0).unwrap();
        apply(&mut b, "set", Some("b"), Some("tui"), 0.0).unwrap();
        apply(&mut b, "wait", Some("a"), None, 60.0).unwrap();
        apply(&mut b, "done", Some("b"), None, 120.0).unwrap();
        assert_eq!(
            b.ordered()[0].state,
            State::Waiting,
            "what needs the user comes first"
        );
        assert_eq!(b.live(), 1);
        assert_eq!(
            digest(&b, 120.0).unwrap(),
            "- [waiting for you] audio (2 min)"
        );
        apply(&mut b, "resume", Some("a"), None, 180.0).unwrap();
        assert_eq!(digest(&b, 180.0).unwrap(), "- audio (3 min)");
        assert!(apply(&mut b, "pause", Some("nope"), None, 0.0).is_err());
    }

    #[test]
    fn finished_tasks_fade_after_a_day() {
        let mut b = Board::default();
        apply(&mut b, "set", Some("old"), Some("x"), 0.0).unwrap();
        apply(&mut b, "done", Some("old"), None, 10.0).unwrap();
        apply(&mut b, "set", Some("new"), Some("y"), 100_000.0).unwrap();
        assert!(
            b.tasks.iter().all(|t| t.id != "old"),
            "a task done more than a day ago is gone"
        );
    }

    #[test]
    fn an_old_task_without_a_state_is_ongoing() {
        let b = parse_board(br#"{"tasks":[{"id":"a","text":"t","since":1.0}]}"#);
        assert_eq!(b.tasks[0].state, State::Ongoing);
    }

    #[test]
    fn a_change_to_an_existing_task_applies_to_the_board_it_was_made_on() {
        // The bug this pins: the CLI checked each change against an empty board first, so every
        // wait/pause/resume/done failed with "no task" and silently changed nothing.
        let mut b = Board::default();
        apply(&mut b, "set", Some("tui"), Some("the live board"), 0.0).unwrap();
        for (action, want) in [
            ("wait", State::Waiting),
            ("pause", State::Paused),
            ("resume", State::Ongoing),
            ("done", State::Done),
        ] {
            apply(&mut b, action, Some("tui"), None, 1.0).unwrap();
            assert_eq!(b.tasks[0].state, want, "{action}");
        }
        assert!(
            apply(&mut Board::default(), "done", Some("tui"), None, 1.0).is_err(),
            "a missing task is an error, not a silent no-op"
        );
    }

    #[test]
    fn the_conversation_interleaves_by_time_and_marks_what_was_read() {
        let inbox = Inbox {
            messages: vec![
                Message {
                    id: "m1".into(),
                    time: 10,
                    text: "hi".into(),
                },
                Message {
                    id: "m2".into(),
                    time: 30,
                    text: "still there?".into(),
                },
            ],
        };
        let board = Board {
            inbox_read_upto: Some("m1".into()),
            replies: vec![Reply {
                id: "r1".into(),
                text: "here".into(),
                at: 20.0,
            }],
            ..Default::default()
        };
        let c = conversation_with(&inbox, &board, None);
        assert_eq!(
            c.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["hi", "here", "still there?"]
        );
        assert_eq!((c[0].read, c[2].read), (true, false));
        assert_eq!(c[1].who, Who::Agent);
    }

    #[test]
    fn on_the_machine_the_chat_adds_the_log_to_the_replies() {
        let inbox = Inbox {
            messages: vec![Message {
                id: "m1".into(),
                time: 10,
                text: "ready".into(),
            }],
        };
        let board = Board {
            replies: vec![Reply {
                id: "r".into(),
                text: "only a reply".into(),
                at: 5.0,
            }],
            ..Default::default()
        };
        let sent = [crate::notify::Sent {
            at: 20.0,
            urgent: true,
            title: "Test 3".into(),
            text: "two tones".into(),
        }];
        let c = conversation_with(&inbox, &board, Some(&sent));
        assert_eq!(
            c.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["only a reply", "ready", "[urgent] Test 3: two tones"]
        );
        assert_eq!(
            conversation_with(&inbox, &board, None)[0].text,
            "only a reply",
            "elsewhere: the replies"
        );
    }

    #[test]
    fn planned_tasks_wait_quietly_and_start_their_clock_when_resumed() {
        let mut b = Board::default();
        apply_in(
            &mut b,
            "plan",
            Some("mic"),
            Some("test the microphone"),
            Some("surya-audio"),
            0.0,
        )
        .unwrap();
        apply_in(
            &mut b,
            "set",
            Some("tui"),
            Some("projects on the board"),
            Some("lab-agent"),
            0.0,
        )
        .unwrap();
        assert_eq!(b.live(), 1, "planned is not live");
        assert_eq!(digest(&b, 60.0).unwrap(), "- projects on the board (1 min)");
        assert_eq!(b.projects(), ["lab-agent", "surya-audio"]);
        assert_eq!(b.ordered().last().unwrap().state, State::Planned);
        apply(&mut b, "resume", Some("mic"), None, 600.0).unwrap();
        let mic = b.tasks.iter().find(|t| t.id == "mic").unwrap();
        assert_eq!(
            (mic.state, mic.since, mic.project.as_deref()),
            (State::Ongoing, 600.0, Some("surya-audio"))
        );
    }

    #[test]
    fn the_chat_has_every_reply_and_the_other_messages_once() {
        let board = Board {
            replies: vec![Reply {
                id: "r".into(),
                text: "got it".into(),
                at: 5.0,
            }],
            ..Default::default()
        };
        let sent = [
            crate::notify::Sent {
                at: 5.0,
                urgent: false,
                title: REPLY_TITLE.into(),
                text: "got it".into(),
            },
            crate::notify::Sent {
                at: 9.0,
                urgent: true,
                title: "Test".into(),
                text: "listen".into(),
            },
        ];
        let c = conversation_with(&Inbox::default(), &board, Some(&sent));
        assert_eq!(
            c.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["got it", "[urgent] Test: listen"]
        );
    }
}
