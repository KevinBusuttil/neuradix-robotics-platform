//! The component lifecycle state machine.
//!
//! Transitions are explicit, validated and recorded with reason, initiator,
//! timestamp and result (requirement COMP-002). The legal transition table is
//! specified in `docs/rfcs/RFC-0001-Component-and-Lifecycle-Model.md`:
//!
//! ```text
//! Declared   -> Configured | Failed
//! Configured -> Inactive   | Failed
//! Inactive   -> Active      | Stopping | Failed
//! Active     -> Degraded    | Stopping | Failed
//! Degraded   -> Active      | Stopping | Failed
//! Failed     -> Stopping    | Stopped
//! Stopping   -> Stopped
//! Stopped    -> (terminal)
//! ```

use neuradix_time::Timestamp;

use crate::error::LifecycleError;

/// The lifecycle state of a managed component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleState {
    /// The component is known to the runtime but not yet configured.
    Declared,
    /// Configuration has been validated and applied.
    Configured,
    /// Configured and resolved, but not yet processing.
    Inactive,
    /// Actively processing.
    Active,
    /// Processing, but with reduced capability or confidence.
    Degraded,
    /// A fault has occurred; the component is not processing.
    Failed,
    /// Shutting down in an orderly fashion.
    Stopping,
    /// Fully stopped (terminal).
    Stopped,
}

impl LifecycleState {
    /// The canonical lowercase spelling of the state.
    pub const fn as_str(self) -> &'static str {
        match self {
            LifecycleState::Declared => "declared",
            LifecycleState::Configured => "configured",
            LifecycleState::Inactive => "inactive",
            LifecycleState::Active => "active",
            LifecycleState::Degraded => "degraded",
            LifecycleState::Failed => "failed",
            LifecycleState::Stopping => "stopping",
            LifecycleState::Stopped => "stopped",
        }
    }

    /// Whether this is a terminal state (no further transitions permitted).
    pub const fn is_terminal(self) -> bool {
        matches!(self, LifecycleState::Stopped)
    }

    /// Whether a direct transition to `next` is permitted from `self`.
    pub fn can_transition_to(self, next: LifecycleState) -> bool {
        use LifecycleState::*;
        matches!(
            (self, next),
            (Declared, Configured)
                | (Declared, Failed)
                | (Configured, Inactive)
                | (Configured, Failed)
                | (Inactive, Active)
                | (Inactive, Stopping)
                | (Inactive, Failed)
                | (Active, Degraded)
                | (Active, Stopping)
                | (Active, Failed)
                | (Degraded, Active)
                | (Degraded, Stopping)
                | (Degraded, Failed)
                | (Failed, Stopping)
                | (Failed, Stopped)
                | (Stopping, Stopped)
        )
    }
}

impl std::fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An immutable record of one lifecycle transition (COMP-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRecord {
    /// The state before the transition.
    pub from: LifecycleState,
    /// The state after the transition.
    pub to: LifecycleState,
    /// Why the transition happened.
    pub reason: String,
    /// Who or what initiated it.
    pub initiator: String,
    /// When it happened (domain-tagged).
    pub at: Timestamp,
}

/// A component's lifecycle: its current state and its audit history.
#[derive(Debug, Clone)]
pub struct Lifecycle {
    current: LifecycleState,
    history: Vec<TransitionRecord>,
}

impl Lifecycle {
    /// Create a new lifecycle in the initial [`LifecycleState::Declared`] state.
    pub fn new() -> Self {
        Self {
            current: LifecycleState::Declared,
            history: Vec::new(),
        }
    }

    /// The current lifecycle state (COMP-001).
    pub fn current(&self) -> LifecycleState {
        self.current
    }

    /// The full transition history, oldest first.
    pub fn history(&self) -> &[TransitionRecord] {
        &self.history
    }

    /// Attempt a transition to `to`, recording reason, initiator and time.
    ///
    /// Returns the recorded transition on success, or
    /// [`LifecycleError::IllegalTransition`] if the transition is not permitted.
    pub fn transition(
        &mut self,
        to: LifecycleState,
        reason: impl Into<String>,
        initiator: impl Into<String>,
        at: Timestamp,
    ) -> Result<&TransitionRecord, LifecycleError> {
        if !self.current.can_transition_to(to) {
            return Err(LifecycleError::IllegalTransition {
                from: self.current,
                to,
            });
        }
        let record = TransitionRecord {
            from: self.current,
            to,
            reason: reason.into(),
            initiator: initiator.into(),
            at,
        };
        self.current = to;
        self.history.push(record);
        // The record was just pushed, so `last` is always present.
        Ok(self.history.last().expect("record was just pushed"))
    }
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuradix_time::Clock;
    use neuradix_time::{ClockDomain, ManualClock, Timestamp};

    fn clock() -> ManualClock {
        ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0))
    }

    #[test]
    fn legal_bring_up_and_shut_down_sequence() {
        let clock = clock();
        let mut lc = Lifecycle::new();
        for state in [
            LifecycleState::Configured,
            LifecycleState::Inactive,
            LifecycleState::Active,
            LifecycleState::Stopping,
            LifecycleState::Stopped,
        ] {
            lc.transition(state, "ok", "test", clock.now())
                .expect("legal");
        }
        assert_eq!(lc.current(), LifecycleState::Stopped);
        assert_eq!(lc.history().len(), 5);
        assert!(lc.current().is_terminal());
    }

    #[test]
    fn illegal_transitions_are_rejected_and_do_not_mutate_state() {
        let clock = clock();
        let mut lc = Lifecycle::new();
        // Declared -> Active is not permitted (must be configured first).
        let err = lc
            .transition(LifecycleState::Active, "skip", "test", clock.now())
            .expect_err("illegal");
        assert_eq!(
            err,
            LifecycleError::IllegalTransition {
                from: LifecycleState::Declared,
                to: LifecycleState::Active,
            }
        );
        assert_eq!(
            lc.current(),
            LifecycleState::Declared,
            "state must be unchanged"
        );
        assert!(lc.history().is_empty());
    }

    #[test]
    fn terminal_state_permits_no_transitions() {
        for target in [
            LifecycleState::Declared,
            LifecycleState::Configured,
            LifecycleState::Inactive,
            LifecycleState::Active,
            LifecycleState::Degraded,
            LifecycleState::Failed,
            LifecycleState::Stopping,
            LifecycleState::Stopped,
        ] {
            assert!(
                !LifecycleState::Stopped.can_transition_to(target),
                "Stopped must be terminal, but allowed -> {target}"
            );
        }
    }

    #[test]
    fn degraded_can_recover_to_active() {
        assert!(LifecycleState::Active.can_transition_to(LifecycleState::Degraded));
        assert!(LifecycleState::Degraded.can_transition_to(LifecycleState::Active));
    }

    #[test]
    fn transition_records_capture_audit_fields() {
        let clock = clock();
        let mut lc = Lifecycle::new();
        let record = lc
            .transition(
                LifecycleState::Configured,
                "cfg ok",
                "operator",
                clock.now(),
            )
            .unwrap();
        assert_eq!(record.from, LifecycleState::Declared);
        assert_eq!(record.to, LifecycleState::Configured);
        assert_eq!(record.reason, "cfg ok");
        assert_eq!(record.initiator, "operator");
        assert_eq!(record.at.domain(), ClockDomain::Simulation);
    }
}
