//! Writes a small journal into the folder given, to look at: `cargo run --example a_small_journal -- FOLDER`.
use oco_agent_core::journal::{Entry, Happened, Journal, Outcome, Why};
use oco_agent_journal_in_tad::InTad;

fn main() {
    let folder = std::env::args().nth(1).expect("give a folder");
    let mut journal = InTad::open(&folder).expect("a folder to keep the journal in");
    let say = |job: &str, at, what| Entry {
        job: job.to_string(),
        at,
        what,
    };
    for entry in [
        say(
            "land the docs",
            100,
            Happened::Submitted {
                by: "the author".into(),
                start_by: 4000,
            },
        ),
        say("land the docs", 101, Happened::Claimed { run: 1 }),
        say(
            "roll out to 21 repositories",
            102,
            Happened::Submitted {
                by: "Claude".into(),
                start_by: 4000,
            },
        ),
        say("land the docs", 160, Happened::Finished(Outcome::Done)),
        say(
            "roll out to 21 repositories",
            161,
            Happened::Claimed { run: 1 },
        ),
        say(
            "roll out to 21 repositories",
            900,
            Happened::Parked(Why::TheAgentRestarted),
        ),
    ] {
        journal.record(entry).expect("each may follow the last");
    }
    println!(
        "a journal of {} entries in {folder}",
        journal.entries().expect("the journal answers").len()
    );
}
