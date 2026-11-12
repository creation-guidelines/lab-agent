//! oco-agent's core: the rules of coordinating work, with nothing in it that touches the world.
//!
//! It decides; the parts around it do - they keep the journal somewhere, run the jobs, tell the time
//! and carry the driver's beats. So every rule here can be tried without a machine, a network or a
//! clock, and replacing how any of those is done changes nothing in here.
//!
//! Every rule exists because of something that went wrong on 2026-11-11 (see DESIGN.md): a job had
//! no record of its own life, so a hundred finished jobs ran again.

pub mod deciding;
pub mod journal;
pub mod leases;
pub mod measure;
