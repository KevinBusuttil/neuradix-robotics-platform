//! Clocks: the injectable source of time.
//!
//! Deterministic and replayable logic MUST obtain time from an injected
//! [`Clock`] rather than reading ambient wall-clock time directly. The
//! [`ManualClock`] makes deterministic tests and simulation possible with no
//! sleeping and no access to the system clock. [`SystemClock`] is the
//! non-deterministic production clock and must not be used inside logic that is
//! expected to replay identically.

use std::cell::Cell;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::domain::ClockDomain;
use crate::duration::Duration;
use crate::error::TimeError;
use crate::timestamp::Timestamp;

/// A source of [`Timestamp`]s in a fixed [`ClockDomain`].
pub trait Clock {
    /// The domain of timestamps produced by this clock.
    fn domain(&self) -> ClockDomain;

    /// The current time according to this clock.
    fn now(&self) -> Timestamp;
}

/// The non-deterministic system clock.
///
/// [`SystemClock::monotonic`] reads the OS monotonic clock; [`SystemClock::wall`]
/// reads UTC wall-clock time. Both read ambient time and therefore MUST NOT be
/// used inside deterministic or replayable logic — inject a [`ManualClock`]
/// there instead.
#[derive(Debug)]
pub struct SystemClock {
    domain: ClockDomain,
    monotonic_base: Instant,
}

impl SystemClock {
    /// A monotonic system clock whose epoch is the moment of construction.
    pub fn monotonic() -> Self {
        Self {
            domain: ClockDomain::Monotonic,
            monotonic_base: Instant::now(),
        }
    }

    /// A UTC wall-clock, counting nanoseconds from the Unix epoch.
    pub fn wall() -> Self {
        Self {
            domain: ClockDomain::Utc,
            monotonic_base: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn domain(&self) -> ClockDomain {
        self.domain
    }

    fn now(&self) -> Timestamp {
        match self.domain {
            ClockDomain::Utc => {
                // Duration since the epoch; if the wall clock is (improbably)
                // before 1970, saturate at zero rather than panic.
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as i128)
                    .unwrap_or(0);
                Timestamp::new(ClockDomain::Utc, nanos)
            }
            _ => {
                let nanos = self.monotonic_base.elapsed().as_nanos() as i128;
                Timestamp::new(self.domain, nanos)
            }
        }
    }
}

/// A manually advanced clock for deterministic tests, simulation and replay.
///
/// Interior mutability (via [`Cell`]) lets components hold a shared reference to
/// the clock while a driver advances it. It is intentionally single-threaded
/// (`!Sync`); a thread-safe simulation clock will be introduced when the
/// deterministic executor needs one.
#[derive(Debug)]
pub struct ManualClock {
    domain: ClockDomain,
    nanos: Cell<i128>,
}

impl ManualClock {
    /// Create a manual clock positioned at `start`.
    pub fn new(start: Timestamp) -> Self {
        Self {
            domain: start.domain(),
            nanos: Cell::new(start.as_nanos()),
        }
    }

    /// Create a manual clock in `domain` starting at `nanos` since the epoch.
    pub fn starting_at(domain: ClockDomain, nanos: i128) -> Self {
        Self {
            domain,
            nanos: Cell::new(nanos),
        }
    }

    /// Advance the clock by `delta`. Returns an error on overflow.
    pub fn advance(&self, delta: Duration) -> Result<(), TimeError> {
        let next = self
            .nanos
            .get()
            .checked_add(delta.as_nanos())
            .ok_or(TimeError::Overflow)?;
        self.nanos.set(next);
        Ok(())
    }

    /// Set the clock to an explicit timestamp in the same domain.
    pub fn set(&self, at: Timestamp) -> Result<(), TimeError> {
        if at.domain() != self.domain {
            return Err(TimeError::DomainMismatch {
                left: self.domain,
                right: at.domain(),
            });
        }
        self.nanos.set(at.as_nanos());
        Ok(())
    }
}

impl Clock for ManualClock {
    fn domain(&self) -> ClockDomain {
        self.domain
    }

    fn now(&self) -> Timestamp {
        Timestamp::new(self.domain, self.nanos.get())
    }
}

/// A [`Clock`] whose current time can be controlled.
///
/// This is the capability a deterministic executor, a simulator or a replay
/// driver needs: it can position the clock exactly (`set`) or step it forward
/// (`advance`). Only controllable clocks (e.g. [`ManualClock`]) implement it;
/// the ambient [`SystemClock`] deliberately does not, so deterministic drivers
/// cannot be handed a non-reproducible clock by mistake.
pub trait ControllableClock: Clock {
    /// Position the clock at `at` (same domain required).
    fn set(&self, at: Timestamp) -> Result<(), TimeError>;

    /// Step the clock forward by `delta`.
    fn advance(&self, delta: Duration) -> Result<(), TimeError>;
}

impl ControllableClock for ManualClock {
    fn set(&self, at: Timestamp) -> Result<(), TimeError> {
        ManualClock::set(self, at)
    }

    fn advance(&self, delta: Duration) -> Result<(), TimeError> {
        ManualClock::advance(self, delta)
    }
}
