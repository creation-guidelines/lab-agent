//! Saying: what drivers say to one another, kept in the order it was said. A driver is a person or a
//! machine; one who can be reached away from a keyboard - on a phone, today - has a voice on this
//! machine, and what is said to them is carried there. The words that open that voice (a topic's
//! name) never leave the machine: whoever says something never needs them.

use crate::journal::{Moment, Trouble};

/// Said to everyone.
pub const EVERYONE: &str = "*";

/// One thing said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    pub from: String,
    pub to: String,
    pub at: Moment,
    pub text: String,
    /// The one it is said to is needed now.
    pub urgent: bool,
    /// When it was carried to the one it was said to, if it was.
    pub told_at: Option<Moment>,
}

/// Where what was said is kept.
pub trait Sayings {
    fn said(&self) -> Result<Vec<Said>, Trouble>;
    fn say(&mut self, said: Said) -> Result<(), Trouble>;
    /// Writes down that the saying at `place` (counted from 1) was carried to whom it was said.
    fn told(&mut self, place: usize, at: Moment) -> Result<(), Trouble>;
}

/// Whatever carries words to a driver away from a keyboard.
pub trait Voice {
    /// Whether `driver` can be reached through this voice.
    fn reaches(&self, driver: &str) -> bool;
    fn tell(&mut self, said: &Said) -> Result<(), String>;
}

/// What `who` has yet to read: said to them or to everyone, by someone else, after place `after`.
pub fn for_whom<'a>(said: &'a [Said], who: &str, after: usize) -> Vec<(usize, &'a Said)> {
    said.iter()
        .enumerate()
        .map(|(i, s)| (i + 1, s))
        .filter(|(place, s)| *place > after && s.from != who && (s.to == who || s.to == EVERYONE))
        .collect()
}

/// Carries every saying not yet told to those this voice reaches; returns how many it carried. One
/// that cannot be carried is left for next time and nothing after it is tried: a voice that fails
/// (a server that counts messages by the day) is not asked again at once.
pub fn carry(
    sayings: &mut impl Sayings,
    voice: &mut impl Voice,
    now: Moment,
) -> Result<usize, Trouble> {
    let mut carried = 0;
    for (i, s) in sayings.said()?.iter().enumerate() {
        if s.told_at.is_some() || !voice.reaches(&s.to) {
            continue;
        }
        if voice.tell(s).is_err() {
            break;
        }
        sayings.told(i + 1, now)?;
        carried += 1;
    }
    Ok(carried)
}

/// Sayings kept in memory: for trying the rules, and the plainest thing that passes the measure.
#[derive(Default)]
pub struct InMemory(Vec<Said>);

impl Sayings for InMemory {
    fn said(&self) -> Result<Vec<Said>, Trouble> {
        Ok(self.0.clone())
    }

    fn say(&mut self, said: Said) -> Result<(), Trouble> {
        self.0.push(said);
        Ok(())
    }

    fn told(&mut self, place: usize, at: Moment) -> Result<(), Trouble> {
        match place.checked_sub(1).and_then(|i| self.0.get_mut(i)) {
            Some(said) => {
                said.told_at = Some(at);
                Ok(())
            }
            None => Err(Trouble::Unreachable {
                because: format!("nothing was said at place {place}"),
            }),
        }
    }
}
