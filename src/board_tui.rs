//! The board as a live view: what is being done, by state, refreshed continuously; `q` quits.
//!
//! On the machine that runs the agent it reads the agent's local copy of the board (no network,
//! a refresh every two seconds); elsewhere, or with `--remote`, it reads GitHub with conditional
//! requests, which cost nothing while the board is unchanged.

use crate::board::{self, Board, State};
use crate::contract::Contract;
use crate::github::{Client, Fetched};
use anyhow::Result;
use argenv_resolve::*;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const LOCAL_EVERY: Duration = Duration::from_secs(2);
const REMOTE_EVERY: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(150);

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn run(resolved: &Resolution) -> Result<()> {
    let local = crate::daemon::state_dir().join(board::BOARD_PATH);
    let remote = Contract::REMOTE
        .get_from_or_default(resolved)
        .unwrap_or(false)
        || !local.exists();
    let (tx, rx) = mpsc::channel::<Result<Board, String>>();
    let source = if remote {
        let (cfg, token) = (crate::config::load()?, crate::config::load_token()?);
        thread::spawn(move || {
            let client = Client::new(token);
            let (mut etag, mut last) = (None::<String>, Board::default());
            loop {
                let got = client
                    .get_file_cond(
                        &cfg.owner,
                        &cfg.jobs_repo,
                        board::BOARD_PATH,
                        &cfg.jobs_branch,
                        etag.as_deref(),
                    )
                    .map(|f| {
                        if let Fetched::Changed { file, etag: tag } = f {
                            last = board::parse_board(&file.content);
                            etag = tag;
                        }
                        last.clone()
                    })
                    .map_err(|e| e.to_string());
                if tx.send(got).is_err() {
                    return;
                }
                thread::sleep(REMOTE_EVERY);
            }
        });
        "GitHub"
    } else {
        thread::spawn(move || loop {
            let got = std::fs::read(&local)
                .map(|b| board::parse_board(&b))
                .map_err(|e| e.to_string());
            if tx.send(got).is_err() {
                return;
            }
            thread::sleep(LOCAL_EVERY);
        });
        "this machine"
    };

    let mut terminal = ratatui::try_init()?;
    let (mut board, mut error) = (Board::default(), None::<String>);
    let result = (|| -> Result<()> {
        loop {
            while let Ok(got) = rx.try_recv() {
                match got {
                    Ok(b) => {
                        board = b;
                        error = None;
                    }
                    Err(e) => error = Some(e),
                }
            }
            terminal.draw(|frame| draw(frame, &board, error.as_deref(), source))?;
            if event::poll(TICK)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press
                        && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                    {
                        return Ok(());
                    }
                }
            }
        }
    })();
    ratatui::restore();
    result
}

/// How a state looks: a mark, and a color that reads at a glance.
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

fn draw(frame: &mut Frame, board: &Board, error: Option<&str>, source: &str) {
    let title = match error {
        Some(e) => format!(" lab-agent board ({source}) - cannot read it: {e} - q to quit "),
        None => format!(
            " lab-agent board ({source}) - {} live, digest every {} min - q to quit ",
            board.live(),
            board.every_minutes
        ),
    };
    let at = now();
    let rows: Vec<Row> = board
        .ordered()
        .into_iter()
        .map(|t| {
            let (mark, style) = look(t.state);
            Row::new(vec![
                Cell::from(format!("{mark} {}", t.state.word())).style(style),
                Cell::from(t.id.clone()).style(style),
                Cell::from(t.text.clone()),
                Cell::from(age(t, at)),
            ])
        })
        .collect();
    let empty = rows.is_empty();
    let header = Row::new(vec!["STATE", "TASK", "WHAT", "FOR"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Min(20),
            Constraint::Length(16),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(table, frame.area());
    if empty {
        let area = frame.area().inner(Margin {
            horizontal: 2,
            vertical: 2,
        });
        frame.render_widget(
            ratatui::widgets::Paragraph::new("(nothing on the board)").dim(),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn age_reads_as_running_time_or_time_since_done() {
        let mut t = board::Task {
            id: "a".into(),
            text: "t".into(),
            since: 0.0,
            state: State::Ongoing,
            done_at: None,
        };
        assert_eq!(age(&t, 600.0), "10 min");
        t.done_at = Some(300.0);
        assert_eq!(age(&t, 600.0), "done 5 min ago");
    }
}
