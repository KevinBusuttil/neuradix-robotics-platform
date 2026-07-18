//! Domain-tagged timestamps.

use std::cmp::Ordering;

use crate::domain::ClockDomain;
use crate::duration::Duration;
use crate::error::TimeError;

/// An instant in a specific [`ClockDomain`], measured in nanoseconds since that
/// domain's epoch.
///
/// The epoch is domain-defined (for example, monotonic time counts from an
/// unspecified start; UTC counts from the Unix epoch). Because a timestamp
/// always carries its domain, arithmetic and comparison across domains is a
/// typed error rather than a silent bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timestamp {
    domain: ClockDomain,
    nanos: i128,
}

impl Timestamp {
    /// Construct a timestamp in `domain` at `nanos` since the domain epoch.
    pub const fn new(domain: ClockDomain, nanos: i128) -> Self {
        Self { domain, nanos }
    }

    /// The domain this timestamp belongs to.
    pub const fn domain(self) -> ClockDomain {
        self.domain
    }

    /// The value in nanoseconds since the domain epoch.
    pub const fn as_nanos(self) -> i128 {
        self.nanos
    }

    /// Add a duration, staying in the same domain.
    pub fn checked_add(self, delta: Duration) -> Result<Timestamp, TimeError> {
        self.nanos
            .checked_add(delta.as_nanos())
            .map(|nanos| Timestamp {
                domain: self.domain,
                nanos,
            })
            .ok_or(TimeError::Overflow)
    }

    /// The signed duration from `earlier` to `self`. Both must share a domain.
    pub fn duration_since(self, earlier: Timestamp) -> Result<Duration, TimeError> {
        self.require_same_domain(earlier)?;
        self.nanos
            .checked_sub(earlier.nanos)
            .map(Duration::from_nanos)
            .ok_or(TimeError::Overflow)
    }

    /// Compare two timestamps in the same domain.
    pub fn compare(self, other: Timestamp) -> Result<Ordering, TimeError> {
        self.require_same_domain(other)?;
        Ok(self.nanos.cmp(&other.nanos))
    }

    fn require_same_domain(self, other: Timestamp) -> Result<(), TimeError> {
        if self.domain == other.domain {
            Ok(())
        } else {
            Err(TimeError::DomainMismatch {
                left: self.domain,
                right: other.domain,
            })
        }
    }
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}ns", self.domain, self.nanos)
    }
}
