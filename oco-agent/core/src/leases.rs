//! Who may act, and until when. A lease is a promise with a time limit: kept alive by beats, gone
//! when they stop - so silence is noticed without anyone having to say goodbye.

use crate::journal::Moment;

/// The driver's lease: whoever is driving the work - a person or a machine - beats while working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    /// When the driver last beat; never, if it has not.
    pub last_beat: Option<Moment>,
    /// How long a beat keeps the lease alive, in seconds.
    pub lasts: u64,
}

impl Lease {
    /// Whether the driver has been heard from recently enough.
    pub fn live(&self, now: Moment) -> bool {
        self.last_beat
            .is_some_and(|beat| now.saturating_sub(beat) <= self.lasts)
    }
}
