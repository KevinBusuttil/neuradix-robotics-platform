//! Constant-size, supervisor-owned monotonic responsiveness state.

use std::time::Instant;

use neuradix_runtime::HealthState;

use crate::{HeartbeatPolicy, WorkerError};

/// Latched session failure category. It survives health queries and shutdown;
/// a replacement owns fresh state. Detailed errors/cleanup are returned separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFailure {
    /// No matching pong completed before the anchored heartbeat deadline.
    HeartbeatTimeout,
    /// Supervision resumed at or after responsiveness expiry.
    HeartbeatExpired,
    /// Monotonic observation regressed or checked deadline arithmetic overflowed.
    Clock,
    /// An ordinary request exhausted its total I/O budget or health window.
    RequestTimeout,
    /// Protocol, size, or queue validation failed.
    Protocol,
    /// Child exit or live-child stdout closure.
    Exited,
    /// Pipe, process ownership, or other operating-system failure.
    Io,
    /// No more sequence numbers can be issued in this session.
    SequenceExhausted,
    /// Explicit shutdown (or Drop) ended an otherwise usable session.
    Shutdown,
    /// A replacement could not launch or complete its handshake.
    Launch,
}
impl WorkerFailure {
    pub(crate) fn from_error(error: &WorkerError) -> Self {
        match error {
            WorkerError::Cleanup { cause, .. } => Self::from_error(cause),
            WorkerError::HeartbeatTimeout => Self::HeartbeatTimeout,
            WorkerError::HeartbeatExpired => Self::HeartbeatExpired,
            WorkerError::SupervisionClock => Self::Clock,
            WorkerError::Timeout => Self::RequestTimeout,
            WorkerError::SequenceExhausted => Self::SequenceExhausted,
            WorkerError::WorkerExited { .. } | WorkerError::StdoutClosed => Self::Exited,
            WorkerError::Protocol(_)
            | WorkerError::IncomingTooLarge { .. }
            | WorkerError::OutgoingTooLarge { .. }
            | WorkerError::QueueBytesExceeded { .. }
            | WorkerError::QueueMessagesExceeded { .. } => Self::Protocol,
            _ => Self::Io,
        }
    }
}

pub(crate) struct Heartbeat {
    policy: HeartbeatPolicy,
    observed: Instant,
    due: Instant,
    expires: Instant,
    confirmed: bool,
    failure: Option<WorkerFailure>,
}
impl Heartbeat {
    pub fn new(policy: HeartbeatPolicy, now: Instant) -> Result<Self, WorkerError> {
        let due = now
            .checked_add(policy.interval())
            .ok_or(WorkerError::SupervisionClock)?;
        let expires = due
            .checked_add(policy.response())
            .ok_or(WorkerError::SupervisionClock)?;
        Ok(Self {
            policy,
            observed: now,
            due,
            expires,
            confirmed: false,
            failure: None,
        })
    }
    pub fn observe(&mut self, now: Instant) -> Result<HealthState, WorkerError> {
        if self.failure.is_some() {
            return Err(WorkerError::Unavailable);
        }
        if now < self.observed {
            self.fail(WorkerFailure::Clock);
            return Err(WorkerError::SupervisionClock);
        }
        self.observed = now;
        if now >= self.expires {
            self.fail(WorkerFailure::HeartbeatExpired);
            return Err(WorkerError::HeartbeatExpired);
        }
        Ok(if !self.confirmed {
            HealthState::Unknown
        } else if now >= self.due {
            HealthState::Degraded
        } else {
            HealthState::Healthy
        })
    }
    pub fn confirm(&mut self, now: Instant) -> Result<(), WorkerError> {
        self.observe(now)?;
        let next = Self::new(self.policy, now).inspect_err(|_| self.fail(WorkerFailure::Clock))?;
        *self = next;
        self.confirmed = true;
        Ok(())
    }
    pub fn due(&self) -> Instant {
        self.due
    }
    pub fn expires(&self) -> Instant {
        self.expires
    }
    pub fn fail(&mut self, reason: WorkerFailure) {
        self.failure.get_or_insert(reason);
    }
    pub fn failure(&self) -> Option<WorkerFailure> {
        self.failure
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn exact_due_expiry_and_confirmation_boundaries() {
        let now = Instant::now();
        let interval = Duration::from_millis(20);
        let response = Duration::from_millis(10);
        let policy = HeartbeatPolicy::new(interval, response).unwrap();
        let mut state = Heartbeat::new(policy, now).unwrap();
        assert_eq!(state.observe(now).unwrap(), HealthState::Unknown);
        state.confirm(now).unwrap();
        assert_eq!(
            state
                .observe(now + interval - Duration::from_nanos(1))
                .unwrap(),
            HealthState::Healthy
        );
        assert_eq!(
            state.observe(now + interval).unwrap(),
            HealthState::Degraded
        );
        let expiry = now + interval + response;
        assert_eq!(
            state.observe(expiry - Duration::from_nanos(1)).unwrap(),
            HealthState::Degraded
        );
        assert!(matches!(
            state.confirm(expiry),
            Err(WorkerError::HeartbeatExpired)
        ));
        assert_eq!(state.failure(), Some(WorkerFailure::HeartbeatExpired));
        assert!(matches!(
            state.confirm(expiry),
            Err(WorkerError::Unavailable)
        ));
        state.fail(WorkerFailure::Shutdown);
        assert_eq!(state.failure(), Some(WorkerFailure::HeartbeatExpired));
        let mut replacement = Heartbeat::new(policy, expiry).unwrap();
        assert_eq!(replacement.observe(expiry).unwrap(), HealthState::Unknown);
        replacement.confirm(expiry).unwrap();
        assert_eq!(replacement.due(), expiry + interval);
    }
    #[test]
    fn regression_latches_and_configuration_is_bounded() {
        for value in [Duration::ZERO, Duration::from_nanos(999_999), Duration::MAX] {
            assert!(HeartbeatPolicy::new(value, Duration::from_secs(1)).is_err());
            assert!(HeartbeatPolicy::new(Duration::from_secs(1), value).is_err());
        }
        assert!(HeartbeatPolicy::new(Duration::from_millis(1), Duration::from_secs(60)).is_ok());
        let now = Instant::now();
        let mut state = Heartbeat::new(HeartbeatPolicy::default(), now).unwrap();
        state.observe(now + Duration::from_nanos(1)).unwrap();
        assert!(matches!(
            state.confirm(now),
            Err(WorkerError::SupervisionClock)
        ));
        assert_eq!(state.failure(), Some(WorkerFailure::Clock));
        assert!(matches!(
            state.observe(now + Duration::from_secs(1)),
            Err(WorkerError::Unavailable)
        ));
    }
    #[test]
    fn representable_instant_boundary_fails_closed() {
        let now = Instant::now();
        let mut seconds = 0u64;
        // Bounded search of the platform Instant range, without assuming its epoch.
        for bit in (0..64).rev() {
            let candidate = seconds | (1u64 << bit);
            if now.checked_add(Duration::from_secs(candidate)).is_some() {
                seconds = candidate;
            }
        }
        let edge = now.checked_add(Duration::from_secs(seconds)).unwrap();
        let policy = HeartbeatPolicy::new(Duration::from_secs(1), Duration::from_secs(1)).unwrap();
        assert!(matches!(
            Heartbeat::new(policy, edge),
            Err(WorkerError::SupervisionClock)
        ));
        let mut state = Heartbeat::new(policy, edge - Duration::from_secs(2)).unwrap();
        assert!(matches!(
            state.confirm(edge - Duration::from_nanos(1)),
            Err(WorkerError::SupervisionClock)
        ));
        assert_eq!(state.failure(), Some(WorkerFailure::Clock));
    }
}
