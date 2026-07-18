//! Fault Detection, Isolation and Recovery (FDIR) — a fault-mode state machine
//! (§16.5, §16.8).
//!
//! The monitor consumes a component's [`HealthState`] over time and drives an
//! explicit system fault mode through the FDIR phases: **detection** (a
//! non-healthy report), **confirmation** (a fault must persist for
//! `confirm_threshold` consecutive reports before it escalates — this debounces
//! transient glitches), **accommodation** (escalation to a `Degraded` or `Safe`
//! mode), **recovery** (a healthy streak of `recovery_threshold` reports may
//! de-escalate `Degraded -> Nominal`), and **return-to-service** (an explicit
//! [`FdirMonitor::reset`] from `Safe`).
//!
//! A restart budget (`max_recoveries`) prevents restart storms: once the budget
//! is spent, the next attempted recovery latches the system into `Safe` instead
//! of flapping. `Safe` never auto-recovers.
//!
//! Evaluation is a pure function of `(policy, monitor state, input, time)`, so
//! FDIR decisions are deterministic and replay identically under the executor
//! (RFC-0016); the monitor is a [`Processor`].

use neuradix_runtime::{ComponentError, HealthState, Processor, TickContext};
use neuradix_time::Timestamp;

/// The outward system fault mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultMode {
    /// Operating normally.
    Nominal,
    /// Operating with reduced capability after a confirmed soft fault.
    Degraded,
    /// A confirmed hard fault (or spent restart budget) forced a safe state.
    Safe,
}

impl FaultMode {
    /// The canonical lowercase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            FaultMode::Nominal => "nominal",
            FaultMode::Degraded => "degraded",
            FaultMode::Safe => "safe",
        }
    }
}

impl std::fmt::Display for FaultMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The severity a health report represents to FDIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Ok,
    Soft,
    Hard,
}

fn severity_of(health: HealthState) -> Severity {
    match health {
        HealthState::Healthy => Severity::Ok,
        HealthState::Degraded => Severity::Soft,
        // Absence of positive health is treated conservatively as a hard fault.
        HealthState::Unhealthy | HealthState::Unavailable | HealthState::Unknown => Severity::Hard,
    }
}

/// FDIR policy thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdirPolicy {
    /// Consecutive fault reports required to confirm (debounce) before escalating.
    pub confirm_threshold: u32,
    /// Consecutive healthy reports required to recover `Degraded -> Nominal`.
    pub recovery_threshold: u32,
    /// Maximum `Degraded -> Nominal` recoveries before the next attempt latches
    /// into `Safe` (restart-storm prevention).
    pub max_recoveries: u32,
}

impl FdirPolicy {
    /// A conservative default policy.
    pub fn new(confirm_threshold: u32, recovery_threshold: u32, max_recoveries: u32) -> Self {
        Self {
            confirm_threshold,
            recovery_threshold,
            max_recoveries,
        }
    }
}

impl Default for FdirPolicy {
    fn default() -> Self {
        Self {
            confirm_threshold: 3,
            recovery_threshold: 3,
            max_recoveries: 2,
        }
    }
}

/// An immutable record of one fault-mode transition.
#[derive(Debug, Clone, PartialEq)]
pub struct FdirTransition {
    /// The mode before the transition.
    pub from: FaultMode,
    /// The mode after the transition.
    pub to: FaultMode,
    /// When it happened.
    pub at: Timestamp,
    /// Why it happened.
    pub reason: String,
}

/// The FDIR fault-mode monitor.
#[derive(Debug, Clone)]
pub struct FdirMonitor {
    policy: FdirPolicy,
    mode: FaultMode,
    consecutive_faults: u32,
    consecutive_healthy: u32,
    recoveries_used: u32,
}

impl FdirMonitor {
    /// Create a monitor in [`FaultMode::Nominal`] with the given policy.
    pub fn new(policy: FdirPolicy) -> Self {
        Self {
            policy,
            mode: FaultMode::Nominal,
            consecutive_faults: 0,
            consecutive_healthy: 0,
            recoveries_used: 0,
        }
    }

    /// The current fault mode.
    pub fn mode(&self) -> FaultMode {
        self.mode
    }

    /// Observe a health report at `at`, returning a transition if the mode changed.
    pub fn observe(&mut self, health: HealthState, at: Timestamp) -> Option<FdirTransition> {
        match severity_of(health) {
            Severity::Ok => self.on_ok(at),
            Severity::Soft => self.on_fault(at, false),
            Severity::Hard => self.on_fault(at, true),
        }
    }

    /// Explicitly return to service from [`FaultMode::Safe`] (an authorised
    /// operator action). Returns the transition, or `None` if not in `Safe`.
    pub fn reset(&mut self, at: Timestamp) -> Option<FdirTransition> {
        if self.mode != FaultMode::Safe {
            return None;
        }
        let from = self.mode;
        self.mode = FaultMode::Nominal;
        self.consecutive_faults = 0;
        self.consecutive_healthy = 0;
        self.recoveries_used = 0;
        Some(FdirTransition {
            from,
            to: FaultMode::Nominal,
            at,
            reason: "operator reset (return to service)".to_owned(),
        })
    }

    fn on_ok(&mut self, at: Timestamp) -> Option<FdirTransition> {
        self.consecutive_faults = 0;
        self.consecutive_healthy = self.consecutive_healthy.saturating_add(1);

        if self.mode == FaultMode::Degraded
            && self.consecutive_healthy >= self.policy.recovery_threshold
        {
            self.consecutive_healthy = 0;
            if self.recoveries_used < self.policy.max_recoveries {
                self.recoveries_used += 1;
                return self.transition(FaultMode::Nominal, at, "recovered after healthy streak");
            }
            // Restart budget exhausted: latch safe instead of flapping.
            return self.transition(
                FaultMode::Safe,
                at,
                "recovery budget exhausted; latching safe",
            );
        }
        None
    }

    fn on_fault(&mut self, at: Timestamp, hard: bool) -> Option<FdirTransition> {
        self.consecutive_healthy = 0;
        self.consecutive_faults = self.consecutive_faults.saturating_add(1);

        if self.consecutive_faults < self.policy.confirm_threshold {
            return None; // not yet confirmed (debounce)
        }

        if hard {
            if self.mode != FaultMode::Safe {
                return self.transition(FaultMode::Safe, at, "hard fault confirmed; entering safe");
            }
        } else if self.mode == FaultMode::Nominal {
            return self.transition(
                FaultMode::Degraded,
                at,
                "soft fault confirmed; entering degraded",
            );
        }
        None
    }

    fn transition(&mut self, to: FaultMode, at: Timestamp, reason: &str) -> Option<FdirTransition> {
        let from = self.mode;
        if from == to {
            return None;
        }
        self.mode = to;
        Some(FdirTransition {
            from,
            to,
            at,
            reason: reason.to_owned(),
        })
    }
}

impl Processor for FdirMonitor {
    type Input = HealthState;
    type Output = FdirTransition;

    fn process(
        &mut self,
        ctx: &TickContext,
        input: HealthState,
    ) -> Result<Vec<FdirTransition>, ComponentError> {
        Ok(self.observe(input, ctx.now).into_iter().collect())
    }
}
