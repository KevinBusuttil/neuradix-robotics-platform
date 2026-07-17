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
}

impl std::fmt::Display for ClockDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
