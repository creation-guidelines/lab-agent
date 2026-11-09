use crate::config::Config;
use crate::github::Client;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use std::io::stdout;
use std::time::Duration;

/// Lists ongoing/recent jobs and their status, refreshed periodically. Reads
/// the results repo - the same place the daemon writes to - so this works
/// from any machine with the token, not just the one running the jobs.
pub fn run(cfg: Config, token: String) -> Result<()> {
    let client = Client::new(token);

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = loop {
        let rows = match fetch_rows(&client, &cfg) {
            Ok(r) => r,
            Err(e) => vec![("error".to_string(), e.to_string(), String::new())],
        };

        terminal.draw(|f| {
            let area = f.area();
            let header = Row::new(vec![
                Cell::from("job"),
                Cell::from("status"),
                Cell::from("exit code"),
            ])
            .style(Style::default().add_modifier(Modifier::BOLD));
            let body: Vec<Row> = rows
                .iter()
                .map(|(name, status, code)| {
                    let style = match status.as_str() {
                        "ok" => Style::default().fg(Color::Green),
                        "failed" | "timed_out" => Style::default().fg(Color::Red),
                        "running" => Style::default().fg(Color::Yellow),
                        _ => Style::default(),
                    };
                    Row::new(vec![
                        Cell::from(name.clone()),
                        Cell::from(status.clone()).style(style),
                        Cell::from(code.clone()),
                    ])
                })
                .collect();
            let table = Table::new(
                body,
                [
                    Constraint::Percentage(50),
                    Constraint::Percentage(25),
                    Constraint::Percentage(25),
                ],
            )
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(format!(
                " lab-agent monitor - {}/{} (q to quit) ",
                cfg.owner, cfg.results_repo
            )));
            f.render_widget(table, area);
        })?;

        if event::poll(Duration::from_secs(3))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break Ok(());
                }
            }
        }
    };

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn fetch_rows(client: &Client, cfg: &Config) -> Result<Vec<(String, String, String)>> {
    let mut job_dirs =
        client.list_dir(&cfg.owner, &cfg.results_repo, "jobs", &cfg.results_branch)?;
    job_dirs.sort();
    job_dirs.reverse(); // newest first
    job_dirs.truncate(30);

    let mut rows = Vec::new();
    for name in job_dirs {
        let path = format!("jobs/{name}/status.json");
        if let Some(f) =
            client.get_file(&cfg.owner, &cfg.results_repo, &path, &cfg.results_branch)?
        {
            let v: serde_json::Value = serde_json::from_slice(&f.content).unwrap_or_default();
            let status = v["status"].as_str().unwrap_or("?").to_string();
            let code = v["exit_code"]
                .as_i64()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string());
            rows.push((name, status, code));
        }
    }
    Ok(rows)
}
