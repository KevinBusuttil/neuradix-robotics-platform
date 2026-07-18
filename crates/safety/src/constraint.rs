//! Safety constraints: range and slew-rate limits (§16.4).
//!
//! Each constraint carries a stable rule identifier so that a decision can
//! report exactly which rule modified a command (NRX-SAF-003).

use neuradix_time::Duration;

use crate::error::SafetyError;

/// A safety constraint applied to a scalar command value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Constraint {
    /// Clamp the value to an inclusive `[min, max]` range.
    Range {
        /// Stable rule identifier.
        id: &'static str,
        /// Lower bound.
        min: f64,
        /// Upper bound.
        max: f64,
    },
    /// Limit the change from the previous applied value to `rate_per_sec * dt`.
    SlewRate {
        /// Stable rule identifier.
        id: &'static str,
        /// Maximum rate of change per second (absolute).
        rate_per_sec: f64,
    },
}

impl Constraint {
    /// A range constraint, validated so `min <= max`.
    pub fn range(id: &'static str, min: f64, max: f64) -> Result<Self, SafetyError> {
        if min > max {
            return Err(SafetyError::InvalidRange {
                id,
                min: min.to_string(),
                max: max.to_string(),
            });
        }
        Ok(Constraint::Range { id, min, max })
    }

    /// A slew-rate constraint, validated so `rate_per_sec >= 0`.
    pub fn slew_rate(id: &'static str, rate_per_sec: f64) -> Result<Self, SafetyError> {
        if rate_per_sec < 0.0 || rate_per_sec.is_nan() {
            return Err(SafetyError::InvalidSlew {
                id,
                rate: rate_per_sec.to_string(),
            });
        }
        Ok(Constraint::SlewRate { id, rate_per_sec })
    }

    /// The stable rule identifier.
    pub fn id(&self) -> &'static str {
        match self {
            Constraint::Range { id, .. } | Constraint::SlewRate { id, .. } => id,
        }
    }

    /// Apply the constraint to `value`. `previous` is the previously-applied
    /// value and the elapsed time since it, or `None` if this is the first
    /// command (in which case a rate limit has no reference and is a no-op).
    /// Returns the constrained value.
    pub fn apply(&self, value: f64, previous: Option<(f64, Duration)>) -> f64 {
        match self {
            Constraint::Range { min, max, .. } => value.clamp(*min, *max),
            Constraint::SlewRate { rate_per_sec, .. } => match previous {
                Some((prev, dt)) => {
                    let max_delta = rate_per_sec * dt.as_secs_f64().abs();
                    value.clamp(prev - max_delta, prev + max_delta)
                }
                None => value,
            },
        }
    }
}
