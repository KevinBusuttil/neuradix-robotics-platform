//! Deterministic clock helpers.

use neuradix_time::{ClockDomain, ManualClock, Timestamp};

/// A [`ManualClock`] in the simulation domain starting at zero.
pub fn sim_clock() -> ManualClock {
    ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0))
}

/// A [`ManualClock`] in `domain` starting at `nanos` since the domain epoch.
pub fn manual(domain: ClockDomain, nanos: i128) -> ManualClock {
    ManualClock::starting_at(domain, nanos)
}
