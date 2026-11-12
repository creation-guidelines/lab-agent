//! The journal kept in tad tables: one row per entry in `data/journal.json`, declared in the
//! `tables.toml` beside it - and with it the drivers' beats, one row per driver in `data/beats.json`. The folder is an ordinary tad dataset - files you can read and diff, text
//! you can reshape, a table you can ask in SQL - and it passes the checks every journal must pass.

use oco_agent_core::journal::{
    refuse_unless_it_may_follow, Entry, Happened, Journal, Moment, Outcome, Trouble, Why,
};
use oco_agent_core::leases::Beats;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const TABLES: &str = include_str!("../tables.toml");

/// How long a writer may hold its turn before it is taken to have died.
const A_TURN_LASTS: Duration = Duration::from_secs(30);

/// A journal kept in a folder, as tad tables.
pub struct InTad {
    folder: PathBuf,
    declaration: tad::Declaration,
}

fn unreachable(because: impl std::fmt::Display) -> Trouble {
    Trouble::Unreachable {
        because: because.to_string(),
    }
}

impl InTad {
    /// The journal kept in `folder`, made there if there is none yet.
    pub fn open(folder: impl Into<PathBuf>) -> Result<InTad, Trouble> {
        let folder = folder.into();
        std::fs::create_dir_all(folder.join("data")).map_err(unreachable)?;
        let tables = folder.join("tables.toml");
        // the declaration is this adapter's own: a folder made by an older one is brought up to it
        if std::fs::read_to_string(&tables).ok().as_deref() != Some(TABLES) {
            std::fs::write(&tables, TABLES).map_err(unreachable)?;
        }
        let declaration = toml::from_str(TABLES).map_err(unreachable)?;
        Ok(InTad {
            folder,
            declaration,
        })
    }

    fn data(&self) -> PathBuf {
        self.folder.join("data")
    }

    fn read(&self) -> Result<Vec<Entry>, Trouble> {
        let text = match std::fs::read_to_string(self.data().join("journal.json")) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(unreachable(e)),
        };
        let mut placed: Vec<(u64, Entry)> = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Value>(line) {
                Ok(Value::Object(row)) => placed.push(entry_of(&row)?),
                _ => return Err(unreachable("a line of the journal is not a row")),
            }
        }
        placed.sort_by_key(|(place, _)| *place);
        Ok(placed.into_iter().map(|(_, entry)| entry).collect())
    }
}

fn whole(row: &Map<String, Value>, field: &str) -> Result<u64, Trouble> {
    row.get(field).and_then(Value::as_u64).ok_or_else(|| {
        unreachable(format!(
            "a row of the journal has no whole number in {field}"
        ))
    })
}

fn words<'a>(row: &'a Map<String, Value>, field: &str) -> &'a str {
    row.get(field).and_then(Value::as_str).unwrap_or("")
}

/// A row of the table, read as the entry it records.
fn entry_of(row: &Map<String, Value>) -> Result<(u64, Entry), Trouble> {
    let what = match words(row, "what") {
        "submitted" => Happened::Submitted {
            by: words(row, "by").to_string(),
            start_by: whole(row, "start_by")?,
            may_run: row.get("may_run").and_then(Value::as_u64).unwrap_or(3600),
        },
        "claimed" => Happened::Claimed {
            run: whole(row, "run")?,
        },
        "finished" => Happened::Finished(match words(row, "outcome") {
            "done" => Outcome::Done,
            "failed" => Outcome::Failed,
            "timed-out" => Outcome::TimedOut,
            other => {
                return Err(unreachable(format!(
                    "the journal names an outcome this agent does not know: {other}"
                )))
            }
        }),
        "parked" => Happened::Parked(Why::TheAgentRestarted),
        "released" => Happened::Released {
            start_by: whole(row, "start_by")?,
        },
        "expired" => Happened::Expired,
        other => {
            return Err(unreachable(format!(
                "the journal names something this agent does not know: {other}"
            )))
        }
    };
    Ok((
        whole(row, "place")?,
        Entry {
            job: words(row, "job").to_string(),
            at: whole(row, "at")?,
            what,
        },
    ))
}

/// An entry, as the fields of its row.
fn row_of(entry: &Entry) -> Vec<(String, String)> {
    let mut row = vec![
        ("job".to_string(), entry.job.clone()),
        ("at".to_string(), entry.at.to_string()),
    ];
    let mut put = |field: &str, value: String| row.push((field.to_string(), value));
    match &entry.what {
        Happened::Submitted {
            by,
            start_by,
            may_run,
        } => {
            put("what", "submitted".into());
            put("by", by.clone());
            put("start_by", start_by.to_string());
            put("may_run", may_run.to_string());
        }
        Happened::Claimed { run } => {
            put("what", "claimed".into());
            put("run", run.to_string());
        }
        Happened::Finished(outcome) => {
            put("what", "finished".into());
            put(
                "outcome",
                match outcome {
                    Outcome::Done => "done",
                    Outcome::Failed => "failed",
                    Outcome::TimedOut => "timed-out",
                }
                .into(),
            );
        }
        Happened::Parked(Why::TheAgentRestarted) => {
            put("what", "parked".into());
            put("why", "the-agent-restarted".into());
        }
        Happened::Released { start_by } => {
            put("what", "released".into());
            put("start_by", start_by.to_string());
        }
        Happened::Expired => put("what", "expired".into()),
    }
    row
}

/// One writer at a time: whoever makes the turn file writes, and it goes when they are done. A writer
/// that died holding its turn does not block the journal: an old turn is taken over.
struct Turn(PathBuf);

impl Turn {
    fn take(folder: &Path) -> Result<Turn, Trouble> {
        let file = folder.join("journal.turn");
        for _ in 0..500 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)
            {
                Ok(_) => return Ok(Turn(file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let held_for = std::fs::metadata(&file)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok());
                    if held_for.is_some_and(|d| d > A_TURN_LASTS) {
                        let _ = std::fs::remove_file(&file);
                    } else {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                Err(e) => return Err(unreachable(e)),
            }
        }
        Err(unreachable("another writer holds the journal"))
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Journal for InTad {
    fn entries(&self) -> Result<Vec<Entry>, Trouble> {
        self.read()
    }

    fn record(&mut self, entry: Entry) -> Result<(), Trouble> {
        let _turn = Turn::take(&self.folder)?;
        let entries = self.read()?;
        refuse_unless_it_may_follow(&entries, &entry)?;
        let place = (entries.len() + 1).to_string();
        tad::set(
            &self.data(),
            &self.declaration,
            "journal",
            &[place],
            &row_of(&entry),
        )
        .map_err(unreachable)?;
        Ok(())
    }
}

impl Beats for InTad {
    fn heard(&self) -> Result<HashMap<String, Moment>, Trouble> {
        let text = match std::fs::read_to_string(self.data().join("beats.json")) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(unreachable(e)),
        };
        let mut heard = HashMap::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Value>(line) {
                Ok(Value::Object(row)) => {
                    heard.insert(words(&row, "driver").to_string(), whole(&row, "at")?)
                }
                _ => return Err(unreachable("a line of the beats is not a row")),
            };
        }
        Ok(heard)
    }

    fn beat(&mut self, driver: &str, at: Moment) -> Result<(), Trouble> {
        let _turn = Turn::take(&self.folder)?;
        if self.heard()?.get(driver).is_some_and(|last| *last >= at) {
            return Ok(());
        }
        tad::set(
            &self.data(),
            &self.declaration,
            "beats",
            &[driver.to_string()],
            &[("at".to_string(), at.to_string())],
        )
        .map_err(unreachable)?;
        Ok(())
    }
}
