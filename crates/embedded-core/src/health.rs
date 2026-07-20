//! Component health — the same vocabulary as the host runtime (§25.5), so a node
//! reports health identically whether it runs on Linux or an MCU.

/// The structured health state a node reports.
///
/// Mirrors `neuradix_runtime::HealthState` exactly; the two are kept in sync so
/// host and firmware speak the same health language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HealthState {
    /// Operating normally.
    Healthy,
    /// Operating with reduced capability or confidence (e.g. in a safe state).
    Degraded,
    /// Not operating correctly.
    Unhealthy,
    /// Present but not reachable / not reporting.
    Unavailable,
    /// Health cannot yet be determined.
    Unknown,
}

impl HealthState {
    /// The canonical lowercase spelling of the state.
    pub const fn as_str(self) -> &'static str {
        match self {
            HealthState::Healthy => "healthy",
            HealthState::Degraded => "degraded",
            HealthState::Unhealthy => "unhealthy",
            HealthState::Unavailable => "unavailable",
            HealthState::Unknown => "unknown",
        }
    }
}

impl core::fmt::Display for HealthState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}
