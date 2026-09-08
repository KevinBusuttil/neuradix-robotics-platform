//! The local command gate — the embedded safety heart.
//!
//! Every actuator command passes through runtime clock, binding/lease, source
//! validity/sequence and numeric/output checks. Only complete acceptance feeds
//! the command watchdog; idle ticks check all held-command expiry conditions.
//! Rejection applies a **local safe output** without
//! any dependency on the host — the §16.1 / NRX-EMB-004 rule that a node can
//! always reach a safe state on its own.

use neuradix_time::Timestamp;

use crate::lease::AuthorityLease;
use neuradix_command_core::{CommandMeta, ConfigError, EvaluationClock, Generation};

/// The actuator envelope a command must satisfy.
///
/// ```compile_fail
/// use neuradix_embedded_core::Limits;
/// let mut limits = Limits::new(-1.0, 1.0, 0.5).unwrap();
/// limits.min = f32::NAN;
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    min: f32,
    max: f32,
    max_step: f32,
}

impl Limits {
    /// Minimum applied value.
    pub fn min(&self) -> f32 {
        self.min
    }

    /// Maximum applied value.
    pub fn max(&self) -> f32 {
        self.max
    }

    /// Maximum change per evaluation (not units per second).
    pub fn max_step(&self) -> f32 {
        self.max_step
    }

    fn permits(&self, value: f32) -> bool {
        value.is_finite() && value >= self.min && value <= self.max
    }

    /// Validated construction: `min <= max`, `max_step >= 0`, all finite.
    pub fn new(min: f32, max: f32, max_step: f32) -> Option<Self> {
        if min.is_finite()
            && max.is_finite()
            && max_step.is_finite()
            && min <= max
            && max_step >= 0.0
        {
            Some(Self { min, max, max_step })
        } else {
            None
        }
    }
}

/// Invalid embedded gate configuration. Construction never coerces safe output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateConfigError {
    /// Safe output must be finite and inside the inclusive actuator envelope.
    InvalidSafeOutput,
}

impl core::fmt::Display for GateConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("safe output must be finite and inside the actuator envelope")
    }
}

impl core::error::Error for GateConfigError {}

/// Shared host/embedded rejection vocabulary.
pub use neuradix_command_core::CommandRejection as SafeReason;

/// A source command bound to a provisioned holder/capability and session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Command {
    /// Source holder identifier; compared with the trusted lease binding.
    pub holder: u64,
    /// Actuator capability identifier.
    pub capability: u64,
    /// Untrusted scalar value.
    pub value: f32,
    /// Source freshness, deadline, sequence, timeline and generation metadata.
    pub meta: CommandMeta,
}

/// The disposition of a gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Applied unchanged, or a still-valid output held during an idle tick.
    Accepted,
    /// Applied after range or per-evaluation slew limiting.
    Modified,
    /// Local safe output with an auditable reason.
    SafeState(SafeReason),
}

/// Result of one evaluation, including idle expiry and source provenance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateDecision {
    /// Incoming source command; None for a runtime idle tick.
    pub request: Option<Command>,
    /// Runtime evaluation time, never a source timestamp substitution.
    pub at: Timestamp,
    /// Finite applied output inside the configured hard bounds.
    pub applied: f32,
    /// Disposition and rejection reason.
    pub outcome: Outcome,
    /// Whether range limiting acted.
    pub range_clamped: bool,
    /// Whether per-evaluation slew limiting acted.
    pub slew_limited: bool,
}

/// Allocation-free gate: binding/lease, command validity, numeric limits, fallback.
/// Runtime must call evaluate periodically, including with None when no input arrives.
#[derive(Debug, Clone, Copy)]
pub struct CommandGate {
    limits: Limits,
    lease: AuthorityLease,
    safe_output: f32,
    last_applied: Option<f32>,
    clock: EvaluationClock,
    active_generation: Option<Generation>,
    fallback_reason: Option<SafeReason>,
}
impl CommandGate {
    /// Reject non-finite/out-of-range safe configuration explicitly. The accepted
    /// command watchdog is configured in the lease's CommandPolicy.
    pub fn new(
        limits: Limits,
        lease: AuthorityLease,
        safe_output: f32,
    ) -> Result<Self, GateConfigError> {
        if !limits.permits(safe_output) {
            return Err(GateConfigError::InvalidSafeOutput);
        }
        Ok(Self {
            limits,
            lease,
            safe_output,
            last_applied: None,
            clock: EvaluationClock::default(),
            active_generation: None,
            fallback_reason: None,
        })
    }
    /// Trusted lease replacement; requires a strictly greater generation. Does
    /// not clear the gate-wide evaluation-clock fault. Evaluate before actuating.
    pub fn replace_lease(&mut self, lease: AuthorityLease) -> Result<(), ConfigError> {
        self.lease.replace(lease)
    }
    /// Trusted renewal preserves sequence and all accepted-command validity times.
    pub fn renew_lease(&mut self, expires: Timestamp, now: Timestamp) -> Result<(), ConfigError> {
        self.lease.session.renew(expires, now, &self.clock)
    }
    /// Trusted revocation; the next evaluation applies the safe output.
    pub fn revoke_lease(&mut self) {
        self.lease.session.revoke();
    }
    /// Validated local safe output.
    pub fn safe_output(&self) -> f32 {
        self.safe_output
    }
    /// Last applied value, if evaluated.
    pub fn last_applied(&self) -> Option<f32> {
        self.last_applied
    }
    /// Receiver time of the last fully accepted command, for watchdog diagnostics.
    pub fn last_accepted_at(&self) -> Option<Timestamp> {
        self.lease.session.last_accepted_at()
    }
    /// Evaluate at trusted runtime time. No rejected command refreshes session
    /// sequence, source age, deadline or the accepted-command watchdog.
    pub fn evaluate(&mut self, request: Option<Command>, now: Timestamp) -> GateDecision {
        if let Err(reason) = self.clock.observe(now) {
            return self.enter_safe(request, now, reason);
        }
        let Some(input) = request else {
            let validity = match self.active_generation {
                Some(generation) if generation != self.lease.config().generation() => {
                    Err(SafeReason::GenerationMismatch)
                }
                Some(_) => self.lease.session.check_held(now),
                None => Err(SafeReason::NoCommand),
            };
            if let Err(reason) = validity {
                return self.enter_safe(None, now, reason);
            }
            if let Some(reason) = self.fallback_reason {
                return self.enter_safe(None, now, reason);
            }
            return GateDecision {
                request: None,
                at: now,
                applied: self.last_applied.unwrap_or(self.safe_output),
                outcome: Outcome::Accepted,
                range_clamped: false,
                slew_limited: false,
            };
        };
        if let Err(reason) = self
            .lease
            .validate(input.holder, input.capability, input.meta, now)
        {
            return self.enter_safe(request, now, reason);
        }
        if !input.value.is_finite() {
            return self.enter_safe(request, now, SafeReason::NonFiniteCommand);
        }
        let clamped = clamp(input.value, self.limits.min, self.limits.max);
        let range_clamped = clamped != input.value;
        let (applied, slew_limited) = match self.last_applied {
            Some(prev) => {
                let delta = clamped - prev;
                if !delta.is_finite() {
                    return self.enter_safe(request, now, SafeReason::InvalidOutput);
                }
                if delta > self.limits.max_step {
                    (prev + self.limits.max_step, true)
                } else if delta < -self.limits.max_step {
                    (prev - self.limits.max_step, true)
                } else {
                    (clamped, false)
                }
            }
            None => (clamped, false),
        };
        if !self.limits.permits(applied) {
            return self.enter_safe(request, now, SafeReason::InvalidOutput);
        }
        if let Err(reason) = self.lease.session.accept(input.meta, now) {
            return self.enter_safe(request, now, reason);
        }
        self.last_applied = Some(applied);
        self.active_generation = Some(input.meta.generation);
        self.fallback_reason = None;
        GateDecision {
            request,
            at: now,
            applied,
            outcome: if range_clamped || slew_limited {
                Outcome::Modified
            } else {
                Outcome::Accepted
            },
            range_clamped,
            slew_limited,
        }
    }
    fn enter_safe(
        &mut self,
        request: Option<Command>,
        now: Timestamp,
        reason: SafeReason,
    ) -> GateDecision {
        self.last_applied = Some(self.safe_output);
        self.fallback_reason = Some(reason);
        GateDecision {
            request,
            at: now,
            applied: self.safe_output,
            outcome: Outcome::SafeState(reason),
            range_clamped: false,
            slew_limited: false,
        }
    }
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    let mut v = value;
    if v < min {
        v = min;
    }
    if v > max {
        v = max;
    }
    v
}
