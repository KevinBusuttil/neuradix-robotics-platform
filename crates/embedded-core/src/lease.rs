//! The authority lease — time-bounded permission to actuate.
//!
//! A node applies external commands only while it holds a valid lease. This is
//! the embedded mirror of `neuradix_safety`'s authority lease: expiry is
//! time-based, and a lapsed lease forces the local safe state (§16, NRX-EMB-004).

use neuradix_time::Timestamp;

/// A time-bounded grant of authority to actuate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityLease {
    /// The instant at (and after) which the lease no longer grants authority.
    pub expires: Timestamp,
}

impl AuthorityLease {
    /// A lease valid up to (but not including) `expires`.
    pub const fn until(expires: Timestamp) -> Self {
        Self { expires }
    }

    /// Whether the lease still grants authority at `now`.
    ///
    /// A cross-domain comparison is conservatively treated as **not** granting:
    /// authority is never inferred across clock domains.
    pub fn grants_at(&self, now: Timestamp) -> bool {
        now.domain() == self.expires.domain() && now.as_nanos() < self.expires.as_nanos()
    }
}
