use crate::config;
use crate::github::Client;
use crate::jobs::{self, JobRow, JobsCache};
use anyhow::Result;
use argenv_resolve::Resolution;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// How often fresh data is fetched in the background. The UI itself redraws far more often than
/// this (every tick, so elapsed-time counters move smoothly) without waiting on the network.
const FETCH_EVERY: Duration = Duration::from_secs(3);
/// How often to look when no job is running.
const IDLE_EVERY: Duration = Duration::from_secs(10);
/// How often the local view re-reads its own files (free: no network).
const LOCAL_EVERY: Duration = Duration::from_secs(1);
/// How long to wait for a key before redrawing anyway (keeps elapsed-time columns ticking and
/// keeps the UI responsive to quit immediately rather than only after a fetch completes).
const TICK: Duration = Duration::from_millis(150);

/// Lists ongoing/recent jobs and their status, refreshed in the background. Reads the results
/// repo - the same place the daemon writes to - so this works from any machine with the token,
/// not just the one running the jobs.
///
/// Fetching happens on its own thread: the event loop never blocks on the network, so the first
/// frame draws immediately (not after however long the first fetch takes) and a keypress is
/// handled the moment it arrives, not only once a fetch finishes.
pub fn run(resolved: &Resolution) -> Result<()> {
    let (rows_tx, rows_rx) = mpsc::channel::<Result<Vec<JobRow>, String>>();
    let label = match jobs::source(resolved) {
        jobs::Source::Local(dir) => {
            // On the machine that runs the agent its own records are the truth: reading them is
            // instant and costs no request, so it can refresh every second and works while GitHub
            // is refusing us.
            thread::spawn(move || loop {
                if rows_tx.send(Ok(jobs::local_rows(&dir, 30))).is_err() {
                    return;
                }
                thread::sleep(LOCAL_EVERY);
            });
            "this machine"
        }
        jobs::Source::Remote => {
            let (cfg, token) = (config::load()?, config::load_token()?);
            thread::spawn(move || {
                let client = Client::new(token);
                let mut cache = JobsCache::default();
                loop {
                    let fetched = jobs::fetch_jobs_cached(&client, &cfg, 30, &mut cache);
                    // Quiet when nothing is running: nothing is going to change.
                    let busy = fetched
                        .as_ref()
                        .is_ok_and(|rows| rows.iter().any(|r| r.status == "running" && !r.stale));
                    // A spent budget will not return sooner for being asked again.
                    let pause = match &fetched {
                        Err(e) => e
                            .downcast_ref::<crate::github::RateLimited>()
                            .map_or(FETCH_EVERY, |l| l.wait().min(IDLE_EVERY)),
                        Ok(_) if busy => FETCH_EVERY,
                        Ok(_) => IDLE_EVERY,
                    };
                    let result = fetched.map_err(|e| e.to_string());
                    // The receiver is gone once the UI exits; stop fetching rather than error forever.
                    if rows_tx.send(result).is_err() {
                        return;
                    }
                    thread::sleep(pause);
                }
            });
            "GitHub"
        }
    };

    let mut terminal = ratatui::try_init()?;
    let mut rows: Vec<JobRow> = Vec::new();
    let mut error: Option<String> = None;
    let mut loading = true;

    let result = (|| -> Result<()> {
        loop {
            // Drain whatever the background thread has produced so far without blocking.
            while let Ok(fetched) = rows_rx.try_recv() {
                loading = false;
                match fetched {
                    Ok(r) => {
                        rows = r;
                        error = None;
                    }
                    Err(e) => error = Some(e),
                }
            }

            terminal.draw(|frame| draw(frame, &rows, error.as_deref(), loading, label))?;

            if event::poll(TICK)? {
                if let Event::Key(key) = event::read()? {
                    // Terminals that report both press and release would otherwise see 'q'
                    // trigger twice, or a release-only event go unhandled depending on the
                    // platform; filtering to Press is the correct, portable match.
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

fn draw(frame: &mut Frame, rows: &[JobRow], error: Option<&str>, loading: bool, label: &str) {
    let area = frame.area();
    let title = if loading {
        format!(" lab-agent monitor ({label}) - loading... (q to quit) ")
    } else if let Some(e) = error {
        format!(" lab-agent monitor ({label}) - last fetch failed: {e} (q to quit) ")
    } else {
        format!(" lab-agent monitor ({label}) - q to quit ")
    };

    let header = Row::new(vec![
        Cell::from("JOB"),
        Cell::from("STATUS"),
        Cell::from("EXIT"),
        Cell::from("STARTED"),
        Cell::from("ELAPSED"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let body: Vec<Row> = rows
        .iter()
        .map(|row| {
            let style = match row.display_status() {
                "ok" => Style::default().fg(Color::Green),
                "failed" | "timed_out" | "stale" => Style::default().fg(Color::Red),
                "running" => Style::default().fg(Color::Yellow),
                _ => Style::default(),
            };
            let started = row
                .started_at
                .map(jobs::format_time)
                .unwrap_or_else(|| "-".into());
            let elapsed = row
                .elapsed_secs()
                .map(jobs::format_duration)
                .unwrap_or_else(|| "-".into());
            let exit = row
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into());
            Row::new(vec![
                Cell::from(row.name.clone()),
                Cell::from(row.display_status().to_string()).style(style),
                Cell::from(exit),
                Cell::from(started),
                Cell::from(elapsed),
            ])
        })
        .collect();

    let table = Table::new(
        body,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(16),
            Constraint::Percentage(10),
            Constraint::Percentage(14),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(table, area);
}
