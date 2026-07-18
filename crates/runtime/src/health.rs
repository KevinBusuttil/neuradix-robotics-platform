//! Component health model (§25.5).

/// The structured health state a component reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HealthState {
    /// Operating normally.
    Healthy,
    /// Operating with reduced capability or confidence.
    Degraded,
    /// Not operating correctly.
    Unhealthy,
    /// Present but not reachable / not reporting.
    Unavailable,
    /// Health cannot be determined.
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

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A health state with an optional human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// The reported state.
    pub state: HealthState,
    /// An optional reason or evidence string.
    pub reason: Option<String>,
}

impl HealthReport {
    /// A report carrying just a state and no reason.
    pub fn new(state: HealthState) -> Self {
        Self {
            state,
            reason: None,
        }
    }

    /// A report carrying a state and a reason.
    pub fn with_reason(state: HealthState, reason: impl Into<String>) -> Self {
        Self {
            state,
            reason: Some(reason.into()),
        }
    }
}
