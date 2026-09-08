//! The local command gate — the embedded safety heart.
//!
//! Every actuator command passes through the gate, which enforces, in order:
//! authority (a valid lease), link liveness (the watchdog), command validity,
//! and the actuator envelope (range + slew). When authority or the link is lost,
//! or a command is not finite, the gate applies a **local safe output** without
//! any dependency on the host — the §16.1 / NRX-EMB-004 rule that a node can
//! always reach a safe state on its own.

use neuradix_time::Timestamp;

use crate::lease::AuthorityLease;
use crate::watchdog::Watchdog;

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

/// Why the gate applied the safe output instead of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeReason {
    /// The authority lease had expired (or was cross-domain).
    LeaseExpired,
    /// No fresh command arrived within the watchdog timeout (link loss).
    LinkLost,
    /// The command value was not a finite number.
    BadCommand,
    /// The evaluation clock changed domain; latched until gate reconstruction.
    EvaluationClockMismatch,
    /// Evaluation time regressed; latched until gate reconstruction.
    EvaluationTimeRegression,
    /// Constraint arithmetic or the final output violated finite hard bounds.
    InvalidOutput,
}

/// The disposition of a gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The command was applied unchanged (or a held value was kept).
    Accepted,
    /// The command was applied but modified by the range or slew limit.
    Modified,
    /// The safe output was applied for the given reason.
    SafeState(SafeReason),
}

/// The result of one gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateDecision {
    /// The value actually applied to the actuator.
    pub applied: f32,
    /// The disposition.
    pub outcome: Outcome,
    /// Whether the range limit acted.
    pub range_clamped: bool,
    /// Whether the slew limit acted.
    pub slew_limited: bool,
}

/// The local command gate: lease + watchdog + envelope + safe output.
#[derive(Debug, Clone, Copy)]
pub struct CommandGate {
    limits: Limits,
    lease: AuthorityLease,
    watchdog: Watchdog,
    safe_output: f32,
    last_applied: Option<f32>,
    last_evaluated: Option<Timestamp>,
    time_fault: Option<SafeReason>,
}

impl CommandGate {
    /// Build a gate with a finite safe output inside the validated envelope.
    /// Invalid safe outputs are rejected, never clamped or replaced silently.
    pub fn new(
        limits: Limits,
        lease: AuthorityLease,
        watchdog: Watchdog,
        safe_output: f32,
    ) -> Result<Self, GateConfigError> {
        if !limits.permits(safe_output) {
            return Err(GateConfigError::InvalidSafeOutput);
        }
        Ok(Self {
            limits,
            lease,
            watchdog,
            safe_output,
            last_applied: None,
            last_evaluated: None,
            time_fault: None,
        })
    }

    /// The safe output value.
    pub fn safe_output(&self) -> f32 {
        self.safe_output
    }

    /// The last applied value, if the gate has evaluated at least once.
    pub fn last_applied(&self) -> Option<f32> {
        self.last_applied
    }

    /// Evaluate a (possibly absent) command request at `now`.
    ///
    /// `request` is `Some` when a fresh command arrived this tick (which feeds
    /// the watchdog) and `None` otherwise. The order is deliberate: authority is
    /// checked before the link, and both before the command is shaped, so a
    /// lapsed lease or a lost link always wins over any requested value.
    /// `now` must come from the local runtime, never the command source. A clock
    /// domain change or regression latches a safe state before watchdog feeding.
    /// Equal times are allowed; slew remains per evaluation in this increment.
    pub fn evaluate(&mut self, request: Option<f32>, now: Timestamp) -> GateDecision {
        if self.time_fault.is_none()
            && let Some(previous) = self.last_evaluated
        {
            if now.domain() != previous.domain() {
                self.time_fault = Some(SafeReason::EvaluationClockMismatch);
            } else if now.as_nanos() < previous.as_nanos() {
                self.time_fault = Some(SafeReason::EvaluationTimeRegression);
            }
        }
        if let Some(reason) = self.time_fault {
            return self.enter_safe(reason);
        }
        self.last_evaluated = Some(now);
        if request.is_some() {
            self.watchdog.feed(now);
        }

        // 1. Authority.
        if !self.lease.grants_at(now) {
            return self.enter_safe(SafeReason::LeaseExpired);
        }
        // 2. Link liveness.
        if self.watchdog.is_expired(now) {
            return self.enter_safe(SafeReason::LinkLost);
        }
        // 3. Command presence: with valid authority and a live link, a tick with
        //    no new command holds the last applied value (the watchdog, not a
        //    single missing sample, governs staleness).
        let Some(requested) = request else {
            let applied = self.last_applied.unwrap_or(self.safe_output);
            self.last_applied = Some(applied);
            return GateDecision {
                applied,
                outcome: Outcome::Accepted,
                range_clamped: false,
                slew_limited: false,
            };
        };
        // 4. Command validity.
        if !requested.is_finite() {
            return self.enter_safe(SafeReason::BadCommand);
        }

        // 5. Envelope: range clamp, then slew from the last applied value. The
        //    first command is not slew-limited (there is no previous output to
        //    rate-limit against) — the same rule the host gate enforces.
        let clamped = clamp(requested, self.limits.min, self.limits.max);
        let range_clamped = clamped != requested;

        let (applied, slew_limited) = match self.last_applied {
            Some(prev) => {
                let delta = clamped - prev;
                if !delta.is_finite() {
                    return self.enter_safe(SafeReason::InvalidOutput);
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

        // Check after all arithmetic, including rounding/overflow in slew.
        if !self.limits.permits(applied) {
            return self.enter_safe(SafeReason::InvalidOutput);
        }
        self.last_applied = Some(applied);
        let outcome = if range_clamped || slew_limited {
            Outcome::Modified
        } else {
            Outcome::Accepted
        };
        GateDecision {
            applied,
            outcome,
            range_clamped,
            slew_limited,
        }
    }

    fn enter_safe(&mut self, reason: SafeReason) -> GateDecision {
        self.last_applied = Some(self.safe_output);
        GateDecision {
            applied: self.safe_output,
            outcome: Outcome::SafeState(reason),
            range_clamped: false,
            slew_limited: false,
        }
    }
}

/// Clamp finite input to the private, validated envelope without allocation.
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
