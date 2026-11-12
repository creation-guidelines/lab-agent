//! The measure, taken of the journal kept in tad tables - and what is its own: the folder is a tad
//! dataset, two holders of one folder cannot both take a job, and a dead writer does not block it.

use oco_agent_core::journal::{Entry, Happened, Journal};
use oco_agent_core::measure;
use oco_agent_journal_in_tad::InTad;
use std::path::PathBuf;

fn folder(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oco-agent-journal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn fresh(name: &str) -> InTad {
    InTad::open(folder(name)).expect("a folder to keep the journal in")
}

#[test]
fn a_restart_never_replays_finished_jobs() {
    measure::a_restart_never_replays_finished_jobs(&mut fresh("replay"));
}

#[test]
fn many_finished_jobs_and_fifty_at_once_still_start_nothing() {
    measure::many_finished_jobs_and_fifty_at_once_still_start_nothing(&mut fresh("many"), 120);
}

#[test]
fn a_job_cut_off_by_a_restart_is_parked_not_run_again() {
    measure::a_job_cut_off_by_a_restart_is_parked_not_run_again(&mut fresh("cut-off"));
}

#[test]
fn a_silent_driver_starts_nothing_and_a_returning_one_carries_on() {
    measure::a_silent_driver_starts_nothing_and_a_returning_one_carries_on(&mut fresh("silent"));
}

#[test]
fn work_nobody_started_in_time_expires_instead_of_waiting_forever() {
    measure::work_nobody_started_in_time_expires_instead_of_waiting_forever(&mut fresh("stale"));
}

#[test]
fn two_agents_reaching_for_one_job_cannot_both_take_it() {
    measure::two_agents_reaching_for_one_job_cannot_both_take_it(&mut fresh("wanted"));
}

#[test]
fn what_is_finished_expired_or_unknown_cannot_be_claimed() {
    measure::what_is_finished_expired_or_unknown_cannot_be_claimed(&mut fresh("once"));
}

#[test]
fn no_more_run_side_by_side_than_allowed() {
    measure::no_more_run_side_by_side_than_allowed(&mut fresh("side-by-side"));
}

#[test]
fn it_gives_back_what_it_was_given_and_the_folder_is_a_tad_dataset() {
    let at = folder("given");
    measure::it_gives_back_what_it_was_given(&mut InTad::open(&at).unwrap());
    let declaration: tad::Declaration =
        toml::from_str(&std::fs::read_to_string(at.join("tables.toml")).unwrap()).unwrap();
    let (rows, problems) = tad::check(&at.join("data"), &declaration).unwrap();
    assert_eq!(
        (rows, problems),
        (8, vec![]),
        "tad finds the journal in order"
    );
    let text = std::fs::read_to_string(at.join("data/journal.json")).unwrap();
    assert!(text.starts_with("{\"place\":1,\"job\":\"a job with a name, in Кириллица\",\"at\":1,\"what\":\"submitted\",\"by\":\"the author\",\"start_by\":9,"), "one readable line per entry: {text}");
}

#[test]
fn two_holders_of_one_folder_cannot_both_take_a_job() {
    let at = folder("two-holders");
    let (mut one, mut two) = (InTad::open(&at).unwrap(), InTad::open(&at).unwrap());
    one.record(Entry {
        job: "wanted".into(),
        at: 0,
        what: Happened::Submitted {
            by: "a driver".into(),
            start_by: 100,
            may_run: 3600,
        },
    })
    .unwrap();
    assert!(one
        .record(Entry {
            job: "wanted".into(),
            at: 1,
            what: Happened::Claimed { run: 1 }
        })
        .is_ok());
    assert!(
        two.record(Entry {
            job: "wanted".into(),
            at: 1,
            what: Happened::Claimed { run: 2 }
        })
        .is_err(),
        "the second holder reads what the first wrote"
    );
    assert_eq!(two.entries().unwrap().len(), 2);
}

#[test]
fn a_writer_that_died_holding_its_turn_does_not_block_the_journal() {
    let at = folder("dead-writer");
    let mut journal = InTad::open(&at).unwrap();
    let turn = std::fs::File::create(at.join("journal.turn")).unwrap();
    turn.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(120))
        .unwrap();
    assert!(journal
        .record(Entry {
            job: "after the crash".into(),
            at: 0,
            what: Happened::Submitted {
                by: "a driver".into(),
                start_by: 100,
                may_run: 3600
            }
        })
        .is_ok());
}

#[test]
fn a_job_waits_for_its_own_driver() {
    measure::a_job_waits_for_its_own_driver(&mut fresh("own-driver"));
}

#[test]
fn beats_are_remembered_and_never_go_back() {
    let at = folder("beats");
    measure::beats_are_remembered_and_never_go_back(&mut InTad::open(&at).unwrap());
    assert_eq!(
        std::fs::read_to_string(at.join("data/beats.json")).unwrap(),
        "{\"driver\":\"ann\",\"at\":11}\n{\"driver\":\"bo\",\"at\":7}\n",
        "one line per driver, the latest beat"
    );
}
