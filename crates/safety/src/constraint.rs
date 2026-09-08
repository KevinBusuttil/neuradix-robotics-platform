//! Validated range and slew-rate limits (§16.4).

use neuradix_time::Duration;
use neuradix_command_core::SlewRate;

use crate::error::SafetyError;

/// A scalar constraint with private, validated configuration.
///
/// Use [`Self::range`] or [`Self::slew_rate`]; unchecked variants are unavailable.
/// ```compile_fail
/// use neuradix_safety::Constraint;
/// let constraint = Constraint::Range { id: "bad", min: f64::NAN, max: 1.0 };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraint {
    id: &'static str,
    kind: ConstraintKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ConstraintKind {
    Range { min: f64, max: f64 },
    SlewRate(SlewRate),
}

impl Constraint {
    /// Construct a finite inclusive range with `min <= max`.
    pub fn range(id: &'static str, min: f64, max: f64) -> Result<Self, SafetyError> {
        if !min.is_finite() || !max.is_finite() || min > max {
            return Err(SafetyError::InvalidRange {
                id,
                min: min.to_string(),
                max: max.to_string(),
            });
        }
        Ok(Self {
            id,
            kind: ConstraintKind::Range { min, max },
        })
    }

    /// Construct a finite, non-negative rate in units per second. Zero holds
    /// the previous output; the first command has no slew reference.
    pub fn slew_rate(id: &'static str, rate_per_sec: f64) -> Result<Self, SafetyError> {
        if !rate_per_sec.is_finite() || rate_per_sec < 0.0 {
            return Err(SafetyError::InvalidSlew {
                id,
                rate: rate_per_sec.to_string(),
            });
        }
        Ok(Self {
            id,
            kind: ConstraintKind::SlewRate(SlewRate::new(rate_per_sec).expect("validated rate")),
        })
    }

    /// The stable rule identifier.
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// Whether a value satisfies this constraint's hard output bounds. A slew
    /// constraint has no hard range; it still requires a finite output.
    pub fn permits_output(&self, value: f64) -> bool {
        value.is_finite()
            && match self.kind {
                ConstraintKind::Range { min, max } => value >= min && value <= max,
                ConstraintKind::SlewRate(_) => true,
            }
    }

    /// Apply a constraint using a finite previous output and non-negative
    /// runtime elapsed time. `None` reports invalid input or arithmetic overflow.
    /// The gate additionally validates the result against *all* hard ranges.
    pub fn apply(&self, value: f64, previous: Option<(f64, Duration)>) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        match self.kind {
            ConstraintKind::Range { min, max } => Some(value.clamp(min, max)),
            ConstraintKind::SlewRate(rate) => rate.apply_f64(value, previous),
        }
    }
}
