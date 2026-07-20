//! A link-loss watchdog.
//!
//! The watchdog is fed whenever a fresh command arrives. If no command arrives
//! within the timeout, the link is considered lost and the node must enter its
//! local safe state (NRX-EMB-004). Wireless/serial links are never trusted as a
//! safety channel; the safe response is local and time-driven.

use neuradix_time::{Duration, Timestamp};

/// A time-based link-loss watchdog.
#[derive(Debug, Clone, Copy)]
pub struct Watchdog {
    timeout: Duration,
    last_fed: Option<Timestamp>,
}

impl Watchdog {
    /// A watchdog that trips when no command has arrived within `timeout`.
    ///
    /// It starts **un-fed**, so it reports expired until the first command — a
    /// node has no authority before it has heard from its commander.
    pub const fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            last_fed: None,
        }
    }

    /// Record that a fresh command arrived at `now`.
    pub fn feed(&mut self, now: Timestamp) {
        self.last_fed = Some(now);
    }

    /// The last time the watchdog was fed, if ever.
    pub fn last_fed(&self) -> Option<Timestamp> {
        self.last_fed
    }

    /// Whether the link is considered lost at `now`.
    ///
    /// Expired if never fed, if `now` is in a different clock domain than the
    /// last feed, or if more than `timeout` has elapsed since the last feed.
    /// Elapsed time is computed with a saturating subtraction so an out-of-order
    /// or extreme timestamp cannot overflow.
    pub fn is_expired(&self, now: Timestamp) -> bool {
        match self.last_fed {
            None => true,
            Some(fed) => {
                now.domain() != fed.domain()
                    || now.as_nanos().saturating_sub(fed.as_nanos()) > self.timeout.as_nanos()
            }
        }
    }
}
