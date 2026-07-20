//! Clock domains.

/// A clock domain identifies the time base a [`crate::Timestamp`] belongs to.
///
/// The functional specification (§14.1) requires that a timestamp without a
/// clock-domain identifier MUST NOT cross a component boundary in production
/// profiles. In this crate that rule is structural: a [`crate::Timestamp`]
/// always carries its domain, and arithmetic across domains is a typed error.
///
/// The vocabulary here mirrors the `clockDomain` values accepted by
/// `neuradix-contracts`. The two crates intentionally do not depend on one
/// another; a test in `neuradix-testkit` asserts the two lists stay in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockDomain {
    /// Monotonic execution time (never steps backwards; no wall relationship).
    Monotonic,
    /// UTC / wall-clock time.
    Utc,
    /// Sensor hardware time.
    Sensor,
    /// Simulation time, advanced by a simulator.
    Simulation,
    /// Replay time, advanced from a recording.
    Replay,
}

impl ClockDomain {
    /// The canonical string spelling, matching the contract vocabulary.
    pub const fn as_str(self) -> &'static str {
        match self {
            ClockDomain::Monotonic => "monotonic",
            ClockDomain::Utc => "utc",
            ClockDomain::Sensor => "sensor",
            ClockDomain::Simulation => "simulation",
            ClockDomain::Replay => "replay",
        }
    }

    /// Parse a canonical clock-domain spelling.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "monotonic" => ClockDomain::Monotonic,
            "utc" => ClockDomain::Utc,
            "sensor" => ClockDomain::Sensor,
            "simulation" => ClockDomain::Simulation,
            "replay" => ClockDomain::Replay,
            _ => return None,
        })
    }

    /// Every supported spelling, in a stable order.
    pub const ALL: &'static [&'static str] =
        &["monotonic", "utc", "sensor", "simulation", "replay"];

    /// A stable one-byte code for compact serialization (e.g. recordings).
    ///
    /// These codes are part of the recording wire format and MUST remain stable.
    pub const fn code(self) -> u8 {
        match self {
            ClockDomain::Monotonic => 0,
            ClockDomain::Utc => 1,
            ClockDomain::Sensor => 2,
            ClockDomain::Simulation => 3,
            ClockDomain::Replay => 4,
        }
    }

    /// The inverse of [`ClockDomain::code`].
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => ClockDomain::Monotonic,
            1 => ClockDomain::Utc,
            2 => ClockDomain::Sensor,
            3 => ClockDomain::Simulation,
            4 => ClockDomain::Replay,
            _ => return None,
        })
    }
}

impl core::fmt::Display for ClockDomain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}
