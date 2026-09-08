//! Shared physical-units-per-second slew arithmetic, independent of authority.

use neuradix_time::Duration;

/// A finite, non-negative slew rate in output units per second.
///
/// Both scalar representations use the same binary64 budget calculation. MCU
/// binary32 output rounds toward the previous output, never away from it. Tiny
/// budgets may therefore hold; no fractional budget is accumulated across ticks.
/// Target timing/cost of binary64 arithmetic requires separate board evidence.
///
/// ```compile_fail
/// use neuradix_command_core::SlewRate;
/// let rate = SlewRate { units_per_second: f64::NAN };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlewRate {
    units_per_second: f64,
}

impl SlewRate {
    /// Validate a rate. Zero holds an established output reference.
    pub fn new(units_per_second: f64) -> Option<Self> {
        if units_per_second.is_finite() && units_per_second >= 0.0 {
            Some(Self { units_per_second })
        } else {
            None
        }
    }

    /// Configured rate in output units per second.
    pub fn units_per_second(self) -> f64 {
        self.units_per_second
    }

    /// Limit from the previous applied output over runtime elapsed time.
    /// No previous evaluation means initialization: only hard range limits apply.
    /// Negative elapsed time, non-finite inputs or overflow return None.
    pub fn apply_f64(self, value: f64, previous: Option<(f64, Duration)>) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        let Some((prev, dt)) = previous else {
            return Some(value);
        };
        if !prev.is_finite() || dt.as_nanos() < 0 {
            return None;
        }
        if dt.is_zero() || self.units_per_second == 0.0 {
            return Some(prev);
        }
        let max_delta = self.units_per_second * (dt.as_nanos() as f64 / 1_000_000_000.0);
        let lower = prev - max_delta;
        let upper = prev + max_delta;
        if !max_delta.is_finite() || !lower.is_finite() || !upper.is_finite() {
            return None;
        }
        let mut result = value.clamp(lower, upper);
        // Inward correction prevents rounded interval endpoints from exceeding
        // the computed budget. Conversion/product rounding remains documented.
        if (result - prev).abs() > max_delta {
            result = if result > prev {
                result.next_down()
            } else {
                result.next_up()
            };
        }
        Some(result)
    }

    /// The same limiter for binary32 outputs, with widened intermediates and
    /// inward output rounding. The final result stays between previous and target.
    pub fn apply_f32(self, value: f32, previous: Option<(f32, Duration)>) -> Option<f32> {
        let result = self.apply_f64(value as f64, previous.map(|(prev, dt)| (prev as f64, dt)))?;
        let mut rounded = result as f32;
        if let Some((prev, _)) = previous {
            if result > prev as f64 && rounded as f64 > result {
                rounded = rounded.next_down();
            } else if result < prev as f64 && (rounded as f64) < result {
                rounded = rounded.next_up();
            }
        }
        rounded.is_finite().then_some(rounded)
    }
}
