//! Who may act, and until when. A lease is a promise with a time limit: kept alive by beats, gone
//! when they stop - so silence is noticed without anyone having to say goodbye. Each driver - a person
//! or a machine - has a lease of its own: a job starts only while whoever asked for it is still there.

use crate::journal::{Moment, Trouble};
use std::collections::HashMap;

/// The drivers' leases as the agent sees them at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Leases {
    /// When each driver was last heard from.
    pub heard: HashMap<String, Moment>,
    /// How long a beat keeps a lease alive, in seconds.
    pub lasts: u64,
}

impl Leases {
    /// Whether `driver` has been heard from recently enough.
    pub fn live(&self, driver: &str, now: Moment) -> bool {
        self.heard
            .get(driver)
            .is_some_and(|beat| now.saturating_sub(*beat) <= self.lasts)
    }
}

/// Where the beats are kept. A beat says "this driver was here at this moment"; it is what is known
/// now, not a history - a newer beat replaces an older one, and an older one never replaces a newer.
pub trait Beats {
    fn heard(&self) -> Result<HashMap<String, Moment>, Trouble>;
    fn beat(&mut self, driver: &str, at: Moment) -> Result<(), Trouble>;
}

/// Beats kept in memory: for trying the rules, and the plainest thing that passes the measure.
#[derive(Default)]
pub struct InMemory(HashMap<String, Moment>);

impl Beats for InMemory {
    fn heard(&self) -> Result<HashMap<String, Moment>, Trouble> {
        Ok(self.0.clone())
    }

    fn beat(&mut self, driver: &str, at: Moment) -> Result<(), Trouble> {
        let last = self.0.entry(driver.to_string()).or_insert(at);
        *last = (*last).max(at);
        Ok(())
    }
}
