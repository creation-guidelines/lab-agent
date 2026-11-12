//! The measure, taken of the journal kept in memory.

use oco_agent_core::journal::InMemory;
use oco_agent_core::measure;

#[test]
fn a_restart_never_replays_finished_jobs() {
    measure::a_restart_never_replays_finished_jobs(&mut InMemory::default());
}

#[test]
fn a_thousand_finished_jobs_and_fifty_at_once_still_start_nothing() {
    measure::many_finished_jobs_and_fifty_at_once_still_start_nothing(
        &mut InMemory::default(),
        1000,
    );
}

#[test]
fn a_job_cut_off_by_a_restart_is_parked_not_run_again() {
    measure::a_job_cut_off_by_a_restart_is_parked_not_run_again(&mut InMemory::default());
}

#[test]
fn a_silent_driver_starts_nothing_and_a_returning_one_carries_on() {
    measure::a_silent_driver_starts_nothing_and_a_returning_one_carries_on(&mut InMemory::default());
}

#[test]
fn work_nobody_started_in_time_expires_instead_of_waiting_forever() {
    measure::work_nobody_started_in_time_expires_instead_of_waiting_forever(
        &mut InMemory::default(),
    );
}

#[test]
fn two_agents_reaching_for_one_job_cannot_both_take_it() {
    measure::two_agents_reaching_for_one_job_cannot_both_take_it(&mut InMemory::default());
}

#[test]
fn what_is_finished_expired_or_unknown_cannot_be_claimed() {
    measure::what_is_finished_expired_or_unknown_cannot_be_claimed(&mut InMemory::default());
}

#[test]
fn no_more_run_side_by_side_than_allowed() {
    measure::no_more_run_side_by_side_than_allowed(&mut InMemory::default());
}

#[test]
fn it_gives_back_what_it_was_given() {
    measure::it_gives_back_what_it_was_given(&mut InMemory::default());
}

#[test]
fn a_job_waits_for_its_own_driver() {
    measure::a_job_waits_for_its_own_driver(&mut InMemory::default());
}

#[test]
fn beats_are_remembered_and_never_go_back() {
    measure::beats_are_remembered_and_never_go_back(
        &mut oco_agent_core::leases::InMemory::default(),
    );
}
