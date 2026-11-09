//! The control board: what the agent is doing, the jobs, and the conversation with a reply box -
//! one live view of the whole system. `q` quits (when the reply box is empty), Ctrl-C always.
//!
//! The layout follows the window: side by side when wide (a desktop), a 2x2 grid when roughly
//! square (a tiling window manager), stacked when tall (a phone). On the machine that runs the
//! agent everything is read from its local copies (no network, every 2 s); elsewhere, or with
//! `--remote`, from GitHub with conditional reads, which cost nothing while nothing changes.

use crate::board::{self, Board, Inbox, Line as Said, State, Who};
use crate::contract::Contract;
use crate::github::{Client, Fetched};
use crate::jobs::{self, JobRow, JobsCache};
use anyhow::Result;
use argenv_resolve::*;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const LOCAL_EVERY: Duration = Duration::from_secs(2);
const REMOTE_EVERY: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);

#[derive(Default, Clone)]
struct Snapshot {
    board: Board,
    inbox: Inbox,
    jobs: Vec<JobRow>,
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn run(resolved: &Resolution) -> Result<()> {
    let dir = crate::daemon::state_dir();
    let remote = Contract::REMOTE
        .get_from_or_default(resolved)
        .unwrap_or(false)
        || !dir.join(board::BOARD_PATH).exists();
    let (tx, rx) = mpsc::channel::<Result<Snapshot, String>>();
    let source = if remote { "GitHub" } else { "this machine" };
    if remote {
        let (cfg, token) = (crate::config::load()?, crate::config::load_token()?);
        thread::spawn(move || {
            let client = Client::new(token);
            let (mut board_tag, mut inbox_tag) = (None::<String>, None::<String>);
            let (mut snap, mut cache) = (Snapshot::default(), JobsCache::default());
            loop {
                let got = (|| -> Result<Snapshot> {
                    if let Fetched::Changed { file, etag } = client.get_file_cond(
                        &cfg.owner,
                        &cfg.jobs_repo,
                        board::BOARD_PATH,
                        &cfg.jobs_branch,
                        board_tag.as_deref(),
                    )? {
                        snap.board = board::parse_board(&file.content);
                        board_tag = etag;
                    }
                    if let Fetched::Changed { file, etag } = client.get_file_cond(
                        &cfg.owner,
                        &cfg.results_repo,
                        board::INBOX_PATH,
                        &cfg.results_branch,
                        inbox_tag.as_deref(),
                    )? {
                        snap.inbox = board::parse_inbox(&file.content);
                        inbox_tag = etag;
                    }
                    snap.jobs = jobs::fetch_jobs_cached(&client, &cfg, 20, &mut cache)?;
                    Ok(snap.clone())
                })();
                if tx.send(got.map_err(|e| e.to_string())).is_err() {
                    return;
                }
                thread::sleep(REMOTE_EVERY);
            }
        });
    } else {
        thread::spawn(move || loop {
            let read = |name: &str| std::fs::read(dir.join(name)).unwrap_or_default();
            let snap = Snapshot {
                board: board::parse_board(&read(board::BOARD_PATH)),
                inbox: board::parse_inbox(&read(board::INBOX_PATH)),
                jobs: jobs::local_rows(&dir, 20),
            };
            if tx.send(Ok(snap)).is_err() {
                return;
            }
            thread::sleep(LOCAL_EVERY);
        });
    }

    let (status_tx, status_rx) = mpsc::channel::<String>();
    let mut terminal = ratatui::try_init()?;
    let (mut snap, mut error, mut input, mut status) = (
        Snapshot::default(),
        None::<String>,
        String::new(),
        String::new(),
    );
    let result = (|| -> Result<()> {
        loop {
            while let Ok(got) = rx.try_recv() {
                match got {
                    Ok(s) => {
                        snap = s;
                        error = None;
                    }
                    Err(e) => error = Some(e),
                }
            }
            while let Ok(s) = status_rx.try_recv() {
                status = s;
            }
            terminal.draw(|f| draw(f, &snap, error.as_deref(), source, &input, &status))?;
            if !event::poll(TICK)? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(())
                }
                KeyCode::Char('q') if input.is_empty() => return Ok(()),
                KeyCode::Char(c) => input.push(c),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Esc => input.clear(),
                KeyCode::Enter if !input.trim().is_empty() => {
                    let text = std::mem::take(&mut input);
                    let tx = status_tx.clone();
                    status = "sending...".into();
                    thread::spawn(move || {
                        let said = match crate::notify::load() {
                            None => {
                                "sending needs notifications set up here (lab-agent notify-setup)"
                                    .to_string()
                            }
                            Some(cfg) => match crate::notify::Notifier::new(cfg).send(
                                crate::notify::Level::Inbox,
                                "you",
                                text.trim(),
                            ) {
                                Ok(()) => "sent - the agent reads it at its next pause".to_string(),
                                Err(e) => e.to_string(),
                            },
                        };
                        let _ = tx.send(said);
                    });
                }
                _ => {}
            }
        }
    })();
    ratatui::restore();
    result
}

/// Rows the reply box needs for `chars` characters (and the cursor) at `width` columns inside its
/// borders, between one and six: it grows with the message instead of hiding it past the edge.
fn input_rows(chars: usize, width: u16) -> u16 {
    let width = usize::from(width.saturating_sub(2).max(1));
    ((chars + 1).div_ceil(width)).clamp(1, 6) as u16
}

/// The window's shape, in terms of what fits: terminal cells are about twice as tall as wide.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Shape {
    Wide,
    Square,
    Tall,
}

fn shape(area: Rect) -> Shape {
    let aspect = f32::from(area.width) / (f32::from(area.height.max(1)) * 2.0);
    if aspect >= 1.4 {
        Shape::Wide
    } else if aspect <= 0.8 {
        Shape::Tall
    } else {
        Shape::Square
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Areas {
    tasks: Rect,
    jobs: Rect,
    talk: Rect,
    input: Rect,
}

/// `input` is the reply box's height, borders included (it grows with what is typed).
fn layout(area: Rect, input: u16) -> Areas {
    use Constraint::{Length, Min, Percentage};
    let split = |dir, area, c: &[Constraint]| {
        Layout::default()
            .direction(dir)
            .constraints(c.to_vec())
            .split(area)
    };
    match shape(area) {
        Shape::Wide => {
            let cols = split(
                Direction::Horizontal,
                area,
                &[Percentage(55), Percentage(45)],
            );
            let left = split(
                Direction::Vertical,
                cols[0],
                &[Percentage(50), Percentage(50)],
            );
            let right = split(Direction::Vertical, cols[1], &[Min(3), Length(input)]);
            Areas {
                tasks: left[0],
                jobs: left[1],
                talk: right[0],
                input: right[1],
            }
        }
        Shape::Square => {
            let rows = split(
                Direction::Vertical,
                area,
                &[Percentage(45), Min(5), Length(input)],
            );
            let top = split(
                Direction::Horizontal,
                rows[0],
                &[Percentage(50), Percentage(50)],
            );
            Areas {
                tasks: top[0],
                jobs: top[1],
                talk: rows[1],
                input: rows[2],
            }
        }
        Shape::Tall => {
            let rows = split(
                Direction::Vertical,
                area,
                &[Percentage(28), Min(5), Length(input), Percentage(24)],
            );
            Areas {
                tasks: rows[0],
                talk: rows[1],
                input: rows[2],
                jobs: rows[3],
            }
        }
    }
}

fn look(state: State) -> (&'static str, Style) {
    match state {
        State::Waiting => (
            "?",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        State::Ongoing => ("●", Style::default().fg(Color::Yellow)),
        State::Paused => ("‖", Style::default().fg(Color::Blue)),
        State::Done => (
            "✓",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::DIM),
        ),
    }
}

fn age(task: &board::Task, at: f64) -> String {
    let mins = |s: f64| ((at - s).max(0.0) / 60.0).round() as u64;
    match task.done_at {
        Some(done) => format!("done {} min ago", mins(done)),
        None => format!("{} min", mins(task.since)),
    }
}

/// A conversation line as shown: time, who, text, and for the user's messages whether it was read.
fn said(line: &Said) -> Line<'static> {
    let (who, color) = match line.who {
        Who::User => ("you", Color::Cyan),
        Who::Agent => ("Claude", Color::Yellow),
    };
    let tick = match line.who {
        Who::User if line.read => " ✓",
        _ => "",
    };
    Line::from(vec![
        Span::styled(
            format!("{} ", jobs::format_time(line.at)),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::styled(
            format!("{who}: "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(line.text.clone()),
        Span::styled(tick, Style::default().fg(Color::Green)),
    ])
}

fn draw(
    f: &mut Frame,
    snap: &Snapshot,
    error: Option<&str>,
    source: &str,
    input: &str,
    status: &str,
) {
    // the box's width does not depend on its height: measure with one row, then grow it
    let width = layout(f.area(), 3).input.width;
    let a = layout(f.area(), input_rows(input.chars().count(), width) + 2);
    let at = now();
    let title = match error {
        Some(e) => format!(" control board ({source}) - cannot read: {e} "),
        None => format!(
            " control board ({source}, times UTC) - {} live tasks - q quits ",
            snap.board.live()
        ),
    };
    let task_rows: Vec<Row> = snap
        .board
        .ordered()
        .into_iter()
        .map(|t| {
            let (mark, style) = look(t.state);
            Row::new(vec![
                Cell::from(format!("{mark} {}", t.state.word())).style(style),
                Cell::from(t.text.clone()),
                Cell::from(age(t, at)),
            ])
        })
        .collect();
    f.render_widget(
        Table::new(
            task_rows,
            [
                Constraint::Length(17),
                Constraint::Min(10),
                Constraint::Length(15),
            ],
        )
        .block(Block::default().borders(Borders::ALL).title(title)),
        a.tasks,
    );
    let job_rows: Vec<Row> = snap
        .jobs
        .iter()
        .map(|j| {
            let style = match j.display_status() {
                "ok" => Style::default().fg(Color::Green),
                "failed" | "timed_out" | "stale" => Style::default().fg(Color::Red),
                "running" => Style::default().fg(Color::Yellow),
                _ => Style::default(),
            };
            let started = j.started_at.map(jobs::format_time).unwrap_or_default();
            let took = j
                .elapsed_secs()
                .map(jobs::format_duration)
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(j.name.clone()),
                Cell::from(j.display_status().to_string()).style(style),
                Cell::from(started),
                Cell::from(took),
            ])
        })
        .collect();
    f.render_widget(
        Table::new(
            job_rows,
            [
                Constraint::Min(12),
                Constraint::Length(11),
                Constraint::Length(9),
                Constraint::Length(7),
            ],
        )
        .block(Block::default().borders(Borders::ALL).title(" jobs ")),
        a.jobs,
    );
    // the conversation, newest at the bottom: scroll so the end is in view
    let lines: Vec<Line> = board::conversation(&snap.inbox, &snap.board)
        .iter()
        .map(said)
        .collect();
    let width = a.talk.width.saturating_sub(2).max(1) as usize;
    let rows: usize = lines.iter().map(|l| l.width().div_ceil(width).max(1)).sum();
    let visible = a.talk.height.saturating_sub(2) as usize;
    let scroll = rows.saturating_sub(visible) as u16;
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" conversation "),
            ),
        a.talk,
    );
    let hint = if status.is_empty() {
        " say something - Enter sends, Esc clears ".to_string()
    } else {
        format!(" {status} ")
    };
    f.render_widget(
        Paragraph::new(format!("{input}▏"))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(hint)),
        a.input,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn a_desktop_window_puts_the_conversation_beside_the_work() {
        let a = layout(r(200, 50), 3);
        assert_eq!(shape(r(200, 50)), Shape::Wide);
        assert!(a.talk.x > a.tasks.x && a.jobs.x == a.tasks.x && a.jobs.y > a.tasks.y);
        assert!(a.input.y > a.talk.y && a.input.x == a.talk.x);
    }

    #[test]
    fn a_square_window_puts_tasks_and_jobs_side_by_side_on_top() {
        let a = layout(r(100, 50), 3);
        assert_eq!(shape(r(100, 50)), Shape::Square);
        assert!(a.tasks.y == a.jobs.y && a.jobs.x > a.tasks.x);
        assert!(a.talk.y > a.tasks.y && a.input.y > a.talk.y);
    }

    #[test]
    fn a_phone_stacks_everything_with_the_reply_box_under_the_conversation() {
        let a = layout(r(48, 90), 3);
        assert_eq!(shape(r(48, 90)), Shape::Tall);
        assert!(a.tasks.y < a.talk.y && a.talk.y < a.input.y && a.input.y < a.jobs.y);
        assert!([a.tasks.x, a.talk.x, a.input.x, a.jobs.x]
            .iter()
            .all(|x| *x == 0));
        assert_eq!(a.input.height, 3);
    }

    #[test]
    fn every_state_has_its_own_mark_and_color() {
        let states = [State::Waiting, State::Ongoing, State::Paused, State::Done];
        let marks: std::collections::HashSet<&str> = states.iter().map(|s| look(*s).0).collect();
        let colors: std::collections::HashSet<_> = states
            .iter()
            .map(|s| format!("{:?}", look(*s).1.fg))
            .collect();
        assert_eq!((marks.len(), colors.len()), (4, 4));
    }

    #[test]
    fn the_reply_box_grows_with_the_message_and_stops_at_six_rows() {
        assert_eq!(input_rows(0, 42), 1);
        assert_eq!(
            input_rows(39, 42),
            1,
            "40 columns inside the borders: 39 chars and the cursor fit"
        );
        assert_eq!(input_rows(40, 42), 2);
        assert_eq!(input_rows(10_000, 42), 6);
        let small = layout(r(48, 90), 3);
        let grown = layout(r(48, 90), 8);
        assert!(
            grown.input.height == 8 && grown.talk.height < small.talk.height,
            "the conversation gives up the room"
        );
    }
}
