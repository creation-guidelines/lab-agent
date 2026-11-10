//! The control board: what the agent is doing, the jobs, and the conversation with a reply box -
//! one live view of the whole system. `q` quits (when the reply box is empty), Ctrl-C always.
//! Tab moves between the panes; arrows, PgUp/PgDn and Home/End scroll the focused one.
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

/// The pane that scrolls when the arrow keys are pressed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Pane {
    #[default]
    Talk,
    Tasks,
    Jobs,
}

impl Pane {
    fn next(self) -> Pane {
        match self {
            Pane::Talk => Pane::Tasks,
            Pane::Tasks => Pane::Jobs,
            Pane::Jobs => Pane::Talk,
        }
    }
    fn prev(self) -> Pane {
        self.next().next()
    }
}

/// Where each pane is scrolled to. Tasks and jobs count rows from the top; the conversation counts
/// from the bottom, so that at 0 it follows new messages.
#[derive(Default, Clone, Debug)]
struct Scroll {
    focus: Pane,
    /// The selected task (in the tasks shown), and the one expanded to its full text, by id.
    selected: usize,
    expanded: Option<String>,
    /// Where each shown task was drawn (top row, height, id), so a click can find it; the first
    /// drawn is task number `hits_from` of those shown.
    task_hits: Vec<(u16, u16, String)>,
    hits_from: usize,
    /// The id of the selected task, as last drawn.
    selected_id: Option<String>,
    /// Where the quit button was drawn.
    quit_button: Option<Rect>,
    /// Where the panes were last drawn, so the mouse can find them.
    areas: Option<Areas>,
    jobs: usize,
    talk_back: usize,
    /// The project shown: 0 is all, then the board's projects in order.
    project: usize,
}

impl Scroll {
    fn task_hits_offset(&self) -> usize {
        self.hits_from
    }
}

/// An offset that keeps a page of `visible` rows inside `total`.
fn clamp_offset(offset: usize, total: usize, visible: usize) -> usize {
    offset.min(total.saturating_sub(visible))
}

/// Where a message the user just sent stands, until it shows up filed in the conversation.
#[derive(Clone, Debug, PartialEq)]
enum Delivery {
    Sending,
    Sent,
    Failed(String),
}

#[derive(Clone, Debug)]
struct Pending {
    id: u64,
    text: String,
    at: f64,
    state: Delivery,
}

/// Forgets the pending messages that are now filed in the conversation: same text, filed no
/// earlier than two minutes before it was sent. A failed one stays, so nothing is lost silently.
fn settle(pending: &mut Vec<Pending>, inbox: &Inbox) {
    pending.retain(|p| {
        p.state != Delivery::Sent
            || !inbox
                .messages
                .iter()
                .any(|m| m.text.trim() == p.text.trim() && m.time as f64 >= p.at - 120.0)
    });
}

#[derive(Default, Clone)]
struct Snapshot {
    board: Board,
    inbox: Inbox,
    jobs: Vec<JobRow>,
    /// this machine's log of what the agent sent (none when read from elsewhere)
    sent: Option<Vec<crate::notify::Sent>>,
    /// the version of the agent running on this machine (none when read from elsewhere)
    agent: Option<String>,
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
                sent: Some(crate::notify::read_sent(&dir.join(crate::notify::SENT_LOG))),
                agent: running_agent(&dir),
            };
            if tx.send(Ok(snap)).is_err() {
                return;
            }
            thread::sleep(LOCAL_EVERY);
        });
    }

    let (status_tx, status_rx) = mpsc::channel::<(u64, Result<(), String>)>();
    let (mut pending, mut next_id) = (Vec::<Pending>::new(), 0u64);
    let mut scroll = Scroll::default();
    let mut terminal = ratatui::try_init()?;
    // clicks select and expand tasks (shift-drag still selects text in most terminals); a paste
    // arrives as one block, so its newlines do not send it in pieces
    ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableMouseCapture,
        ratatui::crossterm::event::EnableBracketedPaste
    )?;
    let (mut snap, mut error, mut input) = (Snapshot::default(), None::<String>, String::new());
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
            while let Ok((id, result)) = status_rx.try_recv() {
                if let Some(p) = pending.iter_mut().find(|p| p.id == id) {
                    p.state = match result {
                        Ok(()) => Delivery::Sent,
                        Err(e) => Delivery::Failed(e),
                    };
                }
            }
            settle(&mut pending, &snap.inbox);
            terminal.draw(|f| {
                draw(
                    f,
                    &snap,
                    error.as_deref(),
                    source,
                    &input,
                    &pending,
                    &mut scroll,
                )
            })?;
            if !event::poll(TICK)? {
                continue;
            }
            let key = match event::read()? {
                Event::Key(key) => key,
                Event::Paste(text) => {
                    input.push_str(&pasted(&text));
                    continue;
                }
                Event::Mouse(mouse) => {
                    // the status bar's quit button
                    if let (event::MouseEventKind::Down(event::MouseButton::Left), Some(q)) =
                        (mouse.kind, scroll.quit_button)
                    {
                        if mouse.column >= q.x
                            && mouse.column < q.x + q.width
                            && mouse.row >= q.y
                            && mouse.row < q.y + q.height
                        {
                            return Ok(());
                        }
                    }
                    // the pane under the mouse takes the focus; the wheel scrolls it
                    let under = scroll
                        .areas
                        .and_then(|a| pane_at(&a, mouse.column, mouse.row));
                    if let Some(pane) = under {
                        scroll.focus = pane;
                    }
                    let wheel: isize = match mouse.kind {
                        event::MouseEventKind::ScrollUp => -1,
                        event::MouseEventKind::ScrollDown => 1,
                        _ => 0,
                    };
                    if wheel != 0 && under.is_some() {
                        let move_by = |v: usize, d: isize| (v as isize + d).max(0) as usize;
                        match scroll.focus {
                            Pane::Tasks => scroll.selected = move_by(scroll.selected, wheel),
                            Pane::Jobs => scroll.jobs = move_by(scroll.jobs, wheel),
                            Pane::Talk => scroll.talk_back = move_by(scroll.talk_back, -wheel),
                        }
                        continue;
                    }
                    if let event::MouseEventKind::Down(event::MouseButton::Left) = mouse.kind {
                        let hit = scroll
                            .task_hits
                            .iter()
                            .position(|(y, h, _)| mouse.row >= *y && mouse.row < y + h);
                        if let Some(i) = hit {
                            let id = scroll.task_hits[i].2.clone();
                            scroll.focus = Pane::Tasks;
                            scroll.expanded = if scroll.expanded.as_deref() == Some(id.as_str()) {
                                None
                            } else {
                                Some(id)
                            };
                            scroll.selected = i + scroll.task_hits_offset();
                        }
                    }
                    continue;
                }
                _ => continue,
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(())
                }
                KeyCode::Char('q') if input.is_empty() => return Ok(()),
                KeyCode::Tab => scroll.focus = scroll.focus.next(),
                // left and right switch the project shown (typing is for the reply box)
                KeyCode::Right => scroll.project += 1,
                KeyCode::Left => scroll.project = scroll.project.saturating_sub(1),
                KeyCode::BackTab => scroll.focus = scroll.focus.prev(),
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End => {
                    let step: isize = match key.code {
                        KeyCode::Up => -1,
                        KeyCode::Down => 1,
                        KeyCode::PageUp => -10,
                        KeyCode::PageDown => 10,
                        KeyCode::Home => -100_000,
                        _ => 100_000,
                    };
                    let move_by = |v: usize, d: isize| (v as isize + d).max(0) as usize;
                    match scroll.focus {
                        Pane::Tasks => scroll.selected = move_by(scroll.selected, step),
                        Pane::Jobs => scroll.jobs = move_by(scroll.jobs, step),
                        // the conversation is measured from its end: up goes back in time
                        Pane::Talk => scroll.talk_back = move_by(scroll.talk_back, -step),
                    }
                }
                KeyCode::Char(c) => input.push(c),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Esc => input.clear(),
                // Enter on an empty reply box opens or closes the selected task
                KeyCode::Enter if input.trim().is_empty() && scroll.focus == Pane::Tasks => {
                    scroll.expanded = match scroll.expanded.take() {
                        Some(id) if Some(&id) == scroll.selected_id.as_ref() => None,
                        _ => scroll.selected_id.clone(),
                    };
                }
                KeyCode::Enter if !input.trim().is_empty() => {
                    let typed = std::mem::take(&mut input).trim().to_string();
                    // a choice for the question the agent is waiting on answers it
                    let text = snap
                        .board
                        .questions
                        .last()
                        .and_then(|q| {
                            board::choice_for(q, &typed).map(|c| board::answer_text(&q.id, &c))
                        })
                        .unwrap_or(typed);
                    next_id += 1;
                    pending.push(Pending {
                        id: next_id,
                        text: text.clone(),
                        at: now(),
                        state: Delivery::Sending,
                    });
                    let (tx, id) = (status_tx.clone(), next_id);
                    thread::spawn(move || {
                        // straight into the inbox on GitHub (and the local copy): no ntfy
                        let result = board::file_from_here(&text).map_err(|e| e.to_string());
                        let _ = tx.send((id, result));
                    });
                }
                _ => {}
            }
        }
    })();
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableMouseCapture,
        ratatui::crossterm::event::DisableBracketedPaste
    );
    ratatui::restore();
    result
}

/// Rows `text` takes inside a box `width` columns wide (borders included), wrapped exactly the
/// way it is drawn.
fn wrapped_rows(text: &str, width: u16) -> usize {
    Paragraph::new(text.to_string())
        .wrap(Wrap { trim: false })
        .line_count(width.saturating_sub(2).max(1))
}

/// The reply box's height: what is typed (and the cursor) as drawn, plus its borders. It grows
/// with the message until three lines of the window are left above it - the conversation keeps at
/// least those (its newest line, framed), the task and job panes give up their room first.
fn input_height(text_rows: usize, window: u16) -> u16 {
    let most = window.saturating_sub(3).max(3);
    (text_rows.min(usize::from(u16::MAX)) as u16 + 2).clamp(3, most)
}

/// `text` broken into lines of at most `width` characters, at spaces where it can be.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let line = lines.last_mut().expect("never empty");
            let used = line.chars().count();
            let gap = usize::from(used > 0);
            if used + gap + word.len() <= width {
                if gap == 1 {
                    line.push(' ');
                }
                line.extend(word.iter());
                break;
            }
            if used > 0 {
                lines.push(String::new());
                continue;
            }
            // a word longer than a line: split it
            let rest = word.split_off(width);
            line.extend(word.iter());
            word = rest;
            lines.push(String::new());
        }
    }
    lines
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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Areas {
    tasks: Rect,
    jobs: Rect,
    talk: Rect,
    input: Rect,
}

/// `input` is the reply box's height, borders included (it grows with what is typed).
/// The pane at a point on the screen; the reply box belongs to the conversation.
fn pane_at(a: &Areas, column: u16, row: u16) -> Option<Pane> {
    let inside =
        |r: Rect| column >= r.x && column < r.x + r.width && row >= r.y && row < r.y + r.height;
    if inside(a.tasks) {
        Some(Pane::Tasks)
    } else if inside(a.jobs) {
        Some(Pane::Jobs)
    } else if inside(a.talk) || inside(a.input) {
        Some(Pane::Talk)
    } else {
        None
    }
}

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
                &[Percentage(45), Min(3), Length(input)],
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
                &[Percentage(28), Min(3), Length(input), Percentage(24)],
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
        State::Planned => ("○", Style::default().fg(Color::Gray)),
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

/// A message the user sent that is not filed yet: shown at once, with where it stands.
fn on_its_way(p: &Pending) -> Line<'static> {
    let (note, style) = match &p.state {
        Delivery::Sending => (
            " · sending…".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        Delivery::Sent => (
            " · sent".to_string(),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Delivery::Failed(why) => (
            format!(" ✗ not sent: {why}"),
            Style::default().fg(Color::Red),
        ),
    };
    Line::from(vec![
        Span::styled(
            format!("{} ", jobs::format_time(p.at)),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::styled(
            "you: ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(p.text.clone()),
        Span::styled(note, style),
    ])
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

/// The project tabs and the one selected: "all" first, then each project.
fn project_tabs(projects: &[String], selected: usize) -> (String, Option<String>) {
    let selected = selected.min(projects.len());
    let names: Vec<String> = std::iter::once("all".to_string())
        .chain(projects.iter().cloned())
        .collect();
    let tabs = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if i == selected {
                format!("[{n}]")
            } else {
                n.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (tabs, (selected > 0).then(|| projects[selected - 1].clone()))
}

/// A pane's frame: highlighted when focused, with a hint that it scrolls.
fn frame(title: String, focused: bool) -> Block<'static> {
    let block = Block::default().borders(Borders::ALL);
    if focused {
        block
            .title(format!("{title}[↑↓ PgUp PgDn] "))
            .border_style(Style::default().fg(Color::Cyan))
    } else {
        block.title(title)
    }
}

fn draw(
    f: &mut Frame,
    snap: &Snapshot,
    error: Option<&str>,
    source: &str,
    input: &str,
    pending: &[Pending],
    scroll: &mut Scroll,
) {
    // the box's width does not depend on its height: measure with one row, then grow it
    // the status bar takes the first line; the panes share the rest
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(f.area());
    let (bar, main) = (rows[0], rows[1]);
    let width = layout(main, 3).input.width;
    let a = layout(
        main,
        input_height(wrapped_rows(&format!("{input}▏"), width), main.height),
    );
    scroll.areas = Some(a);
    let at = now();
    let title = match error {
        Some(e) => format!(" control board ({source}) - cannot read: {e} "),
        None => format!(
            " control board ({source}, local time) - {} live tasks ",
            snap.board.live()
        ),
    };
    // the project shown: tabs in the title, ← → to switch
    let projects = snap.board.projects();
    scroll.project = scroll.project.min(projects.len());
    let (tabs, shown) = project_tabs(&projects, scroll.project);
    let title = format!("{title}- {tabs} (← →) ");
    let shown_tasks: Vec<&board::Task> = snap
        .board
        .ordered()
        .into_iter()
        .filter(|t| shown.is_none() || t.project == shown)
        .collect();
    scroll.selected = scroll.selected.min(shown_tasks.len().saturating_sub(1));
    scroll.selected_id = shown_tasks.get(scroll.selected).map(|t| t.id.clone());
    // the text column: what is left after the state and age columns, the borders and the gaps
    let text_width = usize::from(a.tasks.width.saturating_sub(2 + 17 + 15 + 2)).max(8);
    let focused = scroll.focus == Pane::Tasks;
    let task_rows: Vec<Row> = shown_tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let (mark, style) = look(t.state);
            let open = scroll.expanded.as_deref() == Some(t.id.as_str());
            let mut text = wrap_text(&t.text, text_width);
            if !open && text.len() > 1 {
                let first: String = text[0].chars().take(text_width.saturating_sub(1)).collect();
                text = vec![format!("{first}…")];
            }
            let height = text.len().max(1) as u16;
            let row = Row::new(vec![
                Cell::from(format!("{mark} {}", t.state.word())).style(style),
                Cell::from(text.join("\n")),
                Cell::from(age(t, at)),
            ])
            .height(height);
            if focused && i == scroll.selected {
                row.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                row
            }
        })
        .collect();
    let mut state = ratatui::widgets::TableState::default().with_selected(Some(scroll.selected));
    f.render_stateful_widget(
        Table::new(
            task_rows,
            [
                Constraint::Length(17),
                Constraint::Min(10),
                Constraint::Length(15),
            ],
        )
        .block(frame(title, focused)),
        a.tasks,
        &mut state,
    );
    // where each task landed, so a click can find it
    scroll.hits_from = state.offset();
    scroll.task_hits.clear();
    let mut y = a.tasks.y + 1;
    for t in shown_tasks.iter().skip(state.offset()) {
        let open = scroll.expanded.as_deref() == Some(t.id.as_str());
        let h = if open {
            wrap_text(&t.text, text_width).len().max(1) as u16
        } else {
            1
        };
        if y >= a.tasks.y + a.tasks.height.saturating_sub(1) {
            break;
        }
        scroll.task_hits.push((y, h, t.id.clone()));
        y += h;
    }
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
        .block(frame(" jobs ".into(), scroll.focus == Pane::Jobs)),
        a.jobs,
    );
    // the conversation, newest at the bottom: scroll so the end is in view
    let mut lines: Vec<Line> =
        board::conversation_with(&snap.inbox, &snap.board, snap.sent.as_deref())
            .iter()
            .map(said)
            .collect();
    lines.extend(pending.iter().map(on_its_way));
    // questions the agent is waiting on, last, so they are always in view
    for q in &snap.board.questions {
        let ask = Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD);
        lines.push(Line::from(Span::styled(
            format!("? Claude asks: {}", q.text),
            ask,
        )));
        let options: Vec<String> = q
            .choices
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{} = {c}", i + 1))
            .collect();
        lines.push(Line::from(Span::styled(
            format!("  answer here: type {} and press Enter", options.join(", ")),
            Style::default().fg(Color::Magenta),
        )));
    }
    let talk = Paragraph::new(lines).wrap(Wrap { trim: false });
    let rows = talk.line_count(a.talk.width.saturating_sub(2).max(1));
    let visible = a.talk.height.saturating_sub(2) as usize;
    scroll.talk_back = clamp_offset(scroll.talk_back, rows, visible);
    let top = rows
        .saturating_sub(visible)
        .saturating_sub(scroll.talk_back) as u16;
    f.render_widget(
        talk.scroll((top, 0)).block(frame(
            " conversation (Tab: next pane) ".into(),
            scroll.focus == Pane::Talk,
        )),
        a.talk,
    );
    let hint = " say something - Enter sends, Esc clears ".to_string();
    f.render_widget(
        Paragraph::new(format!("{input}▏"))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(hint)),
        a.input,
    );
    // the status bar: what the agent is doing or waiting for, and a quit button
    let quit = " Quit (Ctrl-C) ";
    let version = agent_label(snap.agent.as_deref(), env!("CARGO_PKG_VERSION"));
    let cols = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(version.chars().count() as u16),
        Constraint::Length(quit.chars().count() as u16),
    ])
    .split(bar);
    f.render_widget(
        Paragraph::new(version).style(Style::default().fg(Color::White).bg(Color::DarkGray)),
        cols[1],
    );
    f.render_widget(
        Paragraph::new(format!(" \u{25cf} Claude {}", agent_state(&snap.board, at)))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        cols[0],
    );
    f.render_widget(
        Paragraph::new(quit).style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        cols[2],
    );
    scroll.quit_button = Some(cols[2]);
}

/// Pasted text for the reply box: whole, its line ends made plain newlines.
fn pasted(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The version of the agent running on this machine, from the file it writes when it starts.
fn running_agent(state_dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(state_dir.join(crate::daemon::AGENT_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["version"].as_str().map(str::to_string)
}

/// Which agent runs, for the status bar - and the newer one installed, while it waits for idle.
fn agent_label(running: Option<&str>, installed: &str) -> String {
    match running {
        Some(v) if v != installed => format!(" agent {v} \u{2192} {installed} when idle "),
        Some(v) => format!(" agent {v} "),
        None => String::new(),
    }
}

/// What the agent is doing, for the status bar: its wait with the time left, else the task it
/// started most recently, else idle.
fn agent_state(board: &Board, at: f64) -> String {
    if let Some(w) = board.agent_wait.as_ref().filter(|w| w.until > at) {
        let left = (w.until - at) as u64;
        return format!("waits {}:{:02} for {}", left / 60, left % 60, w.what);
    }
    board
        .tasks
        .iter()
        .filter(|t| t.state == State::Ongoing)
        .max_by(|a, b| {
            let when = |t: &crate::board::Task| t.touched.unwrap_or(t.since);
            when(a)
                .partial_cmp(&when(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|t| format!("is working on: {}", t.text))
        .unwrap_or_else(|| "is idle".to_string())
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
        let states = [
            State::Waiting,
            State::Ongoing,
            State::Paused,
            State::Planned,
            State::Done,
        ];
        let marks: std::collections::HashSet<&str> = states.iter().map(|s| look(*s).0).collect();
        let colors: std::collections::HashSet<_> = states
            .iter()
            .map(|s| format!("{:?}", look(*s).1.fg))
            .collect();
        assert_eq!((marks.len(), colors.len()), (5, 5));
    }

    #[test]
    fn the_reply_box_grows_on_the_first_wrapped_sign_until_three_lines_are_left() {
        // 40 columns inside the borders; words wrap whole, so a word that does not fit moves down
        assert_eq!(wrapped_rows(&format!("{}▏", "a".repeat(39)), 42), 1);
        assert_eq!(
            wrapped_rows(&format!("{}▏", "a".repeat(40)), 42),
            2,
            "one sign over: a new line"
        );
        assert_eq!(
            wrapped_rows(&format!("{} {}▏", "a".repeat(30), "b".repeat(15)), 42),
            2,
            "the word moves whole"
        );
        assert_eq!(input_height(1, 40), 3);
        assert_eq!(
            input_height(100, 40),
            37,
            "three lines of the window stay above it"
        );
        let small = layout(r(48, 90), 3);
        let grown = layout(r(48, 90), 20);
        assert!(
            grown.input.height == 20 && grown.talk.height < small.talk.height,
            "the conversation gives up the room"
        );
    }

    #[test]
    fn task_text_wraps_at_spaces_and_splits_words_longer_than_a_line() {
        assert_eq!(
            wrap_text("reply box grows with the text", 10),
            ["reply box", "grows with", "the text"]
        );
        assert_eq!(wrap_text("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
        assert_eq!(wrap_text("", 5), [""]);
    }

    #[test]
    fn scrolling_stops_at_the_ends() {
        assert_eq!(clamp_offset(0, 30, 10), 0);
        assert_eq!(clamp_offset(25, 30, 10), 20, "the last page stays full");
        assert_eq!(
            clamp_offset(5, 3, 10),
            0,
            "nothing to scroll when it all fits"
        );
    }

    #[test]
    fn tab_visits_every_pane_and_comes_back() {
        let p = Pane::Talk;
        assert_eq!(
            (p.next(), p.next().next(), p.next().next().next()),
            (Pane::Tasks, Pane::Jobs, Pane::Talk)
        );
        assert_eq!(p.prev(), Pane::Jobs);
    }

    #[test]
    fn a_sent_message_stays_until_it_is_filed_and_a_failed_one_stays() {
        let p = |id, text: &str, state| Pending {
            id,
            text: text.into(),
            at: 1000.0,
            state,
        };
        let mut pending = vec![
            p(1, "ready", Delivery::Sent),
            p(2, "hello", Delivery::Failed("offline".into())),
            p(3, "later", Delivery::Sent),
        ];
        let inbox = Inbox {
            messages: vec![board::Message {
                id: "m".into(),
                time: 1001,
                text: "ready ".into(),
            }],
        };
        settle(&mut pending, &inbox);
        let left: Vec<u64> = pending.iter().map(|p| p.id).collect();
        assert_eq!(
            left,
            [2, 3],
            "filed 'ready' goes; the failed one and the unfiled one stay"
        );
    }

    #[test]
    fn the_project_tabs_start_with_all_and_mark_the_one_shown() {
        let p = vec!["lab-agent".to_string(), "surya-audio".to_string()];
        assert_eq!(
            project_tabs(&p, 0),
            ("[all] lab-agent surya-audio".to_string(), None)
        );
        assert_eq!(project_tabs(&p, 2).1, Some("surya-audio".to_string()));
        assert_eq!(
            project_tabs(&p, 9).1,
            Some("surya-audio".to_string()),
            "past the end stays on the last"
        );
    }

    /// What a person sees at a window size: the screen drawn into memory, as text.
    fn screen(w: u16, h: u16, snap: &Snapshot, input: &str, scroll: &mut Scroll) -> String {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, snap, None, "test", input, &[], scroll))
            .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn busy_board() -> Snapshot {
        let long =
            "a long task description that will not fit on one line of a narrow window at all";
        let tasks = (0..12)
            .map(|i| board::Task {
                id: format!("t{i}"),
                text: format!("{long} {i}"),
                since: 0.0,
                touched: None,
                state: State::Ongoing,
                done_at: None,
                project: Some(
                    if i % 2 == 0 {
                        "surya-audio"
                    } else {
                        "lab-agent"
                    }
                    .into(),
                ),
            })
            .collect();
        let messages = (0..30)
            .map(|i| board::Message {
                id: format!("m{i}"),
                time: i,
                text: format!(
                    "message {i}: {}",
                    "words that wrap around the edge ".repeat(i as usize % 5 + 1)
                ),
            })
            .chain(std::iter::once(board::Message {
                id: "last".into(),
                time: 99,
                text: "the newest one ends with ZQZ".into(),
            }))
            .collect();
        Snapshot {
            board: Board {
                tasks,
                ..Default::default()
            },
            inbox: Inbox { messages },
            jobs: vec![],
            sent: None,
            agent: None,
        }
    }

    #[test]
    fn at_every_window_size_the_newest_message_and_the_reply_box_stay_in_view() {
        let snap = busy_board();
        let typed = "a reply being typed that is long enough to wrap over several lines in a narrow box QXQ";
        let mut scroll = Scroll::default();
        // one session resized through all of these, as a tiling window manager would
        for (w, h) in [
            (250, 80),
            (200, 50),
            (100, 50),
            (80, 24),
            (48, 90),
            (40, 30),
            (30, 16),
            (100, 50),
            (60, 20),
        ] {
            let shown = screen(w, h, &snap, typed, &mut scroll);
            assert!(
                shown.contains("ZQZ"),
                "newest message hidden at {w}x{h}:\n{shown}"
            );
            assert!(
                shown.contains("QXQ"),
                "the end of the reply hidden at {w}x{h}:\n{shown}"
            );
        }
    }

    #[test]
    fn a_window_too_small_for_everything_still_draws() {
        let snap = busy_board();
        for (w, h) in [(1, 1), (10, 4), (20, 8), (12, 30)] {
            let _ = screen(w, h, &snap, "typing", &mut Scroll::default());
        }
    }

    #[test]
    fn the_pane_under_the_mouse_is_found_and_the_reply_box_belongs_to_the_conversation() {
        let a = layout(r(200, 50), 3);
        assert_eq!(pane_at(&a, a.tasks.x + 1, a.tasks.y + 1), Some(Pane::Tasks));
        assert_eq!(pane_at(&a, a.jobs.x + 1, a.jobs.y + 1), Some(Pane::Jobs));
        assert_eq!(pane_at(&a, a.talk.x + 1, a.talk.y + 1), Some(Pane::Talk));
        assert_eq!(pane_at(&a, a.input.x + 1, a.input.y + 1), Some(Pane::Talk));
        assert_eq!(pane_at(&a, 500, 500), None);
    }

    #[test]
    fn the_status_bar_says_what_the_agent_waits_for_or_works_on() {
        let mut b = Board::default();
        assert_eq!(agent_state(&b, 0.0), "is idle");
        board::apply(&mut b, "set", Some("a"), Some("an old task"), 10.0).unwrap();
        board::apply(&mut b, "set", Some("b"), Some("the newest task"), 20.0).unwrap();
        assert_eq!(agent_state(&b, 30.0), "is working on: the newest task");
        board::apply(&mut b, "waiting", Some("125"), Some("your go"), 30.0).unwrap();
        assert_eq!(agent_state(&b, 30.0), "waits 2:05 for your go");
        assert_eq!(
            agent_state(&b, 200.0),
            "is working on: the newest task",
            "a wait that has passed is gone"
        );
    }

    #[test]
    fn the_status_bar_shows_which_agent_runs_and_a_waiting_update() {
        assert_eq!(agent_label(Some("0.1.29"), "0.1.29"), " agent 0.1.29 ");
        assert_eq!(
            agent_label(Some("0.1.28"), "0.1.29"),
            " agent 0.1.28 \u{2192} 0.1.29 when idle "
        );
        assert_eq!(
            agent_label(None, "0.1.29"),
            "",
            "no agent on this machine: nothing shown"
        );
    }

    #[test]
    fn a_paste_keeps_its_lines_and_plain_newlines() {
        assert_eq!(pasted("one\r\ntwo\rthree\nfour"), "one\ntwo\nthree\nfour");
        assert_eq!(pasted("no newline"), "no newline");
    }

    #[test]
    fn the_status_bar_follows_the_task_touched_last_not_the_one_started_last() {
        let mut b = Board::default();
        board::apply(&mut b, "set", Some("old"), Some("started first"), 10.0).unwrap();
        board::apply(&mut b, "set", Some("new"), Some("started later"), 20.0).unwrap();
        board::apply(&mut b, "set", Some("old"), Some("updated just now"), 30.0).unwrap();
        assert_eq!(agent_state(&b, 31.0), "is working on: updated just now");
    }
}
