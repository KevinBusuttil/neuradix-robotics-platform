//! A signed, nanosecond-resolution duration.

use crate::error::TimeError;

/// A signed duration measured in whole nanoseconds.
///
/// A dedicated type (rather than [`std::time::Duration`]) is used so that
/// durations can be signed (time offsets can be negative), parse from and
/// display in the platform's canonical literal form, and never panic on
/// overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Duration {
    nanos: i128,
}

impl Duration {
    /// The zero duration.
    pub const ZERO: Duration = Duration { nanos: 0 };

    /// Construct from a signed number of nanoseconds.
    pub const fn from_nanos(nanos: i128) -> Self {
        Self { nanos }
    }

    /// Construct from a signed number of microseconds.
    pub const fn from_micros(micros: i128) -> Self {
        Self {
            nanos: micros * 1_000,
        }
    }

    /// Construct from a signed number of milliseconds.
    pub const fn from_millis(millis: i128) -> Self {
        Self {
            nanos: millis * 1_000_000,
        }
    }

    /// Construct from a signed number of seconds.
    pub const fn from_secs(secs: i64) -> Self {
        Self {
            nanos: secs as i128 * 1_000_000_000,
        }
    }

    /// The duration in whole nanoseconds.
    pub const fn as_nanos(self) -> i128 {
        self.nanos
    }

    /// The duration as fractional seconds (for display and diagnostics only).
    pub fn as_secs_f64(self) -> f64 {
        self.nanos as f64 / 1_000_000_000.0
    }

    /// Whether this duration is exactly zero.
    pub const fn is_zero(self) -> bool {
        self.nanos == 0
    }

    /// Checked addition; returns [`TimeError::Overflow`] on overflow.
    pub fn checked_add(self, other: Duration) -> Result<Duration, TimeError> {
        self.nanos
            .checked_add(other.nanos)
            .map(Duration::from_nanos)
            .ok_or(TimeError::Overflow)
    }

    /// Parse a duration literal such as `100ms`, `-0.1s`, `500us`, `2m`, `1h`.
    ///
    /// Accepted unit suffixes: `ns`, `us`, `ms`, `s`, `m`, `h`. A leading `-`
    /// is permitted. A bare number without a unit is rejected.
    pub fn parse(input: &str) -> Option<Self> {
        let s = input.trim();
        let (negative, magnitude) = match s.strip_prefix('-') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, s),
        };
        const UNITS: &[(&str, i128)] = &[
            ("ns", 1),
            ("us", 1_000),
            ("ms", 1_000_000),
            ("s", 1_000_000_000),
            ("m", 60_000_000_000),
            ("h", 3_600_000_000_000),
        ];
        let (number, scale) = UNITS.iter().find_map(|(suffix, scale)| {
            magnitude.strip_suffix(suffix).map(|n| (n.trim(), *scale))
        })?;
        let nanos = parse_decimal_scaled(number, scale)?;
        Some(Duration {
            nanos: if negative { -nanos } else { nanos },
        })
    }
}

/// Parse a non-negative decimal `number` scaled by `scale` nanoseconds, exactly.
fn parse_decimal_scaled(number: &str, scale: i128) -> Option<i128> {
    if number.is_empty() {
        return None;
    }
    let (int_part, frac_part) = match number.split_once('.') {
        Some((i, f)) => (i, f),
        None => (number, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.is_empty() && !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac_part.is_empty() && !frac_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let int_value: i128 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().ok()?
    };
    let mut total = int_value.checked_mul(scale)?;
    if !frac_part.is_empty() {
        let frac_value: i128 = frac_part.parse().ok()?;
        let divisor: i128 = 10i128.checked_pow(frac_part.len() as u32)?;
        let scaled = frac_value.checked_mul(scale)?;
        if scaled % divisor != 0 {
            return None;
        }
        total = total.checked_add(scaled / divisor)?;
    }
    Some(total)
}

impl std::fmt::Display for Duration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Emit the largest exact unit for readability.
        let n = self.nanos;
        if n == 0 {
            return f.write_str("0ns");
        }
        const UNITS: &[(&str, i128)] = &[
            ("h", 3_600_000_000_000),
            ("m", 60_000_000_000),
            ("s", 1_000_000_000),
            ("ms", 1_000_000),
            ("us", 1_000),
            ("ns", 1),
        ];
        for (suffix, scale) in UNITS {
            if n % scale == 0 {
                return write!(f, "{}{}", n / scale, suffix);
            }
        }
        write!(f, "{n}ns")
    }
}
