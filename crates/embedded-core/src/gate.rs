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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Minimum applied value.
    pub min: f32,
    /// Maximum applied value.
    pub max: f32,
    /// Maximum change in the applied value per evaluation (slew limit).
    pub max_step: f32,
}

impl Limits {
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

/// Why the gate applied the safe output instead of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeReason {
    /// The authority lease had expired (or was cross-domain).
    LeaseExpired,
    /// No fresh command arrived within the watchdog timeout (link loss).
    LinkLost,
    /// The command value was not a finite number.
    BadCommand,
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
}

impl CommandGate {
    /// Build a gate. `safe_output` is the value applied whenever authority or the
    /// link is lost; it is clamped into the envelope so it is always applicable.
    pub fn new(
        limits: Limits,
        lease: AuthorityLease,
        watchdog: Watchdog,
        safe_output: f32,
    ) -> Self {
        // A non-finite safe output must never reach the actuator; coerce it into
        // the (validated, finite) envelope from a neutral zero.
        let candidate = if safe_output.is_finite() {
            safe_output
        } else {
            0.0
        };
        let safe_output = clamp(candidate, limits.min, limits.max);
        Self {
            limits,
            lease,
            watchdog,
            safe_output,
            last_applied: None,
        }
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
    pub fn evaluate(&mut self, request: Option<f32>, now: Timestamp) -> GateDecision {
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

/// A panic-free clamp: unlike `f32::clamp`, this never panics (the envelope is
/// validated so `min <= max`, but a manual clamp also tolerates a stray NaN
/// bound without a debug panic).
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
